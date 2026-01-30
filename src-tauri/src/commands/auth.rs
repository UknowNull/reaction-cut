use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use std::time::Duration;

use reqwest::header::{HeaderMap, HeaderValue, SET_COOKIE, USER_AGENT};
use serde::Serialize;
use serde_json::{json, Value};
use tauri::State;
use tokio::time::sleep;
use url::Url;

use crate::api::ApiResponse;
use crate::bilibili::client::BilibiliClient;
use crate::login_refresh;
use crate::login_store::AuthInfo;
use crate::AppState;

const QR_CODE_GENERATE_PATH: &str = "/x/passport-login/web/qrcode/generate";
const QR_CODE_POLL_PATH: &str = "/x/passport-login/web/qrcode/poll";

#[derive(Serialize)]
pub struct PollResult {
  pub code: i32,
  pub message: String,
  pub data: Option<Value>,
}

#[tauri::command]
pub async fn auth_qrcode_generate(
  state: State<'_, AppState>,
) -> Result<ApiResponse<Value>, String> {
  append_auth_log(
    Some(state.log_path.as_path()),
    &format!("cmd=qrcode_generate ts={}", now_millis()),
  );
  let url = format!("{}{}", state.bilibili.passport_base_url(), QR_CODE_GENERATE_PATH);
  match state.bilibili.get_json(&url, &[], None, false).await {
    Ok(data) => Ok(ApiResponse::success(data)),
    Err(err) => Ok(ApiResponse::error(format!("Failed to generate QR code: {}", err))),
  }
}

#[tauri::command]
pub async fn auth_qrcode_poll(
  state: State<'_, AppState>,
  qrcode_key: String,
) -> Result<ApiResponse<PollResult>, String> {
  append_auth_log(
    Some(state.log_path.as_path()),
    &format!("cmd=qrcode_poll ts={} key_len={}", now_millis(), qrcode_key.len()),
  );
  match poll_qrcode_once(&state, &qrcode_key).await {
    Ok(result) => Ok(ApiResponse::success(result)),
    Err(err) => Ok(ApiResponse::error(format!("Failed to poll QR code status: {}", err))),
  }
}

#[tauri::command]
pub async fn auth_sms_login(
  state: State<'_, AppState>,
  cid: i64,
  tel: String,
  code: String,
  captcha_key: String,
) -> Result<ApiResponse<i32>, String> {
  let client = reqwest::Client::new();
  let mut headers = HeaderMap::new();
  headers.insert(
    USER_AGENT,
    HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"),
  );

  let response = match client
    .post("https://passport.bilibili.com/x/passport-login/web/login/sms")
    .headers(headers)
    .form(&[
      ("cid", cid.to_string()),
      ("tel", tel),
      ("code", code),
      ("source", "main-fe-header".to_string()),
      ("captcha_key", captcha_key),
      ("keep", "true".to_string()),
    ])
    .send()
    .await
  {
    Ok(response) => response,
    Err(err) => {
      return Ok(ApiResponse::error(format!("短信登录请求失败: {}", err)));
    }
  };

  if !response.status().is_success() {
    return Ok(ApiResponse::error(format!(
      "短信登录请求失败: {}",
      response.status()
    )));
  }

  let headers = response.headers().clone();
  let body: Value = match response.json().await {
    Ok(body) => body,
    Err(err) => {
      return Ok(ApiResponse::error(format!("短信登录响应解析失败: {}", err)));
    }
  };

  if body.get("code").and_then(|value| value.as_i64()).unwrap_or(0) != 0 {
    let message = body
      .get("message")
      .and_then(|value| value.as_str())
      .unwrap_or("短信登录失败");
    return Ok(ApiResponse::error(message.to_string()));
  }

  let cookie = match build_cookie_from_headers(&headers) {
    Some(cookie) => cookie,
    None => return Ok(ApiResponse::error("短信登录未返回有效 Cookie")),
  };

  let refresh_token = extract_refresh_token(&body);
  let profile = fetch_profile(&state.bilibili, &cookie).await.ok();
  let login_data = build_login_payload(&cookie, profile, refresh_token);
  if let Err(err) = state.login_store.save_login_info(&state.db, &login_data) {
    return Ok(ApiResponse::error(format!("保存登录信息失败: {}", err)));
  }

  Ok(ApiResponse::success(0))
}

#[tauri::command]
pub async fn auth_pwd_login(
  state: State<'_, AppState>,
  username: String,
  encoded_pwd: String,
  token: String,
  challenge: String,
  validate: String,
  seccode: String,
) -> Result<ApiResponse<i32>, String> {
  let client = reqwest::Client::new();
  let mut headers = HeaderMap::new();
  headers.insert(
    USER_AGENT,
    HeaderValue::from_static("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36"),
  );

  let response = match client
    .post("https://passport.bilibili.com/x/passport-login/web/login")
    .headers(headers)
    .form(&[
      ("username", username),
      ("password", encoded_pwd),
      ("token", token),
      ("challenge", challenge),
      ("validate", validate),
      ("seccode", seccode),
      ("go_url", "https://www.bilibili.com/".to_string()),
      ("source", "main-fe-header".to_string()),
    ])
    .send()
    .await
  {
    Ok(response) => response,
    Err(err) => {
      return Ok(ApiResponse::error(format!("账号登录请求失败: {}", err)));
    }
  };

  if !response.status().is_success() {
    return Ok(ApiResponse::error(format!(
      "账号登录请求失败: {}",
      response.status()
    )));
  }

  let headers = response.headers().clone();
  let body: Value = match response.json().await {
    Ok(body) => body,
    Err(err) => {
      return Ok(ApiResponse::error(format!("账号登录响应解析失败: {}", err)));
    }
  };

  if body.get("code").and_then(|value| value.as_i64()).unwrap_or(0) != 0 {
    let message = body
      .get("message")
      .and_then(|value| value.as_str())
      .unwrap_or("账号登录失败");
    return Ok(ApiResponse::error(message.to_string()));
  }

  let status = body
    .get("data")
    .and_then(|value| value.get("status"))
    .and_then(|value| value.as_i64())
    .unwrap_or(0);
  if status != 0 {
    let message = body
      .get("data")
      .and_then(|value| value.get("message"))
      .and_then(|value| value.as_str())
      .unwrap_or("账号登录失败");
    return Ok(ApiResponse::error(message.to_string()));
  }

  let cookie = match build_cookie_from_headers(&headers) {
    Some(cookie) => cookie,
    None => return Ok(ApiResponse::error("账号登录未返回有效 Cookie")),
  };

  let refresh_token = extract_refresh_token(&body);
  let profile = fetch_profile(&state.bilibili, &cookie).await.ok();
  let login_data = build_login_payload(&cookie, profile, refresh_token);
  if let Err(err) = state.login_store.save_login_info(&state.db, &login_data) {
    return Ok(ApiResponse::error(format!("保存登录信息失败: {}", err)));
  }

  Ok(ApiResponse::success(0))
}

#[tauri::command]
pub async fn auth_status(
  state: State<'_, AppState>,
) -> Result<ApiResponse<HashMap<String, Value>>, String> {
  append_auth_log(
    Some(state.log_path.as_path()),
    &format!("cmd=auth_status ts={}", now_millis()),
  );
  match build_auth_status(&state).await {
    Ok(data) => Ok(ApiResponse::success(data)),
    Err(err) => Ok(ApiResponse::error(err)),
  }
}

#[tauri::command]
pub async fn auth_refresh(
  state: State<'_, AppState>,
) -> Result<ApiResponse<HashMap<String, Value>>, String> {
  append_auth_log(
    Some(state.log_path.as_path()),
    &format!("cmd=auth_refresh ts={}", now_millis()),
  );
  let refresh_result = login_refresh::refresh_cookie(
    &state.bilibili,
    &state.login_store,
    &state.db,
    &state.app_log_path,
  )
  .await;
  if let Err(err) = refresh_result {
    return Ok(ApiResponse::error(format!("刷新登录失败: {}", err)));
  }
  match build_auth_status(&state).await {
    Ok(data) => Ok(ApiResponse::success(data)),
    Err(err) => Ok(ApiResponse::error(err)),
  }
}

#[tauri::command]
pub async fn auth_client_log(
  state: State<'_, AppState>,
  message: String,
) -> Result<ApiResponse<String>, String> {
  append_auth_log(
    Some(state.log_path.as_path()),
    &format!("client_log ts={} msg={}", now_millis(), message),
  );
  Ok(ApiResponse::success("ok".to_string()))
}

#[tauri::command]
pub async fn auth_logout(state: State<'_, AppState>) -> Result<ApiResponse<String>, String> {
  match state.login_store.logout(&state.db) {
    Ok(()) => Ok(ApiResponse::success("Logged out".to_string())),
    Err(err) => Ok(ApiResponse::error(format!("Failed to logout: {}", err))),
  }
}

#[tauri::command]
pub async fn auth_perform_qrcode_login(
  state: State<'_, AppState>,
) -> Result<ApiResponse<String>, String> {
  let bilibili = state.bilibili.clone();
  let login_store = state.login_store.clone();
  let db = state.db.clone();
  let log_path = state.log_path.clone();

  tauri::async_runtime::spawn(async move {
    if let Ok(qr_data) = bilibili
      .get_json(&format!("{}{}", bilibili.passport_base_url(), QR_CODE_GENERATE_PATH), &[], None, false)
      .await
    {
      let qrcode_key = qr_data
        .get("qrcode_key")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string());

      if let Some(qrcode_key) = qrcode_key {
        let mut attempts = 0;
        while attempts < 30 {
          attempts += 1;
          sleep(Duration::from_secs(3)).await;
          if let Ok(result) =
            poll_qrcode_once_inner(&bilibili, &login_store, &db, &qrcode_key, Some(log_path.as_path()))
              .await
          {
            if result.code == 0 || result.code == 86038 {
              break;
            }
          }
        }
      }
    }
  });

  Ok(ApiResponse::success("QR login flow started".to_string()))
}

async fn poll_qrcode_once(
  state: &State<'_, AppState>,
  qrcode_key: &str,
) -> Result<PollResult, String> {
  let result = poll_qrcode_once_inner(
    &state.bilibili,
    &state.login_store,
    &state.db,
    qrcode_key,
    Some(state.log_path.as_path()),
  )
  .await?;
  if result.code == 86101 {
    sleep(Duration::from_secs(1)).await;
    return poll_qrcode_once_inner(
      &state.bilibili,
      &state.login_store,
      &state.db,
      qrcode_key,
      Some(state.log_path.as_path()),
    )
    .await;
  }
  Ok(result)
}

async fn poll_qrcode_once_inner(
  bilibili: &crate::bilibili::client::BilibiliClient,
  login_store: &crate::login_store::LoginStore,
  db: &crate::db::Db,
  qrcode_key: &str,
  log_path: Option<&Path>,
) -> Result<PollResult, String> {
  let client = reqwest::Client::new();
  let url = format!(
    "{}{}?qrcode_key={}&source=main-fe-header",
    bilibili.passport_base_url(),
    QR_CODE_POLL_PATH,
    qrcode_key
  );
  let response = client
    .get(&url)
    .header(
      USER_AGENT,
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
    )
    .send()
    .await
    .map_err(|err| format!("Request failed: {}", err))?;

  let headers = response.headers().clone();
  let body: Value = response
    .json()
    .await
    .map_err(|err| format!("Failed to parse response: {}", err))?;

  let api_code = body.get("code").and_then(|value| value.as_i64());
  if let Some(code) = api_code {
    if code != 0 {
      let message = body
        .get("message")
        .and_then(|value| value.as_str())
        .unwrap_or("Bilibili returned an error");
      append_auth_log(
        log_path,
        &format!(
          "qr_poll api_error ts={} code={} message={}",
          now_millis(),
          code,
          message
        ),
      );
      return Err(format!("{} (code: {})", message, code));
    }
  }

  let data = body.get("data").cloned().unwrap_or_else(|| body.clone());
  let code_value = data.get("code").or_else(|| body.get("code"));
  let code = parse_code(code_value).unwrap_or(86101) as i32;
  let message = data
    .get("message")
    .or_else(|| body.get("message"))
    .and_then(|value| value.as_str())
    .unwrap_or("Pending")
    .to_string();

  append_auth_log(
    log_path,
    &format!(
      "qr_poll ts={} code={} message={} set_cookie={} url_len={} refresh_token={}",
      now_millis(),
      code,
      message,
      headers.get_all(SET_COOKIE).iter().count(),
      data.get("url")
        .and_then(|value| value.as_str())
        .map(|value| value.len())
        .unwrap_or(0),
      data.get("refresh_token").is_some(),
    ),
  );

  if code == 0 {
    let mut cookie = build_cookie_from_headers(&headers);
    if cookie.is_none() {
      if let Some(url) = data.get("url").and_then(|value| value.as_str()) {
        if let Ok(fetched) = exchange_cookie_from_url(url, log_path).await {
          cookie = fetched;
        }
      }
    }

    let refresh_token = extract_refresh_token(&data);
    let login_data = if let Some(cookie) = cookie {
      let profile = fetch_profile(bilibili, &cookie).await.ok();
      append_auth_log(
        log_path,
        &format!(
          "qr_poll save_cookie ts={} cookie_keys={}",
          now_millis(),
          summarize_cookie_keys(&cookie)
        ),
      );
      build_login_payload(&cookie, profile, refresh_token.clone())
    } else {
      append_auth_log(
        log_path,
        &format!("qr_poll save_cookie ts={} cookie_keys=none", now_millis()),
      );
      match build_login_payload_with_qr(&data, bilibili, refresh_token.clone()).await {
        Ok(payload) => payload,
        Err(_) => data.clone(),
      }
    };
    match login_store.save_login_info(db, &login_data) {
      Ok(Some(user_id)) => append_auth_log(
        log_path,
        &format!("qr_poll save_ok ts={} user_id={}", now_millis(), user_id),
      ),
      Ok(None) => append_auth_log(
        log_path,
        &format!("qr_poll save_ok ts={} user_id=none", now_millis()),
      ),
      Err(err) => {
        append_auth_log(
          log_path,
          &format!("qr_poll save_fail ts={} err={}", now_millis(), err),
        );
        return Err(format!("Failed to save login info: {}", err));
      }
    }
  }

  Ok(PollResult {
    code,
    message,
    data: Some(data),
  })
}

fn build_login_payload(
  cookie: &str,
  profile: Option<Value>,
  refresh_token: Option<String>,
) -> Value {
  let mut map = serde_json::Map::new();
  map.insert("cookie".to_string(), Value::String(cookie.to_string()));
  if let Some(refresh_token) = refresh_token {
    map.insert(
      "refresh_token".to_string(),
      Value::String(refresh_token),
    );
  }
  if let Some(profile) = profile {
    if let Value::Object(obj) = profile {
      for (key, value) in obj {
        map.insert(key, value);
      }
    } else {
      map.insert("profile".to_string(), profile);
    }
  }
  Value::Object(map)
}

async fn build_auth_status(state: &AppState) -> Result<HashMap<String, Value>, String> {
  let auth_info = match state.login_store.load_auth_info(&state.db) {
    Ok(info) => info,
    Err(err) => {
      return Err(format!("Failed to load login info: {}", err));
    }
  };
  let mut data = HashMap::new();
  if let Some(info) = auth_info {
    let mut user_info = info.data.clone();
    if !has_basic_profile(&user_info) || needs_profile_refresh(&user_info) {
      if let Ok(profile) = fetch_profile(&state.bilibili, &info.cookie).await {
        let refresh_token = extract_refresh_token(&info.data);
        let login_data = build_login_payload(&info.cookie, Some(profile), refresh_token);
        let _ = state.login_store.save_login_info(&state.db, &login_data);
        user_info = login_data;
      }
    }
    data.insert("loggedIn".to_string(), Value::Bool(true));
    data.insert("userInfo".to_string(), user_info);
    if let Ok(meta) = load_login_meta(&state.db) {
      if let Some(meta) = meta {
        data.insert("loginMeta".to_string(), meta);
      }
    }
  } else {
    data.insert("loggedIn".to_string(), Value::Bool(false));
  }
  Ok(data)
}

fn parse_code(value: Option<&Value>) -> Option<i64> {
  match value {
    Some(Value::Number(number)) => number.as_i64(),
    Some(Value::String(text)) => text.parse::<i64>().ok(),
    _ => None,
  }
}

fn has_basic_profile(data: &Value) -> bool {
  let root = data.get("data").unwrap_or(data);
  let name = root
    .get("uname")
    .or_else(|| root.get("name"))
    .or_else(|| root.get("username"))
    .and_then(|value| value.as_str())
    .unwrap_or_default();
  let avatar = root
    .get("face")
    .or_else(|| root.get("avatar"))
    .or_else(|| root.get("avatar_url"))
    .and_then(|value| value.as_str())
    .unwrap_or_default();
  !name.is_empty() && !avatar.is_empty()
}

fn needs_profile_refresh(data: &Value) -> bool {
  let root = data.get("data").unwrap_or(data);
  let sign = root
    .get("sign")
    .and_then(|value| value.as_str())
    .unwrap_or("");
  let coins = root
    .get("stat")
    .and_then(|value| value.get("coins"))
    .and_then(|value| value.as_f64());
  sign.trim().is_empty() || coins.is_none()
}

async fn build_login_payload_with_qr(
  data: &Value,
  bilibili: &BilibiliClient,
  refresh_token: Option<String>,
) -> Result<Value, String> {
  let cookie = extract_cookie(data).ok_or("Missing cookie")?;
  let profile = fetch_profile(bilibili, &cookie).await.ok();
  Ok(build_login_payload(&cookie, profile, refresh_token))
}

async fn fetch_profile(bilibili: &BilibiliClient, cookie: &str) -> Result<Value, String> {
  let auth = AuthInfo {
    cookie: cookie.to_string(),
    csrf: None,
    user_id: None,
    data: json!({}),
  };

  let nav_url = format!("{}/x/web-interface/nav", bilibili.base_url());
  let nav = bilibili.get_json(&nav_url, &[], Some(&auth), false).await?;
  let mid = nav.get("mid").and_then(|value| value.as_i64());

  let stat_url = format!("{}/x/web-interface/nav/stat", bilibili.base_url());
  let stat = bilibili.get_json(&stat_url, &[], Some(&auth), false).await.ok();

  let user_info = if let Some(mid) = mid {
    let user_url = format!("{}/x/space/wbi/acc/info", bilibili.base_url());
    let params = vec![("mid".to_string(), mid.to_string())];
    bilibili.get_json(&user_url, &params, Some(&auth), true).await.ok()
  } else {
    None
  };

  let name = user_info
    .as_ref()
    .and_then(|value| value.get("name").and_then(|value| value.as_str()))
    .or_else(|| nav.get("uname").and_then(|value| value.as_str()))
    .or_else(|| nav.get("name").and_then(|value| value.as_str()))
    .unwrap_or("Bilibili 用户");
  let avatar = user_info
    .as_ref()
    .and_then(|value| value.get("face").and_then(|value| value.as_str()))
    .or_else(|| nav.get("face").and_then(|value| value.as_str()))
    .unwrap_or_default();
  let desc = user_info
    .as_ref()
    .and_then(|value| value.get("sign").and_then(|value| value.as_str()))
    .or_else(|| nav.get("sign").and_then(|value| value.as_str()))
    .unwrap_or_default();
  let coins = user_info
    .as_ref()
    .and_then(|value| value.get("coins").and_then(|value| value.as_f64()))
    .or_else(|| nav.get("coins").and_then(|value| value.as_f64()))
    .or_else(|| nav.get("money").and_then(|value| value.as_f64()))
    .unwrap_or(0.0);
  let following = stat
    .as_ref()
    .and_then(|value| value.get("following").and_then(|value| value.as_i64()))
    .unwrap_or(0);
  let follower = stat
    .as_ref()
    .and_then(|value| value.get("follower").and_then(|value| value.as_i64()))
    .unwrap_or(0);
  let dynamic = stat
    .as_ref()
    .and_then(|value| value.get("dynamic_count").and_then(|value| value.as_i64()))
    .unwrap_or(0);

  Ok(json!({
    "mid": mid,
    "uname": name,
    "face": avatar,
    "sign": desc,
    "stat": {
      "following": following,
      "follower": follower,
      "dynamic": dynamic,
      "coins": coins,
    },
  }))
}

fn load_login_meta(db: &crate::db::Db) -> Result<Option<Value>, String> {
  db.with_conn(|conn| {
    let mut stmt = conn.prepare(
      "SELECT login_time, expire_time FROM login_info ORDER BY login_time DESC LIMIT 1",
    )?;
    let mut rows = stmt.query([])?;
    if let Some(row) = rows.next()? {
      let login_time: Option<String> = row.get(0)?;
      let expire_time: Option<String> = row.get(1)?;
      Ok(Some(json!({
        "loginTime": login_time,
        "expireTime": expire_time,
      })))
    } else {
      Ok(None)
    }
  })
  .map_err(|err| err.to_string())
}

fn build_cookie_from_headers(headers: &HeaderMap) -> Option<String> {
  let mut values = HashMap::new();
  for header in headers.get_all(SET_COOKIE).iter() {
    let cookie = header.to_str().ok()?;
    let pair = cookie.split(';').next()?.trim();
    if let Some((name, value)) = pair.split_once('=') {
      values.insert(name.trim().to_string(), format!("{}={}", name.trim(), value.trim()));
    }
  }
  if values.is_empty() {
    return None;
  }
  let mut cookies: Vec<String> = values.into_values().collect();
  cookies.sort();
  Some(cookies.join("; "))
}

async fn exchange_cookie_from_url(url: &str, log_path: Option<&Path>) -> Result<Option<String>, String> {
  let client = reqwest::Client::builder()
    .redirect(reqwest::redirect::Policy::none())
    .build()
    .map_err(|err| format!("Cookie exchange client failed: {}", err))?;
  let response = client
    .get(url)
    .header(
      USER_AGENT,
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36",
    )
    .send()
    .await
    .map_err(|err| format!("Cookie exchange failed: {}", err))?;

  if let Some(cookie) = build_cookie_from_headers(response.headers()) {
    append_auth_log(
      log_path,
      &format!(
        "exchange_cookie ts={} set_cookie={} url_len={}",
        now_millis(),
        response.headers().get_all(SET_COOKIE).iter().count(),
        url.len()
      ),
    );
    return Ok(Some(cookie));
  }

  let final_url = response.url().as_str();
  append_auth_log(
    log_path,
    &format!(
      "exchange_cookie ts={} set_cookie=0 final_url_len={}",
      now_millis(),
      final_url.len()
    ),
  );
  Ok(build_cookie_from_url(final_url))
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

fn extract_refresh_token(data: &Value) -> Option<String> {
  data
    .get("data")
    .and_then(|value| value.get("refresh_token"))
    .and_then(|value| value.as_str())
    .map(|value| value.to_string())
    .or_else(|| {
      data
        .get("refresh_token")
        .and_then(|value| value.as_str())
        .map(|value| value.to_string())
    })
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

fn parse_url_params(url: &str) -> Option<HashMap<String, String>> {
  let parsed = Url::parse(url).ok()?;
  let mut params = HashMap::new();
  for (key, value) in parsed.query_pairs() {
    params.insert(key.to_string(), value.to_string());
  }
  Some(params)
}

fn now_millis() -> u128 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|value| value.as_millis())
    .unwrap_or(0)
}

fn append_auth_log(path: Option<&Path>, line: &str) {
  let Some(path) = path else { return; };
  if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
    let _ = writeln!(file, "{}", line);
  }
}

fn summarize_cookie_keys(cookie: &str) -> String {
  let keys: Vec<&str> = cookie
    .split(';')
    .filter_map(|item| item.trim().split_once('=').map(|(key, _)| key.trim()))
    .collect();
  if keys.is_empty() {
    "none".to_string()
  } else {
    keys.join("|")
  }
}
