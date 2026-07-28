//! Surgical rewrite of track audio identity in `export.pdb`.
//!
//! Uses [`rekordbox_pdb::PdbEditor::update_track_audio`] so track IDs stay
//! stable (playlists, cues, and My Tags keep working).

use std::path::Path;

use rekordbox_pdb::{Database, PdbEditor};
use serde::Serialize;

use crate::error::{Error, Result};

/// One track whose path/bitrate/size should be rewritten.
#[derive(Debug, Clone)]
pub struct TrackAudioRewrite {
    pub track_id: u32,
    pub new_file_path: String,
    pub new_filename: String,
    pub bitrate: u32,
    pub file_size: u32,
    /// DeviceSQL analyze_path (`/PIONEER/USBANLZ/.../ANLZ0000.DAT`), if any.
    pub analyze_path: String,
}

/// Result of applying rewrites to an export.pdb.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PdbRewriteSummary {
    pub updated: u32,
    pub failed: Vec<String>,
}

/// Applies audio-identity rewrites to `export_pdb` in place.
pub fn rewrite_track_audio(
    export_pdb: impl AsRef<Path>,
    rewrites: &[TrackAudioRewrite],
) -> Result<PdbRewriteSummary> {
    if rewrites.is_empty() {
        return Ok(PdbRewriteSummary {
            updated: 0,
            failed: Vec::new(),
        });
    }

    let path = export_pdb.as_ref();
    let mut editor = PdbEditor::from_file(path).map_err(|e| Error::Database(e.to_string()))?;
    let mut updated = 0u32;
    let mut failed = Vec::new();

    for r in rewrites {
        match editor.update_track_audio(
            r.track_id,
            &r.new_file_path,
            &r.new_filename,
            r.bitrate,
            r.file_size,
        ) {
            Ok(()) => updated += 1,
            Err(e) => failed.push(format!("track {}: {e}", r.track_id)),
        }
    }

    editor
        .save(path)
        .map_err(|e| Error::Database(e.to_string()))?;

    Ok(PdbRewriteSummary { updated, failed })
}

/// Verifies that every rewritten track in `export_pdb` now points at an
/// existing MP3 under `usb_root`, and that track/playlist counts are unchanged
/// vs the pre-rewrite snapshot counts.
pub fn verify_rewritten_pdb(
    usb_root: impl AsRef<Path>,
    export_pdb: impl AsRef<Path>,
    expected_track_count: usize,
    expected_playlist_entry_count: usize,
    rewritten_ids: &[u32],
) -> Result<Vec<String>> {
    use crate::pdb::resolve_usb_path;

    let root = usb_root.as_ref();
    let db = Database::from_file(export_pdb.as_ref())
        .map_err(|e| Error::Database(e.to_string()))?;
    let mut problems = Vec::new();

    if db.tracks.len() != expected_track_count {
        problems.push(format!(
            "track count changed: {} → {}",
            expected_track_count,
            db.tracks.len()
        ));
    }
    if db.playlist_entries.len() != expected_playlist_entry_count {
        problems.push(format!(
            "playlist entry count changed: {} → {}",
            expected_playlist_entry_count,
            db.playlist_entries.len()
        ));
    }

    for id in rewritten_ids {
        let Some(track) = db.tracks.iter().find(|t| t.id == *id) else {
            problems.push(format!("rewritten track id {id} missing after save"));
            continue;
        };
        let path = track.file_path();
        if !path.to_ascii_lowercase().ends_with(".mp3") {
            problems.push(format!("track {id} path is not .mp3: {path}"));
        }
        let abs = resolve_usb_path(root, path);
        if !abs.is_file() {
            problems.push(format!(
                "track {id} MP3 missing on disk: {}",
                abs.display()
            ));
        } else if abs.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
            problems.push(format!("track {id} MP3 is empty: {}", abs.display()));
        }
        if track.bitrate == 0 {
            problems.push(format!("track {id} bitrate is 0"));
        }
    }

    Ok(problems)
}
