//! ANLZ analysis-file handling (`PIONEER/USBANLZ/**/ANLZ*.DAT|.EXT|.2EX`).
//!
//! Each analysis file carries a `PPTH` tag: the UTF-16BE, NUL-terminated
//! path of the audio file the analysis belongs to. When a FLAC is replaced
//! by an MP3 the `PPTH` payload must be rewritten and the tag length and
//! file section sizes recomputed. All other tags (beatgrid, waveforms, cue
//! lists) are preserved byte-for-byte.
//!
//! Format reference:
//! <https://djl-analysis.deepsymmetry.org/rekordbox-export-analysis/anlz.html>
//!
//! Implemented in Phases 2 and 4.
