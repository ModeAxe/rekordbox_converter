//! Reader for `export.pdb`.

use std::collections::HashMap;
use std::path::Path;

use crate::{decode_string, row_offsets, u16le, u32le, Result};

/// Table types in `export.pdb`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum TableType {
    Tracks = 0,
    Genres = 1,
    Artists = 2,
    Albums = 3,
    Labels = 4,
    Keys = 5,
    Colors = 6,
    PlaylistTree = 7,
    PlaylistEntries = 8,
    Unknown9 = 9,
    Unknown10 = 10,
    HistoryPlaylists = 11,
    HistoryEntries = 12,
    Artwork = 13,
    Unknown14 = 14,
    Unknown15 = 15,
    Columns = 16,
    Unknown17 = 17,
    Unknown18 = 18,
    History = 19,
}

/// One table-directory entry from the file header.
#[derive(Clone, Copy, Debug)]
pub struct Table {
    pub table_type: u32,
    pub empty_candidate: u32,
    pub first_page: u32,
    pub last_page: u32,
}

/// Names of the 21 track string slots, in file order.
pub const TRACK_STRING_NAMES: [&str; 21] = [
    "isrc",
    "unknown_string_1",
    "unknown_string_2",
    "unknown_string_3",
    "unknown_string_4",
    "message",
    "kuvo_public",
    "autoload_hotcues",
    "unknown_string_5",
    "unknown_string_6",
    "date_added",
    "release_date",
    "mix_name",
    "unknown_string_7",
    "analyze_path",
    "analyze_date",
    "comment",
    "title",
    "unknown_string_8",
    "filename",
    "file_path",
];

/// A track row.
#[derive(Clone, Debug)]
pub struct Track {
    pub index_shift: u16,
    pub bitmask: u32,
    pub sample_rate: u32,
    pub composer_id: u32,
    pub file_size: u32,
    pub artwork_id: u32,
    pub key_id: u32,
    pub original_artist_id: u32,
    pub label_id: u32,
    pub remixer_id: u32,
    pub bitrate: u32,
    pub track_number: u32,
    /// BPM * 100.
    pub tempo: u32,
    pub genre_id: u32,
    pub album_id: u32,
    pub artist_id: u32,
    pub id: u32,
    pub disc_number: u16,
    pub play_count: u16,
    pub year: u16,
    pub sample_depth: u16,
    /// Playing time in seconds.
    pub duration: u16,
    pub color_id: u8,
    pub rating: u8,
    /// The 21 DeviceSQL string slots, in file order.
    pub strings: [String; 21],
}

impl Track {
    fn parse(buf: &[u8], row: usize) -> Result<Track> {
        let mut strings_vec = Vec::with_capacity(21);
        for i in 0..21 {
            let rel = u16le(buf, row + 0x5e + 2 * i);
            if rel == 0 {
                strings_vec.push(String::new());
                continue;
            }
            let o = row + rel as usize;
            strings_vec.push(decode_string(buf, o)?);
        }
        let strings: [String; 21] = strings_vec.try_into().unwrap();
        Ok(Track {
            index_shift: u16le(buf, row + 0x02),
            bitmask: u32le(buf, row + 0x04),
            sample_rate: u32le(buf, row + 0x08),
            composer_id: u32le(buf, row + 0x0c),
            file_size: u32le(buf, row + 0x10),
            artwork_id: u32le(buf, row + 0x1c),
            key_id: u32le(buf, row + 0x20),
            original_artist_id: u32le(buf, row + 0x24),
            label_id: u32le(buf, row + 0x28),
            remixer_id: u32le(buf, row + 0x2c),
            bitrate: u32le(buf, row + 0x30),
            track_number: u32le(buf, row + 0x34),
            tempo: u32le(buf, row + 0x38),
            genre_id: u32le(buf, row + 0x3c),
            album_id: u32le(buf, row + 0x40),
            artist_id: u32le(buf, row + 0x44),
            id: u32le(buf, row + 0x48),
            disc_number: u16le(buf, row + 0x4c),
            play_count: u16le(buf, row + 0x4e),
            year: u16le(buf, row + 0x50),
            sample_depth: u16le(buf, row + 0x52),
            duration: u16le(buf, row + 0x54),
            color_id: buf[row + 0x58],
            rating: buf[row + 0x59],
            strings,
        })
    }

    pub fn message(&self) -> &str {
        &self.strings[5]
    }
    pub fn date_added(&self) -> &str {
        &self.strings[10]
    }
    pub fn release_date(&self) -> &str {
        &self.strings[11]
    }
    pub fn mix_name(&self) -> &str {
        &self.strings[12]
    }
    pub fn analyze_path(&self) -> &str {
        &self.strings[14]
    }
    pub fn analyze_date(&self) -> &str {
        &self.strings[15]
    }
    pub fn comment(&self) -> &str {
        &self.strings[16]
    }
    pub fn title(&self) -> &str {
        &self.strings[17]
    }
    pub fn filename(&self) -> &str {
        &self.strings[19]
    }
    pub fn file_path(&self) -> &str {
        &self.strings[20]
    }
}

macro_rules! named_row {
    ($name:ident, $id_off:expr, $name_off:expr) => {
        #[derive(Clone, Debug)]
        pub struct $name {
            pub id: u32,
            pub name: String,
        }
        impl $name {
            fn parse(buf: &[u8], row: usize) -> Result<$name> {
                Ok($name {
                    id: u32le(buf, row + $id_off),
                    name: decode_string(buf, row + $name_off)?,
                })
            }
        }
    };
}

named_row!(Genre, 0, 4);
named_row!(Label, 0, 4);
named_row!(HistoryPlaylist, 0, 4);

/// An artist. `0x60` (near) rows keep the name offset in one byte; `0x64`
/// (far) rows keep it as a u16 at 0x0a.
#[derive(Clone, Debug)]
pub struct Artist {
    pub id: u32,
    pub name: String,
    pub index_shift: u16,
}

impl Artist {
    fn parse(buf: &[u8], row: usize) -> Result<Artist> {
        let subtype = u16le(buf, row);
        let index_shift = u16le(buf, row + 2);
        let id = u32le(buf, row + 4);
        let name_off = if subtype == 0x64 {
            u16le(buf, row + 0x0a) as usize
        } else {
            buf[row + 9] as usize
        };
        Ok(Artist {
            id,
            index_shift,
            name: decode_string(buf, row + name_off)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Album {
    pub id: u32,
    pub name: String,
    pub artist_id: u32,
    pub index_shift: u16,
}

impl Album {
    fn parse(buf: &[u8], row: usize) -> Result<Album> {
        let index_shift = u16le(buf, row + 2);
        let artist_id = u32le(buf, row + 8);
        let id = u32le(buf, row + 12);
        let name_off = buf[row + 0x15] as usize;
        Ok(Album {
            id,
            index_shift,
            artist_id,
            name: decode_string(buf, row + name_off)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Key {
    pub id: u32,
    pub name: String,
}

impl Key {
    fn parse(buf: &[u8], row: usize) -> Result<Key> {
        Ok(Key {
            id: u32le(buf, row),
            name: decode_string(buf, row + 8)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct Color {
    pub id: u16,
    pub name: String,
}

impl Color {
    fn parse(buf: &[u8], row: usize) -> Result<Color> {
        Ok(Color {
            id: u16le(buf, row + 5),
            name: decode_string(buf, row + 8)?,
        })
    }
}

#[derive(Clone, Debug)]
pub struct PlaylistTreeNode {
    pub id: u32,
    pub parent_id: u32,
    pub sort_order: u32,
    pub name: String,
    pub is_folder: bool,
}

impl PlaylistTreeNode {
    fn parse(buf: &[u8], row: usize) -> Result<PlaylistTreeNode> {
        Ok(PlaylistTreeNode {
            parent_id: u32le(buf, row),
            sort_order: u32le(buf, row + 8),
            id: u32le(buf, row + 12),
            is_folder: u32le(buf, row + 16) != 0,
            name: decode_string(buf, row + 20)?,
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlaylistEntry {
    pub entry_index: u32,
    pub track_id: u32,
    pub playlist_id: u32,
}

impl PlaylistEntry {
    fn parse(buf: &[u8], row: usize) -> Result<PlaylistEntry> {
        Ok(PlaylistEntry {
            entry_index: u32le(buf, row),
            track_id: u32le(buf, row + 4),
            playlist_id: u32le(buf, row + 8),
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct HistoryEntry {
    pub track_id: u32,
    pub playlist_id: u32,
    pub entry_index: u32,
}

impl HistoryEntry {
    fn parse(buf: &[u8], row: usize) -> Result<HistoryEntry> {
        Ok(HistoryEntry {
            track_id: u32le(buf, row),
            playlist_id: u32le(buf, row + 4),
            entry_index: u32le(buf, row + 8),
        })
    }
}

#[derive(Clone, Debug)]
pub struct Artwork {
    pub id: u32,
    pub path: String,
}

impl Artwork {
    fn parse(buf: &[u8], row: usize) -> Result<Artwork> {
        Ok(Artwork {
            id: u32le(buf, row),
            path: decode_string(buf, row + 4)?,
        })
    }
}

/// A UI column name (wrapped in U+FFFA / U+FFFB by rekordbox).
#[derive(Clone, Debug)]
pub struct Column {
    pub id: u16,
    pub name: String,
}

impl Column {
    fn parse(buf: &[u8], row: usize) -> Result<Column> {
        Ok(Column {
            id: u16le(buf, row),
            name: decode_string(buf, row + 4)?,
        })
    }
}

/// A parsed `export.pdb` file.
pub struct Database {
    pub page_size: u32,
    pub tables: Vec<Table>,
    pub tracks: Vec<Track>,
    pub genres: Vec<Genre>,
    pub artists: Vec<Artist>,
    pub albums: Vec<Album>,
    pub labels: Vec<Label>,
    pub keys: Vec<Key>,
    pub colors: Vec<Color>,
    pub playlist_tree: Vec<PlaylistTreeNode>,
    pub playlist_entries: Vec<PlaylistEntry>,
    pub artwork: Vec<Artwork>,
    pub columns: Vec<Column>,
    pub history_playlists: Vec<HistoryPlaylist>,
    pub history_entries: Vec<HistoryEntry>,
    locations: HashMap<u32, Vec<usize>>,
}

fn parse_rows<T>(
    offs: &[usize],
    f: impl Fn(&[u8], usize) -> Result<T>,
    data: &[u8],
) -> Result<Vec<T>> {
    let mut rows = Vec::with_capacity(offs.len());
    for &o in offs {
        match f(data, o) {
            Ok(row) => rows.push(row),
            Err(_) => continue, // skip malformed rows on real exports
        }
    }
    Ok(rows)
}

/// Absolute file offsets of every present row of a table, in order.
pub(crate) fn table_row_offsets(data: &[u8], page_size: usize, table: &Table) -> Vec<usize> {
    let mut locs = Vec::new();
    let mut page_index = table.first_page;
    loop {
        let page_off = page_index as usize * page_size;
        let page = &data[page_off..page_off + page_size];
        for ro in row_offsets(page, page_size) {
            locs.push(page_off + ro);
        }
        if page_index == table.last_page {
            break;
        }
        page_index = u32le(page, 12);
    }
    locs
}

impl Database {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Database> {
        Database::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<Database> {
        let page_size = u32le(&data, 4);
        let num_tables = u32le(&data, 8);
        let mut tables = Vec::with_capacity(num_tables as usize);
        for i in 0..num_tables as usize {
            let o = 0x1c + i * 16;
            tables.push(Table {
                table_type: u32le(&data, o),
                empty_candidate: u32le(&data, o + 4),
                first_page: u32le(&data, o + 8),
                last_page: u32le(&data, o + 12),
            });
        }

        let mut locations: HashMap<u32, Vec<usize>> = HashMap::new();
        for t in &tables {
            locations.insert(
                t.table_type,
                table_row_offsets(&data, page_size as usize, t),
            );
        }

        let get = |ty: TableType| -> Vec<usize> {
            locations.get(&(ty as u32)).cloned().unwrap_or_default()
        };

        let tracks = parse_rows(&get(TableType::Tracks), Track::parse, &data)?;
        let genres = parse_rows(&get(TableType::Genres), Genre::parse, &data)?;
        let artists = parse_rows(&get(TableType::Artists), Artist::parse, &data)?;
        let albums = parse_rows(&get(TableType::Albums), Album::parse, &data)?;
        let labels = parse_rows(&get(TableType::Labels), Label::parse, &data)?;
        let keys = parse_rows(&get(TableType::Keys), Key::parse, &data)?;
        let colors = parse_rows(&get(TableType::Colors), Color::parse, &data)?;
        let playlist_tree = parse_rows(
            &get(TableType::PlaylistTree),
            PlaylistTreeNode::parse,
            &data,
        )?;
        let playlist_entries = parse_rows(
            &get(TableType::PlaylistEntries),
            PlaylistEntry::parse,
            &data,
        )?;
        let artwork = parse_rows(&get(TableType::Artwork), Artwork::parse, &data)?;
        let columns = parse_rows(&get(TableType::Columns), Column::parse, &data)?;
        let history_playlists = parse_rows(
            &get(TableType::HistoryPlaylists),
            HistoryPlaylist::parse,
            &data,
        )?;
        let history_entries =
            parse_rows(&get(TableType::HistoryEntries), HistoryEntry::parse, &data)?;

        Ok(Database {
            page_size,
            tables,
            tracks,
            genres,
            artists,
            albums,
            labels,
            keys,
            colors,
            playlist_tree,
            playlist_entries,
            artwork,
            columns,
            history_playlists,
            history_entries,
            locations,
        })
    }

    /// Absolute file offset of each row of a table, parallel to the row vecs.
    pub fn row_locations(&self, table_type: TableType) -> &[usize] {
        self.locations
            .get(&(table_type as u32))
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }
}
