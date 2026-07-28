# Downloads the FFmpeg "release essentials" build for Windows and places
# ffmpeg.exe / ffprobe.exe where Tauri expects sidecar binaries:
#   src-tauri/binaries/ffmpeg-x86_64-pc-windows-msvc.exe
#   src-tauri/binaries/ffprobe-x86_64-pc-windows-msvc.exe
#
# Run from the repository root:  powershell -File scripts/fetch-ffmpeg.ps1
# Idempotent: skips the download if both binaries already exist.

$ErrorActionPreference = "Stop"

$repoRoot = Split-Path -Parent $PSScriptRoot
$binDir = Join-Path $repoRoot "src-tauri\binaries"
$triple = "x86_64-pc-windows-msvc"
$ffmpegOut = Join-Path $binDir "ffmpeg-$triple.exe"
$ffprobeOut = Join-Path $binDir "ffprobe-$triple.exe"

if ((Test-Path $ffmpegOut) -and (Test-Path $ffprobeOut)) {
    Write-Host "FFmpeg sidecar binaries already present in $binDir - nothing to do."
    exit 0
}

New-Item -ItemType Directory -Force -Path $binDir | Out-Null

$url = "https://www.gyan.dev/ffmpeg/builds/ffmpeg-release-essentials.zip"
$zip = Join-Path $env:TEMP "ffmpeg-release-essentials.zip"
$extractDir = Join-Path $env:TEMP "ffmpeg-release-essentials"

Write-Host "Downloading $url ..."
Invoke-WebRequest -Uri $url -OutFile $zip

Write-Host "Extracting ..."
if (Test-Path $extractDir) { Remove-Item -Recurse -Force $extractDir }
Expand-Archive -Path $zip -DestinationPath $extractDir

$ffmpegSrc = Get-ChildItem -Path $extractDir -Recurse -Filter "ffmpeg.exe" | Select-Object -First 1
$ffprobeSrc = Get-ChildItem -Path $extractDir -Recurse -Filter "ffprobe.exe" | Select-Object -First 1
if (-not $ffmpegSrc -or -not $ffprobeSrc) {
    throw "ffmpeg.exe / ffprobe.exe not found in the downloaded archive."
}

Copy-Item $ffmpegSrc.FullName $ffmpegOut
Copy-Item $ffprobeSrc.FullName $ffprobeOut

Remove-Item $zip -Force
Remove-Item -Recurse -Force $extractDir

Write-Host "Done:"
Write-Host "  $ffmpegOut"
Write-Host "  $ffprobeOut"
