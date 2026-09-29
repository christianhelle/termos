//! Interactive prompt session for running many commands and queries.

use std::io::Write;
use std::time::Duration;

use crate::app::App;
use crate::management::Management;
use crate::prompt::LineReader;
use crate::store::DataPlane;

/// Reads commands and queries from a prompt until the user leaves.
pub struct Repl<M, D, W> {
    app: App<M, D, W>,
    lines: Box<dyn LineReader>,
}

impl<M: Management, D: DataPlane, W: Write> Repl<M, D, W> {
    pub fn new(app: App<M, D, W>, lines: Box<dyn LineReader>) -> Self {
        Repl { app, lines }
    }

    /// Runs until `/exit`, `/quit` or the end of input. Failed lines are reported and skipped.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        while let Some(line) = self.lines.read_line("cosmoscli> ")? {
            match self.handle(&line).await {
                Ok(Flow::Exit) => break,
                Ok(Flow::Continue) => {}
                Err(error) => writeln!(self.app.out, "error: {error:#}")?,
            }
        }
        Ok(())
    }

    async fn handle(&mut self, line: &str) -> anyhow::Result<Flow> {
        match parse_input(line)? {
            Input::Slash(words) => match words.first().map(String::as_str) {
                Some("exit" | "quit") => Ok(Flow::Exit),
                _ => Ok(Flow::Continue),
            },
            Input::Empty | Input::Query(_) => Ok(Flow::Continue),
        }
    }
}

/// Whether the session goes on after a line.
enum Flow {
    Continue,
    Exit,
}

/// A line typed at the interactive prompt.
#[derive(Debug, PartialEq)]
pub enum Input {
    /// Nothing but whitespace.
    Empty,
    /// SQL to run against the current container.
    Query(String),
    /// A slash command split into words, without the leading slash.
    Slash(Vec<String>),
}

/// Works out what a line typed at the prompt asks for.
pub fn parse_input(line: &str) -> anyhow::Result<Input> {
    let line = line.trim();
    if line.is_empty() {
        return Ok(Input::Empty);
    }
    match line.strip_prefix('/') {
        Some(command) => shlex::split(command)
            .map(Input::Slash)
            .ok_or_else(|| anyhow::anyhow!("unbalanced quotes in command")),
        None => Ok(Input::Query(line.to_string())),
    }
}

/// Formats how long something took, such as "245 ms" or "1.23 s".
pub fn format_elapsed(elapsed: Duration) -> String {
    if elapsed < Duration::from_secs(1) {
        format!("{} ms", elapsed.as_millis())
    } else {
        format!("{:.2} s", elapsed.as_secs_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::GlobalArgs;
    use crate::testing::{
        FakeDataPlane, FakeManagement, ScriptedConfirm, ScriptedLines, account, container,
    };

    type TestRepl = Repl<FakeManagement, FakeDataPlane, Vec<u8>>;

    fn shop() -> FakeManagement {
        FakeManagement::with_databases(
            account("orders"),
            &[
                ("shop", &[container("carts", "/userId")]),
                ("audit", &[container("events", "/deviceId")]),
            ],
        )
    }

    fn repl(lines: &ScriptedLines) -> TestRepl {
        let app = App {
            management: shop(),
            data: FakeDataPlane::new("/userId", vec![]),
            input: Box::new(std::io::empty()),
            confirm: Box::new(ScriptedConfirm::answering(false)),
            out: Vec::new(),
            global: GlobalArgs::default(),
        };
        Repl::new(app, Box::new(lines.clone()))
    }

    #[tokio::test]
    async fn session_ends_at_the_end_of_input() {
        let lines = ScriptedLines::new(&[]);

        repl(&lines).run().await.unwrap();
    }

    #[tokio::test]
    async fn exit_and_quit_end_the_session() {
        for command in ["/exit", "/quit"] {
            let lines = ScriptedLines::new(&[command, "SELECT * FROM c"]);

            repl(&lines).run().await.unwrap();

            assert_eq!(lines.lines.borrow().len(), 1, "{command}");
        }
    }

    fn output(repl: &TestRepl) -> String {
        String::from_utf8(repl.app.out.clone()).unwrap()
    }

    #[tokio::test]
    async fn errors_are_reported_and_the_session_goes_on() {
        let lines = ScriptedLines::new(&["/query \"SELECT", "/exit", "SELECT * FROM c"]);
        let mut repl = repl(&lines);

        repl.run().await.unwrap();

        assert_eq!(output(&repl), "error: unbalanced quotes in command\n");
        assert_eq!(lines.lines.borrow().len(), 1);
    }

    fn words(words: &[&str]) -> Input {
        Input::Slash(words.iter().map(|w| w.to_string()).collect())
    }

    #[test]
    fn blank_line_is_empty() {
        assert_eq!(parse_input("   \n").unwrap(), Input::Empty);
    }

    #[test]
    fn text_without_a_slash_is_a_trimmed_query() {
        assert_eq!(
            parse_input("  SELECT * FROM c \n").unwrap(),
            Input::Query("SELECT * FROM c".into())
        );
    }

    #[test]
    fn text_with_a_leading_slash_is_a_command_split_into_words() {
        assert_eq!(
            parse_input(" /items get  --id c-1\n").unwrap(),
            words(&["items", "get", "--id", "c-1"])
        );
    }

    #[test]
    fn quoted_words_stay_together() {
        assert_eq!(
            parse_input(r#"/query "SELECT * FROM c WHERE c.name = 'a b'" -o json"#).unwrap(),
            words(&[
                "query",
                "SELECT * FROM c WHERE c.name = 'a b'",
                "-o",
                "json"
            ])
        );
    }

    #[test]
    fn unbalanced_quotes_are_rejected() {
        assert!(parse_input(r#"/query "SELECT * FROM c"#).is_err());
    }

    #[test]
    fn elapsed_under_a_second_is_shown_in_milliseconds() {
        assert_eq!(format_elapsed(Duration::from_micros(245_900)), "245 ms");
        assert_eq!(format_elapsed(Duration::ZERO), "0 ms");
    }

    #[test]
    fn elapsed_of_a_second_or_more_is_shown_in_seconds() {
        assert_eq!(format_elapsed(Duration::from_millis(1_000)), "1.00 s");
        assert_eq!(format_elapsed(Duration::from_millis(12_346)), "12.35 s");
    }
}
