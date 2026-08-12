use crate::model::{AccelerationPreference, QualityPreset};
use anyhow::{Context, Result, anyhow, bail};
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

// 640x360 is accepted by current NVENC generations; tiny synthetic frames such as 64x64 can
// incorrectly report a healthy encoder as unavailable because they are below its minimum size.
const ENCODER_PROBE_SOURCE: &str = "color=c=black:s=640x360:r=30:d=1";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    Software,
    NvidiaNvenc,
    IntelQuickSync,
    AmdAmf,
    Vaapi,
    VideoToolbox,
}

impl VideoEncoder {
    pub const fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::Software => "libx264",
            Self::NvidiaNvenc => "h264_nvenc",
            Self::IntelQuickSync => "h264_qsv",
            Self::AmdAmf => "h264_amf",
            Self::Vaapi => "h264_vaapi",
            Self::VideoToolbox => "h264_videotoolbox",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Software => "CPU (libx264)",
            Self::NvidiaNvenc => "NVIDIA NVENC",
            Self::IntelQuickSync => "Intel Quick Sync",
            Self::AmdAmf => "AMD AMF",
            Self::Vaapi => "VA-API",
            Self::VideoToolbox => "Apple VideoToolbox",
        }
    }

    pub const fn is_hardware(self) -> bool {
        !matches!(self, Self::Software)
    }
}

#[derive(Debug, Clone)]
pub struct EncoderSelection {
    pub encoder: VideoEncoder,
    pub note: Option<String>,
}

pub fn select_encoder(
    ffmpeg_path: &Path,
    preference: AccelerationPreference,
) -> Result<EncoderSelection> {
    let compiled_encoders = compiled_encoder_names(ffmpeg_path)?;

    if !compiled_encoders.contains(VideoEncoder::Software.ffmpeg_name()) {
        bail!(
            "This FFmpeg build does not include the required CPU encoder '{}'.",
            VideoEncoder::Software.ffmpeg_name()
        );
    }

    probe_encoder(ffmpeg_path, VideoEncoder::Software)
        .context("The required CPU fallback encoder is unavailable")?;

    if preference == AccelerationPreference::Cpu {
        return Ok(EncoderSelection {
            encoder: VideoEncoder::Software,
            note: None,
        });
    }

    let candidates =
        if preference == AccelerationPreference::Auto {
            automatic_candidates()
        } else {
            vec![encoder_for_preference(preference).ok_or_else(|| {
                anyhow!("The selected acceleration mode is not a hardware encoder.")
            })?]
        };

    for encoder in candidates {
        if !compiled_encoders.contains(encoder.ffmpeg_name()) {
            tracing::debug!(
                encoder = encoder.label(),
                "Encoder is not compiled into FFmpeg"
            );
            continue;
        }

        match probe_encoder(ffmpeg_path, encoder) {
            Ok(()) => {
                return Ok(EncoderSelection {
                    encoder,
                    note: None,
                });
            }
            Err(error) => {
                tracing::debug!(
                    encoder = encoder.label(),
                    error = %error,
                    "Hardware encoder runtime probe failed"
                );
            }
        }
    }

    let note = if preference == AccelerationPreference::Auto {
        "No compatible GPU encoder passed the runtime test; using CPU encoding.".to_owned()
    } else {
        format!(
            "{} is unavailable on this system; using CPU encoding.",
            preference.label()
        )
    };

    Ok(EncoderSelection {
        encoder: VideoEncoder::Software,
        note: Some(note),
    })
}

pub fn configure_device(command: &mut Command, encoder: VideoEncoder) -> Result<()> {
    if encoder == VideoEncoder::Vaapi {
        command.arg("-vaapi_device").arg(find_vaapi_device()?);
    }

    Ok(())
}

pub fn subtitle_filter(encoder: VideoEncoder) -> &'static str {
    match encoder {
        VideoEncoder::IntelQuickSync | VideoEncoder::AmdAmf => {
            "subtitles=filename=subtitle.srt:charenc=UTF-8,format=nv12"
        }
        VideoEncoder::Vaapi => "subtitles=filename=subtitle.srt:charenc=UTF-8,format=nv12,hwupload",
        VideoEncoder::Software | VideoEncoder::NvidiaNvenc | VideoEncoder::VideoToolbox => {
            "subtitles=filename=subtitle.srt:charenc=UTF-8"
        }
    }
}

pub fn apply_video_options(command: &mut Command, encoder: VideoEncoder, quality: QualityPreset) {
    command.arg("-c:v").arg(encoder.ffmpeg_name());

    match encoder {
        VideoEncoder::Software => {
            let (crf, preset) = software_quality(quality);
            command
                .arg("-crf")
                .arg(crf)
                .arg("-preset")
                .arg(preset)
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        VideoEncoder::NvidiaNvenc => {
            let (cq, preset) = hardware_quality(quality);
            command
                .arg("-preset")
                .arg(preset)
                .arg("-tune")
                .arg("hq")
                .arg("-rc")
                .arg("vbr")
                .arg("-cq")
                .arg(cq)
                .arg("-b:v")
                .arg("0")
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
        VideoEncoder::IntelQuickSync => {
            let (quality_value, preset) = hardware_quality(quality);
            command
                .arg("-global_quality")
                .arg(quality_value)
                .arg("-preset")
                .arg(preset);
        }
        VideoEncoder::AmdAmf => {
            let (qp, quality_mode) = amd_quality(quality);
            command
                .arg("-usage")
                .arg("transcoding")
                .arg("-quality")
                .arg(quality_mode)
                .arg("-rc")
                .arg("cqp")
                .arg("-qp_i")
                .arg(qp)
                .arg("-qp_p")
                .arg(qp)
                .arg("-qp_b")
                .arg(qp);
        }
        VideoEncoder::Vaapi => {
            let (qp, _) = hardware_quality(quality);
            command.arg("-rc_mode").arg("CQP").arg("-qp").arg(qp);
        }
        VideoEncoder::VideoToolbox => {
            command
                .arg("-q:v")
                .arg(videotoolbox_quality(quality))
                .arg("-pix_fmt")
                .arg("yuv420p");
        }
    }
}

fn compiled_encoder_names(ffmpeg_path: &Path) -> Result<HashSet<String>> {
    let mut command = Command::new(ffmpeg_path);
    command
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);

    let output = command
        .output()
        .with_context(|| format!("Failed to inspect FFmpeg: {}", ffmpeg_path.display()))?;

    if !output.status.success() {
        bail!(
            "FFmpeg could not list its encoders: {}",
            concise_stderr(&output.stderr)
        );
    }

    Ok(parse_encoder_names(&String::from_utf8_lossy(
        &output.stdout,
    )))
}

fn parse_encoder_names(output: &str) -> HashSet<String> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = fields.next()?;
            let name = fields.next()?;

            (flags.len() == 6 && flags.starts_with('V')).then(|| name.to_owned())
        })
        .collect()
}

fn probe_encoder(ffmpeg_path: &Path, encoder: VideoEncoder) -> Result<()> {
    let mut command = Command::new(ffmpeg_path);
    command.args(["-hide_banner", "-loglevel", "error"]);
    configure_device(&mut command, encoder)?;
    command.args([
        "-f",
        "lavfi",
        "-i",
        ENCODER_PROBE_SOURCE,
        "-frames:v",
        "1",
        "-an",
    ]);

    if matches!(encoder, VideoEncoder::IntelQuickSync | VideoEncoder::AmdAmf) {
        command.args(["-vf", "format=nv12"]);
    } else if encoder == VideoEncoder::Vaapi {
        command.args(["-vf", "format=nv12,hwupload"]);
    }

    apply_video_options(&mut command, encoder, QualityPreset::Balanced);

    command
        .args(["-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    hide_console_window(&mut command);

    let output = command.output().with_context(|| {
        format!(
            "Failed to run the {} runtime test with FFmpeg",
            encoder.label()
        )
    })?;

    if !output.status.success() {
        bail!(
            "{} runtime test failed: {}",
            encoder.label(),
            concise_stderr(&output.stderr)
        );
    }

    Ok(())
}

fn automatic_candidates() -> Vec<VideoEncoder> {
    #[cfg(target_os = "windows")]
    {
        vec![
            VideoEncoder::NvidiaNvenc,
            VideoEncoder::IntelQuickSync,
            VideoEncoder::AmdAmf,
        ]
    }

    #[cfg(target_os = "linux")]
    {
        vec![
            VideoEncoder::NvidiaNvenc,
            VideoEncoder::Vaapi,
            VideoEncoder::IntelQuickSync,
        ]
    }

    #[cfg(target_os = "macos")]
    {
        vec![VideoEncoder::VideoToolbox]
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Vec::new()
    }
}

const fn encoder_for_preference(preference: AccelerationPreference) -> Option<VideoEncoder> {
    match preference {
        AccelerationPreference::Auto | AccelerationPreference::Cpu => None,
        AccelerationPreference::Nvidia => Some(VideoEncoder::NvidiaNvenc),
        AccelerationPreference::IntelQuickSync => Some(VideoEncoder::IntelQuickSync),
        AccelerationPreference::Amd => Some(VideoEncoder::AmdAmf),
        AccelerationPreference::Vaapi => Some(VideoEncoder::Vaapi),
        AccelerationPreference::VideoToolbox => Some(VideoEncoder::VideoToolbox),
    }
}

fn find_vaapi_device() -> Result<PathBuf> {
    if let Some(configured_device) = env::var_os("BURNSUBS_VAAPI_DEVICE") {
        let path = PathBuf::from(configured_device);
        if path.exists() {
            return Ok(path);
        }
        bail!(
            "Configured VA-API device does not exist: {}",
            path.display()
        );
    }

    #[cfg(target_os = "linux")]
    {
        let dri_directory = Path::new("/dev/dri");
        let mut render_nodes = std::fs::read_dir(dri_directory)
            .context("VA-API device directory /dev/dri is unavailable")?
            .filter_map(std::result::Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with("renderD"))
            })
            .collect::<Vec<_>>();

        render_nodes.sort();
        return render_nodes
            .into_iter()
            .next()
            .context("No VA-API render device was found in /dev/dri");
    }

    #[cfg(not(target_os = "linux"))]
    bail!("VA-API encoding is only supported on Linux by BurnSubs");
}

const fn software_quality(quality: QualityPreset) -> (&'static str, &'static str) {
    match quality {
        QualityPreset::High => ("18", "slow"),
        QualityPreset::Balanced => ("21", "medium"),
        QualityPreset::SmallerFile => ("24", "slow"),
    }
}

const fn hardware_quality(quality: QualityPreset) -> (&'static str, &'static str) {
    match quality {
        QualityPreset::High => ("18", "slow"),
        QualityPreset::Balanced => ("21", "medium"),
        QualityPreset::SmallerFile => ("25", "slow"),
    }
}

const fn amd_quality(quality: QualityPreset) -> (&'static str, &'static str) {
    match quality {
        QualityPreset::High => ("18", "quality"),
        QualityPreset::Balanced => ("21", "balanced"),
        QualityPreset::SmallerFile => ("25", "quality"),
    }
}

const fn videotoolbox_quality(quality: QualityPreset) -> &'static str {
    match quality {
        QualityPreset::High => "75",
        QualityPreset::Balanced => "60",
        QualityPreset::SmallerFile => "45",
    }
}

fn concise_stderr(stderr: &[u8]) -> String {
    let details = String::from_utf8_lossy(stderr);
    let details = details.trim();
    if details.is_empty() {
        return "FFmpeg returned no error details.".to_owned();
    }

    const MAX_CHARACTERS: usize = 2_000;
    if details.chars().count() <= MAX_CHARACTERS {
        details.to_owned()
    } else {
        let tail = details
            .chars()
            .rev()
            .take(MAX_CHARACTERS)
            .collect::<String>()
            .chars()
            .rev()
            .collect::<String>();
        format!("…{tail}")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_only_video_encoder_rows() {
        let output = "Encoders:\n V....D libx264 description\n A....D aac description\n V..... h264_qsv description\n";
        let names = parse_encoder_names(output);

        assert!(names.contains("libx264"));
        assert!(names.contains("h264_qsv"));
        assert!(!names.contains("aac"));
    }

    #[test]
    fn maps_preferences_to_encoders() {
        assert_eq!(
            encoder_for_preference(AccelerationPreference::Nvidia),
            Some(VideoEncoder::NvidiaNvenc)
        );
        assert_eq!(encoder_for_preference(AccelerationPreference::Cpu), None);
    }

    #[test]
    fn builds_cpu_quality_options() {
        let mut command = Command::new("ffmpeg");
        apply_video_options(&mut command, VideoEncoder::Software, QualityPreset::High);
        let debug = format!("{command:?}");

        assert!(debug.contains("libx264"));
        assert!(debug.contains("-crf"));
        assert!(debug.contains("slow"));
    }

    #[test]
    fn vaapi_filter_uploads_frames() {
        assert!(subtitle_filter(VideoEncoder::Vaapi).ends_with("format=nv12,hwupload"));
    }

    #[test]
    fn hardware_probe_uses_nvenc_compatible_dimensions() {
        assert!(ENCODER_PROBE_SOURCE.contains("s=640x360"));
    }

    #[test]
    #[ignore = "executes the locally installed FFmpeg binary"]
    fn installed_ffmpeg_has_a_working_auto_encoder() {
        let paths = crate::ffmpeg::locator::locate_ffmpeg(None).unwrap();
        let selection = select_encoder(&paths.ffmpeg, AccelerationPreference::Auto).unwrap();
        println!("Selected encoder: {}", selection.encoder.label());
        assert!(!selection.encoder.label().is_empty());
    }
}
