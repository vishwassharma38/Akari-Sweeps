//! Help screen content.
use crate::commands::COMMAND_CATALOGUE;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    let mut out = vec![
        Line::from(Span::styled(
            "Usage — six commands, one app:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];
    for cmd in COMMAND_CATALOGUE {
        out.push(Line::from(vec![
            Span::styled(
                format!("  {:<12}", cmd.public_name()),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(format!("— {}", cmd.description())),
        ]));
    }
    out.extend(vec![
        Line::from(""),
        Line::from("Launching: type the command name (akari, akariclean, …) in"),
        Line::from("PowerShell/CMD after install, or run `akari <name>` (e.g. `akari clean`)."),
        Line::from(""),
        Line::from("What maintenance does:"),
        Line::from("  • cleanup — temp/cache/delivery/store/VS Code/browser/Discord/"),
        Line::from("    npm+pip/DNS/Recycle Bin/Disk Cleanup/SilentCleanup/DISM cleanup."),
        Line::from("  • repair  — DISM Check/Scan/RestoreHealth, SFC /scannow, CHKDSK /scan."),
        Line::from(""),
        Line::from("Privileges: cleanup + repair need Administrator. Without it the"),
        Line::from("app explains how to re-launch elevated instead of failing silently."),
        Line::from(""),
        Line::from("Exit safely: q or Esc quits the interface (a running operation is"),
        Line::from("left to finish; Ctrl+C requests cancel). c re-runs cleanup, r repair."),
    ]);
    out
}
