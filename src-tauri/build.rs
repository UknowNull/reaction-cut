use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    sync_runtime_bins();
    tauri_build::build();
}

fn sync_runtime_bins() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap_or_default());
    if manifest_dir.as_os_str().is_empty() {
        return;
    }
    let source_dir = manifest_dir
        .join("resources")
        .join("bin")
        .join(platform_subdir());
    if !source_dir.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", source_dir.display());

    let runtime_dir = manifest_dir.join("resources").join("bin").join("runtime");
    sync_dir(&source_dir, &runtime_dir);

    let target_dir = env::var("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| manifest_dir.join("target"));
    let profile = env::var("PROFILE").unwrap_or_else(|_| "debug".to_string());
    let runtime_target_dir = target_dir.join(profile).join("bin").join("runtime");
    sync_dir(&source_dir, &runtime_target_dir);
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

fn sync_dir(source_dir: &Path, target_dir: &Path) {
    if fs::create_dir_all(target_dir).is_err() {
        return;
    }
    if let Ok(entries) = fs::read_dir(target_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let _ = fs::remove_file(&path);
            }
        }
    }
    if let Ok(entries) = fs::read_dir(source_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let target = target_dir.join(entry.file_name());
                let _ = fs::copy(&path, &target);
            }
        }
    }
}
