//! Tauri shell for the Rekordbox USB converter.
//!
//! This crate is a thin adapter: every command delegates to `rbusb-core`
//! or to the bundled FFmpeg sidecar. No business logic lives here.

mod sidecar;

/// Returns the version line of the bundled FFmpeg sidecar, e.g.
/// `ffmpeg version 7.1-essentials_build-www.gyan.dev ...`.
///
/// Used by the UI at startup to prove the sidecar is bundled and runnable.
#[tauri::command]
fn ffmpeg_version() -> Result<String, String> {
    sidecar::ffmpeg_version_line().map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![ffmpeg_version])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
