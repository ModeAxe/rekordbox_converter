//! Build a structured, UI-friendly dump of a Rekordbox USB export database.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rekordbox_pdb::{Database, ExtDatabase};
use serde::Serialize;

use crate::anlz;
use crate::error::{Error, Result};
use crate::scanner::{export_ext_pdb_path, export_pdb_path};

/// Full read-only inspection of a Rekordbox export on `root`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportInspect {
    pub root: String,
    pub summary: InspectSummary,
    pub roundtrip: super::roundtrip::RoundtripEvaluation,
    pub playlists: Vec<InspectPlaylist>,
    pub tracks: Vec<InspectTrack>,
    pub my_tags: Vec<MyTagInfo>,
    /// Plain-text dump for side-by-side comparison with Rekordbox.
    pub text_dump: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectSummary {
    pub track_count: u32,
    pub playlist_count: u32,
    pub folder_count: u32,
    pub flac_in_db: u32,
    pub mp3_in_db: u32,
    pub other_in_db: u32,
    pub anlz_files: u32,
    pub anlz_ppath_mismatches: u32,
    pub has_export_ext: bool,
    pub my_tag_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectPlaylist {
    pub id: u32,
    pub name: String,
    pub parent_id: u32,
    pub sort_order: u32,
    pub is_folder: bool,
    pub track_ids: Vec<u32>,
    pub track_count: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InspectTrack {
    pub id: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub genre: String,
    pub key: String,
    pub bpm: f64,
    pub rating: u8,
    pub color: String,
    pub comment: String,
    pub filename: String,
    pub file_path: String,
    pub analyze_path: String,
    pub anlz_audio_path: Option<String>,
    pub artwork_id: u32,
    pub bitrate: u32,
    pub file_size: u64,
    pub duration_secs: u16,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MyTagInfo {
    pub id: u32,
    pub name: String,
    pub category_id: u32,
    pub is_category: bool,
    pub position: u32,
}

/// Reads `export.pdb` (+ optional `exportExt.pdb`) and ANLZ paths from `root`.
pub fn inspect_export(root: impl AsRef<Path>) -> Result<ExportInspect> {
    let root = root.as_ref();
    let export_pdb = export_pdb_path(root);
    if !export_pdb.is_file() {
        return Err(Error::NotARekordboxExport(root.display().to_string()));
    }

    let db = Database::from_file(&export_pdb).map_err(|e| Error::Database(e.to_string()))?;
    let roundtrip = super::roundtrip::evaluate_roundtrip(&export_pdb)?;

    let export_ext = export_ext_pdb_path(root);
    let has_export_ext = export_ext.is_file();
    let my_tags = if has_export_ext {
        ExtDatabase::from_file(&export_ext)
            .map(|ext| {
                ext.tags
                    .iter()
                    .map(|t| MyTagInfo {
                        id: t.id,
                        name: t.name.clone(),
                        category_id: t.category_id,
                        is_category: t.is_category,
                        position: t.position,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let artist_map: HashMap<u32, &str> = db
        .artists
        .iter()
        .map(|a| (a.id, a.name.as_str()))
        .collect();
    let album_map: HashMap<u32, &str> = db.albums.iter().map(|a| (a.id, a.name.as_str())).collect();
    let genre_map: HashMap<u32, &str> = db.genres.iter().map(|g| (g.id, g.name.as_str())).collect();
    let key_map: HashMap<u32, &str> = db.keys.iter().map(|k| (k.id, k.name.as_str())).collect();
    let color_map: HashMap<u16, &str> = db.colors.iter().map(|c| (c.id, c.name.as_str())).collect();

    // ANLZ PPTH paths keyed by normalized analyze path.
    let anlz_index = anlz::index_anlz_paths(root);

    let mut flac_in_db = 0u32;
    let mut mp3_in_db = 0u32;
    let mut other_in_db = 0u32;
    let mut anlz_ppath_mismatches = 0u32;

    let tracks: Vec<InspectTrack> = db
        .tracks
        .iter()
        .map(|t| {
            let ext = PathBuf::from(t.file_path())
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            match ext.as_str() {
                "flac" => flac_in_db += 1,
                "mp3" => mp3_in_db += 1,
                _ => other_in_db += 1,
            }

            let analyze_path = t.analyze_path().to_string();
            let anlz_audio_path = anlz_index.get(&normalize_path(&analyze_path)).cloned();

            if let Some(ref anlz_path) = anlz_audio_path {
                if !paths_equivalent(anlz_path, t.file_path()) {
                    anlz_ppath_mismatches += 1;
                }
            }

            InspectTrack {
                id: t.id,
                title: t.title().to_string(),
                artist: artist_map
                    .get(&t.artist_id)
                    .copied()
                    .unwrap_or("")
                    .to_string(),
                album: album_map.get(&t.album_id).copied().unwrap_or("").to_string(),
                genre: genre_map.get(&t.genre_id).copied().unwrap_or("").to_string(),
                key: key_map.get(&t.key_id).copied().unwrap_or("").to_string(),
                bpm: t.tempo as f64 / 100.0,
                rating: t.rating,
                color: color_map
                    .get(&(t.color_id as u16))
                    .copied()
                    .unwrap_or("")
                    .to_string(),
                comment: t.comment().to_string(),
                filename: t.filename().to_string(),
                file_path: t.file_path().to_string(),
                analyze_path,
                anlz_audio_path,
                artwork_id: t.artwork_id,
                bitrate: t.bitrate,
                file_size: t.file_size as u64,
                duration_secs: t.duration,
            }
        })
        .collect();

    let mut entries_by_playlist: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for e in &db.playlist_entries {
        entries_by_playlist
            .entry(e.playlist_id)
            .or_default()
            .push((e.entry_index, e.track_id));
    }
    for v in entries_by_playlist.values_mut() {
        v.sort_by_key(|(idx, _)| *idx);
    }

    let mut playlists: Vec<InspectPlaylist> = db
        .playlist_tree
        .iter()
        .map(|node| {
            let track_ids = if node.is_folder {
                Vec::new()
            } else {
                entries_by_playlist
                    .get(&node.id)
                    .map(|entries| entries.iter().map(|(_, tid)| *tid).collect())
                    .unwrap_or_default()
            };
            InspectPlaylist {
                id: node.id,
                name: node.name.clone(),
                parent_id: node.parent_id,
                sort_order: node.sort_order,
                is_folder: node.is_folder,
                track_count: track_ids.len() as u32,
                track_ids,
            }
        })
        .collect();

    playlists.sort_by(|a, b| {
        a.parent_id
            .cmp(&b.parent_id)
            .then(a.sort_order.cmp(&b.sort_order))
            .then(a.name.cmp(&b.name))
    });

    let folder_count = playlists.iter().filter(|p| p.is_folder).count() as u32;
    let playlist_count = playlists.iter().filter(|p| !p.is_folder).count() as u32;

    let summary = InspectSummary {
        track_count: tracks.len() as u32,
        playlist_count,
        folder_count,
        flac_in_db,
        mp3_in_db,
        other_in_db,
        anlz_files: anlz_index.len() as u32,
        anlz_ppath_mismatches,
        has_export_ext,
        my_tag_count: my_tags.len() as u32,
    };

    let text_dump = build_text_dump(&summary, &roundtrip, &playlists, &tracks, &my_tags, &db);

    Ok(ExportInspect {
        root: root.display().to_string(),
        summary,
        roundtrip,
        playlists,
        tracks,
        my_tags,
        text_dump,
    })
}

fn build_text_dump(
    summary: &InspectSummary,
    roundtrip: &super::roundtrip::RoundtripEvaluation,
    playlists: &[InspectPlaylist],
    tracks: &[InspectTrack],
    my_tags: &[MyTagInfo],
    db: &Database,
) -> String {
    let mut out = String::new();
    use std::fmt::Write;

    let _ = writeln!(out, "=== Rekordbox export inspect ===");
    let _ = writeln!(out, "Tracks: {}", summary.track_count);
    let _ = writeln!(
        out,
        "Playlists: {} (+ {} folders)",
        summary.playlist_count, summary.folder_count
    );
    let _ = writeln!(
        out,
        "Formats in DB: {} FLAC, {} MP3, {} other",
        summary.flac_in_db, summary.mp3_in_db, summary.other_in_db
    );
    let _ = writeln!(out, "ANLZ files indexed: {}", summary.anlz_files);
    let _ = writeln!(
        out,
        "Round-trip (rekordbox-pdb): {} — {}",
        if roundtrip.rekordbox_pdb.byte_identical {
            "PASS"
        } else {
            "FAIL"
        },
        roundtrip.rekordbox_pdb.message
    );
    let _ = writeln!(
        out,
        "Round-trip (rekordcrate): {} — {}",
        if roundtrip.rekordcrate.parsed {
            "parsed"
        } else {
            "failed"
        },
        roundtrip.rekordcrate.message
    );
    let _ = writeln!(out);

    let _ = writeln!(out, "=== Playlist tree ===");
    dump_playlist_tree(&mut out, playlists, 0, 0);
    let _ = writeln!(out);

    if !my_tags.is_empty() {
        let _ = writeln!(out, "=== My Tags (exportExt.pdb) ===");
        for tag in my_tags {
            let kind = if tag.is_category { "category" } else { "tag" };
            let _ = writeln!(
                out,
                "[{kind}] id={} cat={} pos={} {}",
                tag.id, tag.category_id, tag.position, tag.name
            );
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "=== Playlists (with track order) ===");
    for pl in playlists.iter().filter(|p| !p.is_folder) {
        let _ = writeln!(out, "[{}] {} ({} tracks)", pl.id, pl.name, pl.track_count);
        for (i, tid) in pl.track_ids.iter().enumerate() {
            if let Some(t) = tracks.iter().find(|t| t.id == *tid) {
                let _ = writeln!(
                    out,
                    "  {:>3}. {} — {} | {:.2} BPM | {} | rating {} | {}",
                    i + 1,
                    t.artist,
                    t.title,
                    t.bpm,
                    t.key,
                    t.rating,
                    t.file_path
                );
            } else {
                let _ = writeln!(out, "  {:>3}. (missing track id={tid})", i + 1);
            }
        }
        let _ = writeln!(out);
    }

    let _ = writeln!(out, "=== All tracks ===");
    for t in tracks {
        let _ = writeln!(
            out,
            "id={} | {} — {} | {} | {:.2} BPM | key {} | rating {} | color {} | art={} | {}",
            t.id,
            t.artist,
            t.title,
            extension_of(&t.file_path),
            t.bpm,
            t.key,
            t.rating,
            t.color,
            t.artwork_id,
            t.file_path
        );
        if !t.comment.is_empty() {
            let _ = writeln!(out, "       comment: {}", t.comment);
        }
        if let Some(ref anlz) = t.anlz_audio_path {
            if !paths_equivalent(anlz, &t.file_path) {
                let _ = writeln!(out, "       ANLZ PPTH: {anlz} (MISMATCH)");
            }
        }
    }

    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "=== Reference tables === artists={} albums={} genres={} keys={} colors={} artwork={}",
        db.artists.len(),
        db.albums.len(),
        db.genres.len(),
        db.keys.len(),
        db.colors.len(),
        db.artwork.len()
    );

    out
}

fn dump_playlist_tree(
    out: &mut String,
    playlists: &[InspectPlaylist],
    parent_id: u32,
    depth: usize,
) {
    use std::fmt::Write;
    let indent = "  ".repeat(depth);
    for node in playlists
        .iter()
        .filter(|p| p.parent_id == parent_id)
        .filter(|p| p.is_folder || p.track_count > 0 || depth == 0)
    {
        let kind = if node.is_folder { "folder" } else { "playlist" };
        let _ = writeln!(
            out,
            "{indent}[{kind}] {} (id={}, order={})",
            node.name, node.id, node.sort_order
        );
        if node.is_folder {
            dump_playlist_tree(out, playlists, node.id, depth + 1);
        }
    }
}

fn extension_of(path: &str) -> String {
    std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("?")
        .to_string()
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").trim_start_matches('/').to_ascii_lowercase()
}

fn paths_equivalent(a: &str, b: &str) -> bool {
    normalize_path(a) == normalize_path(b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_equivalent_ignores_slashes_and_case() {
        assert!(paths_equivalent(
            "/Contents/A/track.flac",
            "Contents\\A\\track.FLAC"
        ));
    }
}
