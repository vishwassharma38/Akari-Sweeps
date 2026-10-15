//! Entry point: exe-name dispatch, terminal lifecycle, event loop,
//! background maintenance worker.

mod akari;
mod app;
mod commands;
mod system;
mod ui;

use crate::app::{App, Screen};
use crate::commands::runner::RealRunner;
use crate::commands::{AppCommand, resolve_command};
use crate::system::{AppSummary, ExecEvent, RunTotals};
use crossterm::ExecutableCommand;
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::stdout;
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

fn main() {
    let code = real_main();
    std::process::exit(code);
}

fn real_main() -> i32 {
    let exe = std::env::current_exe()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "akari".to_string());
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = resolve_command(&exe, &args);

    // Invalid invocation hint: `akari <unknown>` stays Home but flags it.
    let mut app = App::new(command);
    if !args.is_empty()
        && matches!(
            exe_normalized(&exe).as_str(),
            "akari" | "akari-sweeps" | "akari_sweeps"
        )
    {
        let t = crate::commands::normalize_token(&args[0]);
        if crate::commands::from_token(&t).is_none() {
            app.push_log(format!(
                "Unknown command '{}'. Showing home — press ? for the command list.",
                args[0]
            ));
        }
    }

    if !crate::system::windows::is_windows() {
        app.push_log(
            "Note: non-Windows host detected. Maintenance subprocesses target Windows; \
             filesystem cleanup still runs against the local FS where paths resolve."
                .to_string(),
        );
    }
    if command.starts_maintenance() && !crate::system::windows::is_elevated() {
        app.push_log(format!(
            "Warning: not running as Administrator. {}",
            crate::system::windows::elevation_help()
        ));
    }

    // Terminal setup with guaranteed restoration.
    if let Err(e) = enable_raw_mode() {
        eprintln!("failed to enable raw mode: {e}");
        return 2;
    }
    let mut out = stdout();
    if let Err(e) = out.execute(EnterAlternateScreen) {
        let _ = disable_raw_mode();
        eprintln!("failed to enter alternate screen: {e}");
        return 2;
    }
    let backend = CrosstermBackend::new(stdout());
    let mut terminal = match Terminal::new(backend) {
        Ok(t) => t,
        Err(e) => {
            let _ = disable_raw_mode();
            let _ = stdout().execute(LeaveAlternateScreen);
            eprintln!("failed to init terminal: {e}");
            return 2;
        }
    };

    // Auto-start worker for maintenance aliases (exactly once).
    let (tx, rx) = mpsc::channel::<ExecEvent>();
    if command.starts_maintenance() {
        spawn_worker(command, tx.clone(), &mut app);
    }

    let mut pending_quit = false;
    let mut tick: u64 = 0;

    loop {
        // Animation tick without blocking on child output.
        let now_ms = app.elapsed_ms();
        if app.animator.tick(now_ms) {
            // frame advanced; redraw below
        }

        if let Err(e) = terminal.draw(|f| ui::render(f, &app)) {
            app.push_log(format!("render error: {e}"));
        }
        app.drain_events(&rx);

        // Poll for input/resize; 100 ms keeps animation smooth + responsive.
        if event::poll(Duration::from_millis(100)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    tick += 1;
                    if handle_key(&mut app, &tx, key.code, key.modifiers, &mut pending_quit) {
                        break;
                    }
                }
                Ok(Event::Resize(_, _)) => {
                    // Next draw adapts; nothing else needed.
                }
                _ => {}
            }
        }
        let _ = tick;
        if app.worker_running {
            // Keep looping while the worker streams; exit only via quit key
            // or when the user quits after Done.
        }
    }

    let _ = disable_raw_mode();
    let _ = stdout().execute(LeaveAlternateScreen);
    app.exit_code
}

fn exe_normalized(exe: &str) -> String {
    crate::commands::normalize_token(exe)
}

/// Returns true when the event loop should exit.
fn handle_key(
    app: &mut App,
    tx: &Sender<ExecEvent>,
    code: KeyCode,
    mods: KeyModifiers,
    pending_quit: &mut bool,
) -> bool {
    // Ctrl+C is deliberate: first press warns, second quits.
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return request_quit(app, pending_quit);
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return request_quit(app, pending_quit),
        KeyCode::Char('?') => {
            app.screen = Screen::Help;
            *pending_quit = false;
        }
        KeyCode::Char('a') => {
            app.screen = Screen::About;
            *pending_quit = false;
        }
        KeyCode::Char('h') => {
            app.screen = Screen::Home;
            *pending_quit = false;
        }
        KeyCode::Char('1') => switch_command(app, tx, AppCommand::Home),
        KeyCode::Char('2') => switch_command(app, tx, AppCommand::Clean),
        KeyCode::Char('3') => switch_command(app, tx, AppCommand::Health),
        KeyCode::Char('4') => switch_command(app, tx, AppCommand::All),
        KeyCode::Char('5') => switch_command(app, tx, AppCommand::About),
        KeyCode::Char('6') => switch_command(app, tx, AppCommand::Help),
        KeyCode::Char('c') => {
            *pending_quit = false;
            start_if_idle(app, tx, AppCommand::Clean);
        }
        KeyCode::Char('r') => {
            *pending_quit = false;
            start_if_idle(app, tx, AppCommand::Health);
        }
        KeyCode::Char('t') => {
            *pending_quit = false;
            start_if_idle(app, tx, AppCommand::All);
        }
        KeyCode::Up => {
            app.scroll = app.scroll.saturating_sub(1);
        }
        KeyCode::Down => {
            app.scroll = app.scroll.saturating_add(1);
        }
        _ => {}
    }
    false
}

fn request_quit(app: &mut App, pending: &mut bool) -> bool {
    if app.worker_running {
        if *pending {
            app.push_log(
                "Quitting while maintenance is running — the current operation may be left incomplete.".to_string(),
            );
            return true;
        }
        *pending = true;
        app.push_log(
            "Maintenance is running. Press q again to quit anyway (current operation may be incomplete), \
             or wait for it to finish."
                .to_string(),
        );
        return false;
    }
    true
}

fn switch_command(app: &mut App, tx: &Sender<ExecEvent>, cmd: AppCommand) {
    if app.worker_running {
        app.push_log("A maintenance run is already in progress.".to_string());
        return;
    }
    let fresh = App::new(cmd);
    app.command = fresh.command;
    app.screen = fresh.screen;
    app.execution = fresh.execution;
    app.summary = None;
    app.operations.clear();
    // Preserve sprite frames to avoid reloading.
    app.sync_animation();
    if cmd.starts_maintenance() {
        spawn_worker(cmd, tx.clone(), app);
    } else {
        app.push_log(format!("Switched to {}.", cmd.public_name()));
    }
}

fn start_if_idle(app: &mut App, tx: &Sender<ExecEvent>, cmd: AppCommand) {
    if app.worker_running {
        app.push_log("A maintenance run is already in progress.".to_string());
        return;
    }
    if app.execution == crate::app::ExecutionState::Running {
        return;
    }
    app.command = cmd;
    app.screen = Screen::Maintenance;
    spawn_worker(cmd, tx.clone(), app);
}

static RUN_SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

fn spawn_worker(cmd: AppCommand, tx: Sender<ExecEvent>, app: &mut App) {
    let token = RUN_SEQ.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    app.mark_started(token);
    app.screen = Screen::Maintenance;
    app.push_log(format!("Starting {}…", cmd.public_name()));
    std::thread::spawn(move || {
        run_requested(cmd, tx);
    });
}

/// Background entry: runs the requested phases, then posts `Finished`.
fn run_requested(cmd: AppCommand, tx: Sender<ExecEvent>) {
    let summary = match cmd {
        AppCommand::Clean => {
            let free_before = crate::system::windows::free_space_kb();
            let mut runner = RealRunner::new(Some(tx.clone()));
            let (results, totals) = crate::commands::clean::run_cleanup_phase(&mut runner);
            let free_after = crate::system::windows::free_space_kb();
            AppSummary::compute(
                "completed".to_string(),
                "not run - cleanup only".to_string(),
                false,
                totals,
                results,
                Vec::new(),
                free_before,
                free_after,
            )
        }
        AppCommand::Health => {
            let mut runner = RealRunner::new(Some(tx.clone()));
            let (results, mut totals, entered) =
                crate::commands::health::run_repair_phase(&mut runner, false);
            // Repair-only still reports an (empty) cleanup section.
            let _ = &mut totals;
            AppSummary::compute(
                "not run - repair only".to_string(),
                if totals.err_count > 0 {
                    "ran - one or more operations reported a failure".to_string()
                } else {
                    "ran - every operation completed without a reported failure".to_string()
                },
                entered,
                totals,
                Vec::new(),
                results,
                None,
                None,
            )
        }
        AppCommand::All => {
            let mut runner = RealRunner::new(Some(tx.clone()));
            crate::commands::all::run_all_phases(&mut runner, false)
        }
        _ => {
            let _ = tx.send(ExecEvent::Error(
                "no maintenance requested for this command".to_string(),
            ));
            return;
        }
    };
    // Guarantee the totals in the summary include warnings/errors only;
    // per-op details were already streamed as OpFinished events.
    let _ = tx.send(ExecEvent::Finished(summary));
}

/// Build an empty summary for tests of exit-code plumbing.
#[allow(dead_code)]
fn empty_summary() -> AppSummary {
    AppSummary::compute(
        "not run".into(),
        "not offered".into(),
        false,
        RunTotals::default(),
        vec![],
        vec![],
        None,
        None,
    )
}
