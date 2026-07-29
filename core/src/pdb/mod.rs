//! Pioneer DeviceSQL database (`export.pdb` / `exportExt.pdb`) access.
//!
//! Reading uses [`rekordbox-pdb`](https://crates.io/crates/rekordbox-pdb) (DeviceSQL
//! format documented in its `FORMAT.md`, derived from
//! [Deep Symmetry](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html)).
//! [`roundtrip`] records the comparison against `rekordcrate`.
//!
//! Surgical rewriting (Phase 4) goes through `rekordbox_pdb::PdbEditor`.

mod inspect;
mod playlists;
mod rewrite;
mod roundtrip;

pub use inspect::{
    inspect_export, ExportInspect, InspectPlaylist, InspectSummary, InspectTrack, MyTagInfo,
};
pub use playlists::{
    list_playlists, resolve_convert_scope, resolve_usb_path, ConvertScope, PlaylistOption,
};
pub use rewrite::{
    rewrite_track_audio, verify_rewritten_pdb, PdbRewriteSummary, TrackAudioRewrite,
};
pub use roundtrip::{evaluate_roundtrip, RoundtripEvaluation, RoundtripLibraryResult};
