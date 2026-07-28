//! Pioneer DeviceSQL database (`export.pdb` / `exportExt.pdb`) access.
//!
//! Reading and *surgical* rewriting of track rows: file path, filename,
//! bitrate and file size only. Track IDs are never changed, which keeps
//! playlists, hot cues, memory cues, beatgrids, My Tags, ratings and colour
//! tags intact by construction.
//!
//! Library choice (rekordcrate vs rekordbox-pdb vs minimal page editor) is
//! decided in Phase 2 based on a byte-identical round-trip test against a
//! real export. Format reference:
//! <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html>
//!
//! Implemented in Phases 2 and 4.
