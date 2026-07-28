//! Lightweight playlist listing for conversion scope selection.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rekordbox_pdb::Database;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::scanner::export_pdb_path;

/// A convertible playlist (folders are excluded).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaylistOption {
    pub id: u32,
    pub name: String,
    /// Indentation hint from the playlist tree (0 = root).
    pub depth: u32,
    pub track_count: u32,
    pub flac_count: u32,
    pub mp3_count: u32,
    pub other_count: u32,
}

/// Result of resolving which FLAC files to convert for a scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertScope {
    /// `None` means all FLACs on the stick.
    pub playlist_id: Option<u32>,
    pub playlist_name: Option<String>,
    pub flac_paths: Vec<String>,
    pub flac_count: u32,
    pub estimated_seconds: u32,
}

/// Lists non-folder playlists with FLAC/MP3 counts for the UI picker.
pub fn list_playlists(root: impl AsRef<Path>) -> Result<Vec<PlaylistOption>> {
    let root = root.as_ref();
    let db = load_db(root)?;

    let track_ext: HashMap<u32, String> = db
        .tracks
        .iter()
        .map(|t| {
            let ext = PathBuf::from(t.file_path())
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            (t.id, ext)
        })
        .collect();

    let mut entries_by_playlist: HashMap<u32, Vec<u32>> = HashMap::new();
    for e in &db.playlist_entries {
        entries_by_playlist
            .entry(e.playlist_id)
            .or_default()
            .push(e.track_id);
    }

    let depth_map = playlist_depths(&db);

    let mut options: Vec<PlaylistOption> = db
        .playlist_tree
        .iter()
        .filter(|n| !n.is_folder)
        .map(|node| {
            let track_ids = entries_by_playlist
                .get(&node.id)
                .cloned()
                .unwrap_or_default();
            let mut flac_count = 0u32;
            let mut mp3_count = 0u32;
            let mut other_count = 0u32;
            for tid in &track_ids {
                match track_ext.get(tid).map(|s| s.as_str()).unwrap_or("") {
                    "flac" => flac_count += 1,
                    "mp3" => mp3_count += 1,
                    _ => other_count += 1,
                }
            }
            PlaylistOption {
                id: node.id,
                name: node.name.clone(),
                depth: depth_map.get(&node.id).copied().unwrap_or(0),
                track_count: track_ids.len() as u32,
                flac_count,
                mp3_count,
                other_count,
            }
        })
        .collect();

    options.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then(a.name.to_ascii_lowercase().cmp(&b.name.to_ascii_lowercase()))
    });

    Ok(options)
}

/// Resolves absolute FLAC paths for `playlist_id`, or all FLACs if `None`.
pub fn resolve_convert_scope(
    root: impl AsRef<Path>,
    playlist_id: Option<u32>,
) -> Result<ConvertScope> {
    let root = root.as_ref();
    let db = load_db(root)?;

    let (playlist_name, track_ids): (Option<String>, HashSet<u32>) = match playlist_id {
        None => (None, db.tracks.iter().map(|t| t.id).collect()),
        Some(pid) => {
            let node = db
                .playlist_tree
                .iter()
                .find(|n| n.id == pid && !n.is_folder)
                .ok_or_else(|| Error::Message(format!("playlist id {pid} not found")))?;
            let ids: HashSet<u32> = db
                .playlist_entries
                .iter()
                .filter(|e| e.playlist_id == pid)
                .map(|e| e.track_id)
                .collect();
            (Some(node.name.clone()), ids)
        }
    };

    let mut flac_paths = Vec::new();
    let mut flac_bytes = 0u64;

    for track in &db.tracks {
        if !track_ids.contains(&track.id) {
            continue;
        }
        let path_in_db = track.file_path();
        let ext = PathBuf::from(path_in_db)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if ext != "flac" {
            continue;
        }
        let abs = resolve_usb_path(root, path_in_db);
        if abs.is_file() {
            if let Ok(meta) = abs.metadata() {
                flac_bytes += meta.len();
            }
            flac_paths.push(abs.display().to_string());
        } else {
            // Still list it so the UI can show a failure later if needed.
            flac_paths.push(abs.display().to_string());
        }
    }

    flac_paths.sort();
    flac_paths.dedup();

    let flac_count = flac_paths.len() as u32;
    let estimated_seconds = estimate_seconds(flac_bytes, flac_count);

    Ok(ConvertScope {
        playlist_id,
        playlist_name,
        flac_paths,
        flac_count,
        estimated_seconds,
    })
}

fn load_db(root: &Path) -> Result<Database> {
    let export_pdb = export_pdb_path(root);
    if !export_pdb.is_file() {
        return Err(Error::NotARekordboxExport(root.display().to_string()));
    }
    Database::from_file(&export_pdb).map_err(|e| Error::Database(e.to_string()))
}

/// Maps a DeviceSQL path like `/Contents/A/B.flac` onto the USB root.
pub fn resolve_usb_path(usb_root: &Path, db_path: &str) -> PathBuf {
    let trimmed = db_path
        .trim()
        .trim_start_matches('/')
        .trim_start_matches('\\')
        .replace('\\', "/");
    let mut out = usb_root.to_path_buf();
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." {
            continue;
        }
        out.push(part);
    }
    out
}

fn playlist_depths(db: &Database) -> HashMap<u32, u32> {
    let by_id: HashMap<u32, &rekordbox_pdb::PlaylistTreeNode> =
        db.playlist_tree.iter().map(|n| (n.id, n)).collect();
    let mut depths = HashMap::new();
    for node in &db.playlist_tree {
        let mut depth = 0u32;
        let mut parent = node.parent_id;
        let mut guard = 0u32;
        while parent != 0 && guard < 64 {
            depth += 1;
            parent = by_id.get(&parent).map(|n| n.parent_id).unwrap_or(0);
            guard += 1;
        }
        depths.insert(node.id, depth);
    }
    depths
}

fn estimate_seconds(flac_bytes: u64, flac_count: u32) -> u32 {
    if flac_count == 0 {
        return 0;
    }
    const SECONDS_PER_MB_FLAC: f64 = 0.35;
    let mb = flac_bytes as f64 / (1024.0 * 1024.0);
    let from_size = (mb * SECONDS_PER_MB_FLAC).ceil() as u32;
    let from_count = flac_count.saturating_mul(2);
    from_size.max(from_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_usb_path_strips_leading_slash() {
        let root = PathBuf::from("E:/");
        let p = resolve_usb_path(&root, "/Contents/Artist/Track.flac");
        assert!(p.to_string_lossy().contains("Contents"));
        assert!(p.to_string_lossy().ends_with("Track.flac"));
    }
}
