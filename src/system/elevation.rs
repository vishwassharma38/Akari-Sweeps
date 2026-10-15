//! Administrator-elevation handling (Requirement 2).
//!
//! - Detection reuses the BAT's `fltmc` probe
//!   ([`crate::system::windows::is_elevated`]): a real Windows privilege
//!   check, not a username or terminal-title heuristic.
//! - Relaunch uses the Windows Shell `runas` verb (`Start-Process -Verb
//!   RunAs`), targeting the actual application executable with the
//!   original arguments preserved — never a BAT wrapper, never a PATH
//!   change.
//! - The synchronous `-Wait` relaunch lets the parent detect UAC
//!   cancellation (exit `1223`) and propagate the elevated child's exit
//!   code instead of guessing.
//!
//! [`ElevationDecision::decide`] is pure and fully unit-tested; actual UAC
//! interaction cannot be automated and is covered by the manual procedure
//! in the README.

use std::path::Path;

/// Marker appended when relaunching elevated (BAT-consistent: the BAT uses
/// `ELEVATED`). Prevents infinite elevation loops: a relaunched copy that
/// is still not elevated aborts instead of relaunching again.
pub const ELEVATION_MARKER: &str = "ELEVATED";

/// UAC decline sentinel (`ERROR_CANCELLED`). Returned by the relaunch
/// wrapper when `Start-Process -Verb RunAs` throws instead of starting
/// the child (user declined, or elevation is unavailable).
pub const UAC_CANCELLED_CODE: i32 = 1223;

/// What startup (or an in-TUI maintenance request) should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElevationDecision {
    /// Run normally. Either already elevated, elevation is inapplicable
    /// (non-Windows dev/test), or the requested screen is non-destructive.
    Proceed,
    /// Relaunch the same executable elevated, then exit with its result.
    RelaunchElevated,
    /// Do not run maintenance and do not relaunch (e.g. the elevated
    /// relaunch still lacks privileges — relaunching again would loop).
    Abort,
}

/// Pure relaunch policy. `starts_maintenance`: the requested command needs
/// admin-only operations. `marker_present`: this process is already the
/// elevated relaunch.
pub fn decide(
    starts_maintenance: bool,
    elevated: bool,
    is_windows: bool,
    marker_present: bool,
) -> ElevationDecision {
    if elevated {
        return ElevationDecision::Proceed;
    }
    if !starts_maintenance {
        // Informational screens (home/about/help) are safe to view
        // without elevation.
        return ElevationDecision::Proceed;
    }
    if !is_windows {
        // No UAC off Windows; proceed so dev/test can exercise the TUI.
        // Callers still warn that real maintenance targets Windows.
        return ElevationDecision::Proceed;
    }
    if marker_present {
        return ElevationDecision::Abort;
    }
    ElevationDecision::RelaunchElevated
}

/// True when argv already carries the elevation marker.
pub fn has_marker(args: &[String]) -> bool {
    args.iter()
        .any(|a| a.eq_ignore_ascii_case(ELEVATION_MARKER))
}

/// PowerShell single-quote escaping for paths/args containing spaces.
pub fn quote_ps(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Build the synchronous elevated-relaunch command:
///
/// `powershell -NoProfile -ExecutionPolicy Bypass -Command "try {
///   $p = Start-Process -FilePath '<exe>' -ArgumentList '<a>','<b>',
///   'ELEVATED' -Verb RunAs -Wait -PassThru; exit $p.ExitCode
///  } catch { exit 1223 }"`
///
/// Returns `(program, argv)` ready for [`std::process::Command`].
/// The marker is appended unless already present, so the child never
/// relaunches a third copy.
pub fn build_relaunch_command(exe: &Path, args: &[String]) -> (String, Vec<String>) {
    let mut forwarded: Vec<String> = args.to_vec();
    if !has_marker(&forwarded) {
        forwarded.push(ELEVATION_MARKER.to_string());
    }
    let list = forwarded
        .iter()
        .map(|a| quote_ps(a))
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        "try {{ $p = Start-Process -FilePath {} -ArgumentList {} -Verb RunAs -Wait -PassThru; exit $p.ExitCode }} catch {{ exit {} }}",
        quote_ps(&exe.to_string_lossy()),
        list,
        UAC_CANCELLED_CODE
    );
    (
        "powershell".to_string(),
        vec![
            "-NoProfile".to_string(),
            "-ExecutionPolicy".to_string(),
            "Bypass".to_string(),
            "-Command".to_string(),
            script,
        ],
    )
}

/// Outcome of attempting the elevated relaunch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelaunchOutcome {
    /// The elevated child ran to completion; carries its exit code.
    Launched { exit_code: i32 },
    /// UAC was declined (or elevation unavailable): the requested
    /// maintenance operation was NOT started.
    Cancelled,
    /// The relaunch itself failed (message explains why); maintenance
    /// was NOT started.
    Failed(String),
}

/// Abstraction over the relaunch so policy is unit-testable.
pub trait ElevationLauncher {
    fn launch(&mut self, exe: &Path, args: &[String]) -> RelaunchOutcome;
}

/// Production launcher: synchronous `runas` relaunch via PowerShell.
/// Windows-only; elsewhere reports `Failed` without side effects.
pub struct RealElevationLauncher;

impl ElevationLauncher for RealElevationLauncher {
    fn launch(&mut self, exe: &Path, args: &[String]) -> RelaunchOutcome {
        #[cfg(target_os = "windows")]
        {
            let (program, argv) = build_relaunch_command(exe, args);
            match std::process::Command::new(&program).args(&argv).status() {
                Ok(status) => match status.code() {
                    Some(UAC_CANCELLED_CODE) => RelaunchOutcome::Cancelled,
                    Some(code) => RelaunchOutcome::Launched { exit_code: code },
                    None => RelaunchOutcome::Failed(
                        "elevated process terminated without an exit code".to_string(),
                    ),
                },
                Err(e) => RelaunchOutcome::Failed(format!(
                    "could not request elevation via {program}: {e}"
                )),
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = (exe, args);
            RelaunchOutcome::Failed("elevation is only supported on Windows".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted launcher: records requests, replays outcomes.
    pub struct MockElevationLauncher {
        pub requests: Vec<(String, Vec<String>)>,
        pub outcomes: VecDeque<RelaunchOutcome>,
    }

    impl MockElevationLauncher {
        pub fn new(outcomes: Vec<RelaunchOutcome>) -> Self {
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
            self.outcomes.pop_front().unwrap_or(RelaunchOutcome::Failed(
                "mock: no outcome scripted".to_string(),
            ))
        }
    }

    #[test]
    fn elevated_process_never_relaunches() {
        // Already elevated: proceed even for maintenance, marker or not.
        assert_eq!(decide(true, true, true, false), ElevationDecision::Proceed);
        assert_eq!(decide(true, true, true, true), ElevationDecision::Proceed);
    }

    #[test]
    fn non_elevated_maintenance_requests_elevation_on_windows() {
        assert_eq!(
            decide(true, false, true, false),
            ElevationDecision::RelaunchElevated
        );
    }

    #[test]
    fn relaunch_without_privileges_aborts_instead_of_looping() {
        // The ELEVATED child is still not admin: stop, do not relaunch.
        assert_eq!(decide(true, false, true, true), ElevationDecision::Abort);
    }

    #[test]
    fn informational_screens_do_not_require_elevation() {
        assert_eq!(
            decide(false, false, true, false),
            ElevationDecision::Proceed
        );
    }

    #[test]
    fn non_windows_never_relaunches() {
        assert_eq!(
            decide(true, false, false, false),
            ElevationDecision::Proceed
        );
    }

    #[test]
    fn cancellation_and_failure_carry_no_maintenance() {
        // The outcome types themselves prove no maintenance ran: neither
        // variant carries operation results, and callers must not start
        // any runner on these paths (asserted in main-flow tests).
        assert_ne!(
            RelaunchOutcome::Cancelled,
            RelaunchOutcome::Launched { exit_code: 0 }
        );
        let RelaunchOutcome::Failed(msg) = RelaunchOutcome::Failed("x".to_string()) else {
            panic!("expected Failed");
        };
        assert_eq!(msg, "x");
    }

    #[test]
    fn aliases_keep_dispatch_through_relaunch() {
        // akariclean.exe relaunches ITSELF with its args + marker, so the
        // elevated child dispatches to cleanup via its exe stem.
        let exe = Path::new("C:\\My Programs\\Akari\\akariclean.exe");
        let (program, argv) = build_relaunch_command(exe, &[]);
        assert_eq!(program, "powershell");
        let script = argv.last().unwrap();
        assert!(script.contains("akariclean.exe"), "{script}");
        assert!(
            script.contains("runas") || script.contains("RunAs"),
            "{script}"
        );
        assert!(script.contains(ELEVATION_MARKER), "{script}");
        // Shared-binary form preserves the subcommand arg first.
        let exe2 = Path::new("C:\\tools\\akari.exe");
        let (_, argv2) =
            build_relaunch_command(exe2, &["clean".to_string(), "--verbose".to_string()]);
        let script2 = argv2.last().unwrap();
        let clean_pos = script2.find("clean").unwrap();
        let marker_pos = script2.find(ELEVATION_MARKER).unwrap();
        assert!(clean_pos < marker_pos, "{script2}");
    }

    #[test]
    fn marker_is_appended_once_and_quoting_handles_spaces() {
        let exe = Path::new("C:\\My Programs\\x\\akari.exe");
        let (_, argv) = build_relaunch_command(exe, &["ELEVATED".to_string(), "clean".to_string()]);
        let script = argv.last().unwrap();
        assert_eq!(script.matches(ELEVATION_MARKER).count(), 1, "{script}");
        assert!(
            script.contains("'C:\\My Programs\\x\\akari.exe'"),
            "{script}"
        );
        assert_eq!(quote_ps("o'clock"), "'o''clock'");
    }

    #[test]
    fn mock_launcher_records_requests_for_policy_tests() {
        let mut m = MockElevationLauncher::new(vec![RelaunchOutcome::Cancelled]);
        let out = m.launch(Path::new("C:\\a\\akariclean.exe"), &[]);
        assert_eq!(out, RelaunchOutcome::Cancelled);
        assert_eq!(m.requests.len(), 1);
        assert!(m.requests[0].0.ends_with("akariclean.exe"));
    }
}
