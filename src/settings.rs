use crate::model::{AccelerationPreference, AppMode, OutputFormat, QualityPreset};
use anyhow::{Context, Result};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
};

const SETTINGS_FILE_NAME: &str = "settings.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AppSettings {
    pub selected_mode: AppMode,

    pub last_video_directory: Option<PathBuf>,
    pub last_subtitle_directory: Option<PathBuf>,
    pub last_batch_input_directory: Option<PathBuf>,
    pub last_output_directory: Option<PathBuf>,

    pub prefix: String,
    pub postfix: String,

    pub output_format: OutputFormat,
    pub quality: QualityPreset,
    pub acceleration: AccelerationPreference,
    pub overwrite_existing: bool,

    pub custom_ffmpeg_directory: Option<PathBuf>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            selected_mode: AppMode::Single,

            last_video_directory: None,
            last_subtitle_directory: None,
            last_batch_input_directory: None,
            last_output_directory: None,

            prefix: String::new(),
            postfix: "_hardsub".to_owned(),

            output_format: OutputFormat::Mp4,
            quality: QualityPreset::Balanced,
            acceleration: AccelerationPreference::Auto,
            overwrite_existing: false,

            custom_ffmpeg_directory: None,
        }
    }
}

impl AppSettings {
    pub fn load() -> Result<Self> {
        let settings_path = settings_file_path()?;

        if !settings_path.is_file() {
            tracing::info!(
                path = %settings_path.display(),
                "Settings file does not exist, using defaults"
            );

            return Ok(Self::default());
        }

        load_from_path(&settings_path)
    }

    pub fn load_or_default() -> Self {
        match Self::load() {
            Ok(settings) => settings,

            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "Could not load settings, using defaults"
                );

                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<()> {
        let settings_path = settings_file_path()?;
        save_to_path(self, &settings_path)
    }
}

pub fn settings_file_path() -> Result<PathBuf> {
    let project_directories = ProjectDirs::from("net", "Rambod", "BurnSubs")
        .context("Could not determine the application settings directory")?;

    Ok(project_directories.config_dir().join(SETTINGS_FILE_NAME))
}

fn load_from_path(path: &Path) -> Result<AppSettings> {
    let json = fs::read_to_string(path)
        .with_context(|| format!("Could not read settings file: {}", path.display()))?;

    serde_json::from_str(&json)
        .with_context(|| format!("Settings file contains invalid JSON: {}", path.display()))
}

fn save_to_path(settings: &AppSettings, path: &Path) -> Result<()> {
    let parent_directory = path
        .parent()
        .context("Settings path does not have a parent directory")?;

    fs::create_dir_all(parent_directory).with_context(|| {
        format!(
            "Could not create settings directory: {}",
            parent_directory.display()
        )
    })?;

    let json = serde_json::to_string_pretty(settings)
        .context("Could not serialize application settings")?;

    let temporary_path = path.with_extension("json.tmp");

    fs::write(&temporary_path, json).with_context(|| {
        format!(
            "Could not write temporary settings file: {}",
            temporary_path.display()
        )
    })?;

    if path.exists() {
        fs::remove_file(path).with_context(|| {
            format!(
                "Could not replace existing settings file: {}",
                path.display()
            )
        })?;
    }

    fs::rename(&temporary_path, path)
        .with_context(|| format!("Could not finalize settings file: {}", path.display()))?;

    tracing::debug!(
        path = %path.display(),
        "Application settings saved"
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn serializes_and_loads_settings() {
        let path = temporary_settings_path("round-trip");

        let settings = AppSettings {
            selected_mode: AppMode::Batch,
            last_video_directory: Some(PathBuf::from("videos")),
            last_subtitle_directory: Some(PathBuf::from("subtitles")),
            last_batch_input_directory: Some(PathBuf::from("batch")),
            last_output_directory: Some(PathBuf::from("output")),
            prefix: "processed_".to_owned(),
            postfix: "_burned".to_owned(),
            output_format: OutputFormat::Mkv,
            quality: QualityPreset::High,
            acceleration: AccelerationPreference::Nvidia,
            overwrite_existing: true,
            custom_ffmpeg_directory: Some(PathBuf::from("tools")),
        };

        save_to_path(&settings, &path).unwrap();

        let loaded = load_from_path(&path).unwrap();

        assert_eq!(loaded.selected_mode, AppMode::Batch);
        assert_eq!(loaded.prefix, "processed_");
        assert_eq!(loaded.postfix, "_burned");
        assert_eq!(loaded.output_format, OutputFormat::Mkv);
        assert_eq!(loaded.quality, QualityPreset::High);
        assert_eq!(loaded.acceleration, AccelerationPreference::Nvidia);
        assert!(loaded.overwrite_existing);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn missing_fields_use_defaults() {
        let json = r#"
        {
            "prefix": "custom_"
        }
        "#;

        let settings: AppSettings = serde_json::from_str(json).unwrap();

        assert_eq!(settings.prefix, "custom_");
        assert_eq!(settings.postfix, "_hardsub");
        assert_eq!(settings.output_format, OutputFormat::Mp4);
        assert_eq!(settings.quality, QualityPreset::Balanced);
        assert_eq!(settings.acceleration, AccelerationPreference::Auto);
        assert!(!settings.overwrite_existing);
    }

    fn temporary_settings_path(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);

        std::env::temp_dir().join(format!(
            "burnsubs-settings-test-{}-{name}-{id}.json",
            std::process::id()
        ))
    }
}
