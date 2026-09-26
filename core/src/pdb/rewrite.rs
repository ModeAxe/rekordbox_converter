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
    pub old_file_path: String,
    pub new_file_path: String,
    pub new_filename: String,
    pub bitrate: u32,
    pub file_size: u32,
    /// Previous DeviceSQL analyze_path, used to find the ANLZ directory.
    pub old_analyze_path: String,
    /// Hash path of `new_file_path` (`/PIONEER/USBANLZ/.../ANLZ0000.DAT`).
    pub new_analyze_path: String,
    /// From ffprobe of the MP3, when available.
    pub sample_rate: Option<u32>,
    pub sample_depth: Option<u16>,
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
            &r.new_analyze_path,
            r.sample_rate,
            r.sample_depth,
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

        let expected_anlz = crate::anlz::analyze_db_path(path);
        let actual_anlz = track.analyze_path();
        if !actual_anlz.is_empty() {
            if !crate::anlz::same_export_path(actual_anlz, &expected_anlz) {
                problems.push(format!(
                    "track {id} analyze_path {actual_anlz} is not the audio-path hash {expected_anlz}"
                ));
            } else {
                let anlz_abs = resolve_usb_path(root, actual_anlz);
                if !anlz_abs.is_file() {
                    problems.push(format!(
                        "track {id} ANLZ missing: {}",
                        anlz_abs.display()
                    ));
                } else {
                    match crate::anlz::read_ppath(&anlz_abs) {
                        Ok(Some(ppth)) if crate::anlz::same_export_path(&ppth, path) => {}
                        Ok(Some(ppth)) => problems.push(format!(
                            "track {id} ANLZ PPTH {ppth} does not match {path}"
                        )),
                        Ok(None) => problems.push(format!("track {id} ANLZ has no PPTH tag")),
                        Err(e) => problems.push(format!("track {id} ANLZ unreadable: {e}")),
                    }
                }
            }
        }
    }

    problems.extend(crate::export_library::verify_against_tracks(
        root,
        &db,
        rewritten_ids,
    ));

    Ok(problems)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verify_flags_anlz_not_at_audio_hash() {
        let root = std::env::temp_dir().join(format!(
            "drokerbox-verify-{}",
            std::process::id()
        ));
        let pdb_dir = root.join("PIONEER").join("rekordbox");
        std::fs::create_dir_all(&pdb_dir).unwrap();
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../vendor/rekordbox-pdb/tests/data/one-song-export.pdb");
        let pdb = pdb_dir.join("export.pdb");
        std::fs::copy(&src, &pdb).unwrap();

        let db = Database::from_file(&pdb).unwrap();
        let problems = verify_rewritten_pdb(
            &root,
            &pdb,
            db.tracks.len(),
            db.playlist_entries.len(),
            &[db.tracks[0].id],
        )
        .unwrap();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("is not the audio-path hash")),
            "{problems:?}"
        );
        let _ = std::fs::remove_dir_all(&root);
    }
}
