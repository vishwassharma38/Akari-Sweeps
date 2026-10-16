//! External-process execution helpers.
//!
//! All maintenance subprocesses go through here so exit codes are honest:
//! launching a command is not success — its exit status is.

use crate::system::ProgramOutcome;
use std::process::{Command, Stdio};

/// Run `program args`, capturing stdout+stderr. Never panics; a spawn
/// failure is reported as the conventional `9009`-style sentinel with the
/// OS error in stderr, and callers record it as a failure.
///
/// Windows npm compatibility: `npm` is commonly a `npm.cmd` shim which
/// `CreateProcess` cannot launch via a bare `npm` lookup (yields 9009
/// even though `where npm` succeeds). When `program == "npm"` fails to
/// spawn on Windows, retry once as `npm.cmd` before reporting 9009.
/// No shell (`cmd /C`) is used; only the exact shim executable.
pub fn run_program_capture(program: &str, args: &[&str]) -> ProgramOutcome {
    match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(out) => ProgramOutcome {
            exit_code: out.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&out.stdout).to_string(),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        },
        Err(e) => {
            #[cfg(target_os = "windows")]
            if program.eq_ignore_ascii_case("npm") {
                // One compatible retry: npm.cmd shim, same args, no shell.
                if let Ok(out) = Command::new("npm.cmd")
                    .args(args)
                    .stdin(Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output()
                {
                    return ProgramOutcome {
                        exit_code: out.status.code().unwrap_or(-1),
                        stdout: String::from_utf8_lossy(&out.stdout).to_string(),
                        stderr: String::from_utf8_lossy(&out.stderr).to_string(),
                    };
                }
            }
            ProgramOutcome {
                exit_code: 9009,
                stdout: String::new(),
                stderr: format!("failed to start '{program}': {e}"),
            }
        }
    }
}

/// Probe whether `program` resolves on PATH (`where` on Windows,
/// direct `PATH` lookup elsewhere — no shell spawn).
pub fn program_exists(program: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        Command::new("where")
            .arg(program)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
    #[cfg(not(target_os = "windows"))]
    {
        // Avoid `sh -c "command -v …"`: a direct PATH search spawns no
        // shell and is deterministic in tests. Handles absolute/relative
        // paths plus bare names resolved via `PATH`.
        use std::os::unix::fs::PermissionsExt as _;
        let p = std::path::Path::new(program);
        if program.contains('/') {
            return is_executable_file(p);
        }
        let Ok(path_var) = std::env::var_os("PATH") else {
            return false;
        };
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join(program);
            if is_executable_file(&candidate) {
                return true;
            }
        }
        return false;

        #[cfg(not(target_os = "windows"))]
        fn is_executable_file(p: &std::path::Path) -> bool {
            let Ok(m) = std::fs::metadata(p) else {
                return false;
            };
            if !m.is_file() {
                return false;
            }
            m.permissions().mode() & 0o111 != 0
        }
    }
}

/// Truncate long child output for the TUI log (keeps memory bounded).
#[allow(dead_code)]
pub fn push_capped(log: &mut Vec<String>, line: String, cap: usize) {
    log.push(line);
    if log.len() > cap {
        let overflow = log.len() - cap;
        log.drain(0..overflow);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_program_reports_spawn_failure() {
        let out = run_program_capture("akari-sweeps-definitely-missing-binary-xyz", &[]);
        assert_eq!(out.exit_code, 9009);
        assert!(!out.stderr.is_empty());
    }

    #[test]
    fn push_capped_bounds_memory() {
        let mut log = Vec::new();
        for i in 0..120 {
            push_capped(&mut log, format!("line {i}"), 50);
        }
        assert_eq!(log.len(), 50);
        assert!(log[0].contains("line 70"));
    }
}
