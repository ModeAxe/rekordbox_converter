//! Shared error type for the core engine.

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Top-level error type for all core operations.
///
/// Variants are added as modules are implemented; every phase of the
/// pipeline maps its failures into this enum so the orchestrator can make
/// rollback decisions from a single type.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Underlying filesystem or process I/O failure.
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// The selected drive is not a valid Rekordbox export.
    #[error("not a Rekordbox export: {0}")]
    NotARekordboxExport(String),

    /// A path argument was missing or unusable.
    #[error("{0}")]
    Message(String),

    /// Pioneer database parse failure.
    #[error("database error: {0}")]
    Database(String),

    /// FFmpeg or conversion failure.
    #[error("conversion error: {0}")]
    Convert(String),
}
