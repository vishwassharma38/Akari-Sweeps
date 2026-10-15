//! Windows-specific checks and helpers.
//!
//! Mirrors the BAT prerequisites: administrator elevation (via `fltmc`,
//! exactly like the script), system-drive discovery, and free-space
//! measurement through PowerShell `DriveInfo` (same source as `:GetFreeKB`).

/// True only on Windows.
pub fn is_windows() -> bool {
    cfg!(target_os = "windows")
}

/// Administrator check mirroring the BAT's `fltmc >nul 2>&1` probe.
/// Returns true when `fltmc` exits 0. On non-Windows hosts always false
/// (tests and `cargo check` on Linux must not claim elevation).
pub fn is_elevated() -> bool {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("fltmc")
            .arg("volumes")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
}

/// System drive letter, e.g. `C:`. Honors `%SystemDrive%` on Windows,
/// defaults to `C:` elsewhere.
pub fn system_drive() -> String {
    std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string())
}

/// Free space on the system drive in KB, via PowerShell `DriveInfo`,
/// exactly like the BAT `:GetFreeKB` helper. `None` when unreadable.
pub fn free_space_kb() -> Option<i64> {
    #[cfg(target_os = "windows")]
    {
        let drive = system_drive();
        let script = format!(
            "[int64][math]::Floor((New-Object System.IO.DriveInfo -ArgumentList '{drive}').AvailableFreeSpace / 1024)"
        );
        let out = std::process::Command::new("powershell")
            .args(["-NoProfile", "-Command", &script])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<i64>()
            .ok()
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = system_drive();
        None
    }
}

/// Human-readable elevation guidance shown in the TUI when admin-only
/// steps are requested without privileges.
pub fn elevation_help() -> &'static str {
    "Administrator privileges are required. Right-click the app (or the \
     installed shortcut) and choose 'Run as administrator', then retry."
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_windows_is_not_elevated_by_default() {
        #[cfg(not(target_os = "windows"))]
        assert!(!is_elevated());
    }

    #[test]
    fn system_drive_nonempty() {
        assert!(!system_drive().is_empty());
    }
}
