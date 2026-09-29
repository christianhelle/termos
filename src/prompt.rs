use std::io::{IsTerminal, Write};

use dialoguer::FuzzySelect;
use dialoguer::theme::ColorfulTheme;
use rustyline::error::ReadlineError;

/// Asks the user to confirm a destructive operation.
pub trait Confirm {
    fn confirm(&mut self, message: &str) -> anyhow::Result<bool>;
}

/// Reads lines typed at an interactive prompt.
pub trait LineReader {
    /// Returns the next line, or `None` when input has ended.
    fn read_line(&mut self, prompt: &str) -> anyhow::Result<Option<String>>;
}

/// Lets the user choose one item from a list.
pub trait Picker {
    /// Returns the index of the chosen item, or `None` when the user cancels.
    fn pick(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>>;
}

/// Whether a typed answer means yes. Anything else, including an empty line, means no.
fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Prompts on the terminal and refuses to guess when stdin is not interactive.
pub struct TerminalConfirm;

impl Confirm for TerminalConfirm {
    fn confirm(&mut self, message: &str) -> anyhow::Result<bool> {
        let stdin = std::io::stdin();
        anyhow::ensure!(
            stdin.is_terminal(),
            "{message} Pass --yes to confirm when not running interactively"
        );
        eprint!("{message} [y/N] ");
        std::io::stderr().flush()?;
        let mut answer = String::new();
        stdin.read_line(&mut answer)?;
        Ok(is_yes(&answer))
    }
}

/// Reads prompt lines on the terminal with line editing and history.
pub struct TerminalLines {
    editor: rustyline::DefaultEditor,
}

impl TerminalLines {
    pub fn new() -> anyhow::Result<Self> {
        Ok(TerminalLines {
            editor: rustyline::DefaultEditor::new()?,
        })
    }
}

impl LineReader for TerminalLines {
    fn read_line(&mut self, prompt: &str) -> anyhow::Result<Option<String>> {
        match self.editor.readline(prompt) {
            Ok(line) => {
                if !line.trim().is_empty() {
                    self.editor.add_history_entry(line.as_str())?;
                }
                Ok(Some(line))
            }
            // Ctrl-C drops the line being typed, Ctrl-D ends the session
            Err(ReadlineError::Interrupted) => Ok(Some(String::new())),
            Err(ReadlineError::Eof) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }
}

/// Picks from a list on the terminal, filtering as the user types.
pub struct TerminalPicker;

impl Picker for TerminalPicker {
    fn pick(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>> {
        let choice = FuzzySelect::with_theme(&ColorfulTheme::default())
            .with_prompt(prompt)
            .items(items)
            .default(0)
            .max_length(15)
            .interact_opt()?;
        Ok(choice)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_y_or_yes_confirms() {
        assert!(is_yes("y\n"));
        assert!(is_yes(" YES "));
        assert!(!is_yes("\n"));
        assert!(!is_yes("n"));
        assert!(!is_yes("yeah"));
    }
}
