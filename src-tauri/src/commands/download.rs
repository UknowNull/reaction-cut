use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};
use tauri::State;
use tokio::time::sleep;
use url::Url;

use crate::api::ApiResponse;
use crate::config::{default_download_dir, DEFAULT_ARIA2C_PATH};
use crate::commands::settings::load_download_settings_from_db;
use crate::ffmpeg::{run_ffmpeg, run_ffmpeg_with_progress, run_ffprobe_json};
use crate::login_store::AuthInfo;
use crate::utils::{append_log, build_output_path, now_rfc3339, sanitize_filename};
use crate::bilibili::client::BilibiliClient;
use crate::db::Db;
use crate::login_store::LoginStore;
use crate::AppState;

#[derive(Clone)]
struct DownloadContext {
  db: Arc<Db>,
  bilibili: Arc<BilibiliClient>,
  login_store: Arc<LoginStore>,
  download_runtime: Arc<crate::DownloadRuntime>,
  app_log_path: Arc<std::path::PathBuf>,
  edit_upload_state: Arc<std::sync::Mutex<crate::commands::submission::EditUploadState>>,
}

impl DownloadContext {
  fn new(state: &State<'_, AppState>) -> Self {
    Self {
      db: state.db.clone(),
      bilibili: state.bilibili.clone(),
      login_store: state.login_store.clone(),
      download_runtime: state.download_runtime.clone(),
      app_log_path: state.app_log_path.clone(),
      edit_upload_state: state.edit_upload_state.clone(),
    }
  }
}

#[derive(Clone)]
struct StreamCandidate {
  id: Option<i64>,
  bandwidth: i64,
  codec: Option<String>,
  urls: Vec<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DownloadConfig {
  pub download_name: Option<String>,
  pub download_path: Option<String>,
  pub resolution: Option<String>,
  pub codec: Option<String>,
  pub format: Option<String>,
  pub content: Option<String>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DownloadPart {
  pub cid: i64,
  pub title: String,
  pub duration: Option<i64>,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct DownloadRequest {
  pub video_url: String,
  pub parts: Vec<DownloadPart>,
  pub config: DownloadConfig,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionVideoPart {
  #[allow(dead_code)]
  pub original_title: String,
  pub file_path: String,
  pub start_time: Option<String>,
  pub end_time: Option<String>,
  #[allow(dead_code)]
  pub cid: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubmissionRequest {
  pub title: String,
  pub description: Option<String>,
  pub partition_id: i64,
  pub tags: Option<String>,
  pub video_type: String,
  pub collection_id: Option<i64>,
  pub segment_prefix: Option<String>,
  pub video_parts: Vec<SubmissionVideoPart>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IntegrationRequest {
  #[allow(dead_code)]
  pub enable_submission: bool,
  #[allow(dead_code)]
  pub workflow_config: Option<Value>,
  pub download_request: Option<DownloadRequest>,
  pub download_requests: Option<Vec<DownloadRequest>>,
  pub submission_request: SubmissionRequest,
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoDownloadRecord {
  pub id: i64,
  pub bvid: Option<String>,
  pub aid: Option<String>,
  pub title: Option<String>,
  pub part_title: Option<String>,
  pub part_count: Option<i64>,
  pub current_part: Option<i64>,
  pub download_url: Option<String>,
  pub local_path: Option<String>,
  pub resolution: Option<String>,
  pub codec: Option<String>,
  pub format: Option<String>,
  pub status: i64,
  pub progress: i64,
  pub create_time: String,
  pub update_time: String,
}

#[tauri::command]
pub async fn download_video(
  state: State<'_, AppState>,
  payload: Value,
) -> Result<ApiResponse<Value>, String> {
  let context = DownloadContext::new(&state);
  let integration = payload.get("downloadRequest").is_some() || payload.get("downloadRequests").is_some();
  if integration {
    let request: IntegrationRequest = match serde_json::from_value(payload) {
      Ok(request) => request,
      Err(err) => {
        return Ok(ApiResponse::error(format!(
          "Failed to parse download request: {}",
          err
        )));
      }
    };

    return Ok(handle_integration_download(context, request).await);
  }

  let request: DownloadRequest = match serde_json::from_value(payload) {
    Ok(request) => request,
    Err(err) => {
      return Ok(ApiResponse::error(format!(
        "Failed to parse download request: {}",
        err
      )));
    }
  };

  match create_download_task(context, request).await {
    Ok(task_id) => Ok(ApiResponse::success(json!(task_id))),
    Err(err) => Ok(ApiResponse::error(err)),
  }
}

#[tauri::command]
pub fn download_get(state: State<'_, AppState>, task_id: i64) -> ApiResponse<VideoDownloadRecord> {
  match state.db.with_conn(|conn| {
    conn.query_row(
      "SELECT id, bvid, aid, title, part_title, part_count, current_part, download_url, local_path, resolution, codec, format, status, progress, create_time, update_time \
       FROM video_download WHERE id = ?1",
      [task_id],
      |row| {
        Ok(VideoDownloadRecord {
          id: row.get(0)?,
          bvid: row.get(1)?,
          aid: row.get(2)?,
          title: row.get(3)?,
          part_title: row.get(4)?,
          part_count: row.get(5)?,
          current_part: row.get(6)?,
          download_url: row.get(7)?,
          local_path: row.get(8)?,
          resolution: row.get(9)?,
          codec: row.get(10)?,
          format: row.get(11)?,
          status: row.get(12)?,
          progress: row.get(13)?,
          create_time: row.get(14)?,
          update_time: row.get(15)?,
        })
      },
    )
  }) {
    Ok(record) => ApiResponse::success(record),
    Err(err) => ApiResponse::error(format!("Failed to load download task: {}", err)),
  }
}

#[tauri::command]
pub fn download_list_by_status(
  state: State<'_, AppState>,
  status: i64,
) -> ApiResponse<Vec<VideoDownloadRecord>> {
  match state.db.with_conn(|conn| {
    let mut stmt = conn.prepare(
      "SELECT id, bvid, aid, title, part_title, part_count, current_part, download_url, local_path, resolution, codec, format, status, progress, create_time, update_time \
       FROM video_download WHERE status = ?1 ORDER BY id DESC",
    )?;
    let list = stmt
      .query_map([status], |row| {
        Ok(VideoDownloadRecord {
          id: row.get(0)?,
          bvid: row.get(1)?,
          aid: row.get(2)?,
          title: row.get(3)?,
          part_title: row.get(4)?,
          part_count: row.get(5)?,
          current_part: row.get(6)?,
          download_url: row.get(7)?,
          local_path: row.get(8)?,
          resolution: row.get(9)?,
          codec: row.get(10)?,
          format: row.get(11)?,
          status: row.get(12)?,
          progress: row.get(13)?,
          create_time: row.get(14)?,
          update_time: row.get(15)?,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(list)
  }) {
    Ok(list) => ApiResponse::success(list),
    Err(err) => ApiResponse::error(format!("Failed to load downloads: {}", err)),
  }
}

#[tauri::command]
pub fn download_delete(state: State<'_, AppState>, task_id: i64) -> ApiResponse<String> {
  match state.db.with_conn(|conn| {
    conn.execute("DELETE FROM video_download WHERE id = ?1", [task_id])?;
    Ok(())
  }) {
    Ok(()) => ApiResponse::success("Deleted".to_string()),
    Err(err) => ApiResponse::error(format!("Failed to delete: {}", err)),
  }
}

#[tauri::command]
pub async fn download_retry(
  state: State<'_, AppState>,
  task_id: i64,
) -> Result<ApiResponse<String>, String> {
  let context = DownloadContext::new(&state);
  let record = context
    .db
    .with_conn(|conn| {
      conn.query_row(
        "SELECT bvid, aid, part_title, local_path, resolution, codec, format, cid, content, status \
         FROM video_download WHERE id = ?1",
        [task_id],
        |row| {
          Ok((
            row.get::<_, Option<String>>(0)?,
            row.get::<_, Option<String>>(1)?,
            row.get::<_, Option<String>>(2)?,
            row.get::<_, Option<String>>(3)?,
            row.get::<_, Option<String>>(4)?,
            row.get::<_, Option<String>>(5)?,
            row.get::<_, Option<String>>(6)?,
            row.get::<_, Option<i64>>(7)?,
            row.get::<_, Option<String>>(8)?,
            row.get::<_, i64>(9)?,
          ))
        },
      )
    })
    .map_err(|err| format!("读取下载任务失败: {}", err))?;

  let (bvid, aid, part_title, local_path, resolution, codec, format, cid, content, status) =
    record;

  if status == 1 {
    return Ok(ApiResponse::error("任务正在下载"));
  }
  if status == 0 {
    return Ok(ApiResponse::error("任务已在队列中"));
  }
  let cid = match cid {
    Some(value) => value,
    None => return Ok(ApiResponse::error("该任务缺少CID，无法重试")),
  };
  let local_path = match local_path {
    Some(value) => value,
    None => return Ok(ApiResponse::error("缺少本地路径，无法重试")),
  };

  let settings = load_download_settings_from_db(&context.db)
    .map_err(|err| format!("Failed to load download settings: {}", err))?;
  let active_count = count_active_downloads(&context)?;
  if active_count >= settings.queue_size.max(1) {
    return Ok(ApiResponse::error("下载队列已满"));
  }

  let part = DownloadPart {
    cid,
    title: part_title.unwrap_or_else(|| "未命名分P".to_string()),
    duration: None,
  };
  let config = DownloadConfig {
    download_name: None,
    download_path: None,
    resolution,
    codec,
    format,
    content,
  };
  let output_path = PathBuf::from(local_path);
  let _ = update_download_status(&context, task_id, 0, 0);
  let context_clone = context.clone();
  tauri::async_runtime::spawn(async move {
    run_download_job(
      context_clone,
      task_id,
      bvid,
      aid,
      part,
      config,
      output_path,
    )
    .await;
  });

  Ok(ApiResponse::success("Retry started".to_string()))
}

async fn handle_integration_download(
  context: DownloadContext,
  request: IntegrationRequest,
) -> ApiResponse<Value> {
  let mut download_requests = Vec::new();
  if let Some(requests) = request.download_requests.clone() {
    download_requests.extend(requests);
  }
  if download_requests.is_empty() {
    if let Some(single) = request.download_request.clone() {
      download_requests.push(single);
    }
  }
  if download_requests.is_empty() {
    return ApiResponse::error("Missing download requests".to_string());
  }

  let mut download_ids = Vec::new();
  for download_request in download_requests {
    match create_download_tasks(context.clone(), download_request).await {
      Ok(task_ids) => download_ids.extend(task_ids),
      Err(err) => return ApiResponse::error(err),
    }
  }

  let submission_id = uuid::Uuid::new_v4().to_string();
  let now = now_rfc3339();
  let submission = request.submission_request;
  let workflow_config = request.workflow_config.clone();

  let insert_result = context.db.with_conn(|conn| {
    conn.execute(
      "INSERT INTO submission_task (task_id, status, title, description, cover_url, partition_id, tags, video_type, collection_id, bvid, aid, created_at, updated_at, segment_prefix) \
       VALUES (?1, ?2, ?3, ?4, NULL, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10, ?11)",
      (
        &submission_id,
        "PENDING",
        submission.title,
        submission.description,
        submission.partition_id,
        submission.tags,
        submission.video_type,
        submission.collection_id,
        &now,
        &now,
        submission.segment_prefix,
      ),
    )?;

    for (index, part) in submission.video_parts.into_iter().enumerate() {
      let part_id = uuid::Uuid::new_v4().to_string();
      conn.execute(
        "INSERT INTO task_source_video (id, task_id, source_file_path, sort_order, start_time, end_time) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
          part_id,
          &submission_id,
          part.file_path,
          (index + 1) as i64,
          part.start_time,
          part.end_time,
        ),
      )?;
    }
    Ok(())
  });

  if let Err(err) = insert_result {
    return ApiResponse::error(format!("Failed to create submission task: {}", err));
  }

  let workflow_instance_id = match workflow_config.as_ref() {
    Some(config) => {
      match crate::commands::submission::create_workflow_instance_for_task(
        &context.db,
        &submission_id,
        config,
      ) {
        Ok((instance_id, _)) => Some(instance_id),
        Err(err) => return ApiResponse::error(err),
      }
    }
    None => None,
  };

  let relation_result = context.db.with_conn(|conn| {
    for download_id in &download_ids {
      conn.execute(
        "INSERT INTO task_relations (download_task_id, submission_task_id, relation_type, status, created_at, updated_at, workflow_instance_id, workflow_status, retry_count) \
         VALUES (?1, ?2, 'INTEGRATED', 'ACTIVE', ?3, ?4, ?5, 'PENDING_DOWNLOAD', 0)",
        (
          download_id,
          &submission_id,
          &now,
          &now,
          workflow_instance_id.as_deref(),
        ),
      )?;
    }
    Ok(())
  });

  match relation_result {
    Ok(()) => ApiResponse::success(json!({
      "downloadTaskIds": download_ids,
      "submissionTaskId": submission_id,
      "workflowInstanceId": workflow_instance_id,
    })),
    Err(err) => ApiResponse::error(format!("Failed to create submission task: {}", err)),
  }
}

async fn create_download_task(
  context: DownloadContext,
  request: DownloadRequest,
) -> Result<i64, String> {
  let record_ids = create_download_tasks(context, request).await?;
  record_ids
    .first()
    .cloned()
    .ok_or_else(|| "No download task created".to_string())
}

async fn create_download_tasks(
  context: DownloadContext,
  request: DownloadRequest,
) -> Result<Vec<i64>, String> {
  let (bvid, aid) = parse_video_id(&request.video_url);
  let video_title = fetch_video_title(&context, bvid.as_deref(), aid.as_deref()).await;

  let folder_name = request
    .config
    .download_name
    .clone()
    .or(video_title.clone())
    .unwrap_or_else(|| "Unknown".to_string());
  let sanitized_folder = sanitize_filename(&folder_name);

  let now = now_rfc3339();

  let parts = request.parts.clone();
  let mut record_ids = Vec::with_capacity(parts.len());
  let part_count = parts.len() as i64;
  let settings = load_download_settings_from_db(&context.db)
    .map_err(|err| format!("Failed to load download settings: {}", err))?;
  let base_dir = request
    .config
    .download_path
    .clone()
    .filter(|path| !path.trim().is_empty())
    .unwrap_or_else(|| settings.download_path.clone());
  let base_dir = if base_dir.trim().is_empty() {
    default_download_dir().to_string_lossy().to_string()
  } else {
    base_dir
  };
  let queue_size = settings.queue_size.max(1);
  let active_count = count_active_downloads(&context)?;
  if active_count + part_count > queue_size {
    return Err("下载队列已满".to_string());
  }

  for (index, part) in parts.iter().enumerate() {
    let file_name = format!("{}.mp4", sanitize_filename(&part.title));
    let output_path = build_output_path(&base_dir, &sanitized_folder, &file_name);

    let record_id = context
      .db
      .with_conn(|conn| {
        conn.execute(
          "INSERT INTO video_download (bvid, aid, title, part_title, part_count, current_part, download_url, local_path, status, progress, create_time, update_time, resolution, codec, format, cid, content) \
           VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, 0, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
          (
            bvid.as_deref(),
            aid.as_deref(),
            video_title.as_deref(),
            part.title.as_str(),
            part_count,
            (index + 1) as i64,
            request.video_url.as_str(),
            output_path.to_string_lossy().to_string(),
            &now,
            &now,
            request.config.resolution.as_deref(),
            request.config.codec.as_deref(),
            request.config.format.as_deref(),
            part.cid,
            request.config.content.as_deref(),
          ),
        )?;
        Ok(conn.last_insert_rowid())
      })
      .map_err(|err| format!("Failed to save download record: {}", err))?;

    record_ids.push(record_id);

    let context_clone = context.clone();
    let part_clone = part.clone();
    let config_clone = request.config.clone();
    let bvid_clone = bvid.clone();
    let aid_clone = aid.clone();
    let output_path_clone = output_path.clone();

    tauri::async_runtime::spawn(async move {
      run_download_job(
        context_clone,
        record_id,
        bvid_clone,
        aid_clone,
        part_clone,
        config_clone,
        output_path_clone,
      )
      .await;
    });
  }

  if record_ids.is_empty() {
    return Err("No download task created".to_string());
  }
  Ok(record_ids)
}

async fn run_download_job(
  context: DownloadContext,
  record_id: i64,
  bvid: Option<String>,
  aid: Option<String>,
  part: DownloadPart,
  config: DownloadConfig,
  output_path: PathBuf,
) {
  if wait_for_download_slot(&context).await.is_err() {
    let _ = update_download_status(&context, record_id, 3, 0);
    return;
  }

  let _ = update_download_status(&context, record_id, 1, 0);
  append_log(
    &context.app_log_path,
    &format!("download_job_start record_id={} cid={}", record_id, part.cid),
  );

  let result = download_part(&context, record_id, bvid, aid, part, config, output_path).await;
  release_download_slot(&context);
  match result {
    Ok(()) => {
      let _ = update_download_status(&context, record_id, 2, 100);
      append_log(
        &context.app_log_path,
        &format!("download_job_complete record_id={} status=completed", record_id),
      );
      let _ = refresh_integration_status(&context, record_id).await;
    }
    Err(err) => {
      let _ = update_download_status(&context, record_id, 3, 0);
      append_log(
        &context.app_log_path,
        &format!("download_job_complete record_id={} status=failed err={}", record_id, err),
      );
      let _ = refresh_integration_status(&context, record_id).await;
    }
  }
}

async fn wait_for_download_slot(context: &DownloadContext) -> Result<(), String> {
  loop {
    let settings = load_download_settings_from_db(&context.db)
      .map_err(|err| format!("Failed to load download settings: {}", err))?;
    let threads = settings.threads.max(1);
    let acquired = {
      let mut active = context
        .download_runtime
        .active_count
        .lock()
        .map_err(|_| "Download state lock failed".to_string())?;
      if *active < threads {
        *active += 1;
        true
      } else {
        false
      }
    };
    if acquired {
      return Ok(());
    }
    sleep(Duration::from_secs(1)).await;
  }
}

fn release_download_slot(context: &DownloadContext) {
  if let Ok(mut active) = context.download_runtime.active_count.lock() {
    if *active > 0 {
      *active -= 1;
    }
  }
}

async fn download_part(
  context: &DownloadContext,
  record_id: i64,
  bvid: Option<String>,
  aid: Option<String>,
  part: DownloadPart,
  config: DownloadConfig,
  output_path: PathBuf,
) -> Result<(), String> {
  let settings = load_download_settings_from_db(&context.db)
    .map_err(|err| format!("Failed to load download settings: {}", err))?;
  let block_pcdn = settings.block_pcdn;
  let enable_aria2c = settings.enable_aria2c;
  let play_info = fetch_play_info(context, bvid.clone(), aid.clone(), part.cid, &config).await?;
  let mut format = config.format.clone().unwrap_or_else(|| "dash".to_string());
  let has_dash = play_info.get("dash").is_some();
  let has_durl = play_info.get("durl").is_some();
  if format == "dash" && !has_dash && has_durl {
    append_log(
      &context.app_log_path,
      &format!("playurl_format_fallback record_id={} from=dash to=mp4", record_id),
    );
    format = "mp4".to_string();
  }
  if (format == "mp4" || format == "flv") && !has_durl && has_dash {
    append_log(
      &context.app_log_path,
      &format!(
        "playurl_format_fallback record_id={} from={} to=dash",
        record_id, format
      ),
    );
    format = "dash".to_string();
  }
  let duration_ms = part.duration.and_then(|value| value.max(0).checked_mul(1000));
  let expected_duration_seconds = part.duration.unwrap_or(0).max(0) as f64;
  let track_progress = duration_ms.unwrap_or(0) > 0;

  let header = build_ffmpeg_headers(context).unwrap_or_default();
  let output_path_string = output_path.to_string_lossy().to_string();

  if format == "mp4" || format == "flv" {
    let urls = collect_durl_urls(&play_info, block_pcdn)?;
    if enable_aria2c {
      if let Err(err) = download_with_aria2c(
        context,
        record_id,
        track_progress,
        &output_path,
        &urls,
        &header,
        None,
      )
      .await
      {
        append_log(
          &context.app_log_path,
          &format!("aria2c_fallback record_id={} err={}", record_id, err),
        );
      } else {
        return Ok(());
      }
    }
    run_ffmpeg_job_with_url_fallback(
      context,
      record_id,
      track_progress,
      duration_ms,
      &format,
      &output_path,
      &urls,
      |url| {
        let mut args = Vec::new();
        if !header.is_empty() {
          args.push("-headers".to_string());
          args.push(header.clone());
        }
        args.push("-i".to_string());
        args.push(url.to_string());
        args.extend(["-c".to_string(), "copy".to_string()]);
        if track_progress {
          args.push("-progress".to_string());
          args.push("pipe:1".to_string());
          args.push("-nostats".to_string());
        }
        args.push(output_path_string.clone());
        args
      },
    )
    .await?;
    return Ok(());
  }

  let dash = play_info
    .get("dash")
    .ok_or_else(|| "Missing dash info".to_string())?;
  let content = config.content.unwrap_or_else(|| "audio_video".to_string());

  match content.as_str() {
    "video_only" => {
      let video_candidates =
        select_video_candidates(dash, config.resolution.as_deref(), config.codec.as_deref(), block_pcdn)?;
      let video_urls = video_candidates
        .first()
        .map(|candidate| candidate.urls.clone())
        .ok_or_else(|| "Missing video URL".to_string())?;
      if enable_aria2c {
        if let Err(err) = download_with_aria2c(
          context,
          record_id,
          track_progress,
          &output_path,
          &video_urls,
          &header,
          None,
        )
        .await
        {
          append_log(
            &context.app_log_path,
            &format!("aria2c_fallback record_id={} err={}", record_id, err),
          );
        } else {
          return Ok(());
        }
      }
      run_ffmpeg_job_with_url_fallback(
        context,
        record_id,
        track_progress,
        duration_ms,
        &format,
        &output_path,
        &video_urls,
        |url| {
          let mut args = Vec::new();
          if !header.is_empty() {
            args.push("-headers".to_string());
            args.push(header.clone());
          }
          args.push("-i".to_string());
          args.push(url.to_string());
          args.extend(["-c".to_string(), "copy".to_string()]);
          if track_progress {
            args.push("-progress".to_string());
            args.push("pipe:1".to_string());
            args.push("-nostats".to_string());
          }
          args.push(output_path_string.clone());
          args
        },
      )
      .await?;
      Ok(())
    }
    "audio_only" => {
      let audio_candidates = select_audio_candidates(dash, block_pcdn)?;
      let audio_urls = audio_candidates
        .first()
        .map(|candidate| candidate.urls.clone())
        .ok_or_else(|| "Missing audio URL".to_string())?;
      if enable_aria2c {
        if let Err(err) = download_with_aria2c(
          context,
          record_id,
          track_progress,
          &output_path,
          &audio_urls,
          &header,
          None,
        )
        .await
        {
          append_log(
            &context.app_log_path,
            &format!("aria2c_fallback record_id={} err={}", record_id, err),
          );
        } else {
          return Ok(());
        }
      }
      run_ffmpeg_job_with_url_fallback(
        context,
        record_id,
        track_progress,
        duration_ms,
        &format,
        &output_path,
        &audio_urls,
        |url| {
          let mut args = Vec::new();
          if !header.is_empty() {
            args.push("-headers".to_string());
            args.push(header.clone());
          }
          args.push("-i".to_string());
          args.push(url.to_string());
          args.extend(["-c".to_string(), "copy".to_string()]);
          if track_progress {
            args.push("-progress".to_string());
            args.push("pipe:1".to_string());
            args.push("-nostats".to_string());
          }
          args.push(output_path_string.clone());
          args
        },
      )
      .await?;
      Ok(())
    }
    _ => {
      let video_candidates =
        select_video_candidates(dash, config.resolution.as_deref(), config.codec.as_deref(), block_pcdn)?;
      let audio_candidates = select_audio_candidates(dash, block_pcdn)?;
      let mut last_error: Option<String> = None;
      let mut aria2c_enabled = enable_aria2c;
      for (video_index, video_candidate) in video_candidates.iter().enumerate() {
        for (audio_index, audio_candidate) in audio_candidates.iter().enumerate() {
          let mut aria2c_failed = !aria2c_enabled;
          let temp_video_path = output_path.with_extension("video");
          let temp_audio_path = output_path.with_extension("audio");
          if aria2c_enabled {
            if let Err(err) = download_with_aria2c(
              context,
              record_id,
              track_progress,
              &temp_video_path,
              &video_candidate.urls,
              &header,
              Some((0, 45)),
            )
            .await
            {
              append_log(
                &context.app_log_path,
                &format!("aria2c_fallback record_id={} err={}", record_id, err),
              );
              if is_aria2c_missing_error(&err) {
                aria2c_enabled = false;
              }
              let _ = std::fs::remove_file(&temp_video_path);
              aria2c_failed = true;
            } else if let Err(err) = download_with_aria2c(
              context,
              record_id,
              track_progress,
              &temp_audio_path,
              &audio_candidate.urls,
              &header,
              Some((45, 90)),
            )
            .await
            {
              append_log(
                &context.app_log_path,
                &format!("aria2c_fallback record_id={} err={}", record_id, err),
              );
              if is_aria2c_missing_error(&err) {
                aria2c_enabled = false;
              }
              let _ = std::fs::remove_file(&temp_video_path);
              let _ = std::fs::remove_file(&temp_audio_path);
              aria2c_failed = true;
            } else {
              let _ = update_download_status(context, record_id, 1, 95);
              let mut args = Vec::new();
              args.push("-i".to_string());
              args.push(temp_video_path.to_string_lossy().to_string());
              args.push("-i".to_string());
              args.push(temp_audio_path.to_string_lossy().to_string());
              args.extend([
                "-map".to_string(),
                "0:v:0".to_string(),
                "-map".to_string(),
                "1:a:0".to_string(),
                "-c".to_string(),
                "copy".to_string(),
              ]);
              args.push(output_path_string.clone());
              match run_ffmpeg_job(
                context,
                record_id,
                false,
                duration_ms,
                &format,
                &output_path,
                args,
              )
              .await
              {
                Ok(_) => {
                  let _ = update_download_status(context, record_id, 1, 99);
                  match probe_stream_durations(&output_path) {
                  Ok((video_duration, audio_duration)) => {
                    if !is_video_complete(
                      video_duration,
                      audio_duration,
                      expected_duration_seconds,
                    ) {
                      append_log(
                        &context.app_log_path,
                        &format!(
                          "ffprobe_video_short record_id={} video={:.3} audio={:.3} expected={:.3}",
                          record_id, video_duration, audio_duration, expected_duration_seconds
                        ),
                      );
                      let _ = std::fs::remove_file(&output_path);
                      last_error = Some("Video stream too short".to_string());
                    } else if is_audio_complete(video_duration, audio_duration) {
                      let _ = std::fs::remove_file(&temp_video_path);
                      let _ = std::fs::remove_file(&temp_audio_path);
                      return Ok(());
                    } else {
                      append_log(
                        &context.app_log_path,
                        &format!(
                          "ffprobe_audio_short record_id={} video={:.3} audio={:.3}",
                          record_id, video_duration, audio_duration
                        ),
                      );
                      let _ = std::fs::remove_file(&output_path);
                      last_error = Some("Audio stream too short".to_string());
                    }
                  }
                  Err(err) => {
                    append_log(
                      &context.app_log_path,
                      &format!("ffprobe_check_fail record_id={} err={}", record_id, err),
                    );
                    let _ = std::fs::remove_file(&output_path);
                    last_error = Some(err);
                  }
                  }
                }
                Err(err) => {
                  let _ = std::fs::remove_file(&output_path);
                  last_error = Some(err);
                }
              }
              let _ = std::fs::remove_file(&temp_video_path);
              let _ = std::fs::remove_file(&temp_audio_path);
              if last_error.is_none() {
                return Ok(());
              }
              aria2c_failed = true;
            }
          }

          if !aria2c_failed {
            continue;
          }

          for (video_url_index, video_url) in video_candidate.urls.iter().enumerate() {
            for (audio_url_index, audio_url) in audio_candidate.urls.iter().enumerate() {
              if video_index > 0
                || audio_index > 0
                || video_url_index > 0
                || audio_url_index > 0
              {
                append_log(
                  &context.app_log_path,
                  &format!(
                    "ffmpeg_retry record_id={} video={} audio={}",
                    record_id,
                    video_index + 1,
                    audio_index + 1
                  ),
                );
              }
              let mut args = Vec::new();
              if !header.is_empty() {
                args.push("-headers".to_string());
                args.push(header.clone());
              }
              args.push("-i".to_string());
              args.push(video_url.clone());
              if !header.is_empty() {
                args.push("-headers".to_string());
                args.push(header.clone());
              }
              args.push("-i".to_string());
              args.push(audio_url.clone());
              args.extend([
                "-map".to_string(),
                "0:v:0".to_string(),
                "-map".to_string(),
                "1:a:0".to_string(),
                "-c".to_string(),
                "copy".to_string(),
              ]);
              if track_progress {
                args.push("-progress".to_string());
                args.push("pipe:1".to_string());
                args.push("-nostats".to_string());
              }
              args.push(output_path_string.clone());
              match run_ffmpeg_job(
                context,
                record_id,
                track_progress,
                duration_ms,
                &format,
                &output_path,
                args,
              )
              .await
              {
                Ok(_) => match probe_stream_durations(&output_path) {
                  Ok((video_duration, audio_duration)) => {
                    if !is_video_complete(
                      video_duration,
                      audio_duration,
                      expected_duration_seconds,
                    ) {
                      append_log(
                        &context.app_log_path,
                        &format!(
                          "ffprobe_video_short record_id={} video={:.3} audio={:.3} expected={:.3}",
                          record_id, video_duration, audio_duration, expected_duration_seconds
                        ),
                      );
                      let _ = std::fs::remove_file(&output_path);
                      last_error = Some("Video stream too short".to_string());
                      break;
                    }
                    if is_audio_complete(video_duration, audio_duration) {
                      return Ok(());
                    }
                    append_log(
                      &context.app_log_path,
                      &format!(
                        "ffprobe_audio_short record_id={} video={:.3} audio={:.3}",
                        record_id, video_duration, audio_duration
                      ),
                    );
                    let _ = std::fs::remove_file(&output_path);
                    last_error = Some("Audio stream too short".to_string());
                    continue;
                  }
                  Err(err) => {
                    append_log(
                      &context.app_log_path,
                      &format!("ffprobe_check_fail record_id={} err={}", record_id, err),
                    );
                    let _ = std::fs::remove_file(&output_path);
                    last_error = Some(err);
                    continue;
                  }
                },
                Err(err) => {
                  let _ = std::fs::remove_file(&output_path);
                  last_error = Some(err);
                  continue;
                }
              }
            }
          }
        }
      }
      Err(last_error.unwrap_or_else(|| "Missing audio streams".to_string()))
    }
  }
}

async fn run_ffmpeg_job(
  context: &DownloadContext,
  record_id: i64,
  track_progress: bool,
  duration_ms: Option<i64>,
  format: &str,
  output_path: &Path,
  args: Vec<String>,
) -> Result<(), String> {
  if let Some(parent) = output_path.parent() {
    std::fs::create_dir_all(parent).map_err(|err| format!("Failed to create directory: {}", err))?;
  }

  append_log(
    &context.app_log_path,
    &format!(
      "ffmpeg_start record_id={} progress={} format={} output={}",
      record_id,
      track_progress,
      format,
      output_path.to_string_lossy()
    ),
  );

  let exec_result = if track_progress {
    let mut last_progress = 0;
    let context_clone = context.clone();
    let record_id_clone = record_id;
    tauri::async_runtime::spawn_blocking(move || {
      run_ffmpeg_with_progress(&args, duration_ms, |progress| {
        if progress > last_progress {
          last_progress = progress;
          let _ = update_download_status(&context_clone, record_id_clone, 1, progress);
        }
      })
    })
    .await
    .map_err(|_| "Failed to execute download task".to_string())?
  } else {
    tauri::async_runtime::spawn_blocking(move || run_ffmpeg(&args))
      .await
      .map_err(|_| "Failed to execute download task".to_string())?
  };

  match &exec_result {
    Ok(_) => {
      append_log(
        &context.app_log_path,
        &format!("ffmpeg_done record_id={} status=ok", record_id),
      );
    }
    Err(err) => {
      append_log(
        &context.app_log_path,
        &format!("ffmpeg_done record_id={} status=err msg={}", record_id, err),
      );
    }
  }

  exec_result
}

async fn run_ffmpeg_job_with_url_fallback<F>(
  context: &DownloadContext,
  record_id: i64,
  track_progress: bool,
  duration_ms: Option<i64>,
  format: &str,
  output_path: &Path,
  urls: &[String],
  build_args: F,
) -> Result<(), String>
where
  F: Fn(&str) -> Vec<String>,
{
  if urls.is_empty() {
    return Err("Missing stream url".to_string());
  }
  let mut last_error = None;
  for (index, url) in urls.iter().enumerate() {
    if index > 0 {
      append_log(
        &context.app_log_path,
        &format!("ffmpeg_url_retry record_id={} index={}", record_id, index + 1),
      );
    }
    let args = build_args(url);
    match run_ffmpeg_job(
      context,
      record_id,
      track_progress,
      duration_ms,
      format,
      output_path,
      args,
    )
    .await
    {
      Ok(_) => return Ok(()),
      Err(err) => {
        let _ = std::fs::remove_file(output_path);
        last_error = Some(err);
        continue;
      }
    }
  }
  Err(last_error.unwrap_or_else(|| "Missing stream url".to_string()))
}

fn build_aria2c_args(
  output_path: &Path,
  urls: &[String],
  header: &str,
) -> Result<Vec<String>, String> {
  let parent = output_path
    .parent()
    .ok_or_else(|| "Missing output directory".to_string())?;
  let file_name = output_path
    .file_name()
    .ok_or_else(|| "Missing output file name".to_string())?;
  let mut args = vec![
    "--allow-overwrite=true".to_string(),
    "--auto-file-renaming=false".to_string(),
    "--file-allocation=none".to_string(),
    "--summary-interval=1".to_string(),
    "--console-log-level=warn".to_string(),
    "--max-connection-per-server=4".to_string(),
    "--split=4".to_string(),
    "--min-split-size=1M".to_string(),
    format!("--dir={}", parent.to_string_lossy()),
    format!("--out={}", file_name.to_string_lossy()),
  ];
  for line in header.split("\r\n").map(|value| value.trim()) {
    if !line.is_empty() {
      args.push(format!("--header={}", line));
    }
  }
  args.extend(urls.iter().cloned());
  Ok(args)
}

fn parse_aria2c_progress(line: &str) -> Option<i64> {
  let mut digits = String::new();
  let mut last_percent = None;
  for ch in line.chars() {
    if ch.is_ascii_digit() {
      digits.push(ch);
      continue;
    }
    if ch == '%' && !digits.is_empty() {
      last_percent = digits.parse::<i64>().ok();
    }
    digits.clear();
  }
  last_percent
}

fn run_aria2c_with_path<F>(
  path: &str,
  args: &[String],
  track_progress: bool,
  on_progress: &mut F,
) -> Result<(), String>
where
  F: FnMut(i64),
{
  let mut child = Command::new(path)
    .args(args)
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .spawn()
    .map_err(|err| format!("Failed to start aria2c: {}", err))?;

  let stdout = child
    .stdout
    .take()
    .ok_or_else(|| "Failed to capture aria2c stdout".to_string())?;
  let mut stderr = child
    .stderr
    .take()
    .ok_or_else(|| "Failed to capture aria2c stderr".to_string())?;

  let (stderr_tx, stderr_rx) = std::sync::mpsc::channel();
  std::thread::spawn(move || {
    let mut buffer = String::new();
    let _ = stderr.read_to_string(&mut buffer);
    let _ = stderr_tx.send(buffer);
  });

  let mut last_progress = -1;
  let reader = BufReader::new(stdout);
  for line in reader.lines().flatten() {
    if track_progress {
      if let Some(progress) = parse_aria2c_progress(&line) {
        let progress = progress.min(99);
        if progress > last_progress {
          last_progress = progress;
          on_progress(progress);
        }
      }
    }
  }

  let status = child
    .wait()
    .map_err(|err| format!("Failed to wait for aria2c: {}", err))?;
  let stderr_output = stderr_rx.recv().unwrap_or_default();
  if status.success() {
    return Ok(());
  }

  Err(format!("aria2c failed: {}", stderr_output.trim()))
}

fn run_aria2c_command<F>(
  args: &[String],
  track_progress: bool,
  on_progress: &mut F,
) -> Result<(), String>
where
  F: FnMut(i64),
{
  let mut last_error = None;
  for path in [DEFAULT_ARIA2C_PATH, "aria2c"] {
    match run_aria2c_with_path(path, args, track_progress, on_progress) {
      Ok(_) => return Ok(()),
      Err(err) => {
        last_error = Some(err);
      }
    }
  }
  Err(last_error.unwrap_or_else(|| "aria2c not available".to_string()))
}

fn is_aria2c_missing_error(message: &str) -> bool {
  let lower = message.to_lowercase();
  lower.contains("aria2c") && (lower.contains("no such file") || lower.contains("not found"))
}

async fn download_with_aria2c(
  context: &DownloadContext,
  record_id: i64,
  track_progress: bool,
  output_path: &Path,
  urls: &[String],
  header: &str,
  progress_range: Option<(i64, i64)>,
) -> Result<(), String> {
  if urls.is_empty() {
    return Err("Missing stream url".to_string());
  }
  if let Some(parent) = output_path.parent() {
    std::fs::create_dir_all(parent).map_err(|err| format!("Failed to create directory: {}", err))?;
  }

  let args = build_aria2c_args(output_path, urls, header)?;
  append_log(
    &context.app_log_path,
    &format!(
      "aria2c_start record_id={} output={}",
      record_id,
      output_path.to_string_lossy()
    ),
  );

  let context_clone = context.clone();
  let output_path = output_path.to_path_buf();
  let exec_result = tauri::async_runtime::spawn_blocking(move || {
    let mut last_progress = -1;
    let mut update = |progress: i64| {
      let progress = progress.min(99);
      let mapped = if let Some((start, end)) = progress_range {
        if end <= start {
          end
        } else {
          start + ((progress * (end - start)) / 100)
        }
      } else {
        progress
      };
      if mapped > last_progress {
        last_progress = mapped;
        let _ = update_download_status(&context_clone, record_id, 1, mapped);
      }
    };
    run_aria2c_command(&args, track_progress, &mut update)
  })
  .await
  .map_err(|_| "Failed to execute download task".to_string())?;

  match &exec_result {
    Ok(_) => {
      append_log(
        &context.app_log_path,
        &format!("aria2c_done record_id={} status=ok", record_id),
      );
    }
    Err(err) => {
      append_log(
        &context.app_log_path,
        &format!("aria2c_done record_id={} status=err msg={}", record_id, err),
      );
      let _ = std::fs::remove_file(&output_path);
      let _ = update_download_status(context, record_id, 1, 0);
    }
  }

  exec_result
}

async fn fetch_play_info(
  context: &DownloadContext,
  bvid: Option<String>,
  aid: Option<String>,
  cid: i64,
  config: &DownloadConfig,
) -> Result<Value, String> {
  let format = config.format.as_deref().unwrap_or("dash");
  let auth = load_auth(context);
  let is_logged_in = auth.is_some();
  let qn = config
    .resolution
    .clone()
    .unwrap_or_else(|| if is_logged_in { "127".to_string() } else { "64".to_string() });
  let fnval = match format {
    "flv" => "0",
    "mp4" => "1",
    _ => {
      if is_logged_in {
        "4048"
      } else {
        "16"
      }
    }
  };
  let mut params = vec![
    ("cid".to_string(), cid.to_string()),
    ("qn".to_string(), qn),
    ("fnval".to_string(), fnval.to_string()),
    ("fnver".to_string(), "0".to_string()),
    ("fourk".to_string(), "1".to_string()),
  ];

  if let Some(bvid) = bvid {
    params.push(("bvid".to_string(), bvid));
  }
  if let Some(aid) = aid {
    params.push(("avid".to_string(), aid));
  }

  let url = format!("{}/x/player/wbi/playurl", context.bilibili.base_url());
  context
    .bilibili
    .get_json(&url, &params, auth.as_ref(), true)
    .await
}

fn collect_durl_urls(play_info: &Value, block_pcdn: bool) -> Result<Vec<String>, String> {
  let durl = play_info
    .get("durl")
    .and_then(|value| value.as_array())
    .and_then(|list| list.get(0))
    .ok_or_else(|| "Missing mp4 url".to_string())?;
  let mut urls = Vec::new();
  if let Some(url) = durl.get("url").and_then(|value| value.as_str()) {
    urls.push(url.to_string());
  }
  if let Some(list) = durl.get("backup_url").and_then(|value| value.as_array()) {
    for item in list {
      if let Some(url) = item.as_str() {
        urls.push(url.to_string());
      }
    }
  }
  let urls = normalize_stream_urls(urls, block_pcdn);
  if urls.is_empty() {
    return Err("Missing mp4 url".to_string());
  }
  Ok(urls)
}

fn candidate_codec_matches(candidate: &StreamCandidate, codec: &str) -> bool {
  candidate
    .codec
    .as_deref()
    .map(|value| value.contains(codec))
    .unwrap_or(false)
}

fn choose_target_resolution(
  candidates: &[StreamCandidate],
  resolution: Option<&str>,
) -> Option<i64> {
  let mut ids: Vec<i64> = candidates.iter().filter_map(|candidate| candidate.id).collect();
  if ids.is_empty() {
    return None;
  }
  if let Some(resolution) = resolution {
    if let Ok(resolution) = resolution.parse::<i64>() {
      if ids.iter().any(|id| *id == resolution) {
        return Some(resolution);
      }
    }
  }
  ids.sort_unstable();
  ids.pop()
}

fn choose_target_codec(
  candidates: &[StreamCandidate],
  target_resolution: Option<i64>,
  codec: Option<&str>,
) -> Option<String> {
  let filtered: Vec<&StreamCandidate> = candidates
    .iter()
    .filter(|candidate| {
      target_resolution
        .map(|resolution| candidate.id == Some(resolution))
        .unwrap_or(true)
    })
    .collect();
  if filtered.is_empty() {
    return None;
  }
  if let Some(codec) = codec {
    if filtered.iter().any(|candidate| candidate_codec_matches(candidate, codec)) {
      return Some(codec.to_string());
    }
  }
  for codec in ["avc1", "hev1", "hvc1", "vp09", "av01"] {
    if filtered.iter().any(|candidate| candidate_codec_matches(candidate, codec)) {
      return Some(codec.to_string());
    }
  }
  filtered.iter().find_map(|candidate| candidate.codec.clone())
}

fn select_audio_candidates(
  dash: &Value,
  block_pcdn: bool,
) -> Result<Vec<StreamCandidate>, String> {
  let audios = dash
    .get("audio")
    .and_then(|value| value.as_array())
    .ok_or_else(|| "Missing audio streams".to_string())?;
  if audios.is_empty() {
    return Err("Missing audio streams".to_string());
  }
  let mut candidates: Vec<StreamCandidate> = Vec::new();
  for item in audios {
    let bandwidth = item.get("bandwidth").and_then(|value| value.as_i64()).unwrap_or(0);
    let urls = stream_urls_from_item(item, block_pcdn);
    if !urls.is_empty() {
      candidates.push(StreamCandidate {
        id: item.get("id").and_then(|value| value.as_i64()),
        bandwidth,
        codec: None,
        urls,
      });
    }
  }
  candidates.sort_by(|a, b| b.bandwidth.cmp(&a.bandwidth));
  if candidates.is_empty() {
    return Err("Missing audio URL".to_string());
  }
  Ok(candidates)
}

fn select_video_candidates(
  dash: &Value,
  resolution: Option<&str>,
  codec: Option<&str>,
  block_pcdn: bool,
) -> Result<Vec<StreamCandidate>, String> {
  let videos = dash
    .get("video")
    .and_then(|value| value.as_array())
    .ok_or_else(|| "Missing video streams".to_string())?;
  if videos.is_empty() {
    return Err("Missing video streams".to_string());
  }
  let mut candidates: Vec<StreamCandidate> = Vec::new();
  for item in videos {
    let bandwidth = item.get("bandwidth").and_then(|value| value.as_i64()).unwrap_or(0);
    let urls = stream_urls_from_item(item, block_pcdn);
    if urls.is_empty() {
      continue;
    }
    candidates.push(StreamCandidate {
      id: item.get("id").and_then(|value| value.as_i64()),
      bandwidth,
      codec: item
        .get("codecs")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string()),
      urls,
    });
  }
  if candidates.is_empty() {
    return Err("Missing video URL".to_string());
  }
  let target_resolution = choose_target_resolution(&candidates, resolution);
  let target_codec = choose_target_codec(&candidates, target_resolution, codec);
  candidates.sort_by(|a, b| {
    let a_res = target_resolution.map(|resolution| a.id == Some(resolution)).unwrap_or(false);
    let b_res = target_resolution.map(|resolution| b.id == Some(resolution)).unwrap_or(false);
    let a_codec = target_codec
      .as_deref()
      .map(|codec| candidate_codec_matches(a, codec))
      .unwrap_or(false);
    let b_codec = target_codec
      .as_deref()
      .map(|codec| candidate_codec_matches(b, codec))
      .unwrap_or(false);
    let a_priority = if a_res && a_codec {
      0
    } else if a_res {
      1
    } else if a_codec {
      2
    } else {
      3
    };
    let b_priority = if b_res && b_codec {
      0
    } else if b_res {
      1
    } else if b_codec {
      2
    } else {
      3
    };
    a_priority.cmp(&b_priority).then_with(|| b.bandwidth.cmp(&a.bandwidth))
  });
  Ok(candidates)
}

fn probe_stream_durations(path: &Path) -> Result<(f64, f64), String> {
  let args = vec![
    "-v".to_string(),
    "error".to_string(),
    "-show_streams".to_string(),
    "-of".to_string(),
    "json".to_string(),
    path.to_string_lossy().to_string(),
  ];
  let data = run_ffprobe_json(&args)?;
  let streams = data
    .get("streams")
    .and_then(|value| value.as_array())
    .ok_or_else(|| "Missing stream info".to_string())?;
  let mut video_duration = 0.0;
  let mut audio_duration = 0.0;
  let format_duration = data
    .get("format")
    .and_then(|value| value.get("duration"))
    .and_then(|value| value.as_str())
    .and_then(|value| value.parse::<f64>().ok())
    .unwrap_or(0.0);
  for stream in streams {
    let codec_type = stream
      .get("codec_type")
      .and_then(|value| value.as_str())
      .unwrap_or("");
    let duration = stream
      .get("duration")
      .and_then(|value| value.as_str())
      .and_then(|value| value.parse::<f64>().ok())
      .unwrap_or(0.0);
    if codec_type == "video" && video_duration <= 0.0 {
      video_duration = duration;
    }
    if codec_type == "audio" && audio_duration <= 0.0 {
      audio_duration = duration;
    }
  }
  if video_duration <= 0.0 && format_duration > 0.0 {
    video_duration = format_duration;
  }
  Ok((video_duration, audio_duration))
}

fn is_video_complete(video_duration: f64, audio_duration: f64, expected_duration: f64) -> bool {
  if video_duration <= 0.0 {
    return false;
  }
  if expected_duration > 0.0
    && video_duration + 10.0 < expected_duration
    && video_duration / expected_duration < 0.9
  {
    return false;
  }
  if audio_duration > 0.0
    && video_duration + 10.0 < audio_duration
    && video_duration / audio_duration < 0.9
  {
    return false;
  }
  true
}

fn is_audio_complete(video_duration: f64, audio_duration: f64) -> bool {
  if audio_duration <= 0.0 {
    return false;
  }
  if video_duration <= 0.0 {
    return true;
  }
  if audio_duration + 10.0 < video_duration && audio_duration / video_duration < 0.9 {
    return false;
  }
  true
}

fn dedup_urls(urls: Vec<String>) -> Vec<String> {
  let mut seen = HashSet::new();
  let mut result = Vec::new();
  for url in urls {
    if seen.insert(url.clone()) {
      result.push(url);
    }
  }
  result
}

fn filter_pcdn_urls(urls: Vec<String>) -> Vec<String> {
  let mut mirror = Vec::new();
  let mut upos = Vec::new();
  let mut bcache = Vec::new();
  let mut others = Vec::new();
  for raw in urls {
    match Url::parse(&raw) {
      Ok(url) => {
        let host = url.host_str().unwrap_or("");
        let os = url
          .query_pairs()
          .find(|(key, _)| key == "os")
          .map(|(_, value)| value.to_string())
          .unwrap_or_default();
        if host.contains("mirror") && os.ends_with("bv") {
          mirror.push(url);
        } else if os == "upos" {
          upos.push(url);
        } else if host.starts_with("cn") && os == "bcache" {
          bcache.push(url);
        } else {
          others.push(url.to_string());
        }
      }
      Err(_) => {
        others.push(raw);
      }
    }
  }
  if !mirror.is_empty() {
    let mut results = if mirror.len() < 2 {
      let mut combined = mirror;
      combined.extend(upos);
      combined
    } else {
      mirror
    };
    return results.drain(..).map(|url| url.to_string()).collect();
  }
  if !upos.is_empty() || !bcache.is_empty() {
    let mut results = if !upos.is_empty() { upos } else { bcache };
    let mirror_list = ["upos-sz-mirrorali.bilivideo.com", "upos-sz-mirrorcos.bilivideo.com"];
    for (index, url) in results.iter_mut().enumerate() {
      if let Some(host) = mirror_list.get(index) {
        let _ = url.set_host(Some(host));
      }
    }
    return results.drain(..).map(|url| url.to_string()).collect();
  }
  others
}

fn normalize_stream_urls(urls: Vec<String>, block_pcdn: bool) -> Vec<String> {
  let urls = dedup_urls(urls);
  let urls = if block_pcdn {
    filter_pcdn_urls(urls)
  } else {
    urls
  };
  dedup_urls(urls)
}

fn stream_urls_from_item(item: &Value, block_pcdn: bool) -> Vec<String> {
  let mut urls = Vec::new();
  if let Some(url) = item
    .get("base_url")
    .or_else(|| item.get("baseUrl"))
    .and_then(|value| value.as_str())
  {
    urls.push(url.to_string());
  }
  if let Some(list) = item
    .get("backup_url")
    .or_else(|| item.get("backupUrl"))
    .and_then(|value| value.as_array())
  {
    for value in list {
      if let Some(url) = value.as_str() {
        urls.push(url.to_string());
      }
    }
  }
  normalize_stream_urls(urls, block_pcdn)
}

async fn fetch_video_title(
  context: &DownloadContext,
  bvid: Option<&str>,
  aid: Option<&str>,
) -> Option<String> {
  let mut params = Vec::new();
  if let Some(bvid) = bvid {
    params.push(("bvid".to_string(), bvid.to_string()));
  }
  if let Some(aid) = aid {
    params.push(("aid".to_string(), aid.to_string()));
  }

  let auth = load_auth(context);
  let url = format!("{}/x/web-interface/view", context.bilibili.base_url());
  let data = context.bilibili.get_json(&url, &params, auth.as_ref(), false).await.ok()?;
  data
    .get("title")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string())
}

fn parse_video_id(url: &str) -> (Option<String>, Option<String>) {
  if let Some(bvid) = extract_bvid(url) {
    return (Some(bvid), None);
  }

  if let Some(aid) = extract_aid(url) {
    return (None, Some(aid));
  }

  (None, None)
}

fn extract_bvid(input: &str) -> Option<String> {
  if let Some(index) = input.find("BV") {
    let value = &input[index..];
    let end = value
      .find(|ch: char| !ch.is_ascii_alphanumeric())
      .unwrap_or(value.len());
    let bvid = &value[..end];
    if bvid.len() > 2 {
      return Some(bvid.to_string());
    }
  }
  None
}

fn extract_aid(input: &str) -> Option<String> {
  if let Some(index) = input.find("av") {
    let value = &input[index + 2..];
    let digits: String = value.chars().take_while(|ch| ch.is_ascii_digit()).collect();
    if !digits.is_empty() {
      return Some(digits);
    }
  }

  if input.chars().all(|ch| ch.is_ascii_digit()) {
    return Some(input.to_string());
  }

  None
}

fn build_ffmpeg_headers(context: &DownloadContext) -> Option<String> {
  let auth = load_auth(context)?;
  let mut headers = String::new();
  headers.push_str("Referer: https://www.bilibili.com\r\n");
  headers.push_str("User-Agent: Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36\r\n");
  headers.push_str(&format!("Cookie: {}\r\n", auth.cookie));
  Some(headers)
}

fn load_auth(context: &DownloadContext) -> Option<AuthInfo> {
  context.login_store.load_auth_info(&context.db).ok().flatten()
}

fn update_download_status(
  context: &DownloadContext,
  record_id: i64,
  status: i64,
  progress: i64,
) -> Result<(), String> {
  let now = now_rfc3339();
  context
    .db
    .with_conn(|conn| {
      conn.execute(
        "UPDATE video_download SET status = ?1, progress = ?2, update_time = ?3 WHERE id = ?4",
        (status, progress, &now, record_id),
      )?;
      Ok(())
    })
    .map_err(|err| format!("Failed to update download status: {}", err))
}

fn load_submission_status(context: &DownloadContext, task_id: &str) -> Result<String, String> {
  context
    .db
    .with_conn(|conn| {
      let mut stmt = conn.prepare("SELECT status FROM submission_task WHERE task_id = ?1")?;
      let status = stmt.query_row([task_id], |row| row.get(0))?;
      Ok(status)
    })
    .map_err(|err| err.to_string())
}

fn update_submission_status(
  context: &DownloadContext,
  task_id: &str,
  status: &str,
) -> Result<(), String> {
  let now = now_rfc3339();
  context
    .db
    .with_conn(|conn| {
      conn.execute(
        "UPDATE submission_task SET status = ?1, updated_at = ?2 WHERE task_id = ?3",
        (status, &now, task_id),
      )?;
      Ok(())
    })
    .map_err(|err| err.to_string())
}

fn update_relation_workflow_status(
  context: &DownloadContext,
  submission_task_id: &str,
  workflow_status: &str,
) -> Result<(), String> {
  let now = now_rfc3339();
  context
    .db
    .with_conn(|conn| {
      conn.execute(
        "UPDATE task_relations SET workflow_status = ?1, updated_at = ?2 WHERE submission_task_id = ?3",
        (workflow_status, &now, submission_task_id),
      )?;
      Ok(())
    })
    .map_err(|err| err.to_string())
}

fn load_workflow_instance_status(
  context: &DownloadContext,
  task_id: &str,
) -> Result<Option<String>, String> {
  context
    .db
    .with_conn(|conn| {
      let mut stmt = conn.prepare(
        "SELECT status FROM workflow_instances WHERE task_id = ?1 ORDER BY created_at DESC LIMIT 1",
      )?;
      let status: Option<String> = stmt.query_row([task_id], |row| row.get(0)).ok();
      Ok(status)
    })
    .map_err(|err| err.to_string())
}

async fn refresh_integration_status(
  context: &DownloadContext,
  record_id: i64,
) -> Result<(), String> {
  let submission_task_id = context
    .db
    .with_conn(|conn| {
      let mut stmt = conn.prepare(
        "SELECT submission_task_id FROM task_relations WHERE download_task_id = ?1 AND relation_type = 'INTEGRATED' LIMIT 1",
      )?;
      let value: Option<String> = stmt.query_row([record_id], |row| row.get(0)).ok();
      Ok(value)
    })
    .map_err(|err| err.to_string())?;

  let submission_task_id = match submission_task_id {
    Some(task_id) => task_id,
    None => return Ok(()),
  };

  let (total, completed, failed) = context
    .db
    .with_conn(|conn| {
      let mut stmt = conn.prepare(
        "SELECT \
          COUNT(*) AS total, \
          SUM(CASE WHEN vd.status = 2 THEN 1 ELSE 0 END) AS completed, \
          SUM(CASE WHEN vd.status = 3 THEN 1 ELSE 0 END) AS failed \
         FROM task_relations tr \
         JOIN video_download vd ON tr.download_task_id = vd.id \
         WHERE tr.submission_task_id = ?1 AND tr.relation_type = 'INTEGRATED'",
      )?;
      let row = stmt.query_row([&submission_task_id], |row| {
        let total: i64 = row.get(0)?;
        let completed: Option<i64> = row.get(1)?;
        let failed: Option<i64> = row.get(2)?;
        Ok((total, completed.unwrap_or(0), failed.unwrap_or(0)))
      })?;
      Ok(row)
    })
    .map_err(|err| err.to_string())?;

  if total == 0 {
    return Ok(());
  }

  if failed > 0 {
    let current_status = load_submission_status(context, &submission_task_id).unwrap_or_default();
    if current_status != "FAILED" && current_status != "COMPLETED" {
      let _ = update_submission_status(context, &submission_task_id, "FAILED");
    }
    let _ = update_relation_workflow_status(context, &submission_task_id, "DOWNLOAD_FAILED");
    return Ok(());
  }

  if completed == total {
    let _ = update_relation_workflow_status(context, &submission_task_id, "READY");
    let submission_status = load_submission_status(context, &submission_task_id)?;
    if submission_status == "FAILED" {
      return Ok(());
    }
    if let Some(status) = load_workflow_instance_status(context, &submission_task_id)? {
      if status == "RUNNING" || status == "COMPLETED" {
        return Ok(());
      }
    }
    let task_id = submission_task_id.clone();
    crate::commands::submission::start_submission_workflow(
      context.db.clone(),
      context.edit_upload_state.clone(),
      task_id,
    );
    let _ = update_relation_workflow_status(context, &submission_task_id, "WORKFLOW_STARTED");
  }

  Ok(())
}

fn count_active_downloads(context: &DownloadContext) -> Result<i64, String> {
  context
    .db
    .with_conn(|conn| {
      let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM video_download WHERE status IN (0, 1)",
        [],
        |row| row.get(0),
      )?;
      Ok(count)
    })
    .map_err(|err| format!("Failed to count downloads: {}", err))
}
