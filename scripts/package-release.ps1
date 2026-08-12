param(
    [string]$Target = "x86_64-pc-windows-msvc"
)

$ErrorActionPreference = "Stop"
$projectRoot = Split-Path -Parent $PSScriptRoot
$manifest = Get-Content -Raw (Join-Path $projectRoot "Cargo.toml")
$versionMatch = [regex]::Match($manifest, '(?m)^version\s*=\s*"([^"]+)"')

if (-not $versionMatch.Success) {
    throw "Could not read the package version from Cargo.toml."
}

$version = $versionMatch.Groups[1].Value
$packageName = "BurnSubs-$version-$Target"
$dist = Join-Path $projectRoot "dist"
$stage = Join-Path $dist $packageName
$binary = Join-Path $projectRoot "target\release\BurnSubs.exe"
$archive = Join-Path $dist "$packageName.zip"

if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) {
    throw "Release binary not found at $binary. Run cargo build --release --locked first."
}

New-Item -ItemType Directory -Force -Path $stage | Out-Null
Copy-Item -LiteralPath $binary -Destination (Join-Path $stage "BurnSubs.exe") -Force
Copy-Item -LiteralPath (Join-Path $projectRoot "README.md") -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $projectRoot "LICENSE") -Destination $stage -Force
Copy-Item -LiteralPath (Join-Path $projectRoot "THIRD_PARTY_NOTICES.md") -Destination $stage -Force

if (Test-Path -LiteralPath $archive) {
    Remove-Item -LiteralPath $archive -Force
}

Compress-Archive -Path $stage -DestinationPath $archive -CompressionLevel Optimal
$hash = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  $([IO.Path]::GetFileName($archive))" |
    Set-Content -LiteralPath "$archive.sha256" -Encoding utf8NoBOM

Write-Host "Created $archive"
