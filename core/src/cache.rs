//! Converted-MP3 cache.
//!
//! Layout mirrors the source: `<cache root>/<Artist>/<Track>.mp3`, with a
//! sidecar index storing the cache key of the source FLAC
//! (file size + modified time + CRC32). A hit copies the cached MP3 instead
//! of re-converting.
//!
//! Implemented in Phase 3.
