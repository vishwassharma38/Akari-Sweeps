//! Controlled administrator elevation and relaunch policy.
//!
//! Policy is kept separate from the Windows launcher so tests never open a
//! UAC dialog. The production launcher uses ShellExecuteExW with the `runas`
//! verb and waits on its process handle to propagate the child exit code.

use std::path::Path;

/// Marker appended to elevated relaunches. A child carrying the marker must
/// verify its token and abort if Windows did not grant elevation.
pub const ELEVATION_MARKER: &str = "ELEVATED";

/// Windows `ERROR_CANCELLED`, the only ShellExecuteExW result classified as
/// a user-declined UAC prompt.
pub const UAC_CANCELLED_CODE: i32 = 1223;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenState {
    Elevated,
    NotElevated,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationIntent {
    /// The plain `akari` launcher requests elevation but may continue in a
    /// restricted TUI if the user declines or launch fails.
    InteractiveStartup,
    /// A direct maintenance entry point must not continue without elevation.
    Maintenance,
    /// About and Help remain available without elevation.
    Informational,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationDecision {
    Proceed,
    RelaunchElevated,
    Abort,
}

/// Whether an unsuccessful elevation request may return to the caller's
/// unelevated interactive TUI. Direct maintenance launches must abort.
pub fn should_continue_restricted(intent: ElevationIntent, outcome: &RelaunchOutcome) -> bool {
    intent == ElevationIntent::InteractiveStartup
        && matches!(
            outcome,
            RelaunchOutcome::Cancelled | RelaunchOutcome::Failed(_)
        )
}

/// Pure policy for startup. A marked relaunch that is not actually elevated
/// always aborts, preventing recursive prompts and unelevated maintenance.
pub fn decide(
    intent: ElevationIntent,
    token: TokenState,
    is_windows: bool,
    marker_present: bool,
) -> ElevationDecision {
    if token == TokenState::Elevated {
        return ElevationDecision::Proceed;
    }
    if !is_windows {
        return ElevationDecision::Proceed;
    }
    match intent {
        ElevationIntent::Informational => ElevationDecision::Proceed,
        _ if marker_present => ElevationDecision::Abort,
        ElevationIntent::InteractiveStartup | ElevationIntent::Maintenance => {
            ElevationDecision::RelaunchElevated
        }
    }
}

/// Central fail-closed authorization check used immediately before any
/// maintenance worker starts and again at the worker boundary.
pub fn require_maintenance_privileges(
    is_windows: bool,
    token: Result<bool, String>,
) -> Result<(), String> {
    if !is_windows {
        return Ok(());
    }
    match token {
        Ok(true) => Ok(()),
        Ok(false) => Err(
            "Administrator privileges are required; maintenance was not started. Approve the Windows UAC prompt and retry."
                .to_string(),
        ),
        Err(error) => Err(format!(
            "Could not verify the Windows administrator token ({error}); maintenance was not started."
        )),
    }
}

/// True when argv carries the internal elevation marker.
pub fn has_marker(args: &[String]) -> bool {
    args.iter()
        .any(|a| a.eq_ignore_ascii_case(ELEVATION_MARKER))
}

/// Produce the `lpParameters` string accepted by ShellExecuteExW. Windows
/// command-line quoting follows the CRT rules for quotes, backslashes, and
/// whitespace. The reserved marker is de-duplicated and always appended last.
pub fn build_relaunch_parameters(args: &[String]) -> Result<String, String> {
    if args.iter().any(|arg| arg.contains('\0')) {
        return Err("an argument contains a NUL character and cannot be forwarded".to_string());
    }
    let mut forwarded: Vec<&str> = args
        .iter()
        .filter(|arg| !arg.eq_ignore_ascii_case(ELEVATION_MARKER))
        .map(String::as_str)
        .collect();
    forwarded.push(ELEVATION_MARKER);
    Ok(forwarded
        .into_iter()
        .map(quote_windows_argument)
        .collect::<Vec<_>>()
        .join(" "))
}

/// Quote a single Windows command-line argument using the standard CRT
/// parsing rules. Always quotes, which also preserves empty arguments.
pub fn quote_windows_argument(argument: &str) -> String {
    let mut quoted = String::with_capacity(argument.len() + 2);
    quoted.push('"');
    let mut backslashes = 0usize;
    for ch in argument.chars() {
        match ch {
            '\\' => backslashes += 1,
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes * 2 + 1));
                quoted.push('"');
                backslashes = 0;
            }
            _ => {
                quoted.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                quoted.push(ch);
            }
        }
    }
    // Backslashes before the closing quote must be doubled.
    quoted.push_str(&"\\".repeat(backslashes * 2));
    quoted.push('"');
    quoted
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaunchOutcome {
    Launched { exit_code: i32 },
    Cancelled,
    Failed(String),
}

/// Injectable relaunch boundary. Policy tests use mocks; this trait's real
/// implementation is the only code that can display a UAC prompt.
pub trait ElevationLauncher {
    fn launch(&mut self, exe: &Path, args: &[String]) -> RelaunchOutcome;
}

pub struct RealElevationLauncher;

impl ElevationLauncher for RealElevationLauncher {
    fn launch(&mut self, exe: &Path, args: &[String]) -> RelaunchOutcome {
        #[cfg(target_os = "windows")]
        {
            launch_windows(exe, args)
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (exe, args);
            RelaunchOutcome::Failed("elevation is only supported on Windows".to_string())
        }
    }
}

#[cfg(target_os = "windows")]
fn launch_windows(exe: &Path, args: &[String]) -> RelaunchOutcome {
    use std::mem::size_of;
    use windows_sys::Win32::Foundation::{GetLastError, WAIT_FAILED, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, INFINITE, WaitForSingleObject,
    };
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };

    if !exe.is_file() {
        return RelaunchOutcome::Failed(format!(
            "application executable does not exist or is not a file: {}",
            exe.display()
        ));
    }
    let parameters = match build_relaunch_parameters(args) {
        Ok(parameters) => parameters,
        Err(error) => return RelaunchOutcome::Failed(error),
    };
    let Ok(working_directory) = std::env::current_dir() else {
        return RelaunchOutcome::Failed(
            "could not determine the current working directory for elevation".to_string(),
        );
    };
    let verb = wide_z("runas");
    let file = wide_os_z(exe.as_os_str());
    let params = wide_z(&parameters);
    let directory = wide_os_z(working_directory.as_os_str());
    // SAFETY: all strings are NUL-terminated and remain alive for the API
    // call. The structure is zero-initialized and cbSize is set below.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.lpDirectory = directory.as_ptr();
    info.nShow = 1; // SW_SHOWNORMAL

    // SAFETY: `info` points to a correctly initialized SHELLEXECUTEINFOW.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        let code = unsafe { GetLastError() } as i32;
        if code == UAC_CANCELLED_CODE {
            return RelaunchOutcome::Cancelled;
        }
        return RelaunchOutcome::Failed(format!(
            "ShellExecuteExW with the runas verb failed with Windows error {code}: {}",
            std::io::Error::from_raw_os_error(code)
        ));
    }

    if info.hProcess.is_null() {
        return RelaunchOutcome::Failed(
            "ShellExecuteExW succeeded but returned no child process handle".to_string(),
        );
    }
    let process = OwnedProcessHandle(info.hProcess);
    // SAFETY: handle was returned by ShellExecuteExW with
    // SEE_MASK_NOCLOSEPROCESS and remains owned until `process` drops.
    let wait = unsafe { WaitForSingleObject(process.0, INFINITE) };
    if wait == WAIT_FAILED {
        let error = std::io::Error::last_os_error();
        return RelaunchOutcome::Failed(format!("waiting for the elevated child failed: {error}"));
    }
    if wait != WAIT_OBJECT_0 {
        return RelaunchOutcome::Failed(format!(
            "waiting for the elevated child returned unexpected status 0x{:08X}",
            wait
        ));
    }
    let mut exit_code = 0u32;
    // SAFETY: the process handle remains valid and `exit_code` is writable.
    if unsafe { GetExitCodeProcess(process.0, &mut exit_code) } == 0 {
        return RelaunchOutcome::Failed(format!(
            "could not retrieve the elevated child's exit code: {}",
            std::io::Error::last_os_error()
        ));
    }
    RelaunchOutcome::Launched {
        exit_code: exit_code as i32,
    }
}

#[cfg(target_os = "windows")]
struct OwnedProcessHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(target_os = "windows")]
impl Drop for OwnedProcessHandle {
    fn drop(&mut self) {
        // SAFETY: this wrapper uniquely owns the process handle returned by
        // ShellExecuteExW. Closing it is correct on every return path.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

#[cfg(target_os = "windows")]
fn wide_z(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn wide_os_z(value: &std::ffi::OsStr) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    value.encode_wide().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct MockElevationLauncher {
        requests: Vec<(String, Vec<String>)>,
        outcomes: VecDeque<RelaunchOutcome>,
    }

    impl MockElevationLauncher {
        fn new(outcomes: Vec<RelaunchOutcome>) -> Self {
            Self {
                requests: Vec::new(),
                outcomes: outcomes.into(),
            }
        }
    }

    impl ElevationLauncher for MockElevationLauncher {
        fn launch(&mut self, exe: &Path, args: &[String]) -> RelaunchOutcome {
            self.requests
                .push((exe.to_string_lossy().to_string(), args.to_vec()));
            self.outcomes
                .pop_front()
                .unwrap_or_else(|| RelaunchOutcome::Failed("mock outcome missing".to_string()))
        }
    }

    #[test]
    fn elevated_process_never_requests_uac() {
        for intent in [
            ElevationIntent::InteractiveStartup,
            ElevationIntent::Maintenance,
            ElevationIntent::Informational,
        ] {
            assert_eq!(
                decide(intent, TokenState::Elevated, true, false),
                ElevationDecision::Proceed
            );
        }
    }

    #[test]
    fn startup_and_maintenance_request_elevation_when_not_elevated() {
        assert_eq!(
            decide(
                ElevationIntent::InteractiveStartup,
                TokenState::NotElevated,
                true,
                false
            ),
            ElevationDecision::RelaunchElevated
        );
        for intent in [ElevationIntent::Maintenance] {
            assert_eq!(
                decide(intent, TokenState::NotElevated, true, false),
                ElevationDecision::RelaunchElevated
            );
        }
    }

    #[test]
    fn unknown_token_fails_toward_elevation_for_protected_intents() {
        for intent in [
            ElevationIntent::InteractiveStartup,
            ElevationIntent::Maintenance,
        ] {
            assert_eq!(
                decide(intent, TokenState::Unknown, true, false),
                ElevationDecision::RelaunchElevated
            );
        }
    }

    #[test]
    fn informational_commands_are_available_without_elevation() {
        assert_eq!(
            decide(
                ElevationIntent::Informational,
                TokenState::NotElevated,
                true,
                false
            ),
            ElevationDecision::Proceed
        );
        assert_eq!(
            decide(
                ElevationIntent::Informational,
                TokenState::NotElevated,
                true,
                true
            ),
            ElevationDecision::Proceed,
            "the internal marker must not change informational dispatch"
        );
    }

    #[test]
    fn marked_non_elevated_child_aborts_without_looping() {
        for token in [TokenState::NotElevated, TokenState::Unknown] {
            assert_eq!(
                decide(ElevationIntent::Maintenance, token, true, true),
                ElevationDecision::Abort
            );
        }
    }

    #[test]
    fn non_windows_never_invokes_uac() {
        assert_eq!(
            decide(
                ElevationIntent::InteractiveStartup,
                TokenState::NotElevated,
                false,
                false
            ),
            ElevationDecision::Proceed
        );
    }

    #[test]
    fn maintenance_privilege_gate_fails_closed() {
        assert!(require_maintenance_privileges(true, Ok(true)).is_ok());
        assert!(require_maintenance_privileges(false, Ok(false)).is_ok());
        assert!(
            require_maintenance_privileges(true, Ok(false))
                .unwrap_err()
                .contains("not started")
        );
        assert!(
            require_maintenance_privileges(true, Err("token query failed".into()))
                .unwrap_err()
                .contains("token query failed")
        );
    }

    #[test]
    fn marker_is_appended_once_and_after_public_arguments() {
        let parameters = build_relaunch_parameters(&[
            "clean".to_string(),
            "ELEVATED".to_string(),
            "elevated".to_string(),
            "--verbose".to_string(),
        ])
        .unwrap();
        assert_eq!(parameters.matches("ELEVATED").count(), 1, "{parameters}");
        assert!(parameters.find("clean").unwrap() < parameters.find("ELEVATED").unwrap());
        assert!(parameters.ends_with("\"ELEVATED\""));
    }

    #[test]
    fn windows_argument_quoting_handles_spaces_quotes_slashes_and_unicode() {
        assert_eq!(quote_windows_argument(""), "\"\"");
        assert_eq!(quote_windows_argument("two words"), "\"two words\"");
        assert_eq!(quote_windows_argument("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_windows_argument("C:\\a path\\"), "\"C:\\a path\\\\\"");
        assert_eq!(
            quote_windows_argument("Akari-日本語 🙂"),
            "\"Akari-日本語 🙂\""
        );
    }

    #[test]
    fn nul_argument_is_rejected_instead_of_truncated() {
        assert!(
            build_relaunch_parameters(&["clean\0oops".to_string()])
                .unwrap_err()
                .contains("NUL")
        );
    }

    #[test]
    fn launcher_mock_records_executable_and_original_arguments() {
        let mut mock = MockElevationLauncher::new(vec![RelaunchOutcome::Cancelled]);
        let args = vec!["clean".to_string(), "a value".to_string()];
        assert_eq!(
            mock.launch(Path::new("C:\\Program Files\\Akari\\akari.exe"), &args),
            RelaunchOutcome::Cancelled
        );
        assert_eq!(mock.requests[0].0, "C:\\Program Files\\Akari\\akari.exe");
        assert_eq!(mock.requests[0].1, args);
    }

    #[test]
    fn cancellation_is_distinct_from_real_launch_failures_and_child_codes() {
        let child_exit_1223 = RelaunchOutcome::Launched { exit_code: 1223 };
        assert!(!should_continue_restricted(
            ElevationIntent::Maintenance,
            &child_exit_1223
        ));
        let launch_failure = RelaunchOutcome::Failed("Windows error 2".into());
        assert!(!should_continue_restricted(
            ElevationIntent::Maintenance,
            &launch_failure
        ));
        assert!(should_continue_restricted(
            ElevationIntent::InteractiveStartup,
            &RelaunchOutcome::Cancelled
        ));
        assert!(should_continue_restricted(
            ElevationIntent::InteractiveStartup,
            &RelaunchOutcome::Failed("shell error".into())
        ));
        assert!(!should_continue_restricted(
            ElevationIntent::Maintenance,
            &RelaunchOutcome::Cancelled
        ));
        assert!(!should_continue_restricted(
            ElevationIntent::Maintenance,
            &RelaunchOutcome::Failed("shell error".into())
        ));
        assert!(!should_continue_restricted(
            ElevationIntent::InteractiveStartup,
            &RelaunchOutcome::Launched { exit_code: 0 }
        ));
    }
}
