use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::Utc;

pub fn now_rfc3339() -> String {
  Utc::now().to_rfc3339()
}

pub fn sanitize_filename(name: &str) -> String {
  let mut sanitized = String::with_capacity(name.len());
  for ch in name.chars() {
    let is_invalid = matches!(ch, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|');
    sanitized.push(if is_invalid { '_' } else { ch });
  }
  sanitized
}

pub fn build_output_path(base_dir: &str, folder: &str, file_name: &str) -> PathBuf {
  let mut path = PathBuf::from(base_dir);
  path.push(folder);
  path.push(file_name);
  path
}

pub fn append_log(path: &Path, message: &str) {
  if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
    let _ = writeln!(file, "ts={} {}", now_rfc3339(), message);
  }
}
