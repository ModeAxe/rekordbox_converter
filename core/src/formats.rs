//! Audio format policy for USB conversion.
//!
//! - **Passthrough:** `.mp3` is left alone.
//! - **Convertible:** known Rekordbox lossless/lossy sources → MP3.
//! - **Ignore:** everything else (artwork, unknown extensions).

use std::path::{Path, PathBuf};

/// Extensions treated as already-compatible MP3 (no re-encode).
pub fn is_mp3(ext: &str) -> bool {
    ext.eq_ignore_ascii_case("mp3")
}

/// Known non-MP3 audio Rekordbox may export; these become MP3.
pub fn is_convertible(ext: &str) -> bool {
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "flac" | "wav" | "aiff" | "aif" | "m4a" | "aac" | "ogg" | "wma"
    )
}

/// Extension of a path, lowercased, or empty if missing.
pub fn extension_of(path: impl AsRef<Path>) -> String {
    path.as_ref()
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// True if this filesystem path is convertible audio.
pub fn path_is_convertible(path: impl AsRef<Path>) -> bool {
    is_convertible(&extension_of(path))
}

/// True if this filesystem path is an MP3.
pub fn path_is_mp3(path: impl AsRef<Path>) -> bool {
    is_mp3(&extension_of(path))
}

/// Sibling MP3 path for a source file (`track.wav` → `track.mp3`).
pub fn to_mp3_path(path: impl AsRef<Path>) -> PathBuf {
    path.as_ref().with_extension("mp3")
}

/// Rewrites a DeviceSQL path extension to `.mp3` (keeps forward slashes).
///
/// Non-convertible / already-mp3 paths still get `.mp3` if they have an
/// extension; bare paths without an extension append `.mp3`.
pub fn db_path_to_mp3(db_path: &str) -> String {
    let as_path = PathBuf::from(db_path);
    if as_path.extension().is_some() {
        let mut p = as_path;
        p.set_extension("mp3");
        return p.to_string_lossy().replace('\\', "/");
    }
    // No extension: append.
    if db_path.is_empty() {
        return ".mp3".into();
    }
    format!("{db_path}.mp3").replace('\\', "/")
}

/// Rough seconds-per-MB for convert-to-320k on a typical laptop CPU.
const SECONDS_PER_MB: f64 = 0.35;

/// Wall-clock estimate from total convertible bytes + file count.
pub fn estimate_seconds(bytes: u64, count: u32) -> u32 {
    if count == 0 {
        return 0;
    }
    let mb = bytes as f64 / (1024.0 * 1024.0);
    let from_size = (mb * SECONDS_PER_MB).ceil() as u32;
    let from_count = count.saturating_mul(2);
    from_size.max(from_count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn convertible_allowlist() {
        assert!(is_convertible("flac"));
        assert!(is_convertible("FLAC"));
        assert!(is_convertible("wav"));
        assert!(is_convertible("aiff"));
        assert!(is_convertible("aif"));
        assert!(is_convertible("m4a"));
        assert!(is_convertible("aac"));
        assert!(is_convertible("ogg"));
        assert!(is_convertible("wma"));
        assert!(!is_convertible("mp3"));
        assert!(!is_convertible("jpg"));
        assert!(!is_convertible("tmp"));
    }

    #[test]
    fn mp3_detection() {
        assert!(is_mp3("mp3"));
        assert!(is_mp3("MP3"));
        assert!(!is_mp3("flac"));
    }

    #[test]
    fn db_path_swap() {
        assert_eq!(
            db_path_to_mp3("/Contents/A/x.flac"),
            "/Contents/A/x.mp3"
        );
        assert_eq!(
            db_path_to_mp3("/Contents/A/x.FLAC"),
            "/Contents/A/x.mp3"
        );
        assert_eq!(
            db_path_to_mp3("/Contents/A/x.wav"),
            "/Contents/A/x.mp3"
        );
        assert_eq!(
            db_path_to_mp3("/Contents/A/x.aiff"),
            "/Contents/A/x.mp3"
        );
        assert_eq!(
            db_path_to_mp3("/Contents/A/x.mp3"),
            "/Contents/A/x.mp3"
        );
    }

    #[test]
    fn to_mp3_path_fs() {
        let p = PathBuf::from(r"E:\Contents\A\track.WAV");
        assert_eq!(to_mp3_path(&p), PathBuf::from(r"E:\Contents\A\track.mp3"));
    }
}
