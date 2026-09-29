use std::io::{IsTerminal, Write};

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
