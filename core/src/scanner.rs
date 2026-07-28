//! Rekordbox export scanning.
//!
//! Validates that a drive root contains a Rekordbox device export
//! (`PIONEER/rekordbox/export.pdb`), walks the music folders and produces
//! an inventory of audio files by format (FLAC / MP3 / other) with size
//! totals used for the conversion-time estimate.

use crate::error::{Error, Result};
use crate::usb::is_rekordbox_export_root;
use serde::Serialize;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Relative paths inside a Rekordbox USB export that we care about.
pub const EXPORT_PDB: &str = "PIONEER/rekordbox/export.pdb";
pub const EXPORT_EXT_PDB: &str = "PIONEER/rekordbox/exportExt.pdb";

/// Rough seconds-per-MB for 320k FLAC→MP3 on a typical laptop CPU.
/// Tuned later once real timing data exists; good enough for UI estimates.
const SECONDS_PER_MB_FLAC: f64 = 0.35;

/// Inventory of one Rekordbox-exported drive.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportScan {
    /// Drive root, e.g. `E:\`.
    pub root: String,
    /// Absolute path to `export.pdb`.
    pub export_pdb: String,
    /// Absolute path to `exportExt.pdb`, if present.
    pub export_ext_pdb: Option<String>,
    /// Whether `exportExt.pdb` was found (My Tags live here).
    pub has_export_ext: bool,
    /// Total audio files found under Contents/ (or the whole stick if no Contents/).
    pub total_tracks: u32,
    pub flac_count: u32,
    pub mp3_count: u32,
    pub other_count: u32,
    /// Total bytes of FLAC files that would need conversion.
    pub flac_bytes: u64,
    /// Estimated wall-clock seconds for converting all FLACs (single-thread baseline).
    pub estimated_seconds: u32,
    /// Absolute paths of every FLAC on the stick (for later phases).
    pub flac_paths: Vec<String>,
}

/// Scans `root` as a Rekordbox export and returns format counts + estimate.
pub fn scan_export(root: impl AsRef<Path>) -> Result<ExportScan> {
    let root = root.as_ref();
    if !is_rekordbox_export_root(root) {
        return Err(Error::NotARekordboxExport(root.display().to_string()));
    }

    let export_pdb = root.join("PIONEER").join("rekordbox").join("export.pdb");
    let export_ext = root.join("PIONEER").join("rekordbox").join("exportExt.pdb");
    let has_export_ext = export_ext.is_file();

    // Rekordbox usually puts audio under Contents/, but older or custom
    // exports may place files elsewhere on the stick. Prefer Contents/ when
    // it exists; otherwise walk the whole drive excluding PIONEER/.
    let search_root = {
        let contents = root.join("Contents");
        if contents.is_dir() {
            contents
        } else {
            root.to_path_buf()
        }
    };

    let mut flac_count = 0u32;
    let mut mp3_count = 0u32;
    let mut other_count = 0u32;
    let mut flac_bytes = 0u64;
    let mut flac_paths = Vec::new();

    for entry in WalkDir::new(&search_root)
        .follow_links(false)
        .into_iter()
        .filter_entry(|e| {
            // Skip the Pioneer system folder when walking the whole drive.
            !(e.depth() == 1
                && e.file_name().eq_ignore_ascii_case("PIONEER")
                && search_root == root)
        })
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        match ext.to_ascii_lowercase().as_str() {
            "flac" => {
                flac_count += 1;
                if let Ok(meta) = entry.metadata() {
                    flac_bytes += meta.len();
                }
                flac_paths.push(path.display().to_string());
            }
            "mp3" => mp3_count += 1,
            "aiff" | "aif" | "wav" | "m4a" | "alac" | "aac" => other_count += 1,
            _ => {}
        }
    }

    let total_tracks = flac_count + mp3_count + other_count;
    let estimated_seconds = estimate_seconds(flac_bytes, flac_count);

    Ok(ExportScan {
        root: root.display().to_string(),
        export_pdb: export_pdb.display().to_string(),
        export_ext_pdb: has_export_ext.then(|| export_ext.display().to_string()),
        has_export_ext,
        total_tracks,
        flac_count,
        mp3_count,
        other_count,
        flac_bytes,
        estimated_seconds,
        flac_paths,
    })
}

fn estimate_seconds(flac_bytes: u64, flac_count: u32) -> u32 {
    if flac_count == 0 {
        return 0;
    }
    let mb = flac_bytes as f64 / (1024.0 * 1024.0);
    // Floor of 2s per track so tiny files don't show "0s".
    let from_size = (mb * SECONDS_PER_MB_FLAC).ceil() as u32;
    let from_count = flac_count.saturating_mul(2);
    from_size.max(from_count)
}

/// Formats an estimate like `1m 42s` or `45s`.
pub fn format_duration(seconds: u32) -> String {
    if seconds < 60 {
        format!("{seconds}s")
    } else {
        let m = seconds / 60;
        let s = seconds % 60;
        if s == 0 {
            format!("{m}m")
        } else {
            format!("{m}m {s}s")
        }
    }
}

/// Convenience: absolute path helpers for a known export root.
pub fn export_pdb_path(root: &Path) -> PathBuf {
    root.join("PIONEER").join("rekordbox").join("export.pdb")
}

pub fn export_ext_pdb_path(root: &Path) -> PathBuf {
    root.join("PIONEER").join("rekordbox").join("exportExt.pdb")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn format_duration_examples() {
        assert_eq!(format_duration(0), "0s");
        assert_eq!(format_duration(45), "45s");
        assert_eq!(format_duration(60), "1m");
        assert_eq!(format_duration(102), "1m 42s");
    }

    #[test]
    fn scan_counts_formats() {
        let dir = std::env::temp_dir().join(format!("rbusb-scan-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("PIONEER/rekordbox")).unwrap();
        fs::write(dir.join("PIONEER/rekordbox/export.pdb"), b"pdb").unwrap();
        fs::write(dir.join("PIONEER/rekordbox/exportExt.pdb"), b"ext").unwrap();
        fs::create_dir_all(dir.join("Contents/Artist")).unwrap();
        fs::write(dir.join("Contents/Artist/a.flac"), vec![0u8; 1024]).unwrap();
        fs::write(dir.join("Contents/Artist/b.flac"), vec![0u8; 2048]).unwrap();
        fs::write(dir.join("Contents/Artist/c.mp3"), b"mp3").unwrap();
        fs::write(dir.join("Contents/Artist/d.wav"), b"wav").unwrap();

        let scan = scan_export(&dir).unwrap();
        assert_eq!(scan.flac_count, 2);
        assert_eq!(scan.mp3_count, 1);
        assert_eq!(scan.other_count, 1);
        assert_eq!(scan.total_tracks, 4);
        assert!(scan.has_export_ext);
        assert_eq!(scan.flac_paths.len(), 2);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rejects_non_export() {
        let dir = std::env::temp_dir().join(format!("rbusb-bad-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let err = scan_export(&dir).unwrap_err();
        assert!(matches!(err, Error::NotARekordboxExport(_)));
        let _ = fs::remove_dir_all(&dir);
    }
}
