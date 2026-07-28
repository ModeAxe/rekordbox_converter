//! Resolution and invocation of the bundled FFmpeg/FFprobe sidecar binaries.
//!
//! Tauri places `externalBin` entries next to the application executable
//! (in dev: `target/debug/`, in a bundle: the install directory), so the
//! binaries are resolved relative to `std::env::current_exe()`.

use std::path::PathBuf;
use std::process::Command;

/// Returns the absolute path of a sidecar binary sitting next to the app
/// executable (`ffmpeg` -> `<exe dir>/ffmpeg.exe` on Windows).
pub fn sidecar_path(name: &str) -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().ok_or_else(|| {
        std::io::Error::other("application executable has no parent directory")
    })?;
    let file = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    };
    Ok(dir.join(file))
}

/// Builds a `Command` for a sidecar, suppressing the console window that
/// Windows would otherwise flash for every spawned process.
pub fn sidecar_command(name: &str) -> std::io::Result<Command> {
    let path = sidecar_path(name)?;
    let cmd = Command::new(path);
    #[cfg(windows)]
    let cmd = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = cmd;
        cmd.creation_flags(CREATE_NO_WINDOW);
        cmd
    };
    Ok(cmd)
}

/// Runs `ffmpeg -version` and returns the first output line.
pub fn ffmpeg_version_line() -> std::io::Result<String> {
    let output = sidecar_command("ffmpeg")?.arg("-version").output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "ffmpeg -version exited with {}",
            output.status
        )));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().next().unwrap_or("unknown").trim().to_string())
}
