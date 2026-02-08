use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::Utc;
use reqwest::header::{HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use tauri::State;

use crate::api::ApiResponse;
use crate::login_store::AuthInfo;
use crate::utils::append_log;
use crate::AppState;

#[derive(Serialize)]
pub struct Partition {
  pub tid: i64,
  pub name: String,
}

#[derive(Serialize)]
pub struct Collection {
  pub season_id: i64,
  pub name: String,
  pub cover: Option<String>,
  pub description: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityTopic {
  pub topic_id: i64,
  pub mission_id: i64,
  pub name: String,
  pub description: Option<String>,
  pub activity_text: Option<String>,
  pub activity_description: Option<String>,
}

#[tauri::command]
pub async fn video_detail(
  state: State<'_, AppState>,
  bvid: Option<String>,
  aid: Option<i64>,
) -> Result<ApiResponse<Value>, String> {
  if bvid.is_none() && aid.is_none() {
    return Ok(ApiResponse::error("Missing bvid or aid"));
  }

  let mut params = Vec::new();
  if let Some(bvid) = bvid {
    params.push(("bvid".to_string(), bvid));
  }
  if let Some(aid) = aid {
    params.push(("aid".to_string(), aid.to_string()));
  }

  let auth = load_auth(&state);
  let url = format!("{}/x/web-interface/view", state.bilibili.base_url());
  match state.bilibili.get_json(&url, &params, auth.as_ref(), false).await {
    Ok(data) => Ok(ApiResponse::success(data)),
    Err(err) => Ok(ApiResponse::error(format!("Failed to load video detail: {}", err))),
  }
}

#[tauri::command]
pub async fn video_playurl(
  state: State<'_, AppState>,
  bvid: String,
  cid: String,
  qn: Option<String>,
  fnval: Option<String>,
  fnver: Option<String>,
  fourk: Option<String>,
) -> Result<ApiResponse<Value>, String> {
  let params = vec![
    ("bvid".to_string(), bvid),
    ("cid".to_string(), cid),
    ("qn".to_string(), qn.unwrap_or_else(|| "112".to_string())),
    ("fnval".to_string(), fnval.unwrap_or_else(|| "4048".to_string())),
    ("fnver".to_string(), fnver.unwrap_or_else(|| "0".to_string())),
    ("fourk".to_string(), fourk.unwrap_or_else(|| "1".to_string())),
  ];

  let auth = load_auth(&state);
  let url = format!("{}/x/player/wbi/playurl", state.bilibili.base_url());
  match state.bilibili.get_json(&url, &params, auth.as_ref(), true).await {
    Ok(data) => Ok(ApiResponse::success(data)),
    Err(err) => Ok(ApiResponse::error(format!("Failed to load playurl: {}", err))),
  }
}

#[tauri::command]
pub async fn video_playurl_by_aid(
  state: State<'_, AppState>,
  aid: String,
  cid: String,
  qn: Option<String>,
  fnval: Option<String>,
  fnver: Option<String>,
  fourk: Option<String>,
) -> Result<ApiResponse<Value>, String> {
  let params = vec![
    ("avid".to_string(), aid),
    ("cid".to_string(), cid),
    ("qn".to_string(), qn.unwrap_or_else(|| "112".to_string())),
    ("fnval".to_string(), fnval.unwrap_or_else(|| "4048".to_string())),
    ("fnver".to_string(), fnver.unwrap_or_else(|| "0".to_string())),
    ("fourk".to_string(), fourk.unwrap_or_else(|| "1".to_string())),
  ];

  let auth = load_auth(&state);
  let url = format!("{}/x/player/wbi/playurl", state.bilibili.base_url());
  match state.bilibili.get_json(&url, &params, auth.as_ref(), true).await {
    Ok(data) => Ok(ApiResponse::success(data)),
    Err(err) => Ok(ApiResponse::error(format!("Failed to load playurl: {}", err))),
  }
}

#[tauri::command]
pub async fn video_proxy_image(url: String) -> Result<ApiResponse<String>, String> {
  let trimmed = url.trim();
  if trimmed.is_empty() {
    return Ok(ApiResponse::error("图片地址不能为空"));
  }

  let mut headers = HeaderMap::new();
  headers.insert(
    USER_AGENT,
    HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"),
  );
  headers.insert(
    "Referer",
    HeaderValue::from_static("https://www.bilibili.com"),
  );

  let client = reqwest::Client::new();
  let response = match client.get(trimmed).headers(headers).send().await {
    Ok(response) => response,
    Err(err) => {
      return Ok(ApiResponse::error(format!("获取图片失败: {}", err)));
    }
  };

  if !response.status().is_success() {
    return Ok(ApiResponse::error(format!(
      "获取图片失败: {}",
      response.status()
    )));
  }

  let content_type = response
    .headers()
    .get(CONTENT_TYPE)
    .and_then(|value| value.to_str().ok())
    .unwrap_or("image/jpeg")
    .to_string();

  let bytes = match response.bytes().await {
    Ok(bytes) => bytes,
    Err(err) => {
      return Ok(ApiResponse::error(format!("读取图片失败: {}", err)));
    }
  };

  let encoded = STANDARD.encode(bytes);
  let data_url = format!("data:{};base64,{}", content_type, encoded);
  Ok(ApiResponse::success(data_url))
}

#[tauri::command]
pub async fn bilibili_collections(
  state: State<'_, AppState>,
  mid: i64,
) -> Result<ApiResponse<Vec<Collection>>, String> {
  let auth = load_auth(&state);
  append_log(
    &state.app_log_path,
    &format!("collections_start mid={} has_auth={}", mid, auth.is_some()),
  );
  if auth.is_none() {
    append_log(&state.app_log_path, &format!("collections_no_auth mid={}", mid));
    return Ok(ApiResponse::error("Login required"));
  }

  let params = vec![
    ("pn".to_string(), "1".to_string()),
    ("ps".to_string(), "100".to_string()),
    ("order".to_string(), "desc".to_string()),
    ("sort".to_string(), "mtime".to_string()),
    ("filter".to_string(), "1".to_string()),
  ];

  let url = "https://member.bilibili.com/x2/creative/web/seasons";
  let data = match state
    .bilibili
    .get_json(url, &params, auth.as_ref(), false)
    .await
  {
    Ok(data) => data,
    Err(err) => {
      append_log(
        &state.app_log_path,
        &format!("collections_api_error mid={} err={}", mid, err),
      );
      return Ok(ApiResponse::error(format!("Failed to load collections: {}", err)));
    }
  };

  let seasons = data.get("seasons").and_then(|value| value.as_array());
  let mut collections = Vec::new();
  if let Some(seasons) = seasons {
    for item in seasons {
      if let Some(season) = item.get("season") {
        if let Some(id) = season.get("id").and_then(|value| value.as_i64()) {
          collections.push(Collection {
            season_id: id,
            name: season
              .get("title")
              .and_then(|value| value.as_str())
              .unwrap_or_default()
              .to_string(),
            cover: season.get("cover").and_then(|value| value.as_str()).map(|value| value.to_string()),
            description: season
              .get("desc")
              .and_then(|value| value.as_str())
              .map(|value| value.to_string()),
          });
        }
      }
    }
  }
  append_log(
    &state.app_log_path,
    &format!("collections_ok mid={} count={}", mid, collections.len()),
  );

  Ok(ApiResponse::success(collections))
}

#[tauri::command]
pub async fn bilibili_partitions(
  state: State<'_, AppState>,
) -> Result<ApiResponse<Vec<Partition>>, String> {
  let auth = load_auth(&state);
  let params = vec![("t".to_string(), format!("{}", Utc::now().timestamp_millis()))];
  let url = "https://member.bilibili.com/x/vupre/web/archive/human/type2/list";

  let data = match state
    .bilibili
    .get_json(url, &params, auth.as_ref(), false)
    .await
  {
    Ok(data) => data,
    Err(_) => return Ok(ApiResponse::success(default_partitions())),
  };

  let list = data.get("type_list").and_then(|value| value.as_array());
  let mut partitions = Vec::new();
  if let Some(list) = list {
    for item in list {
      if let (Some(id), Some(name)) = (
        item.get("id").and_then(|value| value.as_i64()),
        item.get("name").and_then(|value| value.as_str()),
      ) {
        partitions.push(Partition {
          tid: id,
          name: name.to_string(),
        });
      }
    }
  }

  if partitions.is_empty() {
    Ok(ApiResponse::success(default_partitions()))
  } else {
    Ok(ApiResponse::success(partitions))
  }
}

#[tauri::command]
pub async fn bilibili_topics(
  state: State<'_, AppState>,
  partition_id: Option<i64>,
) -> Result<ApiResponse<Vec<ActivityTopic>>, String> {
  let auth = load_auth(&state);
  if auth.is_none() {
    return Ok(ApiResponse::error("Login required"));
  }

  let url = "https://member.bilibili.com/x/vupre/web/topic/type";
  let timestamp = Utc::now().timestamp_millis();
  let mut page = 0;
  let page_size = 50;
  let mut max_page = 1;
  let mut topics = Vec::new();
  let mut seen = HashSet::new();

  loop {
    let mut params = vec![
      ("pn".to_string(), page.to_string()),
      ("ps".to_string(), page_size.to_string()),
      ("t".to_string(), timestamp.to_string()),
    ];
    if let Some(partition_id) = partition_id {
      if partition_id > 0 {
        params.push(("type_id".to_string(), partition_id.to_string()));
      }
    }

    let data = match state
      .bilibili
      .get_json(url, &params, auth.as_ref(), false)
      .await
    {
      Ok(data) => data,
      Err(err) => {
        return Ok(ApiResponse::error(format!("Failed to load topics: {}", err)));
      }
    };

    let next_max_page = data
      .get("maxpage")
      .and_then(|value| value.as_i64())
      .unwrap_or(page + 1);
    max_page = max_page.max(next_max_page);

    if let Some(list) = data.get("topics").and_then(|value| value.as_array()) {
      for item in list {
        let topic_id = item
          .get("topic_id")
          .and_then(|value| value.as_i64())
          .unwrap_or(0);
        if topic_id <= 0 || !seen.insert(topic_id) {
          continue;
        }
        let mission_id = item
          .get("mission_id")
          .and_then(|value| value.as_i64())
          .unwrap_or(0);
        let name = item
          .get("topic_name")
          .and_then(|value| value.as_str())
          .unwrap_or_default()
          .to_string();
        let description = item
          .get("description")
          .and_then(|value| value.as_str())
          .map(|value| value.to_string());
        let activity_text = item
          .get("activity_text")
          .and_then(|value| value.as_str())
          .map(|value| value.to_string());
        let activity_description = item
          .get("activity_description")
          .and_then(|value| value.as_str())
          .map(|value| value.to_string());

        if mission_id <= 0 && activity_text.as_deref().unwrap_or("").is_empty() {
          continue;
        }

        topics.push(ActivityTopic {
          topic_id,
          mission_id,
          name,
          description,
          activity_text,
          activity_description,
        });
      }
    }

    if page + 1 >= max_page {
      break;
    }
    page += 1;
  }

  Ok(ApiResponse::success(topics))
}

fn default_partitions() -> Vec<Partition> {
  vec![
    Partition {
      tid: 1,
      name: "Animation".to_string(),
    },
    Partition {
      tid: 4,
      name: "Game".to_string(),
    },
    Partition {
      tid: 36,
      name: "Knowledge".to_string(),
    },
    Partition {
      tid: 188,
      name: "Technology".to_string(),
    },
  ]
}

fn load_auth(state: &State<'_, AppState>) -> Option<AuthInfo> {
  state.login_store.load_auth_info(&state.db).ok().flatten()
}
