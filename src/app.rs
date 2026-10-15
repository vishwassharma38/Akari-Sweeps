//! Application state, screen navigation, and worker-event handling.

use crate::akari::animation::{AnimationMode, AnimationSet, Animator};
use crate::akari::sprites::{SpriteFrame, load_sweep_frames_best_effort};
use crate::commands::AppCommand;
use crate::system::{AppSummary, ExecEvent, OpResult, OpStatus};
use std::sync::mpsc::Receiver;
use std::time::Instant;

pub const PRODUCT_LABEL: &str = "Akari-Sweeps_V00.1";
pub const PRODUCT_VERSION: &str = "V00.1";
pub const LOG_CAP: usize = 500;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Maintenance,
    About,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionState {
    Idle,
    Running,
    Done,
    Error,
}

pub struct App {
    pub command: AppCommand,
    pub screen: Screen,
    pub execution: ExecutionState,
    pub log: Vec<String>,
    pub operations: Vec<OpItem>,
    pub animator: Animator,
    pub frames: Vec<SpriteFrame>,
    pub sprite_error: Option<String>,
    pub summary: Option<AppSummary>,
    pub exit_code: i32,
    pub started: Instant,
    pub worker_running: bool,
    pub status_line: String,
    /// Guard against double-start from repeated keypresses/events.
    pub run_token: Option<u64>,
    pub scroll: usize,
}

#[derive(Debug, Clone)]
pub struct OpItem {
    pub id: String,
    pub label: String,
    pub target: String,
    pub status: OpStatus,
    pub detail: String,
}

impl App {
    pub fn new(command: AppCommand) -> Self {
        let (frames, sprite_error) = match load_sweep_frames_best_effort() {
            Ok(f) => (f, None),
            Err(e) => (Vec::new(), Some(e)),
        };
        let screen = match command {
            AppCommand::About => Screen::About,
            AppCommand::Help => Screen::Help,
            AppCommand::Clean | AppCommand::Health | AppCommand::All => Screen::Maintenance,
            AppCommand::Home => Screen::Home,
        };
        // Maintenance aliases auto-start; Home waits for a key.
        let execution = match command {
            AppCommand::Clean | AppCommand::Health | AppCommand::All => ExecutionState::Running,
            AppCommand::Home => ExecutionState::Idle,
            AppCommand::About | AppCommand::Help => ExecutionState::Idle,
        };
        let mut app = Self {
            command,
            screen,
            execution,
            log: vec![format!(
                "{PRODUCT_LABEL} — type a command or press ? for help."
            )],
            operations: Vec::new(),
            animator: Animator::new(AnimationSet::Sweep),
            frames,
            sprite_error,
            summary: None,
            exit_code: 0,
            started: Instant::now(),
            worker_running: false,
            status_line: String::new(),
            run_token: None,
            scroll: 0,
        };
        app.sync_animation();
        app
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis() as u64
    }

    /// Keep the animation honest: sweep only while work runs.
    pub fn sync_animation(&mut self) {
        let mode = match self.execution {
            ExecutionState::Running => AnimationMode::Sweeping,
            ExecutionState::Done | ExecutionState::Error => AnimationMode::Settled,
            _ => AnimationMode::Ambient,
        };
        self.animator.set_mode(mode);
    }

    pub fn push_log(&mut self, line: String) {
        // Split embedded newlines so the panel never corrupts on long output.
        for part in line.split('\n') {
            let clean = part.trim_end().to_string();
            if clean.is_empty() {
                continue;
            }
            // Truncate pathological single lines.
            let shown = if clean.len() > 300 {
                format!("{}…", &clean[..300])
            } else {
                clean
            };
            self.log.push(shown);
            if self.log.len() > LOG_CAP {
                self.log.drain(0..self.log.len() - LOG_CAP);
            }
        }
    }

    fn upsert_op(&mut self, id: &str, label: &str, target: &str, status: OpStatus) {
        if let Some(op) = self.operations.iter_mut().find(|o| o.id == id) {
            op.status = status;
            op.label = label.to_string();
            op.target = target.to_string();
        } else {
            self.operations.push(OpItem {
                id: id.to_string(),
                label: label.to_string(),
                target: target.to_string(),
                status,
                detail: String::new(),
            });
        }
    }

    /// Handle one worker event. Returns true when a full summary arrived.
    pub fn on_event(&mut self, ev: ExecEvent) -> bool {
        match ev {
            ExecEvent::Log(line) => {
                self.push_log(line);
                false
            }
            ExecEvent::OpStarted { id, label, target } => {
                self.upsert_op(&id, &label, &target, OpStatus::Running);
                self.push_log(format!("▶ {label} — {target}"));
                false
            }
            ExecEvent::OpProgress { id, line } => {
                self.push_log(format!("  [{id}] {line}"));
                false
            }
            ExecEvent::OpFinished(r) => {
                self.apply_result(&r);
                false
            }
            ExecEvent::PhaseStarted { name } => {
                self.push_log(format!("━━ {name} started ━━"));
                false
            }
            ExecEvent::PhaseFinished { name } => {
                self.push_log(format!("━━ {name} finished ━━"));
                false
            }
            ExecEvent::Finished(summary) => {
                self.apply_summary(summary);
                true
            }
            ExecEvent::Error(e) => {
                self.execution = ExecutionState::Error;
                self.push_log(format!("ERROR: {e}"));
                self.status_line = e;
                self.sync_animation();
                false
            }
        }
    }

    fn apply_result(&mut self, r: &OpResult) {
        self.upsert_op(&r.id, &r.label, &r.target, r.status);
        if let Some(op) = self.operations.iter_mut().find(|o| o.id == r.id) {
            op.detail = r.detail.clone();
        }
        let mark = match r.status {
            OpStatus::Success => "✓",
            OpStatus::Failed => "✗",
            OpStatus::Skipped => "–",
            _ => "…",
        };
        self.push_log(format!("{} {} — {}", mark, r.label, r.detail));
    }

    fn apply_summary(&mut self, summary: AppSummary) {
        self.exit_code = summary.exit_code;
        // Merge per-op results that arrived only in the summary.
        for r in summary
            .cleanup_results
            .iter()
            .chain(summary.repair_results.iter())
        {
            self.upsert_op(&r.id, &r.label, &r.target, r.status);
            if let Some(op) = self.operations.iter_mut().find(|o| o.id == r.id) {
                op.detail = r.detail.clone();
            }
        }
        self.push_log(format!("━━ summary: {} ━━", summary.overall));
        for w in &summary.totals.warnings {
            self.push_log(format!("warning: {w}"));
        }
        for e in &summary.totals.errors {
            self.push_log(format!("error: {e}"));
        }
        self.summary = Some(summary);
        self.execution = ExecutionState::Done;
        self.worker_running = false;
        self.sync_animation();
    }

    /// Drain all pending worker events without blocking the TUI.
    pub fn drain_events(&mut self, rx: &Receiver<ExecEvent>) {
        while let Ok(ev) = rx.try_recv() {
            self.on_event(ev);
        }
    }

    pub fn mark_started(&mut self, token: u64) {
        self.run_token = Some(token);
        self.worker_running = true;
        self.execution = ExecutionState::Running;
        self.operations.clear();
        self.summary = None;
        self.sync_animation();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::RunTotals;

    #[test]
    fn home_starts_idle_without_maintenance() {
        let app = App::new(AppCommand::Home);
        assert_eq!(app.screen, Screen::Home);
        assert_eq!(app.execution, ExecutionState::Idle);
        assert!(app.summary.is_none());
    }

    #[test]
    fn maintenance_aliases_enter_running_screen() {
        for cmd in [AppCommand::Clean, AppCommand::Health, AppCommand::All] {
            let app = App::new(cmd);
            assert_eq!(app.screen, Screen::Maintenance);
            assert_eq!(app.execution, ExecutionState::Running);
        }
    }

    #[test]
    fn about_help_do_not_run_maintenance() {
        for cmd in [AppCommand::About, AppCommand::Help] {
            let app = App::new(cmd);
            assert_eq!(app.execution, ExecutionState::Idle);
            assert!(app.summary.is_none());
        }
    }

    #[test]
    fn failed_op_reported_and_animation_settles() {
        let mut app = App::new(AppCommand::Clean);
        app.on_event(ExecEvent::OpStarted {
            id: "clean:npm".into(),
            label: "npm".into(),
            target: "npm cache".into(),
        });
        let mut r = OpResult::failed("clean:npm", "npm", "npm cache", "exit 1".into());
        r.exit_code = Some(1);
        app.on_event(ExecEvent::OpFinished(r));
        let op = app.operations.iter().find(|o| o.id == "clean:npm").unwrap();
        assert_eq!(op.status, OpStatus::Failed);

        let totals = RunTotals::default();
        let summary = AppSummary::compute(
            "completed".into(),
            "skipped".into(),
            false,
            totals,
            vec![],
            vec![],
            None,
            None,
        );
        app.on_event(ExecEvent::Finished(summary));
        assert_eq!(app.execution, ExecutionState::Done);
        assert_eq!(app.animator.mode(), AnimationMode::Settled);
    }

    #[test]
    fn animation_sweeps_only_while_running() {
        let idle = App::new(AppCommand::Home);
        assert_eq!(idle.animator.mode(), AnimationMode::Ambient);
        let mut running = App::new(AppCommand::Clean);
        assert_eq!(running.animator.mode(), AnimationMode::Sweeping);
        running.execution = ExecutionState::Done;
        running.sync_animation();
        assert_eq!(running.animator.mode(), AnimationMode::Settled);
    }
}
