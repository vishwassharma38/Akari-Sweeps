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
    let exe_path = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("akari"));
    let exe = exe_path.to_string_lossy().to_string();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = resolve_command(&exe, &args);

    // Requirement 2: privilege handling happens BEFORE the TUI owns the
    // terminal. Maintenance aliases relaunch elevated through UAC and this
    // process exits with the elevated child's result (or a safe abort).
    let elevated = crate::system::windows::is_elevated();
    let marker = crate::system::elevation::has_marker(&args);
    match crate::system::elevation::decide(
        command.starts_maintenance(),
        elevated,
        crate::system::windows::is_windows(),
        marker,
    ) {
        crate::system::elevation::ElevationDecision::RelaunchElevated => {
            println!("Requesting administrator privileges (UAC)...");
            let mut launcher = crate::system::elevation::RealElevationLauncher;
            use crate::system::elevation::ElevationLauncher as _;
            match launcher.launch(&exe_path, &args) {
                crate::system::elevation::RelaunchOutcome::Launched { exit_code } => {
                    return exit_code;
                }
                crate::system::elevation::RelaunchOutcome::Cancelled => {
                    eprintln!(
                        "Elevation declined: the UAC prompt was dismissed, so {} was not started. \
                         Re-run and approve the prompt, or run as administrator.",
                        command.public_name()
                    );
                    return crate::system::elevation::UAC_CANCELLED_CODE;
                }
                crate::system::elevation::RelaunchOutcome::Failed(msg) => {
                    eprintln!(
                        "Could not request elevation ({msg}); {} was not started. \
                         Right-click the app and choose 'Run as administrator'.",
                        command.public_name()
                    );
                    return 1;
                }
            }
        }
        crate::system::elevation::ElevationDecision::Abort => {
            eprintln!(
                "ERROR: Administrator privileges could not be obtained. \
                 Right-click the app and choose Run as administrator."
            );
            return 1;
        }
        crate::system::elevation::ElevationDecision::Proceed => {}
    }

    // Invalid invocation hint: `akari <unknown>` stays Home but flags it.
    let mut app = App::new(command);
    if !args.is_empty()
        && matches!(
            exe_normalized(&exe).as_str(),
            "akari" | "akari-sweeps" | "akari_sweeps"
        )
    {
        // The ELEVATED marker is not a user command; never flag it.
        if let Some(first) = args
            .iter()
            .find(|a| !a.eq_ignore_ascii_case(crate::system::elevation::ELEVATION_MARKER))
        {
            let t = crate::commands::normalize_token(first);
            if crate::commands::from_token(&t).is_none() {
                app.push_log(format!(
                    "Unknown command '{first}'. Showing home — press ? for the command list."
                ));
            }
        }
    }

    if !crate::system::windows::is_windows() {
        app.push_log(
            "Note: non-Windows host detected. Maintenance subprocesses target Windows; \
             filesystem cleanup still runs against the local FS where paths resolve."
                .to_string(),
        );
    } else if !elevated && !command.starts_maintenance() {
        app.push_log(
            "Running without administrator rights. These screens work as-is; \
             maintenance will request elevation through UAC first."
                .to_string(),
        );
        app.push_log(crate::system::windows::elevation_help().to_string());
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
    let mut dirty = true;

    loop {
        // Deadline-based animation tick: monotonic clock, no cumulative
        // drift, backlog resync instead of spiral. Never blocks on input.
        let now_ms = app.elapsed_ms();
        if app.animator.tick(now_ms) {
            dirty = true;
        }
        if app.drain_events(&rx) > 0 {
            dirty = true;
        }
        if dirty {
            if let Err(e) = terminal.draw(|f| ui::render(f, &app)) {
                app.push_log(format!("render error: {e}"));
            }
            dirty = false;
        }

        // Poll just long enough to hit the next frame on time (6 FPS),
        // capped so keyboard input stays responsive (≤50 ms latency).
        let timeout = app.animator.ms_until_next_frame(now_ms).min(50);
        if event::poll(Duration::from_millis(timeout)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) => {
                    if handle_key(&mut app, &tx, key.code, key.modifiers, &mut pending_quit) {
                        break;
                    }
                    dirty = true;
                }
                Ok(Event::Resize(_, _)) => {
                    dirty = true; // next draw adapts; nothing else needed
                }
                _ => {}
            }
        }
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
///
/// While the command entry is focused, keystrokes edit it (shortcuts are
/// suspended); otherwise single keys navigate. Both number selection and
/// text entry converge on [`activate`] — one dispatch path.
fn handle_key(
    app: &mut App,
    tx: &Sender<ExecEvent>,
    code: KeyCode,
    mods: KeyModifiers,
    pending_quit: &mut bool,
) -> bool {
    use crossterm::event::KeyModifiers as M;
    if app.input_focused {
        match code {
            KeyCode::Enter => {
                submit_input(app, tx);
                *pending_quit = false;
            }
            KeyCode::Esc => {
                app.set_input_focus(false);
                app.clear_input();
            }
            KeyCode::Backspace => app.backspace_input(),
            KeyCode::Char('u') if mods.contains(M::CONTROL) => app.clear_input(),
            KeyCode::Char(c) if !mods.contains(M::CONTROL) && !mods.contains(M::ALT) => {
                app.push_input(&c.to_string());
            }
            _ => {}
        }
        return false;
    }
    // Ctrl+C is deliberate: first press warns, second quits.
    if mods.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        return request_quit(app, pending_quit);
    }
    match code {
        KeyCode::Char('q') | KeyCode::Esc => return request_quit(app, pending_quit),
        KeyCode::Char('/') => app.set_input_focus(true),
        KeyCode::Enter => app.set_input_focus(true),
        KeyCode::Char('?') => {
            app.screen = Screen::Help;
            *pending_quit = false;
        }
        KeyCode::Char('a') => {
            app.screen = Screen::About;
            *pending_quit = false;
        }
        KeyCode::Char('h') => {
            activate(app, tx, AppCommand::Home);
            *pending_quit = false;
        }
        KeyCode::Char(d @ '1'..='5') => {
            *pending_quit = false;
            if let Some(cmd) = crate::commands::command_from_number(d as u8 - b'0') {
                activate(app, tx, cmd);
            }
        }
        KeyCode::Char('c') => {
            *pending_quit = false;
            activate(app, tx, AppCommand::Clean);
        }
        KeyCode::Char('r') => {
            *pending_quit = false;
            activate(app, tx, AppCommand::Health);
        }
        KeyCode::Char('t') => {
            *pending_quit = false;
            activate(app, tx, AppCommand::All);
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

/// Submit the command-entry field through the same dispatch as number
/// keys. Empty and unknown input are handled safely: nothing starts.
fn submit_input(app: &mut App, tx: &Sender<ExecEvent>) {
    let text = app.input.clone();
    app.set_input_focus(false);
    app.clear_input();
    if text.trim().is_empty() {
        app.push_log("Type a command name (e.g. akariclean) or press 1-5.".to_string());
        return;
    }
    match crate::commands::command_from_input(&text) {
        Some(cmd) => activate(app, tx, cmd),
        None => app.push_log(format!(
            "Unknown command '{}'. Valid: akariclean, akarihealth, akariall, akariabout, akarihelp.",
            text.trim()
        )),
    }
}

/// Single shared dispatch for number selection, text entry, and shortcut
/// keys. Maintenance requests honor Requirement 2 elevation handling.
fn activate(app: &mut App, tx: &Sender<ExecEvent>, cmd: AppCommand) {
    if app.worker_running {
        app.push_log("A maintenance run is already in progress.".to_string());
        return;
    }
    if !cmd.starts_maintenance() {
        let fresh = App::new(cmd);
        // Preserve loaded sprites and elevation state across navigation.
        app.frames = fresh.frames;
        app.sprite_error = fresh.sprite_error;
        app.command = fresh.command;
        app.screen = fresh.screen;
        app.execution = fresh.execution;
        app.summary = None;
        app.operations.clear();
        app.sync_animation();
        app.push_log(format!("Switched to {}.", cmd.public_name()));
        return;
    }
    if cfg!(target_os = "windows") && !app.elevated {
        request_in_tui_elevation(app, tx, cmd);
        return;
    }
    if app.execution == crate::app::ExecutionState::Running {
        return;
    }
    app.command = cmd;
    app.screen = Screen::Maintenance;
    spawn_worker(cmd, tx.clone(), app);
}

/// In-TUI elevation: the running (non-elevated) interface must not run
/// maintenance itself. It relaunches the same executable elevated in a
/// blocking background thread — the TUI stays responsive — and reports
/// the outcome honestly. UAC cancellation logs an actionable message
/// and starts nothing.
fn request_in_tui_elevation(app: &mut App, tx: &Sender<ExecEvent>, cmd: AppCommand) {
    app.command = cmd;
    app.screen = Screen::Maintenance;
    app.push_log(format!(
        "Administrator rights required — requesting elevation for {} via UAC…",
        cmd.public_name()
    ));
    app.push_log(
        "Approve the prompt to run elevated in a new window. Declining cancels safely; nothing runs here."
            .to_string(),
    );
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("akari"));
    let arg = cmd.public_name().to_string();
    let tx2 = tx.clone();
    std::thread::spawn(move || {
        let mut launcher = crate::system::elevation::RealElevationLauncher;
        use crate::system::elevation::ElevationLauncher as _;
        match launcher.launch(&exe, &[arg]) {
            crate::system::elevation::RelaunchOutcome::Launched { exit_code } => {
                let _ = tx2.send(ExecEvent::Log(format!(
                    "Elevated {} finished with exit code {exit_code}.",
                    cmd.public_name()
                )));
            }
            crate::system::elevation::RelaunchOutcome::Cancelled => {
                let _ = tx2.send(ExecEvent::Log(format!(
                    "Elevation declined: {} was not started. Approve the UAC prompt, or run as administrator.",
                    cmd.public_name()
                )));
            }
            crate::system::elevation::RelaunchOutcome::Failed(msg) => {
                let _ = tx2.send(ExecEvent::Error(format!(
                    "Could not request elevation ({msg}); {} was not started.",
                    cmd.public_name()
                )));
            }
        }
    });
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
            let gate = crate::commands::PhaseGate::for_command(AppCommand::Clean)
                .0
                .expect("Clean allows cleanup");
            let (results, totals) = crate::commands::clean::run_cleanup_phase(&mut runner, &gate);
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
            let gate = crate::commands::PhaseGate::for_command(AppCommand::Health)
                .1
                .expect("Health allows repair");
            let (results, mut totals, entered) =
                crate::commands::health::run_repair_phase(&mut runner, &gate, false);
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
