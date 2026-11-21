//! Help screen content.
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    let title_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let mut out = vec![
        Line::from(Span::styled("AKARI-SWEEPS", title_style)),
        Line::from("Windows maintenance utility"),
        Line::from(""),
        heading("USAGE"),
        Line::from("  akari"),
        Line::from("  akari <command> [options]"),
        Line::from(""),
        heading("COMMANDS"),
    ];

    for (name, description) in [
        ("clean", "Run the cleanup phase."),
        ("health", "Run Windows system-image repair."),
        ("all", "Run cleanup followed by system-image repair."),
        ("about", "Display application information."),
        ("help", "Display this help message."),
        (
            "update [--check]",
            "Check for and install a newer Akari release.",
        ),
    ] {
        out.push(row(name, description));
    }

    out.extend([
        Line::from(""),
        heading("OPTIONS"),
        row("--version", "Display the installed version."),
        row("--check", "Check for an update without installing it."),
        Line::from("                    Available with: akari update"),
        Line::from(""),
        heading("INTERACTIVE MODE"),
        row("1-5", "Select a menu item."),
        row(
            "/ <name>",
            "Search for a command by name, then press Enter.",
        ),
        row("back", "Return to the Home screen."),
        row("akariback", "Return to the Home screen."),
        row("-", "Return to the Home screen."),
        row("Esc", "Cancel the current entry or quit the interface."),
        row("q", "Quit the interface."),
        row("Ctrl+U", "Clear the current entry."),
        row("Ctrl+C", "Request cancellation of a running operation."),
        Line::from(""),
        heading("NOTES"),
        Line::from("  Maintenance commands require administrator approval"),
        Line::from("  through Windows UAC."),
        Line::from("  No maintenance operation starts if administrator"),
        Line::from("  approval is declined."),
        Line::from(""),
        Line::from("  Legacy concatenated executable aliases remain supported"),
        Line::from("  for backward compatibility."),
        Line::from(""),
        Line::from("  Quitting the interface does not forcibly terminate its"),
        Line::from("  subprocesses."),
    ]);

    out
}

fn heading(label: &'static str) -> Line<'static> {
    Line::from(Span::styled(
        label,
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))
}

fn row(label: &'static str, description: &'static str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<18}"), Style::default().fg(Color::Yellow)),
        Span::raw(description),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::COMMAND_CATALOGUE;

    fn text() -> String {
        lines()
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn help_uses_the_requested_sections_and_command_layout() {
        let help = text();

        for section in [
            "AKARI-SWEEPS",
            "Windows maintenance utility",
            "USAGE",
            "COMMANDS",
            "OPTIONS",
            "INTERACTIVE MODE",
            "NOTES",
        ] {
            assert!(help.contains(section), "missing help section: {section}");
        }

        assert!(help.contains("  akari\n  akari <command> [options]"));

        for command in COMMAND_CATALOGUE {
            if let Some(name) = command.subcommand_token() {
                let expected = format!("  {name} ");
                assert!(
                    help.lines().any(|line| line.starts_with(expected.as_str())),
                    "help should include the {name} command"
                );
            }
        }

        for expected in [
            "update [--check]",
            "--version",
            "--check",
            "Available with: akari update",
            "/ <name>",
            "akariback",
            "Ctrl+U",
            "Ctrl+C",
            "Maintenance commands require administrator approval",
            "No maintenance operation starts if administrator",
            "Legacy concatenated executable aliases remain supported",
            "Quitting the interface does not forcibly terminate its",
        ] {
            assert!(help.contains(expected), "missing help text: {expected}");
        }

        for legacy in [
            "akariclean",
            "akarihealth",
            "akariall",
            "akariabout",
            "akarihelp",
        ] {
            assert!(
                !help.contains(legacy),
                "help should describe preferred commands, not aliases"
            );
        }
    }
}
