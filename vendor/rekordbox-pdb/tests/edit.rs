//! Write-path tests: every edit must read back correctly and leave the rest
//! of the file intact.

use rekordbox_pdb::{Database, NewTrack, PdbEditor};

const ONE_SONG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/data/one-song-export.pdb"
);
const BIGGER: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/bigger-export.pdb");

fn byte_diff(a: &[u8], b: &[u8]) -> usize {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).filter(|(x, y)| x != y).count()
}

#[test]
fn set_rating_in_place() {
    let orig = std::fs::read(ONE_SONG).unwrap();
    let mut ed = PdbEditor::from_bytes(orig.clone());
    ed.set_track_field(1, "rating", 5).unwrap();
    let out = ed.to_bytes();

    let db = Database::from_bytes(out.clone()).unwrap();
    assert_eq!(db.tracks[0].rating, 5);
    assert_eq!(db.tracks[0].title(), "Super Smash Bros.");
    assert_eq!(byte_diff(&orig, &out), 1); // exactly one byte changed
}

#[test]
fn set_field_rejects_bad_input() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    assert!(ed.set_track_field(1, "title", 5).is_err()); // not a fixed field
    assert!(ed.set_track_field(999, "rating", 5).is_err()); // no such track
    assert!(ed.set_track_field(1, "rating", 300).is_err()); // out of u8 range
                                                            // original value intact after the failed range check
    assert_eq!(ed.database().unwrap().tracks[0].rating, 4);
}

#[test]
fn add_track_minimal() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    let tid = ed
        .add_track(&NewTrack::new(
            "Test Song",
            "/Contents/Test Artist/Test Album/Test Song.mp3",
        ))
        .unwrap();
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    assert_eq!(db.tracks.len(), 2);
    let t = db.tracks.iter().find(|t| t.id == tid).unwrap();
    assert_eq!(t.title(), "Test Song");
    assert_eq!(
        t.file_path(),
        "/Contents/Test Artist/Test Album/Test Song.mp3"
    );
    assert_eq!(t.filename(), "Test Song.mp3");
}

#[test]
fn add_track_full_metadata() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    let spec = NewTrack {
        artist: Some("Test Artist".into()),
        album: Some("Test Album".into()),
        genre: Some("House".into()),
        key: Some("Am".into()),
        tempo: 12800,
        duration: 200,
        year: 2026,
        bitrate: 320,
        sample_rate: 44100,
        file_size: 8_000_000,
        track_number: 3,
        ..NewTrack::new("Test Song", "/Contents/A/B/Test Song.mp3")
    };
    let tid = ed.add_track(&spec).unwrap();
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let t = db.tracks.iter().find(|t| t.id == tid).unwrap();
    assert_eq!((t.tempo, t.duration, t.year), (12800, 200, 2026));
    assert_eq!((t.bitrate, t.sample_rate), (320, 44100));

    let artist = db.artists.iter().find(|a| a.id == t.artist_id).unwrap();
    assert_eq!(artist.name, "Test Artist");
    let album = db.albums.iter().find(|a| a.id == t.album_id).unwrap();
    assert_eq!(album.name, "Test Album");
    let genre = db.genres.iter().find(|g| g.id == t.genre_id).unwrap();
    assert_eq!(genre.name, "House");
    let key = db.keys.iter().find(|k| k.id == t.key_id).unwrap();
    assert_eq!(key.name, "Am");
}

#[test]
fn add_track_reuses_lookups() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    let spec = NewTrack {
        artist: Some("BABY GRAVY".into()), // exists with id 2
        key: Some("Em".into()),            // exists with id 1
        ..NewTrack::new("Another Gravy Song", "/Contents/BABY GRAVY/x/y.mp3")
    };
    let tid = ed.add_track(&spec).unwrap();
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let t = db.tracks.iter().find(|t| t.id == tid).unwrap();
    assert_eq!(t.artist_id, 2);
    assert_eq!(t.key_id, 1);
    assert_eq!(db.artists.len(), 2); // nothing new created
    assert_eq!(db.keys.len(), 1);
}

#[test]
fn add_track_unicode_title() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    let tid = ed
        .add_track(&NewTrack::new(
            "Stavöstranos — épreuve",
            "/Contents/a/b/c.mp3",
        ))
        .unwrap();
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let t = db.tracks.iter().find(|t| t.id == tid).unwrap();
    assert_eq!(t.title(), "Stavöstranos — épreuve");
}

#[test]
fn add_track_leaves_existing_data_intact() {
    let before = Database::from_file(BIGGER).unwrap();
    let mut ed = PdbEditor::from_file(BIGGER).unwrap();
    ed.add_track(&NewTrack::new("New", "/Contents/a/b/New.mp3"))
        .unwrap();
    let after = Database::from_bytes(ed.to_bytes()).unwrap();

    for t in &before.tracks {
        let same = after.tracks.iter().find(|x| x.id == t.id).unwrap();
        assert_eq!(same.title(), t.title());
    }
    let a1: Vec<_> = before.artists.iter().map(|a| (a.id, &a.name)).collect();
    let a2: Vec<_> = after.artists.iter().map(|a| (a.id, &a.name)).collect();
    assert_eq!(a1, a2);
}

#[test]
fn oversized_row_rejected_cleanly() {
    let orig = std::fs::read(ONE_SONG).unwrap();
    let mut ed = PdbEditor::from_bytes(orig.clone());
    let huge = "x".repeat(5000);
    assert!(ed
        .add_track(&NewTrack::new(huge, "/Contents/a/b/c.mp3"))
        .is_err());
    // still usable
    let tid = ed
        .add_track(&NewTrack::new("fine", "/Contents/a/b/fine.mp3"))
        .unwrap();
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    assert!(db.tracks.iter().any(|t| t.id == tid));
}

#[test]
fn playlists() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    let tid = ed
        .add_track(&NewTrack::new("New", "/Contents/a/b/New.mp3"))
        .unwrap();
    ed.add_to_playlist(1, tid, None).unwrap(); // existing "aac"
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let mut entries: Vec<_> = db
        .playlist_entries
        .iter()
        .filter(|e| e.playlist_id == 1)
        .collect();
    entries.sort_by_key(|e| e.entry_index);
    assert_eq!(
        entries.iter().map(|e| e.track_id).collect::<Vec<_>>(),
        vec![1, tid]
    );
    assert_eq!(
        entries.iter().map(|e| e.entry_index).collect::<Vec<_>>(),
        vec![1, 2]
    );
}

#[test]
fn create_playlist_and_folder() {
    let mut ed = PdbEditor::from_file(BIGGER).unwrap();
    let pid = ed.create_playlist("claude mix", 0, false).unwrap();
    for tid in [1, 5, 9] {
        ed.add_to_playlist(pid, tid, None).unwrap();
    }
    let folder = ed.create_playlist("2026 Gigs", 0, true).unwrap();
    let nested = ed.create_playlist("Berghain", folder, false).unwrap();

    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let node = db.playlist_tree.iter().find(|n| n.id == pid).unwrap();
    assert_eq!(node.name, "claude mix");
    assert!(!node.is_folder);
    let mut entries: Vec<_> = db
        .playlist_entries
        .iter()
        .filter(|e| e.playlist_id == pid)
        .collect();
    entries.sort_by_key(|e| e.entry_index);
    assert_eq!(
        entries.iter().map(|e| e.track_id).collect::<Vec<_>>(),
        vec![1, 5, 9]
    );

    let f = db.playlist_tree.iter().find(|n| n.id == folder).unwrap();
    assert!(f.is_folder);
    let nn = db.playlist_tree.iter().find(|n| n.id == nested).unwrap();
    assert_eq!(nn.parent_id, folder);
}

#[test]
fn add_to_unknown_playlist_rejected() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    assert!(ed.add_to_playlist(99, 1, None).is_err());
}

#[test]
fn bulk_tracks_spill_to_new_pages() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    let mut ids = Vec::new();
    for i in 0..50 {
        let spec = NewTrack {
            artist: Some(format!("Bulk Artist {}", i % 7)),
            ..NewTrack::new(
                format!("Bulk Track {i:03}"),
                format!("/Contents/Bulk/Album/{i:03}.mp3"),
            )
        };
        ids.push(ed.add_track(&spec).unwrap());
    }
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    assert_eq!(db.tracks.len(), 51);
    let unique: std::collections::HashSet<_> = ids.iter().collect();
    assert_eq!(unique.len(), ids.len());
    for (i, tid) in ids.iter().enumerate() {
        let t = db.tracks.iter().find(|t| t.id == *tid).unwrap();
        assert_eq!(t.title(), format!("Bulk Track {i:03}"));
    }
}

#[test]
fn update_track_audio_rewrites_path_and_analyze_path() {
    let mut ed = PdbEditor::from_file(ONE_SONG).unwrap();
    ed.update_track_audio(
        1,
        "/Contents/A/x.mp3",
        "x.mp3",
        320,
        1_000_000,
        "/PIONEER/USBANLZ/P036/0002F34C/ANLZ0000.DAT",
        Some(44100),
        Some(16),
    )
    .unwrap();
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let t = db.tracks.iter().find(|t| t.id == 1).unwrap();
    assert_eq!(t.file_path(), "/Contents/A/x.mp3");
    assert_eq!(t.filename(), "x.mp3");
    assert_eq!(
        t.analyze_path(),
        "/PIONEER/USBANLZ/P036/0002F34C/ANLZ0000.DAT"
    );
    assert_eq!(t.bitrate, 320);
    assert_eq!(t.file_size, 1_000_000);
    assert_eq!(t.sample_rate, 44100);
    assert_eq!(t.sample_depth, 16);
    assert_eq!(t.title(), "Super Smash Bros.");
    assert_eq!(t.analyze_date(), "2024-02-29");
}

#[test]
fn playlist_with_hundreds_of_entries() {
    let mut ed = PdbEditor::from_file(BIGGER).unwrap();
    let pid = ed
        .create_playlist("everything, many times", 0, false)
        .unwrap();
    for n in 0..400u32 {
        ed.add_to_playlist(pid, (n % 12) + 1, None).unwrap();
    }
    let db = Database::from_bytes(ed.to_bytes()).unwrap();
    let entries: Vec<_> = db
        .playlist_entries
        .iter()
        .filter(|e| e.playlist_id == pid)
        .collect();
    assert_eq!(entries.len(), 400);
    let mut idx: Vec<u32> = entries.iter().map(|e| e.entry_index).collect();
    idx.sort();
    assert_eq!(idx, (1..=400).collect::<Vec<_>>());
}
