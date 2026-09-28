//! Device Library Plus (`PIONEER/rekordbox/exportLibrary.db`).
//!
//! Newer players (XDJ-AZ, OPUS-QUAD, CDJ-3000X, …) read this SQLCipher
//! database instead of trusting only `export.pdb`. After a format change the
//! `content` row must point at the new audio path and the hashed ANLZ path.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::error::{Error, Result};
use crate::sqlite_mc::{Db, Step};

/// Public Device Library Plus key (SQLCipher 4 defaults). Not a per-user secret.
const EXPORT_LIBRARY_KEY: &str = "r8gddnr4k847830ar6cqzbkk0el6qytmb3trbbx805jm74vez64i5o8fnrqryqls";

/// One converted track to patch in `content`.
#[derive(Debug, Clone)]
pub struct LibraryTrackUpdate {
    pub old_path: String,
    pub new_path: String,
    pub file_name: String,
    pub file_size: u32,
    pub bitrate: u32,
    pub sample_rate: Option<u32>,
    pub sample_depth: Option<u16>,
    pub analyze_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LibraryUpdateSummary {
    pub updated: u32,
    pub missing: Vec<String>,
}

pub fn export_library_db_path(root: &Path) -> PathBuf {
    root.join("PIONEER")
        .join("rekordbox")
        .join("exportLibrary.db")
}

/// Updates matching `content` rows. No-op when the database is absent.
pub fn update_tracks(
    usb_root: &Path,
    updates: &[LibraryTrackUpdate],
) -> Result<LibraryUpdateSummary> {
    let path = export_library_db_path(usb_root);
    if !path.is_file() || updates.is_empty() {
        return Ok(LibraryUpdateSummary {
            updated: 0,
            missing: Vec::new(),
        });
    }

    let conn = open_export_library(&path)?;
    let stmt = conn.prepare(
        "UPDATE content SET
            path = ?2,
            fileName = ?3,
            fileSize = ?4,
            fileType = 1,
            bitrate = ?5,
            samplingRate = COALESCE(?6, samplingRate),
            bitDepth = COALESCE(?7, bitDepth),
            analysisDataFilePath = ?8
         WHERE path = ?1",
    )?;
    let mut updated = 0u32;
    let mut missing = Vec::new();
    for row in updates {
        stmt.reset()?;
        stmt.bind_text(1, &row.old_path)?;
        stmt.bind_text(2, &row.new_path)?;
        stmt.bind_text(3, &row.file_name)?;
        stmt.bind_i64(4, row.file_size as i64)?;
        stmt.bind_i64(5, row.bitrate as i64)?;
        bind_opt_i64(&stmt, 6, row.sample_rate.map(|v| v as i64))?;
        bind_opt_i64(&stmt, 7, row.sample_depth.map(|v| v as i64))?;
        stmt.bind_text(8, &row.analyze_path)?;
        match stmt.step()? {
            Step::Done => {}
            Step::Row => {}
        }
        let changed = conn.changes();
        if changed == 0 {
            missing.push(row.old_path.clone());
        } else {
            updated += changed;
        }
    }
    conn.exec("PRAGMA wal_checkpoint(TRUNCATE);")?;
    Ok(LibraryUpdateSummary { updated, missing })
}

/// Confirms rewritten tracks in `exportLibrary.db` match `export.pdb`.
///
/// Returns an empty list when the database is not on the export.
pub fn verify_against_tracks(
    usb_root: &Path,
    db: &rekordbox_pdb::Database,
    rewritten_ids: &[u32],
) -> Vec<String> {
    let path = export_library_db_path(usb_root);
    if !path.is_file() {
        return Vec::new();
    }
    let conn = match open_export_library(&path) {
        Ok(conn) => conn,
        Err(e) => return vec![format!("exportLibrary.db: {e}")],
    };

    let mut problems = Vec::new();
    for id in rewritten_ids {
        let Some(track) = db.tracks.iter().find(|t| t.id == *id) else {
            continue;
        };
        let file_path = track.file_path();
        let analyze = track.analyze_path();
        match content_paths(&conn, file_path) {
            Ok(Some((path, analysis))) => {
                if !crate::anlz::same_export_path(&analysis, analyze) {
                    problems.push(format!(
                        "exportLibrary content {path} analysis path {analysis} != {analyze}"
                    ));
                }
            }
            Ok(None) => problems.push(format!(
                "exportLibrary.db has no content row for {file_path}"
            )),
            Err(e) => problems.push(format!("exportLibrary.db: {e}")),
        }
    }
    problems
}

fn content_paths(conn: &Db, path: &str) -> Result<Option<(String, String)>> {
    let stmt = conn.prepare(
        "SELECT path, analysisDataFilePath FROM content WHERE path = ?1 LIMIT 1",
    )?;
    stmt.bind_text(1, path)?;
    match stmt.step()? {
        Step::Row => Ok(Some((stmt.column_text(0), stmt.column_text(1)))),
        Step::Done => Ok(None),
    }
}

fn bind_opt_i64(stmt: &crate::sqlite_mc::Stmt, index: i32, value: Option<i64>) -> Result<()> {
    match value {
        Some(v) => stmt.bind_i64(index, v),
        None => stmt.bind_null(index),
    }
}

fn open_export_library(path: &Path) -> Result<Db> {
    Db::open_sqlcipher4(path, EXPORT_LIBRARY_KEY)
        .map_err(|e| Error::Database(format!("exportLibrary.db: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_and_verify_synthetic_library() {
        let root = std::env::temp_dir().join(format!("drokerbox-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let db_path = export_library_db_path(&root);
        std::fs::create_dir_all(db_path.parent().unwrap()).unwrap();

        let conn = open_export_library(&db_path).unwrap();
        conn.exec(
            "CREATE TABLE content (
                content_id integer primary key,
                path varchar,
                fileName varchar,
                fileSize integer,
                fileType integer,
                bitrate integer,
                bitDepth integer,
                samplingRate integer,
                analysisDataFilePath varchar
            );
            INSERT INTO content (
                content_id, path, fileName, fileSize, fileType, bitrate, bitDepth,
                samplingRate, analysisDataFilePath
            ) VALUES (
                1,
                '/Contents/A/Track.flac',
                'Track.flac',
                100,
                5,
                0,
                24,
                96000,
                '/PIONEER/USBANLZ/P024/00006070/ANLZ0000.DAT'
            );",
        )
        .unwrap();
        drop(conn);

        let summary = update_tracks(
            &root,
            &[LibraryTrackUpdate {
                old_path: "/Contents/A/Track.flac".into(),
                new_path: "/Contents/A/Track.mp3".into(),
                file_name: "Track.mp3".into(),
                file_size: 50,
                bitrate: 320,
                sample_rate: Some(44100),
                sample_depth: Some(16),
                analyze_path: "/PIONEER/USBANLZ/P036/0002F34C/ANLZ0000.DAT".into(),
            }],
        )
        .unwrap();
        assert_eq!(summary.updated, 1);
        assert!(summary.missing.is_empty());

        let conn = open_export_library(&db_path).unwrap();
        let stmt = conn
            .prepare(
                "SELECT path, fileName, fileType, samplingRate, analysisDataFilePath FROM content WHERE content_id = 1",
            )
            .unwrap();
        assert!(matches!(stmt.step().unwrap(), Step::Row));
        let path = stmt.column_text(0);
        let name = stmt.column_text(1);
        let file_type = stmt.column_i64(2);
        let rate = stmt.column_i64(3);
        let analysis = stmt.column_text(4);
        assert_eq!(path, "/Contents/A/Track.mp3");
        assert_eq!(name, "Track.mp3");
        assert_eq!(file_type, 1);
        assert_eq!(rate, 44100);
        assert_eq!(analysis, "/PIONEER/USBANLZ/P036/0002F34C/ANLZ0000.DAT");

        let _ = std::fs::remove_dir_all(&root);
    }
}
