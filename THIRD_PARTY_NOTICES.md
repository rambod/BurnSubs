# Third-party notices

BurnSubs uses Rust crates listed in `Cargo.lock`. Each crate remains subject to its own license.

## FFmpeg

BurnSubs starts `ffmpeg` and `ffprobe` as separate processes. FFmpeg is not part of this source
repository or the official BurnSubs application archives. Users install it separately or select a
custom folder containing both programs.

FFmpeg is developed by the FFmpeg project and is available under the LGPL 2.1-or-later, with GPL
terms applying when GPL components are enabled. The exact terms for an installed build are shown by
running `ffmpeg -L`.

- Project: <https://ffmpeg.org/>
- License information: <https://ffmpeg.org/legal.html>
- Source: <https://ffmpeg.org/download.html#get-sources>

Anyone redistributing a BurnSubs archive that also contains FFmpeg is responsible for satisfying
the license and corresponding-source obligations of that exact FFmpeg build.
