//! Real side-effect runner: filesystem + subprocesses + event fan-out.

use crate::commands::clean::{clean_dir_contents, measure_dir};
use crate::system::process::{program_exists, run_program_capture};
use crate::system::{DirOutcome, EventTx, ExecEvent, MaintenanceRunner, ProgramOutcome};
use std::path::Path;

/// Production runner. `clean_dir` mirrors BAT `:CleanDir`/`:TallyDir`;
/// locked files are skipped, measurements are best-effort.
pub struct RealRunner {
    tx: Option<EventTx>,
}

impl RealRunner {
    pub fn new(tx: Option<EventTx>) -> Self {
        Self { tx }
    }
}

impl MaintenanceRunner for RealRunner {
    fn clean_dir(&mut self, path: &Path) -> DirOutcome {
        if !path.exists() {
            return DirOutcome {
                existed: false,
                files_removed: 0,
                kb_removed: 0,
                files_left: 0,
                measure_failed: false,
            };
        }
        let before = measure_dir(path);
        clean_dir_contents(path);
        let after = measure_dir(path);
        match (before, after) {
            (Some((b_items, b_kb)), Some((a_items, _a_kb))) => {
                let removed = b_items.saturating_sub(a_items);
                // KB remeasured from scratch (BAT diffs KB the same way).
                let after_kb = measure_dir(path).map(|(_, kb)| kb).unwrap_or(0);
                let kb_removed = b_kb.saturating_sub(after_kb);
                DirOutcome {
                    existed: true,
                    files_removed: removed,
                    kb_removed,
                    files_left: a_items,
                    measure_failed: false,
                }
            }
            _ => DirOutcome {
                existed: true,
                files_removed: 0,
                kb_removed: 0,
                files_left: 0,
                measure_failed: true,
            },
        }
    }

    fn run_program(&mut self, program: &str, args: &[&str]) -> ProgramOutcome {
        run_program_capture(program, args)
    }

    fn program_exists(&mut self, program: &str) -> bool {
        program_exists(program)
    }

    fn emit(&mut self, event: ExecEvent) {
        if let Some(tx) = &self.tx {
            let _ = tx.send(event);
        }
    }
}
