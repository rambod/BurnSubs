use crate::{
    ffmpeg::{
        acceleration::{VideoEncoder, select_encoder},
        encoder::{EncodeRequest, encode_video},
        locator::{locate_ffmpeg, validate_binary_files},
        probe::probe_video,
    },
    model::{EncodeJob, EncodeSettings},
};
use anyhow::{Result, anyhow};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread::{self, JoinHandle},
};

#[derive(Debug)]
pub enum WorkerCommand {
    Start {
        jobs: Vec<EncodeJob>,
        settings: EncodeSettings,
    },
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    QueueStarted {
        total_jobs: usize,
    },

    JobProbing(u64),

    EncoderSelected {
        encoder: String,
        hardware: bool,
        note: Option<String>,
    },

    JobStarted {
        job_id: u64,
        encoder: String,
    },

    JobFallback {
        job_id: u64,
        from: String,
        to: String,
        reason: String,
    },

    Progress {
        job_id: u64,
        progress: f32,
    },

    JobCompleted(u64),

    JobFailed {
        job_id: u64,
        error: String,
    },

    JobCancelled(u64),

    QueueCompleted {
        completed_jobs: usize,
        failed_jobs: usize,
    },

    QueueCancelled {
        completed_jobs: usize,
        failed_jobs: usize,
    },

    QueueFailed {
        error: String,
    },
}

pub struct WorkerHandle {
    command_sender: Sender<WorkerCommand>,
    event_receiver: Receiver<WorkerEvent>,

    cancel_requested: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,

    thread_handle: Option<JoinHandle<()>>,
}

impl WorkerHandle {
    pub fn spawn() -> Result<Self> {
        let (command_sender, command_receiver) = mpsc::channel::<WorkerCommand>();

        let (event_sender, event_receiver) = mpsc::channel::<WorkerEvent>();

        let cancel_requested = Arc::new(AtomicBool::new(false));
        let busy = Arc::new(AtomicBool::new(false));

        let worker_cancel_requested = Arc::clone(&cancel_requested);
        let worker_busy = Arc::clone(&busy);

        let thread_handle = thread::Builder::new()
            .name("burnsubs-encoder-worker".to_owned())
            .spawn(move || {
                worker_loop(
                    command_receiver,
                    event_sender,
                    worker_cancel_requested,
                    worker_busy,
                );
            })
            .map_err(|error| anyhow!("Could not create encoder worker thread: {error}"))?;

        Ok(Self {
            command_sender,
            event_receiver,
            cancel_requested,
            busy,
            thread_handle: Some(thread_handle),
        })
    }

    pub fn start(&self, jobs: Vec<EncodeJob>, settings: EncodeSettings) -> Result<()> {
        if jobs.is_empty() {
            return Err(anyhow!("The encoding queue is empty."));
        }

        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(anyhow!("Another encoding queue is already running."));
        }

        self.cancel_requested.store(false, Ordering::Release);

        if let Err(error) = self
            .command_sender
            .send(WorkerCommand::Start { jobs, settings })
        {
            self.busy.store(false, Ordering::Release);

            return Err(anyhow!("Could not start the encoding queue: {error}"));
        }

        Ok(())
    }

    pub fn cancel(&self) {
        if self.is_busy() {
            tracing::info!("Queue cancellation requested");

            self.cancel_requested.store(true, Ordering::Release);
        }
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    pub fn drain_events(&self) -> Vec<WorkerEvent> {
        self.event_receiver.try_iter().collect()
    }
}

impl Drop for WorkerHandle {
    fn drop(&mut self) {
        self.cancel_requested.store(true, Ordering::Release);

        let _ = self.command_sender.send(WorkerCommand::Shutdown);

        if let Some(thread_handle) = self.thread_handle.take()
            && thread_handle.join().is_err()
        {
            tracing::error!("Encoder worker thread panicked");
        }
    }
}

fn worker_loop(
    command_receiver: Receiver<WorkerCommand>,
    event_sender: Sender<WorkerEvent>,
    cancel_requested: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
) {
    tracing::info!("Encoder worker started");

    while let Ok(command) = command_receiver.recv() {
        match command {
            WorkerCommand::Start { jobs, settings } => {
                process_queue(jobs, settings, &event_sender, &cancel_requested);

                busy.store(false, Ordering::Release);
            }

            WorkerCommand::Shutdown => {
                tracing::info!("Encoder worker shutting down");
                break;
            }
        }
    }

    busy.store(false, Ordering::Release);
}

fn process_queue(
    jobs: Vec<EncodeJob>,
    settings: EncodeSettings,
    event_sender: &Sender<WorkerEvent>,
    cancel_requested: &AtomicBool,
) {
    let total_jobs = jobs.len();

    if !send_event(event_sender, WorkerEvent::QueueStarted { total_jobs }) {
        cancel_requested.store(true, Ordering::Release);
        return;
    }

    let ffmpeg_paths =
        match locate_ffmpeg(settings.custom_ffmpeg_directory.as_deref()).and_then(|paths| {
            validate_binary_files(&paths)?;
            Ok(paths)
        }) {
            Ok(paths) => paths,

            Err(error) => {
                send_event(
                    event_sender,
                    WorkerEvent::QueueFailed {
                        error: error.to_string(),
                    },
                );

                return;
            }
        };

    let selection = match select_encoder(&ffmpeg_paths.ffmpeg, settings.acceleration) {
        Ok(selection) => selection,
        Err(error) => {
            send_event(
                event_sender,
                WorkerEvent::QueueFailed {
                    error: format!("Could not initialize a video encoder: {error:#}"),
                },
            );
            return;
        }
    };

    if !send_event(
        event_sender,
        WorkerEvent::EncoderSelected {
            encoder: selection.encoder.label().to_owned(),
            hardware: selection.encoder.is_hardware(),
            note: selection.note,
        },
    ) {
        cancel_requested.store(true, Ordering::Release);
        return;
    }

    let mut active_encoder = selection.encoder;

    let mut completed_jobs = 0;
    let mut failed_jobs = 0;

    for job in jobs {
        if cancel_requested.load(Ordering::Acquire) {
            send_event(
                event_sender,
                WorkerEvent::QueueCancelled {
                    completed_jobs,
                    failed_jobs,
                },
            );

            return;
        }

        if !send_event(event_sender, WorkerEvent::JobProbing(job.id)) {
            cancel_requested.store(true, Ordering::Release);
            return;
        }

        let video_info = match probe_video(&ffmpeg_paths.ffprobe, &job.video_path) {
            Ok(info) => info,

            Err(error) => {
                failed_jobs += 1;

                if !send_event(
                    event_sender,
                    WorkerEvent::JobFailed {
                        job_id: job.id,
                        error: error.to_string(),
                    },
                ) {
                    cancel_requested.store(true, Ordering::Release);
                    return;
                }

                continue;
            }
        };

        tracing::debug!(
            video = %job.video_path.display(),
            codec = video_info.video_codec,
            width = video_info.width,
            height = video_info.height,
            format = video_info.format_name.as_deref().unwrap_or("unknown"),
            audio_streams = video_info.audio_stream_count,
            duration_seconds = video_info.duration_seconds,
            "Probed input video"
        );

        if cancel_requested.load(Ordering::Acquire) {
            send_event(event_sender, WorkerEvent::JobCancelled(job.id));

            send_event(
                event_sender,
                WorkerEvent::QueueCancelled {
                    completed_jobs,
                    failed_jobs,
                },
            );

            return;
        }

        if !send_event(
            event_sender,
            WorkerEvent::JobStarted {
                job_id: job.id,
                encoder: active_encoder.label().to_owned(),
            },
        ) {
            cancel_requested.store(true, Ordering::Release);
            return;
        }

        let mut report_progress = |progress| {
            if event_sender
                .send(WorkerEvent::Progress {
                    job_id: job.id,
                    progress,
                })
                .is_err()
            {
                cancel_requested.store(true, Ordering::Release);
            }
        };

        let first_result = encode_video(
            EncodeRequest {
                ffmpeg_path: &ffmpeg_paths.ffmpeg,
                video_path: &job.video_path,
                subtitle_path: &job.subtitle_path,
                output_path: &job.output_path,
                settings: &settings,
                duration_seconds: video_info.duration_seconds,
                encoder: active_encoder,
                cancel_requested,
            },
            &mut report_progress,
        );

        let encoding_result = match first_result {
            Err(hardware_error)
                if active_encoder.is_hardware() && !cancel_requested.load(Ordering::Acquire) =>
            {
                let failed_encoder = active_encoder;
                active_encoder = VideoEncoder::Software;
                let reason = concise_error(&hardware_error);

                send_event(
                    event_sender,
                    WorkerEvent::JobFallback {
                        job_id: job.id,
                        from: failed_encoder.label().to_owned(),
                        to: active_encoder.label().to_owned(),
                        reason,
                    },
                );

                encode_video(
                    EncodeRequest {
                        ffmpeg_path: &ffmpeg_paths.ffmpeg,
                        video_path: &job.video_path,
                        subtitle_path: &job.subtitle_path,
                        output_path: &job.output_path,
                        settings: &settings,
                        duration_seconds: video_info.duration_seconds,
                        encoder: active_encoder,
                        cancel_requested,
                    },
                    &mut report_progress,
                )
                .map_err(|software_error| {
                    anyhow!(
                        "{} failed, then the CPU fallback also failed.\nHardware error: {hardware_error:#}\nCPU error: {software_error:#}",
                        failed_encoder.label()
                    )
                })
            }
            result => result,
        };

        match encoding_result {
            Ok(()) => {
                completed_jobs += 1;

                if !send_event(event_sender, WorkerEvent::JobCompleted(job.id)) {
                    cancel_requested.store(true, Ordering::Release);
                    return;
                }
            }

            Err(error) => {
                if cancel_requested.load(Ordering::Acquire) {
                    send_event(event_sender, WorkerEvent::JobCancelled(job.id));

                    send_event(
                        event_sender,
                        WorkerEvent::QueueCancelled {
                            completed_jobs,
                            failed_jobs,
                        },
                    );

                    return;
                }

                failed_jobs += 1;

                if !send_event(
                    event_sender,
                    WorkerEvent::JobFailed {
                        job_id: job.id,
                        error: error.to_string(),
                    },
                ) {
                    cancel_requested.store(true, Ordering::Release);
                    return;
                }
            }
        }
    }

    send_event(
        event_sender,
        WorkerEvent::QueueCompleted {
            completed_jobs,
            failed_jobs,
        },
    );
}

fn send_event(sender: &Sender<WorkerEvent>, event: WorkerEvent) -> bool {
    sender.send(event).is_ok()
}

fn concise_error(error: &anyhow::Error) -> String {
    let message = format!("{error:#}");
    const MAX_CHARACTERS: usize = 500;

    if message.chars().count() <= MAX_CHARACTERS {
        message
    } else {
        format!(
            "{}…",
            message.chars().take(MAX_CHARACTERS).collect::<String>()
        )
    }
}
