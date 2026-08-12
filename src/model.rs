use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppMode {
    #[default]
    Single,
    Batch,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputFormat {
    #[default]
    Mp4,
    Mkv,
}

impl OutputFormat {
    pub const ALL: [Self; 2] = [Self::Mp4, Self::Mkv];

    pub const fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mkv => "MKV",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QualityPreset {
    High,
    #[default]
    Balanced,
    SmallerFile,
}

impl QualityPreset {
    pub const ALL: [Self; 3] = [Self::High, Self::Balanced, Self::SmallerFile];

    pub const fn label(self) -> &'static str {
        match self {
            Self::High => "High quality",
            Self::Balanced => "Balanced",
            Self::SmallerFile => "Smaller file",
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccelerationPreference {
    #[default]
    Auto,
    Cpu,
    Nvidia,
    IntelQuickSync,
    Amd,
    Vaapi,
    VideoToolbox,
}

impl AccelerationPreference {
    pub fn choices_for_current_platform() -> &'static [Self] {
        #[cfg(target_os = "windows")]
        {
            &[
                Self::Auto,
                Self::Cpu,
                Self::Nvidia,
                Self::IntelQuickSync,
                Self::Amd,
            ]
        }

        #[cfg(target_os = "linux")]
        {
            &[
                Self::Auto,
                Self::Cpu,
                Self::Nvidia,
                Self::IntelQuickSync,
                Self::Vaapi,
            ]
        }

        #[cfg(target_os = "macos")]
        {
            &[Self::Auto, Self::Cpu, Self::VideoToolbox]
        }

        #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
        {
            &[Self::Auto, Self::Cpu]
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Automatic (recommended)",
            Self::Cpu => "CPU (libx264)",
            Self::Nvidia => "NVIDIA NVENC",
            Self::IntelQuickSync => "Intel Quick Sync",
            Self::Amd => "AMD AMF",
            Self::Vaapi => "VA-API (Intel / AMD)",
            Self::VideoToolbox => "Apple VideoToolbox",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::Auto => "Tests installed GPU encoders and safely falls back to the CPU.",
            Self::Cpu => "Uses the slower software encoder with the most consistent quality.",
            Self::Nvidia => "Uses the NVIDIA H.264 hardware encoder when the driver supports it.",
            Self::IntelQuickSync => "Uses Intel Quick Sync H.264 hardware encoding.",
            Self::Amd => "Uses AMD Advanced Media Framework on Windows.",
            Self::Vaapi => "Uses VA-API on Linux for Intel and AMD graphics.",
            Self::VideoToolbox => "Uses Apple's H.264 hardware encoder on macOS.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobStatus {
    Queued,
    Probing,
    Encoding,
    Completed,
    Failed(String),
    Cancelled,
}

impl JobStatus {
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Queued => "Queued",
            Self::Probing => "Reading video information",
            Self::Encoding => "Encoding",
            Self::Completed => "Completed",
            Self::Failed(_) => "Failed",
            Self::Cancelled => "Cancelled",
        }
    }

    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed(_) | Self::Cancelled)
    }
}

#[derive(Debug, Clone)]
pub struct EncodeJob {
    pub id: u64,
    pub video_path: PathBuf,
    pub subtitle_path: PathBuf,
    pub output_path: PathBuf,
    pub status: JobStatus,

    /// Encoding progress from 0.0 to 1.0.
    pub progress: f32,
    pub encoder_name: Option<String>,
    pub fallback_note: Option<String>,
}

impl EncodeJob {
    pub fn new(id: u64, video_path: PathBuf, subtitle_path: PathBuf, output_path: PathBuf) -> Self {
        Self {
            id,
            video_path,
            subtitle_path,
            output_path,
            status: JobStatus::Queued,
            progress: 0.0,
            encoder_name: None,
            fallback_note: None,
        }
    }

    pub fn set_progress(&mut self, progress: f32) {
        self.progress = progress.clamp(0.0, 1.0);
    }

    pub fn video_name(&self) -> String {
        file_name_or_path(&self.video_path)
    }

    pub fn subtitle_name(&self) -> String {
        file_name_or_path(&self.subtitle_path)
    }

    pub fn output_name(&self) -> String {
        file_name_or_path(&self.output_path)
    }
}

#[derive(Debug, Clone)]
pub struct EncodeSettings {
    pub prefix: String,
    pub postfix: String,
    pub output_directory: PathBuf,
    pub output_format: OutputFormat,
    pub quality: QualityPreset,
    pub acceleration: AccelerationPreference,
    pub overwrite_existing: bool,
    pub custom_ffmpeg_directory: Option<PathBuf>,
}

impl EncodeSettings {
    pub fn create_output_path(&self, video_path: &Path) -> PathBuf {
        let original_stem = video_path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or("video");

        let output_name = format!(
            "{}{}{}.{}",
            self.prefix,
            original_stem,
            self.postfix,
            self.output_format.extension()
        );

        self.output_directory.join(output_name)
    }
}

fn file_name_or_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_output_path_with_prefix_and_postfix() {
        let settings = EncodeSettings {
            prefix: "processed_".to_owned(),
            postfix: "_hardsub".to_owned(),
            output_directory: PathBuf::from("output"),
            output_format: OutputFormat::Mp4,
            quality: QualityPreset::Balanced,
            acceleration: AccelerationPreference::Auto,
            overwrite_existing: false,
            custom_ffmpeg_directory: None,
        };

        let result = settings.create_output_path(Path::new("movie.mkv"));

        assert_eq!(
            result,
            PathBuf::from("output").join("processed_movie_hardsub.mp4")
        );
    }

    #[test]
    fn clamps_job_progress() {
        let mut job = EncodeJob::new(
            1,
            PathBuf::from("movie.mkv"),
            PathBuf::from("movie.srt"),
            PathBuf::from("movie_hardsub.mp4"),
        );

        job.set_progress(1.5);
        assert_eq!(job.progress, 1.0);

        job.set_progress(-0.5);
        assert_eq!(job.progress, 0.0);
    }

    #[test]
    fn reports_terminal_statuses() {
        assert!(JobStatus::Completed.is_terminal());
        assert!(JobStatus::Cancelled.is_terminal());
        assert!(JobStatus::Failed("error".to_owned()).is_terminal());
        assert!(!JobStatus::Encoding.is_terminal());
    }
}
