//! FFmpeg wrapper and conversion queue.
//!
//! Spawns the bundled `ffmpeg` sidecar to transcode non-MP3 audio → MP3
//! (default 320 kbps CBR, `-map_metadata 0 -id3v2_version 3`, artwork stream
//! copied) and runs a multi-threaded worker pool with per-track progress.
//!
//! The path to the ffmpeg executable is injected by the caller so this
//! module stays free of Tauri dependencies.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use rayon::prelude::*;
use serde::Serialize;

use crate::cache::{self, CacheManager};
use crate::error::{Error, Result};

/// Options for a conversion batch.
#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Absolute path to the `ffmpeg` executable.
    pub ffmpeg_path: PathBuf,
    /// Cache root directory.
    pub cache_root: PathBuf,
    /// MP3 bitrate string passed to `-b:a` (e.g. `"320k"`).
    pub bitrate: String,
    /// Number of parallel FFmpeg workers.
    pub workers: usize,
    /// USB `Contents/` directory, used to derive cache-relative paths.
    pub contents_root: Option<PathBuf>,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self {
            ffmpeg_path: PathBuf::from("ffmpeg"),
            cache_root: cache::default_cache_root(),
            bitrate: "320k".into(),
            workers: default_worker_count(),
            contents_root: None,
        }
    }
}

fn default_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get().clamp(1, 8))
        .unwrap_or(2)
}

/// How a single track was produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ConvertOutcome {
    CacheHit,
    Converted,
    Failed,
}

/// Progress event emitted after each track.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertProgress {
    pub completed: u32,
    pub total: u32,
    pub current_file: String,
    pub outcome: ConvertOutcome,
    pub cache_path: Option<String>,
    pub error: Option<String>,
    pub cache_hits: u32,
    pub converted: u32,
    pub failed: u32,
}

/// Final summary of a convert-to-cache run.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConvertSummary {
    pub total: u32,
    pub cache_hits: u32,
    pub converted: u32,
    pub failed: u32,
    pub cache_root: String,
    pub errors: Vec<String>,
}

/// Converts (or cache-copies) every source into the cache. Never writes to the USB.
///
/// `on_progress` is called after each track (may be called from worker threads).
pub fn convert_to_cache<F>(
    source_paths: &[PathBuf],
    options: &ConvertOptions,
    cancel: Option<Arc<AtomicBool>>,
    mut on_progress: F,
) -> Result<ConvertSummary>
where
    F: FnMut(ConvertProgress) + Send,
{
    if source_paths.is_empty() {
        return Ok(ConvertSummary {
            total: 0,
            cache_hits: 0,
            converted: 0,
            failed: 0,
            cache_root: options.cache_root.display().to_string(),
            errors: Vec::new(),
        });
    }

    let cache = CacheManager::new(&options.cache_root);
    cache.ensure_root()?;

    let total = source_paths.len() as u32;
    let completed = AtomicU32::new(0);
    let cache_hits = AtomicU32::new(0);
    let converted = AtomicU32::new(0);
    let failed = AtomicU32::new(0);
    let errors: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let progress_lock = std::sync::Mutex::new(&mut on_progress);

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(options.workers.max(1))
        .build()
        .map_err(|e| Error::Message(format!("failed to build worker pool: {e}")))?;

    let contents = options.contents_root.as_deref();
    let ffmpeg = &options.ffmpeg_path;
    let bitrate = &options.bitrate;

    pool.install(|| {
        source_paths.par_iter().for_each(|source| {
            if cancel
                .as_ref()
                .is_some_and(|c| c.load(Ordering::Relaxed))
            {
                return;
            }

            let display_name = source
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| source.display().to_string());

            let result = process_one(source, contents, &cache, ffmpeg, bitrate);

            let (outcome, cache_path, error) = match result {
                Ok((ConvertOutcome::CacheHit, path)) => {
                    cache_hits.fetch_add(1, Ordering::Relaxed);
                    (ConvertOutcome::CacheHit, Some(path), None)
                }
                Ok((ConvertOutcome::Converted, path)) => {
                    converted.fetch_add(1, Ordering::Relaxed);
                    (ConvertOutcome::Converted, Some(path), None)
                }
                Ok((ConvertOutcome::Failed, _)) => unreachable!(),
                Err(e) => {
                    failed.fetch_add(1, Ordering::Relaxed);
                    let msg = format!("{display_name}: {e}");
                    if let Ok(mut errs) = errors.lock() {
                        errs.push(msg.clone());
                    }
                    (ConvertOutcome::Failed, None, Some(msg))
                }
            };

            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            let event = ConvertProgress {
                completed: done,
                total,
                current_file: display_name,
                outcome,
                cache_path: cache_path.map(|p| p.display().to_string()),
                error,
                cache_hits: cache_hits.load(Ordering::Relaxed),
                converted: converted.load(Ordering::Relaxed),
                failed: failed.load(Ordering::Relaxed),
            };
            if let Ok(mut cb) = progress_lock.lock() {
                cb(event);
            }
        });
    });

    Ok(ConvertSummary {
        total,
        cache_hits: cache_hits.load(Ordering::Relaxed),
        converted: converted.load(Ordering::Relaxed),
        failed: failed.load(Ordering::Relaxed),
        cache_root: options.cache_root.display().to_string(),
        errors: errors.into_inner().unwrap_or_default(),
    })
}

fn process_one(
    source: &Path,
    contents: Option<&Path>,
    cache: &CacheManager,
    ffmpeg: &Path,
    bitrate: &str,
) -> Result<(ConvertOutcome, PathBuf)> {
    if let Some(hit) = cache.lookup(source, contents)? {
        return Ok((ConvertOutcome::CacheHit, hit));
    }

    let key = cache::compute_key(source)?;
    let dest = cache.cache_path_for(source, contents);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Temp must end in .mp3 so FFmpeg can pick the muxer (`.mp3.partial` fails).
    let tmp = {
        let stem = dest
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "track".into());
        dest.with_file_name(format!("{stem}.partial.mp3"))
    };
    if tmp.exists() {
        let _ = std::fs::remove_file(&tmp);
    }

    run_ffmpeg(ffmpeg, source, &tmp, bitrate)?;

    if !tmp.is_file() || std::fs::metadata(&tmp)?.len() == 0 {
        let _ = std::fs::remove_file(&tmp);
        return Err(Error::Message(format!(
            "ffmpeg produced empty output for {}",
            source.display()
        )));
    }

    std::fs::rename(&tmp, &dest).or_else(|_| {
        std::fs::copy(&tmp, &dest)?;
        std::fs::remove_file(&tmp)?;
        Ok::<(), Error>(())
    })?;

    cache.store_key(&dest, &key)?;
    Ok((ConvertOutcome::Converted, dest))
}

/// Runs FFmpeg with the project-standard flags, plus artwork stream copy.
///
/// ```text
/// ffmpeg -y -i input.wav \
///   -map 0:a:0 -map 0:v? \
///   -codec:a libmp3lame -b:a 320k \
///   -codec:v copy \
///   -map_metadata 0 -id3v2_version 3 \
///   output.mp3
/// ```
pub fn run_ffmpeg(
    ffmpeg: &Path,
    input: &Path,
    output: &Path,
    bitrate: &str,
) -> Result<()> {
    let mut cmd = Command::new(ffmpeg);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    let output_run = cmd
        .arg("-hide_banner")
        .arg("-nostdin")
        .arg("-y")
        .arg("-i")
        .arg(input)
        .arg("-map")
        .arg("0:a:0")
        .arg("-map")
        .arg("0:v?")
        .arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg(bitrate)
        .arg("-codec:v")
        .arg("copy")
        .arg("-map_metadata")
        .arg("0")
        .arg("-id3v2_version")
        .arg("3")
        .arg("-f")
        .arg("mp3")
        .arg(output)
        .output()?;

    if !output_run.status.success() {
        let stderr = String::from_utf8_lossy(&output_run.stderr);
        let tail: String = stderr
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::Message(format!(
            "ffmpeg failed ({}):\n{tail}",
            output_run.status
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_workers_at_least_one() {
        assert!(default_worker_count() >= 1);
    }
}
