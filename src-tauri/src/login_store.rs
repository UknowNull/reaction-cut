use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use chrono::{DateTime, Duration, TimeZone, Utc};
use serde_json::{json, Value};
use thiserror::Error;
use url::Url;

use crate::db::Db;

#[derive(Debug, Error)]
pub enum LoginStoreError {
  #[error("Failed to read file: {0}")]
  Io(#[from] std::io::Error),
  #[error("Database error: {0}")]
  Db(#[from] crate::db::DbError),
  #[error("Failed to parse JSON: {0}")]
  Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone)]
pub struct AuthInfo {
  pub cookie: String,
  #[allow(dead_code)]
  pub csrf: Option<String>,
  pub user_id: Option<i64>,
  pub data: Value,
}

pub struct LoginStore {
  file_path: PathBuf,
}

impl LoginStore {
  pub fn new(file_path: PathBuf) -> Self {
    Self { file_path }
  }

  pub fn load_auth_info(&self, db: &Db) -> Result<Option<AuthInfo>, LoginStoreError> {
    let file_auth = match self.load_from_file() {
      Ok(auth) => auth,
      Err(_) => None,
    };
    if let Some(auth_info) = file_auth {
      return Ok(Some(auth_info));
    }

    let auth_info = self.load_from_db(db)?;
    if let Some(ref auth_info) = auth_info {
      let login_time_ms = Utc::now().timestamp_millis();
      let file_value = json!({
        "loginTime": login_time_ms,
        "data": auth_info.data,
      });
      let serialized = serde_json::to_string(&file_value)?;
      fs::write(&self.file_path, serialized)?;
    }

    Ok(auth_info)
  }

  pub fn save_login_info(&self, db: &Db, login_data: &Value) -> Result<Option<i64>, LoginStoreError> {
    let login_time_ms = Utc::now().timestamp_millis();
    let file_value = json!({
      "loginTime": login_time_ms,
      "data": login_data,
    });
    fs::write(&self.file_path, serde_json::to_string(&file_value)?)?;

    let user_id = extract_user_id(login_data);
    if user_id.is_none() {
      return Ok(None);
    }

    let now = Utc::now();
    let expire_time = extract_expire_time(login_data)
      .unwrap_or_else(|| now + Duration::hours(24));

    let username = extract_string(login_data, &["uname", "username"]);
    let nickname = extract_string(login_data, &["nickname"]);
    let avatar_url = extract_string(login_data, &["avatar", "avatar_url"]);

    let access_token = extract_url_param(login_data, "SESSDATA");
    let refresh_token = extract_url_param(login_data, "bili_jct");

    let cookie_info = serde_json::to_string(login_data)?;

    let user_id_value = user_id.unwrap();
    let login_time_str = now.to_rfc3339();
    let expire_time_str = expire_time.to_rfc3339();
    let create_time_str = now.to_rfc3339();
    let update_time_str = now.to_rfc3339();

    db.with_conn(|conn| {
      conn.execute(
        "INSERT INTO login_info (user_id, username, nickname, avatar_url, access_token, refresh_token, cookie_info, login_time, expire_time, create_time, update_time) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
         ON CONFLICT(user_id) DO UPDATE SET \
         username = excluded.username, \
         nickname = excluded.nickname, \
         avatar_url = excluded.avatar_url, \
         access_token = excluded.access_token, \
         refresh_token = excluded.refresh_token, \
         cookie_info = excluded.cookie_info, \
         login_time = excluded.login_time, \
         expire_time = excluded.expire_time, \
         update_time = excluded.update_time",
        (
          user_id_value,
          username,
          nickname,
          avatar_url,
          access_token,
          refresh_token,
          cookie_info,
          login_time_str,
          expire_time_str,
          create_time_str,
          update_time_str,
        ),
      )?;
      Ok(())
    })?;

    Ok(Some(user_id_value))
  }

  pub fn logout(&self, db: &Db) -> Result<(), LoginStoreError> {
    if self.file_path.exists() {
      fs::remove_file(&self.file_path)?;
    }
    db.with_conn(|conn| {
      conn.execute("DELETE FROM login_info", [])?;
      Ok(())
    })?;
    Ok(())
  }

  fn load_from_file(&self) -> Result<Option<AuthInfo>, LoginStoreError> {
    if !self.file_path.exists() {
      return Ok(None);
    }

    let content = fs::read_to_string(&self.file_path)?;
    let root: Value = serde_json::from_str(&content)?;
    let login_time = root.get("loginTime").and_then(|value| value.as_i64());
    let data = match root.get("data") {
      Some(data) => data,
      None => return Ok(None),
    };

    let auth_info = build_auth_info(data, login_time);
    Ok(auth_info)
  }

  fn load_from_db(&self, db: &Db) -> Result<Option<AuthInfo>, LoginStoreError> {
    let record = db.with_conn(|conn| {
      let mut stmt = conn.prepare(
        "SELECT user_id, cookie_info, expire_time FROM login_info ORDER BY login_time DESC LIMIT 1",
      )?;
      let mut rows = stmt.query([])?;
      if let Some(row) = rows.next()? {
        let user_id: i64 = row.get(0)?;
        let cookie_info: String = row.get(1)?;
        let expire_time: Option<String> = row.get(2)?;
        Ok(Some((user_id, cookie_info, expire_time)))
      } else {
        Ok(None)
      }
    })?;

    let (user_id, cookie_info, expire_time) = match record {
      Some(record) => record,
      None => return Ok(None),
    };

    if let Some(expire_time) = expire_time {
      if let Ok(expire_dt) = DateTime::parse_from_rfc3339(&expire_time) {
        if expire_dt.with_timezone(&Utc) <= Utc::now() {
          return Ok(None);
        }
      }
    }

    let data: Value = serde_json::from_str(&cookie_info)?;
    if let Some(mut auth_info) = build_auth_info(&data, None) {
      auth_info.user_id = Some(user_id);
      return Ok(Some(auth_info));
    }

    Ok(None)
  }
}

fn build_auth_info(data: &Value, login_time_ms: Option<i64>) -> Option<AuthInfo> {
  let cookie = extract_cookie(data)?;

  if is_login_expired(data, login_time_ms) {
    return None;
  }

  let user_id = extract_user_id(data);
  let csrf = extract_csrf(&cookie);

  Some(AuthInfo {
    cookie,
    csrf,
    user_id,
    data: data.clone(),
  })
}

fn is_login_expired(data: &Value, login_time_ms: Option<i64>) -> bool {
  if let Some(expire_time) = extract_expire_time(data) {
    if expire_time <= Utc::now() {
      return true;
    }
  }

  if let Some(login_time_ms) = login_time_ms {
    if let Some(login_time) = Utc.timestamp_millis_opt(login_time_ms).single() {
      if login_time + Duration::hours(24) <= Utc::now() {
        return true;
      }
    }
  }

  false
}

fn extract_cookie(data: &Value) -> Option<String> {
  if let Some(cookie) = data.get("cookie").and_then(|value| value.as_str()) {
    return Some(cookie.to_string());
  }

  if let Some(cookie) = data.get("cookies").and_then(|value| value.as_str()) {
    return Some(cookie.to_string());
  }

  if let Some(url) = data.get("url").and_then(|value| value.as_str()) {
    return build_cookie_from_url(url);
  }

  if let Some(inner) = data.get("data") {
    return extract_cookie(inner);
  }

  None
}

fn build_cookie_from_url(url: &str) -> Option<String> {
  let params = parse_url_params(url)?;
  let sessdata = params.get("SESSDATA")?;
  let bili_jct = params.get("bili_jct")?;
  if let Some(dede_user_id) = params.get("DedeUserID") {
    return Some(format!(
      "SESSDATA={}; bili_jct={}; DedeUserID={}",
      sessdata, bili_jct, dede_user_id
    ));
  }
  Some(format!("SESSDATA={}; bili_jct={}", sessdata, bili_jct))
}

fn extract_csrf(cookie: &str) -> Option<String> {
  cookie
    .split(';')
    .find_map(|item| {
      let part = item.trim();
      if let Some(value) = part.strip_prefix("bili_jct=") {
        return Some(value.to_string());
      }
      None
    })
}

fn extract_user_id(data: &Value) -> Option<i64> {
  if let Some(url) = data.get("url").and_then(|value| value.as_str()) {
    if let Some(params) = parse_url_params(url) {
      if let Some(user_id) = params.get("DedeUserID") {
        if let Ok(parsed) = user_id.parse::<i64>() {
          return Some(parsed);
        }
      }
    }
  }

  data
    .get("mid")
    .and_then(|value| value.as_i64())
    .or_else(|| data.get("user_id").and_then(|value| value.as_i64()))
}

fn extract_expire_time(data: &Value) -> Option<DateTime<Utc>> {
  if let Some(url) = data.get("url").and_then(|value| value.as_str()) {
    if let Some(params) = parse_url_params(url) {
      if let Some(expires) = params.get("Expires") {
        if let Ok(timestamp) = expires.parse::<i64>() {
          return Utc.timestamp_opt(timestamp, 0).single();
        }
      }
    }
  }

  None
}

fn extract_url_param(data: &Value, key: &str) -> Option<String> {
  data.get("url")
    .and_then(|value| value.as_str())
    .and_then(|url| parse_url_params(url))
    .and_then(|params| params.get(key).cloned())
}

fn extract_string(data: &Value, keys: &[&str]) -> Option<String> {
  for key in keys {
    if let Some(value) = data.get(*key).and_then(|value| value.as_str()) {
      return Some(value.to_string());
    }
  }

  if let Some(inner) = data.get("data") {
    return extract_string(inner, keys);
  }

  None
}

fn parse_url_params(url: &str) -> Option<HashMap<String, String>> {
  let parsed = Url::parse(url).ok()?;
  let mut params = HashMap::new();
  for (key, value) in parsed.query_pairs() {
    params.insert(key.to_string(), value.to_string());
  }
  Some(params)
}
