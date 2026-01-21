use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
  atomic::{AtomicBool, Ordering},
  Arc, Mutex,
};
use std::time::{Duration, Instant};

use chrono::Utc;
use reqwest::blocking::Client;
use reqwest::header::{HeaderValue, REFERER, USER_AGENT};
use rusqlite::OptionalExtension;
use serde_json::Value;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::tungstenite::Message;
use futures_util::{SinkExt, StreamExt};

use crate::bilibili::client::BilibiliClient;
use crate::commands::settings::{
  load_download_settings_from_db, load_live_settings_from_db, LiveSettings,
};
use crate::config::default_download_dir;
use crate::db::Db;
use crate::ffmpeg::run_ffmpeg;
use crate::login_store::{AuthInfo, LoginStore};
use crate::utils::{append_log, now_rfc3339, sanitize_filename};

pub struct LiveRuntime {
  records: Mutex<HashMap<String, LiveRecordHandle>>,
}

pub struct LiveRecordHandle {
  pub stop_flag: Arc<AtomicBool>,
  pub split_flag: Arc<AtomicBool>,
  pub last_title: Arc<Mutex<String>>,
  pub current_file: Arc<Mutex<String>>,
  pub start_time: String,
  pub start_date: String,
}

pub struct LiveRecordInfo {
  pub file_path: String,
  pub start_time: String,
}

#[derive(Clone)]
pub struct LiveContext {
  pub db: Arc<Db>,
  pub bilibili: Arc<BilibiliClient>,
  pub login_store: Arc<LoginStore>,
  pub app_log_path: Arc<PathBuf>,
  pub live_runtime: Arc<LiveRuntime>,
}

#[derive(Clone)]
pub struct LiveRoomInfo {
  pub room_id: String,
  pub uid: String,
  pub live_status: i64,
  pub title: String,
  pub cover: Option<String>,
  pub area_name: Option<String>,
  pub parent_area_name: Option<String>,
}

pub fn new_live_runtime() -> LiveRuntime {
  LiveRuntime {
    records: Mutex::new(HashMap::new()),
  }
}

impl LiveRuntime {
  pub fn is_recording(&self, room_id: &str) -> bool {
    self.records.lock().map(|map| map.contains_key(room_id)).unwrap_or(false)
  }

  pub fn get_record_info(&self, room_id: &str) -> Option<LiveRecordInfo> {
    let map = self.records.lock().ok()?;
    let handle = map.get(room_id)?;
    let file_path = handle.current_file.lock().ok()?.clone();
    Some(LiveRecordInfo {
      file_path,
      start_time: handle.start_time.clone(),
    })
  }

  pub fn mark_split(&self, room_id: &str) {
    if let Ok(map) = self.records.lock() {
      if let Some(handle) = map.get(room_id) {
        handle.split_flag.store(true, Ordering::SeqCst);
      }
    }
  }

  pub fn stop(&self, room_id: &str) {
    if let Ok(map) = self.records.lock() {
      if let Some(handle) = map.get(room_id) {
        handle.stop_flag.store(true, Ordering::SeqCst);
      }
    }
  }
}

pub fn start_auto_record_loop(context: LiveContext) {
  tauri::async_runtime::spawn(async move {
    loop {
      let settings = load_live_settings_from_db(&context.db)
        .unwrap_or_else(|_| crate::commands::settings::default_live_settings());
      let interval_sec = settings.check_interval_sec.max(10);
      if let Ok(rooms) = load_anchor_room_ids(&context.db) {
        for room_id in rooms {
          match fetch_room_info(&context.bilibili, &room_id).await {
            Ok(info) => {
              let _ = update_anchor_status(&context.db, &room_id, info.live_status);
              let auto_record = load_room_auto_record(&context.db, &room_id).unwrap_or(true);
              let recording = context.live_runtime.is_recording(&room_id);
              if info.live_status == 1 && auto_record && !recording {
                match start_recording(context.clone(), &room_id, info.clone(), settings.clone()) {
                  Ok(()) => {
                    append_log(&context.app_log_path, &format!("auto_record_start room={}", room_id));
                  }
                  Err(err) => {
                    append_log(
                      &context.app_log_path,
                      &format!("auto_record_start_failed room={} err={}", room_id, err),
                    );
                  }
                }
              } else if info.live_status != 1 && recording {
                stop_recording(context.clone(), &room_id, "直播结束自动停止");
              }
              if recording && settings.cutting_by_title {
                if let Ok(mut map) = context.live_runtime.records.lock() {
                  if let Some(handle) = map.get_mut(&room_id) {
                    let mut last_title = handle.last_title.lock().unwrap_or_else(|e| e.into_inner());
                    if *last_title != info.title {
                      *last_title = info.title.clone();
                      handle.split_flag.store(true, Ordering::SeqCst);
                    }
                  }
                }
              }
            }
            Err(err) => {
              append_log(&context.app_log_path, &format!("live_check_error room={} err={}", room_id, err));
            }
          }
        }
      }
      tokio::time::sleep(Duration::from_secs(interval_sec as u64)).await;
    }
  });
}

pub fn start_recording(
  context: LiveContext,
  room_id: &str,
  room_info: LiveRoomInfo,
  settings: LiveSettings,
) -> Result<(), String> {
  if context.live_runtime.is_recording(room_id) {
    return Ok(());
  }

  if room_info.live_status != 1 {
    return Err("当前未开播".to_string());
  }

  let nickname = load_anchor_nickname(&context.db, room_id).ok().flatten();
  let stop_flag = Arc::new(AtomicBool::new(false));
  let split_flag = Arc::new(AtomicBool::new(false));
  let current_title = room_info.title.clone();
  let start_time = Utc::now();
  let handle = LiveRecordHandle {
    stop_flag: Arc::clone(&stop_flag),
    split_flag: Arc::clone(&split_flag),
    last_title: Arc::new(Mutex::new(current_title)),
    current_file: Arc::new(Mutex::new(String::new())),
    start_time: start_time.to_rfc3339(),
    start_date: start_time.format("%Y%m%d").to_string(),
  };

  if let Ok(mut map) = context.live_runtime.records.lock() {
    map.insert(room_id.to_string(), handle);
  }

  let runtime = Arc::clone(&context.live_runtime);
  let room_id_owned = room_id.to_string();
  tauri::async_runtime::spawn_blocking(move || {
    let result = run_record_loop(context.clone(), room_id_owned.clone(), room_info, nickname, settings);
    if let Err(err) = result {
      append_log(&context.app_log_path, &format!("record_loop_error room={} err={}", room_id_owned, err));
    }
    if let Ok(mut map) = runtime.records.lock() {
      map.remove(&room_id_owned);
    }
  });

  Ok(())
}

pub fn stop_recording(context: LiveContext, room_id: &str, reason: &str) {
  append_log(
    &context.app_log_path,
    &format!("record_stop room={} reason={}", room_id, reason),
  );
  context.live_runtime.stop(room_id);
}

fn run_record_loop(
  context: LiveContext,
  room_id: String,
  room_info: LiveRoomInfo,
  nickname: Option<String>,
  settings: LiveSettings,
) -> Result<(), String> {
  let mut settings = settings;
  if settings.record_mode == 1 {
    settings.write_metadata = false;
    settings.flv_fix_split_on_missing = false;
  }
  let base_dir = if settings.record_path.trim().is_empty() {
    let download_dir = load_download_settings_from_db(&context.db)
      .map(|settings| settings.download_path)
      .unwrap_or_else(|_| default_download_dir().to_string_lossy().to_string());
    PathBuf::from(download_dir).join("live_recordings")
  } else {
    PathBuf::from(settings.record_path.trim())
  };
  let _ = std::fs::create_dir_all(&base_dir);

  let stop_flag = {
    let map = context.live_runtime.records.lock().map_err(|_| "Lock error")?;
    map.get(&room_id)
      .map(|handle| Arc::clone(&handle.stop_flag))
      .ok_or_else(|| "Record handle missing".to_string())?
  };
  let split_flag = {
    let map = context.live_runtime.records.lock().map_err(|_| "Lock error")?;
    map.get(&room_id)
      .map(|handle| Arc::clone(&handle.split_flag))
      .ok_or_else(|| "Record handle missing".to_string())?
  };

  let mut segment_index = 1;
  let mut current_title = room_info.title.clone();
  let record_start_date = load_record_start_date(&context, &room_id);
  let mut current_file_path = build_record_path(
    &settings.file_name_template,
    &base_dir,
    &room_info,
    nickname.as_deref(),
    &record_start_date,
    segment_index,
  );
  update_current_file(&context, &room_id, &current_file_path);

  if settings.save_cover {
    if let Some(cover) = room_info.cover.as_ref() {
      let _ = download_cover(&current_file_path, cover);
    }
  }

  if should_record_danmaku(&settings) {
    let danmaku_settings = settings.clone();
    let danmaku_context = context.clone();
    let runtime_room = room_id.clone();
    let danmaku_room = room_info.room_id.clone();
    let danmaku_file = current_file_path.clone();
    let danmaku_stop = Arc::clone(&stop_flag);
    tauri::async_runtime::spawn(async move {
      let _ = run_danmaku_loop(
        danmaku_context,
        runtime_room,
        danmaku_room,
        danmaku_file,
        danmaku_settings,
        danmaku_stop,
      )
      .await;
    });
  }

  let client = Client::builder()
    .timeout(Duration::from_millis(settings.stream_connect_timeout_ms.max(1000) as u64))
    .build()
    .map_err(|err| format!("Failed to build client: {}", err))?;
  let auth = context.login_store.load_auth_info(&context.db).ok().flatten();

  loop {
    if stop_flag.load(Ordering::SeqCst) {
      break;
    }

    let stream_url = match fetch_stream_url(
      &context.bilibili,
      &room_info.room_id,
      &settings,
      auth.as_ref(),
      true,
    ) {
      Ok(url) => url,
      Err(err) => {
        append_log(&context.app_log_path, &format!("fetch_stream_url_error room={} err={}", room_id, err));
        if settings.stream_retry_no_qn_sec > 0 {
          std::thread::sleep(Duration::from_secs(settings.stream_retry_no_qn_sec.max(1) as u64));
          match fetch_stream_url(
            &context.bilibili,
            &room_info.room_id,
            &settings,
            auth.as_ref(),
            false,
          ) {
            Ok(url) => url,
            Err(err) => {
              append_log(&context.app_log_path, &format!("fetch_stream_url_fallback_error room={} err={}", room_id, err));
              std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
              continue;
            }
          }
        } else {
          std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
          continue;
        }
      }
    };

    let referer_value = format!("https://live.bilibili.com/{}", room_info.room_id);
    let mut request = client.get(&stream_url);
    request = request.header(
      USER_AGENT,
      HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"),
    );
    if let Ok(value) = HeaderValue::from_str(&referer_value) {
      request = request.header(REFERER, value);
    }
    if let Some(auth) = auth.as_ref() {
      if let Ok(value) = HeaderValue::from_str(&auth.cookie) {
        request = request.header("Cookie", value);
      }
    }
    let response = request.send();
    let mut response = match response {
      Ok(resp) => resp,
      Err(err) => {
        append_log(&context.app_log_path, &format!("stream_connect_error room={} err={}", room_id, err));
        std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
        continue;
      }
    };

    if !response.status().is_success() {
      append_log(
        &context.app_log_path,
        &format!(
          "stream_response_error room={} status={}",
          room_id,
          response.status().as_u16()
        ),
      );
      std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
      continue;
    }

    let mut buf = vec![0u8; 8192];
    let mut parser = FlvStreamParser::new();
    let mut cache = FlvHeaderCache::new();
    let mut segment: Option<SegmentWriter> = None;
    let mut segment_start = Instant::now();

    loop {
      if stop_flag.load(Ordering::SeqCst) {
        if let Some(mut seg) = segment.take() {
          let record_id = seg.record_id;
          let file_path = seg.file_path.clone();
          seg.finish("STOPPED", None)?;
          drop(seg);
          spawn_segment_remux(context.clone(), record_id, file_path);
        }
        return Ok(());
      }

      match response.read(&mut buf) {
        Ok(0) => {
          if settings.flv_fix_split_on_missing && !settings.flv_fix_disable_on_annexb {
            if let Some(mut seg) = segment.take() {
              let record_id = seg.record_id;
              let file_path = seg.file_path.clone();
              seg.finish("COMPLETED", Some("网络中断自动分段"))?;
              drop(seg);
              spawn_segment_remux(context.clone(), record_id, file_path);
            }
            segment_index += 1;
            current_title = load_current_title(&context, &room_id, &current_title);
            current_file_path = build_record_path(
              &settings.file_name_template,
              &base_dir,
              &room_info,
              nickname.as_deref(),
              &record_start_date,
              segment_index,
            );
            update_current_file(&context, &room_id, &current_file_path);
            break;
          } else {
            if let Some(mut seg) = segment.take() {
              seg.finish("FAILED", Some("网络中断"))?;
            }
            segment_index += 1;
            current_title = load_current_title(&context, &room_id, &current_title);
            current_file_path = build_record_path(
              &settings.file_name_template,
              &base_dir,
              &room_info,
              nickname.as_deref(),
              &record_start_date,
              segment_index,
            );
            update_current_file(&context, &room_id, &current_file_path);
            std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
            break;
          }
        }
        Ok(n) => {
          let items = match parser.push(&buf[..n]) {
            Ok(items) => items,
            Err(err) => {
              append_log(
                &context.app_log_path,
                &format!("stream_invalid_header room={} err={}", room_id, err),
              );
              std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
              break;
            }
          };

          for item in items {
            match item {
              FlvParsedItem::Header(header) => {
                cache.set_header(header.clone());
                if segment.is_none() {
                  let mut new_segment = open_segment(
                    &context,
                    &room_id,
                    &current_file_path,
                    &current_title,
                    segment_index,
                    &settings,
                    &room_info,
                    nickname.as_deref(),
                  )?;
                  cache.write_preamble(&mut new_segment)?;
                  segment_start = Instant::now();
                  segment = Some(new_segment);
                }
              }
              FlvParsedItem::Tag(tag) => {
                cache.update_from_tag(&tag);
                if split_flag.load(Ordering::SeqCst) {
                  if cache.has_header() {
                    split_flag.store(false, Ordering::SeqCst);
                    if let Some(mut seg) = segment.take() {
                      let record_id = seg.record_id;
                      let file_path = seg.file_path.clone();
                      seg.finish("COMPLETED", Some("分段切换"))?;
                      drop(seg);
                      spawn_segment_remux(context.clone(), record_id, file_path);
                    }
                    segment_index += 1;
                    current_title = load_current_title(&context, &room_id, &current_title);
                    current_file_path = build_record_path(
                      &settings.file_name_template,
                      &base_dir,
                      &room_info,
                      nickname.as_deref(),
                      &record_start_date,
                      segment_index,
                    );
                    update_current_file(&context, &room_id, &current_file_path);
                    let mut new_segment = open_segment(
                      &context,
                      &room_id,
                      &current_file_path,
                      &current_title,
                      segment_index,
                      &settings,
                      &room_info,
                      nickname.as_deref(),
                    )?;
                    cache.write_preamble(&mut new_segment)?;
                    segment_start = Instant::now();
                    segment = Some(new_segment);
                  } else {
                    append_log(
                      &context.app_log_path,
                      &format!("stream_split_skip room={} reason=no_header", room_id),
                    );
                  }
                }

                if let Some(seg) = segment.as_mut() {
                  seg.write(&tag.bytes)?;
                  if settings.cutting_mode == 1 {
                    let limit = settings.cutting_number.max(1) as u64;
                    if segment_start.elapsed().as_secs() >= limit {
                      split_flag.store(true, Ordering::SeqCst);
                    }
                  } else if settings.cutting_mode == 2 {
                    let limit = settings.cutting_number.max(1) as u64 * 1024 * 1024;
                    if seg.bytes_written >= limit {
                      split_flag.store(true, Ordering::SeqCst);
                    }
                  }
                }
              }
            }
          }
        }
        Err(err) => {
          append_log(&context.app_log_path, &format!("stream_read_error room={} err={}", room_id, err));
          if settings.flv_fix_split_on_missing && !settings.flv_fix_disable_on_annexb {
            if let Some(mut seg) = segment.take() {
              let record_id = seg.record_id;
              let file_path = seg.file_path.clone();
              seg.finish("COMPLETED", Some("读取异常自动分段"))?;
              drop(seg);
              spawn_segment_remux(context.clone(), record_id, file_path);
            }
            segment_index += 1;
            current_title = load_current_title(&context, &room_id, &current_title);
            current_file_path = build_record_path(
              &settings.file_name_template,
              &base_dir,
              &room_info,
              nickname.as_deref(),
              &record_start_date,
              segment_index,
            );
            update_current_file(&context, &room_id, &current_file_path);
            break;
          } else {
            if let Some(mut seg) = segment.take() {
              seg.finish("FAILED", Some("读取异常"))?;
            }
            segment_index += 1;
            current_title = load_current_title(&context, &room_id, &current_title);
            current_file_path = build_record_path(
              &settings.file_name_template,
              &base_dir,
              &room_info,
              nickname.as_deref(),
              &record_start_date,
              segment_index,
            );
            update_current_file(&context, &room_id, &current_file_path);
            std::thread::sleep(Duration::from_millis(settings.stream_retry_ms.max(1000) as u64));
            break;
          }
        }
      }
    }
  }

  Ok(())
}

struct FlvTag {
  tag_type: u8,
  bytes: Vec<u8>,
  data_offset: usize,
  data_len: usize,
}

impl FlvTag {
  fn data(&self) -> &[u8] {
    &self.bytes[self.data_offset..self.data_offset + self.data_len]
  }
}

enum FlvParsedItem {
  Header(Vec<u8>),
  Tag(FlvTag),
}

struct FlvStreamParser {
  buffer: Vec<u8>,
  header_parsed: bool,
}

impl FlvStreamParser {
  fn new() -> Self {
    Self {
      buffer: Vec::new(),
      header_parsed: false,
    }
  }

  fn push(&mut self, data: &[u8]) -> Result<Vec<FlvParsedItem>, String> {
    if !data.is_empty() {
      self.buffer.extend_from_slice(data);
    }
    let mut items = Vec::new();
    let mut offset = 0;
    if !self.header_parsed {
      if self.buffer.len() < 3 {
        return Ok(items);
      }
      if self.buffer[..3] != *b"FLV" {
        return Err("FLV header mismatch".to_string());
      }
      if self.buffer.len() < 13 {
        return Ok(items);
      }
      let header = self.buffer[offset..offset + 13].to_vec();
      offset += 13;
      self.header_parsed = true;
      items.push(FlvParsedItem::Header(header));
    }

    loop {
      if self.buffer.len().saturating_sub(offset) < 11 {
        break;
      }
      let header_start = offset;
      let data_size = read_u24_be(&self.buffer[header_start + 1..header_start + 4]);
      let total = 11 + data_size + 4;
      if self.buffer.len().saturating_sub(offset) < total {
        break;
      }
      let bytes = self.buffer[offset..offset + total].to_vec();
      let tag_type = bytes[0];
      let data_offset = 11;
      let data_len = data_size;
      items.push(FlvParsedItem::Tag(FlvTag {
        tag_type,
        bytes,
        data_offset,
        data_len,
      }));
      offset += total;
    }

    if offset > 0 {
      self.buffer.drain(0..offset);
    }
    Ok(items)
  }
}

struct FlvHeaderCache {
  header: Option<Vec<u8>>,
  script_tag: Option<Vec<u8>>,
  audio_header: Option<Vec<u8>>,
  video_header: Option<Vec<u8>>,
}

impl FlvHeaderCache {
  fn new() -> Self {
    Self {
      header: None,
      script_tag: None,
      audio_header: None,
      video_header: None,
    }
  }

  fn set_header(&mut self, header: Vec<u8>) {
    self.header = Some(header);
  }

  fn has_header(&self) -> bool {
    self.header.is_some()
  }

  fn update_from_tag(&mut self, tag: &FlvTag) {
    match tag.tag_type {
      18 => {
        if self.script_tag.is_none() {
          self.script_tag = Some(normalize_header_tag(&tag.bytes));
        }
      }
      8 => {
        if is_audio_header(tag.data(), self.audio_header.is_some()) {
          self.audio_header = Some(normalize_header_tag(&tag.bytes));
        }
      }
      9 => {
        if is_video_header(tag.data(), self.video_header.is_some()) {
          self.video_header = Some(normalize_header_tag(&tag.bytes));
        }
      }
      _ => {}
    }
  }

  fn write_preamble(&self, segment: &mut SegmentWriter) -> Result<(), String> {
    let header = self
      .header
      .as_ref()
      .ok_or_else(|| "缺少FLV头信息".to_string())?;
    segment.write(header)?;
    if let Some(tag) = self.script_tag.as_ref() {
      segment.write(tag)?;
    }
    if let Some(tag) = self.video_header.as_ref() {
      segment.write(tag)?;
    }
    if let Some(tag) = self.audio_header.as_ref() {
      segment.write(tag)?;
    }
    Ok(())
  }
}

fn read_u24_be(slice: &[u8]) -> usize {
  if slice.len() < 3 {
    return 0;
  }
  ((slice[0] as usize) << 16) | ((slice[1] as usize) << 8) | slice[2] as usize
}

fn normalize_header_tag(tag: &[u8]) -> Vec<u8> {
  let mut normalized = tag.to_vec();
  if normalized.len() >= 11 {
    normalized[4] = 0;
    normalized[5] = 0;
    normalized[6] = 0;
    normalized[7] = 0;
  }
  normalized
}

fn is_audio_header(data: &[u8], has_header: bool) -> bool {
  if data.len() < 2 {
    return false;
  }
  let sound_format = data[0] >> 4;
  if sound_format == 10 {
    data[1] == 0
  } else {
    !has_header
  }
}

fn is_video_header(data: &[u8], has_header: bool) -> bool {
  if data.len() < 2 {
    return false;
  }
  let codec_id = data[0] & 0x0f;
  let packet_type = data[1];
  if codec_id == 7 || codec_id == 12 {
    packet_type == 0
  } else {
    !has_header
  }
}

struct SegmentWriter {
  db: Arc<Db>,
  record_id: i64,
  file_path: String,
  file: File,
  bytes_written: u64,
  title: String,
  metadata_path: Option<String>,
}

impl SegmentWriter {
  fn write(&mut self, buf: &[u8]) -> Result<(), String> {
    self.file.write_all(buf).map_err(|err| format!("写入失败: {}", err))?;
    self.bytes_written += buf.len() as u64;
    Ok(())
  }

  fn finish(&mut self, status: &str, error: Option<&str>) -> Result<(), String> {
    let end_time = now_rfc3339();
    update_record_task(
      &self.db,
      self.record_id,
      status,
      Some(end_time.clone()),
      self.bytes_written,
      error,
    )?;
    if let Some(path) = self.metadata_path.as_ref() {
      update_metadata_file(path, &end_time, self.bytes_written)?;
    }
    Ok(())
  }
}

fn open_segment(
  context: &LiveContext,
  room_id: &str,
  file_path: &str,
  title: &str,
  segment_index: i64,
  settings: &LiveSettings,
  room_info: &LiveRoomInfo,
  nickname: Option<&str>,
) -> Result<SegmentWriter, String> {
  if let Some(parent) = Path::new(file_path).parent() {
    std::fs::create_dir_all(parent).map_err(|err| format!("创建目录失败: {}", err))?;
  }

  let file = OpenOptions::new()
    .create(true)
    .write(true)
    .truncate(true)
    .open(file_path)
    .map_err(|err| format!("创建文件失败: {}", err))?;

  let record_id = insert_record_task(&context.db, room_id, file_path, segment_index, title)?;
  let metadata_path = if settings.write_metadata {
    Some(write_metadata_file(file_path, room_info, nickname, title)?)
  } else {
    None
  };
  Ok(SegmentWriter {
    db: Arc::clone(&context.db),
    record_id,
    file_path: file_path.to_string(),
    file,
    bytes_written: 0,
    title: title.to_string(),
    metadata_path,
  })
}

fn spawn_segment_remux(context: LiveContext, record_id: i64, file_path: String) {
  let source_path = PathBuf::from(file_path);
  let ext = source_path
    .extension()
    .and_then(|value| value.to_str())
    .unwrap_or("")
    .to_string();
  if !ext.eq_ignore_ascii_case("flv") {
    return;
  }
  let target_path = source_path.with_extension("mp4");
  let source = source_path.to_string_lossy().to_string();
  let target = target_path.to_string_lossy().to_string();
  let log_path = context.app_log_path.clone();
  let db = context.db.clone();
  tauri::async_runtime::spawn(async move {
    append_log(
      log_path.as_ref(),
      &format!("live_remux_start record_id={} source={} target={}", record_id, source, target),
    );
    let args = vec![
      "-hide_banner".to_string(),
      "-loglevel".to_string(),
      "error".to_string(),
      "-y".to_string(),
      "-i".to_string(),
      source.clone(),
      "-c".to_string(),
      "copy".to_string(),
      target.clone(),
    ];
    let result = tauri::async_runtime::spawn_blocking(move || run_ffmpeg(&args))
      .await
      .map_err(|_| "转封装执行失败".to_string());
    match result {
      Ok(Ok(())) => {
        let file_size = std::fs::metadata(&target)
          .map(|meta| meta.len())
          .unwrap_or(0);
        if let Err(err) = update_record_task_file_path(&db, record_id, &target, file_size) {
          append_log(
            log_path.as_ref(),
            &format!("live_remux_update_fail record_id={} err={}", record_id, err),
          );
        }
        append_log(
          log_path.as_ref(),
          &format!("live_remux_done record_id={} status=ok", record_id),
        );
      }
      Ok(Err(err)) => {
        append_log(
          log_path.as_ref(),
          &format!("live_remux_done record_id={} status=err err={}", record_id, err),
        );
      }
      Err(err) => {
        append_log(
          log_path.as_ref(),
          &format!("live_remux_done record_id={} status=err err={}", record_id, err),
        );
      }
    }
  });
}

fn insert_record_task(
  db: &Db,
  room_id: &str,
  file_path: &str,
  segment_index: i64,
  title: &str,
) -> Result<i64, String> {
  let now = now_rfc3339();
  db.with_conn(|conn| {
    conn.execute(
      "INSERT INTO live_record_task (room_id, status, file_path, segment_index, start_time, title, create_time, update_time) \
       VALUES (?1, 'RECORDING', ?2, ?3, ?4, ?5, ?6, ?7)",
      (room_id, file_path, segment_index, &now, title, &now, &now),
    )?;
    Ok(conn.last_insert_rowid())
  })
  .map_err(|err| format!("写入录制任务失败: {}", err))
}

fn update_record_task(
  db: &Db,
  record_id: i64,
  status: &str,
  end_time: Option<String>,
  file_size: u64,
  error: Option<&str>,
) -> Result<(), String> {
  let now = now_rfc3339();
  let end_time_value = end_time.unwrap_or_else(|| now.clone());
  let error_message = error.map(|value| value.to_string());
  db.with_conn(|conn| {
    conn.execute(
      "UPDATE live_record_task SET status = ?1, end_time = ?2, file_size = ?3, error_message = ?4, update_time = ?5 WHERE id = ?6",
      (status, &end_time_value, file_size as i64, error_message, &now, record_id),
    )?;
    Ok(())
  })
  .map_err(|err| format!("更新录制任务失败: {}", err))
}

fn update_record_task_file_path(
  db: &Db,
  record_id: i64,
  file_path: &str,
  file_size: u64,
) -> Result<(), String> {
  let now = now_rfc3339();
  db.with_conn(|conn| {
    conn.execute(
      "UPDATE live_record_task SET file_path = ?1, file_size = ?2, update_time = ?3 WHERE id = ?4",
      (file_path, file_size as i64, &now, record_id),
    )?;
    Ok(())
  })
  .map_err(|err| format!("更新录播路径失败: {}", err))
}

fn load_anchor_room_ids(db: &Db) -> Result<Vec<String>, String> {
  db.with_conn(|conn| {
    let mut stmt = conn.prepare("SELECT uid FROM anchor ORDER BY id DESC")?;
    let rows = stmt
      .query_map([], |row| row.get(0))?
      .collect::<Result<Vec<String>, _>>()?;
    Ok(rows)
  })
  .map_err(|err| err.to_string())
}

fn load_anchor_nickname(db: &Db, room_id: &str) -> Result<Option<String>, String> {
  db.with_conn(|conn| {
    conn
      .query_row(
        "SELECT nickname FROM anchor WHERE uid = ?1",
        [room_id],
        |row| row.get(0),
      )
      .optional()
  })
  .map_err(|err| err.to_string())
}

fn load_room_auto_record(db: &Db, room_id: &str) -> Result<bool, String> {
  db.with_conn(|conn| {
    conn
      .query_row(
        "SELECT auto_record FROM live_room_settings WHERE room_id = ?1",
        [room_id],
        |row| row.get::<_, i64>(0),
      )
      .map(|value| value != 0)
      .or(Ok(true))
  })
  .map_err(|err| err.to_string())
}

fn update_anchor_status(db: &Db, room_id: &str, live_status: i64) -> Result<(), String> {
  let now = now_rfc3339();
  db.with_conn(|conn| {
    conn.execute(
      "UPDATE anchor SET live_status = ?1, last_check_time = ?2, update_time = ?3 WHERE uid = ?4",
      (live_status, &now, &now, room_id),
    )?;
    Ok(())
  })
  .map_err(|err| err.to_string())
}

fn update_current_file(context: &LiveContext, room_id: &str, file_path: &str) {
  if let Ok(map) = context.live_runtime.records.lock() {
    if let Some(handle) = map.get(room_id) {
      if let Ok(mut path) = handle.current_file.lock() {
        *path = file_path.to_string();
      }
    }
  }
}

fn load_current_title(context: &LiveContext, room_id: &str, fallback: &str) -> String {
  if let Ok(map) = context.live_runtime.records.lock() {
    if let Some(handle) = map.get(room_id) {
      if let Ok(title) = handle.last_title.lock() {
        return title.clone();
      }
    }
  }
  fallback.to_string()
}

fn load_record_start_date(context: &LiveContext, room_id: &str) -> String {
  if let Ok(map) = context.live_runtime.records.lock() {
    if let Some(handle) = map.get(room_id) {
      if !handle.start_date.trim().is_empty() {
        return handle.start_date.clone();
      }
    }
  }
  Utc::now().format("%Y%m%d").to_string()
}

fn build_record_path(
  template: &str,
  base_dir: &Path,
  info: &LiveRoomInfo,
  nickname: Option<&str>,
  record_start_date: &str,
  segment_index: i64,
) -> String {
  let now = Utc::now();
  let now_str = now.format("%Y%m%d-%H%M%S").to_string();
  let date_str = now.format("%Y%m%d").to_string();
  let time_str = now.format("%H%M%S").to_string();
  let ms_str = format!("{:03}", now.timestamp_subsec_millis());
  let mut output = template.to_string();
  output = output.replace("{{ roomId }}", &info.room_id);
  output = output.replace("{{ uid }}", &info.uid);
  output = output.replace("{{ name }}", nickname.unwrap_or("主播"));
  output = output.replace("{{ title }}", &info.title);
  output = output.replace("{{ now }}", &now_str);
  output = output.replace("{{ date }}", &date_str);
  output = output.replace("{{ liveDate }}", record_start_date);
  output = output.replace("{{ live_date }}", record_start_date);
  output = output.replace("{{ time }}", &time_str);
  output = output.replace("{{ ms }}", &ms_str);
  output = output.replace(
    "{{ \"now\" | format_date: \"yyyyMMdd-HHmmss-fff\" }}",
    &format!("{}-{}", now.format("%Y%m%d-%H%M%S"), ms_str),
  );

  let relative = sanitize_path(&output);
  let mut path = if Path::new(&relative).is_absolute() {
    PathBuf::from(relative)
  } else {
    base_dir.join(relative)
  };

  if path.extension().is_none() {
    path.set_extension("flv");
  }

  if segment_index > 1 {
    let stem = path
      .file_stem()
      .and_then(|value| value.to_str())
      .unwrap_or("record")
      .to_string();
    let ext = path.extension().and_then(|value| value.to_str()).unwrap_or("flv");
    path.set_file_name(format!("{}_part{}.{}", stem, segment_index, ext));
  }

  path.to_string_lossy().to_string()
}

fn sanitize_path(path: &str) -> String {
  let mut parts = Vec::new();
  for part in path.split(['/', '\\']) {
    if part.is_empty() {
      continue;
    }
    parts.push(sanitize_filename(part));
  }
  parts.join(std::path::MAIN_SEPARATOR_STR)
}

fn write_metadata_file(
  file_path: &str,
  room_info: &LiveRoomInfo,
  nickname: Option<&str>,
  title: &str,
) -> Result<String, String> {
  let metadata_path = Path::new(file_path)
    .with_extension("metadata.json")
    .to_string_lossy()
    .to_string();
  let payload = serde_json::json!({
    "roomId": room_info.room_id,
    "uid": room_info.uid,
    "nickname": nickname,
    "title": title,
    "startTime": now_rfc3339(),
  });
  let mut file = File::create(&metadata_path).map_err(|err| format!("创建 metadata 失败: {}", err))?;
  file
    .write_all(payload.to_string().as_bytes())
    .map_err(|err| format!("写入 metadata 失败: {}", err))?;
  Ok(metadata_path)
}

fn update_metadata_file(path: &str, end_time: &str, file_size: u64) -> Result<(), String> {
  let mut value = if let Ok(content) = std::fs::read_to_string(path) {
    serde_json::from_str::<Value>(&content).unwrap_or(Value::Null)
  } else {
    Value::Null
  };
  if !value.is_object() {
    value = serde_json::json!({});
  }
  let obj = value.as_object_mut().ok_or_else(|| "metadata 结构异常".to_string())?;
  obj.insert("endTime".to_string(), Value::String(end_time.to_string()));
  obj.insert("fileSize".to_string(), Value::Number(file_size.into()));
  std::fs::write(path, value.to_string()).map_err(|err| format!("更新 metadata 失败: {}", err))?;
  Ok(())
}

fn download_cover(target_file: &str, cover_url: &str) -> Result<(), String> {
  let response = Client::new()
    .get(cover_url)
    .send()
    .map_err(|err| format!("下载封面失败: {}", err))?;

  let mut ext = "jpg".to_string();
  if let Some(content_type) = response.headers().get(reqwest::header::CONTENT_TYPE) {
    if let Ok(content_type) = content_type.to_str() {
      if content_type.contains("png") {
        ext = "png".to_string();
      } else if content_type.contains("webp") {
        ext = "webp".to_string();
      }
    }
  }

  let cover_path = Path::new(target_file)
    .with_extension(format!("cover.{}", ext))
    .to_string_lossy()
    .to_string();
  let mut file = File::create(&cover_path).map_err(|err| format!("创建封面失败: {}", err))?;
  let bytes = response.bytes().map_err(|err| format!("读取封面失败: {}", err))?;
  file.write_all(&bytes).map_err(|err| format!("保存封面失败: {}", err))?;
  Ok(())
}

fn fetch_stream_url(
  client: &BilibiliClient,
  room_id: &str,
  settings: &LiveSettings,
  auth: Option<&AuthInfo>,
  with_quality: bool,
) -> Result<String, String> {
  let mut params = vec![
    ("cid".to_string(), room_id.to_string()),
    ("platform".to_string(), "web".to_string()),
  ];
  if with_quality {
    let qn = parse_quality(&settings.recording_quality);
    params.push(("qn".to_string(), qn.to_string()));
  }

  let data = tauri::async_runtime::block_on(client.get_json(
    "https://api.live.bilibili.com/room/v1/Room/playUrl",
    &params,
    auth,
    false,
  ))?;

  let durl = data.get("durl").and_then(|value| value.as_array()).ok_or("缺少直播流地址")?;
  let first = durl.get(0).and_then(|value| value.get("url")).and_then(|value| value.as_str());
  first.map(|value| value.to_string()).ok_or("直播流地址为空".to_string())
}

fn parse_quality(value: &str) -> i64 {
  for part in value.split(',') {
    let digits: String = part.chars().filter(|ch| ch.is_ascii_digit()).collect();
    if let Ok(qn) = digits.parse::<i64>() {
      if qn > 0 {
        return qn;
      }
    }
  }
  10000
}

pub async fn fetch_room_info(
  client: &BilibiliClient,
  room_id: &str,
) -> Result<LiveRoomInfo, String> {
  let params = vec![("room_id".to_string(), room_id.to_string())];
  let data = client
    .get_json(
      "https://api.live.bilibili.com/room/v1/Room/get_info",
      &params,
      None,
      false,
    )
    .await?;

  let room_id = data
    .get("room_id")
    .and_then(|value| value.as_i64())
    .map(|value| value.to_string())
    .unwrap_or_else(|| room_id.to_string());
  let uid = data
    .get("uid")
    .and_then(|value| value.as_i64())
    .map(|value| value.to_string())
    .unwrap_or_else(|| "0".to_string());
  let live_status = data
    .get("live_status")
    .and_then(|value| value.as_i64())
    .unwrap_or(0);
  let title = data
    .get("title")
    .and_then(|value| value.as_str())
    .unwrap_or("直播标题")
    .to_string();
  let cover = data
    .get("user_cover")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string());
  let area_name = data
    .get("area_name")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string());
  let parent_area_name = data
    .get("parent_area_name")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string());

  Ok(LiveRoomInfo {
    room_id,
    uid,
    live_status,
    title,
    cover,
    area_name,
    parent_area_name,
  })
}


struct DanmakuWriter {
  live_runtime: Arc<LiveRuntime>,
  runtime_room_id: String,
  fallback_path: String,
  current_path: Option<String>,
  file: Option<File>,
}

impl DanmakuWriter {
  fn new(live_runtime: Arc<LiveRuntime>, runtime_room_id: String, fallback_path: String) -> Self {
    Self {
      live_runtime,
      runtime_room_id,
      fallback_path,
      current_path: None,
      file: None,
    }
  }

  fn ensure_file(&mut self) -> Result<(), String> {
    let current_video = self
      .live_runtime
      .get_record_info(&self.runtime_room_id)
      .map(|info| info.file_path)
      .unwrap_or_else(|| self.fallback_path.clone());
    let target_path = Path::new(&current_video)
      .with_extension("danmaku.jsonl")
      .to_string_lossy()
      .to_string();
    if self.current_path.as_deref() == Some(target_path.as_str()) {
      return Ok(());
    }
    if let Some(parent) = Path::new(&target_path).parent() {
      std::fs::create_dir_all(parent).map_err(|err| format!("创建弹幕目录失败: {}", err))?;
    }
    let file = OpenOptions::new()
      .create(true)
      .append(true)
      .open(&target_path)
      .map_err(|err| format!("创建弹幕文件失败: {}", err))?;
    self.current_path = Some(target_path);
    self.file = Some(file);
    Ok(())
  }

  fn write_line(&mut self, line: &str) -> Result<(), String> {
    self.ensure_file()?;
    let file = self.file.as_mut().ok_or_else(|| "弹幕文件未就绪".to_string())?;
    writeln!(file, "{}", line).map_err(|err| format!("写入弹幕失败: {}", err))?;
    Ok(())
  }
}

async fn run_danmaku_loop(
  context: LiveContext,
  runtime_room_id: String,
  danmaku_room_id: String,
  record_file: String,
  settings: LiveSettings,
  stop_flag: Arc<AtomicBool>,
) -> Result<(), String> {
  if !should_record_danmaku(&settings) {
    return Ok(());
  }

  let writer = Arc::new(Mutex::new(DanmakuWriter::new(
    Arc::clone(&context.live_runtime),
    runtime_room_id.clone(),
    record_file,
  )));
  {
    let mut writer_guard = writer.lock().map_err(|_| "弹幕文件锁定失败")?;
    if let Err(err) = writer_guard.ensure_file() {
      append_log(
        &context.app_log_path,
        &format!("danmaku_file_prepare_failed room={} err={}", runtime_room_id, err),
      );
    } else if let Some(path) = writer_guard.current_path.as_ref() {
      append_log(
        &context.app_log_path,
        &format!("danmaku_file_prepare_ok room={} path={}", runtime_room_id, path),
      );
    }
  }

  let auth = context.login_store.load_auth_info(&context.db).ok().flatten();
  let uid = auth.as_ref().and_then(|info| info.user_id).unwrap_or(0);
  loop {
    if stop_flag.load(Ordering::SeqCst) {
      break;
    }
    let danmaku_info = match fetch_danmaku_info(&context.bilibili, &danmaku_room_id, auth.as_ref()).await {
      Ok(info) => info,
      Err(err) => {
        append_log(
          &context.app_log_path,
          &format!("danmaku_info_error room={} err={}", runtime_room_id, err),
        );
        tokio::time::sleep(Duration::from_secs(5)).await;
        continue;
      }
    };
    let host = danmaku_info
      .get("host_list")
      .and_then(|value| value.as_array())
      .and_then(|list| list.first())
      .cloned()
      .unwrap_or(Value::Null);
    let host_name = host.get("host").and_then(|value| value.as_str()).unwrap_or_default();
    let wss_port = host.get("wss_port").and_then(|value| value.as_i64()).unwrap_or(0);
    let ws_port = host.get("ws_port").and_then(|value| value.as_i64()).unwrap_or(0);
    let tcp_port = host.get("port").and_then(|value| value.as_i64()).unwrap_or(0);
    let token = danmaku_info.get("token").and_then(|value| value.as_str()).unwrap_or_default();
    let transport = settings.danmaku_transport;
    let mut buvid3 = auth
      .as_ref()
      .and_then(|info| extract_cookie_value(&info.cookie, "buvid3"));
    if buvid3.is_none() {
      buvid3 = context.bilibili.cached_buvid3();
    }

    let url = match transport {
      1 => format!("tcp://{}:{}", host_name, tcp_port),
      2 => format!("ws://{}:{}/sub", host_name, ws_port),
      3 => format!("wss://{}:{}/sub", host_name, wss_port),
      _ => {
        if wss_port > 0 {
          format!("wss://{}:{}/sub", host_name, wss_port)
        } else if ws_port > 0 {
          format!("ws://{}:{}/sub", host_name, ws_port)
        } else {
          format!("tcp://{}:{}", host_name, tcp_port)
        }
      }
    };

    let result = if url.starts_with("tcp://") {
      run_danmaku_tcp(&url, &danmaku_room_id, token, uid, buvid3.clone(), &settings, &stop_flag, &writer).await
    } else {
      run_danmaku_ws(&url, &danmaku_room_id, token, uid, buvid3.clone(), &settings, &stop_flag, &writer).await
    };

    if result.is_err() {
      append_log(
        &context.app_log_path,
        &format!("danmaku_error room={} err={}", runtime_room_id, result.clone().unwrap_err()),
      );
    }

    tokio::time::sleep(Duration::from_secs(5)).await;
  }

  Ok(())
}

fn should_record_danmaku(settings: &LiveSettings) -> bool {
  settings.record_danmaku
    || settings.record_danmaku_raw
    || settings.record_danmaku_superchat
    || settings.record_danmaku_gift
    || settings.record_danmaku_guard
}

async fn fetch_danmaku_info(
  client: &BilibiliClient,
  room_id: &str,
  auth: Option<&AuthInfo>,
) -> Result<Value, String> {
  let params = vec![
    ("id".to_string(), room_id.to_string()),
    ("type".to_string(), "0".to_string()),
    ("web_location".to_string(), "444.8".to_string()),
  ];
  client
    .get_json(
      "https://api.live.bilibili.com/xlive/web-room/v1/index/getDanmuInfo",
      &params,
      auth,
      true,
    )
    .await
}

async fn run_danmaku_ws(
  url: &str,
  room_id: &str,
  token: &str,
  uid: i64,
  buvid3: Option<String>,
  settings: &LiveSettings,
  stop_flag: &Arc<AtomicBool>,
  output: &Arc<Mutex<DanmakuWriter>>,
) -> Result<(), String> {
  let (ws_stream, _) = tokio_tungstenite::connect_async(url)
    .await
    .map_err(|err| format!("连接弹幕失败: {}", err))?;
  let (mut write, mut read) = ws_stream.split();
  let auth_packet = build_danmaku_packet(
    7,
    build_danmaku_auth_payload(room_id, token, uid, buvid3),
  );
  write
    .send(Message::Binary(auth_packet))
    .await
    .map_err(|err| format!("弹幕鉴权失败: {}", err))?;

  let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
  loop {
    if stop_flag.load(Ordering::SeqCst) {
      break;
    }

    tokio::select! {
      _ = heartbeat.tick() => {
        let packet = build_danmaku_packet(2, Vec::new());
        let _ = write.send(Message::Binary(packet)).await;
      }
      msg = read.next() => {
        match msg {
          Some(Ok(Message::Binary(data))) => {
            handle_danmaku_payload(&data, settings, output)?;
          }
          Some(Ok(_)) => {}
          Some(Err(err)) => return Err(format!("弹幕读取失败: {}", err)),
          None => break,
        }
      }
    }
  }
  Ok(())
}

async fn run_danmaku_tcp(
  url: &str,
  room_id: &str,
  token: &str,
  uid: i64,
  buvid3: Option<String>,
  settings: &LiveSettings,
  stop_flag: &Arc<AtomicBool>,
  output: &Arc<Mutex<DanmakuWriter>>,
) -> Result<(), String> {
  let addr = url.trim_start_matches("tcp://");
  let mut stream = tokio::net::TcpStream::connect(addr)
    .await
    .map_err(|err| format!("连接弹幕失败: {}", err))?;

  let auth_packet = build_danmaku_packet(
    7,
    build_danmaku_auth_payload(room_id, token, uid, buvid3),
  );
  stream
    .write_all(&auth_packet)
    .await
    .map_err(|err| format!("弹幕鉴权失败: {}", err))?;

  let mut heartbeat = tokio::time::interval(Duration::from_secs(30));
  let mut buffer = vec![0u8; 16];
  loop {
    if stop_flag.load(Ordering::SeqCst) {
      break;
    }

    tokio::select! {
      _ = heartbeat.tick() => {
        let packet = build_danmaku_packet(2, Vec::new());
        let _ = stream.write_all(&packet).await;
      }
      read = stream.read_exact(&mut buffer) => {
        if read.is_err() {
          break;
        }
        let packet_len = u32::from_be_bytes([buffer[0], buffer[1], buffer[2], buffer[3]]) as usize;
        let header_len = u16::from_be_bytes([buffer[4], buffer[5]]) as usize;
        let mut body = vec![0u8; packet_len - header_len];
        stream.read_exact(&mut body).await.map_err(|err| format!("读取弹幕失败: {}", err))?;
        let mut full = Vec::with_capacity(packet_len);
        full.extend_from_slice(&buffer);
        full.extend_from_slice(&body);
        handle_danmaku_payload(&full, settings, output)?;
      }
    }
  }

  Ok(())
}

fn handle_danmaku_payload(
  data: &[u8],
  settings: &LiveSettings,
  output: &Arc<Mutex<DanmakuWriter>>,
) -> Result<(), String> {
  for payload in parse_danmaku_packets(data)? {
    if payload.op != 5 {
      continue;
    }
    let text = String::from_utf8_lossy(&payload.body).to_string();
    if let Ok(value) = serde_json::from_str::<Value>(&text) {
      let cmd = value.get("cmd").and_then(|value| value.as_str()).unwrap_or("");
      let should_write = if settings.record_danmaku_raw {
        true
      } else {
        match cmd {
          "DANMU_MSG" => settings.record_danmaku,
          "SUPER_CHAT_MESSAGE" | "SUPER_CHAT_MESSAGE_JPN" => settings.record_danmaku_superchat,
          "SEND_GIFT" => settings.record_danmaku_gift,
          "GUARD_BUY" | "USER_TOAST_MSG" => settings.record_danmaku_guard,
          _ => false,
        }
      };
      if should_write {
        let line = serde_json::json!({
          "cmd": cmd,
          "data": value,
          "timestamp": now_rfc3339(),
        });
        let mut writer = output.lock().map_err(|_| "弹幕文件锁定失败")?;
        writer.write_line(&line.to_string())?;
      }
    } else if settings.record_danmaku_raw {
      let mut writer = output.lock().map_err(|_| "弹幕文件锁定失败")?;
      writer.write_line(&text)?;
    }
  }
  Ok(())
}

struct DanmakuPacket {
  op: u32,
  version: u16,
  body: Vec<u8>,
}

fn parse_danmaku_packets(data: &[u8]) -> Result<Vec<DanmakuPacket>, String> {
  let mut packets = Vec::new();
  let mut offset = 0usize;
  while offset + 16 <= data.len() {
    let packet_len = u32::from_be_bytes(data[offset..offset + 4].try_into().unwrap()) as usize;
    let header_len = u16::from_be_bytes(data[offset + 4..offset + 6].try_into().unwrap()) as usize;
    let version = u16::from_be_bytes(data[offset + 6..offset + 8].try_into().unwrap());
    let op = u32::from_be_bytes(data[offset + 8..offset + 12].try_into().unwrap());
    let body_start = offset + header_len;
    let body_end = offset + packet_len;
    if body_end > data.len() || body_start > data.len() {
      break;
    }
    let body = data[body_start..body_end].to_vec();
    if version == 2 {
      let decompressed = decompress_zlib(&body)?;
      let inner = parse_danmaku_packets(&decompressed)?;
      packets.extend(inner);
    } else if version == 3 {
      let decompressed = decompress_brotli(&body)?;
      let inner = parse_danmaku_packets(&decompressed)?;
      packets.extend(inner);
    } else {
      packets.push(DanmakuPacket { op, version, body });
    }
    offset += packet_len;
  }
  Ok(packets)
}

fn decompress_zlib(data: &[u8]) -> Result<Vec<u8>, String> {
  let mut decoder = flate2::read::ZlibDecoder::new(data);
  let mut output = Vec::new();
  decoder
    .read_to_end(&mut output)
    .map_err(|err| format!("zlib 解压失败: {}", err))?;
  Ok(output)
}

fn decompress_brotli(data: &[u8]) -> Result<Vec<u8>, String> {
  let mut decoder = brotli::Decompressor::new(data, 4096);
  let mut output = Vec::new();
  decoder
    .read_to_end(&mut output)
    .map_err(|err| format!("brotli 解压失败: {}", err))?;
  Ok(output)
}

fn build_danmaku_auth_payload(room_id: &str, token: &str, uid: i64, buvid3: Option<String>) -> Vec<u8> {
  let mut payload = serde_json::Map::new();
  payload.insert(
    "uid".to_string(),
    Value::Number(serde_json::Number::from(uid)),
  );
  payload.insert(
    "roomid".to_string(),
    Value::Number(serde_json::Number::from(room_id.parse::<i64>().unwrap_or(0))),
  );
  payload.insert("protover".to_string(), Value::Number(serde_json::Number::from(3)));
  payload.insert("platform".to_string(), Value::String("web".to_string()));
  payload.insert("type".to_string(), Value::Number(serde_json::Number::from(2)));
  payload.insert("key".to_string(), Value::String(token.to_string()));
  if let Some(buvid3) = buvid3 {
    payload.insert("buvid".to_string(), Value::String(buvid3));
  }
  Value::Object(payload).to_string().into_bytes()
}

fn extract_cookie_value(cookie: &str, key: &str) -> Option<String> {
  let needle = format!("{}=", key);
  cookie.split(';').find_map(|item| {
    let part = item.trim();
    part.strip_prefix(&needle).map(|value| value.to_string())
  })
}

fn build_danmaku_packet(op: u32, body: Vec<u8>) -> Vec<u8> {
  let header_len = 16u16;
  let packet_len = header_len as u32 + body.len() as u32;
  let mut buf = Vec::with_capacity(packet_len as usize);
  buf.extend_from_slice(&packet_len.to_be_bytes());
  buf.extend_from_slice(&header_len.to_be_bytes());
  buf.extend_from_slice(&1u16.to_be_bytes());
  buf.extend_from_slice(&op.to_be_bytes());
  buf.extend_from_slice(&1u32.to_be_bytes());
  buf.extend_from_slice(&body);
  buf
}
