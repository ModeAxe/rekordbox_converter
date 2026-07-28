# rekordbox-pdb (Rust)

A dependency-free Rust crate to **read and write** the databases rekordbox
writes to USB sticks for CDJ/XDJ players:

* `PIONEER/rekordbox/export.pdb` — tracks, artists, albums, genres, keys,
  colors, labels, playlists, artwork, history
* `PIONEER/rekordbox/exportExt.pdb` — My Tag categories and tags

It is a straight port of the [Python `rekordbox-pdb`
library](https://github.com/fragmede/rekordbox-pdb); the on-disk format is
documented in that repo's `FORMAT.md`. The two implementations produce
**byte-identical** output for the same edits (verified in CI-style differential
tests), and each reads the other's files.

No external crates — standard library only.

## Reading

```rust
use rekordbox_pdb::{Database, ExtDatabase};

let db = Database::from_file("PIONEER/rekordbox/export.pdb")?;
for track in &db.tracks {
    println!("{} {:.2} {}", track.title(), track.tempo as f64 / 100.0, track.file_path());
}

let ext = ExtDatabase::from_file("PIONEER/rekordbox/exportExt.pdb")?;
for tag in &ext.tags {
    println!("{} {}", tag.category_id, tag.name);
}
```

CLI dump:

```sh
cargo run -- path/to/export.pdb
cargo run -- path/to/exportExt.pdb --ext
```

## Writing

`PdbEditor` edits an `export.pdb` the way rekordbox itself does — surgical,
incremental changes rather than rewriting the file.

```rust
use rekordbox_pdb::{NewTrack, PdbEditor};

let mut ed = PdbEditor::from_file("PIONEER/rekordbox/export.pdb")?;

// edit a fixed field in place
ed.set_track_field(1, "rating", 5)?;

// add a track (creates/reuses artist/album/genre/key rows as needed)
let tid = ed.add_track(&NewTrack {
    artist: Some("Artist".into()),
    album: Some("Album".into()),
    genre: Some("House".into()),
    key: Some("Am".into()),
    tempo: 12800,
    duration: 200,
    bitrate: 320,
    sample_rate: 44100,
    ..NewTrack::new("My Track", "/Contents/Artist/Album/My Track.mp3")
})?;

// playlists and folders
let pid = ed.create_playlist("Warm-up set", 0, false)?;
ed.add_to_playlist(pid, tid, None)?;

ed.save("PIONEER/rekordbox/export.pdb")?;
```

It handles the format's page allocation, the heap and row-directory
bookkeeping, presence/written bitmasks, and the >255-slot count encoding.

> **Note:** a database entry alone does not make a CDJ play a track — the
> audio file must exist at `file_path` on the stick, and players expect ANLZ
> analysis files (which this crate does not create). Always back up a stick
> before editing it.

## Tests

```sh
cargo test
```

Integration tests assert the same hand-verified fixture values as the Python
library, and `examples/diffgen.rs` reproduces a fixed edit sequence for
byte-comparison against the Python editor.

## Disclaimer

Not affiliated with or endorsed by AlphaTheta / Pioneer DJ. "rekordbox",
"CDJ", and "XDJ" are trademarks of their respective owners. Independent,
interoperability-focused project; use it on your own media and keep backups.

## License

MIT — see [LICENSE](LICENSE).
