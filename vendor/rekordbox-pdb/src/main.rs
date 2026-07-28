//! Dump the contents of a rekordbox export.pdb / exportExt.pdb file.
//!
//! Usage:
//!     rekordbox-pdb <file.pdb>          # export.pdb
//!     rekordbox-pdb <file.pdb> --ext    # exportExt.pdb

use std::collections::HashMap;
use std::process::ExitCode;

use rekordbox_pdb::{Database, ExtDatabase};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ext = args.iter().any(|a| a == "--ext");
    let path = match args.iter().find(|a| !a.starts_with("--")) {
        Some(p) => p.clone(),
        None => {
            eprintln!("usage: rekordbox-pdb <file.pdb> [--ext]");
            return ExitCode::from(2);
        }
    };

    let result = if ext {
        dump_ext(&path)
    } else {
        dump_export(&path)
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn dump_export(path: &str) -> rekordbox_pdb::Result<()> {
    let db = Database::from_file(path)?;
    let artists: HashMap<u32, &str> = db.artists.iter().map(|a| (a.id, a.name.as_str())).collect();
    let albums: HashMap<u32, &str> = db.albums.iter().map(|a| (a.id, a.name.as_str())).collect();
    let genres: HashMap<u32, &str> = db.genres.iter().map(|g| (g.id, g.name.as_str())).collect();
    let keys: HashMap<u32, &str> = db.keys.iter().map(|k| (k.id, k.name.as_str())).collect();

    println!("{} tracks", db.tracks.len());
    let mut tracks: Vec<&_> = db.tracks.iter().collect();
    tracks.sort_by_key(|t| t.id);
    for t in tracks {
        let bpm = t.tempo as f64 / 100.0;
        println!(
            "  [{}] {:?} artist={:?} album={:?} genre={:?} key={:?} bpm={:.2} {}:{:02} rating={}",
            t.id,
            t.title(),
            artists.get(&t.artist_id).copied().unwrap_or(""),
            albums.get(&t.album_id).copied().unwrap_or(""),
            genres.get(&t.genre_id).copied().unwrap_or(""),
            keys.get(&t.key_id).copied().unwrap_or(""),
            bpm,
            t.duration / 60,
            t.duration % 60,
            t.rating,
        );
        println!("        {}", t.file_path());
    }

    println!("{} playlists", db.playlist_tree.len());
    let mut counts: HashMap<u32, usize> = HashMap::new();
    for e in &db.playlist_entries {
        *counts.entry(e.playlist_id).or_default() += 1;
    }
    for node in &db.playlist_tree {
        let kind = if node.is_folder { "folder" } else { "playlist" };
        println!(
            "  [{}] {} {:?} ({} entries)",
            node.id,
            kind,
            node.name,
            counts.get(&node.id).copied().unwrap_or(0)
        );
    }
    Ok(())
}

fn dump_ext(path: &str) -> rekordbox_pdb::Result<()> {
    let ext = ExtDatabase::from_file(path)?;
    let mut categories: Vec<&_> = ext.tags.iter().filter(|t| t.is_category).collect();
    categories.sort_by_key(|c| c.position);
    println!("{} My Tag categories", categories.len());
    for cat in categories {
        println!("  [{}] {:?}", cat.id, cat.name);
        let mut tags: Vec<&_> = ext
            .tags
            .iter()
            .filter(|t| !t.is_category && t.category_id == cat.id)
            .collect();
        tags.sort_by_key(|t| t.position);
        for tag in tags {
            println!("      {:?} (id={:#010x})", tag.name, tag.id);
        }
    }
    Ok(())
}
