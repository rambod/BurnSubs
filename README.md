# BurnSubs

BurnSubs is a small cross-platform desktop app for permanently rendering SRT subtitles into video.
It supports a single video/subtitle pair and predictable batch matching, with automatic H.264 GPU
acceleration and a safe CPU fallback.

> [!IMPORTANT]
> Hard subtitles become part of the video pixels, so the video must be encoded again. BurnSubs keeps
> the original dimensions and offers a high-quality preset, but no lossy re-encode is bit-for-bit
> identical to the source.

## Highlights

- Single-file and same-name batch workflows
- NVIDIA NVENC, Intel Quick Sync, AMD AMF, Linux VA-API, and Apple VideoToolbox
- A real runtime encoder test before every queue, with automatic `libx264` fallback
- MP4 or MKV output and three quality presets
- Output naming controls and overwrite protection
- Progress, cancellation, partial-file cleanup, and per-job error reporting
- Local settings only; no accounts, analytics, or network service

## Requirements

- Windows 10/11, a current macOS release, or a current 64-bit Linux desktop
- `ffmpeg` and `ffprobe` from the same FFmpeg installation
- An FFmpeg build containing `libass` and `libx264`; optional GPU encoders depend on your hardware
- A current graphics driver for hardware encoding

FFmpeg is deliberately not included in the source repository or official app archives. Install it
from the [official FFmpeg download page](https://ffmpeg.org/download.html), your operating system's
package manager, or select a custom FFmpeg folder in **Advanced**. BurnSubs also honors the
`BURNSUBS_FFMPEG_DIR` environment variable.

Check an installation with:

```console
ffmpeg -version
ffprobe -version
ffmpeg -filters
ffmpeg -encoders
```

The filter list must contain `subtitles`; the encoder list must contain `libx264`. A GPU encoder such
as `h264_nvenc`, `h264_qsv`, `h264_amf`, `h264_vaapi`, or `h264_videotoolbox` is optional.

## Install a release

1. Open the repository's **Releases** page.
2. Download the archive for your operating system and CPU.
3. Extract it, install FFmpeg, and run `BurnSubs` (`BurnSubs.exe` on Windows).

The macOS archives are unsigned. On first launch, Control-click BurnSubs, choose **Open**, and
confirm. Code signing and notarization require an Apple Developer identity and are not performed by
the public build workflow.

## Use BurnSubs

1. Select **Single** or **Batch**.
2. Choose the video and SRT file, or choose a batch folder and scan it.
3. Choose the output directory, container, quality, and acceleration mode.
4. Start the queue and monitor progress in the main panel.

Batch mode scans only the selected directory. A video is processed only when exactly one `.srt`
file has the same base name, ignoring case. Supported input containers are MKV, MP4, AVI, MOV, WebM,
and M4V; actual codec support comes from the installed FFmpeg build.

### GPU behavior

| Platform | Hardware encoders BurnSubs can use |
| --- | --- |
| Windows | NVIDIA NVENC, Intel Quick Sync, AMD AMF |
| Linux | NVIDIA NVENC, Intel Quick Sync, VA-API for Intel/AMD |
| macOS | Apple VideoToolbox |

**Automatic** performs a small one-frame test and selects the first working hardware encoder. If the
requested backend is not compiled into FFmpeg, has no usable device, or fails during a job, BurnSubs
reports the reason and retries safely with `libx264`.

Subtitle shaping and drawing are performed by FFmpeg/libass on the CPU. Completed frames are encoded
by the selected GPU. For this filter pipeline, forcing hardware decoding would normally add an
expensive GPU-to-CPU transfer, so BurnSubs intentionally leaves decoding automatic.

On Linux, the first `/dev/dri/renderD*` node is used for VA-API. Set `BURNSUBS_VAAPI_DEVICE` when a
specific render device is required.

## Build from source

Install the stable [Rust toolchain](https://www.rust-lang.org/tools/install). The project uses the
Rust 2024 edition.

```console
git clone <repository-url>
cd BurnSubs
cargo build --release --locked
```

The binary is written to `target/release/BurnSubs` (or `BurnSubs.exe` on Windows).

On Debian/Ubuntu, install the common desktop build dependencies first:

```console
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libdbus-1-dev libgl1-mesa-dev \
  libwayland-dev libx11-dev libxcursor-dev libxi-dev libxkbcommon-dev libxrandr-dev
```

Development checks:

```console
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
```

## Releases

The GitHub Actions release workflow builds native x86-64 Windows and Linux archives plus Intel and
Apple Silicon macOS archives. Push a SemVer tag such as `v1.0.0`; the workflow creates or updates a
**draft** release and uploads archives and SHA-256 files. Review the draft, then publish it from
GitHub, or run the workflow manually with **Publish release** enabled.

A draft can be updated repeatedly without creating another release. For a mutable published release,
additional assets can also be attached later. Rebuilding a version after users have downloaded it is
discouraged; prefer `v1.0.1` for code changes. Repositories with immutable releases enabled cannot
replace assets after publication, so attach everything while the release is still a draft.

See [CONTRIBUTING.md](CONTRIBUTING.md) for the complete maintainer checklist.

## License

BurnSubs is available under the [MIT License](LICENSE). FFmpeg is a separate project with separate
terms; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
