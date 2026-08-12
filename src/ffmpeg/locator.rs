use anyhow::{Context, Result, anyhow};
use std::{
    collections::HashSet,
    env,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone)]
pub struct FfmpegPaths {
    pub ffmpeg: PathBuf,
    pub ffprobe: PathBuf,
}

pub fn locate_ffmpeg(custom_directory: Option<&Path>) -> Result<FfmpegPaths> {
    let target_directory = current_target_directory()?;
    let mut searched_directories = Vec::new();

    if let Some(directory) = custom_directory {
        let custom_candidates = [directory.to_path_buf(), directory.join(target_directory)];

        for candidate in &custom_candidates {
            if let Some(paths) = binaries_in_directory(candidate) {
                tracing::info!(
                    ffmpeg = %paths.ffmpeg.display(),
                    ffprobe = %paths.ffprobe.display(),
                    "Located custom FFmpeg binaries"
                );
                return Ok(paths);
            }
        }

        return custom_directory_error(directory, &custom_candidates);
    }

    for directory in candidate_directories(target_directory) {
        searched_directories.push(directory.clone());

        if let Some(paths) = binaries_in_directory(&directory) {
            tracing::info!(
                ffmpeg = %paths.ffmpeg.display(),
                ffprobe = %paths.ffprobe.display(),
                "Located FFmpeg binaries"
            );

            return Ok(paths);
        }
    }

    if let Some(paths) = locate_in_system_path() {
        tracing::info!(
            ffmpeg = %paths.ffmpeg.display(),
            ffprobe = %paths.ffprobe.display(),
            "Located FFmpeg binaries in system PATH"
        );

        return Ok(paths);
    }

    let searched = searched_directories
        .iter()
        .map(|path| format!("  - {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");

    Err(anyhow!(
        "Could not locate FFmpeg and FFprobe.\n\
         Expected both binaries in one of these directories:\n{searched}\n\
         Or available through the system PATH."
    ))
}

fn candidate_directories(target_directory: &str) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    // Explicit override for development, testing, or custom installations.
    if let Some(directory) = env::var_os("BURNSUBS_FFMPEG_DIR") {
        candidates.push(PathBuf::from(directory));
    }

    if let Ok(executable_path) = env::current_exe()
        && let Some(executable_directory) = executable_path.parent()
    {
        candidates.push(executable_directory.join("tools").join(target_directory));

        candidates.push(executable_directory.join("tools"));
        candidates.push(executable_directory.to_path_buf());

        // During development the executable is normally under:
        // target/debug or target/release.
        //
        // Walking through its ancestors allows discovery of:
        // project-root/tools/<target-triple>.
        for ancestor in executable_directory.ancestors().take(6) {
            candidates.push(ancestor.join("tools").join(target_directory));
        }

        #[cfg(target_os = "macos")]
        {
            // Packaged macOS application:
            // BurnSubs.app/Contents/MacOS/BurnSubs
            // BurnSubs.app/Contents/Resources/tools/<target-triple>
            if let Some(contents_directory) = executable_directory.parent() {
                candidates.push(
                    contents_directory
                        .join("Resources")
                        .join("tools")
                        .join(target_directory),
                );

                candidates.push(contents_directory.join("Resources").join("tools"));
            }
        }
    }

    if let Ok(current_directory) = env::current_dir() {
        candidates.push(current_directory.join("tools").join(target_directory));

        candidates.push(current_directory.join("tools"));
    }

    remove_duplicate_paths(candidates)
}

fn custom_directory_error(
    configured_directory: &Path,
    candidates: &[PathBuf],
) -> Result<FfmpegPaths> {
    let searched = candidates
        .iter()
        .map(|path| format!("  - {}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");

    Err(anyhow!(
        "The custom FFmpeg folder '{}' does not contain both {} and {}.\nSearched:\n{}",
        configured_directory.display(),
        ffmpeg_file_name(),
        ffprobe_file_name(),
        searched
    ))
}

fn binaries_in_directory(directory: &Path) -> Option<FfmpegPaths> {
    let ffmpeg = directory.join(ffmpeg_file_name());
    let ffprobe = directory.join(ffprobe_file_name());

    if ffmpeg.is_file() && ffprobe.is_file() {
        return Some(FfmpegPaths {
            ffmpeg: absolute_path(ffmpeg),
            ffprobe: absolute_path(ffprobe),
        });
    }

    None
}

fn locate_in_system_path() -> Option<FfmpegPaths> {
    let ffmpeg = find_in_path(ffmpeg_file_name())?;
    let ffprobe = find_in_path(ffprobe_file_name())?;

    Some(FfmpegPaths { ffmpeg, ffprobe })
}

fn find_in_path(file_name: &str) -> Option<PathBuf> {
    let system_path = env::var_os("PATH")?;

    for directory in env::split_paths(&system_path) {
        let candidate = directory.join(file_name);

        if candidate.is_file() {
            return Some(absolute_path(candidate));
        }
    }

    None
}

fn absolute_path(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

fn remove_duplicate_paths(paths: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut unique_paths = Vec::new();

    for path in paths {
        if seen.insert(path.clone()) {
            unique_paths.push(path);
        }
    }

    unique_paths
}

const fn ffmpeg_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    }
}

const fn ffprobe_file_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "ffprobe.exe"
    } else {
        "ffprobe"
    }
}

fn current_target_directory() -> Result<&'static str> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok("x86_64-pc-windows-msvc");
    }

    #[cfg(all(target_os = "windows", target_arch = "aarch64"))]
    {
        return Ok("aarch64-pc-windows-msvc");
    }

    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Ok("x86_64-apple-darwin");
    }

    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok("aarch64-apple-darwin");
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok("x86_64-unknown-linux-gnu");
    }

    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        return Ok("aarch64-unknown-linux-gnu");
    }

    #[allow(unreachable_code)]
    Err(anyhow!(
        "BurnSubs does not currently support target {}-{}.",
        env::consts::OS,
        env::consts::ARCH
    ))
}

pub fn validate_binary_files(paths: &FfmpegPaths) -> Result<()> {
    if !paths.ffmpeg.is_file() {
        return Err(anyhow!(
            "FFmpeg binary does not exist: {}",
            paths.ffmpeg.display()
        ));
    }

    if !paths.ffprobe.is_file() {
        return Err(anyhow!(
            "FFprobe binary does not exist: {}",
            paths.ffprobe.display()
        ));
    }

    paths
        .ffmpeg
        .canonicalize()
        .with_context(|| format!("Could not access FFmpeg binary: {}", paths.ffmpeg.display()))?;

    paths.ffprobe.canonicalize().with_context(|| {
        format!(
            "Could not access FFprobe binary: {}",
            paths.ffprobe.display()
        )
    })?;

    Ok(())
}
