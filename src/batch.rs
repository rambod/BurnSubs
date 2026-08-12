use crate::model::{EncodeJob, EncodeSettings};
use anyhow::{Context, Result, bail};
use std::{
    collections::{HashMap, HashSet},
    env, fmt, fs,
    path::{Path, PathBuf},
};

pub const VIDEO_EXTENSIONS: &[&str] = &["mkv", "mp4", "avi", "mov", "webm", "m4v"];

#[derive(Debug, Clone)]
pub struct BatchScanResult {
    pub jobs: Vec<EncodeJob>,
    pub skipped_videos: Vec<SkippedVideo>,
    pub orphan_subtitles: Vec<PathBuf>,
    pub ignored_file_count: usize,
}

#[derive(Debug, Clone)]
pub struct SkippedVideo {
    pub video_path: PathBuf,
    pub reason: BatchSkipReason,
}

#[derive(Debug, Clone)]
pub enum BatchSkipReason {
    MissingSubtitle,

    AmbiguousSubtitle {
        candidates: Vec<PathBuf>,
    },

    OutputAlreadyExists {
        output_path: PathBuf,
    },

    OutputWouldReplaceInput {
        output_path: PathBuf,
    },

    OutputConflict {
        output_path: PathBuf,
        videos: Vec<PathBuf>,
    },
}

impl fmt::Display for BatchSkipReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSubtitle => {
                write!(formatter, "No same-name SRT subtitle was found")
            }

            Self::AmbiguousSubtitle { candidates } => {
                write!(
                    formatter,
                    "Multiple matching SRT files were found: {}",
                    join_paths(candidates)
                )
            }

            Self::OutputAlreadyExists { output_path } => {
                write!(
                    formatter,
                    "Output already exists: {}",
                    output_path.display()
                )
            }

            Self::OutputWouldReplaceInput { output_path } => {
                write!(
                    formatter,
                    "Output would replace the input video: {}",
                    output_path.display()
                )
            }

            Self::OutputConflict {
                output_path,
                videos,
            } => {
                write!(
                    formatter,
                    "Multiple videos would produce the same output '{}': {}",
                    output_path.display(),
                    join_paths(videos)
                )
            }
        }
    }
}

pub fn scan_batch_folder(
    input_directory: &Path,
    settings: &EncodeSettings,
) -> Result<BatchScanResult> {
    validate_input_directory(input_directory)?;

    let mut video_paths = Vec::new();
    let mut subtitle_paths = Vec::new();
    let mut ignored_file_count = 0;

    let entries = fs::read_dir(input_directory).with_context(|| {
        format!(
            "Could not read batch input directory: {}",
            input_directory.display()
        )
    })?;

    for entry_result in entries {
        let entry = entry_result.with_context(|| {
            format!(
                "Could not read an entry inside: {}",
                input_directory.display()
            )
        })?;

        let path = entry.path();

        if !path.is_file() {
            continue;
        }

        if has_supported_video_extension(&path) {
            video_paths.push(path);
        } else if has_extension(&path, "srt") {
            subtitle_paths.push(path);
        } else {
            ignored_file_count += 1;
        }
    }

    sort_paths(&mut video_paths);
    sort_paths(&mut subtitle_paths);

    let subtitles_by_stem = group_paths_by_stem(&subtitle_paths);
    let video_stems = video_paths
        .iter()
        .filter_map(|path| normalized_stem(path))
        .collect::<HashSet<_>>();

    let mut orphan_subtitles = subtitle_paths
        .iter()
        .filter(|subtitle_path| {
            normalized_stem(subtitle_path).is_none_or(|stem| !video_stems.contains(&stem))
        })
        .cloned()
        .collect::<Vec<_>>();

    sort_paths(&mut orphan_subtitles);

    let mut skipped_videos = Vec::new();
    let mut candidates = Vec::new();

    for video_path in video_paths {
        let Some(video_stem) = normalized_stem(&video_path) else {
            skipped_videos.push(SkippedVideo {
                video_path,
                reason: BatchSkipReason::MissingSubtitle,
            });
            continue;
        };

        let Some(matching_subtitles) = subtitles_by_stem.get(&video_stem) else {
            skipped_videos.push(SkippedVideo {
                video_path,
                reason: BatchSkipReason::MissingSubtitle,
            });
            continue;
        };

        if matching_subtitles.len() > 1 {
            skipped_videos.push(SkippedVideo {
                video_path,
                reason: BatchSkipReason::AmbiguousSubtitle {
                    candidates: matching_subtitles.clone(),
                },
            });
            continue;
        }

        let subtitle_path = matching_subtitles[0].clone();
        let output_path = settings.create_output_path(&video_path);

        candidates.push(CandidateJob {
            video_path,
            subtitle_path,
            output_path,
        });
    }

    let generated_output_keys = candidates
        .iter()
        .map(|candidate| path_comparison_key(&candidate.output_path))
        .collect::<HashSet<_>>();

    let mut retained_skipped_videos = Vec::new();

    for skipped_video in skipped_videos {
        let is_previous_generated_output =
            matches!(skipped_video.reason, BatchSkipReason::MissingSubtitle)
                && generated_output_keys.contains(&path_comparison_key(&skipped_video.video_path));

        if is_previous_generated_output {
            ignored_file_count += 1;
        } else {
            retained_skipped_videos.push(skipped_video);
        }
    }

    let mut skipped_videos = retained_skipped_videos;
    let output_conflicts = find_output_conflicts(&candidates);
    let mut jobs = Vec::new();

    for (candidate_index, candidate) in candidates.into_iter().enumerate() {
        if let Some(conflicting_videos) = output_conflicts.get(&candidate_index) {
            skipped_videos.push(SkippedVideo {
                video_path: candidate.video_path,
                reason: BatchSkipReason::OutputConflict {
                    output_path: candidate.output_path,
                    videos: conflicting_videos.clone(),
                },
            });
            continue;
        }

        if paths_conflict(&candidate.video_path, &candidate.output_path)? {
            skipped_videos.push(SkippedVideo {
                video_path: candidate.video_path,
                reason: BatchSkipReason::OutputWouldReplaceInput {
                    output_path: candidate.output_path,
                },
            });
            continue;
        }

        if candidate.output_path.exists() && !settings.overwrite_existing {
            skipped_videos.push(SkippedVideo {
                video_path: candidate.video_path,
                reason: BatchSkipReason::OutputAlreadyExists {
                    output_path: candidate.output_path,
                },
            });
            continue;
        }

        let job_id = jobs.len() as u64 + 1;

        jobs.push(EncodeJob::new(
            job_id,
            candidate.video_path,
            candidate.subtitle_path,
            candidate.output_path,
        ));
    }

    skipped_videos.sort_by(|left, right| {
        path_sort_key(&left.video_path).cmp(&path_sort_key(&right.video_path))
    });

    Ok(BatchScanResult {
        jobs,
        skipped_videos,
        orphan_subtitles,
        ignored_file_count,
    })
}

pub fn has_supported_video_extension(path: &Path) -> bool {
    VIDEO_EXTENSIONS
        .iter()
        .any(|extension| has_extension(path, extension))
}

fn validate_input_directory(input_directory: &Path) -> Result<()> {
    if !input_directory.exists() {
        bail!(
            "Batch input directory does not exist: {}",
            input_directory.display()
        );
    }

    if !input_directory.is_dir() {
        bail!(
            "Batch input path is not a directory: {}",
            input_directory.display()
        );
    }

    Ok(())
}

fn group_paths_by_stem(paths: &[PathBuf]) -> HashMap<String, Vec<PathBuf>> {
    let mut grouped_paths = HashMap::<String, Vec<PathBuf>>::new();

    for path in paths {
        let Some(stem) = normalized_stem(path) else {
            continue;
        };

        grouped_paths.entry(stem).or_default().push(path.clone());
    }

    for matching_paths in grouped_paths.values_mut() {
        sort_paths(matching_paths);
    }

    grouped_paths
}

fn find_output_conflicts(candidates: &[CandidateJob]) -> HashMap<usize, Vec<PathBuf>> {
    let mut indices_by_output = HashMap::<String, Vec<usize>>::new();

    for (index, candidate) in candidates.iter().enumerate() {
        indices_by_output
            .entry(path_comparison_key(&candidate.output_path))
            .or_default()
            .push(index);
    }

    let mut conflicts = HashMap::new();

    for indices in indices_by_output.values() {
        if indices.len() < 2 {
            continue;
        }

        let videos = indices
            .iter()
            .map(|index| candidates[*index].video_path.clone())
            .collect::<Vec<_>>();

        for index in indices {
            conflicts.insert(*index, videos.clone());
        }
    }

    conflicts
}

fn paths_conflict(left: &Path, right: &Path) -> Result<bool> {
    Ok(path_comparison_key(&make_absolute(left)?) == path_comparison_key(&make_absolute(right)?))
}

fn make_absolute(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }

    Ok(env::current_dir()
        .context("Could not determine the current directory")?
        .join(path))
}

fn normalized_stem(path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_string_lossy();

    if stem.is_empty() {
        return None;
    }

    Some(stem.to_lowercase())
}

fn has_extension(path: &Path, expected_extension: &str) -> bool {
    path.extension().is_some_and(|extension| {
        extension
            .to_string_lossy()
            .eq_ignore_ascii_case(expected_extension)
    })
}

fn path_comparison_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn path_sort_key(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_else(|| path_comparison_key(path))
}

fn sort_paths(paths: &mut [PathBuf]) {
    paths.sort_by_key(|path| path_sort_key(path));
}

fn join_paths(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|path| path.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

#[derive(Debug)]
struct CandidateJob {
    video_path: PathBuf,
    subtitle_path: PathBuf,
    output_path: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{AccelerationPreference, OutputFormat, QualityPreset};
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[test]
    fn matches_video_and_subtitle_by_stem() {
        let directory = TestDirectory::new("matches");

        create_file(&directory.path.join("Movie.mkv"));
        create_file(&directory.path.join("movie.SRT"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert_eq!(result.jobs.len(), 1);
        assert!(result.skipped_videos.is_empty());
        assert!(result.orphan_subtitles.is_empty());
        assert_eq!(result.jobs[0].video_name(), "Movie.mkv");
        assert_eq!(result.jobs[0].subtitle_name(), "movie.SRT");
    }

    #[test]
    fn reports_missing_subtitle() {
        let directory = TestDirectory::new("missing-subtitle");

        create_file(&directory.path.join("movie.mkv"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert!(result.jobs.is_empty());
        assert_eq!(result.skipped_videos.len(), 1);

        assert!(matches!(
            result.skipped_videos[0].reason,
            BatchSkipReason::MissingSubtitle
        ));
    }

    #[test]
    fn reports_orphan_subtitle() {
        let directory = TestDirectory::new("orphan-subtitle");

        create_file(&directory.path.join("movie.mkv"));
        create_file(&directory.path.join("movie.srt"));
        create_file(&directory.path.join("unused.srt"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert_eq!(result.jobs.len(), 1);
        assert_eq!(result.orphan_subtitles.len(), 1);
        assert_eq!(
            result.orphan_subtitles[0]
                .file_name()
                .unwrap()
                .to_string_lossy(),
            "unused.srt"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reports_ambiguous_subtitles() {
        let directory = TestDirectory::new("ambiguous");

        create_file(&directory.path.join("movie.mkv"));
        create_file(&directory.path.join("movie.srt"));
        create_file(&directory.path.join("MOVIE.SRT"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert!(result.jobs.is_empty());
        assert_eq!(result.skipped_videos.len(), 1);

        match &result.skipped_videos[0].reason {
            BatchSkipReason::AmbiguousSubtitle { candidates } => {
                assert_eq!(candidates.len(), 2);
            }
            other => panic!("Unexpected skip reason: {other:?}"),
        }
    }

    #[test]
    fn reports_output_conflict_for_duplicate_video_stems() {
        let directory = TestDirectory::new("output-conflict");

        create_file(&directory.path.join("movie.mkv"));
        create_file(&directory.path.join("movie.mp4"));
        create_file(&directory.path.join("movie.srt"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert!(result.jobs.is_empty());
        assert_eq!(result.skipped_videos.len(), 2);

        assert!(
            result
                .skipped_videos
                .iter()
                .all(|skipped| matches!(skipped.reason, BatchSkipReason::OutputConflict { .. }))
        );
    }

    #[test]
    fn skips_existing_output_when_overwrite_is_disabled() {
        let directory = TestDirectory::new("existing-output");

        create_file(&directory.path.join("movie.mkv"));
        create_file(&directory.path.join("movie.srt"));
        create_file(&directory.path.join("movie_hardsub.mp4"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert!(result.jobs.is_empty());
        assert_eq!(result.skipped_videos.len(), 1);
        assert_eq!(result.ignored_file_count, 1);

        assert!(matches!(
            result.skipped_videos[0].reason,
            BatchSkipReason::OutputAlreadyExists { .. }
        ));
    }

    #[test]
    fn counts_ignored_files() {
        let directory = TestDirectory::new("ignored-files");

        create_file(&directory.path.join("movie.mkv"));
        create_file(&directory.path.join("movie.srt"));
        create_file(&directory.path.join("notes.txt"));

        let result = scan_batch_folder(&directory.path, &settings_for(&directory.path)).unwrap();

        assert_eq!(result.jobs.len(), 1);
        assert_eq!(result.ignored_file_count, 1);
    }

    fn settings_for(output_directory: &Path) -> EncodeSettings {
        EncodeSettings {
            prefix: String::new(),
            postfix: "_hardsub".to_owned(),
            output_directory: output_directory.to_path_buf(),
            output_format: OutputFormat::Mp4,
            quality: QualityPreset::Balanced,
            acceleration: AccelerationPreference::Auto,
            overwrite_existing: false,
            custom_ffmpeg_directory: None,
        }
    }

    fn create_file(path: &Path) {
        fs::write(path, []).unwrap();
    }

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new(name: &str) -> Self {
            let id = TEST_DIRECTORY_COUNTER.fetch_add(1, Ordering::Relaxed);

            let path = env::temp_dir().join(format!(
                "burnsubs-batch-test-{}-{name}-{id}",
                std::process::id()
            ));

            if path.exists() {
                fs::remove_dir_all(&path).unwrap();
            }

            fs::create_dir_all(&path).unwrap();

            Self { path }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
