//! Removable-drive detection.
//!
//! Enumerates candidate USB drives on Windows (via `GetLogicalDrives` /
//! `GetDriveTypeW`) so the UI can let the user pick which stick to convert.
//!
//! Non-Windows builds return an empty list; the architecture stays portable
//! even though Windows is the first target.

use serde::Serialize;
use std::path::PathBuf;

/// A removable (or otherwise candidate) drive that might hold a Rekordbox export.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DriveInfo {
    /// Root path, e.g. `E:\` on Windows.
    pub root: String,
    /// Drive letter without trailing slash, e.g. `E:`.
    pub label: String,
    /// Windows drive type name (`Removable`, `Fixed`, …).
    pub drive_type: String,
    /// Whether `PIONEER/rekordbox/export.pdb` exists on this drive.
    pub is_rekordbox_export: bool,
}

/// Lists drives that are plausible Rekordbox USB targets.
///
/// On Windows this returns every removable drive, plus any fixed drive that
/// already has a Rekordbox export layout (useful when testing from a folder
/// copy mounted as a fixed volume).
pub fn list_candidate_drives() -> Vec<DriveInfo> {
    #[cfg(windows)]
    {
        list_candidate_drives_windows()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(windows)]
fn list_candidate_drives_windows() -> Vec<DriveInfo> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows::Win32::System::WindowsProgramming::{DRIVE_FIXED, DRIVE_REMOVABLE};

    let mask = unsafe { GetLogicalDrives() };
    let mut drives = Vec::new();

    for i in 0..26u32 {
        if mask & (1 << i) == 0 {
            continue;
        }
        let letter = (b'A' + i as u8) as char;
        let root = format!("{letter}:\\");
        let root_wide: Vec<u16> = root.encode_utf16().chain(std::iter::once(0)).collect();
        let drive_type_code = unsafe { GetDriveTypeW(PCWSTR(root_wide.as_ptr())) };

        let drive_type = match drive_type_code {
            DRIVE_REMOVABLE => "Removable",
            DRIVE_FIXED => "Fixed",
            _ => continue,
        };

        let is_rekordbox = is_rekordbox_export_root(PathBuf::from(&root).as_path());
        // Include all removable drives; fixed only if they already look like an export
        // (handy for local test copies).
        if drive_type_code != DRIVE_REMOVABLE && !is_rekordbox {
            continue;
        }

        let volume_label = volume_label(&root_wide).unwrap_or_default();
        let label = if volume_label.is_empty() {
            format!("{letter}:")
        } else {
            format!("{letter}: ({volume_label})")
        };

        drives.push(DriveInfo {
            root,
            label,
            drive_type: drive_type.to_string(),
            is_rekordbox_export: is_rekordbox,
        });
    }

    drives
}

#[cfg(windows)]
fn volume_label(root_wide: &[u16]) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::GetVolumeInformationW;

    let mut name_buf = [0u16; 256];
    let ok = unsafe {
        GetVolumeInformationW(
            PCWSTR(root_wide.as_ptr()),
            Some(&mut name_buf),
            None,
            None,
            None,
            None,
        )
    };
    if ok.is_err() {
        return None;
    }
    let end = name_buf.iter().position(|&c| c == 0).unwrap_or(name_buf.len());
    let label = String::from_utf16_lossy(&name_buf[..end]);
    if label.is_empty() {
        None
    } else {
        Some(label)
    }
}

/// Returns true when `<root>/PIONEER/rekordbox/export.pdb` exists.
pub fn is_rekordbox_export_root(root: &std::path::Path) -> bool {
    root.join("PIONEER")
        .join("rekordbox")
        .join("export.pdb")
        .is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn detects_export_layout() {
        let dir = tempfile_dir();
        assert!(!is_rekordbox_export_root(&dir));
        fs::create_dir_all(dir.join("PIONEER/rekordbox")).unwrap();
        fs::write(dir.join("PIONEER/rekordbox/export.pdb"), b"x").unwrap();
        assert!(is_rekordbox_export_root(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    fn tempfile_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rbusb-usb-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
