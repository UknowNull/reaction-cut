use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::State;

use crate::api::ApiResponse;
use crate::commands::settings::{default_live_settings, load_live_settings_from_db};
use crate::live_recorder::{fetch_room_info, start_recording, stop_recording, LiveContext};
use crate::utils::{append_log, now_rfc3339};
use crate::AppState;

const LIVE_ROOM_INFO_URL: &str = "https://api.live.bilibili.com/room/v1/Room/get_info";
const LIVE_USER_INFO_URL: &str = "https://api.live.bilibili.com/live_user/v1/Master/info";

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscribeRequest {
  pub uids: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Anchor {
  pub id: i64,
  pub uid: String,
  pub nickname: Option<String>,
  pub live_status: i64,
  pub last_check_time: Option<String>,
  pub create_time: String,
  pub update_time: String,
  pub avatar_url: Option<String>,
  pub live_title: Option<String>,
  pub category: Option<String>,
  pub auto_record: bool,
  pub recording_status: Option<String>,
  pub recording_file: Option<String>,
  pub recording_start_time: Option<String>,
}

struct AnchorLiveInfo {
  nickname: Option<String>,
  live_status: i64,
  avatar_url: Option<String>,
  live_title: Option<String>,
  category: Option<String>,
}

#[tauri::command]
pub async fn anchor_subscribe(
  state: State<'_, AppState>,
  payload: SubscribeRequest,
) -> Result<ApiResponse<Vec<Anchor>>, String> {
  let now = now_rfc3339();
  let settings = load_live_settings_from_db(&state.db).unwrap_or_else(|_| default_live_settings());
  let context = LiveContext {
    db: state.db.clone(),
    bilibili: state.bilibili.clone(),
    login_store: state.login_store.clone(),
    app_log_path: state.app_log_path.clone(),
    live_runtime: state.live_runtime.clone(),
  };
  append_log(
    &state.app_log_path,
    &format!("anchor_subscribe_start uids={}", payload.uids.join(",")),
  );

  for uid in payload.uids {
    let uid = uid.trim().to_string();
    if uid.is_empty() {
      continue;
    }

    let info = match fetch_live_info(&state, &uid).await {
      Ok(value) => value,
      Err(_) => AnchorLiveInfo {
        nickname: None,
        live_status: 0,
        avatar_url: None,
        live_title: None,
        category: None,
      },
    };

    let result = state.db.with_conn(|conn| {
      conn.execute(
        "INSERT INTO anchor (uid, nickname, live_status, last_check_time, create_time, update_time) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(uid) DO UPDATE SET \
         nickname = excluded.nickname, \
         live_status = excluded.live_status, \
         last_check_time = excluded.last_check_time, \
         update_time = excluded.update_time",
        (
          &uid,
          info.nickname.as_deref(),
          info.live_status,
          &now,
          &now,
          &now,
        ),
      )?;
      conn.execute(
        "INSERT INTO live_room_settings (room_id, auto_record, update_time) VALUES (?1, 1, ?2) \
         ON CONFLICT(room_id) DO UPDATE SET update_time = excluded.update_time",
        (&uid, &now),
      )?;
      Ok(())
    });

    if let Err(err) = result {
      append_log(
        &state.app_log_path,
        &format!("anchor_subscribe_error uid={} err={}", uid, err),
      );
      return Ok(ApiResponse::error("Failed to subscribe anchor"));
    }

    if info.live_status == 1 {
      if let Ok(room_info) = fetch_room_info(&state.bilibili, &uid).await {
        if !state.live_runtime.is_recording(&uid) {
          if let Err(err) = start_recording(context.clone(), &uid, room_info, settings.clone()) {
            append_log(
              &state.app_log_path,
              &format!("auto_record_subscribe_failed room={} err={}", uid, err),
            );
          } else {
            append_log(
              &state.app_log_path,
              &format!("auto_record_subscribe_start room={}", uid),
            );
          }
        }
      }
    }
  }

  Ok(anchor_list(state))
}

#[tauri::command]
pub fn anchor_list(state: State<'_, AppState>) -> ApiResponse<Vec<Anchor>> {
  match state.db.with_conn(|conn| {
    let mut stmt = conn.prepare(
      "SELECT a.id, a.uid, a.nickname, a.live_status, a.last_check_time, a.create_time, a.update_time, IFNULL(l.auto_record, 1) \
       FROM anchor a LEFT JOIN live_room_settings l ON a.uid = l.room_id ORDER BY a.id DESC",
    )?;
    let anchors = stmt
      .query_map([], |row| {
        let uid: String = row.get(1)?;
        let auto_record: i64 = row.get(7)?;
        let record_info = state.live_runtime.get_record_info(&uid);
        Ok(Anchor {
          id: row.get(0)?,
          uid: uid.clone(),
          nickname: row.get(2)?,
          live_status: row.get::<_, i64>(3)?,
          last_check_time: row.get(4)?,
          create_time: row.get(5)?,
          update_time: row.get(6)?,
          avatar_url: None,
          live_title: None,
          category: None,
          auto_record: auto_record != 0,
          recording_status: record_info.as_ref().map(|_| "RECORDING".to_string()),
          recording_file: record_info.as_ref().map(|info| info.file_path.clone()),
          recording_start_time: record_info.map(|info| info.start_time),
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(anchors)
  }) {
    Ok(list) => ApiResponse::success(list),
    Err(err) => ApiResponse::error(format!("Failed to load anchors: {}", err)),
  }
}

#[tauri::command]
pub fn anchor_unsubscribe(state: State<'_, AppState>, uid: String) -> ApiResponse<String> {
  let context = LiveContext {
    db: state.db.clone(),
    bilibili: state.bilibili.clone(),
    login_store: state.login_store.clone(),
    app_log_path: state.app_log_path.clone(),
    live_runtime: state.live_runtime.clone(),
  };
  stop_recording(context, &uid, "取消订阅");
  let uid_value = uid;
  match state.db.with_conn(|conn| {
    conn.execute("DELETE FROM anchor WHERE uid = ?1", [uid_value.as_str()])?;
    conn.execute("DELETE FROM live_room_settings WHERE room_id = ?1", [uid_value.as_str()])?;
    Ok(())
  }) {
    Ok(()) => ApiResponse::success("Unsubscribed".to_string()),
    Err(err) => ApiResponse::error(format!("Failed to unsubscribe: {}", err)),
  }
}

#[tauri::command]
pub async fn anchor_check(state: State<'_, AppState>) -> Result<ApiResponse<Vec<Anchor>>, String> {
  let settings = load_live_settings_from_db(&state.db).unwrap_or_else(|_| default_live_settings());
  let context = LiveContext {
    db: state.db.clone(),
    bilibili: state.bilibili.clone(),
    login_store: state.login_store.clone(),
    app_log_path: state.app_log_path.clone(),
    live_runtime: state.live_runtime.clone(),
  };
  let anchors = match state.db.with_conn(|conn| {
    let mut stmt = conn.prepare(
      "SELECT a.id, a.uid, a.nickname, a.live_status, a.last_check_time, a.create_time, a.update_time, IFNULL(l.auto_record, 1) \
       FROM anchor a LEFT JOIN live_room_settings l ON a.uid = l.room_id ORDER BY a.id DESC",
    )?;
    let list = stmt
      .query_map([], |row| {
        Ok(Anchor {
          id: row.get(0)?,
          uid: row.get(1)?,
          nickname: row.get(2)?,
          live_status: row.get(3)?,
          last_check_time: row.get(4)?,
          create_time: row.get(5)?,
          update_time: row.get(6)?,
          avatar_url: None,
          live_title: None,
          category: None,
          auto_record: row.get::<_, i64>(7)? != 0,
          recording_status: None,
          recording_file: None,
          recording_start_time: None,
        })
      })?
      .collect::<Result<Vec<_>, _>>()?;
    Ok(list)
  }) {
    Ok(list) => list,
    Err(err) => return Ok(ApiResponse::error(format!("Failed to read anchors: {}", err))),
  };

  let now = now_rfc3339();
  let mut updated = Vec::new();
  for anchor in anchors {
    let info = match fetch_live_info(&state, &anchor.uid).await {
      Ok(value) => value,
      Err(_) => AnchorLiveInfo {
        nickname: anchor.nickname.clone(),
        live_status: anchor.live_status,
        avatar_url: None,
        live_title: None,
        category: None,
      },
    };

    let _ = state.db.with_conn(|conn| {
      conn.execute(
        "UPDATE anchor SET nickname = ?1, live_status = ?2, last_check_time = ?3, update_time = ?4 WHERE id = ?5",
        (
          info.nickname.as_deref(),
          info.live_status,
          &now,
          &now,
          anchor.id,
        ),
      )?;
      Ok(())
    });

    let room_id = anchor.uid.clone();
    let record_info = state.live_runtime.get_record_info(&room_id);
    updated.push(Anchor {
      id: anchor.id,
      uid: room_id.clone(),
      nickname: info.nickname,
      live_status: info.live_status,
      last_check_time: Some(now.clone()),
      create_time: anchor.create_time,
      update_time: now.clone(),
      avatar_url: info.avatar_url,
      live_title: info.live_title,
      category: info.category,
      auto_record: anchor.auto_record,
      recording_status: record_info.as_ref().map(|_| "RECORDING".to_string()),
      recording_file: record_info.as_ref().map(|info| info.file_path.clone()),
      recording_start_time: record_info.map(|info| info.start_time),
    });

    if anchor.auto_record && info.live_status == 1 && !state.live_runtime.is_recording(&room_id) {
      if let Ok(room_info) = fetch_room_info(&state.bilibili, &room_id).await {
        if let Err(err) = start_recording(context.clone(), &room_id, room_info, settings.clone()) {
          append_log(
            &state.app_log_path,
            &format!("auto_record_check_failed room={} err={}", room_id, err),
          );
        } else {
          append_log(
            &state.app_log_path,
            &format!("auto_record_check_start room={}", room_id),
          );
        }
      }
    }
  }

  Ok(ApiResponse::success(updated))
}


async fn fetch_live_info(
  state: &State<'_, AppState>,
  uid: &str,
) -> Result<AnchorLiveInfo, String> {
  let params = vec![("room_id".to_string(), uid.to_string())];
  let data = state
    .bilibili
    .get_json(LIVE_ROOM_INFO_URL, &params, None, false)
    .await?;

  let live_status = data
    .get("live_status")
    .and_then(|value| value.as_i64())
    .unwrap_or(0);

  let live_title = data
    .get("title")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string());

  let area_name = data
    .get("area_name")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string());
  let parent_area = data
    .get("parent_area_name")
    .and_then(|value| value.as_str())
    .map(|value| value.to_string());
  let category = match (parent_area, area_name) {
    (Some(parent), Some(child)) => Some(format!("{} / {}", parent, child)),
    (Some(parent), None) => Some(parent),
    (None, Some(child)) => Some(child),
    _ => None,
  };

  let uid_value = data
    .get("uid")
    .and_then(|value| value.as_i64())
    .unwrap_or(0);
  let (nickname, avatar_url) = if uid_value > 0 {
    let user_params = vec![("uid".to_string(), uid_value.to_string())];
    let user_data = state
      .bilibili
      .get_json(LIVE_USER_INFO_URL, &user_params, None, false)
      .await?;
    let info = user_data.get("info").cloned().unwrap_or(Value::Null);
    let nickname = info
      .get("uname")
      .and_then(|value| value.as_str())
      .map(|value| value.to_string());
    let avatar_url = info
      .get("face")
      .and_then(|value| value.as_str())
      .map(|value| value.to_string());
    (nickname, avatar_url)
  } else {
    (None, None)
  };

  Ok(AnchorLiveInfo {
    nickname,
    live_status,
    avatar_url,
    live_title,
    category,
  })
}
