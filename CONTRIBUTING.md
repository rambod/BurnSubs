# Contributing to BurnSubs

Thank you for helping improve BurnSubs.

## Development

1. Install stable Rust and FFmpeg/FFprobe.
2. Create a focused branch from `main`.
3. Make the change and add or update tests.
4. Run the same checks as CI:

```console
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
cargo build --release --locked
```

Linux contributors may need the native packages listed in README.md. Keep platform-specific code
behind `cfg` attributes and do not commit local FFmpeg binaries, IDE metadata, `target/`, or `dist/`.

## Pull requests

- Explain the user-visible problem and the chosen solution.
- Keep unrelated formatting or refactors out of the change.
- Include manual test notes for UI, FFmpeg, GPU, or operating-system behavior.
- Update README.md and CHANGELOG.md when behavior or compatibility changes.

## Maintainer release checklist

1. Update `Cargo.toml` and `CHANGELOG.md` to the same SemVer version.
2. Run all checks on a clean checkout.
3. Commit and push the release code.
4. Create and push an annotated tag, for example:

   ```console
   git tag -a v1.0.0 -m "BurnSubs 1.0.0"
   git push origin v1.0.0
   ```

5. Wait for `.github/workflows/release.yml` to build every platform and update the draft release.
6. Download each archive, verify its `.sha256`, and smoke-test it on the matching operating system.
7. Review the generated notes and publish the draft from GitHub.

The release workflow can be run manually for an existing tag to replace draft assets. It can also
publish the draft when `publish` is selected. Do not move a version tag after publication; publish a
patch version instead.

### Add a locally built macOS archive to the same release

Run `sh ./scripts/package-release.sh` on the Mac, install and authenticate GitHub CLI, then upload the
archive and checksum to the existing draft:

```console
gh release upload v1.0.0 dist/BurnSubs-1.0.0-*-apple-darwin.tar.gz* --clobber
```

This updates the existing `v1.0.0` release; it does not create another one. It works after publication
only when release immutability is disabled.
