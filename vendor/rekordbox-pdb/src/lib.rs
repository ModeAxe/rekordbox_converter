//! Read and write the rekordbox `export.pdb` / `exportExt.pdb` databases that
//! rekordbox writes to `PIONEER/rekordbox/` on exported USB media.
//!
//! The on-disk format is a custom page-based store ("DeviceSQL"): a file
//! header with a table directory, fixed-size 4096-byte pages, a row heap that
//! grows up from the page header, and a row-offset directory that grows down
//! from the page end. See `FORMAT.md` for the full byte-level spec.
//!
//! This is a port of the Python `rekordbox_pdb` library; behaviour and the
//! verified fixture values match it exactly.
//!
//! ```no_run
//! use rekordbox_pdb::Database;
//! let db = Database::from_file("PIONEER/rekordbox/export.pdb").unwrap();
//! for track in &db.tracks {
//!     println!("{} {:.2} {}", track.title(), track.tempo as f64 / 100.0, track.file_path());
//! }
//! ```

use std::fmt;

pub mod edit;
pub mod ext;
pub mod pdb;

pub use edit::{NewTrack, PdbEditor};
pub use ext::{ExtDatabase, ExtTableType, Tag};
pub use pdb::{
    Album, Artist, Artwork, Color, Column, Database, Genre, HistoryEntry, HistoryPlaylist, Key,
    Label, PlaylistEntry, PlaylistTreeNode, TableType, Track,
};

/// Size of a page header, in bytes. The row heap starts here.
pub(crate) const PAGE_HEADER_SIZE: usize = 0x28;
/// Bytes per row-offset group (16 u16 offsets + u16 presence + u16 written).
pub(crate) const ROW_GROUP_SIZE: usize = 0x24;

/// Errors returned when reading or editing a database.
#[derive(Debug)]
pub enum PdbError {
    Io(std::io::Error),
    /// A DeviceSQL string had an unrecognised kind byte.
    UnknownString {
        kind: u8,
        offset: usize,
    },
    /// No table of the requested type exists in the file.
    NoTable(u32),
    /// `set_track_field` was given a name that is not an editable fixed field.
    UnknownField(String),
    /// No track with the given id.
    TrackNotFound(u32),
    /// No playlist with the given id (or the id is a folder).
    PlaylistNotFound(u32),
    /// A row's allocation would not fit in a single page.
    RowTooLarge {
        alloc: usize,
        max: usize,
    },
    /// A numeric field value does not fit in its on-disk width.
    ValueOutOfRange {
        field: String,
        value: u64,
        size: usize,
    },
    /// A page would exceed the 511-slot count encoding limit.
    TooManySlots,
    /// A string is too long for the DeviceSQL encoding.
    StringTooLong,
}

impl fmt::Display for PdbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PdbError::Io(e) => write!(f, "io error: {e}"),
            PdbError::UnknownString { kind, offset } => {
                write!(
                    f,
                    "unknown DeviceSQL string kind {kind:#04x} at offset {offset:#x}"
                )
            }
            PdbError::NoTable(t) => write!(f, "no table of type {t}"),
            PdbError::UnknownField(name) => write!(f, "unknown or non-fixed track field {name:?}"),
            PdbError::TrackNotFound(id) => write!(f, "no track with id {id}"),
            PdbError::PlaylistNotFound(id) => write!(f, "no playlist with id {id}"),
            PdbError::RowTooLarge { alloc, max } => {
                write!(f, "row allocation {alloc} exceeds page capacity {max}")
            }
            PdbError::ValueOutOfRange { field, value, size } => {
                write!(f, "{field}={value} does not fit in {size} byte(s)")
            }
            PdbError::TooManySlots => write!(f, "slot count encoding caps at 511 per page"),
            PdbError::StringTooLong => write!(f, "string too long for DeviceSQL encoding"),
        }
    }
}

impl std::error::Error for PdbError {}

impl From<std::io::Error> for PdbError {
    fn from(e: std::io::Error) -> Self {
        PdbError::Io(e)
    }
}

pub type Result<T> = std::result::Result<T, PdbError>;

// ---------------------------------------------------------------------------
// Shared low-level helpers
// ---------------------------------------------------------------------------

#[inline]
pub(crate) fn u16le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
pub(crate) fn u32le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Decode a DeviceSQL string starting at `off`.
///
/// Offsets in real exports are not always perfectly aligned on the kind byte
/// (e.g. unknown track slots pointing at payload `0x32` instead of header `0x05`
/// for `"2"`). We therefore scan a few bytes backward before giving up.
pub(crate) fn decode_string(buf: &[u8], off: usize) -> Result<String> {
    if off >= buf.len() {
        return Ok(String::new());
    }
    for try_off in (off.saturating_sub(4)..=off).rev() {
        if let Some(s) = try_decode_string_at(buf, try_off) {
            return Ok(s);
        }
    }
    Ok(String::new())
}

/// Returns `Some` when `off` looks like a valid DeviceSQL string start.
fn try_decode_string_at(buf: &[u8], off: usize) -> Option<String> {
    if off >= buf.len() {
        return Some(String::new());
    }
    let b0 = buf[off];
    if b0 == 0 {
        return Some(String::new());
    }
    if b0 & 1 != 0 {
        let len = ((b0 >> 1) as usize).saturating_sub(1);
        let end = off + 1 + len;
        if end > buf.len() {
            return None;
        }
        let bytes = &buf[off + 1..end];
        return Some(String::from_utf8_lossy(bytes).into_owned());
    }
    if off + 4 > buf.len() {
        return None;
    }
    let total = u16le(buf, off + 1) as usize;
    if total < 4 || off + total > buf.len() {
        return None;
    }
    let payload = &buf[off + 4..off + total];
    match b0 {
        0x40 => Some(String::from_utf8_lossy(payload).into_owned()),
        0x90 => {
            if payload.first() == Some(&0x03) {
                let ascii = &payload[1..];
                let end = ascii.iter().position(|&b| b == 0).unwrap_or(ascii.len());
                return Some(String::from_utf8_lossy(&ascii[..end]).into_owned());
            }
            let units: Vec<u16> = payload
                .chunks_exact(2)
                .map(|c| u16::from_le_bytes([c[0], c[1]]))
                .take_while(|&u| u != 0)
                .collect();
            Some(String::from_utf16_lossy(&units))
        }
        _ => None,
    }
}

/// Encode a string the way rekordbox does: short ASCII up to 126 chars, then
/// long ASCII (`0x40`), and UTF-16LE (`0x90`) for any non-ASCII text.
pub(crate) fn encode_string(text: &str) -> Result<Vec<u8>> {
    if text.is_ascii() {
        let bytes = text.as_bytes();
        if bytes.len() <= 126 {
            let mut v = Vec::with_capacity(bytes.len() + 1);
            v.push(((bytes.len() + 1) * 2 + 1) as u8);
            v.extend_from_slice(bytes);
            return Ok(v);
        }
        if bytes.len() > 32763 {
            return Err(PdbError::StringTooLong);
        }
        let mut v = Vec::with_capacity(bytes.len() + 4);
        v.push(0x40);
        v.extend_from_slice(&((bytes.len() + 4) as u16).to_le_bytes());
        v.push(0);
        v.extend_from_slice(bytes);
        return Ok(v);
    }
    let payload: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    if payload.len() + 4 > 0x7fff {
        return Err(PdbError::StringTooLong);
    }
    let mut v = Vec::with_capacity(payload.len() + 4);
    v.push(0x90);
    v.extend_from_slice(&((payload.len() + 4) as u16).to_le_bytes());
    v.push(0);
    v.extend_from_slice(&payload);
    Ok(v)
}

#[inline]
pub(crate) fn align4(n: usize) -> usize {
    (n + 3) & !3
}

/// Offsets (within `page`) of every present row on a data page. Empty for
/// index pages (flag bit 0x40). `page` is a full page slice of length
/// `page_size`.
pub(crate) fn row_offsets(page: &[u8], page_size: usize) -> Vec<usize> {
    if page[0x1b] & 0x40 != 0 {
        return Vec::new();
    }
    // Slot count: low byte at 0x18 plus an overflow bit at 0x19.
    let num_rows = page[0x18] as usize + 0x100 * (page[0x19] & 1) as usize;
    if num_rows == 0 {
        return Vec::new();
    }
    let mut offsets = Vec::with_capacity(num_rows);
    let num_groups = (num_rows - 1) / 16 + 1;
    for group in 0..num_groups {
        let base = page_size - group * ROW_GROUP_SIZE;
        let present = u16le(page, base - 4);
        let in_group = if group < num_groups - 1 {
            16
        } else {
            (num_rows - 1) % 16 + 1
        };
        for i in 0..in_group {
            if (present >> i) & 1 != 0 {
                offsets.push(PAGE_HEADER_SIZE + u16le(page, base - 6 - 2 * i) as usize);
            }
        }
    }
    offsets
}

#[cfg(test)]
mod tests {
    use super::decode_string;

    #[test]
    fn misaligned_offset_finds_short_ascii_header() {
        // Kind 0x05 ("2") with payload 0x32 — offset pointing at payload byte.
        let buf = [0x05, b'2'];
        assert_eq!(decode_string(&buf, 1).unwrap(), "2");
    }

    #[test]
    fn zero_byte_is_empty() {
        assert_eq!(decode_string(&[0x00, 0x05, b'a'], 0).unwrap(), "");
    }
}
