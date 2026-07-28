//! Tauri shell for the Rekordbox USB converter.
//!
//! This crate is a thin adapter: every command delegates to `rbusb-core`
//! or to the bundled FFmpeg sidecar. No business logic lives here.

mod sidecar;

use rbusb_core::pdb::{inspect_export, ExportInspect};
use rbusb_core::scanner::{self, ExportScan};
use rbusb_core::usb::{self, DriveInfo};

#[tauri::command]
fn ffmpeg_version() -> Result<String, String> {
    sidecar::ffmpeg_version_line().map_err(|e| e.to_string())
}

#[tauri::command]
fn list_drives() -> Vec<DriveInfo> {
    usb::list_candidate_drives()
}

#[tauri::command]
fn scan_drive(root: String) -> Result<ExportScan, String> {
    scanner::scan_export(root).map_err(|e| e.to_string())
}

#[tauri::command]
fn format_estimate(seconds: u32) -> String {
    scanner::format_duration(seconds)
}

/// Reads export.pdb, exportExt.pdb and ANLZ PPTH tags; returns structured inspect data.
#[tauri::command]
fn inspect_drive(root: String) -> Result<ExportInspect, String> {
    inspect_export(root).map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            ffmpeg_version,
            list_drives,
            scan_drive,
            format_estimate,
            inspect_drive
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
