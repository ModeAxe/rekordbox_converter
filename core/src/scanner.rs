//! Rekordbox export scanning.
//!
//! Validates that a drive root contains a Rekordbox device export
//! (`PIONEER/rekordbox/export.pdb`), walks the `Contents/` directory and
//! produces an inventory of audio files by format (FLAC / MP3 / other) with
//! size totals used for the conversion-time estimate.
//!
//! Implemented in Phase 1.
