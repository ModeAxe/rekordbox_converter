//! ANLZ analysis-file handling (`PIONEER/USBANLZ/**/ANLZ*.DAT|.EXT|.2EX`).
//!
//! Each analysis file carries a `PPTH` tag: the UTF-16BE, NUL-terminated
//! path of the audio file the analysis belongs to. Format reference:
//! <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>

use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

/// Reads the `PPTH` audio path from one ANLZ file, if present.
pub fn read_ppath(anlz_path: impl AsRef<Path>) -> std::io::Result<Option<String>> {
    let data = std::fs::read(anlz_path)?;
    Ok(parse_ppath(&data))
}

/// Scans `PIONEER/USBANLZ` under `export_root` and maps analyze-path → audio path.
pub fn index_anlz_paths(export_root: impl AsRef<Path>) -> HashMap<String, String> {
    let anlz_root = export_root.as_ref().join("PIONEER").join("USBANLZ");
    let mut index = HashMap::new();
    if !anlz_root.is_dir() {
        return index;
    }

    for entry in WalkDir::new(&anlz_root)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy();
        if !name.starts_with("ANLZ") {
            continue;
        }
        let path = entry.path();
        if let Ok(Some(audio_path)) = read_ppath(path) {
            let analyze_key = anlz_path_to_db_key(path, &anlz_root);
            index.insert(analyze_key, audio_path);
        }
    }
    index
}

/// Converts an on-disk ANLZ path to the form stored in `export.pdb` track rows.
fn anlz_path_to_db_key(anlz_path: &Path, anlz_root: &Path) -> String {
    let rel = anlz_path
        .strip_prefix(anlz_root)
        .unwrap_or(anlz_path)
        .to_string_lossy()
        .replace('\\', "/");
    format!("/PIONEER/USBANLZ/{rel}")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

/// Finds a `PPTH` section in raw ANLZ bytes and decodes the UTF-16BE path.
fn parse_ppath(data: &[u8]) -> Option<String> {
    let mut i = 0usize;
    while i + 16 <= data.len() {
        if &data[i..i + 4] != b"PPTH" {
            i += 1;
            continue;
        }
        let len_header = u32::from_be_bytes(data[i + 4..i + 8].try_into().ok()?);
        let _len_tag = u32::from_be_bytes(data[i + 8..i + 12].try_into().ok()?);
        let len_path = u32::from_be_bytes(data[i + 12..i + 16].try_into().ok()?);
        let path_start = i + len_header as usize;
        let path_end = path_start.checked_add(len_path as usize)?;
        if path_end > data.len() {
            i += 1;
            continue;
        }
        let path_bytes = &data[path_start..path_end];
        return Some(decode_utf16be_nul(path_bytes));
    }
    None
}

fn decode_utf16be_nul(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .take_while(|&u| u != 0)
        .collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ppath_from_synthetic_anlz() {
        // Minimal PPTH tag: header len 16, path "/A/B.flac" as UTF-16BE + NUL
        let path = "/A/B.flac";
        let mut utf16: Vec<u8> = path
            .encode_utf16()
            .flat_map(|u| u.to_be_bytes())
            .collect();
        utf16.extend_from_slice(&[0, 0]);
        let len_path = utf16.len() as u32;
        let len_header = 16u32;
        let len_tag = len_header + len_path;

        let mut data = Vec::new();
        data.extend_from_slice(b"PPTH");
        data.extend_from_slice(&len_header.to_be_bytes());
        data.extend_from_slice(&len_tag.to_be_bytes());
        data.extend_from_slice(&len_path.to_be_bytes());
        data.extend_from_slice(&utf16);

        assert_eq!(parse_ppath(&data).as_deref(), Some("/A/B.flac"));
    }
}
