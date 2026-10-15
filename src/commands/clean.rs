//! Cleanup phase — faithful port of BAT `:PhaseCleanup` ([1/18]–[18/18]).
//!
//! Authority: `Akari-Sweeps.bat`. No extra deletion targets. Every
//! `CleanDir` call below appears in the same order as the script.

use crate::system::{
    ExecEvent, MaintenanceRunner, OpResult, OpStatus, RunTotals, expand_windows_template,
};
use std::path::{Path, PathBuf};

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

/// Delete directory *contents* but leave the directory itself,
/// mirroring `del /f /s /q + rd` with locked files skipped silently.
pub fn clean_dir_contents(path: &Path) {
    let entries = match std::fs::read_dir(path) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };
        let p = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(_) => {
                let _ = std::fs::remove_file(&p);
                continue;
            }
        };
        if ft.is_dir() {
            let _ = std::fs::remove_dir_all(&p);
        } else {
            let _ = std::fs::remove_file(&p);
        }
    }
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
    let outcome = runner.clean_dir(&path);
    if !outcome.existed {
        totals.dirs_missing += 1;
        let mut r = OpResult::skipped(&id, phase, &display, "not present - skipped".to_string());
        r.target = display.clone();
        results.push(r);
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
    let mut r = OpResult::ok(
        &id,
        phase,
        &display,
        format!(
            "removed {} files ({} KB); {} left",
            outcome.files_removed, outcome.kb_removed, outcome.files_left
        ),
    );
    r.files_removed = outcome.files_removed;
    r.kb_removed = outcome.kb_removed;
    r.files_left = outcome.files_left;
    if outcome.measure_failed {
        r.detail
            .push_str("; size measurement failed for this folder");
    }
    results.push(r);
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
            results.push(OpResult::skipped(
                &format!("clean:{template}"),
                phase,
                template,
                "environment variable unavailable - skipped".to_string(),
            ));
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

fn tail(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.len() <= max {
        t.to_string()
    } else {
        format!("...{}", &t[t.len() - max..])
    }
}

/// Execute the full cleanup phase. Returns (results, totals).
/// Never panics on missing env vars, locked files, or absent programs.
pub fn run_cleanup_phase(runner: &mut dyn MaintenanceRunner) -> (Vec<OpResult>, RunTotals) {
    let mut totals = RunTotals::default();
    let mut results: Vec<OpResult> = Vec::new();

    emit(
        runner,
        ExecEvent::PhaseStarted {
            name: "cleanup".to_string(),
        },
    );
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
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("1/18"))
    {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    // [2/18]
    emit(
        runner,
        ExecEvent::Log("[2/18] WINDOWS TEMPORARY FILES".to_string()),
    );
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("2/18"))
    {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    // [3/18]
    emit(
        runner,
        ExecEvent::Log("[3/18] WINDOWS ERROR REPORTING / CRASH DUMPS".to_string()),
    );
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("3/18"))
    {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }
    // [4/18]
    emit(
        runner,
        ExecEvent::Log("[4/18] THUMBNAIL / GRAPHICS SHADER CACHES".to_string()),
    );
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("4/18"))
    {
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
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("5/18"))
    {
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
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("6/18"))
    {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [7/18]
    emit(
        runner,
        ExecEvent::Log("[7/18] MICROSOFT STORE CACHE".to_string()),
    );
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("7/18"))
    {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [8/18]
    emit(
        runner,
        ExecEvent::Log("[8/18] VS CODE CACHE / LOGS".to_string()),
    );
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("8/18"))
    {
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
    for t in fixed_dir_targets_in_order()
        .iter()
        .filter(|t| t.phase.starts_with("15/18"))
    {
        clean_dir_template(runner, &mut totals, &mut results, t.phase, t.template);
    }

    // [16/18] npm + pip (only when installed; else skipped, like BAT).
    emit(
        runner,
        ExecEvent::Log("[16/18] DEVELOPER PACKAGE CACHES".to_string()),
    );
    if runner.program_exists("npm") {
        emit(runner, ExecEvent::Log("Cleaning npm cache...".to_string()));
        let out = runner.run_program("npm", &["cache", "clean", "--force"]);
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:npm",
                "16/18 NPM",
                "npm cache",
                "cache cleaned - exit code 0".to_string(),
            );
            r.exit_code = Some(0);
            results.push(r);
        } else {
            totals.add_error(format!(
                "npm cache clean failed with exit code {}.",
                out.exit_code
            ));
            let mut r = OpResult::failed(
                "clean:npm",
                "16/18 NPM",
                "npm cache",
                format!("FAILED - exit code {}", out.exit_code),
            );
            r.exit_code = Some(out.exit_code);
            results.push(r);
        }
    } else {
        results.push(OpResult::skipped(
            "clean:npm",
            "16/18 NPM",
            "npm cache",
            "npm not installed - skipped".to_string(),
        ));
    }
    if runner.program_exists("pip") {
        emit(runner, ExecEvent::Log("Cleaning pip cache...".to_string()));
        let out = runner.run_program("pip", &["cache", "purge"]);
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:pip",
                "16/18 PIP",
                "pip cache",
                "cache purged - exit code 0".to_string(),
            );
            r.exit_code = Some(0);
            results.push(r);
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
            results.push(r);
        }
    } else {
        results.push(OpResult::skipped(
            "clean:pip",
            "16/18 PIP",
            "pip cache",
            "pip not installed - skipped".to_string(),
        ));
    }

    // [17/18] DNS + Recycle Bin.
    emit(
        runner,
        ExecEvent::Log("[17/18] DNS + RECYCLE BIN".to_string()),
    );
    {
        let out = runner.run_program("ipconfig", &["/flushdns"]);
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
            results.push(r);
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
            results.push(r);
        }
    }
    {
        let out = runner.run_program(
            "powershell",
            &[
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
            ],
        );
        if out.exit_code == 0 {
            totals.add_ok();
            let mut r = OpResult::ok(
                "clean:recyclebin",
                "17/18 RECYCLE BIN",
                "Recycle Bin",
                "command issued - exit code 0 - result not independently verified".to_string(),
            );
            r.exit_code = Some(0);
            results.push(r);
        } else {
            totals.add_warning(format!(
                "Recycle Bin command returned exit code {}; the result could not be verified.",
                out.exit_code
            ));
            let mut r = OpResult::failed(
                "clean:recyclebin",
                "17/18 RECYCLE BIN",
                "Recycle Bin",
                format!(
                    "command returned exit code {} - result unknown",
                    out.exit_code
                ),
            );
            r.exit_code = Some(out.exit_code);
            r.status = OpStatus::Failed;
            // BAT records this as warning, not error: downgrade.
            totals.err_count = totals.err_count.saturating_sub(0);
            results.push(r);
        }
    }

    // [18/18] cleanmgr + SilentCleanup + DISM StartComponentCleanup.
    emit(
        runner,
        ExecEvent::Log("[18/18] WINDOWS BUILT-IN CLEANUP".to_string()),
    );
    {
        emit(
            runner,
            ExecEvent::Log("Running Windows Disk Cleanup...".to_string()),
        );
        let out = runner.run_program("cleanmgr.exe", &["/verylowdisk"]);
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
            results.push(r);
        } else {
            totals.add_warning(format!(
                "Disk Cleanup exited with code {}; its exit codes are not documented, so the result is unknown.",
                out.exit_code
            ));
            let mut r = OpResult::failed(
                "clean:cleanmgr",
                "18/18 DISK CLEANUP",
                "cleanmgr /verylowdisk",
                format!(
                    "process finished with exit code {} - result unknown",
                    out.exit_code
                ),
            );
            r.exit_code = Some(out.exit_code);
            results.push(r);
        }
    }
    {
        emit(
            runner,
            ExecEvent::Log("Running Windows SilentCleanup task...".to_string()),
        );
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
                    results.push(r);
                }
                2 => {
                    totals.add_warning("SilentCleanup was still running after 20 minutes; the script continued without waiting.".to_string());
                    results.push(OpResult::failed(
                        "clean:silent",
                        "18/18 SILENTCLEANUP",
                        "SilentCleanup task",
                        "still running after 20 minutes - not waited for any longer".to_string(),
                    ));
                }
                _ => {
                    totals.add_warning(
                        "SilentCleanup was started, but its completion could not be verified."
                            .to_string(),
                    );
                    results.push(OpResult::failed(
                        "clean:silent",
                        "18/18 SILENTCLEANUP",
                        "SilentCleanup task",
                        "started - completion could not be verified".to_string(),
                    ));
                }
            }
        } else {
            totals.add_warning(format!(
                "SilentCleanup task could not be started; schtasks exit code {}.",
                out.exit_code
            ));
            results.push(OpResult::failed(
                "clean:silent",
                "18/18 SILENTCLEANUP",
                "SilentCleanup task",
                format!("NOT STARTED - schtasks exit code {}", out.exit_code),
            ));
        }
    }
    {
        emit(
            runner,
            ExecEvent::Log("Running Windows component-store cleanup...".to_string()),
        );
        let out = runner.run_program(
            "DISM.exe",
            &["/Online", "/Cleanup-Image", "/StartComponentCleanup"],
        );
        for line in out.stdout.lines().take(8) {
            emit(runner, ExecEvent::Log(line.to_string()));
        }
        let r = record_dism(
            &mut totals,
            "StartComponentCleanup",
            out.exit_code,
            &tail(&out.stdout, 160),
        );
        results.push(r);
    }

    // BAT end-of-cleanup bookkeeping: +1 OK, warnings for leftovers.
    totals.add_ok();
    if totals.files_left > 0 {
        totals.add_warning(format!(
            "{} file[s] in the cleaned folders could not be deleted [locked or in use].",
            totals.files_left
        ));
    }
    if totals.measure_fail > 0 {
        totals.add_warning(format!(
            "Size measurement failed for {} folder[s]; the folder totals may be understated.",
            totals.measure_fail
        ));
    }

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
    use crate::system::{DirOutcome, ExecEvent, ProgramOutcome};
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
                existing_programs: vec!["npm".into(), "pip".into()],
                events: Vec::new(),
            }
        }

        pub fn all_ok() -> Self {
            let mut m = Self::new();
            // Every program the cleanup phase can call succeeds.
            for key in [
                "net stop wuauserv /y",
                "net stop bits /y",
                "net start bits",
                "net start wuauserv",
                "npm cache clean --force",
                "pip cache purge",
                "ipconfig /flushdns",
                "powershell Clear-RecycleBin -Force -ErrorAction SilentlyContinue",
                "cleanmgr.exe /verylowdisk",
                "schtasks.exe /Run /TN \\Microsoft\\Windows\\DiskCleanup\\SilentCleanup",
                "powershell wait-silent",
                "DISM.exe /Online /Cleanup-Image /StartComponentCleanup",
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
        let (results, totals) = run_cleanup_phase(&mut runner);
        assert!(!results.is_empty());
        assert!(totals.ok_count > 0);
        assert_eq!(totals.err_count, 0);
        // npm + pip + dns + recycle + cleanmgr + silent + dism recorded.
        for id in ["clean:npm", "clean:pip", "clean:dns", "clean:silent"] {
            assert!(results.iter().any(|r| r.id == id), "missing result {id}");
        }
    }

    #[test]
    fn failed_npm_is_reported_not_swallowed() {
        let mut runner = MockRunner::all_ok();
        runner
            .programs
            .insert("npm cache clean --force".to_string(), 1);
        let (results, totals) = run_cleanup_phase(&mut runner);
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
}
