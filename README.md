# DrokerBox

**DrokerBox** (“Droker” = Rekord backwards) is a Windows desktop app that
post-processes Rekordbox-exported USB drives: non-MP3 audio on the stick
(FLAC, WAV, AIFF, M4A, …) is converted to 320 kbps MP3 and the Pioneer
database (`export.pdb`) plus the ANLZ analysis files are rewritten so that
playlists, hot cues, memory cues, beatgrids, waveforms, artwork and metadata
keep working on CDJ/XDJ players that cannot play those formats. Existing MP3s
are left alone. The master Rekordbox library is never touched.

**Unofficial tool — not affiliated with AlphaTheta, Pioneer DJ, or Rekordbox.**

Current version: **0.1.0**

## Architecture

- `core/` — pure Rust engine (`rbusb-core`): USB detection, export scanner,
  PDB/ANLZ read+rewrite, FFmpeg wrapper, cache, settings, transactional pipeline.
- `src-tauri/` — Tauri 2 shell (commands/events) + bundled `ffmpeg`/`ffprobe`.
- `src/` — React + TypeScript UI styled with [98.css](https://jdan.github.io/98.css/).

## Settings & data

Stored under `%LOCALAPPDATA%\DrokerBox\`:

| Path | Purpose |
|------|---------|
| `settings.json` | User preferences |
| `Cache\` | Converted MP3 cache |
| `Staged\` | Safe converted copies (USB untouched) |

| Setting | Default | Notes |
|---------|---------|--------|
| MP3 bitrate | `320k` | Also `192k` / `256k` |
| FFmpeg threads | Auto | 1–8 |
| Cache location | `%LOCALAPPDATA%\DrokerBox\Cache` | Empty = default |
| Keep source files | off | Skip deletion after in-place convert |
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

The FFmpeg binaries land in `src-tauri/binaries/` (gitignored). The app shows
the detected FFmpeg version at startup as a smoke test.

## Building a release bundle

FFmpeg sidecars must exist before the release build (they are not in git):

```powershell
npm install
powershell -File scripts/fetch-ffmpeg.ps1
npm run tauri build
```

Installers land under `src-tauri/target/release/bundle/` (NSIS / MSI depending
on Tauri targets). Always smoke-test the **built** installer, not only `tauri dev`.

Version is kept in sync across:

- [`Cargo.toml`](Cargo.toml) workspace `version`
- [`package.json`](package.json) `version`
- [`src-tauri/tauri.conf.json`](src-tauri/tauri.conf.json) `version`

## Recommended usage

1. Export a USB from Rekordbox as usual.
2. Prefer **Staged** (converted copy on disk) and test in Rekordbox / on a CDJ.
3. Then use **Convert USB** on the real stick (backup + rollback built in).
4. Eject safely before plugging into a player.

## License

DrokerBox is MIT — see [LICENSE](LICENSE).

Bundled FFmpeg comes from the [gyan.dev Windows essentials builds](https://www.gyan.dev/ffmpeg/builds/).
FFmpeg is subject to its own LGPL/GPL licenses; include those notices when you
redistribute a release that contains the sidecars.

Vendored [`rekordbox-pdb`](vendor/rekordbox-pdb/) is MIT.

## Format references

- Pioneer DeviceSQL export format: <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html>
- ANLZ analysis files: <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>
- Candidate parser libraries: [rekordcrate](https://github.com/Holzhaus/rekordcrate),
  [rekordbox-pdb](https://crates.io/crates/rekordbox-pdb) (vendored with patches in `vendor/rekordbox-pdb/`)
