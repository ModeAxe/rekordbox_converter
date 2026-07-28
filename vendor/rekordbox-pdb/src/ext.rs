//! Reader for `exportExt.pdb` (My Tag data).

use std::path::Path;

use crate::pdb::{table_row_offsets, Table};
use crate::{decode_string, u16le, u32le, Result};

/// Table types in `exportExt.pdb` (a different set from `export.pdb`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u32)]
pub enum ExtTableType {
    /// My Tag categories and tags.
    Tags = 3,
    /// Believed to associate tags with tracks (unverified; empty in captures).
    TagTracks = 8,
}

/// A My Tag category or tag.
///
/// Categories have small ordinal ids (1..4 by default) and `is_category` set;
/// tags carry their parent's id in `category_id` and a persistent 32-bit id.
#[derive(Clone, Debug)]
pub struct Tag {
    pub id: u32,
    pub name: String,
    pub category_id: u32,
    pub position: u32,
    pub is_category: bool,
    pub index_shift: u16,
}

impl Tag {
    fn parse(buf: &[u8], row: usize) -> Result<Tag> {
        let name_off = buf[row + 0x1d] as usize;
        Ok(Tag {
            index_shift: u16le(buf, row + 2),
            category_id: u32le(buf, row + 0x0c),
            position: u32le(buf, row + 0x10),
            id: u32le(buf, row + 0x14),
            is_category: buf[row + 0x1b] != 0,
            name: decode_string(buf, row + name_off)?,
        })
    }
}

/// A parsed `exportExt.pdb` file.
pub struct ExtDatabase {
    pub page_size: u32,
    pub tables: Vec<Table>,
    pub tags: Vec<Tag>,
}

impl ExtDatabase {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<ExtDatabase> {
        ExtDatabase::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> Result<ExtDatabase> {
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

        let mut tags = Vec::new();
        if let Some(t) = tables
            .iter()
            .find(|t| t.table_type == ExtTableType::Tags as u32)
        {
            for off in table_row_offsets(&data, page_size as usize, t) {
                tags.push(Tag::parse(&data, off)?);
            }
        }

        Ok(ExtDatabase {
            page_size,
            tables,
            tags,
        })
    }
}
