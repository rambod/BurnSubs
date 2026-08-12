use crate::{
    ffmpeg::acceleration::{VideoEncoder, apply_video_options, configure_device, subtitle_filter},
    model::{EncodeSettings, OutputFormat},
};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    env, fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, AtomicU64, Ordering},
    thread,
};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

pub struct EncodeRequest<'a> {
    pub ffmpeg_path: &'a Path,
    pub video_path: &'a Path,
    pub subtitle_path: &'a Path,
    pub output_path: &'a Path,
    pub settings: &'a EncodeSettings,
    pub duration_seconds: f64,
    pub encoder: VideoEncoder,
    pub cancel_requested: &'a AtomicBool,
}

pub fn encode_video<F>(request: EncodeRequest<'_>, mut on_progress: F) -> Result<()>
where
    F: FnMut(f32),
{
    validate_inputs(
        request.ffmpeg_path,
        request.video_path,
        request.subtitle_path,
        request.duration_seconds,
    )?;

    let ffmpeg_path = request.ffmpeg_path.canonicalize().with_context(|| {
        format!(
            "Could not resolve FFmpeg path: {}",
            request.ffmpeg_path.display()
        )
    })?;

    let video_path = request.video_path.canonicalize().with_context(|| {
        format!(
            "Could not resolve video path: {}",
            request.video_path.display()
        )
    })?;

    let subtitle_path = request.subtitle_path.canonicalize().with_context(|| {
        format!(
            "Could not resolve subtitle path: {}",
            request.subtitle_path.display()
        )
    })?;

    let output_path = make_absolute(request.output_path)?;

    let output_directory = output_path
        .parent()
        .context("The output path does not have a parent directory")?;

    fs::create_dir_all(output_directory).with_context(|| {
        format!(
            "Could not create output directory: {}",
            output_directory.display()
        )
    })?;

    if output_path.exists() && !request.settings.overwrite_existing {
        bail!("The output file already exists: {}", output_path.display());
    }

    let temporary_id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);

    let subtitle_working_directory = create_subtitle_working_directory(temporary_id)?;

    let temporary_subtitle_path = subtitle_working_directory.join("subtitle.srt");

    fs::copy(&subtitle_path, &temporary_subtitle_path).with_context(|| {
        format!(
            "Could not prepare subtitle file: {}",
            subtitle_path.display()
        )
    })?;

    let partial_output_path = create_partial_output_path(&output_path, temporary_id)?;

    if partial_output_path.exists() {
        fs::remove_file(&partial_output_path).with_context(|| {
            format!(
                "Could not remove stale temporary output: {}",
                partial_output_path.display()
            )
        })?;
    }

    let encoding_result = run_ffmpeg(
        FfmpegRun {
            ffmpeg_path: &ffmpeg_path,
            video_path: &video_path,
            subtitle_working_directory: &subtitle_working_directory,
            partial_output_path: &partial_output_path,
            settings: request.settings,
            duration_seconds: request.duration_seconds,
            encoder: request.encoder,
            cancel_requested: request.cancel_requested,
        },
        &mut on_progress,
    );

    let _ = fs::remove_dir_all(&subtitle_working_directory);

    match encoding_result {
        Ok(()) => {
            finalize_output(
                &partial_output_path,
                &output_path,
                request.settings.overwrite_existing,
            )?;

            on_progress(1.0);

            tracing::info!(
                output = %output_path.display(),
                "Encoding completed"
            );

            Ok(())
        }

        Err(error) => {
            let _ = fs::remove_file(&partial_output_path);
            Err(error)
        }
    }
}

struct FfmpegRun<'a> {
    ffmpeg_path: &'a Path,
    video_path: &'a Path,
    subtitle_working_directory: &'a Path,
    partial_output_path: &'a Path,
    settings: &'a EncodeSettings,
    duration_seconds: f64,
    encoder: VideoEncoder,
    cancel_requested: &'a AtomicBool,
}

fn run_ffmpeg<F>(run: FfmpegRun<'_>, on_progress: &mut F) -> Result<()>
where
    F: FnMut(f32),
{
    let mut command = Command::new(run.ffmpeg_path);

    command
        .arg("-hide_banner")
        .arg("-nostdin")
        .arg("-y")
        .arg("-progress")
        .arg("pipe:1")
        .arg("-nostats")
        .arg("-stats_period")
        .arg("0.25");

    configure_device(&mut command, run.encoder)?;

    command
        .arg("-i")
        .arg(run.video_path)
        .arg("-map")
        .arg("0:v:0")
        .arg("-map")
        .arg("0:a?")
        .arg("-map_metadata")
        .arg("0")
        .arg("-map_chapters")
        .arg("0")
        .arg("-vf")
        .arg(subtitle_filter(run.encoder));

    apply_video_options(&mut command, run.encoder, run.settings.quality);

    match run.settings.output_format {
        OutputFormat::Mp4 => {
            command
                .arg("-c:a")
                .arg("aac")
                .arg("-b:a")
                .arg("192k")
                .arg("-movflags")
                .arg("+faststart");
        }

        OutputFormat::Mkv => {
            command.arg("-c:a").arg("copy");
        }
    }

    command
        .arg(run.partial_output_path)
        .current_dir(run.subtitle_working_directory)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    hide_console_window(&mut command);

    tracing::info!(
        video = %run.video_path.display(),
        output = %run.partial_output_path.display(),
        encoder = run.encoder.label(),
        "Starting FFmpeg"
    );

    let mut child = command
        .spawn()
        .with_context(|| format!("Failed to launch FFmpeg: {}", run.ffmpeg_path.display()))?;

    let stdout = child
        .stdout
        .take()
        .context("Could not capture FFmpeg progress output")?;

    let mut stderr = child
        .stderr
        .take()
        .context("Could not capture FFmpeg error output")?;

    let stderr_thread = thread::spawn(move || {
        let mut error_output = String::new();
        let _ = stderr.read_to_string(&mut error_output);
        error_output
    });

    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    let mut was_cancelled = false;

    loop {
        if run.cancel_requested.load(Ordering::Relaxed) && !was_cancelled {
            was_cancelled = true;

            tracing::info!("Encoding cancellation requested");

            let _ = child.kill();
        }

        line.clear();

        let bytes_read = reader
            .read_line(&mut line)
            .context("Could not read FFmpeg progress output")?;

        if bytes_read == 0 {
            break;
        }

        let progress_line = line.trim();

        if progress_line == "progress=end" {
            on_progress(1.0);
            continue;
        }

        if let Some(progress) = parse_progress_line(progress_line, run.duration_seconds) {
            on_progress(progress);
        }
    }

    let status = child
        .wait()
        .context("Could not wait for FFmpeg to finish")?;

    let stderr_output = stderr_thread
        .join()
        .map_err(|_| anyhow!("FFmpeg error reader thread panicked"))?;

    if was_cancelled {
        bail!("Encoding was cancelled.");
    }

    if !status.success() {
        let details = stderr_output.trim();

        bail!(
            "FFmpeg failed with exit code {}.\n{}",
            status.code().unwrap_or(-1),
            if details.is_empty() {
                "FFmpeg returned no error details."
            } else {
                details
            }
        );
    }

    Ok(())
}

fn validate_inputs(
    ffmpeg_path: &Path,
    video_path: &Path,
    subtitle_path: &Path,
    duration_seconds: f64,
) -> Result<()> {
    if !ffmpeg_path.is_file() {
        bail!("FFmpeg binary was not found: {}", ffmpeg_path.display());
    }

    if !video_path.is_file() {
        bail!("Video file was not found: {}", video_path.display());
    }

    if !subtitle_path.is_file() {
        bail!("Subtitle file was not found: {}", subtitle_path.display());
    }

    if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
        bail!("Video duration must be greater than zero.");
    }

    Ok(())
}

fn create_subtitle_working_directory(temporary_id: u64) -> Result<PathBuf> {
    let directory = env::temp_dir()
        .join("BurnSubs")
        .join(format!("subtitle-{}-{temporary_id}", std::process::id()));

    if directory.exists() {
        fs::remove_dir_all(&directory).with_context(|| {
            format!(
                "Could not clear temporary directory: {}",
                directory.display()
            )
        })?;
    }

    fs::create_dir_all(&directory).with_context(|| {
        format!(
            "Could not create temporary directory: {}",
            directory.display()
        )
    })?;

    Ok(directory)
}

fn create_partial_output_path(output_path: &Path, temporary_id: u64) -> Result<PathBuf> {
    let output_directory = output_path
        .parent()
        .context("Output path has no parent directory")?;

    let file_stem = output_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");

    let extension = output_path
        .extension()
        .and_then(|value| value.to_str())
        .context("Output path has no file extension")?;

    let temporary_name = format!(
        ".{file_stem}.burnsubs-part-{}-{temporary_id}.{extension}",
        std::process::id()
    );

    Ok(output_directory.join(temporary_name))
}

fn finalize_output(
    partial_output_path: &Path,
    output_path: &Path,
    overwrite_existing: bool,
) -> Result<()> {
    if output_path.exists() {
        if !overwrite_existing {
            bail!(
                "The output file appeared while encoding: {}",
                output_path.display()
            );
        }

        fs::remove_file(output_path).with_context(|| {
            format!(
                "Could not replace existing output file: {}",
                output_path.display()
            )
        })?;
    }

    fs::rename(partial_output_path, output_path).with_context(|| {
        format!(
            "Could not move completed video to: {}",
            output_path.display()
        )
    })?;

    Ok(())
}

fn parse_progress_line(line: &str, duration_seconds: f64) -> Option<f32> {
    let (key, value) = line.split_once('=')?;

    let elapsed_seconds = match key {
        // FFmpeg reports these fields in microseconds.
        "out_time_us" | "out_time_ms" => value.parse::<f64>().ok()? / 1_000_000.0,

        "out_time" => parse_timestamp(value)?,

        _ => return None,
    };

    if !elapsed_seconds.is_finite() {
        return None;
    }

    Some((elapsed_seconds / duration_seconds).clamp(0.0, 0.999) as f32)
}

fn parse_timestamp(value: &str) -> Option<f64> {
    let mut sections = value.split(':');

    let hours = sections.next()?.parse::<f64>().ok()?;
    let minutes = sections.next()?.parse::<f64>().ok()?;
    let seconds = sections.next()?.parse::<f64>().ok()?;

    if sections.next().is_some() {
        return None;
    }

    Some((hours * 3600.0) + (minutes * 60.0) + seconds)
}

fn make_absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(env::current_dir()
        .context("Could not determine the current directory")?
        .join(path))
}

#[cfg(target_os = "windows")]
fn hide_console_window(command: &mut Command) {
    use std::os::windows::process::CommandExt;

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    command.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(target_os = "windows"))]
fn hide_console_window(_command: &mut Command) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ffmpeg::{acceleration::VideoEncoder, probe::probe_video},
        model::{AccelerationPreference, OutputFormat, QualityPreset},
    };
    use std::sync::atomic::AtomicBool;

    #[test]
    fn parses_microsecond_progress() {
        let progress = parse_progress_line("out_time_us=5000000", 10.0).unwrap();

        assert!((progress - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn parses_timestamp_progress() {
        let progress = parse_progress_line("out_time=00:01:30.000000", 180.0).unwrap();

        assert!((progress - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn prevents_progress_from_reaching_one_early() {
        let progress = parse_progress_line("out_time_us=20000000", 10.0).unwrap();

        assert_eq!(progress, 0.999);
    }

    #[test]
    #[ignore = "executes the locally installed FFmpeg binary"]
    fn burns_a_real_subtitle_with_cpu_fallback_encoder() {
        let paths = crate::ffmpeg::locator::locate_ffmpeg(None).unwrap();
        let ffmpeg = paths.ffmpeg;
        let ffprobe = paths.ffprobe;
        let fixture = TestFixture::new();
        let input = fixture.path.join("input.mp4");
        let subtitle = fixture.path.join("input.srt");
        let output = fixture.path.join("burned.mp4");

        let generated = Command::new(&ffmpeg)
            .args([
                "-hide_banner",
                "-loglevel",
                "error",
                "-f",
                "lavfi",
                "-i",
                "color=c=navy:s=320x180:r=24:d=2",
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                "-y",
            ])
            .arg(&input)
            .status()
            .unwrap();
        assert!(generated.success());

        fs::write(
            &subtitle,
            "1\n00:00:00,100 --> 00:00:01,800\nBurnSubs validation\n",
        )
        .unwrap();

        let info = probe_video(&ffprobe, &input).unwrap();
        let settings = EncodeSettings {
            prefix: String::new(),
            postfix: "_burned".to_owned(),
            output_directory: fixture.path.clone(),
            output_format: OutputFormat::Mp4,
            quality: QualityPreset::Balanced,
            acceleration: AccelerationPreference::Cpu,
            overwrite_existing: false,
            custom_ffmpeg_directory: None,
        };
        let cancelled = AtomicBool::new(false);
        let mut progress = Vec::new();

        encode_video(
            EncodeRequest {
                ffmpeg_path: &ffmpeg,
                video_path: &input,
                subtitle_path: &subtitle,
                output_path: &output,
                settings: &settings,
                duration_seconds: info.duration_seconds,
                encoder: VideoEncoder::Software,
                cancel_requested: &cancelled,
            },
            |value| progress.push(value),
        )
        .unwrap();

        assert!(output.is_file());
        assert!(fs::metadata(output).unwrap().len() > 0);
        assert_eq!(progress.last(), Some(&1.0));
    }

    struct TestFixture {
        path: PathBuf,
    }

    impl TestFixture {
        fn new() -> Self {
            let path = env::temp_dir().join(format!(
                "burnsubs-encode-test-{}-{}",
                std::process::id(),
                TEMP_COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            if path.exists() {
                fs::remove_dir_all(&path).unwrap();
            }
            fs::create_dir_all(&path).unwrap();
            Self { path }
        }
    }

    impl Drop for TestFixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
