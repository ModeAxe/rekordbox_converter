//! Writing support for `export.pdb`.
//!
//! Follows rekordbox's own incremental save behaviour: fields are patched in
//! place and rows are appended to page heaps with full row-directory / page
//! bookkeeping, rather than rewriting the file.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::pdb::{Database, TableType};
use crate::{align4, encode_string, u16le, u32le, PdbError, Result, PAGE_HEADER_SIZE};

/// Fixed-width track field editable in place: name -> (row offset, size).
fn field_spec(field: &str) -> Option<(usize, usize)> {
    Some(match field {
        "sample_rate" => (0x08, 4),
        "file_size" => (0x10, 4),
        "artwork_id" => (0x1c, 4),
        "key_id" => (0x20, 4),
        "original_artist_id" => (0x24, 4),
        "label_id" => (0x28, 4),
        "remixer_id" => (0x2c, 4),
        "bitrate" => (0x30, 4),
        "track_number" => (0x34, 4),
        "tempo" => (0x38, 4),
        "genre_id" => (0x3c, 4),
        "album_id" => (0x40, 4),
        "artist_id" => (0x44, 4),
        "disc_number" => (0x4c, 2),
        "play_count" => (0x4e, 2),
        "year" => (0x50, 2),
        "sample_depth" => (0x52, 2),
        "duration" => (0x54, 2),
        "color_id" => (0x58, 1),
        "rating" => (0x59, 1),
        "file_type" => (0x5a, 2),
        _ => return None,
    })
}

fn file_type_code(path: &str) -> u16 {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "mp3" => 0x01,
        "m4a" => 0x04,
        "wav" => 0x0b,
        "aiff" | "aif" => 0x0c,
        _ => 0x01,
    }
}

fn check_range(field: &str, value: u64, size: usize) -> Result<()> {
    if value >= 1u64 << (8 * size) {
        return Err(PdbError::ValueOutOfRange {
            field: field.to_string(),
            value,
            size,
        });
    }
    Ok(())
}

/// Metadata for a track to add. Use `NewTrack::new(title, path)` then set
/// fields, or construct with `..Default::default()`.
#[derive(Clone, Debug)]
pub struct NewTrack {
    pub title: String,
    pub file_path: String,
    pub filename: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub key: Option<String>,
    pub label: Option<String>,
    pub comment: String,
    pub mix_name: String,
    pub release_date: String,
    pub analyze_path: String,
    /// Defaults to empty; set to a `YYYY-MM-DD` string if desired.
    pub date_added: String,
    pub tempo: u32,
    pub duration: u16,
    pub year: u16,
    pub bitrate: u32,
    pub sample_rate: u32,
    pub sample_depth: u16,
    pub file_size: u32,
    pub track_number: u32,
    pub disc_number: u16,
    pub play_count: u16,
    pub rating: u8,
    pub color_id: u8,
    pub artwork_id: u32,
}

impl Default for NewTrack {
    fn default() -> Self {
        NewTrack {
            title: String::new(),
            file_path: String::new(),
            filename: None,
            artist: None,
            album: None,
            genre: None,
            key: None,
            label: None,
            comment: String::new(),
            mix_name: String::new(),
            release_date: String::new(),
            analyze_path: String::new(),
            date_added: String::new(),
            tempo: 0,
            duration: 0,
            year: 0,
            bitrate: 0,
            sample_rate: 44100,
            sample_depth: 16,
            file_size: 0,
            track_number: 0,
            disc_number: 0,
            play_count: 0,
            rating: 0,
            color_id: 0,
            artwork_id: 0,
        }
    }
}

impl NewTrack {
    pub fn new(title: impl Into<String>, file_path: impl Into<String>) -> NewTrack {
        NewTrack {
            title: title.into(),
            file_path: file_path.into(),
            ..Default::default()
        }
    }
}

/// Edits an `export.pdb` in memory; write the result with `save`.
pub struct PdbEditor {
    buf: Vec<u8>,
    page_size: usize,
    orig_slots: HashMap<u32, u32>,
    appends: HashMap<u32, u32>,
    new_gen: HashMap<u32, u32>,
    touched_pages: HashMap<u32, HashSet<u32>>,
    structural: bool,
    final_sequence: Option<u32>,
}

impl PdbEditor {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<PdbEditor> {
        Ok(PdbEditor::from_bytes(std::fs::read(path)?))
    }

    pub fn from_bytes(data: Vec<u8>) -> PdbEditor {
        let page_size = u32le(&data, 4) as usize;
        PdbEditor {
            buf: data,
            page_size,
            orig_slots: HashMap::new(),
            appends: HashMap::new(),
            new_gen: HashMap::new(),
            touched_pages: HashMap::new(),
            structural: false,
            final_sequence: None,
        }
    }

    pub fn to_bytes(&mut self) -> Vec<u8> {
        self.finalize();
        self.buf.clone()
    }

    pub fn save<P: AsRef<Path>>(&mut self, path: P) -> Result<()> {
        let bytes = self.to_bytes();
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Parse the current buffer state.
    pub fn database(&self) -> Result<Database> {
        Database::from_bytes(self.buf.clone())
    }

    // -- byte helpers --------------------------------------------------------

    fn u16(&self, off: usize) -> u16 {
        u16le(&self.buf, off)
    }
    fn u32(&self, off: usize) -> u32 {
        u32le(&self.buf, off)
    }
    fn put_u16(&mut self, off: usize, v: u16) {
        self.buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }
    fn put_u32(&mut self, off: usize, v: u32) {
        self.buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn table_entry_offset(&self, table_type: u32) -> Result<usize> {
        let num_tables = self.u32(8);
        for i in 0..num_tables as usize {
            let o = 0x1c + i * 16;
            if self.u32(o) == table_type {
                return Ok(o);
            }
        }
        Err(PdbError::NoTable(table_type))
    }

    // -- in-place edits ------------------------------------------------------

    /// Patch a fixed-width numeric track field in place (changes only that
    /// field's bytes; does not touch page bookkeeping).
    pub fn set_track_field(&mut self, track_id: u32, field: &str, value: u32) -> Result<()> {
        let (offset, size) =
            field_spec(field).ok_or_else(|| PdbError::UnknownField(field.to_string()))?;
        check_range(field, value as u64, size)?;
        let db = self.database()?;
        let locs = db.row_locations(TableType::Tracks);
        for (track, &loc) in db.tracks.iter().zip(locs) {
            if track.id == track_id {
                let bytes = value.to_le_bytes();
                self.buf[loc + offset..loc + offset + size].copy_from_slice(&bytes[..size]);
                return Ok(());
            }
        }
        Err(PdbError::TrackNotFound(track_id))
    }

    // -- appending rows ------------------------------------------------------

    fn slot_count(&self, page_off: usize) -> u32 {
        self.buf[page_off + 0x18] as u32 + 0x100 * (self.buf[page_off + 0x19] & 1) as u32
    }

    fn dir_bytes(n: u32) -> usize {
        if n == 0 {
            0
        } else {
            (2 * n + 4 * n.div_ceil(16)) as usize
        }
    }

    fn present_count(&self, page_off: usize, n: u32) -> u32 {
        let mut count = 0;
        let groups = n.div_ceil(16);
        for group in 0..groups {
            let base = page_off + self.page_size - group as usize * 0x24;
            let in_group = std::cmp::min(16, n - group * 16);
            let mask = if in_group >= 16 {
                u16::MAX
            } else {
                (1u16 << in_group) - 1
            };
            let flags = self.u16(base - 4) & mask;
            count += flags.count_ones();
        }
        count
    }

    // `contains_key` + `insert`, not the entry API: the body also borrows
    // `self` for other reads/writes, which the entry API would conflict with.
    #[allow(clippy::map_entry)]
    fn mark_table_touched(&mut self, table_type: u32, page_index: u32) -> Result<()> {
        if !self.new_gen.contains_key(&table_type) {
            let entry = self.table_entry_offset(table_type)?;
            let (first, last) = (self.u32(entry + 8), self.u32(entry + 12));
            let mut gen = 0;
            let mut page = first;
            loop {
                gen = gen.max(self.u32(page as usize * self.page_size + 0x10));
                if page == last {
                    break;
                }
                page = self.u32(page as usize * self.page_size + 0x0c);
            }
            self.new_gen.insert(table_type, gen + 1);
        }
        self.touched_pages
            .entry(table_type)
            .or_default()
            .insert(page_index);
        self.structural = true;
        Ok(())
    }

    #[allow(clippy::map_entry)]
    fn append_row(
        &mut self,
        table_type: u32,
        row: &[u8],
        alloc: usize,
        index_shift_at: Option<usize>,
    ) -> Result<()> {
        let max_alloc = self.page_size - PAGE_HEADER_SIZE - Self::dir_bytes(1);
        if alloc > max_alloc {
            return Err(PdbError::RowTooLarge {
                alloc,
                max: max_alloc,
            });
        }
        let entry = self.table_entry_offset(table_type)?;
        let mut page_index = self.u32(entry + 12); // last_page
        let mut page_off = page_index as usize * self.page_size;

        if self.buf[page_off + 0x1b] & 0x40 != 0 {
            // Empty table: its only page is the index page. Allocate.
            page_index = self.allocate_page(table_type)?;
            page_off = page_index as usize * self.page_size;
        }

        let mut n = self.slot_count(page_off);
        let mut used = self.u16(page_off + 0x1e) as usize;
        if PAGE_HEADER_SIZE + used + alloc + Self::dir_bytes(n + 1) > self.page_size {
            page_index = self.allocate_page(table_type)?;
            page_off = page_index as usize * self.page_size;
            n = 0;
            used = 0;
        }

        if !self.orig_slots.contains_key(&page_index) {
            self.orig_slots.insert(page_index, n);
            self.appends.insert(page_index, 0);
            // Reset the written-this-save masks, as a fresh save does.
            for group in 0..n.div_ceil(16) {
                let off = page_off + self.page_size - group as usize * 0x24 - 2;
                self.put_u16(off, 0);
            }
        }
        self.mark_table_touched(table_type, page_index)?;

        let slot = n;
        let group = (slot / 16) as usize;
        let bit = slot % 16;
        let group_base = page_off + self.page_size - group * 0x24;
        if bit == 0 {
            // New group: init its presence + written flag words.
            self.put_u16(group_base - 4, 0);
            self.put_u16(group_base - 2, 0);
        }

        // Row bytes (zero-padded to the allocation) into the heap.
        let row_start = page_off + PAGE_HEADER_SIZE + used;
        self.buf[row_start..row_start + row.len()].copy_from_slice(row);
        for b in &mut self.buf[row_start + row.len()..row_start + alloc] {
            *b = 0;
        }
        if let Some(shift_at) = index_shift_at {
            self.put_u16(row_start + shift_at, (0x20 * slot) as u16);
        }

        // Row directory: offset, presence bit, written bit.
        self.put_u16(group_base - 6 - 2 * bit as usize, used as u16);
        let presence = self.u16(group_base - 4) | (1 << bit);
        self.put_u16(group_base - 4, presence);
        let written = self.u16(group_base - 2) | (1 << bit);
        self.put_u16(group_base - 2, written);
        *self.appends.get_mut(&page_index).unwrap() += 1;

        // Page header bookkeeping.
        let new_n = n + 1;
        if new_n > 511 {
            return Err(PdbError::TooManySlots);
        }
        let present = self.present_count(page_off, new_n);
        self.buf[page_off + 0x18] = (new_n & 0xff) as u8;
        self.put_u16(
            page_off + 0x19,
            (0x20 * present) as u16 | if new_n > 255 { 1 } else { 0 },
        );
        used += alloc;
        self.put_u16(page_off + 0x1e, used as u16);
        self.put_u16(
            page_off + 0x1c,
            (self.page_size - PAGE_HEADER_SIZE - used - Self::dir_bytes(new_n)) as u16,
        );
        let appends = self.appends[&page_index];
        self.put_u16(page_off + 0x20, appends as u16);
        self.put_u16(page_off + 0x22, self.orig_slots[&page_index] as u16);
        Ok(())
    }

    fn allocate_page(&mut self, table_type: u32) -> Result<u32> {
        let entry = self.table_entry_offset(table_type)?;
        let new_page = self.u32(entry + 4); // empty_candidate
        let next_unused = self.u32(12);

        let needed = (new_page as usize + 1) * self.page_size;
        if self.buf.len() < needed {
            self.buf.resize(needed, 0);
        }

        let old_last = self.u32(entry + 12);
        let page_off = new_page as usize * self.page_size;
        for b in &mut self.buf[page_off..page_off + self.page_size] {
            *b = 0;
        }
        self.put_u32(page_off + 0x04, new_page);
        self.put_u32(page_off + 0x08, table_type);
        self.put_u32(page_off + 0x0c, next_unused); // next_page -> new empty candidate
        self.buf[page_off + 0x1b] = 0x24;
        self.put_u16(page_off + 0x1c, (self.page_size - PAGE_HEADER_SIZE) as u16);

        self.put_u32(entry + 12, new_page); // table.last_page
        self.put_u32(entry + 4, next_unused); // table.empty_candidate
        self.put_u32(12, next_unused + 1); // header next_unused_page

        if self.buf[old_last as usize * self.page_size + 0x1b] & 0x40 != 0 {
            // First data page of a previously-empty table: point the index
            // page's "first data page" heap field at it.
            let heap = old_last as usize * self.page_size + PAGE_HEADER_SIZE;
            if self.u32(heap + 4) == 0x03FFFFFF {
                self.put_u32(heap + 4, new_page);
            }
        }

        self.orig_slots.insert(new_page, 0);
        self.appends.insert(new_page, 0);
        self.mark_table_touched(table_type, new_page)?;
        Ok(new_page)
    }

    fn finalize(&mut self) {
        let touched: Vec<(u32, Vec<u32>)> = self
            .touched_pages
            .iter()
            .map(|(t, pages)| (*t, pages.iter().copied().collect()))
            .collect();
        for (table_type, pages) in touched {
            let gen = self.new_gen[&table_type];
            for page in pages {
                self.put_u32(page as usize * self.page_size + 0x10, gen);
            }
        }
        if self.structural {
            if self.final_sequence.is_none() {
                self.final_sequence = Some(self.u32(0x14) + 1);
            }
            let seq = self.final_sequence.unwrap();
            self.put_u32(0x14, seq);
        }
    }

    // -- lookup-table helpers ------------------------------------------------

    fn get_or_create_artist(&mut self, name: &str) -> Result<u32> {
        let db = self.database()?;
        if let Some(a) = db.artists.iter().find(|a| a.name == name) {
            return Ok(a.id);
        }
        let new_id = db.artists.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let s = encode_string(name)?;
        let mut row = vec![0x60u8, 0x00, 0x00, 0x00];
        row.extend_from_slice(&new_id.to_le_bytes());
        row.push(0x03);
        row.push(0x0a);
        row.extend_from_slice(&s);
        let alloc = align4(10 + align4(s.len())) + 4;
        self.append_row(TableType::Artists as u32, &row, alloc, Some(2))?;
        Ok(new_id)
    }

    fn get_or_create_album(&mut self, name: &str, artist_id: u32) -> Result<u32> {
        let db = self.database()?;
        if let Some(a) = db.albums.iter().find(|a| a.name == name) {
            return Ok(a.id);
        }
        let new_id = db.albums.iter().map(|a| a.id).max().unwrap_or(0) + 1;
        let s = encode_string(name)?;
        let mut row = vec![0x80u8, 0x00, 0x00, 0x00]; // magic, index_shift
        row.extend_from_slice(&0u32.to_le_bytes()); // unknown
        row.extend_from_slice(&artist_id.to_le_bytes());
        row.extend_from_slice(&new_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes()); // unknown
        row.push(0x03);
        row.push(0x16);
        row.extend_from_slice(&s);
        let alloc = align4(22 + align4(s.len())) + 4;
        self.append_row(TableType::Albums as u32, &row, alloc, Some(2))?;
        Ok(new_id)
    }

    fn get_or_create_named(&mut self, table: TableType, name: &str) -> Result<u32> {
        let db = self.database()?;
        let existing: Vec<(u32, &str)> = match table {
            TableType::Genres => db.genres.iter().map(|g| (g.id, g.name.as_str())).collect(),
            TableType::Labels => db.labels.iter().map(|l| (l.id, l.name.as_str())).collect(),
            _ => unreachable!(),
        };
        if let Some((id, _)) = existing.iter().find(|(_, n)| *n == name) {
            return Ok(*id);
        }
        let new_id = existing.iter().map(|(id, _)| *id).max().unwrap_or(0) + 1;
        let s = encode_string(name)?;
        let mut row = new_id.to_le_bytes().to_vec();
        row.extend_from_slice(&s);
        let alloc = align4(4 + s.len());
        self.append_row(table as u32, &row, alloc, None)?;
        Ok(new_id)
    }

    fn get_or_create_key(&mut self, name: &str) -> Result<u32> {
        let db = self.database()?;
        if let Some(k) = db.keys.iter().find(|k| k.name == name) {
            return Ok(k.id);
        }
        let new_id = db.keys.iter().map(|k| k.id).max().unwrap_or(0) + 1;
        let s = encode_string(name)?;
        let mut row = new_id.to_le_bytes().to_vec();
        row.extend_from_slice(&new_id.to_le_bytes());
        row.extend_from_slice(&s);
        let alloc = align4(8 + s.len());
        self.append_row(TableType::Keys as u32, &row, alloc, None)?;
        Ok(new_id)
    }

    // -- tracks --------------------------------------------------------------

    /// Add a track; returns its id. Creates or reuses artist/album/genre/key/
    /// label rows by name.
    pub fn add_track(&mut self, t: &NewTrack) -> Result<u32> {
        let filename = t.filename.clone().unwrap_or_else(|| {
            t.file_path
                .rsplit('/')
                .next()
                .unwrap_or(&t.file_path)
                .to_string()
        });

        // Validate everything that can fail BEFORE creating lookup rows.
        for (name, value, size) in [
            ("sample_rate", t.sample_rate as u64, 4),
            ("file_size", t.file_size as u64, 4),
            ("bitrate", t.bitrate as u64, 4),
            ("track_number", t.track_number as u64, 4),
            ("tempo", t.tempo as u64, 4),
            ("disc_number", t.disc_number as u64, 2),
            ("play_count", t.play_count as u64, 2),
            ("year", t.year as u64, 2),
            ("sample_depth", t.sample_depth as u64, 2),
            ("duration", t.duration as u64, 2),
            ("artwork_id", t.artwork_id as u64, 4),
            ("color_id", t.color_id as u64, 1),
            ("rating", t.rating as u64, 1),
        ] {
            check_range(name, value, size)?;
        }

        let strings = [
            "",
            "",
            "2",
            "2",
            "",
            "",
            "ON",
            "ON",
            "",
            "",
            t.date_added.as_str(),
            t.release_date.as_str(),
            t.mix_name.as_str(),
            "",
            t.analyze_path.as_str(),
            t.date_added.as_str(),
            t.comment.as_str(),
            t.title.as_str(),
            "",
            filename.as_str(),
            t.file_path.as_str(),
        ];
        let encoded: Vec<Vec<u8>> = strings
            .iter()
            .map(|s| encode_string(s))
            .collect::<Result<_>>()?;
        let alloc = 0x88 + encoded.iter().map(|e| align4(e.len())).sum::<usize>() + 4;
        let max_alloc = self.page_size - PAGE_HEADER_SIZE - Self::dir_bytes(1);
        if alloc > max_alloc {
            return Err(PdbError::RowTooLarge {
                alloc,
                max: max_alloc,
            });
        }

        let artist_id = match &t.artist {
            Some(a) => self.get_or_create_artist(a)?,
            None => 0,
        };
        let album_id = match &t.album {
            Some(a) => self.get_or_create_album(a, artist_id)?,
            None => 0,
        };
        let genre_id = match &t.genre {
            Some(g) => self.get_or_create_named(TableType::Genres, g)?,
            None => 0,
        };
        let key_id = match &t.key {
            Some(k) => self.get_or_create_key(k)?,
            None => 0,
        };
        let label_id = match &t.label {
            Some(l) => self.get_or_create_named(TableType::Labels, l)?,
            None => 0,
        };

        let db = self.database()?;
        let track_id = db.tracks.iter().map(|t| t.id).max().unwrap_or(0) + 1;
        // Unique per-track value at 0x14 (unknown purpose; kept unique).
        let seen_max = db
            .row_locations(TableType::Tracks)
            .iter()
            .map(|&loc| u32le(&self.buf, loc + 0x14))
            .max()
            .unwrap_or(0x0010_0000);
        let unique = seen_max + 1;
        let file_type = file_type_code(&t.file_path);

        // Fixed 0x5e-byte part, field by field in file order.
        let mut fixed: Vec<u8> = Vec::with_capacity(0x5e);
        let pu16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_le_bytes());
        let pu32 = |v: &mut Vec<u8>, x: u32| v.extend_from_slice(&x.to_le_bytes());
        pu16(&mut fixed, 0x0024); // magic
        pu16(&mut fixed, 0); // index_shift (patched on append)
        pu32(&mut fixed, 0x000C_0700); // bitmask
        pu32(&mut fixed, t.sample_rate);
        pu32(&mut fixed, 0); // composer_id
        pu32(&mut fixed, t.file_size);
        pu32(&mut fixed, unique);
        pu16(&mut fixed, 0xAE49);
        pu16(&mut fixed, 0x03DD);
        pu32(&mut fixed, t.artwork_id);
        pu32(&mut fixed, key_id);
        pu32(&mut fixed, 0); // original_artist_id
        pu32(&mut fixed, label_id);
        pu32(&mut fixed, 0); // remixer_id
        pu32(&mut fixed, t.bitrate);
        pu32(&mut fixed, t.track_number);
        pu32(&mut fixed, t.tempo);
        pu32(&mut fixed, genre_id);
        pu32(&mut fixed, album_id);
        pu32(&mut fixed, artist_id);
        pu32(&mut fixed, track_id);
        pu16(&mut fixed, t.disc_number);
        pu16(&mut fixed, t.play_count);
        pu16(&mut fixed, t.year);
        pu16(&mut fixed, t.sample_depth);
        pu16(&mut fixed, t.duration);
        pu16(&mut fixed, 0x0029);
        fixed.push(t.color_id);
        fixed.push(t.rating);
        pu16(&mut fixed, file_type);
        pu16(&mut fixed, 0x0003);
        debug_assert_eq!(fixed.len(), 0x5e);

        // 21 u16 string offsets (row-relative), then strings packed in order.
        let mut blob: Vec<u8> = Vec::new();
        let mut offsets = [0u16; 21];
        let mut pos = 0x88usize;
        for (i, enc) in encoded.iter().enumerate() {
            if enc[0] & 1 == 0 && pos % 4 != 0 {
                let pad = 4 - pos % 4;
                blob.extend(std::iter::repeat(0u8).take(pad));
                pos += pad;
            }
            offsets[i] = pos as u16;
            blob.extend_from_slice(enc);
            pos += enc.len();
        }

        let mut row = fixed;
        for off in offsets {
            row.extend_from_slice(&off.to_le_bytes());
        }
        row.extend_from_slice(&blob);

        self.append_row(TableType::Tracks as u32, &row, alloc, Some(2))?;
        Ok(track_id)
    }

    // -- playlists -----------------------------------------------------------

    pub fn create_playlist(&mut self, name: &str, parent_id: u32, is_folder: bool) -> Result<u32> {
        let db = self.database()?;
        let node_id = db.playlist_tree.iter().map(|n| n.id).max().unwrap_or(0) + 1;
        let sort_order = db
            .playlist_tree
            .iter()
            .filter(|n| n.parent_id == parent_id)
            .map(|n| n.sort_order as i64)
            .max()
            .unwrap_or(-1)
            + 1;
        let s = encode_string(name)?;
        let mut row = Vec::new();
        row.extend_from_slice(&parent_id.to_le_bytes());
        row.extend_from_slice(&0u32.to_le_bytes());
        row.extend_from_slice(&(sort_order as u32).to_le_bytes());
        row.extend_from_slice(&node_id.to_le_bytes());
        row.extend_from_slice(&(is_folder as u32).to_le_bytes());
        row.extend_from_slice(&s);
        let alloc = align4(20 + s.len());
        self.append_row(TableType::PlaylistTree as u32, &row, alloc, None)?;
        Ok(node_id)
    }

    pub fn add_to_playlist(
        &mut self,
        playlist_id: u32,
        track_id: u32,
        entry_index: Option<u32>,
    ) -> Result<()> {
        let db = self.database()?;
        let node = db.playlist_tree.iter().find(|n| n.id == playlist_id);
        match node {
            Some(n) if !n.is_folder => {}
            _ => return Err(PdbError::PlaylistNotFound(playlist_id)),
        }
        let entry_index = entry_index.unwrap_or_else(|| {
            db.playlist_entries
                .iter()
                .filter(|e| e.playlist_id == playlist_id)
                .map(|e| e.entry_index)
                .max()
                .unwrap_or(0)
                + 1
        });
        let mut row = Vec::new();
        row.extend_from_slice(&entry_index.to_le_bytes());
        row.extend_from_slice(&track_id.to_le_bytes());
        row.extend_from_slice(&playlist_id.to_le_bytes());
        self.append_row(TableType::PlaylistEntries as u32, &row, 12, None)?;
        Ok(())
    }

    /// Updates a track's on-disk audio identity for a FLAC→MP3 conversion.
    ///
    /// Patches `bitrate`, `file_size`, and `file_type` in place, then rewrites
    /// the filename (string slot 19) and file_path (slot 20) DeviceSQL strings
    /// in place within the existing row allocation. Track id is unchanged.
    ///
    /// Returns an error if the new strings do not fit in the current allocation
    /// (unexpected for a `.flac` → `.mp3` shrink, but possible for other renames).
    pub fn update_track_audio(
        &mut self,
        track_id: u32,
        new_file_path: &str,
        new_filename: &str,
        bitrate: u32,
        file_size: u32,
    ) -> Result<()> {
        self.set_track_field(track_id, "bitrate", bitrate)?;
        self.set_track_field(track_id, "file_size", file_size)?;
        self.set_track_field(track_id, "file_type", file_type_code(new_file_path) as u32)?;

        let db = self.database()?;
        let locs = db.row_locations(TableType::Tracks);
        let (track, &loc) = db
            .tracks
            .iter()
            .zip(locs)
            .find(|(t, _)| t.id == track_id)
            .ok_or(PdbError::TrackNotFound(track_id))?;
        let _ = track;

        let page_off = (loc / self.page_size) * self.page_size;
        let row_end = self.row_alloc_end(page_off, loc)?;

        // String offsets are u16 relative to the row start (at loc).
        let off19 = self.u16(loc + 0x5e + 2 * 19) as usize;
        let strings_start = loc + off19;
        if strings_start >= row_end {
            return Err(PdbError::RowTooLarge {
                alloc: 0,
                max: row_end.saturating_sub(loc),
            });
        }

        let enc_name = encode_string(new_filename)?;
        let enc_path = encode_string(new_file_path)?;

        // Pack like rekordbox: long (even kind) strings 4-byte align relative to row.
        let mut blob = Vec::new();
        let mut pos = off19;
        let mut new_off19 = pos;
        // filename (slot 19)
        if enc_name[0] & 1 == 0 && pos % 4 != 0 {
            let pad = 4 - pos % 4;
            blob.extend(std::iter::repeat(0u8).take(pad));
            pos += pad;
            new_off19 = pos;
        }
        blob.extend_from_slice(&enc_name);
        pos += enc_name.len();

        let mut new_off20 = pos;
        if enc_path[0] & 1 == 0 && pos % 4 != 0 {
            let pad = 4 - pos % 4;
            blob.extend(std::iter::repeat(0u8).take(pad));
            pos += pad;
            new_off20 = pos;
        }
        blob.extend_from_slice(&enc_path);
        pos += enc_path.len();

        let available = row_end - strings_start;
        if blob.len() > available {
            return Err(PdbError::RowTooLarge {
                alloc: blob.len(),
                max: available,
            });
        }

        // Zero the old tail, write new strings, update offsets.
        self.buf[strings_start..row_end].fill(0);
        self.buf[strings_start..strings_start + blob.len()].copy_from_slice(&blob);
        self.put_u16(loc + 0x5e + 2 * 19, new_off19 as u16);
        self.put_u16(loc + 0x5e + 2 * 20, new_off20 as u16);

        // Touch generation so players notice the edit.
        let page_index = (loc / self.page_size) as u32;
        self.mark_table_touched(TableType::Tracks as u32, page_index)?;
        let _ = pos;
        Ok(())
    }

    /// Absolute file offset of the first byte past this row's allocation.
    fn row_alloc_end(&self, page_off: usize, row_abs: usize) -> Result<usize> {
        let heap_base = page_off + PAGE_HEADER_SIZE;
        let this_rel = row_abs - heap_base;
        let n = self.slot_count(page_off);
        let mut offsets = Vec::with_capacity(n as usize);
        let groups = n.div_ceil(16);
        for group in 0..groups {
            let base = page_off + self.page_size - group as usize * 0x24;
            let in_group = std::cmp::min(16, n - group * 16);
            let present = self.u16(base - 4);
            for i in 0..in_group {
                if (present >> i) & 1 != 0 {
                    offsets.push(self.u16(base - 6 - 2 * i as usize) as usize);
                }
            }
        }
        offsets.sort_unstable();
        let used = self.u16(page_off + 0x1e) as usize;
        if let Some(idx) = offsets.iter().position(|&o| o == this_rel) {
            if let Some(&next) = offsets.get(idx + 1) {
                return Ok(heap_base + next);
            }
        }
        Ok(heap_base + used)
    }
}
