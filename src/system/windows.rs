//! Windows-specific checks and helpers.
//!
//! Windows-specific checks and helpers: token-based administrator elevation,
//! system-drive discovery, and free-space measurement.

/// True only on Windows.
pub fn is_windows() -> bool {
    cfg!(target_os = "windows")
}

/// Query the effective process token. `Ok(false)` means the process has a
/// valid, non-elevated token; `Err` means Windows could not determine it.
pub fn elevation_status() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        use std::mem::size_of;
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::Security::{
            GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
        };
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

        let mut token = std::ptr::null_mut();
        // SAFETY: GetCurrentProcess returns the current process pseudo-handle;
        // OpenProcessToken initializes `token` on success.
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(format!(
                "OpenProcessToken failed: {}",
                std::io::Error::last_os_error()
            ));
        }

        let mut elevation = TOKEN_ELEVATION::default();
        let mut returned = 0;
        // SAFETY: `elevation` is writable and its exact size is supplied.
        let result = unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                (&mut elevation as *mut TOKEN_ELEVATION).cast(),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut returned,
            )
        };
        let query_error = (result == 0).then(std::io::Error::last_os_error);
        // SAFETY: `token` is the handle returned by OpenProcessToken above.
        unsafe { CloseHandle(token) };

        if let Some(error) = query_error {
            return Err(format!(
                "GetTokenInformation(TokenElevation) failed: {error}"
            ));
        }
        if returned < size_of::<TOKEN_ELEVATION>() as u32 {
            return Err(format!(
                "GetTokenInformation returned an incomplete token elevation record ({returned} bytes)"
            ));
        }
        Ok(elevation.TokenIsElevated != 0)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(false)
    }
}

/// Convenience query for UI display and legacy callers. Security decisions
/// use `elevation_status()` so an API failure is never mistaken for a valid
/// non-elevated result.
pub fn is_elevated() -> bool {
    elevation_status().unwrap_or(false)
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
    fn non_windows_elevation_state_is_a_successful_negative_detection() {
        #[cfg(not(target_os = "windows"))]
        assert_eq!(elevation_status(), Ok(false));
    }

    #[test]
    fn system_drive_nonempty() {
        assert!(!system_drive().is_empty());
    }
}
