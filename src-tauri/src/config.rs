use std::env;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use tauri::path::BaseDirectory;
use tauri::AppHandle;
use tauri::Manager;

pub const DEFAULT_FFMPEG_PATH: &str = "/opt/homebrew/bin/ffmpeg";
pub const DEFAULT_FFPROBE_PATH: &str = "/opt/homebrew/bin/ffprobe";
pub const DEFAULT_ARIA2C_PATH: &str = "/opt/homebrew/bin/aria2c";

const ENV_FFMPEG_PATH: &str = "REACTION_CUT_FFMPEG_PATH";
const ENV_FFPROBE_PATH: &str = "REACTION_CUT_FFPROBE_PATH";
const ENV_ARIA2C_PATH: &str = "REACTION_CUT_ARIA2C_PATH";

fn resolve_home_dir() -> Option<PathBuf> {
  if cfg!(target_os = "windows") {
    env::var_os("USERPROFILE")
      .map(PathBuf::from)
      .or_else(|| {
        let drive = env::var_os("HOMEDRIVE");
        let path = env::var_os("HOMEPATH");
        match (drive, path) {
          (Some(drive), Some(path)) => {
            let mut buf = PathBuf::from(drive);
            buf.push(path);
            Some(buf)
          }
          _ => None,
        }
      })
  } else {
    env::var_os("HOME").map(PathBuf::from)
  }
}

pub fn default_download_dir() -> PathBuf {
  let mut base = resolve_home_dir().unwrap_or_else(|| {
    if cfg!(target_os = "windows") {
      PathBuf::from("C:\\")
    } else {
      PathBuf::from("/tmp")
    }
  });
  base.push("Downloads");
  base
}

pub fn default_temp_dir() -> PathBuf {
  default_download_dir().join("temp")
}

pub fn init_resource_bins(app_handle: &AppHandle) {
  let base_dir = match app_handle.path().resolve("bin", BaseDirectory::Resource) {
    Ok(path) => path,
    Err(_) => return,
  };
  let runtime_dir = base_dir.join("runtime");
  let platform_dir = base_dir.join(platform_subdir());

  let ffmpeg_path = resolve_bin_in_dirs(&runtime_dir, &platform_dir, &base_dir, "ffmpeg");
  if let Some(path) = ffmpeg_path {
    set_env_if_exists(ENV_FFMPEG_PATH, path);
  }
  let ffprobe_path = resolve_bin_in_dirs(&runtime_dir, &platform_dir, &base_dir, "ffprobe");
  if let Some(path) = ffprobe_path {
    set_env_if_exists(ENV_FFPROBE_PATH, path);
  }
  let aria2c_path = resolve_bin_in_dirs(&runtime_dir, &platform_dir, &base_dir, "aria2c");
  if let Some(path) = aria2c_path {
    set_env_if_exists(ENV_ARIA2C_PATH, path);
  }
}

pub fn resolve_ffmpeg_path() -> PathBuf {
  resolve_bin_path(ENV_FFMPEG_PATH, DEFAULT_FFMPEG_PATH)
}

pub fn resolve_ffprobe_path() -> PathBuf {
  resolve_bin_path(ENV_FFPROBE_PATH, DEFAULT_FFPROBE_PATH)
}

pub fn resolve_aria2c_candidates() -> Vec<String> {
  let mut candidates = Vec::new();
  if let Ok(value) = env::var(ENV_ARIA2C_PATH) {
    if !value.trim().is_empty() {
      candidates.push(value);
    }
  }
  if !candidates.iter().any(|item| item == DEFAULT_ARIA2C_PATH) {
    candidates.push(DEFAULT_ARIA2C_PATH.to_string());
  }
  candidates.push("aria2c".to_string());
  if cfg!(target_os = "windows") {
    candidates.push("aria2c.exe".to_string());
  }
  candidates.dedup();
  candidates
}

fn resolve_bin_path(env_key: &str, fallback: &str) -> PathBuf {
  if let Ok(value) = env::var(env_key) {
    if !value.trim().is_empty() {
      return PathBuf::from(value);
    }
  }
  PathBuf::from(fallback)
}

fn set_env_if_exists(key: &str, path: PathBuf) {
  if path.exists() {
    #[cfg(unix)]
    {
      let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755));
    }
    env::set_var(key, path.to_string_lossy().to_string());
  }
}

fn bin_name(base: &str) -> String {
  if cfg!(target_os = "windows") {
    format!("{}.exe", base)
  } else {
    base.to_string()
  }
}

fn platform_subdir() -> &'static str {
  if cfg!(target_os = "windows") {
    "windows"
  } else if cfg!(target_os = "macos") {
    "macos"
  } else {
    "linux"
  }
}

fn resolve_bin_in_dirs(
  runtime: &PathBuf,
  platform: &PathBuf,
  fallback: &PathBuf,
  base: &str,
) -> Option<PathBuf> {
  for dir in [runtime, platform, fallback] {
    let candidate = dir.join(bin_name(base));
    if candidate.exists() {
      return Some(candidate);
    }
  }
  None
}
