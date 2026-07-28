# Rekordbox USB Converter

A Windows desktop app that post-processes Rekordbox-exported USB drives:
every FLAC on the stick is converted to 320 kbps MP3 and the Pioneer
database (`export.pdb`) plus the ANLZ analysis files are rewritten so that
all playlists, hot cues, memory cues, beatgrids, waveforms, artwork and
metadata keep working on CDJ/XDJ players that cannot play FLAC. The master
Rekordbox library is never touched.

## Status

The app is being built in phases, each validated manually on real hardware
before the next begins:

- [x] Phase 0 — Scaffold (Tauri app, core crate, FFmpeg sidecar)
- [x] Phase 1 — USB detection and export scan (read-only)
- [x] Phase 2 — Pioneer database reading and round-trip proof
- [x] Phase 3 — FLAC→MP3 conversion engine and cache
- [x] Phase 4 — Database and ANLZ rewriting (on a copy)
- [ ] Phase 5 — Full transactional pipeline on the USB
- [ ] Phase 6 — Settings, tests, polish

## Architecture

- `core/` — pure Rust engine (`rbusb-core`): USB detection, export scanner,
  PDB/ANLZ read+rewrite, FFmpeg wrapper, cache, transactional pipeline.
  No UI dependencies; each module is documented in its source file.
- `src-tauri/` — Tauri 2 shell exposing the engine as commands/events, and
  bundling `ffmpeg`/`ffprobe` as sidecar binaries.
- `src/` — React + TypeScript frontend (Vite).

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
