use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::{
    path::Path,
    process::{Command, Stdio},
};

#[derive(Debug, Clone)]
pub struct VideoInfo {
    pub duration_seconds: f64,
    pub format_name: Option<String>,
    pub video_codec: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub audio_stream_count: usize,
}

pub fn probe_video(ffprobe_path: &Path, video_path: &Path) -> Result<VideoInfo> {
    validate_paths(ffprobe_path, video_path)?;

    let mut command = Command::new(ffprobe_path);

    command
        .args([
            "-v",
            "error",
            "-of",
            "json",
            "-show_entries",
            "format=duration,format_name:stream=codec_type,codec_name,width,height,duration",
        ])
        .arg(video_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    hide_console_window(&mut command);

    let output = command
        .output()
        .with_context(|| format!("Failed to launch FFprobe: {}", ffprobe_path.display()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();

        bail!(
            "FFprobe failed for '{}'.\n{}",
            video_path.display(),
            if stderr.is_empty() {
                "FFprobe returned no error details."
            } else {
                &stderr
            }
        );
    }

    parse_probe_output(&output.stdout).with_context(|| {
        format!(
            "Could not read video information from '{}'",
            video_path.display()
        )
    })
}

fn validate_paths(ffprobe_path: &Path, video_path: &Path) -> Result<()> {
    if !ffprobe_path.is_file() {
        bail!("FFprobe binary was not found: {}", ffprobe_path.display());
    }

    if !video_path.is_file() {
        bail!("Video file was not found: {}", video_path.display());
    }

    Ok(())
}

fn parse_probe_output(output: &[u8]) -> Result<VideoInfo> {
    let probe: ProbeOutput =
        serde_json::from_slice(output).context("FFprobe returned invalid JSON")?;

    let video_stream = probe
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("video"))
        .context("The selected file does not contain a video stream")?;

    let duration_seconds = parse_duration(probe.format.duration.as_deref())
        .or_else(|| {
            probe
                .streams
                .iter()
                .filter_map(|stream| parse_duration(stream.duration.as_deref()))
                .max_by(f64::total_cmp)
        })
        .context("FFprobe did not report a valid video duration")?;

    let video_codec = video_stream
        .codec_name
        .clone()
        .unwrap_or_else(|| "unknown".to_owned());

    let audio_stream_count = probe
        .streams
        .iter()
        .filter(|stream| stream.codec_type.as_deref() == Some("audio"))
        .count();

    Ok(VideoInfo {
        duration_seconds,
        format_name: probe.format.format_name,
        video_codec,
        width: video_stream.width,
        height: video_stream.height,
        audio_stream_count,
    })
}

fn parse_duration(value: Option<&str>) -> Option<f64> {
    let duration = value?.parse::<f64>().ok()?;

    if duration.is_finite() && duration > 0.0 {
        Some(duration)
    } else {
        None
    }
}

#[cfg(target_os = "windows")]
fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_console_window(_command: &mut Command) {}

#[derive(Debug, Deserialize)]
struct ProbeOutput {
    #[serde(default)]
    streams: Vec<ProbeStream>,

    #[serde(default)]
    format: ProbeFormat,
}

#[derive(Debug, Default, Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    format_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    duration: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_video_information() {
        let json = br#"
        {
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "h264",
                    "width": 1920,
                    "height": 1080,
                    "duration": "120.500000"
                },
                {
                    "codec_type": "audio",
                    "codec_name": "aac"
                }
            ],
            "format": {
                "duration": "120.500000",
                "format_name": "matroska,webm"
            }
        }
        "#;

        let info = parse_probe_output(json).unwrap();

        assert_eq!(info.duration_seconds, 120.5);
        assert_eq!(info.video_codec, "h264");
        assert_eq!(info.width, Some(1920));
        assert_eq!(info.height, Some(1080));
        assert_eq!(info.audio_stream_count, 1);
        assert_eq!(info.format_name.as_deref(), Some("matroska,webm"));
    }

    #[test]
    fn falls_back_to_stream_duration() {
        let json = br#"
        {
            "streams": [
                {
                    "codec_type": "video",
                    "codec_name": "hevc",
                    "width": 3840,
                    "height": 2160,
                    "duration": "42.25"
                }
            ],
            "format": {
                "duration": null,
                "format_name": "matroska,webm"
            }
        }
        "#;

        let info = parse_probe_output(json).unwrap();

        assert_eq!(info.duration_seconds, 42.25);
    }

    #[test]
    fn rejects_files_without_video_streams() {
        let json = br#"
        {
            "streams": [
                {
                    "codec_type": "audio",
                    "codec_name": "aac",
                    "duration": "20.0"
                }
            ],
            "format": {
                "duration": "20.0",
                "format_name": "mp3"
            }
        }
        "#;

        let error = parse_probe_output(json).unwrap_err();

        assert!(
            error
                .to_string()
                .contains("does not contain a video stream")
        );
    }

    #[test]
    fn rejects_invalid_duration() {
        assert_eq!(parse_duration(Some("0")), None);
        assert_eq!(parse_duration(Some("-2")), None);
        assert_eq!(parse_duration(Some("N/A")), None);
        assert_eq!(parse_duration(None), None);
        assert_eq!(parse_duration(Some("10.5")), Some(10.5));
    }
}
