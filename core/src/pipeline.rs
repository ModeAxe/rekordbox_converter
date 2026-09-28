//! Staged-copy pipeline (Phase 4) and in-place USB pipeline (Phase 5).
//!
//! Phase 4 builds a converted copy under a user-chosen output directory.
//! Phase 5 converts on the live USB with backup + rollback before any source
//! deletion.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;

use rekordbox_pdb::Database;
use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

use crate::anlz;
use crate::cache::{self, CacheManager};
use crate::convert::{self, ConvertOptions};
use crate::error::{Error, Result};
use crate::pdb::{
    resolve_convert_scope, resolve_usb_path, rewrite_track_audio, verify_rewritten_pdb,
    TrackAudioRewrite,
};
use crate::scanner::{export_ext_pdb_path, export_pdb_path};

/// Backup folder name written next to the export on the USB.
pub const BACKUP_DIR_NAME: &str = ".rbconvert-backup";

/// Progress event for the staged-copy pipeline.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineProgress {
    pub phase: String,
    pub completed: u32,
    pub total: u32,
    pub detail: String,
}

/// Summary returned when a staged copy finishes.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StagedCopySummary {
    pub output_root: String,
    pub tracks_converted: u32,
    pub cache_hits: u32,
    pub pdb_updated: u32,
    pub anlz_updated: u32,
    pub sources_removed: u32,
    pub verify_problems: Vec<String>,
    pub errors: Vec<String>,
}

/// Options for building a converted staged copy.
#[derive(Debug, Clone)]
pub struct StagedCopyOptions {
    pub source_root: PathBuf,
    pub output_root: PathBuf,
    pub playlist_id: Option<u32>,
    pub ffmpeg_path: PathBuf,
    pub cache_root: PathBuf,
    pub bitrate: String,
    pub workers: usize,
}

/// Builds a converted Rekordbox export under `output_root`.
///
/// Steps:
/// 1. Create output dir; copy `PIONEER/` tree
/// 2. Copy scoped convertible files into `Contents/` (preserving relative paths)
/// 3. Convert/copy-from-cache each source → MP3 alongside it
/// 4. Rewrite `export.pdb` track rows + ANLZ `PPTH` tags
/// 5. Verify; delete sources from the copy
///
/// The source USB is never written.
pub fn build_staged_copy<F>(
    options: &StagedCopyOptions,
    cancel: Option<Arc<AtomicBool>>,
    mut on_progress: F,
) -> Result<StagedCopySummary>
where
    F: FnMut(PipelineProgress) + Send,
{
    let src = options.source_root.as_path();
    let out = options.output_root.as_path();

    if !export_pdb_path(src).is_file() {
        return Err(Error::NotARekordboxExport(src.display().to_string()));
    }

    let scope = resolve_convert_scope(src, options.playlist_id)?;
    if scope.convertible_paths.is_empty() {
        return Err(Error::Message(
            "no convertible tracks in the selected scope".into(),
        ));
    }

    // Snapshot counts for verification.
    let src_db = Database::from_file(export_pdb_path(src))
        .map_err(|e| Error::Database(e.to_string()))?;
    let expected_tracks = src_db.tracks.len();
    let expected_entries = src_db.playlist_entries.len();

    // Map scoped sources → track rows for rewrite.
    let scope_paths: Vec<PathBuf> = scope.convertible_paths.iter().map(PathBuf::from).collect();
    let mut track_rows: Vec<&rekordbox_pdb::Track> = Vec::new();
    for t in &src_db.tracks {
        let abs = resolve_usb_path(src, t.file_path());
        if scope_paths.iter().any(|p| paths_loose_eq(p, &abs)) {
            track_rows.push(t);
        }
    }

    let bitrate_kbps = crate::settings::parse_bitrate_kbps(&options.bitrate).unwrap_or(320);

    emit(
        &mut on_progress,
        "prepare",
        0,
        1,
        "Creating output directory",
    );
    if out.exists() {
        return Err(Error::Message(format!(
            "output already exists: {}",
            out.display()
        )));
    }
    fs::create_dir_all(out)?;

    // 1. Copy PIONEER/
    emit(
        &mut on_progress,
        "copy-pioneer",
        0,
        1,
        "Copying PIONEER database and analysis",
    );
    let src_pioneer = src.join("PIONEER");
    let out_pioneer = out.join("PIONEER");
    if src_pioneer.is_dir() {
        copy_dir_recursive(&src_pioneer, &out_pioneer, &cancel)?;
    } else {
        return Err(Error::Message("PIONEER folder missing on source USB".into()));
    }
    check_cancel(&cancel)?;

    // 2. Copy scoped convertible sources
    let total_sources = scope.convertible_paths.len() as u32;
    let mut copied = 0u32;
    for src_path in &scope.convertible_paths {
        check_cancel(&cancel)?;
        let src_file = PathBuf::from(src_path);
        let rel = relative_contents_path(src, &src_file);
        let dest_file = out.join(&rel);
        if let Some(parent) = dest_file.parent() {
            fs::create_dir_all(parent)?;
        }
        if src_file.is_file() {
            fs::copy(&src_file, &dest_file)?;
        }
        copied += 1;
        emit(
            &mut on_progress,
            "copy-audio",
            copied,
            total_sources,
            format!("Copied {}", file_name(&src_file)),
        );
    }

    // 3. Convert each source on the copy (prefer cache).
    let contents_out = {
        let c = out.join("Contents");
        if c.is_dir() {
            Some(c)
        } else {
            None
        }
    };
    let cache = CacheManager::new(&options.cache_root);
    cache.ensure_root()?;

    let convert_opts = ConvertOptions {
        ffmpeg_path: options.ffmpeg_path.clone(),
        cache_root: options.cache_root.clone(),
        bitrate: options.bitrate.clone(),
        workers: options.workers.max(1),
        contents_root: contents_out.clone(),
    };

    let out_sources: Vec<PathBuf> = scope
        .convertible_paths
        .iter()
        .map(|p| out.join(relative_contents_path(src, Path::new(p))))
        .filter(|p| p.is_file())
        .collect();

    let mut cache_hits = 0u32;
    let mut converted = 0u32;
    let mut errors = Vec::new();
    let done = AtomicU32::new(0);
    let total = out_sources.len() as u32;

    // Sequential for simpler path pairing with DB rewrite; conversion itself
    // uses the cache so repeats are cheap. Parallel convert_to_cache already
    // exists for cache warm-up (Phase 3).
    for source in &out_sources {
        check_cancel(&cancel)?;
        let name = file_name(source);
        // Cache was warmed from the source USB paths in Phase 3 — look up by source.
        let src_file = src.join(relative_contents_path(out, source));
        let cache_source = if src_file.is_file() {
            src_file.as_path()
        } else {
            source.as_path()
        };
        let src_contents = {
            let c = src.join("Contents");
            if c.is_dir() {
                Some(c)
            } else {
                None
            }
        };
        match place_mp3_beside_source(
            source,
            cache_source,
            src_contents.as_deref(),
            &cache,
            &convert_opts,
        ) {
            Ok(ConvertPlace::CacheHit) => cache_hits += 1,
            Ok(ConvertPlace::Converted) => converted += 1,
            Err(e) => errors.push(format!("{name}: {e}")),
        }
        let n = done.fetch_add(1, Ordering::Relaxed) + 1;
        emit(
            &mut on_progress,
            "convert",
            n,
            total,
            format!("Converted {name}"),
        );
    }

    // 4. Build rewrite list from DB paths.
    emit(&mut on_progress, "rewrite-pdb", 0, 1, "Rewriting export.pdb");
    let mut rewrites = Vec::new();
    let mut rewritten_ids = Vec::new();
    for track in &track_rows {
        let src_file = resolve_usb_path(src, track.file_path());
        let out_file = out.join(relative_contents_path(src, &src_file));
        let out_mp3 = crate::formats::to_mp3_path(&out_file);
        if !out_mp3.is_file() {
            errors.push(format!(
                "MP3 missing after convert: {}",
                out_mp3.display()
            ));
            continue;
        }
        let file_size = out_mp3.metadata().map(|m| m.len() as u32).unwrap_or(0);
        let new_db_path = crate::formats::db_path_to_mp3(track.file_path());
        let new_filename = PathBuf::from(&new_db_path)
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| new_db_path.clone());
        let old_analyze = track.analyze_path().to_string();
        let new_analyze = analyze_path_for(&old_analyze, &new_db_path);
        let (sample_rate, sample_depth) = probe_mp3(&options.ffmpeg_path, &out_mp3);

        rewrites.push(TrackAudioRewrite {
            track_id: track.id,
            old_file_path: track.file_path().to_string(),
            new_file_path: new_db_path,
            new_filename,
            bitrate: bitrate_kbps,
            file_size,
            old_analyze_path: old_analyze,
            new_analyze_path: new_analyze,
            sample_rate,
            sample_depth,
        });
        rewritten_ids.push(track.id);
    }

    // Move ANLZ next to the MP3 hash path before the database points at it.
    emit(
        &mut on_progress,
        "rewrite-anlz",
        0,
        1,
        "Relocating ANLZ files and rewriting PPTH",
    );
    let mut anlz_updated = 0u32;
    let mut relocated = Vec::new();
    for r in &rewrites {
        match retarget_anlz(out, r, &mut relocated) {
            Ok(n) => anlz_updated += n,
            Err(e) => errors.push(e.to_string()),
        }
    }

    let out_pdb = export_pdb_path(out);
    let pdb_summary = rewrite_track_audio(&out_pdb, &rewrites)?;
    for f in &pdb_summary.failed {
        errors.push(f.clone());
    }

    if let Err(e) = sync_export_library(out, &rewrites) {
        errors.push(e.to_string());
    }

    // 6. Verify
    emit(&mut on_progress, "verify", 0, 1, "Verifying database");
    let verify_problems = verify_rewritten_pdb(
        out,
        &out_pdb,
        expected_tracks,
        expected_entries,
        &rewritten_ids,
    )?;

    // Also ensure exportExt was copied if present.
    if export_ext_pdb_path(src).is_file() && !export_ext_pdb_path(out).is_file() {
        errors.push("exportExt.pdb missing from staged copy".into());
    }

    // 7. Delete sources from the copy (only after verify of MP3s exists).
    let mut sources_removed = 0u32;
    if verify_problems.is_empty() {
        for source in &out_sources {
            let mp3 = crate::formats::to_mp3_path(source);
            if mp3.is_file() {
                let _ = fs::remove_file(source);
                sources_removed += 1;
            }
        }
    }

    emit(
        &mut on_progress,
        "done",
        1,
        1,
        format!("Staged copy ready at {}", out.display()),
    );

    Ok(StagedCopySummary {
        output_root: out.display().to_string(),
        tracks_converted: converted + cache_hits,
        cache_hits,
        pdb_updated: pdb_summary.updated,
        anlz_updated,
        sources_removed,
        verify_problems,
        errors,
    })
}

enum ConvertPlace {
    CacheHit,
    Converted,
}

fn place_mp3_beside_source(
    dest_source: &Path,
    cache_source: &Path,
    cache_contents: Option<&Path>,
    cache: &CacheManager,
    options: &ConvertOptions,
) -> Result<ConvertPlace> {
    let dest_mp3 = crate::formats::to_mp3_path(dest_source);
    if dest_mp3.is_file() && dest_mp3.metadata().map(|m| m.len()).unwrap_or(0) > 0 {
        return Ok(ConvertPlace::CacheHit);
    }

    if let Some(hit) = cache.lookup(cache_source, cache_contents)? {
        fs::copy(&hit, &dest_mp3)?;
        return Ok(ConvertPlace::CacheHit);
    }

    let key = cache::compute_key(cache_source)?;
    let tmp = {
        let stem = dest_mp3
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "track".into());
        dest_mp3.with_file_name(format!("{stem}.partial.mp3"))
    };
    if tmp.exists() {
        let _ = fs::remove_file(&tmp);
    }
    convert::run_ffmpeg(&options.ffmpeg_path, dest_source, &tmp, &options.bitrate)?;
    if !tmp.is_file() || tmp.metadata()?.len() == 0 {
        let _ = fs::remove_file(&tmp);
        return Err(Error::Message("ffmpeg produced empty output".into()));
    }
    fs::rename(&tmp, &dest_mp3).or_else(|_| {
        fs::copy(&tmp, &dest_mp3)?;
        fs::remove_file(&tmp)?;
        Ok::<(), Error>(())
    })?;

    let _ = cache.store_copy(cache_source, cache_contents, &dest_mp3, &key);
    Ok(ConvertPlace::Converted)
}

fn relative_contents_path(usb_root: &Path, abs_file: &Path) -> PathBuf {
    if let Ok(rel) = abs_file.strip_prefix(usb_root) {
        return rel.to_path_buf();
    }
    // Fallback: Contents/Unknown/filename
    PathBuf::from("Contents")
        .join("Unknown")
        .join(abs_file.file_name().unwrap_or_default())
}

fn paths_loose_eq(a: &Path, b: &Path) -> bool {
    let na = a.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    let nb = b.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    na == nb
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| p.display().to_string())
}

fn emit(cb: &mut impl FnMut(PipelineProgress), phase: &str, completed: u32, total: u32, detail: impl Into<String>) {
    cb(PipelineProgress {
        phase: phase.into(),
        completed,
        total,
        detail: detail.into(),
    });
}

fn check_cancel(cancel: &Option<Arc<AtomicBool>>) -> Result<()> {
    if cancel.as_ref().is_some_and(|c| c.load(Ordering::Relaxed)) {
        return Err(Error::Message("cancelled".into()));
    }
    Ok(())
}

fn copy_dir_recursive(
    src: &Path,
    dst: &Path,
    cancel: &Option<Arc<AtomicBool>>,
) -> Result<()> {
    fs::create_dir_all(dst)?;
    for entry in WalkDir::new(src).follow_links(false).into_iter().filter_map(|e| e.ok()) {
        check_cancel(cancel)?;
        let path = entry.path();
        let rel = path.strip_prefix(src).unwrap_or(path);
        let target = dst.join(rel);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target)?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(path, &target)?;
        }
    }
    Ok(())
}

/// Default staged-output parent: `%LOCALAPPDATA%/DrokerBox/Staged`
pub fn default_staged_parent() -> PathBuf {
    crate::app_dirs::app_data_root().join("Staged")
}

// ---------------------------------------------------------------------------
// Phase 5 — in-place USB conversion with backup / rollback
// ---------------------------------------------------------------------------

/// Options for converting non-MP3 audio on the live USB.
#[derive(Debug, Clone)]
pub struct InPlaceOptions {
    pub usb_root: PathBuf,
    pub playlist_id: Option<u32>,
    pub ffmpeg_path: PathBuf,
    pub cache_root: PathBuf,
    pub bitrate: String,
    pub workers: usize,
    /// If true, keep source files on the USB after a successful conversion.
    pub keep_source: bool,
    /// Fail the run if post-rewrite verify finds problems (triggers rollback).
    pub verify_output: bool,
    /// Resolve scope and report only — no USB writes.
    pub dry_run: bool,
}

/// Summary of an in-place conversion.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InPlaceSummary {
    pub usb_root: String,
    pub tracks_converted: u32,
    pub cache_hits: u32,
    pub pdb_updated: u32,
    pub anlz_updated: u32,
    pub sources_removed: u32,
    pub rolled_back: bool,
    pub backup_kept: bool,
    pub verify_problems: Vec<String>,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    /// Paths relative to the USB root that were copied into the backup.
    files: Vec<String>,
}

/// Converts scoped non-MP3 audio on the live USB.
///
/// Crash-safe order:
/// 1. Backup `export.pdb`, `exportExt.pdb`, and affected ANLZ files
/// 2. Write MP3s alongside sources (cache or FFmpeg)
/// 3. Verify every MP3
/// 4. Rewrite DB + ANLZ
/// 5. Re-verify
/// 6. Delete sources (unless `keep_source`)
/// 7. Remove backup on success
///
/// Any failure before step 6 restores the backed-up files.
pub fn convert_usb_inplace<F>(
    options: &InPlaceOptions,
    cancel: Option<Arc<AtomicBool>>,
    mut on_progress: F,
) -> Result<InPlaceSummary>
where
    F: FnMut(PipelineProgress) + Send,
{
    let root = options.usb_root.as_path();
    if !export_pdb_path(root).is_file() {
        return Err(Error::NotARekordboxExport(root.display().to_string()));
    }

    let scope = resolve_convert_scope(root, options.playlist_id)?;
    if scope.convertible_paths.is_empty() {
        return Err(Error::Message(
            "no convertible tracks in the selected scope".into(),
        ));
    }

    let db = Database::from_file(export_pdb_path(root))
        .map_err(|e| Error::Database(e.to_string()))?;
    let expected_tracks = db.tracks.len();
    let expected_entries = db.playlist_entries.len();

    let scope_paths: Vec<PathBuf> = scope.convertible_paths.iter().map(PathBuf::from).collect();
    let mut track_rows: Vec<&rekordbox_pdb::Track> = Vec::new();
    for t in &db.tracks {
        let abs = resolve_usb_path(root, t.file_path());
        if scope_paths.iter().any(|p| paths_loose_eq(p, &abs)) {
            track_rows.push(t);
        }
    }

    let bitrate_kbps = crate::settings::parse_bitrate_kbps(&options.bitrate).unwrap_or(320);

    if options.dry_run {
        emit(
            &mut on_progress,
            "dry-run",
            0,
            1,
            format!(
                "Dry run: would convert {} source(s), rewrite {} track row(s)",
                scope_paths.len(),
                track_rows.len()
            ),
        );
        return Ok(InPlaceSummary {
            usb_root: root.display().to_string(),
            tracks_converted: 0,
            cache_hits: 0,
            pdb_updated: 0,
            anlz_updated: 0,
            sources_removed: 0,
            rolled_back: false,
            backup_kept: false,
            verify_problems: Vec::new(),
            errors: vec![format!(
                "dry run — no changes written ({} source(s) → MP3 @ {})",
                scope_paths.len(),
                options.bitrate
            )],
        });
    }

    let backup_root = root.join(BACKUP_DIR_NAME);
    if backup_root.exists() {
        return Err(Error::Message(format!(
            "backup folder already exists at {} — remove or rename it before converting",
            backup_root.display()
        )));
    }

    // ---- 1. Backup -------------------------------------------------------
    emit(
        &mut on_progress,
        "backup",
        0,
        1,
        "Backing up database and analysis files",
    );
    let manifest = create_backup(root, &backup_root, &track_rows)?;

    let mut errors = Vec::new();
    let mut cache_hits = 0u32;
    let mut converted = 0u32;
    let mut anlz_updated = 0u32;
    let mut relocated: Vec<PathBuf> = Vec::new();
    let mut sources_removed = 0u32;
    let mut pdb_updated = 0u32;
    let mut rolled_back = false;
    let mut backup_kept = true;

    let result = (|| -> Result<()> {
        check_cancel(&cancel)?;

        // ---- 2. Convert MP3s alongside sources ---------------------------
        let cache = CacheManager::new(&options.cache_root);
        cache.ensure_root()?;
        let contents = {
            let c = root.join("Contents");
            if c.is_dir() {
                Some(c)
            } else {
                None
            }
        };
        let convert_opts = ConvertOptions {
            ffmpeg_path: options.ffmpeg_path.clone(),
            cache_root: options.cache_root.clone(),
            bitrate: options.bitrate.clone(),
            workers: options.workers.max(1),
            contents_root: contents.clone(),
        };

        let total = scope_paths.len() as u32;
        let done = AtomicU32::new(0);
        for source in &scope_paths {
            check_cancel(&cancel)?;
            if !source.is_file() {
                errors.push(format!("source missing: {}", source.display()));
                continue;
            }
            let name = file_name(source);
            match place_mp3_beside_source(
                source,
                source,
                contents.as_deref(),
                &cache,
                &convert_opts,
            ) {
                Ok(ConvertPlace::CacheHit) => cache_hits += 1,
                Ok(ConvertPlace::Converted) => converted += 1,
                Err(e) => {
                    errors.push(format!("{name}: {e}"));
                }
            }
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            emit(
                &mut on_progress,
                "convert",
                n,
                total,
                format!("Placed MP3 for {name}"),
            );
        }

        if !errors.is_empty() && converted + cache_hits == 0 {
            return Err(Error::Message(
                "every conversion failed — see errors".into(),
            ));
        }

        // ---- 3. Verify MP3s before touching the DB -----------------------
        emit(&mut on_progress, "verify-mp3", 0, 1, "Verifying MP3 files");
        let mut rewrites = Vec::new();
        let mut rewritten_ids = Vec::new();
        for track in &track_rows {
            let source = resolve_usb_path(root, track.file_path());
            let mp3 = crate::formats::to_mp3_path(&source);
            if !mp3.is_file() || mp3.metadata().map(|m| m.len()).unwrap_or(0) == 0 {
                errors.push(format!(
                    "MP3 missing/empty before DB rewrite: {}",
                    mp3.display()
                ));
                continue;
            }
            let file_size = mp3.metadata().map(|m| m.len() as u32).unwrap_or(0);
            let new_db_path = crate::formats::db_path_to_mp3(track.file_path());
            let new_filename = PathBuf::from(&new_db_path)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| new_db_path.clone());
            let old_analyze = track.analyze_path().to_string();
            let new_analyze = analyze_path_for(&old_analyze, &new_db_path);
            let (sample_rate, sample_depth) = probe_mp3(&options.ffmpeg_path, &mp3);
            rewrites.push(TrackAudioRewrite {
                track_id: track.id,
                old_file_path: track.file_path().to_string(),
                new_file_path: new_db_path,
                new_filename,
                bitrate: bitrate_kbps,
                file_size,
                old_analyze_path: old_analyze,
                new_analyze_path: new_analyze,
                sample_rate,
                sample_depth,
            });
            rewritten_ids.push(track.id);
        }

        if rewrites.is_empty() {
            return Err(Error::Message(
                "no tracks ready for database rewrite".into(),
            ));
        }

        // ---- 4. Relocate ANLZ, then rewrite PDB + Device Library Plus -----
        emit(
            &mut on_progress,
            "rewrite-anlz",
            0,
            1,
            "Relocating ANLZ files and rewriting PPTH",
        );
        for r in &rewrites {
            match retarget_anlz(root, r, &mut relocated) {
                Ok(n) => anlz_updated += n,
                Err(e) => return Err(e),
            }
        }

        emit(
            &mut on_progress,
            "rewrite-pdb",
            0,
            1,
            "Rewriting export.pdb",
        );
        let pdb_summary = rewrite_track_audio(export_pdb_path(root), &rewrites)?;
        pdb_updated = pdb_summary.updated;
        for f in pdb_summary.failed {
            errors.push(f);
        }

        sync_export_library(root, &rewrites)?;

        // ---- 5. Re-verify ------------------------------------------------
        emit(
            &mut on_progress,
            "verify-db",
            0,
            1,
            "Verifying database consistency",
        );
        let verify_problems = verify_rewritten_pdb(
            root,
            export_pdb_path(root),
            expected_tracks,
            expected_entries,
            &rewritten_ids,
        )?;
        if options.verify_output && !verify_problems.is_empty() {
            return Err(Error::Message(format!(
                "verification failed:\n{}",
                verify_problems.join("\n")
            )));
        }
        if !verify_problems.is_empty() {
            for p in verify_problems {
                errors.push(format!("verify (ignored): {p}"));
            }
        }

        // ---- 6. Delete sources (in-memory track rows still point at originals) --
        if !options.keep_source {
            emit(
                &mut on_progress,
                "cleanup",
                0,
                1,
                "Removing source files from USB",
            );
            for source in &scope_paths {
                let mp3 = crate::formats::to_mp3_path(source);
                if source.is_file() && mp3.is_file() {
                    match fs::remove_file(source) {
                        Ok(()) => sources_removed += 1,
                        Err(e) => errors.push(format!(
                            "could not delete {}: {e}",
                            source.display()
                        )),
                    }
                }
            }
        }

        // ---- 7. Remove backup --------------------------------------------
        emit(&mut on_progress, "finalize", 0, 1, "Removing backup");
        let _ = fs::remove_dir_all(&backup_root);
        backup_kept = false;
        let _ = manifest; // written for crash recovery if we crash mid-run

        Ok(())
    })();

    if let Err(ref e) = result {
        emit(
            &mut on_progress,
            "rollback",
            0,
            1,
            format!("Rolling back: {e}"),
        );
        if let Err(rb) = restore_backup(root, &backup_root) {
            errors.push(format!("rollback failed: {rb}"));
        } else {
            rolled_back = true;
            // Keep backup so the user can inspect / recover manually if needed.
            backup_kept = true;
        }
        for dir in &relocated {
            let _ = fs::remove_dir_all(dir);
        }
        errors.push(e.to_string());
    }

    emit(
        &mut on_progress,
        "done",
        1,
        1,
        if rolled_back {
            "Rolled back — USB database restored from backup".to_string()
        } else {
            "USB conversion finished".to_string()
        },
    );

    // Surface verify problems only on success path (already folded into Err).
    Ok(InPlaceSummary {
        usb_root: root.display().to_string(),
        tracks_converted: converted + cache_hits,
        cache_hits,
        pdb_updated,
        anlz_updated,
        sources_removed,
        rolled_back,
        backup_kept,
        verify_problems: Vec::new(),
        errors,
    })
}

fn create_backup(
    usb_root: &Path,
    backup_root: &Path,
    tracks: &[&rekordbox_pdb::Track],
) -> Result<BackupManifest> {
    fs::create_dir_all(backup_root)?;
    let mut files = Vec::new();

    let pdb = export_pdb_path(usb_root);
    backup_one(usb_root, backup_root, &pdb, &mut files)?;

    let ext = export_ext_pdb_path(usb_root);
    if ext.is_file() {
        backup_one(usb_root, backup_root, &ext, &mut files)?;
    }

    let library = crate::export_library::export_library_db_path(usb_root);
    if library.is_file() {
        backup_one(usb_root, backup_root, &library, &mut files)?;
        for side in sqlite_sidecars(&library) {
            if side.is_file() {
                backup_one(usb_root, backup_root, &side, &mut files)?;
            }
        }
    }

    for track in tracks {
        if track.analyze_path().is_empty() {
            continue;
        }
        let anlz_dat = resolve_usb_path(usb_root, track.analyze_path());
        let Some(dir) = anlz_dat.parent() else {
            continue;
        };
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                if name.starts_with("ANLZ") {
                    backup_one(usb_root, backup_root, &p, &mut files)?;
                }
            }
        }
    }

    let manifest = BackupManifest { files };
    let manifest_path = backup_root.join("manifest.json");
    let json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| Error::Message(e.to_string()))?;
    fs::write(manifest_path, json)?;
    Ok(manifest)
}

fn backup_one(
    usb_root: &Path,
    backup_root: &Path,
    abs: &Path,
    files: &mut Vec<String>,
) -> Result<()> {
    if !abs.is_file() {
        return Ok(());
    }
    let rel = abs
        .strip_prefix(usb_root)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| PathBuf::from(abs.file_name().unwrap_or_default()));
    let dest = backup_root.join(&rel);
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(abs, &dest)?;
    files.push(rel.to_string_lossy().replace('\\', "/"));
    Ok(())
}

fn restore_backup(usb_root: &Path, backup_root: &Path) -> Result<()> {
    let manifest_path = backup_root.join("manifest.json");
    let files: Vec<String> = if manifest_path.is_file() {
        let data = fs::read_to_string(&manifest_path)?;
        let m: BackupManifest =
            serde_json::from_str(&data).map_err(|e| Error::Message(e.to_string()))?;
        m.files
    } else {
        // Fallback: restore everything under backup.
        let mut all = Vec::new();
        for entry in WalkDir::new(backup_root)
            .follow_links(false)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file()
                && entry.file_name() != "manifest.json"
            {
                if let Ok(rel) = entry.path().strip_prefix(backup_root) {
                    all.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        all
    };

    for rel in files {
        let src = backup_root.join(&rel);
        let dest = usb_root.join(&rel);
        if !src.is_file() {
            continue;
        }
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&src, &dest)?;
    }
    Ok(())
}

fn analyze_path_for(old_analyze: &str, new_audio_path: &str) -> String {
    if old_analyze.is_empty() {
        String::new()
    } else {
        anlz::analyze_db_path(new_audio_path)
    }
}

fn probe_mp3(ffmpeg: &Path, mp3: &Path) -> (Option<u32>, Option<u16>) {
    let bin = convert::ffprobe_path_for(ffmpeg);
    if !bin.is_file() {
        return (None, None);
    }
    match convert::probe_audio(&bin, mp3) {
        Ok(info) => (Some(info.sample_rate), info.sample_depth),
        Err(_) => (None, None),
    }
}

/// Moves ANLZ files onto the hash path of the new audio file and rewrites PPTH.
/// Returns how many analysis files had their PPTH updated.
fn retarget_anlz(
    root: &Path,
    rewrite: &TrackAudioRewrite,
    relocated: &mut Vec<PathBuf>,
) -> Result<u32> {
    if rewrite.old_analyze_path.is_empty() {
        return Ok(0);
    }
    let new_analyze = if rewrite.new_analyze_path.is_empty() {
        anlz::analyze_db_path(&rewrite.new_file_path)
    } else {
        rewrite.new_analyze_path.clone()
    };
    let src_file = resolve_usb_path(root, &rewrite.old_analyze_path);
    let src_dir = src_file.parent().ok_or_else(|| {
        Error::Message(format!(
            "analyze path has no directory: {}",
            rewrite.old_analyze_path
        ))
    })?;
    let dest_file = resolve_usb_path(root, &new_analyze);
    let dest_dir = dest_file.parent().ok_or_else(|| {
        Error::Message(format!("analyze path has no directory: {new_analyze}"))
    })?;

    if !anlz::same_export_path(&rewrite.old_analyze_path, &new_analyze) {
        relocated.push(dest_dir.to_path_buf());
        anlz::relocate_anlz_dir(src_dir, dest_dir)?;
    }
    anlz::rewrite_dir_ppath(dest_dir, &rewrite.new_file_path)
}

fn sync_export_library(root: &Path, rewrites: &[TrackAudioRewrite]) -> Result<()> {
    let updates: Vec<crate::export_library::LibraryTrackUpdate> = rewrites
        .iter()
        .map(|r| crate::export_library::LibraryTrackUpdate {
            old_path: r.old_file_path.clone(),
            new_path: r.new_file_path.clone(),
            file_name: r.new_filename.clone(),
            file_size: r.file_size,
            bitrate: r.bitrate,
            sample_rate: r.sample_rate,
            sample_depth: r.sample_depth,
            analyze_path: r.new_analyze_path.clone(),
        })
        .collect();
    let summary = crate::export_library::update_tracks(root, &updates)?;
    if summary.missing.is_empty() {
        return Ok(());
    }
    Err(Error::Message(format!(
        "exportLibrary.db missing content rows: {}",
        summary.missing.join(", ")
    )))
}

fn sqlite_sidecars(db: &Path) -> [PathBuf; 2] {
    let name = db
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent = db.parent().unwrap_or_else(|| Path::new(""));
    [
        parent.join(format!("{name}-wal")),
        parent.join(format!("{name}-shm")),
    ]
}

#[cfg(test)]
mod tests {
    #[test]
    fn db_path_extension_swap() {
        assert_eq!(
            crate::formats::db_path_to_mp3("/Contents/A/x.flac"),
            "/Contents/A/x.mp3"
        );
        assert_eq!(
            crate::formats::db_path_to_mp3("/Contents/A/x.FLAC"),
            "/Contents/A/x.mp3"
        );
        assert_eq!(
            crate::formats::db_path_to_mp3("/Contents/A/x.wav"),
            "/Contents/A/x.mp3"
        );
    }
}
