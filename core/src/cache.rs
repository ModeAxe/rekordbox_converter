//! Converted-MP3 cache.
//!
//! Layout mirrors the USB `Contents/` tree:
//! `<cache root>/<Artist>/…/<Track>.mp3`, with a sidecar
//! `<Track>.mp3.cachekey` storing `size-mtime-crc32` of the source FLAC.
//! A hit copies (or reuses) the cached MP3 instead of re-converting.

use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// Sidecar suffix written next to each cached MP3.
pub const CACHE_KEY_SUFFIX: &str = ".cachekey";

/// Identity of a source FLAC used as the cache key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CacheKey {
    pub size: u64,
    /// Modified time as Unix epoch milliseconds.
    pub mtime_ms: u64,
    pub crc32: u32,
}

impl CacheKey {
    /// Compact on-disk encoding: `size-mtime-crc`.
    pub fn encode(&self) -> String {
        format!("{}-{}-{:08x}", self.size, self.mtime_ms, self.crc32)
    }

    pub fn decode(s: &str) -> Option<Self> {
        let mut parts = s.trim().splitn(3, '-');
        let size = parts.next()?.parse().ok()?;
        let mtime_ms = parts.next()?.parse().ok()?;
        let crc32 = u32::from_str_radix(parts.next()?, 16).ok()?;
        Some(Self {
            size,
            mtime_ms,
            crc32,
        })
    }
}

/// Computes size + mtime + CRC32 for `path`.
pub fn compute_key(path: impl AsRef<Path>) -> Result<CacheKey> {
    let path = path.as_ref();
    let meta = fs::metadata(path)?;
    let size = meta.len();
    let mtime_ms = meta
        .modified()
        .unwrap_or(SystemTime::UNIX_EPOCH)
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);

    let file = File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut hasher = crc32fast::Hasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }

    Ok(CacheKey {
        size,
        mtime_ms,
        crc32: hasher.finalize(),
    })
}

/// Manages a conversion cache root directory.
#[derive(Debug, Clone)]
pub struct CacheManager {
    root: PathBuf,
}

impl CacheManager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Ensures the cache root exists.
    pub fn ensure_root(&self) -> Result<()> {
        fs::create_dir_all(&self.root)?;
        Ok(())
    }

    /// Maps a FLAC on the USB to its cached MP3 path.
    ///
    /// Uses the path relative to `contents_root` when possible, otherwise
    /// `parent_name/filename.mp3`.
    pub fn cache_path_for(&self, flac: &Path, contents_root: Option<&Path>) -> PathBuf {
        let relative = if let Some(contents) = contents_root {
            flac.strip_prefix(contents)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| fallback_relative(flac))
        } else {
            fallback_relative(flac)
        };
        let mut mp3_rel = relative;
        mp3_rel.set_extension("mp3");
        self.root.join(mp3_rel)
    }

    fn key_path_for_mp3(mp3: &Path) -> PathBuf {
        let mut s = mp3.as_os_str().to_owned();
        s.push(CACHE_KEY_SUFFIX);
        PathBuf::from(s)
    }

    /// Returns the cached MP3 path if it exists and the key matches.
    pub fn lookup(&self, flac: &Path, contents_root: Option<&Path>) -> Result<Option<PathBuf>> {
        let mp3 = self.cache_path_for(flac, contents_root);
        if !mp3.is_file() {
            return Ok(None);
        }
        let key_path = Self::key_path_for_mp3(&mp3);
        let stored = match fs::read_to_string(&key_path) {
            Ok(s) => CacheKey::decode(&s),
            Err(_) => return Ok(None),
        };
        let Some(stored) = stored else {
            return Ok(None);
        };
        let current = compute_key(flac)?;
        if stored == current {
            Ok(Some(mp3))
        } else {
            Ok(None)
        }
    }

    /// Writes (or overwrites) the cache key sidecar for a freshly produced MP3.
    pub fn store_key(&self, mp3: &Path, key: &CacheKey) -> Result<()> {
        if let Some(parent) = mp3.parent() {
            fs::create_dir_all(parent)?;
        }
        let key_path = Self::key_path_for_mp3(mp3);
        fs::write(key_path, key.encode())?;
        Ok(())
    }

    /// Copies `src_mp3` into the cache slot for `flac`, writing the key sidecar.
    pub fn store_copy(
        &self,
        flac: &Path,
        contents_root: Option<&Path>,
        src_mp3: &Path,
        key: &CacheKey,
    ) -> Result<PathBuf> {
        let dest = self.cache_path_for(flac, contents_root);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(src_mp3, &dest)?;
        self.store_key(&dest, key)?;
        Ok(dest)
    }
}

fn fallback_relative(flac: &Path) -> PathBuf {
    let file = flac.file_name().unwrap_or_default();
    let artist = flac
        .parent()
        .and_then(|p| p.file_name())
        .unwrap_or_else(|| std::ffi::OsStr::new("Unknown"));
    PathBuf::from(artist).join(file)
}

/// Default cache directory: `%LOCALAPPDATA%/RekordboxUsbConverter/Cache` on Windows,
/// otherwise `~/.cache/rekordbox-usb-converter`.
pub fn default_cache_root() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local)
            .join("RekordboxUsbConverter")
            .join("Cache");
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home)
            .join(".cache")
            .join("rekordbox-usb-converter");
    }
    std::env::temp_dir().join("rekordbox-usb-converter-cache")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_roundtrip() {
        let k = CacheKey {
            size: 12345,
            mtime_ms: 999,
            crc32: 0xdead_beef,
        };
        assert_eq!(CacheKey::decode(&k.encode()), Some(k));
    }

    #[test]
    fn lookup_hit_and_miss() {
        let dir = std::env::temp_dir().join(format!("rbusb-cache-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let contents = dir.join("Contents");
        let cache_root = dir.join("Cache");
        fs::create_dir_all(contents.join("Artist")).unwrap();
        let flac = contents.join("Artist/song.flac");
        fs::write(&flac, b"fake flac data for crc").unwrap();

        let cache = CacheManager::new(&cache_root);
        cache.ensure_root().unwrap();
        assert!(cache.lookup(&flac, Some(&contents)).unwrap().is_none());

        let key = compute_key(&flac).unwrap();
        let mp3 = cache.cache_path_for(&flac, Some(&contents));
        fs::create_dir_all(mp3.parent().unwrap()).unwrap();
        fs::write(&mp3, b"mp3").unwrap();
        cache.store_key(&mp3, &key).unwrap();

        let hit = cache.lookup(&flac, Some(&contents)).unwrap();
        assert_eq!(hit.as_deref(), Some(mp3.as_path()));

        fs::write(&flac, b"changed").unwrap();
        assert!(cache.lookup(&flac, Some(&contents)).unwrap().is_none());

        let _ = fs::remove_dir_all(&dir);
    }
}
