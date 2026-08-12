use crate::model::{AppMode, EncodeSettings, OutputFormat, QualityPreset};
use eframe::egui::{self, RichText};
use rfd::FileDialog;
use std::path::{Path, PathBuf};

const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "mov", "webm", "m4v"];

pub struct BurnSubsApp {
    selected_mode: AppMode,

    single_video: Option<PathBuf>,
    single_subtitle: Option<PathBuf>,

    batch_input_directory: Option<PathBuf>,
    output_directory: Option<PathBuf>,

    prefix: String,
    postfix: String,
    output_format: OutputFormat,
    quality: QualityPreset,
    overwrite_existing: bool,

    status: Option<AppStatus>,
}

#[derive(Debug)]
enum AppStatus {
    Info(String),
    Error(String),
}

impl BurnSubsApp {
    pub fn new(_creation_context: &eframe::CreationContext<'_>) -> Self {
        Self {
            selected_mode: AppMode::Single,

            single_video: None,
            single_subtitle: None,

            batch_input_directory: None,
            output_directory: None,

            prefix: String::new(),
            postfix: "_hardsub".to_owned(),
            output_format: OutputFormat::Mp4,
            quality: QualityPreset::Balanced,
            overwrite_existing: false,

            status: None,
        }
    }

    fn draw_header(&self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.heading("BurnSubs");
            ui.label("Burn SRT subtitles permanently into video files.");
        });
    }

    fn draw_mode_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut self.selected_mode,
                AppMode::Single,
                "Single video",
            );

            ui.selectable_value(
                &mut self.selected_mode,
                AppMode::Batch,
                "Batch folder",
            );
        });
    }

    fn draw_single_mode(&mut self, ui: &mut egui::Ui) {
        ui.heading("Single video");

        if path_picker_row(
            ui,
            "Video",
            self.single_video.as_deref(),
            "Select video...",
        ) {
            self.select_video();
        }

        ui.add_space(8.0);

        if path_picker_row(
            ui,
            "Subtitle",
            self.single_subtitle.as_deref(),
            "Select SRT...",
        ) {
            self.select_subtitle();
        }

        if let Some(output_path) = self.single_output_path() {
            ui.add_space(10.0);
            ui.label(RichText::new("Output preview").strong());
            ui.indent("single_output_preview", |ui| {
                ui.label(output_path.to_string_lossy());
            });
        }
    }

    fn draw_batch_mode(&mut self, ui: &mut egui::Ui) {
        ui.heading("Batch folder");

        if path_picker_row(
            ui,
            "Input folder",
            self.batch_input_directory.as_deref(),
            "Select folder...",
        ) {
            self.select_batch_input_directory();
        }

        ui.add_space(8.0);
        ui.label(
            "Videos will be matched with SRT files that have the same base filename.",
        );
    }

    fn draw_output_settings(&mut self, ui: &mut egui::Ui) {
        ui.heading("Output");

        if path_picker_row(
            ui,
            "Output folder",
            self.output_directory.as_deref(),
            "Select folder...",
        ) {
            self.select_output_directory();
        }

        ui.add_space(8.0);

        egui::Grid::new("output_settings_grid")
            .num_columns(2)
            .spacing([16.0, 8.0])
            .show(ui, |ui| {
                ui.label("Prefix");
                ui.text_edit_singleline(&mut self.prefix);
                ui.end_row();

                ui.label("Postfix");
                ui.text_edit_singleline(&mut self.postfix);
                ui.end_row();

                ui.label("Format");
                egui::ComboBox::from_id_salt("output_format")
                    .selected_text(self.output_format.label())
                    .show_ui(ui, |ui| {
                        for format in OutputFormat::ALL {
                            ui.selectable_value(
                                &mut self.output_format,
                                format,
                                format.label(),
                            );
                        }
                    });
                ui.end_row();

                ui.label("Quality");
                egui::ComboBox::from_id_salt("quality_preset")
                    .selected_text(self.quality.label())
                    .show_ui(ui, |ui| {
                        for quality in QualityPreset::ALL {
                            ui.selectable_value(
                                &mut self.quality,
                                quality,
                                quality.label(),
                            );
                        }
                    });
                ui.end_row();
            });

        ui.add_space(6.0);
        ui.checkbox(
            &mut self.overwrite_existing,
            "Overwrite existing output files",
        );
    }

    fn draw_action_area(&mut self, ui: &mut egui::Ui) {
        let can_validate = match self.selected_mode {
            AppMode::Single => {
                self.single_video.is_some()
                    && self.single_subtitle.is_some()
                    && self.output_directory.is_some()
            }
            AppMode::Batch => {
                self.batch_input_directory.is_some()
                    && self.output_directory.is_some()
            }
        };

        if ui
            .add_enabled(
                can_validate,
                egui::Button::new("Check configuration"),
            )
            .clicked()
        {
            self.status = Some(match self.validate_configuration() {
                Ok(message) => AppStatus::Info(message),
                Err(message) => AppStatus::Error(message),
            });
        }

        if !can_validate {
            ui.label(
                RichText::new(
                    "Select the required input files and output folder.",
                )
                .weak(),
            );
        }

        if let Some(status) = &self.status {
            ui.add_space(8.0);

            match status {
                AppStatus::Info(message) => {
                    ui.label(message);
                }
                AppStatus::Error(message) => {
                    ui.label(
                        RichText::new(message)
                            .color(ui.visuals().error_fg_color),
                    );
                }
            }
        }
    }

    fn select_video(&mut self) {
        let mut dialog = FileDialog::new()
            .set_title("Select a video")
            .add_filter("Video files", VIDEO_EXTENSIONS);

        if let Some(current_directory) = selected_parent(&self.single_video) {
            dialog = dialog.set_directory(current_directory);
        }

        let Some(video_path) = dialog.pick_file() else {
            return;
        };

        if self.output_directory.is_none() {
            self.output_directory = video_path.parent().map(Path::to_path_buf);
        }

        self.single_subtitle = find_matching_srt(&video_path);
        self.single_video = Some(video_path);

        self.status = match &self.single_subtitle {
            Some(path) => Some(AppStatus::Info(format!(
                "Matching subtitle detected: {}",
                path.to_string_lossy()
            ))),
            None => None,
        };
    }

    fn select_subtitle(&mut self) {
        let mut dialog = FileDialog::new()
            .set_title("Select an SRT subtitle")
            .add_filter("SubRip subtitle", &["srt"]);

        if let Some(current_directory) = selected_parent(&self.single_video)
            .or_else(|| selected_parent(&self.single_subtitle))
        {
            dialog = dialog.set_directory(current_directory);
        }

        if let Some(path) = dialog.pick_file() {
            self.single_subtitle = Some(path);
            self.status = None;
        }
    }

    fn select_batch_input_directory(&mut self) {
        let mut dialog = FileDialog::new().set_title("Select the batch folder");

        if let Some(current_directory) = self.batch_input_directory.as_deref() {
            dialog = dialog.set_directory(current_directory);
        }

        let Some(path) = dialog.pick_folder() else {
            return;
        };

        if self.output_directory.is_none() {
            self.output_directory = Some(path.clone());
        }

        self.batch_input_directory = Some(path);
        self.status = None;
    }

    fn select_output_directory(&mut self) {
        let mut dialog = FileDialog::new().set_title("Select the output folder");

        if let Some(current_directory) = self.output_directory.as_deref() {
            dialog = dialog.set_directory(current_directory);
        }

        if let Some(path) = dialog.pick_folder() {
            self.output_directory = Some(path);
            self.status = None;
        }
    }

    fn encode_settings(&self) -> Option<EncodeSettings> {
        Some(EncodeSettings {
            prefix: self.prefix.clone(),
            postfix: self.postfix.clone(),
            output_directory: self.output_directory.clone()?,
            output_format: self.output_format,
            quality: self.quality,
            overwrite_existing: self.overwrite_existing,
        })
    }

    fn single_output_path(&self) -> Option<PathBuf> {
        let video_path = self.single_video.as_deref()?;
        let settings = self.encode_settings()?;

        Some(settings.create_output_path(video_path))
    }

    fn validate_configuration(&self) -> Result<String, String> {
        let output_directory = self
            .output_directory
            .as_deref()
            .ok_or_else(|| "Select an output folder.".to_owned())?;

        if !output_directory.is_dir() {
            return Err("The selected output folder does not exist.".to_owned());
        }

        match self.selected_mode {
            AppMode::Single => {
                let video_path = self
                    .single_video
                    .as_deref()
                    .ok_or_else(|| "Select a video file.".to_owned())?;

                let subtitle_path = self
                    .single_subtitle
                    .as_deref()
                    .ok_or_else(|| "Select an SRT subtitle file.".to_owned())?;

                if !video_path.is_file() {
                    return Err("The selected video file no longer exists.".to_owned());
                }

                if !subtitle_path.is_file() {
                    return Err(
                        "The selected subtitle file no longer exists.".to_owned(),
                    );
                }

                let output_path = self.single_output_path().ok_or_else(|| {
                    "Could not generate the output filename.".to_owned()
                })?;

                if output_path == video_path {
                    return Err(
                        "The output path cannot replace the input video directly."
                            .to_owned(),
                    );
                }

                if output_path.exists() && !self.overwrite_existing {
                    return Err(format!(
                        "The output already exists: {}",
                        output_path.to_string_lossy()
                    ));
                }

                Ok(format!(
                    "Configuration is valid. Output: {}",
                    output_path.to_string_lossy()
                ))
            }
            AppMode::Batch => {
                let input_directory = self
                    .batch_input_directory
                    .as_deref()
                    .ok_or_else(|| "Select a batch input folder.".to_owned())?;

                if !input_directory.is_dir() {
                    return Err(
                        "The selected batch input folder no longer exists."
                            .to_owned(),
                    );
                }

                Ok(
                    "Configuration is valid. Batch scanning will be connected next."
                        .to_owned(),
                )
            }
        }
    }
}

impl eframe::App for BurnSubsApp {
    fn update(
        &mut self,
        context: &egui::Context,
        _frame: &mut eframe::Frame,
    ) {
        egui::CentralPanel::default().show(context, |ui| {
            self.draw_header(ui);

            ui.add_space(12.0);
            self.draw_mode_selector(ui);
            ui.separator();

            match self.selected_mode {
                AppMode::Single => self.draw_single_mode(ui),
                AppMode::Batch => self.draw_batch_mode(ui),
            }

            ui.add_space(16.0);
            ui.separator();
            self.draw_output_settings(ui);

            ui.add_space(16.0);
            ui.separator();
            self.draw_action_area(ui);
        });
    }
}

fn path_picker_row(
    ui: &mut egui::Ui,
    label: &str,
    path: Option<&Path>,
    button_text: &str,
) -> bool {
    let clicked = ui
        .horizontal(|ui| {
            ui.label(RichText::new(label).strong());
            ui.button(button_text).clicked()
        })
        .inner;

    ui.indent(format!("{label}_selected_path"), |ui| match path {
        Some(path) => {
            ui.label(path.to_string_lossy());
        }
        None => {
            ui.label(RichText::new("Nothing selected").weak());
        }
    });

    clicked
}

fn selected_parent(path: &Option<PathBuf>) -> Option<&Path> {
    path.as_deref().and_then(Path::parent)
}

fn find_matching_srt(video_path: &Path) -> Option<PathBuf> {
    let direct_candidate = video_path.with_extension("srt");

    if direct_candidate.is_file() {
        return Some(direct_candidate);
    }

    let parent = video_path.parent()?;
    let video_stem = video_path.file_stem()?.to_string_lossy();

    let entries = std::fs::read_dir(parent).ok()?;

    for entry in entries.flatten() {
        let path = entry.path();

        let is_srt = path
            .extension()
            .is_some_and(|extension| {
                extension.to_string_lossy().eq_ignore_ascii_case("srt")
            });

        if !is_srt {
            continue;
        }

        let has_matching_stem = path
            .file_stem()
            .is_some_and(|stem| {
                stem.to_string_lossy()
                    .eq_ignore_ascii_case(&video_stem)
            });

        if has_matching_stem {
            return Some(path);
        }
    }

    None
}
