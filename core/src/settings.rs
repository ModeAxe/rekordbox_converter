//! Persistent user settings for the converter.
//!
//! Stored as JSON under `%LOCALAPPDATA%/RekordboxUsbConverter/settings.json`.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::cache;
use crate::error::{Error, Result};

/// User-configurable options.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    /// FFmpeg `-b:a` value, e.g. `"320k"`.
    pub bitrate: String,
    /// Override cache directory. Empty / missing → default cache root.
    #[serde(default)]
    pub cache_root: String,
    /// Parallel FFmpeg workers. `0` = auto (1–8 based on CPU).
    pub workers: u32,
    /// Keep FLAC files on the USB after a successful in-place convert.
    pub keep_flac: bool,
    /// Re-verify the rewritten PDB before deleting FLACs.
    pub verify_output: bool,
    /// Plan-only: resolve scope and report, do not write the USB.
    pub dry_run: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            bitrate: "320k".into(),
            cache_root: String::new(),
            workers: 0,
            keep_flac: false,
            verify_output: true,
            dry_run: false,
        }
    }
}

impl AppSettings {
    /// Effective cache directory.
    pub fn resolved_cache_root(&self) -> PathBuf {
        if self.cache_root.trim().is_empty() {
            cache::default_cache_root()
        } else {
            PathBuf::from(self.cache_root.trim())
        }
    }

    /// Resolve worker count (`0` → auto).
    pub fn resolved_workers(&self) -> usize {
        if self.workers == 0 {
            std::thread::available_parallelism()
                .map(|n| n.get().clamp(1, 8))
                .unwrap_or(2)
        } else {
            (self.workers as usize).clamp(1, 16)
        }
    }

    /// Numeric kbps for PDB rewrite (e.g. `"320k"` → `320`).
    pub fn bitrate_kbps(&self) -> u32 {
        parse_bitrate_kbps(&self.bitrate).unwrap_or(320)
    }
}

/// Parses `"320k"` / `"320"` / `"192K"` into kbps.
pub fn parse_bitrate_kbps(s: &str) -> Option<u32> {
    let t = s.trim().to_ascii_lowercase();
    let digits = t.strip_suffix('k').unwrap_or(&t);
    digits.parse().ok()
}

/// `%LOCALAPPDATA%/RekordboxUsbConverter/settings.json`
pub fn settings_path() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local)
            .join("RekordboxUsbConverter")
            .join("settings.json");
    }
    std::env::temp_dir()
        .join("rekordbox-usb-converter")
        .join("settings.json")
}

pub fn load() -> AppSettings {
    let path = settings_path();
    let Ok(data) = fs::read_to_string(&path) else {
        return AppSettings::default();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

pub fn save(settings: &AppSettings) -> Result<()> {
    let path = settings_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| Error::Message(e.to_string()))?;
    fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_bitrate_variants() {
        assert_eq!(parse_bitrate_kbps("320k"), Some(320));
        assert_eq!(parse_bitrate_kbps("192K"), Some(192));
        assert_eq!(parse_bitrate_kbps("256"), Some(256));
        assert_eq!(parse_bitrate_kbps("nope"), None);
    }

    #[test]
    fn default_roundtrip_json() {
        let s = AppSettings::default();
        let json = serde_json::to_string(&s).unwrap();
        let back: AppSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.bitrate, "320k");
        assert!(back.verify_output);
        assert!(!back.dry_run);
    }
}
