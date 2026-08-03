//! Lightweight playlist listing for conversion scope selection.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use rekordbox_pdb::Database;
use serde::Serialize;

use crate::error::{Error, Result};
use crate::formats::{self, estimate_seconds};
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
    /// Non-MP3 audio tracks that would be converted.
    pub convertible_count: u32,
    pub mp3_count: u32,
    pub other_count: u32,
}

/// Result of resolving which non-MP3 files to convert for a scope.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertScope {
    /// `None` means all convertible files on the stick.
    pub playlist_id: Option<u32>,
    pub playlist_name: Option<String>,
    pub convertible_paths: Vec<String>,
    pub convertible_count: u32,
    pub estimated_seconds: u32,
}

/// Lists non-folder playlists with convertible/MP3 counts for the UI picker.
pub fn list_playlists(root: impl AsRef<Path>) -> Result<Vec<PlaylistOption>> {
    let root = root.as_ref();
    let db = load_db(root)?;

    let track_ext: HashMap<u32, String> = db
        .tracks
        .iter()
        .map(|t| {
            let ext = formats::extension_of(PathBuf::from(t.file_path()));
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
            let mut convertible_count = 0u32;
            let mut mp3_count = 0u32;
            let mut other_count = 0u32;
            for tid in &track_ids {
                let ext = track_ext.get(tid).map(|s| s.as_str()).unwrap_or("");
                if formats::is_mp3(ext) {
                    mp3_count += 1;
                } else if formats::is_convertible(ext) {
                    convertible_count += 1;
                } else if !ext.is_empty() {
                    other_count += 1;
                }
            }
            PlaylistOption {
                id: node.id,
                name: node.name.clone(),
                depth: depth_map.get(&node.id).copied().unwrap_or(0),
                track_count: track_ids.len() as u32,
                convertible_count,
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

/// Resolves absolute convertible paths for `playlist_id`, or all if `None`.
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

    let mut convertible_paths = Vec::new();
    let mut convertible_bytes = 0u64;

    for track in &db.tracks {
        if !track_ids.contains(&track.id) {
            continue;
        }
        let path_in_db = track.file_path();
        let ext = formats::extension_of(PathBuf::from(path_in_db));
        if !formats::is_convertible(&ext) {
            continue;
        }
        let abs = resolve_usb_path(root, path_in_db);
        if abs.is_file() {
            if let Ok(meta) = abs.metadata() {
                convertible_bytes += meta.len();
            }
            convertible_paths.push(abs.display().to_string());
        } else {
            // Still list it so the UI can show a failure later if needed.
            convertible_paths.push(abs.display().to_string());
        }
    }

    convertible_paths.sort();
    convertible_paths.dedup();

    let convertible_count = convertible_paths.len() as u32;
    let estimated_seconds = estimate_seconds(convertible_bytes, convertible_count);

    Ok(ConvertScope {
        playlist_id,
        playlist_name,
        convertible_paths,
        convertible_count,
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
