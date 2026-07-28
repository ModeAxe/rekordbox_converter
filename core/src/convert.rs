//! FFmpeg wrapper and conversion queue.
//!
//! Spawns the bundled `ffmpeg` sidecar to transcode FLAC → MP3 (default
//! 320 kbps CBR, `-map_metadata 0 -id3v2_version 3`, artwork stream copied)
//! and runs a bounded multi-threaded worker pool with per-track progress
//! reporting.
//!
//! The path to the ffmpeg/ffprobe executables is injected by the caller
//! (the Tauri shell resolves the sidecar location) so this module stays
//! free of Tauri dependencies.
//!
//! Implemented in Phase 3.
