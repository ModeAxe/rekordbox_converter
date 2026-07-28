//! Apply a fixed, deterministic edit sequence and write the result, so the
//! output can be byte-compared against the Python library doing the same.
//!
//!   cargo run --example diffgen -- <input.pdb> <output.pdb>

use rekordbox_pdb::{NewTrack, PdbEditor};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (input, output) = (&args[0], &args[1]);
    let mode = args.get(2).map(String::as_str).unwrap_or("simple");

    let mut ed = PdbEditor::from_file(input).unwrap();

    if mode == "bulk" {
        for i in 0..60u32 {
            let spec = NewTrack {
                artist: Some(format!("Bulk Ärtist {}", i % 5)),
                genre: Some("Techno".into()),
                tempo: 12000 + i,
                date_added: "2026-02-03".into(),
                ..NewTrack::new(
                    format!("Bülk Träck {i:03}"),
                    format!("/Contents/B/{i:03}.mp3"),
                )
            };
            ed.add_track(&spec).unwrap();
        }
        let pid = ed.create_playlist("Bulk Mix", 0, false).unwrap();
        for n in 0..500u32 {
            ed.add_to_playlist(pid, (n % 30) + 1, None).unwrap();
        }
        ed.save(output).unwrap();
        return;
    }

    ed.set_track_field(1, "rating", 5).unwrap();

    let spec = NewTrack {
        artist: Some("Diff Artist".into()),
        album: Some("Diff Album".into()),
        genre: Some("Techno".into()),
        key: Some("Am".into()),
        tempo: 13000,
        duration: 321,
        year: 2025,
        bitrate: 320,
        sample_rate: 44100,
        file_size: 1_234_567,
        track_number: 4,
        date_added: "2026-02-03".into(),
        analyze_path: "/PIONEER/USBANLZ/P999/00000001/ANLZ0000.DAT".into(),
        ..NewTrack::new("Diff Track", "/Contents/D/E/Diff.mp3")
    };
    let tid = ed.add_track(&spec).unwrap();

    let pid = ed.create_playlist("Diff Mix", 0, false).unwrap();
    ed.add_to_playlist(pid, tid, None).unwrap();
    ed.add_to_playlist(pid, 1, None).unwrap();

    ed.save(output).unwrap();
}
