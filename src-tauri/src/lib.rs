//! Tauri shell for the DrokerBox USB converter.
//!
//! This crate is a thin adapter: every command delegates to `rbusb-core`
//! or to the bundled FFmpeg sidecar. No business logic lives here.

mod sidecar;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rbusb_core::convert::{self, ConvertOptions, ConvertProgress, ConvertSummary};
use rbusb_core::pdb::{
    inspect_export, list_playlists, resolve_convert_scope, ConvertScope, ExportInspect,
    PlaylistOption,
};
use rbusb_core::pipeline::{
    self, InPlaceOptions, InPlaceSummary, PipelineProgress, StagedCopyOptions, StagedCopySummary,
};
use rbusb_core::scanner::{self, ExportScan};
use rbusb_core::settings::{self, AppSettings};
use rbusb_core::usb::{self, DriveInfo};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

struct AppState {
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

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

#[tauri::command]
fn inspect_drive(root: String) -> Result<ExportInspect, String> {
    inspect_export(root).map_err(|e| e.to_string())
}

#[tauri::command]
fn list_export_playlists(root: String) -> Result<Vec<PlaylistOption>, String> {
    list_playlists(root).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_convert_scope(root: String, playlist_id: Option<u32>) -> Result<ConvertScope, String> {
    resolve_convert_scope(root, playlist_id).map_err(|e| e.to_string())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct CacheInfo {
    root: String,
}

#[tauri::command]
fn get_cache_root() -> CacheInfo {
    let s = settings::load();
    CacheInfo {
        root: s.resolved_cache_root().display().to_string(),
    }
}

#[tauri::command]
fn get_settings() -> AppSettings {
    settings::load()
}

#[tauri::command]
fn save_settings(settings: AppSettings) -> Result<AppSettings, String> {
    settings::save(&settings).map_err(|e| e.to_string())?;
    Ok(settings)
}

/// Convert FLACs into the local cache. Pass `playlist_id` to limit scope;
/// `None` converts every FLAC on the stick. USB is not modified.
#[tauri::command]
fn convert_to_cache(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    playlist_id: Option<u32>,
) -> Result<ConvertSummary, String> {
    let cfg = settings::load();
    let scope = resolve_convert_scope(&root, playlist_id).map_err(|e| e.to_string())?;
    if scope.convertible_paths.is_empty() {
        return Ok(ConvertSummary {
            total: 0,
            cache_hits: 0,
            converted: 0,
            failed: 0,
            cache_root: cfg.resolved_cache_root().display().to_string(),
            errors: Vec::new(),
        });
    }

    let ffmpeg = sidecar::sidecar_path("ffmpeg").map_err(|e| e.to_string())?;
    if !ffmpeg.is_file() {
        return Err(format!("ffmpeg sidecar not found at {}", ffmpeg.display()));
    }

    let contents = {
        let c = PathBuf::from(&root).join("Contents");
        if c.is_dir() {
            Some(c)
        } else {
            None
        }
    };

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.cancel.lock().map_err(|e| e.to_string())?;
        *slot = Some(Arc::clone(&cancel));
    }

    let options = ConvertOptions {
        ffmpeg_path: ffmpeg,
        cache_root: cfg.resolved_cache_root(),
        bitrate: cfg.bitrate.clone(),
        workers: cfg.resolved_workers(),
        contents_root: contents,
    };

    let sources: Vec<PathBuf> = scope.convertible_paths.iter().map(PathBuf::from).collect();
    let app_for_progress = app.clone();

    let summary = convert::convert_to_cache(
        &sources,
        &options,
        Some(Arc::clone(&cancel)),
        move |progress: ConvertProgress| {
            let _ = app_for_progress.emit("conversion-progress", &progress);
        },
    )
    .map_err(|e| e.to_string())?;

    {
        let mut slot = state.cancel.lock().map_err(|e| e.to_string())?;
        *slot = None;
    }

    let _ = app.emit("conversion-finished", &summary);
    Ok(summary)
}

#[tauri::command]
fn cancel_conversion(state: State<'_, AppState>) -> Result<(), String> {
    let slot = state.cancel.lock().map_err(|e| e.to_string())?;
    if let Some(flag) = slot.as_ref() {
        flag.store(true, Ordering::Relaxed);
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct StagedParentInfo {
    root: String,
}

#[tauri::command]
fn get_staged_parent() -> StagedParentInfo {
    StagedParentInfo {
        root: pipeline::default_staged_parent().display().to_string(),
    }
}

/// Phase 4: build a converted copy under a new folder. Source USB is not modified.
#[tauri::command]
fn build_staged_copy(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    playlist_id: Option<u32>,
    output_name: Option<String>,
) -> Result<StagedCopySummary, String> {
    let cfg = settings::load();
    let ffmpeg = sidecar::sidecar_path("ffmpeg").map_err(|e| e.to_string())?;
    if !ffmpeg.is_file() {
        return Err(format!("ffmpeg sidecar not found at {}", ffmpeg.display()));
    }

    let parent = pipeline::default_staged_parent();
    std::fs::create_dir_all(&parent).map_err(|e| e.to_string())?;
    let name = output_name.unwrap_or_else(|| {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        format!("converted-{stamp}")
    });
    let output_root = parent.join(name);

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.cancel.lock().map_err(|e| e.to_string())?;
        *slot = Some(Arc::clone(&cancel));
    }

    let options = StagedCopyOptions {
        source_root: PathBuf::from(&root),
        output_root,
        playlist_id,
        ffmpeg_path: ffmpeg,
        cache_root: cfg.resolved_cache_root(),
        bitrate: cfg.bitrate.clone(),
        workers: cfg.resolved_workers(),
    };

    let app_progress = app.clone();
    let summary = pipeline::build_staged_copy(
        &options,
        Some(Arc::clone(&cancel)),
        move |progress: PipelineProgress| {
            let _ = app_progress.emit("pipeline-progress", &progress);
        },
    )
    .map_err(|e| e.to_string())?;

    {
        let mut slot = state.cancel.lock().map_err(|e| e.to_string())?;
        *slot = None;
    }

    let _ = app.emit("pipeline-finished", &summary);
    Ok(summary)
}

/// Convert scoped FLACs on the live USB (backup → MP3 → rewrite → verify →
/// delete FLACs). Rolls back the database/ANLZ on failure before FLAC deletion.
#[tauri::command]
fn convert_usb_inplace(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    playlist_id: Option<u32>,
) -> Result<InPlaceSummary, String> {
    let cfg = settings::load();
    let ffmpeg = sidecar::sidecar_path("ffmpeg").map_err(|e| e.to_string())?;
    if !ffmpeg.is_file() {
        return Err(format!("ffmpeg sidecar not found at {}", ffmpeg.display()));
    }

    let cancel = Arc::new(AtomicBool::new(false));
    {
        let mut slot = state.cancel.lock().map_err(|e| e.to_string())?;
        *slot = Some(Arc::clone(&cancel));
    }

    let options = InPlaceOptions {
        usb_root: PathBuf::from(&root),
        playlist_id,
        ffmpeg_path: ffmpeg,
        cache_root: cfg.resolved_cache_root(),
        bitrate: cfg.bitrate.clone(),
        workers: cfg.resolved_workers(),
        keep_source: cfg.keep_source,
        verify_output: cfg.verify_output,
        dry_run: cfg.dry_run,
    };

    let app_progress = app.clone();
    let summary = pipeline::convert_usb_inplace(
        &options,
        Some(Arc::clone(&cancel)),
        move |progress: PipelineProgress| {
            let _ = app_progress.emit("inplace-progress", &progress);
        },
    )
    .map_err(|e| e.to_string())?;

    {
        let mut slot = state.cancel.lock().map_err(|e| e.to_string())?;
        *slot = None;
    }

    let _ = app.emit("inplace-finished", &summary);
    Ok(summary)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(AppState {
            cancel: Mutex::new(None),
        })
        .invoke_handler(tauri::generate_handler![
            ffmpeg_version,
            list_drives,
            scan_drive,
            format_estimate,
            inspect_drive,
            list_export_playlists,
            get_convert_scope,
            get_cache_root,
            get_settings,
            save_settings,
            convert_to_cache,
            cancel_conversion,
            get_staged_parent,
            build_staged_copy,
            convert_usb_inplace
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
