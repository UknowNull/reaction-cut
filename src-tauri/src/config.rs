use std::env;
use std::path::PathBuf;

pub const DEFAULT_FFMPEG_PATH: &str = "/opt/homebrew/bin/ffmpeg";
pub const DEFAULT_FFPROBE_PATH: &str = "/opt/homebrew/bin/ffprobe";
pub const DEFAULT_ARIA2C_PATH: &str = "/opt/homebrew/bin/aria2c";

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
