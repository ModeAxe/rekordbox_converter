//! App data directories for DrokerBox (cache, settings, staged copies).

use std::path::PathBuf;

/// Product data folder under `%LOCALAPPDATA%` / `~/.cache`.
pub const APP_DATA_DIR: &str = "DrokerBox";

/// Legacy folder from early builds — still read for one-time settings migration.
pub const LEGACY_APP_DATA_DIR: &str = "RekordboxUsbConverter";

/// `%LOCALAPPDATA%/DrokerBox` on Windows, otherwise `~/.cache/DrokerBox` or temp.
pub fn app_data_root() -> PathBuf {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join(APP_DATA_DIR);
    }
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join(APP_DATA_DIR);
    }
    std::env::temp_dir().join("DrokerBox")
}

/// Previous installs used `%LOCALAPPDATA%/RekordboxUsbConverter`.
pub fn legacy_app_data_root() -> Option<PathBuf> {
    std::env::var("LOCALAPPDATA")
        .ok()
        .map(|local| PathBuf::from(local).join(LEGACY_APP_DATA_DIR))
}
