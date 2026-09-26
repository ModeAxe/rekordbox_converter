# Patched fork of rekordbox-pdb 0.1.0 for real-world Rekordbox exports.
#
# Upstream: https://github.com/fragmede/rekordbox-pdb
#
# Patches vs crates.io 0.1.0:
# - decode_string: treat 0x00 and out-of-range offsets as empty (padding)
# - decode_string: scan up to 4 bytes backward when offset lands on payload
#   (fixes 0x32 / '2' errors on unknown track string slots in real exports)
# - decode_string: ISRC slot (0x90 + inner 0x03 + ASCII) per FORMAT.md
# - decode_string: never aborts whole-database read on a single bad string
# - Track::parse: zero string offsets mean empty, not row header bytes
# - PdbEditor::update_track_audio: in-place filename/path/analyze_path/bitrate/size/file_type
#   rewrite for FLAC→MP3 (keeps track IDs stable); optional sample_rate/sample_depth
