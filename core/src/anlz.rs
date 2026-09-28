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

/// Pioneer USBANLZ directory for `audio_db_path` (e.g. `/Contents/A/Track.mp3`).
///
/// Players ignore the track row's `analyze_path` and hash the audio path
/// themselves. The result is `/PIONEER/USBANLZ/P{XXX}/{YYYYYYYY}/ANLZ0000.DAT`.
pub fn analyze_db_path(audio_db_path: &str) -> String {
    let (p_value, hash_value) = hash_audio_path(audio_db_path);
    format!("/PIONEER/USBANLZ/P{p_value:03X}/{hash_value:08X}/ANLZ0000.DAT")
}

/// `(P value, hash)` used in the USBANLZ directory name.
///
/// Matches the algorithm used by rekordbox / CDJ `CreateAnlzFileFolderPath`:
/// UTF-16 code units, multipliers `0x5BC9` and `0x93B5`, modulo `200003`.
pub fn hash_audio_path(file_path: &str) -> (u32, u32) {
    let mut hash_val: u32 = 0;
    for ch in file_path.chars() {
        let c = (ch as u32) & 0xFFFF;
        let temp = hash_val.wrapping_mul(0x5BC9).wrapping_add(c);
        hash_val = temp.wrapping_mul(0x93B5).wrapping_add(c);
    }
    let hash_result = hash_val % 200_003;
    let mut p = 0u32;
    p |= (hash_result >> 0) & 0x01;
    p |= (hash_result >> 1) & 0x02;
    p |= (hash_result >> 4) & 0x04;
    p |= (hash_result >> 4) & 0x08;
    p |= (hash_result >> 5) & 0x10;
    p |= (hash_result >> 8) & 0x20;
    p |= (hash_result >> 10) & 0x40;
    (p, hash_result)
}

/// True when two DeviceSQL paths refer to the same export location.
pub fn same_export_path(a: &str, b: &str) -> bool {
    normalize_export_path(a) == normalize_export_path(b)
}

fn normalize_export_path(path: &str) -> String {
    path.trim()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_ascii_lowercase()
}

/// Moves every `ANLZ*` file from `src_dir` into `dest_dir`.
///
/// Returns how many files were moved. The source directory (and its parent,
/// when left empty) is removed after a full move.
pub fn relocate_anlz_dir(src_dir: &Path, dest_dir: &Path) -> Result<u32> {
    if !src_dir.is_dir() {
        return Err(Error::Message(format!(
            "ANLZ directory missing: {}",
            src_dir.display()
        )));
    }
    if same_dir(src_dir, dest_dir) {
        return Ok(0);
    }

    let files = anlz_files_in(src_dir);
    if files.is_empty() {
        return Err(Error::Message(format!(
            "no ANLZ files in {}",
            src_dir.display()
        )));
    }

    std::fs::create_dir_all(dest_dir)?;
    for src in &files {
        let name = src.file_name().ok_or_else(|| {
            Error::Message(format!("ANLZ path has no file name: {}", src.display()))
        })?;
        let dest = dest_dir.join(name);
        if dest.exists() {
            std::fs::remove_file(&dest)?;
        }
        move_file(src, &dest)?;
    }

    remove_dir_if_empty(src_dir);
    if let Some(parent) = src_dir.parent() {
        remove_dir_if_empty(parent);
    }
    Ok(files.len() as u32)
}

/// Rewrites `PPTH` in every `ANLZ*` file in `dir`. Returns how many succeeded.
pub fn rewrite_dir_ppath(dir: &Path, new_audio_path: &str) -> Result<u32> {
    if !dir.is_dir() {
        return Err(Error::Message(format!(
            "ANLZ directory missing: {}",
            dir.display()
        )));
    }
    let mut updated = 0u32;
    for path in anlz_files_in(dir) {
        rewrite_ppath(&path, new_audio_path)?;
        updated += 1;
    }
    Ok(updated)
}

fn anlz_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name.starts_with("ANLZ") {
            out.push(path);
        }
    }
    out.sort();
    out
}

fn same_dir(a: &Path, b: &Path) -> bool {
    normalize_export_path(&a.to_string_lossy()) == normalize_export_path(&b.to_string_lossy())
}

fn move_file(src: &Path, dest: &Path) -> Result<()> {
    if std::fs::rename(src, dest).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dest)?;
    std::fs::remove_file(src)?;
    Ok(())
}

fn remove_dir_if_empty(dir: &Path) {
    let Ok(mut entries) = std::fs::read_dir(dir) else {
        return;
    };
    if entries.next().is_none() {
        let _ = std::fs::remove_dir(dir);
    }
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

    #[test]
    fn hash_matches_known_exports() {
        let cases = [
            (
                "/Contents/Leo Portela/Bon Vibrant - Leo Portela.flac",
                "/PIONEER/USBANLZ/P00E/000281CE/ANLZ0000.DAT",
            ),
            (
                "/Contents/Daniela Cast/Jazzy - Daniela Cast.flac",
                "/PIONEER/USBANLZ/P00A/0000CC9C/ANLZ0000.DAT",
            ),
            (
                "/Contents/Zorrovian/BIOS/Zorrovian - BIOS.flac",
                "/PIONEER/USBANLZ/P024/00006070/ANLZ0000.DAT",
            ),
            (
                "/Contents/Zorrovian/BIOS/Zorrovian - BIOS.mp3",
                "/PIONEER/USBANLZ/P036/0002F34C/ANLZ0000.DAT",
            ),
        ];
        for (audio, anlz) in cases {
            assert_eq!(analyze_db_path(audio), anlz, "{audio}");
        }
        assert_ne!(
            analyze_db_path("/Contents/A/Track.flac"),
            analyze_db_path("/Contents/A/Track.mp3")
        );
    }

    #[test]
    fn relocate_moves_anlz_and_rewrites_ppath() {
        let root = std::env::temp_dir().join(format!(
            "drokerbox-anlz-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let src = root.join("P024").join("00006070");
        let dest = root.join("P036").join("0002F34C");
        std::fs::create_dir_all(&src).unwrap();
        let audio = "/Contents/Zorrovian/BIOS/Zorrovian - BIOS.flac";
        let dat = sample_anlz(audio);
        std::fs::write(src.join("ANLZ0000.DAT"), &dat).unwrap();
        std::fs::write(src.join("ANLZ0000.EXT"), &dat).unwrap();

        assert_eq!(relocate_anlz_dir(&src, &dest).unwrap(), 2);
        assert!(!src.exists());
        let mp3 = "/Contents/Zorrovian/BIOS/Zorrovian - BIOS.mp3";
        assert_eq!(rewrite_dir_ppath(&dest, mp3).unwrap(), 2);
        let rewritten = std::fs::read(dest.join("ANLZ0000.DAT")).unwrap();
        assert_eq!(parse_ppath(&rewritten).as_deref(), Some(mp3));
        assert!(dest.join("ANLZ0000.EXT").is_file());

        let _ = std::fs::remove_dir_all(&root);
    }
}
