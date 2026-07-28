//! Byte-identical round-trip evaluation for Pioneer PDB libraries.
//!
//! Acceptance criterion (Phase 2): load `export.pdb`, re-serialize with zero edits,
//! and compare against the original bytes. Whichever library passes becomes the
//! writer for Phase 4.

use std::path::Path;

use serde::Serialize;

/// Outcome of evaluating PDB libraries on a single `export.pdb` file.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtripEvaluation {
    pub source_path: String,
    pub source_bytes: u64,
    pub rekordbox_pdb: RoundtripLibraryResult,
    pub rekordcrate: RoundtripLibraryResult,
    /// Which library we recommend for Phase 4 rewriting.
    pub recommended: RecommendedLibrary,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RecommendedLibrary {
    RekordboxPdb,
    Rekordcrate,
    None,
}

/// Per-library round-trip / parse result.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoundtripLibraryResult {
    pub parsed: bool,
    pub byte_identical: bool,
    pub track_count: Option<u32>,
    pub message: String,
}

/// Runs the Phase 2 acceptance tests against `export_pdb`.
pub fn evaluate_roundtrip(export_pdb: impl AsRef<Path>) -> crate::Result<RoundtripEvaluation> {
    let path = export_pdb.as_ref();
    let original = std::fs::read(path)?;
    let source_bytes = original.len() as u64;

    let rekordbox_pdb = eval_rekordbox_pdb(&original);
    let rekordcrate = eval_rekordcrate(&original);

    let recommended = if rekordbox_pdb.byte_identical {
        RecommendedLibrary::RekordboxPdb
    } else if rekordcrate.byte_identical {
        RecommendedLibrary::Rekordcrate
    } else if rekordbox_pdb.parsed {
        RecommendedLibrary::RekordboxPdb
    } else {
        RecommendedLibrary::None
    };

    Ok(RoundtripEvaluation {
        source_path: path.display().to_string(),
        source_bytes,
        rekordbox_pdb,
        rekordcrate,
        recommended,
    })
}

fn eval_rekordbox_pdb(original: &[u8]) -> RoundtripLibraryResult {
    use rekordbox_pdb::{Database, PdbEditor};

    let parsed_db = Database::from_bytes(original.to_vec());

    match parsed_db {
        Err(e) => RoundtripLibraryResult {
            parsed: false,
            byte_identical: false,
            track_count: None,
            message: format!("parse failed: {e}"),
        },
        Ok(db) => {
            let mut editor = PdbEditor::from_bytes(original.to_vec());
            let rewritten = editor.to_bytes();
            let identical = rewritten == original;
            RoundtripLibraryResult {
                parsed: true,
                byte_identical: identical,
                track_count: Some(db.tracks.len() as u32),
                message: if identical {
                    "no-op round-trip is byte-identical".into()
                } else {
                    format!(
                        "no-op round-trip differs by {} byte(s)",
                        byte_diff_count(original, &rewritten)
                    )
                },
            }
        }
    }
}

fn eval_rekordcrate(original: &[u8]) -> RoundtripLibraryResult {
    #[cfg(feature = "rekordcrate")]
    {
        use std::io::Cursor;

        use binrw::BinRead;
        use rekordcrate::pdb::{Header, PageType, Row};

        let mut cursor = Cursor::new(original);
        match Header::read(&mut cursor) {
            Err(e) => RoundtripLibraryResult {
                parsed: false,
                byte_identical: false,
                track_count: None,
                message: format!("parse failed: {e}"),
            },
            Ok(header) => {
                let mut track_count = 0u32;
                for table in &header.tables {
                    if table.page_type != PageType::Tracks {
                        continue;
                    }
                    let pages = header.read_pages(
                        &mut Cursor::new(original),
                        binrw::Endian::Little,
                        (&table.first_page, &table.last_page),
                    );
                    if let Ok(pages) = pages {
                        for page in pages {
                            for group in &page.row_groups {
                                for row in group.present_rows() {
                                    if matches!(row, Row::Track(_)) {
                                        track_count += 1;
                                    }
                                }
                            }
                        }
                    }
                }
                RoundtripLibraryResult {
                    parsed: true,
                    byte_identical: false,
                    track_count: Some(track_count),
                    message: "read-only parser — no full-file serialize API in rekordcrate 0.3.0"
                        .into(),
                }
            }
        }
    }
    #[cfg(not(feature = "rekordcrate"))]
    {
        let _ = original;
        RoundtripLibraryResult {
            parsed: false,
            byte_identical: false,
            track_count: None,
            message: "rekordcrate feature not enabled in this build".into(),
        }
    }
}

fn byte_diff_count(a: &[u8], b: &[u8]) -> usize {
    a.iter()
        .zip(b.iter())
        .filter(|(x, y)| x != y)
        .count()
        .saturating_add(a.len().abs_diff(b.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_diff_count_works() {
        assert_eq!(byte_diff_count(b"abc", b"abc"), 0);
        assert_eq!(byte_diff_count(b"abc", b"abd"), 1);
    }

    #[test]
    fn evaluate_on_missing_file_errors() {
        let err = evaluate_roundtrip("/nonexistent/export.pdb").unwrap_err();
        assert!(matches!(err, crate::Error::Io(_)));
    }
}
