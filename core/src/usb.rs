//! Removable-drive detection.
//!
//! Enumerates candidate USB drives on Windows (via `GetLogicalDrives` /
//! `GetDriveTypeW`) and, later, watches for insertion/removal so the UI can
//! refresh automatically.
//!
//! Implemented in Phase 1.
