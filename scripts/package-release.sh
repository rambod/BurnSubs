#!/usr/bin/env sh
set -eu

project_root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$project_root"

version=$(sed -n 's/^version[[:space:]]*=[[:space:]]*"\([^"]*\)"/\1/p' Cargo.toml | head -n 1)
if [ -z "$version" ]; then
    echo "Could not read the package version from Cargo.toml." >&2
    exit 1
fi

case "$(uname -s)" in
    Linux) target="x86_64-unknown-linux-gnu" ;;
    Darwin)
        case "$(uname -m)" in
            arm64) target="aarch64-apple-darwin" ;;
            x86_64) target="x86_64-apple-darwin" ;;
            *) echo "Unsupported macOS architecture: $(uname -m)" >&2; exit 1 ;;
        esac
        ;;
    *) echo "This script supports Linux and macOS. Use package-release.ps1 on Windows." >&2; exit 1 ;;
esac

binary="target/release/BurnSubs"
if [ ! -f "$binary" ]; then
    echo "Release binary not found at $binary. Run cargo build --release --locked first." >&2
    exit 1
fi

package_name="BurnSubs-$version-$target"
stage="dist/$package_name"
archive="dist/$package_name.tar.gz"
mkdir -p "$stage"

if [ "$(uname -s)" = "Darwin" ]; then
    app="$stage/BurnSubs.app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    install -m 0755 "$binary" "$app/Contents/MacOS/BurnSubs"
    cp README.md LICENSE THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"
    cat > "$app/Contents/Info.plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "https://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key><string>BurnSubs</string>
  <key>CFBundleExecutable</key><string>BurnSubs</string>
  <key>CFBundleIdentifier</key><string>net.rambod.burnsubs</string>
  <key>CFBundleName</key><string>BurnSubs</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$version</string>
  <key>CFBundleVersion</key><string>$version</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
EOF
else
    install -m 0755 "$binary" "$stage/BurnSubs"
    cp README.md LICENSE THIRD_PARTY_NOTICES.md "$stage/"
fi

tar -C dist -czf "$archive" "$package_name"

if command -v sha256sum >/dev/null 2>&1; then
    (cd dist && sha256sum "$(basename "$archive")") > "$archive.sha256"
else
    (cd dist && shasum -a 256 "$(basename "$archive")") > "$archive.sha256"
fi

echo "Created $archive"
