# Changelog

All notable changes to BurnSubs are documented here. The project follows
[Semantic Versioning](https://semver.org/).

## [1.0.0] - 2026-08-12

### Added

- Single and batch hard-subtitle workflows.
- Automatic NVIDIA, Intel, AMD, VA-API, and VideoToolbox encoder detection.
- CPU fallback when hardware encoding is unavailable or fails.
- Cross-platform CI and draft-release packaging for Windows, Linux, and macOS.

### Changed

- Reorganized the interface into a compact setup, queue, and persistent action layout.
- Removed third-party executable artifacts from the source repository.
