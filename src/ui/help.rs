//! Help screen content.
use crate::commands::COMMAND_CATALOGUE;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    let mut out = vec![
        Line::from(Span::styled(
            "Usage — one launcher, five selectable commands:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];
    for (i, cmd) in COMMAND_CATALOGUE.iter().enumerate() {
        let key = match cmd.public_name() {
            "akari" => "  (launcher)".to_string(),
            _ => format!("  {} ", i),
        };
        out.push(Line::from(vec![
            Span::styled(format!("{key:<13}"), Style::default().fg(Color::Yellow)),
            Span::raw(format!("{} — {}", cmd.public_name(), cmd.description())),
        ]));
    }
    out.extend(vec![
        Line::from(""),
        Line::from("Selecting: press 1-5, or press / and type the command name,"),
        Line::from("then Enter. Input is trimmed and case-insensitive; both paths"),
        Line::from("run the same dispatch. Esc cancels entry, Ctrl+U clears it."),
        Line::from(""),
        Line::from("Returning to the main menu: type back, akariback (any case),"),
        Line::from("or press - . The - key works immediately when not typing;"),
        Line::from("while typing, Enter submits it like any command. 1-5 still"),
        Line::from("launch operations."),
        Line::from(""),
        Line::from("Launching: type the command name (akari, akariclean, …) in"),
        Line::from("PowerShell/CMD after install, or run `akari <name>` (e.g. `akari clean`)."),
        Line::from(""),
        Line::from("What maintenance does:"),
        Line::from("  • cleanup — temp/cache/delivery/store/VS Code/browser/Discord/"),
        Line::from("    npm+pip/DNS/Recycle Bin/Disk Cleanup/SilentCleanup."),
        Line::from("    Never runs DISM, SFC, CHKDSK, or servicing."),
        Line::from("  • repair  — DISM StartComponentCleanup + Check/Scan/RestoreHealth,"),
        Line::from("    SFC /scannow, CHKDSK /scan. Never deletes temp files."),
        Line::from(""),
        Line::from("Privileges: maintenance needs Administrator. A non-elevated request"),
        Line::from("relaunches itself through UAC; declining UAC cancels safely and"),
        Line::from("runs nothing. Viewing these screens needs no elevation."),
        Line::from(""),
        Line::from("Exit safely: q or Esc quits the interface (a running operation is"),
        Line::from("left to finish; Ctrl+C requests cancel)."),
    ]);
    out
}
