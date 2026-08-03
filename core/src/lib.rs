//! # rbusb-core
//!
//! Core engine for the Rekordbox USB non-MP3→MP3 converter.
//!
//! This crate contains all business logic and is deliberately free of any
//! UI-framework dependency (no Tauri imports). The Tauri shell in
//! `src-tauri` is a thin adapter that exposes these modules as commands
//! and events.
//!
//! ## Module map
//!
//! | Module       | Responsibility                                                        |
//! |--------------|-----------------------------------------------------------------------|
//! | [`usb`]      | Enumerate removable drives and watch for insertion/removal (Windows)  |
//! | [`scanner`]  | Validate a drive as a Rekordbox export and inventory its audio files  |
//! | [`formats`]  | Which extensions are convertible vs MP3 passthrough                   |
//! | [`pdb`]      | Read and surgically rewrite `export.pdb` / `exportExt.pdb`            |
//! | [`anlz`]     | Read and rewrite ANLZ analysis files (`.DAT`/`.EXT`/`.2EX`, PPTH tag) |
//! | [`convert`]  | FFmpeg wrapper and multi-threaded conversion queue                    |
//! | [`cache`]    | Converted-MP3 cache keyed by source file identity                     |
//! | [`settings`] | Persistent user options (bitrate, workers, dry-run, …)                |
//! | [`pipeline`] | Transactional orchestration: backup, convert, rewrite, verify, rollback |

pub mod anlz;
pub mod cache;
pub mod convert;
pub mod error;
pub mod formats;
pub mod pdb;
pub mod pipeline;
pub mod scanner;
pub mod settings;
pub mod usb;

pub use error::{Error, Result};
