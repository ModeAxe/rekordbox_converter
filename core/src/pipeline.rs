//! Transactional pipeline orchestration.
//!
//! Ordering is chosen so the USB can never be left in a mixed state:
//!
//! 1. Backup `export.pdb`, `exportExt.pdb` and affected ANLZ files
//! 2. Convert (or copy from cache) every FLAC to MP3 alongside the FLAC
//! 3. Verify every MP3 (exists, non-zero, decodable)
//! 4. Rewrite database + ANLZ path tags
//! 5. Re-verify database consistency
//! 6. Delete FLACs (only after everything above succeeded)
//! 7. Remove backups
//!
//! Any failure before step 6 triggers automatic rollback from the backups.
//!
//! Implemented in Phases 4 and 5.
