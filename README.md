# DrokerBox USB Converter

A Windows desktop app that post-processes Rekordbox-exported USB drives:
every FLAC on the stick is converted to 320 kbps MP3 and the Pioneer
database (`export.pdb`) plus the ANLZ analysis files are rewritten so that
all playlists, hot cues, memory cues, beatgrids, waveforms, artwork and
metadata keep working on CDJ/XDJ players that cannot play FLAC.

## Architecture

- `core/` — pure Rust engine (`rbusb-core`): USB detection, export scanner,
  PDB/ANLZ read+rewrite, FFmpeg wrapper, cache, settings, transactional pipeline.
  No UI dependencies; each module is documented in its source file.
- `src-tauri/` — Tauri 2 shell exposing the engine as commands/events, and
  bundling `ffmpeg`/`ffprobe` as sidecar binaries.
- `src/` — React + TypeScript frontend (Vite) styled with [98.css](https://jdan.github.io/98.css/).

## Settings

Stored at `%LOCALAPPDATA%\DrokerboxUsbConverter\settings.json`:

| Setting | Default | Notes |
|---------|---------|--------|
| MP3 bitrate | `320k` | Also `192k` / `256k` |
| FFmpeg threads | Auto | 1–8 |
| Cache location | `%LOCALAPPDATA%\…\Cache` | Empty = default |
| Keep FLAC on USB | off | Skip deletion after in-place convert |
| Verify output | on | Rollback if post-rewrite checks fail |
| Dry run | off | Plan only — no USB writes |

## Prerequisites (Windows)

- [Node.js](https://nodejs.org/) 18+
- [Rust](https://rustup.rs/) (stable, MSVC toolchain)
- Visual Studio C++ Build Tools
- WebView2 runtime (preinstalled on Windows 11)

## Getting started

```powershell
npm install
powershell -File scripts/fetch-ffmpeg.ps1   # downloads ffmpeg/ffprobe sidecars (~200 MB, one-time)
npm run tauri dev
```

The FFmpeg binaries land in `src-tauri/binaries/` (gitignored). The dev
window shows the detected FFmpeg version at startup as a smoke test.

## Building a release bundle

```powershell
npm run tauri build
```

## Format references

- Pioneer DeviceSQL export format: <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html>
- ANLZ analysis files: <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>
- Candidate parser libraries: [rekordcrate](https://github.com/Holzhaus/rekordcrate),
  [rekordbox-pdb](https://crates.io/crates/rekordbox-pdb) (vendored with patches in `vendor/rekordbox-pdb/`)
