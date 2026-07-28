//! Pioneer DeviceSQL database (`export.pdb` / `exportExt.pdb`) access.
//!
//! Reading uses [`rekordbox-pdb`](https://crates.io/crates/rekordbox-pdb) (DeviceSQL
//! format documented in its `FORMAT.md`, derived from
//! [Deep Symmetry](https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/exports.html)).
//! Phase 2 selects this library after a byte-identical no-op round-trip test;
//! [`roundtrip`] records the comparison against `rekordcrate`.
//!
//! Surgical rewriting (Phase 4) goes through `rekordbox_pdb::PdbEditor`.

mod inspect;
mod roundtrip;

pub use inspect::{
    inspect_export, ExportInspect, InspectPlaylist, InspectSummary, InspectTrack, MyTagInfo,
};
pub use roundtrip::{evaluate_roundtrip, RoundtripEvaluation, RoundtripLibraryResult};
