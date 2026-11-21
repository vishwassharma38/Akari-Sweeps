//! Help screen content.
use crate::commands::COMMAND_CATALOGUE;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub fn lines() -> Vec<Line<'static>> {
    let mut out = vec![
        Line::from(Span::styled(
            "Commands and usage:",
            Style::default().fg(Color::Cyan),
        )),
        Line::from(""),
    ];
    for cmd in COMMAND_CATALOGUE {
        out.push(Line::from(vec![
            Span::styled(
                format!("{:<14}", cmd.public_name()),
                Style::default().fg(Color::Yellow),
            ),
            Span::raw(cmd.description()),
        ]));
    }
    out.extend([
        Line::from(""),
        Line::from("Maintenance aliases can also be run as `akari clean`,"),
        Line::from("`akari health`, or `akari all`. Cleanup removes supported"),
        Line::from("temporary files and caches. Health runs Windows image repair."),
        Line::from("All runs cleanup followed by repair. Maintenance requires"),
        Line::from("administrator approval on Windows; no work starts if UAC is declined."),
        Line::from(""),
        Line::from("Other primary launcher options:"),
        Line::from("  akari --version       Print the installed version."),
        Line::from("  akari update          Check for and install a newer Windows release."),
        Line::from("  akari update --check  Check for a release without installing it."),
        Line::from("These options are available through `akari`, not maintenance aliases."),
        Line::from(""),
        Line::from("Inside the interface: press 1-5 to select a command, or press /"),
        Line::from("and type its name, then Enter. Entry is case-insensitive; Esc cancels"),
        Line::from("and Ctrl+U clears it. Type back, akariback, or - to return Home."),
        Line::from("Press - to return immediately when the entry field is not focused."),
        Line::from("Press q or Esc to quit. Ctrl+C requests cancellation of a running"),
        Line::from("operation; quitting does not forcibly stop its subprocesses."),
    ]);
    out
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
    fn help_documents_every_registered_command_and_early_cli_option() {
        let help = text();
        for command in COMMAND_CATALOGUE {
            assert!(help.contains(command.public_name()));
            assert!(help.contains(command.description()));
        }
        for usage in ["akari --version", "akari update", "akari update --check"] {
            assert!(help.contains(usage));
        }
        assert!(!help.contains("akari <name>"));
    }
}
