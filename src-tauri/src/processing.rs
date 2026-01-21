use std::fs;
use std::path::{Path, PathBuf};

use crate::ffmpeg::{run_ffmpeg, run_ffprobe_json};

#[derive(Clone)]
pub struct ClipSource {
  pub input_path: String,
  pub start_time: Option<String>,
  pub end_time: Option<String>,
  pub order: i64,
}

pub fn clip_sources(sources: &[ClipSource], output_dir: &Path) -> Result<Vec<PathBuf>, String> {
  fs::create_dir_all(output_dir).map_err(|err| format!("Failed to create output dir: {}", err))?;

  let use_copy = can_concat_copy_sources(sources).unwrap_or(false);
  let mut outputs = Vec::new();
  for source in sources {
    let output_path = output_dir.join(format!("clip_{:03}.mp4", source.order));
    clip_single(source, &output_path, use_copy)?;
    outputs.push(output_path);
  }

  Ok(outputs)
}

pub fn merge_files(files: &[PathBuf], output_path: &Path) -> Result<(), String> {
  if let Some(parent) = output_path.parent() {
    fs::create_dir_all(parent).map_err(|err| format!("Failed to create output dir: {}", err))?;
  }

  let list_path = output_path.with_extension("txt");
  let list_content = files
    .iter()
    .map(|path| format!("file '{}'", path.to_string_lossy()))
    .collect::<Vec<_>>()
    .join("\n");

  fs::write(&list_path, list_content).map_err(|err| format!("Failed to write concat file: {}", err))?;

  let mut args = vec![
    "-f".to_string(),
    "concat".to_string(),
    "-safe".to_string(),
    "0".to_string(),
    "-i".to_string(),
    list_path.to_string_lossy().to_string(),
  ];

  args.push("-c".to_string());
  args.push("copy".to_string());

  args.push(output_path.to_string_lossy().to_string());

  run_ffmpeg(&args)?;
  let _ = fs::remove_file(list_path);
  Ok(())
}

struct VideoProbeInfo {
  codec_name: String,
  width: i64,
  height: i64,
  fps: f64,
  time_base: String,
}

struct AudioProbeInfo {
  codec_name: String,
  sample_rate: i64,
  channels: i64,
}

struct MediaProbeInfo {
  video: VideoProbeInfo,
  audio: Option<AudioProbeInfo>,
}

fn parse_fraction(value: &str) -> Option<f64> {
  let trimmed = value.trim();
  if trimmed.is_empty() {
    return None;
  }
  if let Some((num, den)) = trimmed.split_once('/') {
    let num: f64 = num.trim().parse().ok()?;
    let den: f64 = den.trim().parse().ok()?;
    if den == 0.0 {
      return None;
    }
    return Some(num / den);
  }
  trimmed.parse::<f64>().ok()
}

fn probe_media_info(path: &Path) -> Result<MediaProbeInfo, String> {
  let args = vec![
    "-v".to_string(),
    "error".to_string(),
    "-show_streams".to_string(),
    "-of".to_string(),
    "json".to_string(),
    path.to_string_lossy().to_string(),
  ];
  let args_line = args.join(" ");
  let data = run_ffprobe_json(&args)
    .map_err(|err| format!("ffprobe_fail path={} args={} err={}", path.to_string_lossy(), args_line, err))?;
  let streams = data
    .get("streams")
    .and_then(|value| value.as_array())
    .ok_or_else(|| "无法读取媒体流信息".to_string())?;
  let mut video: Option<VideoProbeInfo> = None;
  let mut audio: Option<AudioProbeInfo> = None;

  for stream in streams {
    let codec_type = stream
      .get("codec_type")
      .and_then(|value| value.as_str())
      .unwrap_or("");
    if codec_type == "video" && video.is_none() {
      let codec_name = stream
        .get("codec_name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
      let width = stream.get("width").and_then(|value| value.as_i64()).unwrap_or(0);
      let height = stream.get("height").and_then(|value| value.as_i64()).unwrap_or(0);
      let time_base = stream
        .get("time_base")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
      let avg_frame_rate = stream
        .get("avg_frame_rate")
        .and_then(|value| value.as_str())
        .unwrap_or("");
      let r_frame_rate = stream
        .get("r_frame_rate")
        .and_then(|value| value.as_str())
        .unwrap_or("");
      let fps = parse_fraction(avg_frame_rate)
        .filter(|value| *value > 0.0)
        .or_else(|| parse_fraction(r_frame_rate).filter(|value| *value > 0.0))
        .unwrap_or(0.0);
      video = Some(VideoProbeInfo {
        codec_name,
        width,
        height,
        fps,
        time_base,
      });
    }
    if codec_type == "audio" && audio.is_none() {
      let codec_name = stream
        .get("codec_name")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .to_string();
      let sample_rate = stream
        .get("sample_rate")
        .and_then(|value| value.as_str())
        .and_then(|value| value.parse::<i64>().ok())
        .unwrap_or(0);
      let channels = stream
        .get("channels")
        .and_then(|value| value.as_i64())
        .unwrap_or(0);
      audio = Some(AudioProbeInfo {
        codec_name,
        sample_rate,
        channels,
      });
    }
  }

  let video = video.ok_or_else(|| "缺少视频流".to_string())?;
  Ok(MediaProbeInfo { video, audio })
}

fn can_concat_copy(files: &[PathBuf]) -> Result<bool, String> {
  if files.is_empty() {
    return Ok(false);
  }
  let base = probe_media_info(&files[0])?;
  let base_audio = match base.audio {
    Some(audio) => audio,
    None => return Ok(false),
  };
  if base.video.codec_name.is_empty()
    || base.video.width <= 0
    || base.video.height <= 0
    || base.video.time_base.is_empty()
    || base.video.fps <= 0.0
    || base_audio.codec_name.is_empty()
    || base_audio.sample_rate <= 0
    || base_audio.channels <= 0
  {
    return Ok(false);
  }

  for path in files.iter().skip(1) {
    let current = probe_media_info(path)?;
    let current_audio = match current.audio {
      Some(audio) => audio,
      None => return Ok(false),
    };
    if current.video.codec_name != base.video.codec_name {
      return Ok(false);
    }
    if current.video.width != base.video.width || current.video.height != base.video.height {
      return Ok(false);
    }
    if current.video.time_base != base.video.time_base {
      return Ok(false);
    }
    if (current.video.fps - base.video.fps).abs() > 0.01 {
      return Ok(false);
    }
    if current_audio.codec_name != base_audio.codec_name {
      return Ok(false);
    }
    if current_audio.sample_rate != base_audio.sample_rate {
      return Ok(false);
    }
    if current_audio.channels != base_audio.channels {
      return Ok(false);
    }
  }
  Ok(true)
}

pub fn can_concat_copy_sources(sources: &[ClipSource]) -> Result<bool, String> {
  let files: Vec<PathBuf> = sources
    .iter()
    .map(|source| PathBuf::from(&source.input_path))
    .collect();
  can_concat_copy(&files)
}

fn probe_duration_seconds(path: &Path) -> Result<f64, String> {
  let args = vec![
    "-v".to_string(),
    "error".to_string(),
    "-show_entries".to_string(),
    "format=duration".to_string(),
    "-of".to_string(),
    "json".to_string(),
    path.to_string_lossy().to_string(),
  ];
  let data = run_ffprobe_json(&args)?;
  let duration = data
    .get("format")
    .and_then(|value| value.get("duration"))
    .and_then(|value| value.as_str())
    .and_then(|value| value.parse::<f64>().ok())
    .unwrap_or(0.0);
  if duration <= 0.0 {
    return Err("无法读取视频时长".to_string());
  }
  Ok(duration)
}

fn merge_last_short_segment(outputs: &mut Vec<PathBuf>, min_seconds: f64) -> Result<(), String> {
  if outputs.len() < 2 {
    return Ok(());
  }
  let last_index = outputs.len() - 1;
  let prev_index = outputs.len() - 2;
  let last_path = outputs[last_index].clone();
  let prev_path = outputs[prev_index].clone();
  let last_duration = probe_duration_seconds(&last_path)?;
  if last_duration >= min_seconds {
    return Ok(());
  }

  let output_dir = prev_path
    .parent()
    .ok_or_else(|| "无法读取分段目录".to_string())?;
  let list_path = output_dir.join("concat_tail.txt");
  let list_content = format!(
    "file '{}'\nfile '{}'",
    prev_path.to_string_lossy(),
    last_path.to_string_lossy()
  );
  fs::write(&list_path, list_content)
    .map_err(|err| format!("Failed to write concat file: {}", err))?;
  let merged_temp = output_dir.join("tail_merge.mp4");
  let args = vec![
    "-f".to_string(),
    "concat".to_string(),
    "-safe".to_string(),
    "0".to_string(),
    "-i".to_string(),
    list_path.to_string_lossy().to_string(),
    "-c".to_string(),
    "copy".to_string(),
    merged_temp.to_string_lossy().to_string(),
  ];
  run_ffmpeg(&args)?;
  let _ = fs::remove_file(&list_path);

  fs::rename(&merged_temp, &prev_path)
    .map_err(|err| format!("Failed to replace merged segment: {}", err))?;
  let _ = fs::remove_file(&last_path);
  outputs.pop();
  Ok(())
}

pub fn segment_file(
  input_path: &Path,
  output_dir: &Path,
  segment_seconds: i64,
) -> Result<Vec<PathBuf>, String> {
  fs::create_dir_all(output_dir).map_err(|err| format!("Failed to create segment dir: {}", err))?;

  let output_pattern = output_dir.join("part_%03d.mp4");
  let args = vec![
    "-i".to_string(),
    input_path.to_string_lossy().to_string(),
    "-c".to_string(),
    "copy".to_string(),
    "-f".to_string(),
    "segment".to_string(),
    "-segment_time".to_string(),
    segment_seconds.to_string(),
    "-reset_timestamps".to_string(),
    "1".to_string(),
    output_pattern.to_string_lossy().to_string(),
  ];

  run_ffmpeg(&args)?;

  let mut outputs: Vec<PathBuf> = fs::read_dir(output_dir)
    .map_err(|err| format!("Failed to read segment dir: {}", err))?
    .flatten()
    .map(|entry| entry.path())
    .filter(|path| path.is_file())
    .collect();

  outputs.sort();
  merge_last_short_segment(&mut outputs, 10.0)?;
  Ok(outputs)
}

fn clip_single(source: &ClipSource, output_path: &Path, use_copy: bool) -> Result<(), String> {
  let mut args = vec!["-i".to_string(), source.input_path.clone()];

  if let Some(start) = source.start_time.as_deref() {
    if !start.is_empty() && start != "00:00:00" {
      args.push("-ss".to_string());
      args.push(start.to_string());
    }
  }

  if let Some(end) = source.end_time.as_deref() {
    if !end.is_empty() && end != "00:00:00" {
      args.push("-to".to_string());
      args.push(end.to_string());
    }
  }

  if use_copy {
    args.extend(["-c".to_string(), "copy".to_string()]);
  } else {
    args.extend([
      "-vf".to_string(),
      "fps=60,pad=1920:1080:(ow-iw)/2:(oh-ih)/2".to_string(),
      "-af".to_string(),
      "aresample=48000:async=1:first_pts=0".to_string(),
      "-c:v".to_string(),
      "h264_videotoolbox".to_string(),
      "-b:v".to_string(),
      "5M".to_string(),
      "-c:a".to_string(),
      "aac".to_string(),
      "-ar".to_string(),
      "48000".to_string(),
    ]);
  }
  args.push(output_path.to_string_lossy().to_string());

  let args_line = args.join(" ");
  run_ffmpeg(&args).map_err(|err| {
    format!(
      "clip_ffmpeg_fail input={} output={} args={} err={}",
      source.input_path,
      output_path.to_string_lossy(),
      args_line,
      err
    )
  })
}
