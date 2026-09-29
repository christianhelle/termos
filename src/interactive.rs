//! Interactive prompt session for running many commands and queries.

use std::io::Write;
use std::time::{Duration, Instant};

use crate::app::App;
use clap::{CommandFactory, FromArgMatches};

use crate::cli::{Cli, OutputFormat};
use crate::management::{Account, Management};
use crate::output::{render_json, render_table};
use crate::prompt::{LineReader, Picker};
use crate::store::{DataPlane, DataStore};

/// The container queries run against, with its open connection.
struct Current<S> {
    database: String,
    container: String,
    store: S,
}

/// Tells the time, so tests can control how long things take.
pub trait Clock {
    fn now(&self) -> Instant;
}

/// The real monotonic clock.
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Reads commands and queries from a prompt until the user leaves.
pub struct Repl<M, D: DataPlane, W> {
    app: App<M, D, W>,
    lines: Box<dyn LineReader>,
    picker: Box<dyn Picker>,
    clock: Box<dyn Clock>,
    /// Time spent waiting for the user to pick, left out of the command's duration.
    picking: Duration,
    /// The account commands and queries run against.
    account: Option<Account>,
    /// The container of the current account that queries run against.
    current: Option<Current<D::Store>>,
    /// How typed queries render their documents.
    output: OutputFormat,
}

impl<M: Management, D: DataPlane, W: Write> Repl<M, D, W> {
    pub fn new(
        app: App<M, D, W>,
        lines: Box<dyn LineReader>,
        picker: Box<dyn Picker>,
        clock: Box<dyn Clock>,
    ) -> Self {
        Repl {
            app,
            lines,
            picker,
            clock,
            picking: Duration::ZERO,
            account: None,
            current: None,
            output: OutputFormat::Table,
        }
    }

    /// Runs until `/exit`, `/quit` or the end of input. Failed lines are reported and skipped.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        while let Some(line) = self.lines.read_line("cosmoscli> ")? {
            let started = self.clock.now();
            self.picking = Duration::ZERO;
            match self.handle(&line).await {
                Ok(Flow::Exit) => break,
                Ok(Flow::Untimed) => continue,
                Ok(Flow::Timed) => {}
                Err(error) => writeln!(self.app.out, "error: {error:#}")?,
            }
            let elapsed = self.clock.now() - started - self.picking;
            writeln!(self.app.out, "Completed in {}", format_elapsed(elapsed))?;
        }
        Ok(())
    }

    async fn handle(&mut self, line: &str) -> anyhow::Result<Flow> {
        match parse_input(line)? {
            Input::Slash(words) => match words.first().map(String::as_str) {
                Some("exit" | "quit") => Ok(Flow::Exit),
                Some("accounts") if is_listing(&words) => self.pick_account().await,
                Some("containers") if is_listing(&words) => self.pick_container().await,
                Some("output") => self.set_output(&words[1..]),
                _ => self.run_cli(words).await,
            },
            Input::Query(sql) => self.query(&sql).await,
            Input::Empty => Ok(Flow::Untimed),
        }
    }

    async fn query(&mut self, sql: &str) -> anyhow::Result<Flow> {
        let Some(Current { store, .. }) = &self.current else {
            anyhow::bail!(
                "select an account with /accounts and a container with /containers first"
            );
        };
        let docs = store.query(sql, None).await?;
        let rendered = match self.output {
            OutputFormat::Table => render_table(&docs, store.partition_key_path()),
            OutputFormat::Json => render_json(&docs),
        };
        writeln!(self.app.out, "{rendered}")?;
        Ok(Flow::Timed)
    }

    /// Runs a CLI command, filling in the current account, database and container.
    async fn run_cli(&mut self, words: Vec<String>) -> anyhow::Result<Flow> {
        let mut defaults = Vec::new();
        if let Some(account) = &self.account {
            defaults.push(("account", account.name.clone()));
        }
        if let Some(current) = &self.current {
            defaults.push(("database", current.database.clone()));
            defaults.push(("container", current.container.clone()));
        }
        let args = std::iter::once("cosmoscli".to_string()).chain(words);
        let matches = match with_defaults(Cli::command(), &defaults).try_get_matches_from(args) {
            Ok(matches) => matches,
            // Help and usage errors come fully worded from clap
            Err(usage) => {
                write!(self.app.out, "{}", usage.render())?;
                return Ok(Flow::Untimed);
            }
        };
        let Some(command) = Cli::from_arg_matches(&matches)?.command else {
            anyhow::bail!("type a command after /, see /help");
        };
        self.app.run(command).await?;
        Ok(Flow::Timed)
    }

    fn set_output(&mut self, args: &[String]) -> anyhow::Result<Flow> {
        self.output = match args {
            [format] if format == "json" => OutputFormat::Json,
            [format] if format == "table" => OutputFormat::Table,
            _ => anyhow::bail!("expected /output json or /output table"),
        };
        writeln!(self.app.out, "Query output is now {}", args[0])?;
        Ok(Flow::Timed)
    }

    /// Lets the user pick an item, keeping the time spent choosing out of the duration.
    fn pick(&mut self, prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>> {
        let asked = self.clock.now();
        let choice = self.picker.pick(prompt, items);
        self.picking += self.clock.now() - asked;
        choice
    }

    async fn pick_account(&mut self) -> anyhow::Result<Flow> {
        let subscription = self.app.global.subscription.as_deref();
        let mut accounts = self.app.management.list_accounts(subscription).await?;
        anyhow::ensure!(!accounts.is_empty(), "no Cosmos DB accounts found");
        let labels: Vec<String> = accounts
            .iter()
            .map(|a| format!("{} ({}, {})", a.name, a.resource_group, a.location))
            .collect();
        if let Some(index) = self.pick("Select an account", &labels)? {
            let account = accounts.swap_remove(index);
            writeln!(self.app.out, "Using account {}", account.name)?;
            self.account = Some(account);
            self.current = None;
        }
        Ok(Flow::Timed)
    }

    async fn pick_container(&mut self) -> anyhow::Result<Flow> {
        let Some(account) = self.account.clone() else {
            anyhow::bail!("select an account with /accounts first");
        };
        let mut containers = self.app.containers_of(&account, None).await?;
        anyhow::ensure!(
            !containers.is_empty(),
            "no containers found on account {}",
            account.name
        );
        let labels: Vec<String> = containers
            .iter()
            .map(|(database, c)| {
                let pk = c.partition_key_paths.join(", ");
                format!("{database}/{} ({pk})", c.name)
            })
            .collect();
        if let Some(index) = self.pick("Select a container", &labels)? {
            let (database, container) = containers.swap_remove(index);
            let store = self
                .app
                .connect_to(&account, &database, &container.name)
                .await?;
            writeln!(
                self.app.out,
                "Using container {database}/{}",
                container.name
            )?;
            self.current = Some(Current {
                database,
                container: container.name,
                store,
            });
        }
        Ok(Flow::Timed)
    }
}

/// Gives every required argument with a default a value, so it may be left out.
fn with_defaults(command: clap::Command, defaults: &[(&str, String)]) -> clap::Command {
    command
        .mut_args(|arg| {
            let default = defaults.iter().find(|(id, _)| arg.get_id() == *id);
            match default {
                Some((_, value)) if arg.is_required_set() => {
                    arg.default_value(value.clone()).required(false)
                }
                _ => arg,
            }
        })
        .mut_subcommands(|sub| with_defaults(sub, defaults))
}

/// Whether slash command words ask for a picker, such as `/accounts` or `/accounts list`.
fn is_listing(words: &[String]) -> bool {
    match &words[1..] {
        [] => true,
        [word] => word == "list",
        _ => false,
    }
}

/// Whether the session goes on after a line, and whether to show how long the line took.
enum Flow {
    Timed,
    Untimed,
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
    use crate::store::Credential;
    use crate::testing::{
        FakeClock, FakeDataPlane, FakeManagement, ScriptedConfirm, ScriptedLines, ScriptedPicker,
        account, container,
    };
    use serde_json::json;

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
        repl_picking(lines, &ScriptedPicker::default())
    }

    fn repl_picking(lines: &ScriptedLines, picker: &ScriptedPicker) -> TestRepl {
        let app = App {
            management: shop(),
            data: FakeDataPlane::new("/userId", vec![]),
            input: Box::new(std::io::empty()),
            confirm: Box::new(ScriptedConfirm::answering(false)),
            out: Vec::new(),
            global: GlobalArgs::default(),
        };
        let mut management = shop();
        management.accounts.push(account("inventory"));
        let app = App { management, ..app };
        Repl::new(
            app,
            Box::new(lines.clone()),
            Box::new(picker.clone()),
            Box::new(FakeClock::frozen()),
        )
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
    async fn queries_need_an_account_and_container_first() {
        let lines = ScriptedLines::new(&["SELECT * FROM c"]);
        let mut repl = repl(&lines);

        repl.run().await.unwrap();

        assert_eq!(
            output(&repl),
            "error: select an account with /accounts and a container with /containers first\n\
             Completed in 0 ms\n"
        );
    }

    #[tokio::test]
    async fn accounts_picks_the_current_account_from_a_list() {
        for command in ["/accounts", "/accounts list"] {
            let lines = ScriptedLines::new(&[command]);
            let picker = ScriptedPicker::answering(&[Some(1)]);
            let mut repl = repl_picking(&lines, &picker);

            repl.run().await.unwrap();

            assert_eq!(
                *picker.shown.borrow(),
                vec![vec![
                    "orders (rg-data, West Europe)".to_string(),
                    "inventory (rg-data, West Europe)".to_string(),
                ]]
            );
            assert_eq!(
                output(&repl),
                "Using account inventory\nCompleted in 0 ms\n",
                "{command}"
            );
            assert_eq!(repl.account, Some(account("inventory")));
        }
    }

    #[tokio::test]
    async fn cancelling_the_account_picker_keeps_the_current_account() {
        let lines = ScriptedLines::new(&["/accounts", "/accounts"]);
        let picker = ScriptedPicker::answering(&[Some(0), None]);
        let mut repl = repl_picking(&lines, &picker);

        repl.run().await.unwrap();

        assert_eq!(repl.account, Some(account("orders")));
        assert_eq!(
            output(&repl),
            "Using account orders\nCompleted in 0 ms\nCompleted in 0 ms\n"
        );
    }

    #[tokio::test]
    async fn accounts_says_so_when_there_are_none_instead_of_picking() {
        let lines = ScriptedLines::new(&["/accounts"]);
        let picker = ScriptedPicker::default();
        let mut repl = repl_picking(&lines, &picker);
        repl.app.management.accounts.clear();

        repl.run().await.unwrap();

        assert!(picker.shown.borrow().is_empty());
        assert_eq!(
            output(&repl),
            "error: no Cosmos DB accounts found\nCompleted in 0 ms\n"
        );
    }

    #[tokio::test]
    async fn containers_needs_an_account_first() {
        let lines = ScriptedLines::new(&["/containers"]);
        let picker = ScriptedPicker::default();
        let mut repl = repl_picking(&lines, &picker);

        repl.run().await.unwrap();

        assert!(picker.shown.borrow().is_empty());
        assert_eq!(
            output(&repl),
            "error: select an account with /accounts first\nCompleted in 0 ms\n"
        );
    }

    #[tokio::test]
    async fn containers_picks_the_current_container_across_databases() {
        let lines = ScriptedLines::new(&["/accounts", "/containers"]);
        let picker = ScriptedPicker::answering(&[Some(0), Some(1)]);
        let mut repl = repl_picking(&lines, &picker);

        repl.run().await.unwrap();

        assert_eq!(
            picker.shown.borrow()[1],
            vec!["shop/carts (/userId)", "audit/events (/deviceId)"]
        );
        assert_eq!(
            output(&repl),
            "Using account orders\nCompleted in 0 ms\nUsing container audit/events\nCompleted in 0 ms\n"
        );
        assert_eq!(*repl.app.data.connections.borrow(), vec![Credential::Entra]);
    }

    #[tokio::test]
    async fn queries_run_against_the_current_container_over_one_connection() {
        let lines = ScriptedLines::new(&[
            "/accounts",
            "/containers",
            "SELECT * FROM c",
            "SELECT * FROM c WHERE c.userId = 'u-1'",
        ]);
        let picker = ScriptedPicker::answering(&[Some(0), Some(0)]);
        let mut repl = repl_picking(&lines, &picker);
        repl.app.data =
            FakeDataPlane::new("/userId", vec![json!({ "id": "c-1", "userId": "u-1" })]);

        repl.run().await.unwrap();

        let table = "\
+-----+---------+
| id  | /userId |
+===============+
| c-1 | u-1     |
+-----+---------+
";
        assert_eq!(
            output(&repl),
            format!("{SELECTED}{table}Completed in 0 ms\n{table}Completed in 0 ms\n")
        );
        assert_eq!(
            repl.app.data.container.borrow().queries,
            vec!["SELECT * FROM c", "SELECT * FROM c WHERE c.userId = 'u-1'"]
        );
        assert_eq!(repl.app.data.connections.borrow().len(), 1);
    }

    #[tokio::test]
    async fn switching_accounts_forgets_the_current_container() {
        let lines =
            ScriptedLines::new(&["/accounts", "/containers", "/accounts", "SELECT * FROM c"]);
        let picker = ScriptedPicker::answering(&[Some(0), Some(0), Some(1)]);
        let mut repl = repl_picking(&lines, &picker);

        repl.run().await.unwrap();

        assert!(output(&repl).ends_with(
            "Using account inventory\nCompleted in 0 ms\n\
             error: select an account with /accounts and a container with /containers first\n\
             Completed in 0 ms\n"
        ));
        assert!(repl.app.data.container.borrow().queries.is_empty());
    }

    #[tokio::test]
    async fn containers_says_so_when_the_account_has_none() {
        let lines = ScriptedLines::new(&["/accounts", "/containers"]);
        let picker = ScriptedPicker::answering(&[Some(0)]);
        let mut repl = repl_picking(&lines, &picker);
        repl.app.management.databases.borrow_mut().clear();

        repl.run().await.unwrap();

        assert_eq!(picker.shown.borrow().len(), 1);
        assert!(
            output(&repl)
                .ends_with("error: no containers found on account orders\nCompleted in 0 ms\n")
        );
    }

    const SELECTED: &str =
        "Using account orders\nCompleted in 0 ms\nUsing container shop/carts\nCompleted in 0 ms\n";

    /// A session that picks the orders account and shop/carts container before the given lines.
    fn selected(lines: &[&str], docs: Vec<serde_json::Value>) -> TestRepl {
        let lines: Vec<&str> = ["/accounts", "/containers"]
            .into_iter()
            .chain(lines.iter().copied())
            .collect();
        let picker = ScriptedPicker::answering(&[Some(0), Some(0)]);
        let mut repl = repl_picking(&ScriptedLines::new(&lines), &picker);
        repl.app.data = FakeDataPlane::new("/userId", docs);
        repl
    }

    fn output_after_selection(repl: &TestRepl) -> String {
        output(repl).strip_prefix(SELECTED).unwrap().to_string()
    }

    #[tokio::test]
    async fn output_switches_queries_to_json_and_back() {
        let mut repl = selected(
            &[
                "/output json",
                "SELECT * FROM c",
                "/output table",
                "SELECT * FROM c",
            ],
            vec![json!({ "id": "c-1", "userId": "u-1" })],
        );

        repl.run().await.unwrap();

        let expected = "\
Query output is now json
Completed in 0 ms
[
  {
    \"id\": \"c-1\",
    \"userId\": \"u-1\"
  }
]
Completed in 0 ms
Query output is now table
Completed in 0 ms
+-----+---------+
| id  | /userId |
+===============+
| c-1 | u-1     |
+-----+---------+
Completed in 0 ms
";
        assert_eq!(output_after_selection(&repl), expected);
    }

    #[tokio::test]
    async fn output_rejects_unknown_formats() {
        let mut repl = selected(&["/output yaml", "/output"], vec![]);

        repl.run().await.unwrap();

        assert_eq!(
            output_after_selection(&repl),
            "error: expected /output json or /output table\nCompleted in 0 ms\n\
             error: expected /output json or /output table\nCompleted in 0 ms\n"
        );
    }

    #[tokio::test]
    async fn every_command_and_query_reports_how_long_it_took_without_picking_time() {
        let mut repl = selected(&["SELECT * FROM c", "", "/exit"], vec![]);
        repl.clock = Box::new(FakeClock::stepping(Duration::from_millis(5)));

        repl.run().await.unwrap();

        assert_eq!(
            output(&repl),
            "Using account orders\nCompleted in 10 ms\n\
             Using container shop/carts\nCompleted in 10 ms\n\
             No documents found.\nCompleted in 5 ms\n"
        );
    }

    #[tokio::test]
    async fn slash_commands_run_cli_commands_on_the_current_container() {
        let mut repl = selected(
            &["/items get --id c-1 --pk u-1"],
            vec![json!({ "id": "c-1", "userId": "u-1" })],
        );

        repl.run().await.unwrap();

        assert_eq!(
            output_after_selection(&repl),
            "{\n  \"id\": \"c-1\",\n  \"userId\": \"u-1\"\n}\nCompleted in 0 ms\n"
        );
    }

    #[tokio::test]
    async fn slash_commands_can_name_another_account_explicitly() {
        let lines = ScriptedLines::new(&["/databases list -a orders"]);
        let mut repl = repl(&lines);

        repl.run().await.unwrap();

        assert_eq!(output(&repl), "shop\naudit\nCompleted in 0 ms\n");
    }

    #[tokio::test]
    async fn slash_commands_default_to_the_current_account() {
        let lines = ScriptedLines::new(&["/accounts", "/databases list"]);
        let picker = ScriptedPicker::answering(&[Some(0)]);
        let mut repl = repl_picking(&lines, &picker);

        repl.run().await.unwrap();

        assert!(
            output(&repl).ends_with("shop\naudit\nCompleted in 0 ms\n"),
            "{}",
            output(&repl)
        );
    }

    #[tokio::test]
    async fn slash_command_help_is_printed_as_is() {
        let lines = ScriptedLines::new(&["/query --help"]);
        let mut repl = repl(&lines);

        repl.run().await.unwrap();

        let out = output(&repl);
        assert!(out.starts_with("Query documents in a container\n"), "{out}");
        assert!(!out.contains("Completed in"), "{out}");
    }

    #[tokio::test]
    async fn slash_command_usage_errors_are_printed_once() {
        let lines = ScriptedLines::new(&["/nope"]);
        let mut repl = repl(&lines);

        repl.run().await.unwrap();

        let out = output(&repl);
        assert!(
            out.starts_with("error: unrecognized subcommand 'nope'"),
            "{out}"
        );
        assert!(!out.contains("Completed in"), "{out}");
    }

    #[tokio::test]
    async fn errors_are_reported_and_the_session_goes_on() {
        let lines = ScriptedLines::new(&["/query \"SELECT", "/exit", "SELECT * FROM c"]);
        let mut repl = repl(&lines);

        repl.run().await.unwrap();

        assert_eq!(
            output(&repl),
            "error: unbalanced quotes in command\nCompleted in 0 ms\n"
        );
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
