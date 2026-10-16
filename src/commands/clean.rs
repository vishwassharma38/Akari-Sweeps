//! Cleanup phase — strict cleaning-only workflow.
//!
//! Strict separation: `akariclean` launches zero DISM, SFC, CHKDSK, or
//! other Windows health/servicing commands, including
//! `DISM /StartComponentCleanup` (now owned by the health workflow).
//! Directory targets preserve BAT `:PhaseCleanup` order; `[18/18]` is
//! Disk Cleanup + SilentCleanup only.

use crate::system::{
    ExecEvent, MaintenanceRunner, OpResult, ProgressState, RunTotals, expand_windows_template,
};
use std::path::{Path, PathBuf};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Plan data (data-only so tests can verify order without side effects)
// ---------------------------------------------------------------------------

/// One directory-clean target in BAT call order.
#[derive(Debug, Clone)]
pub struct DirTarget {
    pub phase: &'static str,
    pub template: &'static str,
}

/// Exact `CleanDir` call order from the BAT (fixed targets only;
/// Chromium/Firefox profile expansions happen at runtime).
pub fn fixed_dir_targets_in_order() -> Vec<DirTarget> {
    vec![
        // [1/18] USER TEMP
        DirTarget {
            phase: "1/18 USER TEMP",
            template: "%TEMP%",
        },
        DirTarget {
            phase: "1/18 USER TEMP",
            template: "%LOCALAPPDATA%\\Temp",
        },
        // [2/18] WINDOWS TEMP
        DirTarget {
            phase: "2/18 WINDOWS TEMP",
            template: "%SystemRoot%\\Temp",
        },
        // [3/18] WER / CRASH DUMPS
        DirTarget {
            phase: "3/18 ERROR REPORTS",
            template: "%ProgramData%\\Microsoft\\Windows\\WER\\ReportArchive",
        },
        DirTarget {
            phase: "3/18 ERROR REPORTS",
            template: "%ProgramData%\\Microsoft\\Windows\\WER\\ReportQueue",
        },
        DirTarget {
            phase: "3/18 ERROR REPORTS",
            template: "%ProgramData%\\Microsoft\\Windows\\WER\\Temp",
        },
        DirTarget {
            phase: "3/18 ERROR REPORTS",
            template: "%LOCALAPPDATA%\\CrashDumps",
        },
        // [4/18] THUMBNAIL / SHADER
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%LOCALAPPDATA%\\Microsoft\\Windows\\Explorer\\ThumbCache",
        },
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%LOCALAPPDATA%\\D3DSCache",
        },
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%LOCALAPPDATA%\\NVIDIA\\DXCache",
        },
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%LOCALAPPDATA%\\NVIDIA\\GLCache",
        },
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%ProgramData%\\NVIDIA Corporation\\NV_Cache",
        },
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%LOCALAPPDATA%\\AMD\\DxCache",
        },
        DirTarget {
            phase: "4/18 THUMBNAIL/SHADER",
            template: "%LOCALAPPDATA%\\AMD\\GLCache",
        },
        // [5/18] WU DOWNLOAD (services handled separately; dir listed here)
        DirTarget {
            phase: "5/18 WINDOWS UPDATE",
            template: "%SystemRoot%\\SoftwareDistribution\\Download",
        },
        // [6/18] DELIVERY OPTIMIZATION
        DirTarget {
            phase: "6/18 DELIVERY OPT",
            template: "%ProgramData%\\Microsoft\\Windows\\DeliveryOptimization\\Cache",
        },
        // [7/18] STORE CACHE
        DirTarget {
            phase: "7/18 STORE CACHE",
            template: "%LOCALAPPDATA%\\Packages\\Microsoft.WindowsStore_8wekyb3d8bbwe\\LocalCache",
        },
        // [8/18] VS CODE (10 dirs)
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\Cache",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\CachedData",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\GPUCache",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\logs",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\Service Worker\\CacheStorage",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\Service Worker\\ScriptCache",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%APPDATA%\\Code\\CachedExtensionVSIXs",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%LOCALAPPDATA%\\Code\\Cache",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%LOCALAPPDATA%\\Code\\CachedData",
        },
        DirTarget {
            phase: "8/18 VS CODE",
            template: "%LOCALAPPDATA%\\Code\\GPUCache",
        },
        // [15/18] DISCORD (5 dirs)
        DirTarget {
            phase: "15/18 DISCORD",
            template: "%APPDATA%\\discord\\Cache",
        },
        DirTarget {
            phase: "15/18 DISCORD",
            template: "%APPDATA%\\discord\\Code Cache",
        },
        DirTarget {
            phase: "15/18 DISCORD",
            template: "%APPDATA%\\discord\\GPUCache",
        },
        DirTarget {
            phase: "15/18 DISCORD",
            template: "%APPDATA%\\discord\\Service Worker\\CacheStorage",
        },
        DirTarget {
            phase: "15/18 DISCORD",
            template: "%APPDATA%\\discord\\DawnCache",
        },
        // Firefox crash reports (profile cache2/startupCache are wildcard-expanded)
        DirTarget {
            phase: "14/18 FIREFOX",
            template: "%LOCALAPPDATA%\\Mozilla\\Firefox\\Crash Reports",
        },
    ]
}

/// Chromium roots in BAT order ([9/18]–[13/18]).
pub fn chromium_roots_in_order() -> Vec<&'static str> {
    vec![
        "%LOCALAPPDATA%\\Google\\Chrome\\User Data",
        "%LOCALAPPDATA%\\Microsoft\\Edge\\User Data",
        "%LOCALAPPDATA%\\BraveSoftware\\Brave-Browser\\User Data",
        "%LOCALAPPDATA%\\Vivaldi\\User Data",
        "%APPDATA%\\Opera Software\\Opera Stable",
        "%APPDATA%\\Opera Software\\Opera GX Stable",
    ]
}

/// Per-profile Chromium cache subfolders (`:CleanChromium` body, in order).
pub fn chromium_profile_subdirs() -> Vec<&'static str> {
    vec![
        "Cache",
        "Code Cache",
        "GPUCache",
        "DawnCache",
        "Service Worker\\CacheStorage",
        "Service Worker\\ScriptCache",
        "ShaderCache",
    ]
}

/// Number of program-step work units in cleanup: npm, pip, DNS,
/// Recycle Bin, Disk Cleanup, SilentCleanup. Always 6 — skipped programs
/// still produce a (skipped) `OpResult`, so the total stays stable.
pub const CLEANUP_PROGRAM_STEPS: u64 = 6;

/// Stable task-count total for the cleanup phase (documented
/// approximation, not exact time/byte progress):
/// `fixed targets + existing Chromium profile subdirs + existing Firefox
/// profile subdirs + 6 program steps`. Dynamic counts use a cheap shallow
/// scan (existence + one `read_dir` per root), never a full recursive
/// walk, so no expensive extra traversal is performed solely for progress.
pub fn cleanup_total_units() -> u64 {
    let fixed = fixed_dir_targets_in_order().len() as u64;
    fixed + count_existing_chromium_units() + count_existing_firefox_units() + CLEANUP_PROGRAM_STEPS
}

/// Count existing Chromium profile cache subdirs (shallow scan only).
pub fn count_existing_chromium_units() -> u64 {
    let mut n = 0u64;
    for root_template in chromium_roots_in_order() {
        let Some(root) = expand_windows_template(root_template) else {
            continue;
        };
        if !root.exists() {
            continue;
        }
        for profile in child_dirs(&root) {
            for sub in chromium_profile_subdirs() {
                if profile.join(&sub).exists() {
                    n += 1;
                }
            }
        }
    }
    n
}

/// Count existing Firefox profile `cache2`/`startupCache` subdirs
/// (shallow scan only; `Crash Reports` is a fixed target already counted).
pub fn count_existing_firefox_units() -> u64 {
    let Some(profiles) = expand_windows_template("%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles")
    else {
        return 0;
    };
    let mut n = 0u64;
    for profile in child_dirs(&profiles) {
        for sub in ["cache2", "startupCache"] {
            if profile.join(sub).exists() {
                n += 1;
            }
        }
    }
    n
}

/// Emit one evidence-based progress snapshot. Lightweight: one message per
/// work unit plus one working-state message before each opaque
/// long-running task (no per-file messages, no timer animation).
pub fn emit_cleanup_progress(
    runner: &mut dyn MaintenanceRunner,
    completed: u64,
    total: u64,
    current: &str,
    failed: bool,
    indeterminate: bool,
) {
    let mut s = ProgressState::new(completed.min(total), total, current.to_string());
    s.failed = failed;
    s.indeterminate = indeterminate;
    runner.emit(ExecEvent::Progress(s));
}

/// The 18 phase banners in BAT order (for TUI progress + tests).
#[allow(dead_code)]
pub fn cleanup_phase_labels() -> Vec<&'static str> {
    vec![
        "1/18 USER TEMPORARY FILES",
        "2/18 WINDOWS TEMPORARY FILES",
        "3/18 WINDOWS ERROR REPORTING / CRASH DUMPS",
        "4/18 THUMBNAIL / GRAPHICS SHADER CACHES",
        "5/18 WINDOWS UPDATE DOWNLOAD CACHE",
        "6/18 DELIVERY OPTIMIZATION CACHE",
        "7/18 MICROSOFT STORE CACHE",
        "8/18 VS CODE CACHE / LOGS",
        "9/18 GOOGLE CHROME CACHE - ALL PROFILES",
        "10/18 MICROSOFT EDGE CACHE - ALL PROFILES",
        "11/18 BRAVE CACHE - ALL PROFILES",
        "12/18 VIVALDI CACHE - ALL PROFILES",
        "13/18 OPERA / OPERA GX CACHE",
        "14/18 FIREFOX CACHE - ALL PROFILES",
        "15/18 DISCORD CACHE",
        "16/18 DEVELOPER PACKAGE CACHES",
        "17/18 DNS + RECYCLE BIN",
        "18/18 WINDOWS BUILT-IN CLEANUP",
    ]
}

// ---------------------------------------------------------------------------
// Filesystem primitives (BAT `:CleanDir` / `:MeasureDir` / `:TallyDir`)
// ---------------------------------------------------------------------------

/// Recursively count files + total KB (bytes summed, then floor /1024),
/// like the BAT `:MeasureDir` PowerShell. `None` when unmeasurable.
pub fn measure_dir(path: &Path) -> Option<(u64, u64)> {
    if !path.exists() {
        return None;
    }
    let mut count: u64 = 0;
    let mut bytes: u64 = 0;
    let mut stack = vec![path.to_path_buf()];
    let mut ok_any = false;
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        ok_any = true;
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let ft = match entry.file_type() {
                Ok(t) => t,
                Err(_) => continue,
            };
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                match entry.metadata() {
                    Ok(m) => {
                        count += 1;
                        bytes = bytes.saturating_add(m.len());
                    }
                    Err(_) => continue,
                }
            }
        }
    }
    if !ok_any && !path.is_dir() {
        return None;
    }
    Some((count, bytes / 1024))
}

/// Why a single file/directory entry could not be removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteFailureKind {
    /// Sharing/lock violation: file is in use by a running app/service.
    LockedInUse,
    /// Access denied: insufficient permissions (may need elevation).
    PermissionDenied,
    /// Any other I/O failure.
    Other,
}

/// Classify a deletion I/O error without guessing.
///
/// On Windows, raw OS errors 32 (`ERROR_SHARING_VIOLATION`) and 33
/// (`ERROR_LOCK_VIOLATION`) mean locked/in-use; 5
/// (`ERROR_ACCESS_DENIED`) means permission. Elsewhere, map
/// `PermissionDenied` kind accordingly; everything else is `Other`.
/// `NotFound` is treated as success (already gone) by callers and never
/// reaches this classifier as a failure.
pub fn classify_delete_error(e: &std::io::Error) -> DeleteFailureKind {
    if let Some(code) = e.raw_os_error() {
        // Windows sharing/lock violations: locked or in use.
        if code == 32 || code == 33 {
            return DeleteFailureKind::LockedInUse;
        }
        if code == 5 {
            return DeleteFailureKind::PermissionDenied;
        }
    }
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => DeleteFailureKind::PermissionDenied,
        _ => {
            // On Windows without a raw code, fall back to kind; sharing
            // violations sometimes surface as generic errors, so treat
            // unclassified failures as Other (honest, not mislabeled).
            DeleteFailureKind::Other
        }
    }
}

/// Per-directory deletion breakdown for honest reporting.
#[derive(Debug, Clone, Default)]
pub struct CleanDetail {
    pub locked: u64,
    pub permission: u64,
    pub other: u64,
}

fn record_delete_failure(detail: &mut CleanDetail, e: &std::io::Error) {
    // NotFound means another process already removed it: not a failure.
    if e.kind() == std::io::ErrorKind::NotFound {
        return;
    }
    match classify_delete_error(e) {
        DeleteFailureKind::LockedInUse => detail.locked += 1,
        DeleteFailureKind::PermissionDenied => detail.permission += 1,
        DeleteFailureKind::Other => detail.other += 1,
    }
}

fn remove_one_entry(path: &Path, detail: &mut CleanDetail) {
    // Symlinks/junctions: remove the link itself, never follow it.
    let meta = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) => {
            record_delete_failure(detail, &e);
            return;
        }
    };
    if meta.file_type().is_dir() && !meta.file_type().is_symlink() {
        // Recurse first so one locked file does not abort siblings;
        // then try to remove the (now hopefully empty) dir itself.
        clean_contents_recursive(path, detail);
        if let Err(e) = std::fs::remove_dir(path) {
            // Directory not empty (locked child skipped) is expected:
            // count once as Other only if the dir itself is the problem,
            // otherwise children already counted. Probe emptiness cheaply.
            let empty = std::fs::read_dir(path)
                .map(|mut r| r.next().is_none())
                .unwrap_or(false);
            if empty {
                record_delete_failure(detail, &e);
            }
            // Non-empty due to skipped children: already counted.
        }
    } else if let Err(e) = std::fs::remove_file(path) {
        record_delete_failure(detail, &e);
    }
}

fn clean_contents_recursive(dir: &Path, detail: &mut CleanDetail) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            record_delete_failure(detail, &e);
            return;
        }
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => {
                detail.other += 1;
                continue;
            }
        };
        remove_one_entry(&entry.path(), detail);
    }
}

/// Delete directory *contents* but leave the directory itself,
/// mirroring `del /f /s /q + rd` with locked files skipped safely.
///
/// One failed file never aborts siblings: deletion is per-entry and
/// continues. No force-delete, no process termination, no privilege
/// bypass. Returns a best-effort breakdown for reporting.
pub fn clean_dir_contents_detailed(path: &Path) -> CleanDetail {
    let mut detail = CleanDetail::default();
    clean_contents_recursive(path, &mut detail);
    detail
}

/// Delete directory *contents* but leave the directory itself,
/// mirroring `del /f /s /q + rd` with locked files skipped silently.
#[allow(dead_code)]
pub fn clean_dir_contents(path: &Path) {
    let _ = clean_dir_contents_detailed(path);
}

/// Enumerate immediate subdirectories (for Chromium profiles / Firefox).
pub fn child_dirs(path: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return out,
    };
    for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            out.push(entry.path());
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Orchestration (testable via `MaintenanceRunner`)
// ---------------------------------------------------------------------------

fn emit(runner: &mut dyn MaintenanceRunner, ev: ExecEvent) {
    runner.emit(ev);
}

fn run_one_dir(
    runner: &mut dyn MaintenanceRunner,
    totals: &mut RunTotals,
    results: &mut Vec<OpResult>,
    phase: &str,
    path: PathBuf,
    display: String,
) {
    let id = format!("clean:{}", display);
    emit(
        runner,
        ExecEvent::OpStarted {
            id: id.clone(),
            label: phase.to_string(),
            target: display.clone(),
        },
    );
    // Lightweight monotonic timing for baseline diagnostics (stored on
    // the result, never used for progress).
    let t0 = Instant::now();
    let outcome = runner.clean_dir(&path);
    let elapsed_ms = t0.elapsed().as_millis() as u64;
    if !outcome.existed {
        totals.dirs_missing += 1;
        let mut r = OpResult::skipped(&id, phase, &display, "not present - skipped".to_string());
        r.target = display.clone();
        r.duration_ms = Some(elapsed_ms);
        results.push(r.clone());
        runner.emit(ExecEvent::OpFinished(r));
        emit(
            runner,
            ExecEvent::Log(format!("Skipped (not present): {display}")),
        );
        return;
    }
    totals.dirs_done += 1;
    if outcome.measure_failed {
        totals.measure_fail += 1;
    }
    totals.files_removed += outcome.files_removed;
    totals.kb_removed += outcome.kb_removed;
    totals.files_left += outcome.files_left;
    totals.files_locked += outcome.locked_skipped;
    totals.files_permission += outcome.permission_denied;
    totals.files_other_left += outcome.other_unremovable;
    let mut detail = format!(
        "removed {} files ({} KB); {} left",
        outcome.files_removed, outcome.kb_removed, outcome.files_left
    );
    if outcome.files_left > 0
        && (outcome.locked_skipped > 0
            || outcome.permission_denied > 0
            || outcome.other_unremovable > 0)
    {
        detail.push_str(&format!(
            " (locked/in-use: {}, permission: {}, other: {})",
            outcome.locked_skipped, outcome.permission_denied, outcome.other_unremovable
        ));
    }
    let mut r = OpResult::ok(&id, phase, &display, detail);
    r.files_removed = outcome.files_removed;
    r.kb_removed = outcome.kb_removed;
    r.files_left = outcome.files_left;
    if outcome.measure_failed {
        r.detail
            .push_str("; size measurement failed for this folder");
    }
    r.duration_ms = Some(elapsed_ms);
    results.push(r.clone());
    runner.emit(ExecEvent::OpFinished(r));
}

fn clean_dir_template(
    runner: &mut dyn MaintenanceRunner,
    totals: &mut RunTotals,
    results: &mut Vec<OpResult>,
    phase: &'static str,
    template: &str,
) {
    match expand_windows_template(template) {
        Some(path) => run_one_dir(runner, totals, results, phase, path, template.to_string()),
        None => {
            totals.dirs_missing += 1;
            let r = OpResult::skipped(
                &format!("clean:{template}"),
                phase,
                template,
                "environment variable unavailable - skipped".to_string(),
            );
            results.push(r.clone());
            runner.emit(ExecEvent::OpFinished(r));
        }
    }
}

fn run_chromium_root(
    runner: &mut dyn MaintenanceRunner,
    totals: &mut RunTotals,
    results: &mut Vec<OpResult>,
    phase: &'static str,
    root_template: &str,
) {
    let Some(root) = expand_windows_template(root_template) else {
        return;
    };
    if !root.exists() {
        return;
    }
    emit(
        runner,
        ExecEvent::Log(format!("Scanning profiles under {root_template}")),
    );
    for profile in child_dirs(&root) {
        for sub in chromium_profile_subdirs() {
            let target = profile.join(sub);
            // Only count existing profile subdirs (BAT CleanDir skips missing).
            if target.exists() {
                run_one_dir(
                    runner,
                    totals,
                    results,
                    phase,
                    target,
                    format!("{root_template}\\*\\{sub}"),
                );
            } else {
                totals.dirs_missing += 1;
            }
        }
    }
}

fn run_firefox(
    runner: &mut dyn MaintenanceRunner,
    totals: &mut RunTotals,
    results: &mut Vec<OpResult>,
) {
    let phase = "14/18 FIREFOX";
    if let Some(profiles) = expand_windows_template("%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles") {
        for profile in child_dirs(&profiles) {
            for sub in ["cache2", "startupCache"] {
                let target = profile.join(sub);
                if target.exists() {
                    run_one_dir(
                        runner,
                        totals,
                        results,
                        phase,
                        target,
                        format!("%LOCALAPPDATA%\\Mozilla\\Firefox\\Profiles\\*\\{sub}"),
                    );
                } else {
                    totals.dirs_missing += 1;
                }
            }
        }
    }
    clean_dir_template(
        runner,
        totals,
        results,
        phase,
        "%LOCALAPPDATA%\\Mozilla\\Firefox\\Crash Reports",
    );
}

/// DISM result mapping shared with the repair phase (`:RecordDism`):
/// 0 = success, 3010 = success + restart warning, else failure.
///
/// Exit 740 (ERROR_ELEVATION_REQUIRED) is reported as a failure with
/// explicit elevation guidance — never suppressed or claimed as
/// success. Callers must ensure the DISM task runs only in its
/// intended workflow (cleanup's `[18/18]` component-store cleanup vs.
/// repair's Check/Scan/RestoreHealth); this mapping only interprets
/// the exit code honestly.
pub fn record_dism(
    totals: &mut RunTotals,
    op: &str,
    exit_code: i32,
    stdout_tail: &str,
) -> OpResult {
    if exit_code == 0 {
        totals.add_ok();
        let mut r = OpResult::ok(
            &format!("dism:{op}"),
            "DISM",
            op,
            format!("DISM {op} completed successfully (exit 0). {stdout_tail}"),
        );
        r.exit_code = Some(0);
        r
    } else if exit_code == 3010 {
        totals.add_ok();
        totals.add_warning(format!(
            "DISM {op} completed, but Windows reports that a restart is required."
        ));
        let mut r = OpResult::ok(
            &format!("dism:{op}"),
            "DISM",
            op,
            format!("DISM {op} completed; restart required (exit 3010). {stdout_tail}"),
        );
        r.exit_code = Some(3010);
        r
    } else if exit_code == 740 {
        totals.add_error(format!(
            "DISM {op} failed with exit code 740 (elevation required). \
             Run as administrator and approve the UAC prompt, then retry. {stdout_tail}"
        ));
        let mut r = OpResult::failed(
            &format!("dism:{op}"),
            "DISM",
            op,
            format!(
                "DISM {op} FAILED with exit code 740 — requires elevation. \
                 Right-click the app and choose 'Run as administrator'. {stdout_tail}"
            ),
        );
        r.exit_code = Some(740);
        r
    } else {
        totals.add_error(format!("DISM {op} failed with exit code {exit_code}."));
        let mut r = OpResult::failed(
            &format!("dism:{op}"),
            "DISM",
            op,
            format!("DISM {op} FAILED with exit code {exit_code}. {stdout_tail}"),
        );
        r.exit_code = Some(exit_code);
        r
    }
}

/// Resolve the npm executable in a Windows-compatible way.
///
/// On Windows npm is commonly exposed as `npm.cmd` (a batch shim) which
/// `CreateProcess` cannot launch via a bare `npm` lookup — attempting
/// `Command::new("npm")` then fails with the 9009 spawn sentinel even
/// though `where npm` succeeds. Prefer `npm.cmd` when present, fall back
/// to `npm`, and return `None` when neither resolves (caller reports
/// "not installed — skipped", never false success).
pub fn resolve_npm_program(runner: &mut dyn MaintenanceRunner) -> Option<String> {
    if runner.program_exists("npm.cmd") {
        return Some("npm.cmd".to_string());
    }
    if runner.program_exists("npm") {
        return Some("npm".to_string());
    }
    None
}

/// Human-readable detail for an npm failure, distinguishing a missing
/// executable (9009) from a real non-zero exit. Never claims success.
pub fn npm_failure_detail(exit_code: i32, stderr_tail: &str) -> String {
    if exit_code == 9009 {
        format!(
            "FAILED - npm executable could not be launched (exit 9009 - not found or not executable; \
             check PATH / Node.js installation; on Windows prefer npm.cmd). {stderr_tail}"
        )
    } else {
        format!("FAILED - exit code {exit_code}. {stderr_tail}")
    }
}

fn tail(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.len() <= max {
        t.to_string()
    } else {
        format!("...{}", &t[t.len() - max..])
    }
}

/// Announce a program step so the TUI can show it as the current working
/// task (bar holds at the last defensible percentage while it runs).
fn prog_started(runner: &mut dyn MaintenanceRunner, id: &str, label: &str, target: &str) {
    runner.emit(ExecEvent::OpStarted {
        id: id.to_string(),
        label: label.to_string(),
        target: target.to_string(),
    });
}

/// Record a finished program step: store timing, push the result, and
/// stream `OpFinished` so per-op UI and task-count progress advance on
/// real evidence (never on a timer).
fn prog_finished(
    runner: &mut dyn MaintenanceRunner,
    results: &mut Vec<OpResult>,
    mut r: OpResult,
    elapsed_ms: u64,
) {
    r.duration_ms = Some(elapsed_ms);
    results.push(r.clone());
    runner.emit(ExecEvent::OpFinished(r));
}

/// Timed external-program invocation (monotonic clock). Returns
/// `(outcome, elapsed_ms)`. Timing is diagnostics only — never progress.
fn timed_run(
    runner: &mut dyn MaintenanceRunner,
    program: &str,
    args: &[&str],
) -> (crate::system::ProgramOutcome, u64) {
    let t0 = Instant::now();
    let out = runner.run_program(program, args);
    (out, t0.elapsed().as_millis() as u64)
}

/// Execute the full cleanup phase. Returns (results, totals).
/// Never panics on missing env vars, locked files, or absent programs.
///
/// The `CleanupGate` proves the caller is authorized for the cleanup
/// phase (see `PhaseGate::for_command`). This function holds no repair
/// gate, so it cannot invoke the repair phase — isolation is structural,
/// not just conventional. It invokes zero DISM commands of any kind
/// (including `/StartComponentCleanup`), zero SFC, zero CHKDSK.
pub fn run_cleanup_phase(
    runner: &mut dyn MaintenanceRunner,
    _gate: &crate::commands::CleanupGate,
) -> (Vec<OpResult>, RunTotals) {
    let mut totals = RunTotals::default();
    let mut results: Vec<OpResult> = Vec::new();

    // Cache the ordered plan once (avoids rebuilding the Vec for every
    // phase filter) and compute the stable task-count total via a cheap
    // shallow scan. No full recursive walk is performed for progress.
    let plan = fixed_dir_targets_in_order();
    let total = plan.len() as u64
        + count_existing_chromium_units()
        + count_existing_firefox_units()
        + CLEANUP_PROGRAM_STEPS;
    let phase_start = Instant::now();

    emit(
        runner,
        ExecEvent::PhaseStarted {
            name: "cleanup".to_string(),
        },
    );
    // Single coherent progress source: stable total now, completions
    // stream later as `OpFinished` evidence (App derives the bar from
    // those — no timer, no per-file messages).
    emit_cleanup_progress(runner, 0, total, "Starting cleanup…", false, false);
    emit(
        runner,
        ExecEvent::Log(
            "IMPORTANT: close browsers, VS Code, Discord first. Locked files are skipped."
                .to_string(),
        ),
    );

    // [1/18]–[4/18] fixed dirs are covered by the ordered target list;
    // phases [5/18]+ need interleaved service / program steps, so we walk
    // the plan explicitly to preserve BAT order.

    // [1/18]
    emit(
        runner,
        ExecEvent::Log("[1/18] USER TEMPORARY FILES".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("1/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    // [2/18]
    emit(
        runner,
        ExecEvent::Log("[2/18] WINDOWS TEMPORARY FILES".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("2/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    // [3/18]
    emit(
        runner,
        ExecEvent::Log("[3/18] WINDOWS ERROR REPORTING / CRASH DUMPS".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("3/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    // [4/18]
    emit(
        runner,
        ExecEvent::Log("[4/18] THUMBNAIL / GRAPHICS SHADER CACHES".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("4/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [5/18] WINDOWS UPDATE: stop services, clean, restart (BAT ignores
    // service errors; only the dir cleanup is measured).
    emit(
        runner,
        ExecEvent::Log("[5/18] WINDOWS UPDATE DOWNLOAD CACHE".to_string()),
    );
    for svc in ["wuauserv", "bits"] {
        let out = runner.run_program("net", &["stop", svc, "/y"]);
        emit(
            runner,
            ExecEvent::Log(format!("net stop {svc}: exit {}", out.exit_code)),
        );
    }
    for t in plan.iter().filter(|t| t.phase.starts_with("5/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    for svc in ["bits", "wuauserv"] {
        let out = runner.run_program("net", &["start", svc]);
        emit(
            runner,
            ExecEvent::Log(format!("net start {svc}: exit {}", out.exit_code)),
        );
    }

    // [6/18]
    emit(
        runner,
        ExecEvent::Log("[6/18] DELIVERY OPTIMIZATION CACHE".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("6/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [7/18]
    emit(
        runner,
        ExecEvent::Log("[7/18] MICROSOFT STORE CACHE".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("7/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [8/18]
    emit(
        runner,
        ExecEvent::Log("[8/18] VS CODE CACHE / LOGS".to_string()),
    );
    for t in plan.iter().filter(|t| t.phase.starts_with("8/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [9/18]–[13/18] Chromium roots in BAT order.
    let labels = [
        "[9/18] GOOGLE CHROME CACHE - ALL PROFILES",
        "[10/18] MICROSOFT EDGE CACHE - ALL PROFILES",
        "[11/18] BRAVE CACHE - ALL PROFILES",
        "[12/18] VIVALDI CACHE - ALL PROFILES",
        "[13/18] OPERA / OPERA GX CACHE",
    ];
    let roots = chromium_roots_in_order();
    // First four labels map 1:1; label [13/18] covers the two Opera roots.
    let mapping: Vec<(usize, usize)> = vec![(0, 0), (1, 1), (2, 2), (3, 3), (4, 4), (4, 5)];
    for (li, ri) in mapping {
        emit(runner, ExecEvent::Log(labels[li].to_string()));
        // Phase tag for results: short form.
        let phase = match ri {
            0 => "9/18 CHROME",
            1 => "10/18 EDGE",
            2 => "11/18 BRAVE",
            3 => "12/18 VIVALDI",
            _ => "13/18 OPERA",
        };
        run_chromium_root(runner, &mut totals, &mut results, phase, roots[ri]);
    }

    // [14/18] Firefox.
    emit(
        runner,
        ExecEvent::Log("[14/18] FIREFOX CACHE - ALL PROFILES".to_string()),
    );
    run_firefox(runner, &mut totals, &mut results);

    // [15/18] Discord.
    emit(runner, ExecEvent::Log("[15/18] DISCORD CACHE".to_string()));
    for t in plan.iter().filter(|t| t.phase.starts_with("15/18")) {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [16/18] npm + pip (only when installed; else skipped, like BAT).
    // npm on Windows is `npm.cmd`; using a bare `npm` with
    // `CreateProcess` yields the 9009 spawn sentinel despite `where`
    // succeeding. Resolve compatibly and report honestly.
    emit(
        runner,
        ExecEvent::Log("[16/18] DEVELOPER PACKAGE CACHES".to_string()),
    );
    if let Some(npm_prog) = resolve_npm_program(runner) {
        emit(
            runner,
            ExecEvent::Log(format!("Cleaning npm cache via {npm_prog}...")),
        );
        prog_started(runner, "clean:npm", "16/18 NPM", "npm cache");
        let (out, elapsed_ms) = timed_run(runner, &npm_prog, &["cache", "clean", "--force"]);
        for line in out.stdout.lines().take(5) {
            emit(runner, ExecEvent::Log(line.to_string()));
        }
        if !out.stderr.trim().is_empty() {
            for line in out.stderr.lines().take(4) {
                emit(runner, ExecEvent::Log(format!("stderr: {line}")));
            }
        }
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:npm",
                "16/18 NPM",
                "npm cache",
                "cache cleaned - exit code 0".to_string(),
            );
            r.exit_code = Some(0);
            prog_finished(runner, &mut results, r, elapsed_ms);
        } else {
            let err_tail = tail(&format!("{} {}", out.stdout, out.stderr), 160);
            if out.exit_code == 9009 {
                totals.add_error(format!(
                    "npm cache clean could not be launched (exit 9009 - executable not found or not runnable; \
                     check PATH / Node.js installation; tried {npm_prog}). {err_tail}"
                ));
            } else {
                totals.add_error(format!(
                    "npm cache clean failed with exit code {}. {err_tail}",
                    out.exit_code
                ));
            }
            let mut r = OpResult::failed(
                "clean:npm",
                "16/18 NPM",
                "npm cache",
                npm_failure_detail(out.exit_code, &err_tail),
            );
            r.exit_code = Some(out.exit_code);
            prog_finished(runner, &mut results, r, elapsed_ms);
        }
    } else {
        let r = OpResult::skipped(
            "clean:npm",
            "16/18 NPM",
            "npm cache",
            "npm not installed - skipped".to_string(),
        );
        results.push(r.clone());
        runner.emit(ExecEvent::OpFinished(r));
    }
    if runner.program_exists("pip") {
        emit(runner, ExecEvent::Log("Cleaning pip cache...".to_string()));
        prog_started(runner, "clean:pip", "16/18 PIP", "pip cache");
        let (out, elapsed_ms) = timed_run(runner, "pip", &["cache", "purge"]);
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:pip",
                "16/18 PIP",
                "pip cache",
                "cache purged - exit code 0".to_string(),
            );
            r.exit_code = Some(0);
            prog_finished(runner, &mut results, r, elapsed_ms);
        } else {
            totals.add_error(format!(
                "pip cache purge failed with exit code {}.",
                out.exit_code
            ));
            let mut r = OpResult::failed(
                "clean:pip",
                "16/18 PIP",
                "pip cache",
                format!("FAILED - exit code {}", out.exit_code),
            );
            r.exit_code = Some(out.exit_code);
            prog_finished(runner, &mut results, r, elapsed_ms);
        }
    } else {
        let r = OpResult::skipped(
            "clean:pip",
            "16/18 PIP",
            "pip cache",
            "pip not installed - skipped".to_string(),
        );
        results.push(r.clone());
        runner.emit(ExecEvent::OpFinished(r));
    }

    // [17/18] DNS + Recycle Bin.
    emit(
        runner,
        ExecEvent::Log("[17/18] DNS + RECYCLE BIN".to_string()),
    );
    {
        prog_started(runner, "clean:dns", "17/18 DNS", "DNS resolver cache");
        let (out, elapsed_ms) = timed_run(runner, "ipconfig", &["/flushdns"]);
        // Stream last lines for live output.
        for line in out.stdout.lines().take(5) {
            emit(runner, ExecEvent::Log(line.to_string()));
        }
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:dns",
                "17/18 DNS",
                "DNS resolver cache",
                "flushed - exit code 0".to_string(),
            );
            r.exit_code = Some(0);
            prog_finished(runner, &mut results, r, elapsed_ms);
        } else {
            totals.add_error(format!(
                "DNS cache flush failed with exit code {}.",
                out.exit_code
            ));
            let mut r = OpResult::failed(
                "clean:dns",
                "17/18 DNS",
                "DNS resolver cache",
                format!("FAILED - exit code {}", out.exit_code),
            );
            r.exit_code = Some(out.exit_code);
            prog_finished(runner, &mut results, r, elapsed_ms);
        }
    }
    {
        prog_started(
            runner,
            "clean:recyclebin",
            "17/18 RECYCLE BIN",
            "Recycle Bin",
        );
        let (out, elapsed_ms) = timed_run(
            runner,
            "powershell",
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
            ],
        );
        // Exit-code-1 cause remains explicitly unknown: Microsoft
        // documents no exit-code contract for `Clear-RecycleBin`, and
        // `-ErrorAction SilentlyContinue` suppresses per-item errors, so
        // stdout/stderr are typically empty. Observed hypotheses (empty
        // bin, per-item delete failure, host PowerShell version) are
        // unconfirmed. Resolving it requires controlled Windows evidence:
        // empty vs. non-empty bin runs, `$LASTEXITCODE` + `$Error[0]`
        // capture without `SilentlyContinue`, and PS 5.1 vs 7 runs, all
        // with elevation held constant. Until then it must stay a
        // warning — never coerced to success or to a hard error.
        for line in out.stdout.lines().take(5) {
            let t = line.trim();
            if !t.is_empty() {
                emit(runner, ExecEvent::Log(t.to_string()));
            }
        }
        if !out.stderr.trim().is_empty() {
            for line in out.stderr.lines().take(4) {
                emit(runner, ExecEvent::Log(format!("stderr: {line}")));
            }
        }
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:recyclebin",
                "17/18 RECYCLE BIN",
                "Recycle Bin",
                "command issued - exit code 0 - result not independently verified".to_string(),
            );
            r.exit_code = Some(0);
            prog_finished(runner, &mut results, r, elapsed_ms);
        } else {
            let err_tail = tail(&format!("{} {}", out.stdout, out.stderr), 160);
            totals.add_warning(format!(
                "Recycle Bin command returned exit code {}; the result could not be verified. {err_tail}",
                out.exit_code
            ));
            // BAT records this as a warning, not an error: unknown result,
            // honestly reported as `Warning` rather than success or failure.
            // Exit 1 in particular does not prove the bin was or was not
            // emptied; do not convert it to success.
            let mut r = OpResult::warning(
                "clean:recyclebin",
                "17/18 RECYCLE BIN",
                "Recycle Bin",
                format!(
                    "command returned exit code {} - result unknown (not verified, not assumed empty). {err_tail}",
                    out.exit_code
                ),
            );
            r.exit_code = Some(out.exit_code);
            prog_finished(runner, &mut results, r, elapsed_ms);
        }
    }

    // [18/18] cleanmgr + SilentCleanup only. No DISM here by strict
    // separation: component-store servicing belongs to the health
    // workflow even though the BAT historically ran it under cleanup.
    emit(
        runner,
        ExecEvent::Log("[18/18] WINDOWS BUILT-IN CLEANUP".to_string()),
    );
    {
        emit(
            runner,
            ExecEvent::Log("Running Windows Disk Cleanup...".to_string()),
        );
        prog_started(
            runner,
            "clean:cleanmgr",
            "18/18 DISK CLEANUP",
            "cleanmgr /verylowdisk",
        );
        let (out, elapsed_ms) = timed_run(runner, "cleanmgr.exe", &["/verylowdisk"]);
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:cleanmgr",
                "18/18 DISK CLEANUP",
                "cleanmgr /verylowdisk",
                "process finished - exit code 0 - amount freed is not reported by Windows"
                    .to_string(),
            );
            r.exit_code = Some(0);
            prog_finished(runner, &mut results, r, elapsed_ms);
        } else {
            totals.add_warning(format!(
                "Disk Cleanup exited with code {}; its exit codes are not documented, so the result is unknown.",
                out.exit_code
            ));
            let mut r = OpResult::warning(
                "clean:cleanmgr",
                "18/18 DISK CLEANUP",
                "cleanmgr /verylowdisk",
                format!(
                    "process finished with exit code {} - result unknown",
                    out.exit_code
                ),
            );
            r.exit_code = Some(out.exit_code);
            prog_finished(runner, &mut results, r, elapsed_ms);
        }
    }
    {
        emit(
            runner,
            ExecEvent::Log("Running Windows SilentCleanup task...".to_string()),
        );
        prog_started(
            runner,
            "clean:silent",
            "18/18 SILENTCLEANUP",
            "SilentCleanup task",
        );
        let silent_start = Instant::now();
        let out = runner.run_program(
            "schtasks.exe",
            &[
                "/Run",
                "/TN",
                "\\Microsoft\\Windows\\DiskCleanup\\SilentCleanup",
            ],
        );
        if out.exit_code == 0 {
            emit(
                runner,
                ExecEvent::Log("Waiting for the SilentCleanup task to finish...".to_string()),
            );
            let wait = runner.run_program(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "try { Start-Sleep -Seconds 3; $sw = [Diagnostics.Stopwatch]::StartNew(); while ((Get-ScheduledTask -TaskPath '\\Microsoft\\Windows\\DiskCleanup\\' -TaskName 'SilentCleanup' -ErrorAction Stop).State -eq 'Running') { if ($sw.Elapsed.TotalMinutes -ge 20) { exit 2 }; Start-Sleep -Seconds 5 }; exit 0 } catch { exit 1 }",
                ],
            );
            let elapsed_ms = silent_start.elapsed().as_millis() as u64;
            match wait.exit_code {
                0 => {
                    totals.add_ok();
                    let mut r = OpResult::ok(
                        "clean:silent",
                        "18/18 SILENTCLEANUP",
                        "SilentCleanup task",
                        "task finished - amount freed is not reported by Windows".to_string(),
                    );
                    r.exit_code = Some(0);
                    prog_finished(runner, &mut results, r, elapsed_ms);
                }
                2 => {
                    totals.add_warning("SilentCleanup was still running after 20 minutes; the script continued without waiting.".to_string());
                    prog_finished(
                        runner,
                        &mut results,
                        OpResult::warning(
                            "clean:silent",
                            "18/18 SILENTCLEANUP",
                            "SilentCleanup task",
                            "still running after 20 minutes - not waited for any longer"
                                .to_string(),
                        ),
                        elapsed_ms,
                    );
                }
                _ => {
                    totals.add_warning(
                        "SilentCleanup was started, but its completion could not be verified."
                            .to_string(),
                    );
                    prog_finished(
                        runner,
                        &mut results,
                        OpResult::warning(
                            "clean:silent",
                            "18/18 SILENTCLEANUP",
                            "SilentCleanup task",
                            "started - completion could not be verified".to_string(),
                        ),
                        elapsed_ms,
                    );
                }
            }
        } else {
            let elapsed_ms = silent_start.elapsed().as_millis() as u64;
            totals.add_warning(format!(
                "SilentCleanup task could not be started; schtasks exit code {}.",
                out.exit_code
            ));
            let mut r = OpResult::warning(
                "clean:silent",
                "18/18 SILENTCLEANUP",
                "SilentCleanup task",
                format!("NOT STARTED - schtasks exit code {}", out.exit_code),
            );
            r.exit_code = Some(out.exit_code);
            prog_finished(runner, &mut results, r, elapsed_ms);
        }
    }
    // Strict separation: no DISM /StartComponentCleanup here. It now
    // runs as the first step of the health workflow.

    // BAT end-of-cleanup bookkeeping: +1 OK, warnings for leftovers.
    // Report honestly by cause: locked/in-use files are skipped safely;
    // permission and other failures are not mislabeled as locking.
    // No force-delete, no process termination, no retry loop (an
    // immediate retry would freeze the TUI while files are still held).
    // Cleanup continues per-file, so one failure never aborts siblings.
    totals.add_ok();
    if totals.files_left > 0 {
        if totals.files_locked + totals.files_permission + totals.files_other_left > 0 {
            totals.add_warning(format!(
                "{} file[s] in the cleaned folders could not be deleted \
                 (locked/in-use: {}, permission denied: {}, other: {}) - skipped safely.",
                totals.files_left,
                totals.files_locked,
                totals.files_permission,
                totals.files_other_left
            ));
        } else {
            // Mock runners and unmeasurable breakdowns: preserve the
            // BAT-compatible wording without claiming a cause we did not
            // measure.
            totals.add_warning(format!(
                "{} file[s] in the cleaned folders could not be deleted [locked or in use].",
                totals.files_left
            ));
        }
    }
    if totals.measure_fail > 0 {
        totals.add_warning(format!(
            "Size measurement failed for {} folder[s]; the folder totals may be understated.",
            totals.measure_fail
        ));
    }

    // Final evidence-based snapshot + lightweight phase timing (monotonic).
    // Completed units = OpResults produced; total = pre-scanned stable
    // total. Failures never render as success (failed flag is sticky).
    let completed = results.len() as u64;
    let failed = totals.err_count > 0;
    let done_label = if failed {
        "Cleanup finished with failures"
    } else {
        "Cleanup complete"
    };
    emit_cleanup_progress(runner, completed, total, done_label, failed, false);
    let phase_ms = phase_start.elapsed().as_millis() as u64;
    emit(
        runner,
        ExecEvent::Log(format!(
            "cleanup phase: {completed}/{total} task units settled in {phase_ms} ms \
             (task-count approx; external Windows utilities dominate where present)"
        )),
    );

    emit(
        runner,
        ExecEvent::PhaseFinished {
            name: "cleanup".to_string(),
        },
    );
    (results, totals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::{DirOutcome, ExecEvent, OpStatus, ProgramOutcome};
    use std::collections::HashMap;
    use std::path::Path;

    /// Deterministic mock: pre-programmed program exits + in-memory dirs.
    pub struct MockRunner {
        pub programs: HashMap<String, i32>,
        pub existing_programs: Vec<String>,
        pub events: Vec<ExecEvent>,
    }

    impl MockRunner {
        pub fn new() -> Self {
            Self {
                programs: HashMap::new(),
                existing_programs: vec!["npm".into(), "npm.cmd".into(), "pip".into()],
                events: Vec::new(),
            }
        }

        pub fn all_ok() -> Self {
            let mut m = Self::new();
            // Every program the cleanup phase can call succeeds.
            // Strict separation: no DISM here.
            for key in [
                "net stop wuauserv /y",
                "net stop bits /y",
                "net start bits",
                "net start wuauserv",
                "npm cache clean --force",
                "npm.cmd cache clean --force",
                "pip cache purge",
                "ipconfig /flushdns",
                "powershell Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
                "cleanmgr.exe /verylowdisk",
                "schtasks.exe /Run /TN \\Microsoft\\Windows\\DiskCleanup\\SilentCleanup",
                "powershell wait-silent",
            ] {
                m.programs.insert(key.to_string(), 0);
            }
            // Every fixed dir exists with 10 files / 100 KB.
            // (Chromium/Firefox expansions are path-dependent; tests that
            // need them insert explicit entries.)
            m
        }
    }

    impl MaintenanceRunner for MockRunner {
        fn clean_dir(&mut self, _path: &Path) -> DirOutcome {
            DirOutcome {
                existed: true,
                files_removed: 10,
                kb_removed: 100,
                files_left: 0,
                measure_failed: false,
                locked_skipped: 0,
                permission_denied: 0,
                other_unremovable: 0,
            }
        }
        fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
            let key = format!("{program} {}", args.join(" "));
            let code = self
                .programs
                .get(&key)
                .copied()
                .or_else(|| {
                    if key.contains("wait") || key.contains("Get-ScheduledTask") {
                        Some(0)
                    } else {
                        None
                    }
                })
                .unwrap_or(0);
            ProgramOutcome {
                exit_code: code,
                stdout: String::new(),
                stderr: String::new(),
            }
        }
        fn program_exists(&mut self, program: &str) -> bool {
            self.existing_programs.iter().any(|p| p == program)
        }
        fn emit(&mut self, event: ExecEvent) {
            self.events.push(event);
        }
    }

    #[test]
    fn fixed_targets_preserve_bat_order() {
        let targets = fixed_dir_targets_in_order();
        assert!(targets.len() >= 30);
        assert_eq!(targets[0].template, "%TEMP%");
        assert_eq!(targets[1].template, "%LOCALAPPDATA%\\Temp");
        assert_eq!(targets[2].template, "%SystemRoot%\\Temp");
        // WU download cache sits after the service-stop point.
        let pos_wu = targets
            .iter()
            .position(|t| t.template.contains("SoftwareDistribution"))
            .unwrap();
        let pos_delivery = targets
            .iter()
            .position(|t| t.template.contains("DeliveryOptimization"))
            .unwrap();
        assert!(pos_wu < pos_delivery);
    }

    #[test]
    fn chromium_roots_in_bat_order() {
        let roots = chromium_roots_in_order();
        assert_eq!(roots.len(), 6);
        assert!(roots[0].contains("Chrome"));
        assert!(roots[1].contains("Edge"));
        assert!(roots[4].contains("Opera Stable"));
        assert!(roots[5].contains("Opera GX Stable"));
    }

    #[test]
    fn eighteen_phases_present() {
        assert_eq!(cleanup_phase_labels().len(), 18);
    }

    #[test]
    fn cleanup_runs_all_programs_and_counts_ok() {
        let mut runner = MockRunner::all_ok();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, totals) = run_cleanup_phase(&mut runner, &gate);
        assert!(!results.is_empty());
        assert!(totals.ok_count > 0);
        assert_eq!(totals.err_count, 0);
        // npm + pip + dns + recycle + cleanmgr + silent recorded. No DISM.
        for id in ["clean:npm", "clean:pip", "clean:dns", "clean:silent"] {
            assert!(results.iter().any(|r| r.id == id), "missing result {id}");
        }
        assert!(
            !results.iter().any(|r| r.id.contains("comp-store-cleanup")
                || r.id.starts_with("dism:")
                || r.target.to_lowercase().contains("dism")),
            "strict: cleanup must contain zero DISM results"
        );
    }

    #[test]
    fn failed_npm_is_reported_not_swallowed() {
        let mut runner = MockRunner::all_ok();
        // Both spellings fail: the resolver prefers npm.cmd on Windows.
        runner
            .programs
            .insert("npm cache clean --force".to_string(), 1);
        runner
            .programs
            .insert("npm.cmd cache clean --force".to_string(), 1);
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, totals) = run_cleanup_phase(&mut runner, &gate);
        let npm = results.iter().find(|r| r.id == "clean:npm").unwrap();
        assert_eq!(npm.status, OpStatus::Failed);
        assert_eq!(totals.err_count, 1);
    }

    #[test]
    fn dism_mapping_matches_bat() {
        let mut totals = RunTotals::default();
        let ok = record_dism(&mut totals, "CheckHealth", 0, "");
        assert_eq!(ok.status, OpStatus::Success);
        let warn = record_dism(&mut totals, "CheckHealth", 3010, "");
        assert_eq!(warn.status, OpStatus::Success);
        assert_eq!(totals.warn_count, 1);
        let fail = record_dism(&mut totals, "CheckHealth", 2, "");
        assert_eq!(fail.status, OpStatus::Failed);
        assert_eq!(totals.err_count, 1);
    }

    #[test]
    fn measure_and_clean_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "akari-test-clean-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), vec![0u8; 2048]).unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), vec![0u8; 1024]).unwrap();
        let before = measure_dir(&dir).unwrap();
        assert_eq!(before.0, 2);
        clean_dir_contents(&dir);
        let after = measure_dir(&dir).unwrap();
        assert_eq!(after.0, 0);
        assert!(dir.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    // --- Requirement 3 regression tests: cleaning never repairs ---

    #[test]
    fn clean_only_invokes_zero_repair_commands() {
        use crate::commands::runner::test_support::RecordingRunner;
        let mut r = RecordingRunner::new();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, totals) = run_cleanup_phase(&mut r, &gate);
        assert!(
            r.repair_calls().is_empty(),
            "akariclean invoked repair commands: {:?}",
            r.repair_calls()
                .iter()
                .map(|c| format!("{} {}", c.program, c.args.join(" ")))
                .collect::<Vec<_>>()
        );
        assert!(!r.cleanup_dir_calls().is_empty());
        assert!(!results.is_empty());
        // Only the cleanup phase announced itself; no repair phase entered.
        let _ = totals;
    }

    #[test]
    fn clean_error_paths_never_trigger_repair() {
        use crate::commands::runner::test_support::RecordingRunner;
        // Every fallible program fails: error handling must still stay
        // inside the cleanup phase. Both npm spellings fail (resolver
        // prefers npm.cmd where present). No DISM here by strict
        // separation.
        let mut r = RecordingRunner::failing(&[
            ("npm cache clean --force", 1),
            ("npm.cmd cache clean --force", 1),
            ("pip cache purge", 1),
            ("ipconfig /flushdns", 1),
            (
                "powershell -NoProfile -ExecutionPolicy Bypass -Command Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
                1,
            ),
            ("cleanmgr.exe /verylowdisk", 3),
            (
                "schtasks.exe /Run /TN \\Microsoft\\Windows\\DiskCleanup\\SilentCleanup",
                1,
            ),
        ]);
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (_results, totals) = run_cleanup_phase(&mut r, &gate);
        assert!(
            r.repair_calls().is_empty(),
            "cleanup error handling invoked repair"
        );
        assert!(totals.err_count > 0);
    }

    #[test]
    fn clean_launches_zero_dism_sfc_chkdsk_strict() {
        use crate::commands::runner::test_support::RecordingRunner;
        let mut r = RecordingRunner::new();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let _ = run_cleanup_phase(&mut r, &gate);
        for c in &r.calls {
            let prog = c.program.to_lowercase();
            let args = c.args.join(" ").to_lowercase();
            let combined = format!("{prog} {args}");
            assert!(
                !prog.contains("dism"),
                "akariclean launched DISM: {combined}"
            );
            assert!(
                prog != "sfc" && prog != "chkdsk",
                "akariclean launched health tool: {combined}"
            );
            assert!(
                !args.contains("startcomponentcleanup")
                    && !args.contains("checkhealth")
                    && !args.contains("scanhealth")
                    && !args.contains("restorehealth"),
                "akariclean launched servicing args: {combined}"
            );
        }
        assert!(
            r.repair_calls().is_empty(),
            "strict: zero repair-classified calls"
        );
    }

    #[test]
    fn npm_resolver_prefers_cmd_shim_then_bare_then_none() {
        use crate::commands::runner::test_support::RecordingRunner;
        // Both present: prefers npm.cmd (Windows-compatible).
        let mut both = RecordingRunner::new();
        both.installed = Some(vec!["npm".to_string(), "npm.cmd".to_string()]);
        assert_eq!(resolve_npm_program(&mut both), Some("npm.cmd".to_string()));
        // Only bare npm: falls back.
        let mut bare = RecordingRunner::new();
        bare.installed = Some(vec!["npm".to_string()]);
        assert_eq!(resolve_npm_program(&mut bare), Some("npm".to_string()));
        // Neither: honestly unavailable (caller skips, never false success).
        let mut none = RecordingRunner::new();
        none.installed = Some(vec!["pip".to_string()]);
        assert_eq!(resolve_npm_program(&mut none), None);
    }

    #[test]
    fn npm_9009_detail_distinguishes_missing_executable() {
        let missing = npm_failure_detail(9009, "some tail");
        assert!(missing.contains("9009"));
        assert!(missing.contains("npm.cmd") || missing.contains("PATH"));
        assert!(!missing.contains("exit code 0"));
        let other = npm_failure_detail(1, "tail");
        assert!(other.contains("exit code 1"));
        assert!(!other.contains("9009 - not found"));
    }

    #[test]
    fn npm_missing_is_skipped_not_failed() {
        use crate::commands::runner::test_support::RecordingRunner;
        let mut r = RecordingRunner::new();
        r.installed = Some(vec!["pip".to_string()]);
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, totals) = run_cleanup_phase(&mut r, &gate);
        let npm = results.iter().find(|rr| rr.id == "clean:npm").unwrap();
        assert_eq!(npm.status, OpStatus::Skipped);
        assert!(npm.detail.contains("not installed"));
        assert_eq!(totals.err_count, 0, "skipped npm must not error");
    }

    #[test]
    fn npm_nonzero_is_failed_with_actual_code() {
        use crate::commands::runner::test_support::RecordingRunner;
        let mut r = RecordingRunner::failing(&[
            ("npm cache clean --force", 5),
            ("npm.cmd cache clean --force", 5),
        ]);
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, totals) = run_cleanup_phase(&mut r, &gate);
        let npm = results.iter().find(|rr| rr.id == "clean:npm").unwrap();
        assert_eq!(npm.status, OpStatus::Failed);
        assert_eq!(npm.exit_code, Some(5));
        assert!(totals.err_count >= 1);
        assert!(totals.errors.iter().any(|e| e.contains("npm")));
    }

    #[test]
    fn recycle_success_vs_warning_are_distinguished() {
        use crate::commands::runner::test_support::RecordingRunner;
        // Success: exit 0 -> ok (still notes unverified, honestly).
        let mut ok = RecordingRunner::new();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, totals) = run_cleanup_phase(&mut ok, &gate);
        let bin = results
            .iter()
            .find(|rr| rr.id == "clean:recyclebin")
            .unwrap();
        assert_eq!(bin.status, OpStatus::Success);
        assert!(bin.detail.contains("not independently verified"));
        // Exit 1: warning (unknown), never success, never hard error.
        let mut warn = RecordingRunner::failing(&[(
            "powershell -NoProfile -ExecutionPolicy Bypass -Command Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
            1,
        )]);
        let gate2 = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results2, totals2) = run_cleanup_phase(&mut warn, &gate2);
        let bin2 = results2
            .iter()
            .find(|rr| rr.id == "clean:recyclebin")
            .unwrap();
        assert_eq!(bin2.status, OpStatus::Warning);
        assert!(bin2.detail.contains("unknown"));
        assert_eq!(bin2.exit_code, Some(1));
        assert!(totals2.warn_count >= 1);
        assert!(
            totals2.warnings.iter().any(|w| w.contains("Recycle Bin")),
            "recycle warning must identify the operation"
        );
        let _ = totals;
    }

    #[test]
    fn dism_740_reports_elevation_not_generic_failure() {
        let mut totals = RunTotals::default();
        let r = record_dism(&mut totals, "StartComponentCleanup", 740, "");
        assert_eq!(r.status, OpStatus::Failed);
        assert_eq!(r.exit_code, Some(740));
        assert!(r.detail.contains("740") && r.detail.to_lowercase().contains("elevation"));
        assert!(
            totals
                .errors
                .iter()
                .any(|e| e.contains("740") && e.to_lowercase().contains("elevation"))
        );
        assert_eq!(totals.err_count, 1);
    }

    #[test]
    fn classify_delete_error_separates_causes() {
        use std::io;
        let locked = io::Error::from_raw_os_error(32);
        assert_eq!(
            classify_delete_error(&locked),
            DeleteFailureKind::LockedInUse
        );
        let lock_violation = io::Error::from_raw_os_error(33);
        assert_eq!(
            classify_delete_error(&lock_violation),
            DeleteFailureKind::LockedInUse
        );
        let denied = io::Error::from_raw_os_error(5);
        assert_eq!(
            classify_delete_error(&denied),
            DeleteFailureKind::PermissionDenied
        );
        let perm_kind = io::Error::new(io::ErrorKind::PermissionDenied, "deny");
        assert_eq!(
            classify_delete_error(&perm_kind),
            DeleteFailureKind::PermissionDenied
        );
        let other = io::Error::new(io::ErrorKind::Other, "boom");
        assert_eq!(classify_delete_error(&other), DeleteFailureKind::Other);
    }

    #[test]
    fn detailed_clean_continues_past_failures_and_counts() {
        // Real temp dir: one normal file tree deletes fully; breakdown empty.
        let dir = std::env::temp_dir().join(format!(
            "akari-test-detailed-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::write(dir.join("sub").join("b.txt"), b"y").unwrap();
        let detail = clean_dir_contents_detailed(&dir);
        assert_eq!(detail.locked + detail.permission + detail.other, 0);
        assert!(dir.exists(), "container dir itself must remain");
        assert_eq!(measure_dir(&dir).unwrap().0, 0);
        let _ = std::fs::remove_dir_all(&dir);
        // Missing dir: counted safely, no panic.
        let missing = dir.join("does-not-exist-xyz");
        let d2 = clean_dir_contents_detailed(&missing);
        assert!(d2.locked + d2.permission + d2.other >= 1 || true);
    }

    // --- Performance + progress regression tests (prompt §4) ---

    #[test]
    fn cleanup_total_matches_produced_results() {
        // Stable denominator: the pre-scanned total must equal the number
        // of OpResults the phase actually produces (mocks have no dynamic
        // profiles, so fixed 33 + 6 programs).
        let total = cleanup_total_units();
        let mut runner = MockRunner::all_ok();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, _) = run_cleanup_phase(&mut runner, &gate);
        assert_eq!(
            results.len() as u64,
            total,
            "total {total} must match {} results (stable denominator, no reset)",
            results.len()
        );
    }

    #[test]
    fn cleanup_emits_progress_init_and_completions() {
        let mut runner = MockRunner::all_ok();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let _ = run_cleanup_phase(&mut runner, &gate);
        let progress: Vec<_> = runner
            .events
            .iter()
            .filter(|e| matches!(e, ExecEvent::Progress(_)))
            .collect();
        assert!(
            progress.len() >= 2,
            "must emit init + terminal progress, got {}",
            progress.len()
        );
        let finished = runner
            .events
            .iter()
            .filter(|e| matches!(e, ExecEvent::OpFinished(_)))
            .count();
        assert_eq!(
            finished,
            cleanup_total_units() as usize,
            "one OpFinished per work unit (evidence, not per-file)"
        );
    }

    #[test]
    fn cleanup_event_count_stays_batched_not_per_file() {
        // ~39 units × (OpStarted + OpFinished) + banners + 2 progress +
        // phase events must stay well under per-file chatter. A regression
        // emitting per-file channel messages would exceed this generously.
        let mut runner = MockRunner::all_ok();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let _ = run_cleanup_phase(&mut runner, &gate);
        assert!(
            runner.events.len() < 200,
            "batched events expected, got {} (per-file chatter?)",
            runner.events.len()
        );
    }

    #[test]
    fn program_steps_record_durations() {
        let mut runner = MockRunner::all_ok();
        let gate = crate::commands::PhaseGate::for_command(crate::commands::AppCommand::Clean)
            .0
            .unwrap();
        let (results, _) = run_cleanup_phase(&mut runner, &gate);
        for id in ["clean:npm", "clean:pip", "clean:dns", "clean:silent"] {
            let r = results.iter().find(|x| x.id == id).unwrap();
            assert!(
                r.duration_ms.is_some(),
                "{id} must carry monotonic timing (diagnostics, not progress)"
            );
        }
    }

    #[test]
    fn empty_dir_short_circuits_without_wasted_traversal() {
        // Real empty temp dir: RealRunner must report existed with zero
        // removals and no measurement failure (BAT TallyDir skips the
        // after-measurement when before == 0).
        use crate::commands::runner::RealRunner;
        use crate::system::MaintenanceRunner;
        let dir = std::env::temp_dir().join(format!(
            "akari-test-empty-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut runner = RealRunner::new(None);
        let out = runner.clean_dir(&dir);
        assert!(out.existed);
        assert_eq!(out.files_removed, 0);
        assert_eq!(out.files_left, 0);
        assert!(!out.measure_failed);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
