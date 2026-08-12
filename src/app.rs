use crate::{
    batch::{BatchScanResult, VIDEO_EXTENSIONS, scan_batch_folder},
    model::{
        AccelerationPreference, AppMode, EncodeJob, EncodeSettings, JobStatus, OutputFormat,
        QualityPreset,
    },
    settings::AppSettings,
    worker::{WorkerEvent, WorkerHandle},
};
use eframe::egui::{self, RichText};
use rfd::FileDialog;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

const ACCENT: egui::Color32 = egui::Color32::from_rgb(72, 128, 255);
const SUCCESS: egui::Color32 = egui::Color32::from_rgb(76, 190, 132);
const WARNING: egui::Color32 = egui::Color32::from_rgb(236, 176, 72);
const ERROR: egui::Color32 = egui::Color32::from_rgb(238, 92, 102);
const PANEL_FILL: egui::Color32 = egui::Color32::from_rgb(25, 28, 34);
const WORKSPACE_FILL: egui::Color32 = egui::Color32::from_rgb(20, 22, 27);

pub struct BurnSubsApp {
    selected_mode: AppMode,

    single_video: Option<PathBuf>,
    single_subtitle: Option<PathBuf>,
    batch_input_directory: Option<PathBuf>,
    output_directory: Option<PathBuf>,

    last_video_directory: Option<PathBuf>,
    last_subtitle_directory: Option<PathBuf>,
    last_batch_input_directory: Option<PathBuf>,
    last_output_directory: Option<PathBuf>,
    custom_ffmpeg_directory: Option<PathBuf>,

    prefix: String,
    postfix: String,
    output_format: OutputFormat,
    quality: QualityPreset,
    acceleration: AccelerationPreference,
    overwrite_existing: bool,

    worker: Option<WorkerHandle>,
    jobs: Vec<EncodeJob>,
    batch_preview: Option<BatchScanResult>,
    queue_total_jobs: usize,

    status: Option<AppStatus>,
}

#[derive(Debug)]
enum AppStatus {
    Info(String),
    Warning(String),
    Error(String),
}

impl BurnSubsApp {
    pub fn new(creation_context: &eframe::CreationContext<'_>) -> Self {
        configure_interface(&creation_context.egui_ctx);
        let settings = AppSettings::load_or_default();

        let batch_input_directory = existing_directory(settings.last_batch_input_directory.clone());

        let output_directory = existing_directory(settings.last_output_directory.clone());

        let (worker, status) = match WorkerHandle::spawn() {
            Ok(worker) => (Some(worker), None),
            Err(error) => (
                None,
                Some(AppStatus::Error(format!(
                    "Could not create the encoding worker: {error}"
                ))),
            ),
        };

        Self {
            selected_mode: settings.selected_mode,

            single_video: None,
            single_subtitle: None,
            batch_input_directory,
            output_directory,

            last_video_directory: settings.last_video_directory,
            last_subtitle_directory: settings.last_subtitle_directory,
            last_batch_input_directory: settings.last_batch_input_directory,
            last_output_directory: settings.last_output_directory,
            custom_ffmpeg_directory: settings.custom_ffmpeg_directory,

            prefix: settings.prefix,
            postfix: settings.postfix,
            output_format: settings.output_format,
            quality: settings.quality,
            acceleration: settings.acceleration,
            overwrite_existing: settings.overwrite_existing,

            worker,
            jobs: Vec::new(),
            batch_preview: None,
            queue_total_jobs: 0,

            status,
        }
    }

    fn draw_header(&mut self, ui: &mut egui::Ui) {
        egui::containers::Sides::new()
            .height(38.0)
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    ui.label(RichText::new("BurnSubs").size(20.0).strong());
                    ui.label(RichText::new("Hard subtitle studio").weak());
                },
                |ui| {
                    ui.add_enabled_ui(!self.is_busy(), |ui| self.draw_mode_selector(ui));
                },
            );
    }

    fn draw_mode_selector(&mut self, ui: &mut egui::Ui) {
        let previous_mode = self.selected_mode;

        ui.selectable_value(&mut self.selected_mode, AppMode::Batch, "Batch");
        ui.selectable_value(&mut self.selected_mode, AppMode::Single, "Single");

        if self.selected_mode != previous_mode {
            self.status = None;
            self.jobs.clear();
        }
    }

    fn draw_single_mode(&mut self, ui: &mut egui::Ui) {
        section_title(ui, "Source");

        if path_picker_row(ui, "Video", self.single_video.as_deref(), "Browse") {
            self.select_video();
        }

        if path_picker_row(ui, "Subtitle", self.single_subtitle.as_deref(), "Browse") {
            self.select_subtitle();
        }
    }

    fn draw_batch_mode(&mut self, ui: &mut egui::Ui) {
        section_title(ui, "Source");

        if path_picker_row(
            ui,
            "Folder",
            self.batch_input_directory.as_deref(),
            "Browse",
        ) {
            self.select_batch_input_directory();
        }

        let can_scan = self.batch_input_directory.is_some() && self.output_directory.is_some();
        egui::containers::Sides::new()
            .height(30.0)
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    ui.label(RichText::new("Matches videos with same-name SRT files.").weak());
                },
                |ui| {
                    if ui
                        .add_enabled(can_scan, egui::Button::new("Scan now"))
                        .clicked()
                    {
                        self.scan_batch_preview();
                    }
                },
            );
    }

    fn draw_output_settings(&mut self, ui: &mut egui::Ui) {
        section_title(ui, "Output");

        if path_picker_row(ui, "Folder", self.output_directory.as_deref(), "Browse") {
            self.select_output_directory();
        }

        let previous_prefix = self.prefix.clone();
        let previous_postfix = self.postfix.clone();
        let previous_format = self.output_format;
        let previous_quality = self.quality;
        let previous_acceleration = self.acceleration;
        let previous_ffmpeg_directory = self.custom_ffmpeg_directory.clone();
        let previous_overwrite = self.overwrite_existing;

        ui.columns_const(|[left, right]| {
            compact_text_field(left, "Prefix", &mut self.prefix);
            compact_text_field(right, "Postfix", &mut self.postfix);
        });

        ui.columns_const(|[left, right]| {
            field_label(left, "Format");
            egui::ComboBox::from_id_salt("output_format")
                .width(left.available_width())
                .selected_text(self.output_format.label())
                .show_ui(left, |ui| {
                    for format in OutputFormat::ALL {
                        ui.selectable_value(&mut self.output_format, format, format.label());
                    }
                });

            field_label(right, "Quality");
            egui::ComboBox::from_id_salt("quality_preset")
                .width(right.available_width())
                .selected_text(self.quality.label())
                .show_ui(right, |ui| {
                    for quality in QualityPreset::ALL {
                        ui.selectable_value(&mut self.quality, quality, quality.label());
                    }
                });
        });

        field_label(ui, "Video encoder");
        egui::ComboBox::from_id_salt("hardware_acceleration")
            .width(ui.available_width())
            .truncate()
            .selected_text(self.acceleration.label())
            .show_ui(ui, |ui| {
                for preference in AccelerationPreference::choices_for_current_platform() {
                    ui.selectable_value(&mut self.acceleration, *preference, preference.label());
                }
            })
            .response
            .on_hover_text(self.acceleration.description());

        ui.checkbox(
            &mut self.overwrite_existing,
            "Replace existing output files",
        );

        if let Some(output_path) = self.single_output_path() {
            let output_text = output_path.to_string_lossy();
            ui.add(
                egui::Label::new(RichText::new(format!("Will create  {output_text}")).weak())
                    .truncate(),
            )
            .on_hover_text(output_text);
        }

        egui::CollapsingHeader::new("Advanced")
            .default_open(false)
            .show(ui, |ui| {
                let ffmpeg_text = self.custom_ffmpeg_directory.as_deref().map_or_else(
                    || "Bundled / system FFmpeg".to_owned(),
                    |path| path.to_string_lossy().into_owned(),
                );
                egui::containers::Sides::new()
                    .height(30.0)
                    .shrink_left()
                    .truncate()
                    .show(
                        ui,
                        |ui| {
                            ui.add(egui::Label::new(ffmpeg_text.clone()).truncate())
                                .on_hover_text(ffmpeg_text);
                        },
                        |ui| {
                            if self.custom_ffmpeg_directory.is_some()
                                && ui.small_button("Reset").clicked()
                            {
                                self.custom_ffmpeg_directory = None;
                            }
                            if ui.small_button("Choose folder").clicked() {
                                self.select_ffmpeg_directory();
                            }
                        },
                    );
            });

        let settings_changed = previous_prefix != self.prefix
            || previous_postfix != self.postfix
            || previous_format != self.output_format
            || previous_quality != self.quality
            || previous_acceleration != self.acceleration
            || previous_ffmpeg_directory != self.custom_ffmpeg_directory
            || previous_overwrite != self.overwrite_existing;

        if settings_changed {
            self.batch_preview = None;
            self.status = None;
        }
    }

    fn draw_action_area(&mut self, ui: &mut egui::Ui) {
        let worker_available = self.worker.is_some();
        let is_busy = self.is_busy();
        let can_start = worker_available && self.has_required_selection();
        let (status_text, status_color) = self.footer_status(worker_available, can_start, is_busy);

        egui::containers::Sides::new()
            .height(42.0)
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    status_dot(ui, status_color);
                    ui.add(egui::Label::new(status_text.clone()).truncate())
                        .on_hover_text(status_text);
                },
                |ui| {
                    if is_busy {
                        if ui
                            .add_sized([126.0, 36.0], egui::Button::new("Cancel"))
                            .clicked()
                        {
                            if let Some(worker) = &self.worker {
                                worker.cancel();
                            }
                            self.status = Some(AppStatus::Info(
                                "Cancellation requested. Waiting for FFmpeg to stop.".to_owned(),
                            ));
                        }
                    } else {
                        let button_text = match self.selected_mode {
                            AppMode::Single => "Start encoding",
                            AppMode::Batch => "Start batch",
                        };
                        let button = egui::Button::new(RichText::new(button_text).strong())
                            .fill(ACCENT)
                            .min_size(egui::vec2(140.0, 36.0));
                        if ui.add_enabled(can_start, button).clicked() {
                            self.start_encoding();
                        }
                    }
                },
            );
    }

    fn draw_monitor(&self, ui: &mut egui::Ui) {
        egui::containers::Sides::new()
            .height(34.0)
            .shrink_left()
            .truncate()
            .show(
                ui,
                |ui| {
                    ui.label(RichText::new(self.monitor_title()).size(17.0).strong());
                },
                |ui| {
                    if !self.jobs.is_empty() {
                        ui.label(
                            RichText::new(format!(
                                "{} / {} finished",
                                self.finished_job_count(),
                                self.queue_total_jobs.max(self.jobs.len())
                            ))
                            .weak(),
                        );
                    }
                },
            );

        if !self.jobs.is_empty() {
            ui.add(
                egui::ProgressBar::new(self.overall_progress())
                    .desired_width(ui.available_width())
                    .corner_radius(4)
                    .fill(ACCENT)
                    .show_percentage(),
            );
            ui.add_space(8.0);
            self.draw_queue_rows(ui);
        } else if self.selected_mode == AppMode::Batch {
            match &self.batch_preview {
                Some(preview) => draw_batch_preview(ui, preview),
                None => empty_monitor(
                    ui,
                    "Batch preview",
                    "Choose a folder and scan it to review matched videos before encoding.",
                ),
            }
        } else {
            empty_monitor(
                ui,
                "Ready when you are",
                "Choose a video and subtitle. The active queue and progress will appear here.",
            );
        }
    }

    fn draw_queue_rows(&self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .id_salt("queue_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for job in &self.jobs {
                    queue_row(ui, job);
                    ui.separator();
                }
            });
    }

    fn monitor_title(&self) -> &'static str {
        if self.jobs.is_empty() && self.selected_mode == AppMode::Batch {
            "Batch preview"
        } else {
            "Encoding queue"
        }
    }

    fn footer_status(
        &self,
        worker_available: bool,
        can_start: bool,
        is_busy: bool,
    ) -> (String, egui::Color32) {
        if let Some(status) = &self.status {
            return match status {
                AppStatus::Info(message) => (message.clone(), ACCENT),
                AppStatus::Warning(message) => (message.clone(), WARNING),
                AppStatus::Error(message) => (message.clone(), ERROR),
            };
        }

        if !worker_available {
            return (
                "Encoding worker unavailable. Restart BurnSubs.".to_owned(),
                ERROR,
            );
        }
        if is_busy {
            return ("Encoding is running.".to_owned(), ACCENT);
        }
        if can_start {
            return ("Ready to encode.".to_owned(), SUCCESS);
        }

        (
            "Choose the required inputs and an output folder.".to_owned(),
            WARNING,
        )
    }

    fn select_video(&mut self) {
        let mut dialog = FileDialog::new()
            .set_title("Select a video")
            .add_filter("Video files", VIDEO_EXTENSIONS);

        if let Some(directory) = self
            .last_video_directory
            .as_deref()
            .filter(|path| path.is_dir())
            .or_else(|| selected_parent(&self.single_video))
        {
            dialog = dialog.set_directory(directory);
        }

        let Some(video_path) = dialog.pick_file() else {
            return;
        };

        self.last_video_directory = video_path.parent().map(Path::to_path_buf);

        if self.output_directory.is_none() {
            self.output_directory = video_path.parent().map(Path::to_path_buf);

            self.last_output_directory = self.output_directory.clone();
        }

        self.single_subtitle = find_matching_srt(&video_path);

        if let Some(subtitle_path) = &self.single_subtitle {
            self.last_subtitle_directory = subtitle_path.parent().map(Path::to_path_buf);
        }

        self.single_video = Some(video_path);
        self.jobs.clear();

        self.status = self.single_subtitle.as_ref().map(|path| {
            AppStatus::Info(format!(
                "Matching subtitle detected: {}",
                path.to_string_lossy()
            ))
        });
    }

    fn select_subtitle(&mut self) {
        let mut dialog = FileDialog::new()
            .set_title("Select an SRT subtitle")
            .add_filter("SubRip subtitle", &["srt"]);

        if let Some(directory) = self
            .last_subtitle_directory
            .as_deref()
            .filter(|path| path.is_dir())
            .or_else(|| selected_parent(&self.single_video))
            .or_else(|| selected_parent(&self.single_subtitle))
        {
            dialog = dialog.set_directory(directory);
        }

        if let Some(path) = dialog.pick_file() {
            self.last_subtitle_directory = path.parent().map(Path::to_path_buf);

            self.single_subtitle = Some(path);
            self.jobs.clear();
            self.status = None;
        }
    }

    fn select_batch_input_directory(&mut self) {
        let mut dialog = FileDialog::new().set_title("Select the batch folder");

        if let Some(directory) = self
            .last_batch_input_directory
            .as_deref()
            .filter(|path| path.is_dir())
            .or(self.batch_input_directory.as_deref())
        {
            dialog = dialog.set_directory(directory);
        }

        let Some(path) = dialog.pick_folder() else {
            return;
        };

        self.last_batch_input_directory = Some(path.clone());

        if self.output_directory.is_none() {
            self.output_directory = Some(path.clone());
            self.last_output_directory = Some(path.clone());
        }

        self.batch_input_directory = Some(path);
        self.batch_preview = None;
        self.jobs.clear();
        self.status = None;
    }

    fn select_output_directory(&mut self) {
        let mut dialog = FileDialog::new().set_title("Select the output folder");

        if let Some(directory) = self
            .last_output_directory
            .as_deref()
            .filter(|path| path.is_dir())
            .or(self.output_directory.as_deref())
        {
            dialog = dialog.set_directory(directory);
        }

        if let Some(path) = dialog.pick_folder() {
            self.last_output_directory = Some(path.clone());
            self.output_directory = Some(path);
            self.batch_preview = None;
            self.jobs.clear();
            self.status = None;
        }
    }

    fn select_ffmpeg_directory(&mut self) {
        let mut dialog = FileDialog::new().set_title("Select folder containing FFmpeg and FFprobe");

        if let Some(directory) = self
            .custom_ffmpeg_directory
            .as_deref()
            .filter(|path| path.is_dir())
            .or(self.last_output_directory.as_deref())
        {
            dialog = dialog.set_directory(directory);
        }

        if let Some(path) = dialog.pick_folder() {
            self.custom_ffmpeg_directory = Some(path);
            self.status = None;
        }
    }

    fn scan_batch_preview(&mut self) {
        let result = self.encode_settings().and_then(|settings| {
            let input_directory = self
                .batch_input_directory
                .as_deref()
                .ok_or_else(|| "Select a batch input folder.".to_owned())?;

            validate_affixes(&settings)?;

            scan_batch_folder(input_directory, &settings).map_err(|error| error.to_string())
        });

        match result {
            Ok(preview) => {
                let matched_jobs = preview.jobs.len();
                let skipped_videos = preview.skipped_videos.len();

                self.status = Some(if matched_jobs == 0 {
                    AppStatus::Warning(format!(
                        "No videos are ready. {skipped_videos} video(s) were skipped."
                    ))
                } else {
                    AppStatus::Info(format!(
                        "Found {matched_jobs} ready job(s). {skipped_videos} video(s) will be skipped."
                    ))
                });

                self.batch_preview = Some(preview);
            }

            Err(error) => {
                self.batch_preview = None;
                self.status = Some(AppStatus::Error(error));
            }
        }
    }

    fn start_encoding(&mut self) {
        let result = self.prepare_jobs();

        let (jobs, settings) = match result {
            Ok(value) => value,

            Err(error) => {
                self.status = Some(AppStatus::Error(error));
                return;
            }
        };

        if self.worker.is_none() {
            self.status = Some(AppStatus::Error(
                "The encoding worker is unavailable.".to_owned(),
            ));
            return;
        }

        self.queue_total_jobs = jobs.len();
        self.jobs = jobs.clone();

        let start_result = self
            .worker
            .as_ref()
            .expect("worker existence was checked")
            .start(jobs, settings);

        if let Err(error) = start_result {
            self.status = Some(AppStatus::Error(error.to_string()));
            return;
        }

        self.save_settings();

        self.status = Some(AppStatus::Info(format!(
            "Started {} encoding job(s).",
            self.queue_total_jobs
        )));
    }

    fn prepare_jobs(&mut self) -> Result<(Vec<EncodeJob>, EncodeSettings), String> {
        let settings = self.encode_settings()?;

        validate_affixes(&settings)?;
        validate_output_directory(&settings.output_directory)?;

        match self.selected_mode {
            AppMode::Single => {
                let video_path = self
                    .single_video
                    .clone()
                    .ok_or_else(|| "Select a video file.".to_owned())?;

                let subtitle_path = self
                    .single_subtitle
                    .clone()
                    .ok_or_else(|| "Select an SRT subtitle file.".to_owned())?;

                if !video_path.is_file() {
                    return Err("The selected video file no longer exists.".to_owned());
                }

                if !subtitle_path.is_file() {
                    return Err("The selected subtitle file no longer exists.".to_owned());
                }

                if !has_srt_extension(&subtitle_path) {
                    return Err("The selected subtitle must be an SRT file.".to_owned());
                }

                let output_path = settings.create_output_path(&video_path);

                if paths_refer_to_same_output(&video_path, &output_path) {
                    return Err("The output path would replace the input video.".to_owned());
                }

                if output_path.exists() && !settings.overwrite_existing {
                    return Err(format!(
                        "The output already exists: {}",
                        output_path.display()
                    ));
                }

                Ok((
                    vec![EncodeJob::new(1, video_path, subtitle_path, output_path)],
                    settings,
                ))
            }

            AppMode::Batch => {
                let input_directory = self
                    .batch_input_directory
                    .as_deref()
                    .ok_or_else(|| "Select a batch input folder.".to_owned())?;

                let scan_result = scan_batch_folder(input_directory, &settings)
                    .map_err(|error| error.to_string())?;

                let jobs = scan_result.jobs.clone();
                let skipped_count = scan_result.skipped_videos.len();

                self.batch_preview = Some(scan_result);

                if jobs.is_empty() {
                    return Err(format!(
                        "No videos are ready to encode. {skipped_count} video(s) were skipped."
                    ));
                }

                Ok((jobs, settings))
            }
        }
    }

    fn encode_settings(&self) -> Result<EncodeSettings, String> {
        let output_directory = self
            .output_directory
            .clone()
            .ok_or_else(|| "Select an output folder.".to_owned())?;

        Ok(EncodeSettings {
            prefix: self.prefix.clone(),
            postfix: self.postfix.clone(),
            output_directory,
            output_format: self.output_format,
            quality: self.quality,
            acceleration: self.acceleration,
            overwrite_existing: self.overwrite_existing,
            custom_ffmpeg_directory: self.custom_ffmpeg_directory.clone(),
        })
    }

    fn single_output_path(&self) -> Option<PathBuf> {
        let video_path = self.single_video.as_deref()?;
        let settings = self.encode_settings().ok()?;

        Some(settings.create_output_path(video_path))
    }

    fn has_required_selection(&self) -> bool {
        match self.selected_mode {
            AppMode::Single => {
                self.single_video.is_some()
                    && self.single_subtitle.is_some()
                    && self.output_directory.is_some()
            }

            AppMode::Batch => {
                self.batch_input_directory.is_some() && self.output_directory.is_some()
            }
        }
    }

    fn is_busy(&self) -> bool {
        self.worker.as_ref().is_some_and(WorkerHandle::is_busy)
    }

    fn poll_worker_events(&mut self) {
        let events = self
            .worker
            .as_ref()
            .map(WorkerHandle::drain_events)
            .unwrap_or_default();

        for event in events {
            self.handle_worker_event(event);
        }
    }

    fn handle_worker_event(&mut self, event: WorkerEvent) {
        match event {
            WorkerEvent::QueueStarted { total_jobs } => {
                self.queue_total_jobs = total_jobs;
            }

            WorkerEvent::EncoderSelected {
                encoder,
                hardware,
                note,
            } => {
                self.status = Some(match note {
                    Some(note) => AppStatus::Warning(note),
                    None if hardware => {
                        AppStatus::Info(format!("GPU acceleration active: {encoder}."))
                    }
                    None => AppStatus::Info(format!("Encoding with {encoder}.")),
                });
            }

            WorkerEvent::JobProbing(job_id) => {
                if let Some(job) = self.job_mut(job_id) {
                    job.status = JobStatus::Probing;
                    job.set_progress(0.0);
                }
            }

            WorkerEvent::JobStarted { job_id, encoder } => {
                if let Some(job) = self.job_mut(job_id) {
                    job.status = JobStatus::Encoding;
                    job.encoder_name = Some(encoder);
                }
            }

            WorkerEvent::JobFallback {
                job_id,
                from,
                to,
                reason,
            } => {
                if let Some(job) = self.job_mut(job_id) {
                    job.status = JobStatus::Encoding;
                    job.set_progress(0.0);
                    job.encoder_name = Some(to.clone());
                    job.fallback_note =
                        Some(format!("{from} failed; retrying with {to}. {reason}"));
                }

                self.status = Some(AppStatus::Warning(format!(
                    "{from} failed for a job; this and remaining jobs will use {to}."
                )));
            }

            WorkerEvent::Progress { job_id, progress } => {
                if let Some(job) = self.job_mut(job_id) {
                    job.set_progress(progress);
                }
            }

            WorkerEvent::JobCompleted(job_id) => {
                if let Some(job) = self.job_mut(job_id) {
                    job.status = JobStatus::Completed;
                    job.set_progress(1.0);
                }
            }

            WorkerEvent::JobFailed { job_id, error } => {
                if let Some(job) = self.job_mut(job_id) {
                    job.status = JobStatus::Failed(error);
                    job.set_progress(1.0);
                }
            }

            WorkerEvent::JobCancelled(job_id) => {
                if let Some(job) = self.job_mut(job_id) {
                    job.status = JobStatus::Cancelled;
                }
            }

            WorkerEvent::QueueCompleted {
                completed_jobs,
                failed_jobs,
            } => {
                self.status = Some(if failed_jobs == 0 {
                    AppStatus::Info(format!(
                        "Queue completed. {completed_jobs} job(s) finished successfully."
                    ))
                } else {
                    AppStatus::Warning(format!(
                        "Queue completed with {completed_jobs} successful and {failed_jobs} failed job(s)."
                    ))
                });
            }

            WorkerEvent::QueueCancelled {
                completed_jobs,
                failed_jobs,
            } => {
                for job in &mut self.jobs {
                    if !job.status.is_terminal() {
                        job.status = JobStatus::Cancelled;
                    }
                }

                self.status = Some(AppStatus::Warning(format!(
                    "Queue cancelled. {completed_jobs} completed and {failed_jobs} failed before cancellation."
                )));
            }

            WorkerEvent::QueueFailed { error } => {
                self.status = Some(AppStatus::Error(error));
            }
        }
    }

    fn job_mut(&mut self, job_id: u64) -> Option<&mut EncodeJob> {
        self.jobs.iter_mut().find(|job| job.id == job_id)
    }

    fn overall_progress(&self) -> f32 {
        if self.jobs.is_empty() {
            return 0.0;
        }

        let total = self
            .jobs
            .iter()
            .map(|job| match &job.status {
                JobStatus::Completed | JobStatus::Failed(_) | JobStatus::Cancelled => 1.0,

                JobStatus::Encoding => job.progress,

                JobStatus::Queued | JobStatus::Probing => 0.0,
            })
            .sum::<f32>();

        (total / self.jobs.len() as f32).clamp(0.0, 1.0)
    }

    fn finished_job_count(&self) -> usize {
        self.jobs
            .iter()
            .filter(|job| job.status.is_terminal())
            .count()
    }

    fn save_settings(&self) {
        let settings = AppSettings {
            selected_mode: self.selected_mode,

            last_video_directory: self.last_video_directory.clone(),
            last_subtitle_directory: self.last_subtitle_directory.clone(),
            last_batch_input_directory: self.last_batch_input_directory.clone(),
            last_output_directory: self.last_output_directory.clone(),

            prefix: self.prefix.clone(),
            postfix: self.postfix.clone(),

            output_format: self.output_format,
            quality: self.quality,
            acceleration: self.acceleration,
            overwrite_existing: self.overwrite_existing,

            custom_ffmpeg_directory: self.custom_ffmpeg_directory.clone(),
        };

        if let Err(error) = settings.save() {
            tracing::warn!(
                error = %error,
                "Could not save application settings"
            );
        }
    }
}

impl eframe::App for BurnSubsApp {
    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.poll_worker_events();

        if self.is_busy() {
            root_ui
                .ctx()
                .request_repaint_after(Duration::from_millis(100));
        }

        egui::Panel::top("app_header")
            .exact_size(54.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 8))
                    .fill(PANEL_FILL),
            )
            .show(root_ui, |ui| self.draw_header(ui));

        egui::Panel::bottom("action_footer")
            .exact_size(62.0)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .fill(PANEL_FILL),
            )
            .show(root_ui, |ui| self.draw_action_area(ui));

        egui::Panel::left("setup_rail")
            .exact_size(430.0)
            .resizable(false)
            .show_separator_line(true)
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(16, 14))
                    .fill(PANEL_FILL),
            )
            .show(root_ui, |ui| {
                ui.add_enabled_ui(!self.is_busy(), |ui| {
                    match self.selected_mode {
                        AppMode::Single => self.draw_single_mode(ui),
                        AppMode::Batch => self.draw_batch_mode(ui),
                    }

                    major_separator(ui);
                    self.draw_output_settings(ui);
                });
            });

        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(18, 14))
                    .fill(WORKSPACE_FILL),
            )
            .show(root_ui, |ui| self.draw_monitor(ui));
    }
}

impl Drop for BurnSubsApp {
    fn drop(&mut self) {
        self.save_settings();
    }
}

fn path_picker_row(ui: &mut egui::Ui, label: &str, path: Option<&Path>, button_text: &str) -> bool {
    ui.horizontal(|ui| {
        ui.add_sized(
            [58.0, 30.0],
            egui::Label::new(RichText::new(label).strong()),
        );

        let button_width = 68.0;
        let path_width = (ui.available_width() - button_width - 8.0).max(60.0);
        let path_text = path
            .map(|value| value.to_string_lossy())
            .unwrap_or_else(|| "Nothing selected".into());
        let shown_text = path
            .and_then(Path::file_name)
            .map(|value| value.to_string_lossy())
            .unwrap_or_else(|| path_text.clone());

        ui.add_sized(
            [path_width, 30.0],
            egui::Label::new(RichText::new(shown_text).weak()).truncate(),
        )
        .on_hover_text(path_text);

        ui.add_sized([button_width, 30.0], egui::Button::new(button_text))
            .clicked()
    })
    .inner
}

fn draw_batch_preview(ui: &mut egui::Ui, preview: &BatchScanResult) {
    ui.horizontal(|ui| {
        summary_metric(ui, preview.jobs.len(), "ready", SUCCESS);
        summary_metric(ui, preview.skipped_videos.len(), "skipped", WARNING);
        summary_metric(ui, preview.orphan_subtitles.len(), "orphan", ACCENT);
        summary_metric(
            ui,
            preview.ignored_file_count,
            "ignored",
            ui.visuals().weak_text_color(),
        );
    });
    ui.add_space(10.0);

    egui::ScrollArea::vertical()
        .id_salt("batch_preview_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !preview.jobs.is_empty() {
                list_heading(ui, "Ready to encode", preview.jobs.len());
                for job in &preview.jobs {
                    preview_row(ui, &job.video_name(), &job.subtitle_name(), SUCCESS);
                }
            }

            if !preview.skipped_videos.is_empty() {
                ui.add_space(10.0);
                list_heading(ui, "Skipped", preview.skipped_videos.len());
                for skipped in &preview.skipped_videos {
                    preview_row(
                        ui,
                        &file_name_or_path(&skipped.video_path),
                        &skipped.reason.to_string(),
                        WARNING,
                    );
                }
            }

            if !preview.orphan_subtitles.is_empty() {
                ui.add_space(10.0);
                list_heading(ui, "Orphan subtitles", preview.orphan_subtitles.len());
                for path in &preview.orphan_subtitles {
                    preview_row(ui, &file_name_or_path(path), "No matching video", ACCENT);
                }
            }
        });
}

fn configure_interface(context: &egui::Context) {
    context.set_theme(egui::Theme::Dark);
    let mut style = (*context.style_of(egui::Theme::Dark)).clone();
    style.animation_time = 0.18;
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(12.0, 7.0);
    style.spacing.interact_size.y = 30.0;
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = PANEL_FILL;
    style.visuals.window_fill = PANEL_FILL;
    style.visuals.extreme_bg_color = egui::Color32::from_rgb(16, 18, 22);
    style.visuals.faint_bg_color = egui::Color32::from_rgb(31, 35, 42);
    style.visuals.selection.bg_fill = ACCENT;
    style.visuals.selection.stroke.color = egui::Color32::WHITE;
    style.visuals.widgets.noninteractive.corner_radius = 6.into();
    style.visuals.widgets.inactive.corner_radius = 6.into();
    style.visuals.widgets.hovered.corner_radius = 6.into();
    style.visuals.widgets.active.corner_radius = 6.into();
    style.visuals.widgets.open.corner_radius = 6.into();
    style.visuals.window_corner_radius = 8.into();
    style.visuals.error_fg_color = ERROR;
    context.set_style_of(egui::Theme::Dark, style);
}

fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).size(16.0).strong());
    ui.add_space(2.0);
}

fn major_separator(ui: &mut egui::Ui) {
    ui.add_space(10.0);
    ui.separator();
    ui.add_space(10.0);
}

fn field_label(ui: &mut egui::Ui, label: &str) {
    ui.label(RichText::new(label).small().weak());
}

fn compact_text_field(ui: &mut egui::Ui, label: &str, value: &mut String) {
    field_label(ui, label);
    ui.add_sized(
        [ui.available_width(), 30.0],
        egui::TextEdit::singleline(value),
    );
}

fn status_dot(ui: &mut egui::Ui, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 18.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

fn empty_monitor(ui: &mut egui::Ui, title: &str, description: &str) {
    ui.with_layout(
        egui::Layout::centered_and_justified(egui::Direction::TopDown),
        |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(title).size(18.0).strong());
                ui.add_space(4.0);
                ui.label(RichText::new(description).weak());
            });
        },
    );
}

fn queue_row(ui: &mut egui::Ui, job: &EncodeJob) {
    let color = job_status_color(&job.status);
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(4, 7))
        .show(ui, |ui| {
            egui::containers::Sides::new()
                .height(22.0)
                .shrink_left()
                .truncate()
                .show(
                    ui,
                    |ui| {
                        status_dot(ui, color);
                        ui.add(
                            egui::Label::new(RichText::new(job.video_name()).strong()).truncate(),
                        )
                        .on_hover_text(job.video_path.display().to_string());
                    },
                    |ui| {
                        ui.label(RichText::new(job.status.label()).color(color));
                    },
                );

            egui::containers::Sides::new()
                .height(22.0)
                .shrink_left()
                .truncate()
                .show(
                    ui,
                    |ui| {
                        let detail = job.encoder_name.as_deref().map_or_else(
                            || job.output_name(),
                            |encoder| format!("{}  ·  {encoder}", job.output_name()),
                        );
                        let hover = job_detail_hover(job);
                        ui.add(egui::Label::new(RichText::new(detail).small().weak()).truncate())
                            .on_hover_text(hover);
                    },
                    |ui| {
                        if matches!(job.status, JobStatus::Encoding | JobStatus::Probing) {
                            ui.add(
                                egui::ProgressBar::new(job.progress)
                                    .desired_width(112.0)
                                    .corner_radius(3)
                                    .fill(ACCENT)
                                    .show_percentage(),
                            );
                        }
                    },
                );
        });
}

fn job_status_color(status: &JobStatus) -> egui::Color32 {
    match status {
        JobStatus::Queued => egui::Color32::GRAY,
        JobStatus::Probing | JobStatus::Encoding => ACCENT,
        JobStatus::Completed => SUCCESS,
        JobStatus::Failed(_) => ERROR,
        JobStatus::Cancelled => WARNING,
    }
}

fn job_detail_hover(job: &EncodeJob) -> String {
    let mut details = format!("Output: {}", job.output_path.display());
    if let JobStatus::Failed(error) = &job.status {
        details.push_str("\n\nError: ");
        details.push_str(error);
    }
    if let Some(note) = &job.fallback_note {
        details.push_str("\n\n");
        details.push_str(note);
    }
    details
}

fn summary_metric(ui: &mut egui::Ui, value: usize, label: &str, color: egui::Color32) {
    ui.label(RichText::new(value.to_string()).color(color).strong());
    ui.label(RichText::new(label).weak());
    ui.add_space(10.0);
}

fn list_heading(ui: &mut egui::Ui, title: &str, count: usize) {
    ui.label(RichText::new(format!("{title} ({count})")).strong());
    ui.add_space(3.0);
}

fn preview_row(ui: &mut egui::Ui, primary: &str, secondary: &str, color: egui::Color32) {
    egui::containers::Sides::new()
        .height(32.0)
        .shrink_left()
        .truncate()
        .show(
            ui,
            |ui| {
                status_dot(ui, color);
                ui.add(egui::Label::new(primary).truncate())
                    .on_hover_text(primary);
            },
            |ui| {
                ui.add(egui::Label::new(RichText::new(secondary).small().weak()).truncate())
                    .on_hover_text(secondary);
            },
        );
    ui.separator();
}

fn file_name_or_path(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn selected_parent(path: &Option<PathBuf>) -> Option<&Path> {
    path.as_deref().and_then(Path::parent)
}

fn existing_directory(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|directory| directory.is_dir())
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

        let is_srt = has_srt_extension(&path);

        if !is_srt {
            continue;
        }

        let has_matching_stem = path
            .file_stem()
            .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(&video_stem));

        if has_matching_stem {
            return Some(path);
        }
    }

    None
}

fn has_srt_extension(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.to_string_lossy().eq_ignore_ascii_case("srt"))
}

fn validate_output_directory(output_directory: &Path) -> Result<(), String> {
    if !output_directory.exists() {
        return Err(format!(
            "The output folder does not exist: {}",
            output_directory.display()
        ));
    }

    if !output_directory.is_dir() {
        return Err(format!(
            "The output path is not a folder: {}",
            output_directory.display()
        ));
    }

    Ok(())
}

fn validate_affixes(settings: &EncodeSettings) -> Result<(), String> {
    validate_affix("Prefix", &settings.prefix)?;
    validate_affix("Postfix", &settings.postfix)?;

    Ok(())
}

fn validate_affix(label: &str, value: &str) -> Result<(), String> {
    const INVALID_CHARACTERS: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

    if value.chars().any(char::is_control) {
        return Err(format!(
            "{label} contains an unsupported control character."
        ));
    }

    if let Some(character) = value
        .chars()
        .find(|character| INVALID_CHARACTERS.contains(character))
    {
        return Err(format!(
            "{label} contains an invalid filename character: {character}"
        ));
    }

    if value.ends_with(' ') || value.ends_with('.') {
        return Err(format!("{label} cannot end with a space or a period."));
    }

    Ok(())
}

fn paths_refer_to_same_output(input_path: &Path, output_path: &Path) -> bool {
    let input_key = path_comparison_key(input_path);
    let output_key = path_comparison_key(output_path);

    input_key == output_key
}

fn path_comparison_key(path: &Path) -> String {
    let absolute_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()
            .map(|directory| directory.join(path))
            .unwrap_or_else(|_| path.to_path_buf())
    };

    let normalized = absolute_path.to_string_lossy().replace('\\', "/");

    if cfg!(target_os = "windows") {
        normalized.to_lowercase()
    } else {
        normalized
    }
}
