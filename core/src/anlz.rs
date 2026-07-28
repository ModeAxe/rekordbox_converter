//! ANLZ analysis-file handling (`PIONEER/USBANLZ/**/ANLZ*.DAT|.EXT|.2EX`).
//!
//! Each analysis file carries a `PPTH` tag: the UTF-16BE, NUL-terminated
//! path of the audio file the analysis belongs to. Format reference:
//! <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::{Error, Result};

/// Reads the `PPTH` audio path from one ANLZ file, if present.
pub fn read_ppath(anlz_path: impl AsRef<Path>) -> std::io::Result<Option<String>> {
    let data = std::fs::read(anlz_path)?;
    Ok(parse_ppath(&data))
}

/// Rewrites the `PPTH` tag to `new_audio_path` and updates the PMAI total size.
///
/// Other tags (beatgrid, cues, waveforms) are preserved byte-for-byte, shifted
/// only if the new path encoding changes the PPTH section length.
pub fn rewrite_ppath(anlz_path: impl AsRef<Path>, new_audio_path: &str) -> Result<()> {
    let path = anlz_path.as_ref();
    let data = std::fs::read(path)?;
    let rewritten = rewrite_ppath_bytes(&data, new_audio_path)?;
    std::fs::write(path, rewritten)?;
    Ok(())
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

/// Absolute paths of every ANLZ file under the export (DAT/EXT/2EX).
pub fn list_anlz_files(export_root: impl AsRef<Path>) -> Vec<PathBuf> {
    let anlz_root = export_root.as_ref().join("PIONEER").join("USBANLZ");
    let mut out = Vec::new();
    if !anlz_root.is_dir() {
        return out;
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
        if name.starts_with("ANLZ") {
            out.push(entry.path().to_path_buf());
        }
    }
    out
}

/// Converts an on-disk ANLZ path to the form stored in `export.pdb` track rows.
pub fn anlz_path_to_db_key(anlz_path: &Path, anlz_root: &Path) -> String {
    let rel = anlz_path
        .strip_prefix(anlz_root)
        .unwrap_or(anlz_path)
        .to_string_lossy()
        .replace('\\', "/");
    format!("/PIONEER/USBANLZ/{rel}")
        .replace('\\', "/")
        .to_ascii_lowercase()
}

fn rewrite_ppath_bytes(data: &[u8], new_audio_path: &str) -> Result<Vec<u8>> {
    let (tag_start, old_tag_len) = find_ppath_section(data).ok_or_else(|| {
        Error::Message("ANLZ file has no PPTH tag".into())
    })?;

    let new_tag = build_ppath_tag(new_audio_path);
    let mut out = Vec::with_capacity(data.len() - old_tag_len + new_tag.len());
    out.extend_from_slice(&data[..tag_start]);
    out.extend_from_slice(&new_tag);
    out.extend_from_slice(&data[tag_start + old_tag_len..]);

    // Update PMAI total_size (bytes 8..12, big-endian) when present.
    if out.len() >= 12 && &out[0..4] == b"PMAI" {
        let total = out.len() as u32;
        out[8..12].copy_from_slice(&total.to_be_bytes());
    }
    Ok(out)
}

fn find_ppath_section(data: &[u8]) -> Option<(usize, usize)> {
    let mut i = 0usize;
    while i + 12 <= data.len() {
        if &data[i..i + 4] != b"PPTH" {
            i += 1;
            continue;
        }
        let len_tag = u32::from_be_bytes(data[i + 8..i + 12].try_into().ok()?) as usize;
        if len_tag < 12 || i + len_tag > data.len() {
            i += 1;
            continue;
        }
        return Some((i, len_tag));
    }
    None
}

fn build_ppath_tag(path: &str) -> Vec<u8> {
    let mut utf16: Vec<u8> = path
        .encode_utf16()
        .flat_map(|u| u.to_be_bytes())
        .collect();
    utf16.extend_from_slice(&[0, 0]); // NUL
    let len_path = utf16.len() as u32;
    let len_header = 16u32;
    let len_tag = len_header + len_path;

    let mut tag = Vec::with_capacity(len_tag as usize);
    tag.extend_from_slice(b"PPTH");
    tag.extend_from_slice(&len_header.to_be_bytes());
    tag.extend_from_slice(&len_tag.to_be_bytes());
    tag.extend_from_slice(&len_path.to_be_bytes());
    tag.extend_from_slice(&utf16);
    tag
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

    fn sample_anlz(path: &str) -> Vec<u8> {
        let ppth = build_ppath_tag(path);
        let mut data = Vec::new();
        // Minimal PMAI header wrapping the rest.
        let total = (12 + ppth.len()) as u32;
        data.extend_from_slice(b"PMAI");
        data.extend_from_slice(&12u32.to_be_bytes());
        data.extend_from_slice(&total.to_be_bytes());
        data.extend_from_slice(&ppth);
        data
    }

    #[test]
    fn parse_ppath_from_synthetic_anlz() {
        let data = sample_anlz("/A/B.flac");
        assert_eq!(parse_ppath(&data).as_deref(), Some("/A/B.flac"));
    }

    #[test]
    fn rewrite_ppath_flac_to_mp3() {
        let data = sample_anlz("/Contents/A/Track.flac");
        let out = rewrite_ppath_bytes(&data, "/Contents/A/Track.mp3").unwrap();
        assert_eq!(parse_ppath(&out).as_deref(), Some("/Contents/A/Track.mp3"));
        let total = u32::from_be_bytes(out[8..12].try_into().unwrap());
        assert_eq!(total as usize, out.len());
        // Shorter extension → smaller file.
        assert!(out.len() < data.len());
    }
}
