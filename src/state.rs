//! What the screen shows, and how keys and finished work change it.

use crate::management::{Account, Container};
use std::collections::BTreeSet;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::editor::Editor;
use crate::input::TextInput;
use crate::json::highlight_json_array;
use crate::partition::{display_value, value_at_path};
use crate::query::{DEFAULT_QUERY, build_query};
use crate::settings::{ContainerSettings, Field, SettingsTab};

/// Work for the runtime to do in the background.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    LoadAccounts,
    LoadContainers(Account),
    /// Runs a query on a container. Only the latest query of each origin is shown.
    Query {
        id: u64,
        target: Target,
        sql: String,
        origin: Origin,
    },
    /// Reads the next page of the latest query of its origin.
    LoadMore {
        id: u64,
    },
    /// Puts text on the system clipboard.
    Copy(String),
    /// Writes text to a file.
    Save {
        path: String,
        contents: String,
    },
    /// Deletes a document, found by its id and partition key value.
    Delete {
        target: Target,
        id: String,
        partition_key: Option<Value>,
    },
    /// Deletes several documents, each found by its id and partition key value.
    DeleteMany {
        target: Target,
        items: Vec<(String, Option<Value>)>,
    },
    /// Reads the properties of a container, to show its settings.
    LoadSettings(Target),
    /// Replaces the properties of a container, to save its settings.
    ReplaceSettings {
        target: Target,
        properties: Value,
    },
}

/// Where a query was asked for, each showing its latest query's results on its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Origin {
    /// The search bar, or opening a container.
    Browse,
    /// The query editor.
    Editor,
}

/// A container to query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Target {
    pub account: Account,
    pub database: String,
    pub container: String,
}

/// Finished background work.
#[derive(Debug)]
pub enum Msg {
    AccountsLoaded(Result<Vec<Account>, String>),
    /// The containers of an account, with their database names.
    ContainersLoaded {
        account: String,
        result: Result<Vec<(String, Container)>, String>,
    },
    QueryDone {
        id: u64,
        result: Result<QueryResult, String>,
    },
    /// The next page of a query.
    MoreLoaded {
        id: u64,
        result: Result<QueryResult, String>,
    },
    Copied(Result<(), String>),
    /// The path of a saved file, or why it could not be saved.
    Saved(Result<String, String>),
    /// The outcome of deleting the document with this id.
    Deleted {
        id: String,
        result: Result<(), String>,
    },
    /// The outcome of deleting each of several documents, by id.
    DeletedMany {
        results: Vec<(String, Result<(), String>)>,
    },
    /// The properties of a container, for its settings.
    SettingsLoaded {
        target: Target,
        result: Result<Value, String>,
    },
    /// The properties of a container as saved, or why they could not be.
    SettingsSaved {
        target: Target,
        result: Result<Value, String>,
    },
}

/// The documents a query found.
#[derive(Debug)]
pub struct QueryResult {
    pub docs: Vec<Value>,
    /// Whether the query found more documents than these.
    pub more: bool,
    /// The partition key path of the queried container.
    pub pk_path: String,
    pub elapsed: Duration,
    pub stats: QueryStats,
}

/// The latest query of the query editor and the documents it found,
/// kept apart from the results the browse panes show.
#[derive(Debug, Default)]
pub struct QueryOutput {
    /// The id of the latest editor query.
    pub id: u64,
    pub results: Vec<Value>,
    /// Whether the query found more documents than the results show.
    pub more: bool,
    /// Whether the next page of results is being read.
    pub loading_more: bool,
    /// What reading the results cost so far.
    pub stats: QueryStats,
}

/// What reading documents of a query cost.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct QueryStats {
    /// The request units the reads used.
    pub request_charge: f64,
    /// How many pages were read from the service.
    pub round_trips: usize,
    /// The query metrics the service reported, by name, added up over the pages.
    pub query_metrics: Vec<(String, f64)>,
}

impl QueryStats {
    /// Adds what more reads cost to these.
    pub fn add(&mut self, more: QueryStats) {
        self.request_charge += more.request_charge;
        self.round_trips += more.round_trips;
        for (name, value) in more.query_metrics {
            self.add_metric(name, value);
        }
    }

    /// Adds the metrics the service reports for a page, as `name=value` pairs
    /// separated by semicolons. Values that are not numbers are left out.
    pub fn add_query_metrics(&mut self, metrics: &str) {
        for pair in metrics.split(';') {
            let Some((name, value)) = pair.split_once('=') else {
                continue;
            };
            if let Ok(value) = value.trim().parse::<f64>() {
                self.add_metric(name.trim().to_string(), value);
            }
        }
    }

    fn add_metric(&mut self, name: String, value: f64) {
        match self.query_metrics.iter_mut().find(|(n, _)| *n == name) {
            Some((_, total)) => *total += value,
            None => self.query_metrics.push((name, value)),
        }
    }
}

/// Something that happened, for [`update`] to act on.
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(Mouse),
    Msg(Msg),
}

/// A click, drag, release or turn of the wheel over a pane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mouse {
    pub action: MouseAction,
    pub pane: Focus,
    /// Where in the pane, inside its borders, or `None` on a border.
    pub at: Option<Position>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MouseAction {
    Click,
    /// The mouse moved with its button held down.
    Drag,
    /// The button was let go.
    Release,
    ScrollUp,
    ScrollDown,
}

/// Something shown in a pane that may still be loading or may have failed.
#[derive(Debug)]
pub enum Load<T> {
    Loading,
    Loaded(T),
    Failed(String),
}

/// An account in the tree.
#[derive(Debug)]
pub struct AccountNode {
    pub account: Account,
    pub expanded: bool,
    /// The account's databases, once they were asked for.
    pub databases: Option<Load<Vec<DatabaseNode>>>,
}

impl AccountNode {
    /// An account whose databases were not asked for yet.
    fn closed(account: Account) -> Self {
        AccountNode {
            account,
            expanded: false,
            databases: None,
        }
    }
}

/// A database in the tree, under its account.
#[derive(Debug)]
pub struct DatabaseNode {
    pub name: String,
    pub expanded: bool,
    pub containers: Vec<Container>,
}

/// What a tree row stands for, by position in the tree.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Node {
    Account(usize),
    Database(usize, usize),
    Container(usize, usize, usize),
    /// A line about the account's databases, such as that they are loading.
    Note(usize),
    /// A line about the account list.
    AccountsNote,
}

/// The names leading to a tree node.
#[derive(Debug, Clone, PartialEq)]
struct TreePath {
    account: String,
    database: Option<String>,
    container: Option<String>,
}

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub node: Node,
    pub depth: usize,
    pub label: String,
    /// Whether the node is open, for nodes that can open.
    pub expanded: Option<bool>,
}

/// The message on the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Info(String),
    Error(String),
}

/// A place in the pretty-printed document, in characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct DocPoint {
    pub line: usize,
    pub column: usize,
}

/// Text picked in the document, from where it started to where it reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Selection {
    pub anchor: DocPoint,
    pub head: DocPoint,
}

impl Selection {
    /// The first and last picked characters, in reading order.
    pub fn range(&self) -> (DocPoint, DocPoint) {
        (self.anchor.min(self.head), self.anchor.max(self.head))
    }
}

/// The pane that keys go to.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Focus {
    Tree,
    Search,
    Results,
    Document,
    /// The query editor, in query mode.
    Editor,
    /// The documents the query editor's query found, in query mode.
    Output,
    /// The settings tab, with time to live and the geospatial type, in settings mode.
    SettingsForm,
    /// The indexing policy tab, in settings mode.
    IndexingPolicy,
    /// The computed properties tab, in settings mode.
    ComputedProperties,
}

/// A dialog asking where to save the query or its results.
#[derive(Debug)]
pub struct SavePrompt {
    pub saving: Saving,
    pub path: TextInput,
}

/// What a save dialog saves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Saving {
    /// The query in the query editor.
    Query,
    /// The documents the query found, as a JSON array.
    Results,
}

/// Which panes show next to the tree.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Mode {
    /// The search bar, the results and the selected document.
    Browse,
    /// The query editor and the documents its query found.
    Query,
    /// The settings of the picked container.
    Settings,
}

/// What the query editor's output shows.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum OutputTab {
    /// The documents the query found.
    Results,
    /// What the query cost.
    Stats,
}

impl Mode {
    /// The panes Tab cycles through; the search bar is only reached with `/`.
    fn panes(self) -> &'static [Focus] {
        match self {
            Mode::Browse => &[Focus::Tree, Focus::Results, Focus::Document],
            Mode::Query => &[Focus::Tree, Focus::Editor, Focus::Output],
            Mode::Settings => &[
                Focus::Tree,
                Focus::SettingsForm,
                Focus::IndexingPolicy,
                Focus::ComputedProperties,
            ],
        }
    }
}

impl Focus {
    /// The settings tab this pane shows, for the panes of settings mode.
    pub fn settings_tab(self) -> Option<SettingsTab> {
        match self {
            Focus::SettingsForm => Some(SettingsTab::Settings),
            Focus::IndexingPolicy => Some(SettingsTab::IndexingPolicy),
            Focus::ComputedProperties => Some(SettingsTab::ComputedProperties),
            _ => None,
        }
    }

    fn next(self, mode: Mode) -> Focus {
        let order = mode.panes();
        match order.iter().position(|f| *f == self) {
            Some(index) => order[(index + 1) % order.len()],
            None => order[1],
        }
    }

    fn previous(self, mode: Mode) -> Focus {
        let order = mode.panes();
        match order.iter().position(|f| *f == self) {
            Some(index) => order[(index + order.len() - 1) % order.len()],
            None => order[0],
        }
    }
}

/// Everything the screen shows.
pub struct AppState {
    pub accounts: Load<Vec<AccountNode>>,
    pub status: Status,
    pub quit: bool,
    /// Whether the key help covers the screen.
    pub show_help: bool,
    /// Whether a dialog asks to confirm deleting the selected document.
    pub confirm_delete: bool,
    /// Whether a dialog asks to discard the changes to the settings.
    pub confirm_discard: bool,
    /// Whether the accounts tree is hidden, leaving its room to the other panes.
    pub tree_hidden: bool,
    /// Whether the focused pane takes the whole screen.
    pub zoomed: bool,
    pub focus: Focus,
    pub mode: Mode,
    /// The query written in query mode.
    pub editor: Editor,
    /// How many lines the query editor shows at once.
    pub editor_height: u16,
    /// How many lines the query output is scrolled down.
    pub output_scroll: u16,
    /// How many lines of the query output show at once.
    pub output_height: u16,
    pub output_tab: OutputTab,
    /// What is typed in the search bar.
    pub search: TextInput,
    /// Index of the selected tree row.
    pub tree_selected: usize,
    /// The container that queries run against.
    pub target: Option<Target>,
    /// How many queries were asked for, to give each its own id.
    pub queries: u64,
    /// The id of the latest browse query.
    pub query_id: u64,
    /// The documents the latest query found.
    pub results: Vec<Value>,
    /// The partition key path of the container the results came from.
    pub pk_path: String,
    /// Index of the selected document in the results.
    pub result_selected: usize,
    /// Indices of the results marked to delete together.
    pub marked: BTreeSet<usize>,
    /// The SQL of the latest query, to run again on refresh.
    pub last_sql: String,
    /// How many lines the document pane is scrolled down.
    pub doc_scroll: u16,
    /// How many lines of a document the document pane shows at once.
    pub doc_height: u16,
    /// Text picked in the document, to copy.
    pub doc_selection: Option<Selection>,
    /// How many documents the results table shows at once.
    pub results_height: u16,
    /// How many rows the tree shows at once.
    pub tree_height: u16,
    /// Whether a `g` was just pressed, waiting for a second one.
    pub pending_g: bool,
    /// Whether the latest query found more documents than the results show.
    pub more: bool,
    /// Whether the next page of results is being read.
    pub loading_more: bool,
    /// What reading the latest query's results cost so far.
    pub stats: QueryStats,
    /// The dialog asking where to save, while it is open.
    pub prompt: Option<SavePrompt>,
    /// What the query editor's latest query found.
    pub output: QueryOutput,
    /// Whether another container was picked in query mode, so the browse results
    /// are listed again when it is left.
    pub browse_stale: bool,
    /// Why the latest query failed, shown in a dialog until the next key.
    pub error: Option<String>,
    /// The settings of the picked container, while settings mode shows them.
    pub settings: Option<Load<ContainerSettings>>,
    /// The mode to go back to when settings mode is left.
    pub settings_from: Mode,
    /// The settings tab shown, also while the tree has the focus.
    pub settings_tab: SettingsTab,
    /// How many lines the settings tabs show at once.
    pub settings_height: u16,
    /// The id of the query refreshing documents an earlier run saved, while it runs.
    pub refreshing_query: Option<u64>,
}

impl AppState {
    /// The state on startup, with the work to start right away.
    pub fn new() -> (Self, Vec<Effect>) {
        let state = AppState {
            accounts: Load::Loading,
            status: Status::Info("Loading accounts…".into()),
            quit: false,
            show_help: false,
            confirm_delete: false,
            confirm_discard: false,
            tree_hidden: false,
            zoomed: false,
            focus: Focus::Tree,
            mode: Mode::Browse,
            editor: Editor::default(),
            editor_height: 0,
            output_scroll: 0,
            output_height: 0,
            output_tab: OutputTab::Results,
            search: TextInput::default(),
            tree_selected: 0,
            target: None,
            queries: 0,
            query_id: 0,
            results: Vec::new(),
            pk_path: String::new(),
            result_selected: 0,
            marked: BTreeSet::new(),
            last_sql: String::new(),
            doc_scroll: 0,
            doc_height: 0,
            doc_selection: None,
            results_height: 0,
            tree_height: 0,
            pending_g: false,
            more: false,
            loading_more: false,
            stats: QueryStats::default(),
            prompt: None,
            output: QueryOutput::default(),
            browse_stale: false,
            error: None,
            settings: None,
            settings_from: Mode::Browse,
            settings_tab: SettingsTab::Settings,
            settings_height: 0,
            refreshing_query: None,
        };
        (state, vec![Effect::LoadAccounts])
    }

    /// The state on startup, showing accounts saved by an earlier run while they refresh.
    pub fn with_cached_accounts(cached: Option<Vec<Account>>) -> (Self, Vec<Effect>) {
        let (mut state, effects) = Self::new();
        if let Some(accounts) = cached {
            let nodes = accounts.into_iter().map(AccountNode::closed).collect();
            state.accounts = Load::Loaded(nodes);
            state.status = Status::Info("Refreshing accounts…".into());
        }
        (state, effects)
    }

    /// The tree lines that are visible, top to bottom.
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        let accounts = match &self.accounts {
            Load::Loading => return vec![row(Node::AccountsNote, 0, "loading accounts…")],
            Load::Loaded(accounts) => accounts,
            Load::Failed(_) => return Vec::new(),
        };
        let mut rows = Vec::new();
        for (a, node) in accounts.iter().enumerate() {
            rows.push(TreeRow {
                expanded: Some(node.expanded),
                ..row(Node::Account(a), 0, &node.account.name)
            });
            if !node.expanded {
                continue;
            }
            match &node.databases {
                None => {}
                Some(Load::Loading) => rows.push(row(Node::Note(a), 1, "loading…")),
                Some(Load::Failed(error)) => {
                    rows.push(row(Node::Note(a), 1, &format!("error: {error}")));
                }
                Some(Load::Loaded(databases)) => {
                    for (d, database) in databases.iter().enumerate() {
                        rows.push(TreeRow {
                            expanded: Some(database.expanded),
                            ..row(Node::Database(a, d), 1, &database.name)
                        });
                        if !database.expanded {
                            continue;
                        }
                        for (c, container) in database.containers.iter().enumerate() {
                            rows.push(row(Node::Container(a, d, c), 2, &container.name));
                        }
                    }
                }
            }
        }
        rows
    }

    /// The pane that takes the whole screen, if any.
    ///
    /// The search bar is never zoomed: every pane shows while searching.
    pub fn zoomed_pane(&self) -> Option<Focus> {
        (self.zoomed && self.focus != Focus::Search).then_some(self.focus)
    }

    /// The document shown in the document pane.
    pub fn selected_document(&self) -> Option<&Value> {
        self.results.get(self.result_selected)
    }

    /// How far the document pane scrolls: until the last line is at the bottom.
    pub fn max_doc_scroll(&self) -> u16 {
        self.document_lines().saturating_sub(self.doc_height)
    }

    /// How many lines the selected document takes when pretty-printed.
    pub fn document_lines(&self) -> u16 {
        let lines = self.selected_document().map_or(0, |doc| {
            let pretty = serde_json::to_string_pretty(doc).unwrap_or_default();
            pretty.lines().count()
        });
        u16::try_from(lines).unwrap_or(u16::MAX)
    }

    /// How many lines the query output takes: the results as one pretty-printed array.
    pub fn output_lines(&self) -> u16 {
        u16::try_from(highlight_json_array(&self.output.results).len()).unwrap_or(u16::MAX)
    }

    /// How far the query output scrolls: until its last line is at the bottom.
    pub fn max_output_scroll(&self) -> u16 {
        self.output_lines().saturating_sub(self.output_height)
    }

    /// The picked text of the document, or `None` when nothing is picked.
    pub fn selected_text(&self) -> Option<String> {
        let selection = self.doc_selection?;
        if selection.anchor == selection.head {
            return None;
        }
        let pretty = serde_json::to_string_pretty(self.selected_document()?).ok()?;
        let (start, end) = selection.range();
        let picked: Vec<String> = pretty
            .lines()
            .enumerate()
            .skip(start.line)
            .take(end.line + 1 - start.line)
            .map(|(index, line)| {
                let from = if index == start.line { start.column } else { 0 };
                let to = if index == end.line {
                    end.column + 1
                } else {
                    usize::MAX
                };
                line.chars()
                    .skip(from)
                    .take(to.saturating_sub(from))
                    .collect()
            })
            .collect();
        Some(picked.join(
            "
",
        ))
    }

    /// What the selected tree row stands for.
    fn selected_node(&self) -> Option<Node> {
        let rows = self.tree_rows();
        rows.get(self.tree_selected).map(|row| row.node)
    }

    fn account_mut(&mut self, index: usize) -> Option<&mut AccountNode> {
        match &mut self.accounts {
            Load::Loaded(accounts) => accounts.get_mut(index),
            Load::Loading | Load::Failed(_) => None,
        }
    }

    fn database_mut(&mut self, account: usize, index: usize) -> Option<&mut DatabaseNode> {
        match &mut self.account_mut(account)?.databases {
            Some(Load::Loaded(databases)) => databases.get_mut(index),
            _ => None,
        }
    }

    /// The names leading to a tree node, which stay the same when the tree is rebuilt.
    fn path_of(&self, node: Option<Node>) -> Option<TreePath> {
        let Load::Loaded(accounts) = &self.accounts else {
            return None;
        };
        let (a, d, c) = match node? {
            Node::Account(a) | Node::Note(a) => (a, None, None),
            Node::Database(a, d) => (a, Some(d), None),
            Node::Container(a, d, c) => (a, Some(d), Some(c)),
            Node::AccountsNote => return None,
        };
        let account = accounts.get(a)?;
        let database = match (&account.databases, d) {
            (Some(Load::Loaded(databases)), Some(d)) => databases.get(d),
            _ => None,
        };
        let container = database
            .zip(c)
            .and_then(|(database, c)| database.containers.get(c));
        Some(TreePath {
            account: account.account.name.clone(),
            database: database.map(|database| database.name.clone()),
            container: container.map(|container| container.name.clone()),
        })
    }

    /// Selects the row at a path, or its account when the path is gone, or the nearest row.
    fn reselect(&mut self, path: Option<TreePath>) {
        let rows = self.tree_rows();
        let position = |path: &TreePath| {
            rows.iter()
                .position(|row| self.path_of(Some(row.node)).as_ref() == Some(path))
        };
        let found = path.and_then(|path| {
            let account = TreePath {
                account: path.account.clone(),
                database: None,
                container: None,
            };
            position(&path).or_else(|| position(&account))
        });
        self.tree_selected = found.unwrap_or(self.tree_selected.min(rows.len().saturating_sub(1)));
    }

    /// Selects the tree row of a node, if it is visible.
    fn select_node(&mut self, node: Node) {
        if let Some(index) = self.tree_rows().iter().position(|row| row.node == node) {
            self.tree_selected = index;
        }
    }
}

fn row(node: Node, depth: usize, label: &str) -> TreeRow {
    TreeRow {
        node,
        depth,
        label: label.to_string(),
        expanded: None,
    }
}

/// Applies an event to the state, returning the background work it starts.
pub fn update(state: &mut AppState, event: Event) -> Vec<Effect> {
    match event {
        Event::Key(key) => on_key(state, key),
        Event::Mouse(mouse) => on_mouse(state, mouse),
        Event::Msg(msg) => on_msg(state, msg),
    }
}

fn on_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if key.code != KeyCode::Char('g') {
        state.pending_g = false;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let ctrl_c = ctrl && key.code == KeyCode::Char('c');
    if state.confirm_delete && !ctrl_c {
        // Only y or Enter confirms. Any other key cancels, and only that
        state.confirm_delete = false;
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Enter) {
            return delete_selected(state);
        }
        return Vec::new();
    }
    if state.confirm_discard && !ctrl_c {
        // Only y or Enter discards. Any other key keeps the changes, and only that
        state.confirm_discard = false;
        if matches!(key.code, KeyCode::Char('y') | KeyCode::Enter) {
            return close_settings(state);
        }
        return Vec::new();
    }
    if state.error.is_some() && !ctrl_c {
        // Any other key closes the error, and only that
        state.error = None;
        return Vec::new();
    }
    if state.prompt.is_some() && !ctrl_c {
        return on_prompt_key(state, key);
    }
    if state.show_help && !ctrl_c {
        // Any other key closes the help, and only that
        state.show_help = false;
        return Vec::new();
    }
    match key.code {
        KeyCode::Char('c') if ctrl => state.quit = true,
        KeyCode::Char('b') if ctrl => toggle_tree(state),
        KeyCode::Char('s') if ctrl && state.mode == Mode::Settings => return save_settings(state),
        KeyCode::Esc if state.mode == Mode::Settings => return leave_settings(state),
        KeyCode::Tab => move_focus(state, Focus::next),
        KeyCode::BackTab => move_focus(state, Focus::previous),
        _ => {
            return match state.focus {
                Focus::Search => on_search_key(state, key),
                Focus::Editor => on_editor_key(state, key),
                Focus::IndexingPolicy | Focus::ComputedProperties => {
                    on_settings_editor_key(state, key)
                }
                _ => on_pane_key(state, key),
            };
        }
    }
    Vec::new()
}

/// Moves the focus one pane along, past the tree while it is hidden.
fn move_focus(state: &mut AppState, step: fn(Focus, Mode) -> Focus) {
    state.focus = step(state.focus, state.mode);
    if state.tree_hidden && state.focus == Focus::Tree {
        state.focus = step(state.focus, state.mode);
    }
    if let Some(tab) = state.focus.settings_tab() {
        state.settings_tab = tab;
    }
}

/// Hides or shows the tree, moving the focus off it when it hides.
fn toggle_tree(state: &mut AppState) {
    state.tree_hidden = !state.tree_hidden;
    if state.tree_hidden && state.focus == Focus::Tree {
        state.focus = match state.mode {
            Mode::Browse => Focus::Search,
            Mode::Query => Focus::Editor,
            Mode::Settings => Focus::SettingsForm,
        };
    }
}

/// How many lines the wheel scrolls the document.
const WHEEL_LINES: usize = 3;

fn on_mouse(state: &mut AppState, mouse: Mouse) -> Vec<Effect> {
    state.pending_g = false;
    if state.show_help {
        // A click closes the help, and only that
        if mouse.action == MouseAction::Click {
            state.show_help = false;
        }
        return Vec::new();
    }
    if mouse.action == MouseAction::Click && mouse.pane.settings_tab().is_some() {
        // The settings pane is the tab it shows
        state.focus = settings_focus(state.settings_tab);
        return Vec::new();
    }
    match mouse.action {
        MouseAction::Click => on_click(state, mouse.pane, mouse.at),
        // Some terminals send no moves, so the release ends a drag too
        MouseAction::Drag | MouseAction::Release => on_drag(state, mouse.pane, mouse.at),
        MouseAction::ScrollUp => on_wheel(state, mouse.pane, KeyCode::Up),
        MouseAction::ScrollDown => on_wheel(state, mouse.pane, KeyCode::Down),
    }
}

/// The first row a list shows, as ratatui scrolls it: from the top,
/// until the selection would go past the bottom.
fn first_visible(selected: usize, height: u16) -> usize {
    selected.saturating_sub(usize::from(height).saturating_sub(1))
}

/// Focuses the clicked pane. A click on a row selects it,
/// a click on the selected tree row opens or closes it like Enter,
/// and a click in the document starts picking text there.
fn on_click(state: &mut AppState, pane: Focus, at: Option<Position>) -> Vec<Effect> {
    state.focus = pane;
    let Some(at) = at else {
        return Vec::new();
    };
    let line = usize::from(at.y);
    match pane {
        Focus::Tree => {
            let index = first_visible(state.tree_selected, state.tree_height) + line;
            if index == state.tree_selected {
                return toggle(state);
            }
            if index < state.tree_rows().len() {
                state.tree_selected = index;
                return open_selected_container(state);
            }
        }
        // The first line is the header
        Focus::Results if line > 0 => {
            let index = first_visible(state.result_selected, state.results_height) + line - 1;
            if index < state.results.len() && index != state.result_selected {
                state.result_selected = index;
                show_from_top(state);
            }
        }
        Focus::Search => state.search.move_to(usize::from(at.x)),
        Focus::Editor => {
            let editor = &mut state.editor;
            let row = editor.scroll(usize::from(state.editor_height)) + line;
            // Less the line numbers and the space after them
            let column = usize::from(at.x).saturating_sub(editor.number_width() + 1);
            editor.move_to(row, column);
        }
        Focus::Document => {
            let point = doc_point(state, at);
            state.doc_selection = Some(Selection {
                anchor: point,
                head: point,
            });
        }
        Focus::Results
        | Focus::Output
        | Focus::SettingsForm
        | Focus::IndexingPolicy
        | Focus::ComputedProperties => {}
    }
    Vec::new()
}

/// Picks the document text up to where the mouse is dragged.
fn on_drag(state: &mut AppState, pane: Focus, at: Option<Position>) -> Vec<Effect> {
    if let (Focus::Document, Some(at)) = (pane, at) {
        let point = doc_point(state, at);
        if let Some(selection) = &mut state.doc_selection {
            selection.head = point;
        }
    }
    Vec::new()
}

/// The place in the document under a point inside the document pane.
fn doc_point(state: &AppState, at: Position) -> DocPoint {
    // The pane may have grown since the document was scrolled
    let scroll = state.doc_scroll.min(state.max_doc_scroll());
    DocPoint {
        line: usize::from(scroll) + usize::from(at.y),
        column: usize::from(at.x),
    }
}

/// Shows another document from its top, with nothing picked.
fn show_from_top(state: &mut AppState) {
    state.doc_scroll = 0;
    state.doc_selection = None;
}

/// Scrolls the pane under the mouse like the arrow keys, without focusing it.
fn on_wheel(state: &mut AppState, pane: Focus, code: KeyCode) -> Vec<Effect> {
    let key = KeyEvent::from(code);
    match pane {
        Focus::Tree => on_tree_key(state, key),
        Focus::Results => on_results_key(state, key),
        Focus::Document => (0..WHEEL_LINES)
            .flat_map(|_| on_document_key(state, key))
            .collect(),
        Focus::Output => (0..WHEEL_LINES)
            .flat_map(|_| on_output_key(state, key))
            .collect(),
        Focus::Search
        | Focus::Editor
        | Focus::SettingsForm
        | Focus::IndexingPolicy
        | Focus::ComputedProperties => Vec::new(),
    }
}

/// Keys that work in every pane but the search bar, where they are typed instead.
fn on_pane_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('q') => state.quit = true,
        KeyCode::Char('/') if state.mode == Mode::Query => state.focus = Focus::Editor,
        KeyCode::Char('/') if state.mode == Mode::Settings => {}
        KeyCode::Char('/') => state.focus = Focus::Search,
        KeyCode::Char('?') => state.show_help = true,
        KeyCode::Char('z') => state.zoomed = !state.zoomed,
        KeyCode::Char('n') if state.mode == Mode::Query => return leave_query_editor(state),
        KeyCode::Char('n') if state.mode == Mode::Settings => {}
        KeyCode::Char('n') => open_query_editor(state),
        // Ctrl-S saves, and s in the query output switches its tab
        KeyCode::Char('s') if plain_s(state, key) && state.mode == Mode::Settings => {
            return leave_settings(state);
        }
        KeyCode::Char('s') if plain_s(state, key) => return open_settings(state),
        _ => {
            return match state.focus {
                Focus::Tree => on_tree_key(state, key),
                Focus::Results => on_results_key(state, key),
                Focus::Document => on_document_key(state, key),
                Focus::Output => on_output_key(state, key),
                Focus::SettingsForm => on_settings_form_key(state, key),
                Focus::Search
                | Focus::Editor
                | Focus::IndexingPolicy
                | Focus::ComputedProperties => Vec::new(),
            };
        }
    }
    Vec::new()
}

/// Shows the search bar, results and document again, keeping the query in the editor,
/// and lists the documents of a container picked meanwhile.
fn leave_query_editor(state: &mut AppState) -> Vec<Effect> {
    state.mode = Mode::Browse;
    if matches!(state.focus, Focus::Editor | Focus::Output) {
        state.focus = Focus::Results;
    }
    if std::mem::take(&mut state.browse_stale) {
        return run_query(state, DEFAULT_QUERY.to_string(), Origin::Browse);
    }
    Vec::new()
}

fn on_output_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if is_run_key(key) || key.code == KeyCode::Char('r') {
        return run_editor_query(state);
    }
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        ask_where_to_save(state, Saving::Query);
        return Vec::new();
    }
    if key.code == KeyCode::Char('w') {
        ask_where_to_save(state, Saving::Results);
        return Vec::new();
    }
    if key.code == KeyCode::Char('s') {
        state.output_tab = match state.output_tab {
            OutputTab::Results => OutputTab::Stats,
            OutputTab::Stats => OutputTab::Results,
        };
        return Vec::new();
    }
    if key.code == KeyCode::Char('y') {
        let json = serde_json::to_string_pretty(&state.output.results).unwrap_or_default();
        return vec![Effect::Copy(json)];
    }
    let last = usize::from(state.max_output_scroll());
    let page = usize::from(state.output_height).max(1);
    let scroll = usize::from(state.output_scroll);
    let target = jump(key, &mut state.pending_g, scroll, last, page).unwrap_or(match key.code {
        KeyCode::Down | KeyCode::Char('j') if scroll >= last => return load_more_output(state),
        KeyCode::Down | KeyCode::Char('j') => scroll + 1,
        KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
        KeyCode::Esc => return leave_query_editor(state),
        _ => scroll,
    });
    state.output_scroll = u16::try_from(target).unwrap_or(u16::MAX);
    Vec::new()
}

/// Shows the query editor in place of the results and document, starting it with
/// the latest query when nothing is written in it yet.
fn open_query_editor(state: &mut AppState) {
    if state.target.is_none() {
        state.status = Status::Error("pick a container first".into());
        return;
    }
    state.mode = Mode::Query;
    state.focus = Focus::Editor;
    if !state.editor.text().trim().is_empty() {
        return;
    }
    let sql = if state.last_sql.is_empty() {
        DEFAULT_QUERY
    } else {
        &state.last_sql
    };
    state.editor.set_text(sql);
}

/// Whether `s` opens or leaves the settings: without Ctrl, and outside the query output.
fn plain_s(state: &AppState, key: KeyEvent) -> bool {
    !key.modifiers.contains(KeyModifiers::CONTROL) && state.focus != Focus::Output
}

/// Shows the settings of the picked container in place of the other panes,
/// and asks for them.
fn open_settings(state: &mut AppState) -> Vec<Effect> {
    let Some(target) = state.target.clone() else {
        state.status = Status::Error("pick a container first".into());
        return Vec::new();
    };
    state.settings_from = state.mode;
    state.mode = Mode::Settings;
    state.focus = Focus::SettingsForm;
    state.settings_tab = SettingsTab::Settings;
    state.settings = Some(Load::Loading);
    vec![Effect::LoadSettings(target)]
}

/// The loaded settings, to edit.
fn settings_mut(state: &mut AppState) -> Option<&mut ContainerSettings> {
    match &mut state.settings {
        Some(Load::Loaded(settings)) => Some(settings),
        _ => None,
    }
}

/// The pane that shows a settings tab.
pub fn settings_focus(tab: SettingsTab) -> Focus {
    match tab {
        SettingsTab::Settings => Focus::SettingsForm,
        SettingsTab::IndexingPolicy => Focus::IndexingPolicy,
        SettingsTab::ComputedProperties => Focus::ComputedProperties,
    }
}

/// Saves the edited settings, or shows on its tab why they cannot be saved.
fn save_settings(state: &mut AppState) -> Vec<Effect> {
    let Some(target) = state.target.clone() else {
        return Vec::new();
    };
    let Some(settings) = settings_mut(state) else {
        return Vec::new();
    };
    match settings.to_properties() {
        Ok(properties) => {
            state.status = Status::Info("Saving the settings…".into());
            vec![Effect::ReplaceSettings { target, properties }]
        }
        Err(error) => {
            state.settings_tab = error.tab;
            state.focus = settings_focus(error.tab);
            state.error = Some(error.message);
            Vec::new()
        }
    }
}

/// Moves through the settings form and changes its choices, or edits the seconds.
fn on_settings_form_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(settings) = settings_mut(state) else {
        return Vec::new();
    };
    let seconds = &mut settings.seconds;
    match (settings.field, key.code) {
        (_, KeyCode::Down | KeyCode::Char('j')) => settings.next_field(),
        (_, KeyCode::Up | KeyCode::Char('k')) => settings.previous_field(),
        (Field::Seconds, KeyCode::Char(c)) if c.is_ascii_digit() => seconds.insert(c),
        (Field::Seconds, KeyCode::Backspace) => seconds.backspace(),
        (Field::Seconds, KeyCode::Delete) => seconds.delete(),
        (Field::Seconds, KeyCode::Left) => seconds.left(),
        (Field::Seconds, KeyCode::Right) => seconds.right(),
        (Field::Seconds, KeyCode::Home) => seconds.home(),
        (Field::Seconds, KeyCode::End) => seconds.end(),
        (_, KeyCode::Right | KeyCode::Char('l' | ' ')) => settings.next_choice(),
        (_, KeyCode::Left | KeyCode::Char('h')) => settings.previous_choice(),
        _ => {}
    }
    Vec::new()
}

/// Goes back to the mode settings mode was opened from, after asking to discard
/// any changes.
fn leave_settings(state: &mut AppState) -> Vec<Effect> {
    if settings_changed(state) {
        state.confirm_discard = true;
        return Vec::new();
    }
    close_settings(state)
}

/// Whether any of the settings shown were edited since they were loaded or saved.
fn settings_changed(state: &AppState) -> bool {
    let tabs = [
        SettingsTab::Settings,
        SettingsTab::IndexingPolicy,
        SettingsTab::ComputedProperties,
    ];
    match &state.settings {
        Some(Load::Loaded(settings)) => tabs.into_iter().any(|tab| settings.modified(tab)),
        _ => false,
    }
}

/// Goes back to the mode settings mode was opened from, dropping the settings.
fn close_settings(state: &mut AppState) -> Vec<Effect> {
    state.settings = None;
    state.mode = state.settings_from;
    if state.focus != Focus::Tree {
        state.focus = state.mode.panes()[1];
    }
    if state.mode == Mode::Browse && std::mem::take(&mut state.browse_stale) {
        return run_query(state, DEFAULT_QUERY.to_string(), Origin::Browse);
    }
    Vec::new()
}

/// Where a key that jumps by pages or to an end moves a position, from 0 to `last`.
///
/// `gg` takes two presses, so `pending_g` remembers a first one.
fn jump(
    key: KeyEvent,
    pending_g: &mut bool,
    position: usize,
    last: usize,
    page: usize,
) -> Option<usize> {
    let half = (page / 2).max(1);
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let first_g = key.code == KeyCode::Char('g') && !*pending_g;
    *pending_g = first_g;
    let target = match key.code {
        KeyCode::Char('g') if first_g => return None,
        KeyCode::Char('g') => 0,
        KeyCode::Char('G') => last,
        KeyCode::PageDown => position.saturating_add(page),
        KeyCode::PageUp => position.saturating_sub(page),
        KeyCode::Char('d') if ctrl => position.saturating_add(half),
        KeyCode::Char('u') if ctrl => position.saturating_sub(half),
        KeyCode::Home => 0,
        KeyCode::End => last,
        _ => return None,
    };
    Some(target.min(last))
}

fn on_results_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let selected = state.result_selected;
    let last = state.results.len().saturating_sub(1);
    let page = usize::from(state.results_height).max(1);
    if let Some(target) = jump(key, &mut state.pending_g, selected, last, page) {
        state.result_selected = target;
        if target != selected {
            show_from_top(state);
        }
        return Vec::new();
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            let last = state.results.len().saturating_sub(1);
            if state.result_selected == last && state.more {
                return load_more(state);
            }
            state.result_selected = (state.result_selected + 1).min(last);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.result_selected = state.result_selected.saturating_sub(1);
        }
        KeyCode::Enter if state.selected_document().is_some() => state.focus = Focus::Document,
        KeyCode::Char(' ') if state.selected_document().is_some() => {
            if !state.marked.remove(&state.result_selected) {
                state.marked.insert(state.result_selected);
            }
            state.result_selected = (state.result_selected + 1).min(last);
        }
        KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.marked = (0..state.results.len()).collect();
        }
        KeyCode::Esc => state.marked.clear(),
        KeyCode::Char('r') => return run_query(state, state.last_sql.clone(), Origin::Browse),
        KeyCode::Char('d') if state.selected_document().is_some() => state.confirm_delete = true,
        _ => {}
    }
    if state.result_selected != selected {
        show_from_top(state);
    }
    Vec::new()
}

/// Reads the next page of the query editor's query, unless it found everything or is reading it already.
fn load_more_output(state: &mut AppState) -> Vec<Effect> {
    let output = &mut state.output;
    if !output.more || output.loading_more {
        return Vec::new();
    }
    output.loading_more = true;
    state.status = Status::Info("Loading more documents…".into());
    vec![Effect::LoadMore { id: output.id }]
}

/// Reads the next page of the latest browse query, unless it found everything or is reading it already.
fn load_more(state: &mut AppState) -> Vec<Effect> {
    // Documents an earlier run saved have no open query until the refresh finds them
    let refreshing = state.refreshing_query == Some(state.query_id);
    if !state.more || state.loading_more || refreshing {
        return Vec::new();
    }
    state.loading_more = true;
    state.status = Status::Info("Loading more documents…".into());
    vec![Effect::LoadMore { id: state.query_id }]
}

fn on_document_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('y') => return copy_document(state),
        KeyCode::Char('d') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
            state.confirm_delete = state.selected_document().is_some();
            return Vec::new();
        }
        KeyCode::Esc => state.doc_selection = None,
        _ => {}
    }
    let last = usize::from(state.max_doc_scroll());
    let page = usize::from(state.doc_height).max(1);
    let scroll = usize::from(state.doc_scroll);
    let target = jump(key, &mut state.pending_g, scroll, last, page).unwrap_or(match key.code {
        KeyCode::Down | KeyCode::Char('j') => (scroll + 1).min(last),
        KeyCode::Up | KeyCode::Char('k') => scroll.saturating_sub(1),
        _ => scroll,
    });
    state.doc_scroll = u16::try_from(target).unwrap_or(u16::MAX);
    Vec::new()
}

/// Deletes the selected document from the current container.
fn delete_selected(state: &mut AppState) -> Vec<Effect> {
    if !state.marked.is_empty() {
        return delete_marked(state);
    }
    let (Some(target), Some(doc)) = (state.target.clone(), state.selected_document()) else {
        return Vec::new();
    };
    let id = display_value(doc.get("id"));
    let partition_key = value_at_path(doc, &state.pk_path).cloned();
    state.status = Status::Info(format!("Deleting {id}…"));
    vec![Effect::Delete {
        target,
        id,
        partition_key,
    }]
}

fn delete_marked(state: &mut AppState) -> Vec<Effect> {
    let Some(target) = state.target.clone() else {
        return Vec::new();
    };
    let items: Vec<(String, Option<Value>)> = state
        .marked
        .iter()
        .filter_map(|&index| state.results.get(index))
        .map(|doc| {
            (
                display_value(doc.get("id")),
                value_at_path(doc, &state.pk_path).cloned(),
            )
        })
        .collect();
    state.status = Status::Info(format!("Deleting {}…", documents(items.len())));
    vec![Effect::DeleteMany { target, items }]
}

/// Copies the picked text, or the whole document when nothing is picked.
fn copy_document(state: &AppState) -> Vec<Effect> {
    let text = state.selected_text().or_else(|| {
        let doc = state.selected_document()?;
        serde_json::to_string_pretty(doc).ok()
    });
    text.map(Effect::Copy).into_iter().collect()
}

fn on_search_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Char(c) => state.search.insert(c),
        KeyCode::Backspace => state.search.backspace(),
        KeyCode::Delete => state.search.delete(),
        KeyCode::Left => state.search.left(),
        KeyCode::Right => state.search.right(),
        KeyCode::Home => state.search.home(),
        KeyCode::End => state.search.end(),
        KeyCode::Esc => state.focus = Focus::Results,
        KeyCode::Enter => {
            state.focus = Focus::Results;
            return run_query(state, build_query(state.search.text()), Origin::Browse);
        }
        _ => {}
    }
    Vec::new()
}

fn on_editor_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    if is_run_key(key) {
        return run_editor_query(state);
    }
    if key.code == KeyCode::Char('s') && key.modifiers.contains(KeyModifiers::CONTROL) {
        ask_where_to_save(state, Saving::Query);
        return Vec::new();
    }
    if key.code == KeyCode::Esc {
        state.focus = Focus::Output;
    }
    edit(&mut state.editor, key, state.editor_height);
    Vec::new()
}

/// Edits text with a key, moving a page at a time by this many lines.
fn edit(editor: &mut Editor, key: KeyEvent, page: u16) {
    let page = usize::from(page).max(1);
    match key.code {
        KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => editor.insert(c),
        KeyCode::Enter => editor.newline(),
        KeyCode::Backspace => editor.backspace(),
        KeyCode::Delete => editor.delete(),
        KeyCode::Left => editor.left(),
        KeyCode::Right => editor.right(),
        KeyCode::Up => editor.up(),
        KeyCode::Down => editor.down(),
        KeyCode::Home => editor.home(),
        KeyCode::End => editor.end(),
        KeyCode::PageUp => editor.page_up(page),
        KeyCode::PageDown => editor.page_down(page),
        _ => {}
    }
}

/// Edits the JSON of the indexing policy or computed properties tab.
fn on_settings_editor_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let page = state.settings_height;
    let focus = state.focus;
    let Some(settings) = settings_mut(state) else {
        return Vec::new();
    };
    let editor = match focus {
        Focus::IndexingPolicy => &mut settings.indexing,
        _ => &mut settings.computed,
    };
    edit(editor, key, page);
    Vec::new()
}

/// Opens the dialog asking where to save, suggesting a file named after the container.
fn ask_where_to_save(state: &mut AppState, saving: Saving) {
    let name = state
        .target
        .as_ref()
        .map_or("query", |target| target.container.as_str());
    let extension = match saving {
        Saving::Query => "sql",
        Saving::Results => "json",
    };
    let mut path = TextInput::default();
    format!("{name}.{extension}")
        .chars()
        .for_each(|c| path.insert(c));
    state.prompt = Some(SavePrompt { saving, path });
}

/// Edits the path in the save dialog, then saves with Enter or cancels with Esc.
fn on_prompt_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let Some(prompt) = &mut state.prompt else {
        return Vec::new();
    };
    let path = &mut prompt.path;
    match key.code {
        KeyCode::Char(c) => path.insert(c),
        KeyCode::Backspace => path.backspace(),
        KeyCode::Delete => path.delete(),
        KeyCode::Left => path.left(),
        KeyCode::Right => path.right(),
        KeyCode::Home => path.home(),
        KeyCode::End => path.end(),
        KeyCode::Esc => state.prompt = None,
        KeyCode::Enter => {
            let path = path.text().to_string();
            let saving = prompt.saving;
            state.prompt = None;
            let text = match saving {
                Saving::Query => state.editor.text(),
                Saving::Results => {
                    serde_json::to_string_pretty(&state.output.results).unwrap_or_default()
                }
            };
            let contents = format!("{text}\n");
            return vec![Effect::Save { path, contents }];
        }
        _ => {}
    }
    Vec::new()
}

/// Whether the key runs the query editor's query: F5, Ctrl-R, or Shift-Enter
/// in terminals that tell it apart from Enter. Ctrl-Enter and Alt-Enter run it
/// too, as some terminals report Shift-Enter, such as Warp on Windows, or
/// terminals set up to send Esc and Enter for it.
fn is_run_key(key: KeyEvent) -> bool {
    let modified = KeyModifiers::SHIFT | KeyModifiers::CONTROL | KeyModifiers::ALT;
    match key.code {
        KeyCode::F(5) => true,
        KeyCode::Char('r') => key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Enter => key.modifiers.intersects(modified),
        _ => false,
    }
}

/// Runs the query in the editor as it is written.
fn run_editor_query(state: &mut AppState) -> Vec<Effect> {
    let sql = state.editor.text().trim().to_string();
    run_query(state, sql, Origin::Editor)
}

fn on_tree_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let selected = state.tree_selected;
    let last = state.tree_rows().len().saturating_sub(1);
    let page = usize::from(state.tree_height).max(1);
    if let Some(target) = jump(key, &mut state.pending_g, selected, last, page) {
        state.tree_selected = target;
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            state.tree_selected = (state.tree_selected + 1).min(last);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.tree_selected = state.tree_selected.saturating_sub(1);
        }
        KeyCode::Enter => return toggle(state),
        KeyCode::Right | KeyCode::Char('l') => return expand(state),
        KeyCode::Left | KeyCode::Char('h') => collapse(state),
        _ => {}
    }
    if state.tree_selected != selected {
        return open_selected_container(state);
    }
    Vec::new()
}

/// Lists the documents of the selected tree row, when it is a container.
fn open_selected_container(state: &mut AppState) -> Vec<Effect> {
    match state.selected_node() {
        Some(Node::Container(a, d, c)) => open_container(state, a, d, c),
        _ => Vec::new(),
    }
}

/// Opens a closed tree node, or closes an open one.
/// A container's documents are listed already, so it moves on to them.
fn toggle(state: &mut AppState) -> Vec<Effect> {
    let expanded = match state.selected_node() {
        Some(Node::Account(a)) => state.account_mut(a).is_some_and(|node| node.expanded),
        Some(Node::Database(a, d)) => state.database_mut(a, d).is_some_and(|node| node.expanded),
        Some(Node::Container(..)) => {
            state.focus = state.mode.panes()[1];
            return Vec::new();
        }
        _ => false,
    };
    if expanded {
        collapse(state);
        Vec::new()
    } else {
        expand(state)
    }
}

/// Makes a container the one queries run against, and lists its documents
/// unless the query editor is open.
fn open_container(state: &mut AppState, a: usize, d: usize, c: usize) -> Vec<Effect> {
    let Some(account) = state.account_mut(a).map(|node| node.account.clone()) else {
        return Vec::new();
    };
    let Some(database) = state.database_mut(a, d) else {
        return Vec::new();
    };
    let Some(container) = database.containers.get(c) else {
        return Vec::new();
    };
    let target = Target {
        account,
        database: database.name.clone(),
        container: container.name.clone(),
    };
    if state.mode == Mode::Settings && settings_changed(state) {
        state.status = Status::Error(
            "save (Ctrl-S) or discard (Esc) the changes to the settings first".into(),
        );
        return Vec::new();
    }
    state.target = Some(target.clone());
    state.results.clear();
    state.marked.clear();
    state.result_selected = 0;
    if state.mode == Mode::Query {
        // The query editor's query runs on it when it is run next,
        // and its documents are listed when the query editor is left
        state.browse_stale = true;
        return Vec::new();
    }
    if state.mode == Mode::Settings {
        // Its documents are listed when the settings are left
        state.browse_stale = true;
        state.settings = Some(Load::Loading);
        return vec![Effect::LoadSettings(target)];
    }
    run_query(state, DEFAULT_QUERY.to_string(), Origin::Browse)
}

/// Queries the current container, if there is one.
fn run_query(state: &mut AppState, sql: String, origin: Origin) -> Vec<Effect> {
    let Some(target) = state.target.clone() else {
        state.status = Status::Error("pick a container first".into());
        return Vec::new();
    };
    state.queries += 1;
    let id = state.queries;
    match origin {
        Origin::Browse => {
            state.query_id = id;
            state.more = false;
            state.loading_more = false;
            state.last_sql.clone_from(&sql);
        }
        Origin::Editor => {
            state.output.id = id;
            state.output.more = false;
            state.output.loading_more = false;
        }
    }
    state.status = Status::Info(format!(
        "Querying {}/{}…",
        target.database, target.container
    ));
    vec![Effect::Query {
        id,
        target,
        sql,
        origin,
    }]
}

/// Runs the latest query again for documents an earlier run saved, which stay
/// shown, with whether there are more, until the query finds them afresh.
pub fn refresh_documents(state: &mut AppState) -> Vec<Effect> {
    let more = state.more;
    let effects = run_query(state, state.last_sql.clone(), Origin::Browse);
    state.more = more;
    if !effects.is_empty() {
        state.refreshing_query = Some(state.query_id);
    }
    effects
}

/// Opens the selected tree node, loading an account's containers the first time.
fn expand(state: &mut AppState) -> Vec<Effect> {
    match state.selected_node() {
        Some(Node::Account(a)) => {
            let Some(node) = state.account_mut(a) else {
                return Vec::new();
            };
            node.expanded = true;
            if node.databases.is_some() {
                return Vec::new();
            }
            node.databases = Some(Load::Loading);
            vec![Effect::LoadContainers(node.account.clone())]
        }
        Some(Node::Database(a, d)) => {
            if let Some(node) = state.database_mut(a, d) {
                node.expanded = true;
            }
            Vec::new()
        }
        _ => Vec::new(),
    }
}

/// Closes the selected tree node, or moves to its parent when it is closed already.
fn collapse(state: &mut AppState) {
    let parent = match state.selected_node() {
        Some(Node::Account(a)) => {
            if let Some(node) = state.account_mut(a) {
                node.expanded = false;
            }
            None
        }
        Some(Node::Database(a, d)) => match state.database_mut(a, d) {
            Some(node) if node.expanded => {
                node.expanded = false;
                None
            }
            _ => Some(Node::Account(a)),
        },
        Some(Node::Container(a, d, _)) => Some(Node::Database(a, d)),
        Some(Node::Note(a)) => Some(Node::Account(a)),
        Some(Node::AccountsNote) | None => None,
    };
    if let Some(parent) = parent {
        state.select_node(parent);
    }
}

/// Puts an account's containers under it, grouped by database in listing order.
/// Databases listed before, such as by an earlier run, stay open or closed, and
/// the selection stays where it was.
fn containers_loaded(
    state: &mut AppState,
    account: &str,
    result: Result<Vec<(String, Container)>, String>,
) {
    let selected = state.path_of(state.selected_node());
    let Load::Loaded(accounts) = &mut state.accounts else {
        return;
    };
    let Some(node) = accounts
        .iter_mut()
        .find(|node| node.account.name == account)
    else {
        return;
    };
    let containers = match result {
        Ok(containers) => containers,
        Err(error) => {
            node.databases = Some(Load::Failed(error.clone()));
            state.status = Status::Error(error);
            return;
        }
    };
    let closed: Vec<String> = match &node.databases {
        Some(Load::Loaded(databases)) => databases
            .iter()
            .filter(|database| !database.expanded)
            .map(|database| database.name.clone())
            .collect(),
        _ => Vec::new(),
    };
    let mut databases: Vec<DatabaseNode> = Vec::new();
    for (database, container) in containers {
        match databases.iter_mut().find(|d| d.name == database) {
            Some(node) => node.containers.push(container),
            None => databases.push(DatabaseNode {
                expanded: !closed.contains(&database),
                name: database,
                containers: vec![container],
            }),
        }
    }
    node.databases = Some(Load::Loaded(databases));
    state.reselect(selected);
}

/// Shows the documents a query found.
fn query_done(state: &mut AppState, id: u64, result: Result<QueryResult, String>) {
    if id == state.output.id {
        return output_done(state, result);
    }
    if id != state.query_id {
        return;
    }
    if state
        .refreshing_query
        .take_if(|refreshing| *refreshing == id)
        .is_some()
    {
        match result {
            Ok(result) => refreshed(state, result),
            Err(error) => {
                // The saved documents are still worth showing, without their open query
                state.more = false;
                state.status = Status::Error(format!("could not refresh documents: {error}"));
            }
        }
        return;
    }
    match result {
        Ok(result) => {
            state.stats = result.stats;
            state.status = Status::Info(found(result.docs.len(), &state.stats, result.elapsed));
            state.results = result.docs;
            state.marked.clear();
            state.more = result.more;
            state.loading_more = false;
            state.result_selected = 0;
            show_from_top(state);
            state.pk_path = result.pk_path;
        }
        Err(error) => {
            state.status = Status::Error("the query failed".into());
            state.error = Some(error);
        }
    }
}

/// Shows documents found afresh in place of those an earlier run saved, keeping
/// the selected and marked ones by id and partition key, and the document scrolled.
fn refreshed(state: &mut AppState, result: QueryResult) {
    let pk_path = result.pk_path.clone();
    let key = |doc: &Value| {
        (
            doc.get("id").cloned(),
            value_at_path(doc, &pk_path).cloned(),
        )
    };
    let selected = state.selected_document().map(key);
    let marked: Vec<_> = state
        .marked
        .iter()
        .filter_map(|&index| state.results.get(index))
        .map(key)
        .collect();
    let keys: Vec<_> = result.docs.iter().map(key).collect();
    match keys
        .iter()
        .position(|found| Some(found) == selected.as_ref())
    {
        Some(index) => state.result_selected = index,
        None => {
            state.result_selected = state.result_selected.min(keys.len().saturating_sub(1));
            show_from_top(state);
        }
    }
    state.marked = (0..keys.len())
        .filter(|&index| marked.contains(&keys[index]))
        .collect();
    state.stats = result.stats;
    state.status = Status::Info(found(result.docs.len(), &state.stats, result.elapsed));
    state.results = result.docs;
    state.more = result.more;
    state.loading_more = false;
    state.pk_path = result.pk_path;
}

/// Shows the documents the query editor's query found in its output, from the top.
fn output_done(state: &mut AppState, result: Result<QueryResult, String>) {
    let output = &mut state.output;
    match result {
        Ok(result) => {
            output.stats = result.stats;
            output.results = result.docs;
            output.more = result.more;
            output.loading_more = false;
            state.output_scroll = 0;
            state.status = Status::Info(found(output.results.len(), &output.stats, result.elapsed));
        }
        Err(error) => {
            state.status = Status::Error("the query failed".into());
            state.error = Some(error);
        }
    }
}

/// Adds the next page of the query editor's query to its output.
fn more_output_loaded(state: &mut AppState, result: Result<QueryResult, String>) {
    let output = &mut state.output;
    output.loading_more = false;
    match result {
        Ok(result) => {
            output.results.extend(result.docs);
            output.more = result.more;
            output.stats.add(result.stats);
            state.status = Status::Info(found(output.results.len(), &output.stats, result.elapsed));
        }
        Err(error) => {
            // The open query is gone, so only running it again gets the rest
            output.more = false;
            state.status = Status::Error(format!("{error}, press F5 to run the query again"));
        }
    }
}

/// Adds the next page of a query to the results, selecting its first document.
fn more_loaded(state: &mut AppState, id: u64, result: Result<QueryResult, String>) {
    if id == state.output.id {
        return more_output_loaded(state, result);
    }
    if id != state.query_id {
        return;
    }
    state.loading_more = false;
    match result {
        Ok(result) => {
            if !result.docs.is_empty() {
                state.result_selected = state.results.len();
                show_from_top(state);
            }
            state.results.extend(result.docs);
            state.more = result.more;
            state.stats.add(result.stats);
            state.status = Status::Info(found(state.results.len(), &state.stats, result.elapsed));
        }
        Err(error) => {
            // The open query is gone, so only running it again gets the rest
            state.more = false;
            state.status = Status::Error(format!("{error}, press r to run the query again"));
        }
    }
}

/// Shows the listed accounts. Accounts already shown, such as from the cache,
/// keep their open databases and containers, and the selection stays where it was.
fn accounts_loaded(state: &mut AppState, result: Result<Vec<Account>, String>) {
    let selected = state.path_of(state.selected_node());
    let shown = match std::mem::replace(&mut state.accounts, Load::Loading) {
        Load::Loaded(nodes) => nodes,
        Load::Loading | Load::Failed(_) => Vec::new(),
    };
    let accounts = match result {
        Ok(accounts) => accounts,
        Err(error) if shown.is_empty() => {
            state.accounts = Load::Loaded(shown);
            state.status = Status::Error(error);
            return;
        }
        Err(error) => {
            state.accounts = Load::Loaded(shown);
            state.status = Status::Error(format!("could not refresh accounts: {error}"));
            return;
        }
    };
    let mut shown: Vec<Option<AccountNode>> = shown.into_iter().map(Some).collect();
    let nodes = accounts
        .into_iter()
        .map(|account| {
            let earlier = shown
                .iter_mut()
                .find(|node| {
                    node.as_ref()
                        .is_some_and(|n| n.account.name == account.name)
                })
                .and_then(Option::take);
            match earlier {
                Some(node) => AccountNode { account, ..node },
                None => AccountNode::closed(account),
            }
        })
        .collect();
    state.accounts = Load::Loaded(nodes);
    // With a container picked already, the status is about its documents
    if state.target.is_none() {
        state.status = Status::Info("Pick a container".into());
    }
    state.reselect(selected);
}

/// Drops a deleted document from the results, or says why it was not deleted.
fn deleted(state: &mut AppState, id: &str, result: Result<(), String>) {
    if let Err(error) = result {
        state.status = Status::Error(format!("could not delete {id}: {error}"));
        return;
    }
    let position = state
        .results
        .iter()
        .position(|doc| display_value(doc.get("id")) == id);
    if let Some(position) = position {
        state.results.remove(position);
        let last = state.results.len().saturating_sub(1);
        state.result_selected = state.result_selected.min(last);
        show_from_top(state);
    }
    state.status = Status::Info(format!("Deleted {id}"));
}

fn deleted_many(state: &mut AppState, results: Vec<(String, Result<(), String>)>) {
    let mut gone = 0;
    let mut failed = 0;
    let mut first_error = None;
    for (id, result) in results {
        match result {
            Ok(()) => {
                let position = state
                    .results
                    .iter()
                    .position(|doc| display_value(doc.get("id")) == id);
                if let Some(position) = position {
                    state.results.remove(position);
                }
                gone += 1;
            }
            Err(error) => {
                failed += 1;
                first_error.get_or_insert(format!("could not delete {id}: {error}"));
            }
        }
    }
    state.marked.clear();
    let last = state.results.len().saturating_sub(1);
    state.result_selected = state.result_selected.min(last);
    show_from_top(state);
    state.status = match first_error {
        None => Status::Info(format!("Deleted {}", documents(gone))),
        Some(error) => Status::Error(format!(
            "deleted {}, {failed} failed: {error}",
            documents(gone)
        )),
    };
}

/// Counts documents in words, such as "1 document" or "3 documents".
/// How many documents a query found, what reading them cost, when known, and how long it took.
/// Whether settings that finished loading or saving are for the settings shown.
fn shows_settings_of(state: &AppState, target: &Target) -> bool {
    state.mode == Mode::Settings && state.target.as_ref() == Some(target)
}

fn settings_loaded(state: &mut AppState, target: &Target, result: Result<Value, String>) {
    if !shows_settings_of(state, target) {
        return;
    }
    state.settings = Some(match result {
        Ok(properties) => Load::Loaded(ContainerSettings::from_properties(&properties)),
        Err(error) => Load::Failed(error),
    });
}

fn settings_saved(state: &mut AppState, target: &Target, result: Result<Value, String>) {
    if !shows_settings_of(state, target) {
        return;
    }
    match result {
        Ok(properties) => {
            let settings = ContainerSettings::from_properties(&properties);
            state.settings = Some(Load::Loaded(settings));
            state.status = Status::Info(format!(
                "Saved the settings of {}/{}",
                target.database, target.container
            ));
        }
        Err(error) => {
            state.error = Some(error);
            state.status = Status::Error("could not save the settings".into());
        }
    }
}

fn found(count: usize, stats: &QueryStats, elapsed: Duration) -> String {
    let cost = if stats.round_trips > 0 {
        format!(" · {:.2} RU", stats.request_charge)
    } else {
        String::new()
    };
    format!(
        "{}{cost} in {:.2}s",
        documents(count),
        elapsed.as_secs_f64()
    )
}

fn documents(count: usize) -> String {
    match count {
        1 => "1 document".to_string(),
        n => format!("{n} documents"),
    }
}

fn on_msg(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    match msg {
        Msg::AccountsLoaded(result) => accounts_loaded(state, result),
        Msg::ContainersLoaded { account, result } => containers_loaded(state, &account, result),
        Msg::QueryDone { id, result } => query_done(state, id, result),
        Msg::MoreLoaded { id, result } => more_loaded(state, id, result),
        Msg::Deleted { id, result } => deleted(state, &id, result),
        Msg::DeletedMany { results } => deleted_many(state, results),
        Msg::Copied(Ok(())) => state.status = Status::Info("Copied to the clipboard".into()),
        Msg::Copied(Err(error)) => {
            state.status = Status::Error(format!("could not copy: {error}"));
        }
        Msg::Saved(Ok(path)) => state.status = Status::Info(format!("Saved {path}")),
        Msg::Saved(Err(error)) => {
            state.status = Status::Error(format!("could not save: {error}"));
        }
        Msg::SettingsLoaded { target, result } => settings_loaded(state, &target, result),
        Msg::SettingsSaved { target, result } => settings_saved(state, &target, result),
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{Field, Geospatial, SettingsTab, TimeToLive};
    use crate::testing::{account, container};
    use serde_json::{Value, json};
    use std::time::Duration;

    /// The tree as indented text, one line per row.
    fn outline(state: &AppState) -> Vec<String> {
        state
            .tree_rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.label))
            .collect()
    }

    #[test]
    fn starts_by_loading_accounts() {
        let (state, effects) = AppState::new();

        assert_eq!(effects, vec![Effect::LoadAccounts]);
        assert_eq!(outline(&state), vec!["loading accounts…"]);
    }

    #[test]
    fn shows_the_loaded_accounts() {
        let (mut state, _) = AppState::new();

        update(
            &mut state,
            Event::Msg(Msg::AccountsLoaded(Ok(vec![
                account("orders"),
                account("inventory"),
            ]))),
        );

        assert_eq!(outline(&state), vec!["orders", "inventory"]);
    }

    #[test]
    fn reports_accounts_that_failed_to_load() {
        let (mut state, _) = AppState::new();

        update(
            &mut state,
            Event::Msg(Msg::AccountsLoaded(Err("az login first".into()))),
        );

        assert_eq!(state.status, Status::Error("az login first".into()));
        assert!(outline(&state).is_empty());
    }

    fn press(state: &mut AppState, code: KeyCode) -> Vec<Effect> {
        update(state, Event::Key(KeyEvent::from(code)))
    }

    #[test]
    fn q_and_ctrl_c_quit() {
        let (mut state, _) = AppState::new();
        press(&mut state, KeyCode::Char('q'));
        assert!(state.quit);

        let (mut state, _) = AppState::new();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        update(&mut state, Event::Key(ctrl_c));
        assert!(state.quit);
    }

    fn ctrl_b(state: &mut AppState) -> Vec<Effect> {
        let key = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);
        update(state, Event::Key(key))
    }

    #[test]
    fn ctrl_b_hides_the_tree_and_shows_it_again() {
        let (mut state, _) = AppState::new();
        assert!(!state.tree_hidden);

        ctrl_b(&mut state);
        assert!(state.tree_hidden);

        ctrl_b(&mut state);
        assert!(!state.tree_hidden);
    }

    #[test]
    fn hiding_the_focused_tree_focuses_the_search_bar() {
        let (mut state, _) = AppState::new();
        assert_eq!(state.focus, Focus::Tree);

        ctrl_b(&mut state);
        assert_eq!(state.focus, Focus::Search);
    }

    #[test]
    fn hiding_the_tree_keeps_the_focus_on_another_pane() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Results;

        ctrl_b(&mut state);
        assert_eq!(state.focus, Focus::Results);
    }

    #[test]
    fn tab_and_shift_tab_skip_the_hidden_tree() {
        let (mut state, _) = AppState::new();
        ctrl_b(&mut state);
        state.focus = Focus::Document;

        press(&mut state, KeyCode::Tab);
        assert_eq!(state.focus, Focus::Results);

        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.focus, Focus::Document);
    }

    #[test]
    fn z_zooms_the_focused_pane_and_shows_every_pane_again() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Results;
        assert_eq!(state.zoomed_pane(), None);

        press(&mut state, KeyCode::Char('z'));
        assert_eq!(state.zoomed_pane(), Some(Focus::Results));

        press(&mut state, KeyCode::Char('z'));
        assert_eq!(state.zoomed_pane(), None);
    }

    #[test]
    fn the_zoom_moves_with_the_focus_but_never_to_the_search_bar() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Results;
        press(&mut state, KeyCode::Char('z'));

        press(&mut state, KeyCode::Tab);
        assert_eq!(state.zoomed_pane(), Some(Focus::Document));

        press(&mut state, KeyCode::Char('/'));
        assert_eq!(state.zoomed_pane(), None);

        press(&mut state, KeyCode::Char('z'));
        assert_eq!(state.search.text(), "z");

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.zoomed_pane(), Some(Focus::Results));
    }

    fn with_accounts(names: &[&str]) -> AppState {
        let (mut state, _) = AppState::new();
        let accounts = names.iter().map(|name| account(name)).collect();
        update(&mut state, Event::Msg(Msg::AccountsLoaded(Ok(accounts))));
        state
    }

    #[test]
    fn arrow_and_vim_keys_move_through_the_tree_within_its_rows() {
        let mut state = with_accounts(&["orders", "inventory"]);
        assert_eq!(state.tree_selected, 0);

        press(&mut state, KeyCode::Down);
        assert_eq!(state.tree_selected, 1);
        press(&mut state, KeyCode::Char('j'));
        assert_eq!(state.tree_selected, 1);
        press(&mut state, KeyCode::Char('k'));
        assert_eq!(state.tree_selected, 0);
        press(&mut state, KeyCode::Up);
        assert_eq!(state.tree_selected, 0);
    }

    #[test]
    fn home_and_end_select_the_first_and_last_tree_row() {
        let mut state = with_accounts(&["orders", "inventory", "billing"]);

        press(&mut state, KeyCode::End);
        assert_eq!(state.tree_selected, 2);
        press(&mut state, KeyCode::Home);
        assert_eq!(state.tree_selected, 0);
    }

    #[test]
    fn gg_selects_the_first_tree_row_and_shift_g_the_last() {
        let mut state = with_accounts(&["orders", "inventory", "billing"]);

        press(&mut state, KeyCode::Char('G'));
        assert_eq!(state.tree_selected, 2);
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.tree_selected, 2);
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.tree_selected, 0);
    }

    #[test]
    fn page_down_and_page_up_move_the_tree_selection_a_page() {
        let names: Vec<String> = (0..25).map(|i| format!("account{i:02}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut state = with_accounts(&names);
        state.tree_height = 10;

        press(&mut state, KeyCode::PageDown);
        assert_eq!(state.tree_selected, 10);
        press(&mut state, KeyCode::PageDown);
        press(&mut state, KeyCode::PageDown);
        assert_eq!(state.tree_selected, 24);
        press(&mut state, KeyCode::PageUp);
        assert_eq!(state.tree_selected, 14);
        press(&mut state, KeyCode::PageUp);
        press(&mut state, KeyCode::PageUp);
        assert_eq!(state.tree_selected, 0);
    }

    #[test]
    fn ctrl_d_and_ctrl_u_move_the_tree_selection_half_a_page() {
        let names: Vec<String> = (0..12).map(|i| format!("account{i:02}")).collect();
        let names: Vec<&str> = names.iter().map(String::as_str).collect();
        let mut state = with_accounts(&names);
        state.tree_height = 10;

        press_ctrl(&mut state, 'd');
        assert_eq!(state.tree_selected, 5);
        press_ctrl(&mut state, 'd');
        assert_eq!(state.tree_selected, 10);
        press_ctrl(&mut state, 'd');
        assert_eq!(state.tree_selected, 11);
        press_ctrl(&mut state, 'u');
        assert_eq!(state.tree_selected, 6);
        press_ctrl(&mut state, 'u');
        press_ctrl(&mut state, 'u');
        assert_eq!(state.tree_selected, 0);
    }

    #[test]
    fn expanding_an_account_loads_its_containers() {
        let mut state = with_accounts(&["orders", "inventory"]);

        let effects = press(&mut state, KeyCode::Enter);

        assert_eq!(effects, vec![Effect::LoadContainers(account("orders"))]);
        assert_eq!(outline(&state), vec!["orders", "  loading…", "inventory"]);
    }

    fn shop_containers() -> Vec<(String, Container)> {
        vec![
            ("shop".into(), container("carts", "/tenantId")),
            ("shop".into(), container("orders", "/tenantId")),
            ("audit".into(), container("events", "/day")),
        ]
    }

    fn containers_loaded(
        state: &mut AppState,
        name: &str,
        result: Result<Vec<(String, Container)>, String>,
    ) {
        update(
            state,
            Event::Msg(Msg::ContainersLoaded {
                account: name.into(),
                result,
            }),
        );
    }

    #[test]
    fn shows_the_containers_of_an_expanded_account_by_database() {
        let mut state = with_accounts(&["orders", "inventory"]);
        press(&mut state, KeyCode::Enter);

        containers_loaded(&mut state, "orders", Ok(shop_containers()));

        assert_eq!(
            outline(&state),
            vec![
                "orders",
                "  shop",
                "    carts",
                "    orders",
                "  audit",
                "    events",
                "inventory"
            ]
        );
    }

    #[test]
    fn shows_why_containers_failed_to_load_under_the_account() {
        let mut state = with_accounts(&["orders"]);
        press(&mut state, KeyCode::Enter);

        containers_loaded(&mut state, "orders", Err("forbidden".into()));

        assert_eq!(outline(&state), vec!["orders", "  error: forbidden"]);
        assert_eq!(state.status, Status::Error("forbidden".into()));
    }

    fn with_orders_expanded() -> AppState {
        let mut state = with_accounts(&["orders", "inventory"]);
        press(&mut state, KeyCode::Enter);
        containers_loaded(&mut state, "orders", Ok(shop_containers()));
        state
    }

    fn selected_label(state: &AppState) -> String {
        state.tree_rows()[state.tree_selected].label.clone()
    }

    #[test]
    fn left_goes_to_the_parent_then_collapses_it() {
        let mut state = with_orders_expanded();
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);
        assert_eq!(selected_label(&state), "carts");

        press(&mut state, KeyCode::Left);
        assert_eq!(selected_label(&state), "shop");

        press(&mut state, KeyCode::Char('h'));
        assert_eq!(
            outline(&state),
            vec!["orders", "  shop", "  audit", "    events", "inventory"]
        );

        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Left);
        assert_eq!(outline(&state), vec!["orders", "inventory"]);
        assert_eq!(selected_label(&state), "orders");
    }

    #[test]
    fn selecting_an_account_or_database_runs_no_query() {
        let mut state = with_orders_expanded();

        let effects = press(&mut state, KeyCode::Down);

        assert_eq!(selected_label(&state), "shop");
        assert!(effects.is_empty());
    }

    #[test]
    fn enter_toggles_a_loaded_account_without_reloading_it() {
        let mut state = with_orders_expanded();

        let effects = press(&mut state, KeyCode::Enter);
        assert_eq!(outline(&state), vec!["orders", "inventory"]);

        let effects = [effects, press(&mut state, KeyCode::Enter)].concat();
        assert!(effects.is_empty());
        assert_eq!(outline(&state).len(), 7);
    }

    #[test]
    fn enter_on_a_container_moves_to_its_documents_without_querying_again() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);

        let effects = press(&mut state, KeyCode::Enter);

        assert!(effects.is_empty());
        assert_eq!(state.focus, Focus::Results);
    }

    fn carts() -> Target {
        Target {
            account: account("orders"),
            database: "shop".into(),
            container: "carts".into(),
        }
    }

    /// Selects orders/shop/carts, returning the work that starts.
    fn open_carts(state: &mut AppState) -> Vec<Effect> {
        press(state, KeyCode::Down);
        press(state, KeyCode::Down)
    }

    #[test]
    fn selecting_a_container_queries_every_document() {
        let mut state = with_orders_expanded();

        let effects = open_carts(&mut state);

        assert_eq!(
            effects,
            vec![Effect::Query {
                id: 1,
                target: carts(),
                sql: "SELECT * FROM c".into(),
                origin: Origin::Browse,
            }]
        );
        assert_eq!(state.target, Some(carts()));
    }

    fn cart_docs() -> Vec<Value> {
        vec![
            json!({ "id": "c-1", "tenantId": "contoso" }),
            json!({ "id": "c-2", "tenantId": "fabrikam" }),
        ]
    }

    fn query_done(state: &mut AppState, id: u64, result: Result<Vec<Value>, String>) {
        let result = result.map(|docs| QueryResult {
            docs,
            more: false,
            pk_path: "/tenantId".into(),
            elapsed: Duration::from_millis(250),
            stats: QueryStats::default(),
        });
        update(state, Event::Msg(Msg::QueryDone { id, result }));
    }

    #[test]
    fn shows_the_documents_a_query_found() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);

        query_done(&mut state, 1, Ok(cart_docs()));

        assert_eq!(state.results, cart_docs());
        assert_eq!(state.pk_path, "/tenantId");
        assert_eq!(state.status, Status::Info("2 documents in 0.25s".into()));
    }

    #[test]
    fn ignores_results_of_a_query_that_was_superseded() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Down);

        query_done(&mut state, 1, Ok(cart_docs()));

        assert!(state.results.is_empty());
        assert_eq!(state.status, Status::Info("Querying shop/carts…".into()));
    }

    #[test]
    fn reports_a_failed_query_in_a_dialog() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);

        query_done(&mut state, 1, Err("syntax error".into()));

        assert_eq!(state.error.as_deref(), Some("syntax error"));
        assert_eq!(state.status, Status::Error("the query failed".into()));
    }

    #[test]
    fn any_key_closes_the_error_dialog_and_does_nothing_else() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        query_done(&mut state, 1, Err("syntax error".into()));

        let effects = press(&mut state, KeyCode::Char('q'));

        assert!(effects.is_empty());
        assert!(state.error.is_none());
        assert!(!state.quit);
    }

    #[test]
    fn tab_and_shift_tab_cycle_through_the_panes_but_the_search_bar() {
        let (mut state, _) = AppState::new();
        assert_eq!(state.focus, Focus::Tree);

        let order: Vec<Focus> = (0..3)
            .map(|_| {
                press(&mut state, KeyCode::Tab);
                state.focus
            })
            .collect();
        assert_eq!(order, vec![Focus::Results, Focus::Document, Focus::Tree]);

        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.focus, Focus::Document);
    }

    #[test]
    fn tab_and_shift_tab_leave_the_search_bar() {
        let (mut state, _) = AppState::new();

        press(&mut state, KeyCode::Char('/'));
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.focus, Focus::Results);

        press(&mut state, KeyCode::Char('/'));
        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.focus, Focus::Tree);
    }

    #[test]
    fn slash_jumps_to_the_search_bar() {
        let (mut state, _) = AppState::new();

        press(&mut state, KeyCode::Char('/'));

        assert_eq!(state.focus, Focus::Search);
    }

    #[test]
    fn tree_keys_only_act_while_the_tree_has_focus() {
        let mut state = with_accounts(&["orders", "inventory"]);
        state.focus = Focus::Results;

        let effects = press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Down);

        assert!(effects.is_empty());
        assert_eq!(state.tree_selected, 0);
    }

    fn type_text(state: &mut AppState, text: &str) {
        for c in text.chars() {
            press(state, KeyCode::Char(c));
        }
    }

    #[test]
    fn enter_in_the_search_bar_runs_the_typed_filter() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        press(&mut state, KeyCode::Char('/'));
        type_text(&mut state, "c.qty > 1");

        let effects = press(&mut state, KeyCode::Enter);

        assert_eq!(
            effects,
            vec![Effect::Query {
                id: 2,
                target: carts(),
                sql: "SELECT * FROM c WHERE c.qty > 1".into(),
                origin: Origin::Browse,
            }]
        );
        assert_eq!(state.focus, Focus::Results);
    }

    #[test]
    fn the_search_bar_edits_at_the_cursor() {
        let (mut state, _) = AppState::new();
        press(&mut state, KeyCode::Char('/'));
        type_text(&mut state, "c.q > 1");

        press(&mut state, KeyCode::Home);
        type_text(&mut state, "qx");
        press(&mut state, KeyCode::Backspace);
        press(&mut state, KeyCode::End);
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Delete);
        type_text(&mut state, "2");

        assert_eq!(state.search.text(), "qc.q > 2");
        assert_eq!(state.search.cursor(), 8);
    }

    #[test]
    fn escape_leaves_the_search_bar() {
        let (mut state, _) = AppState::new();
        press(&mut state, KeyCode::Char('/'));

        press(&mut state, KeyCode::Esc);

        assert_eq!(state.focus, Focus::Results);
    }

    #[test]
    fn searching_without_a_container_asks_for_one() {
        let mut state = with_accounts(&["orders"]);
        press(&mut state, KeyCode::Char('/'));

        let effects = press(&mut state, KeyCode::Enter);

        assert!(effects.is_empty());
        assert_eq!(state.status, Status::Error("pick a container first".into()));
    }

    #[test]
    fn n_opens_the_query_editor_with_the_last_query() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        press(&mut state, KeyCode::Char('/'));
        type_text(&mut state, "c.qty > 1");
        press(&mut state, KeyCode::Enter);

        let effects = press(&mut state, KeyCode::Char('n'));

        assert!(effects.is_empty());
        assert_eq!(state.mode, Mode::Query);
        assert_eq!(state.focus, Focus::Editor);
        assert_eq!(state.editor.text(), "SELECT * FROM c WHERE c.qty > 1");
    }

    #[test]
    fn n_without_a_container_asks_for_one() {
        let mut state = with_accounts(&["orders"]);

        press(&mut state, KeyCode::Char('n'));

        assert_eq!(state.mode, Mode::Browse);
        assert_eq!(state.status, Status::Error("pick a container first".into()));
    }

    /// Carts open in the query editor, holding `SELECT * FROM c`.
    fn with_query_editor() -> AppState {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        query_done(&mut state, 1, Ok(cart_docs()));
        press(&mut state, KeyCode::Char('n'));
        state
    }

    #[test]
    fn keys_in_the_query_editor_edit_the_query() {
        let mut state = with_query_editor();

        press(&mut state, KeyCode::Enter);
        type_text(&mut state, "WHERE c.q = 1x");
        press(&mut state, KeyCode::Backspace);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::End);
        press(&mut state, KeyCode::Delete);
        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Left);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::PageUp);
        press(&mut state, KeyCode::PageDown);

        assert_eq!(state.editor.text(), "SELECT * FROM cWHERE c.q = 1");
        assert!(!state.quit);
        assert_eq!(state.focus, Focus::Editor);
    }

    #[test]
    fn f5_ctrl_r_and_enter_with_a_modifier_run_the_query_as_written() {
        let run_keys = [
            KeyEvent::from(KeyCode::F(5)),
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT),
            KeyEvent::new(KeyCode::Enter, KeyModifiers::CONTROL),
        ];
        for key in run_keys {
            let mut state = with_query_editor();
            state.editor.set_text("  SELECT VALUE c.id\nFROM c\n");

            let effects = update(&mut state, Event::Key(key));

            assert_eq!(
                effects,
                vec![Effect::Query {
                    id: 2,
                    target: carts(),
                    sql: "SELECT VALUE c.id\nFROM c".into(),
                    origin: Origin::Editor,
                }],
                "{key:?}"
            );
            assert_eq!(state.editor.text(), "  SELECT VALUE c.id\nFROM c\n");
            assert_eq!(state.focus, Focus::Editor);
        }
    }

    #[test]
    fn tab_and_shift_tab_cycle_through_the_tree_editor_and_output_in_query_mode() {
        let mut state = with_query_editor();

        press(&mut state, KeyCode::Tab);
        assert_eq!(state.focus, Focus::Output);
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.focus, Focus::Tree);
        press(&mut state, KeyCode::Tab);
        assert_eq!(state.focus, Focus::Editor);
        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.focus, Focus::Tree);
    }

    #[test]
    fn escape_leaves_the_editor_for_the_output_then_the_query_mode() {
        let mut state = with_query_editor();

        press(&mut state, KeyCode::Esc);
        assert_eq!(state.focus, Focus::Output);
        assert_eq!(state.mode, Mode::Query);
        press(&mut state, KeyCode::Esc);

        assert_eq!(state.mode, Mode::Browse);
        assert_eq!(state.focus, Focus::Results);
    }

    #[test]
    fn n_in_the_output_leaves_the_query_mode_keeping_the_query() {
        let mut state = with_query_editor();
        state.editor.set_text("SELECT VALUE c.id FROM c");
        press(&mut state, KeyCode::Esc);

        press(&mut state, KeyCode::Char('n'));

        assert_eq!(state.mode, Mode::Browse);
        assert_eq!(state.editor.text(), "SELECT VALUE c.id FROM c");
    }

    #[test]
    fn hiding_the_focused_tree_in_query_mode_focuses_the_editor() {
        let mut state = with_query_editor();
        state.focus = Focus::Tree;

        ctrl_b(&mut state);

        assert_eq!(state.focus, Focus::Editor);
    }

    #[test]
    fn reopening_the_query_editor_keeps_the_query_written_in_it() {
        let mut state = with_query_editor();
        state.editor.set_text("SELECT VALUE c.id FROM c");
        press(&mut state, KeyCode::Esc);
        press(&mut state, KeyCode::Esc);

        press(&mut state, KeyCode::Char('n'));

        assert_eq!(state.editor.text(), "SELECT VALUE c.id FROM c");
    }

    #[test]
    fn selecting_a_container_in_query_mode_queries_it_next_without_running() {
        let mut state = with_query_editor();
        state.focus = Focus::Tree;

        let effects = press(&mut state, KeyCode::Down);

        assert!(effects.is_empty());
        assert_eq!(
            state.target.as_ref().map(|t| t.container.as_str()),
            Some("orders")
        );
        assert!(state.results.is_empty());
        assert_eq!(state.mode, Mode::Query);
    }

    #[test]
    fn leaving_the_query_editor_lists_the_container_switched_to_meanwhile() {
        let mut state = with_output();
        state.focus = Focus::Tree;
        press(&mut state, KeyCode::Down);
        state.focus = Focus::Output;

        let effects = press(&mut state, KeyCode::Esc);

        let [
            Effect::Query {
                target,
                sql,
                origin,
                ..
            },
        ] = &effects[..]
        else {
            panic!("unexpected {effects:?}");
        };
        assert_eq!(target.container, "orders");
        assert_eq!(sql, "SELECT * FROM c");
        assert_eq!(*origin, Origin::Browse);
    }

    #[test]
    fn leaving_the_query_editor_on_the_same_container_queries_nothing() {
        let mut state = with_output();

        assert!(press(&mut state, KeyCode::Char('n')).is_empty());
        assert_eq!(state.results, cart_docs());
    }

    #[test]
    fn enter_on_a_container_in_query_mode_moves_to_the_editor() {
        let mut state = with_query_editor();
        state.focus = Focus::Tree;

        press(&mut state, KeyCode::Enter);

        assert_eq!(state.focus, Focus::Editor);
    }

    /// The query editor's output focused, four lines high, holding the two carts
    /// its query found in ten lines.
    fn with_output() -> AppState {
        let mut state = with_query_editor();
        press(&mut state, KeyCode::F(5));
        query_done(&mut state, 2, Ok(cart_docs()));
        state.output_height = 4;
        press(&mut state, KeyCode::Esc);
        state
    }

    #[test]
    fn the_output_scrolls_no_further_than_its_last_line_at_the_bottom() {
        let mut state = with_output();

        press(&mut state, KeyCode::Down);
        assert_eq!(state.output_scroll, 1);
        press(&mut state, KeyCode::Char('G'));
        assert_eq!(state.output_scroll, 6);
        press(&mut state, KeyCode::Char('j'));
        assert_eq!(state.output_scroll, 6);
        press(&mut state, KeyCode::PageUp);
        assert_eq!(state.output_scroll, 2);
        press(&mut state, KeyCode::Char('g'));
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.output_scroll, 0);
    }

    #[test]
    fn down_at_the_bottom_of_the_output_loads_more_once() {
        let mut state = with_output();
        state.output.more = true;
        press(&mut state, KeyCode::End);

        let effects = press(&mut state, KeyCode::Down);
        assert_eq!(effects, vec![Effect::LoadMore { id: 2 }]);
        assert!(press(&mut state, KeyCode::Down).is_empty());
    }

    #[test]
    fn a_new_query_shows_its_output_from_the_top() {
        let mut state = with_output();
        press(&mut state, KeyCode::End);
        state.focus = Focus::Editor;

        press(&mut state, KeyCode::F(5));
        query_done(&mut state, 3, Ok(cart_docs()));

        assert_eq!(state.output_scroll, 0);
    }

    #[test]
    fn f5_and_r_in_the_output_run_the_editor_query_again() {
        for code in [KeyCode::F(5), KeyCode::Char('r')] {
            let mut state = with_output();
            state.editor.set_text("SELECT VALUE c.id FROM c");

            let effects = press(&mut state, code);

            assert_eq!(
                effects,
                vec![Effect::Query {
                    id: 3,
                    target: carts(),
                    sql: "SELECT VALUE c.id FROM c".into(),
                    origin: Origin::Editor,
                }]
            );
        }
    }

    fn with_cart_results() -> AppState {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        query_done(&mut state, 1, Ok(cart_docs()));
        state.focus = Focus::Results;
        state
    }

    #[test]
    fn space_marks_the_selected_result_and_moves_down() {
        let mut state = with_cart_results();

        press(&mut state, KeyCode::Char(' '));

        assert_eq!(state.marked, BTreeSet::from([0]));
        assert_eq!(state.result_selected, 1);
    }

    #[test]
    fn space_on_a_marked_result_unmarks_it() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Char(' '));
        press(&mut state, KeyCode::Up);

        press(&mut state, KeyCode::Char(' '));

        assert!(state.marked.is_empty());
    }

    #[test]
    fn ctrl_a_marks_every_result() {
        let mut state = with_cart_results();

        press_ctrl(&mut state, 'a');

        assert_eq!(state.marked, BTreeSet::from([0, 1]));
        assert_eq!(state.result_selected, 0);
    }

    #[test]
    fn escape_clears_the_marks() {
        let mut state = with_cart_results();
        press_ctrl(&mut state, 'a');

        press(&mut state, KeyCode::Esc);

        assert!(state.marked.is_empty());
    }

    #[test]
    fn new_results_clear_the_marks() {
        let mut state = with_cart_results();
        press_ctrl(&mut state, 'a');

        press(&mut state, KeyCode::Char('r'));
        query_done(&mut state, 2, Ok(cart_docs()));

        assert!(state.marked.is_empty());
    }

    #[test]
    fn moving_through_results_selects_the_document_to_show() {
        let mut state = with_cart_results();
        assert_eq!(state.selected_document(), Some(&cart_docs()[0]));

        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Char('j'));
        assert_eq!(state.selected_document(), Some(&cart_docs()[1]));

        press(&mut state, KeyCode::Up);
        assert_eq!(state.selected_document(), Some(&cart_docs()[0]));
    }

    fn with_many_results(count: usize) -> AppState {
        let mut state = with_cart_results();
        state.results = (0..count).map(|i| json!({ "id": i })).collect();
        state.results_height = 10;
        state
    }

    #[test]
    fn page_keys_move_the_selection_a_page_at_a_time() {
        let mut state = with_many_results(25);

        press(&mut state, KeyCode::PageDown);
        assert_eq!(state.result_selected, 10);
        press(&mut state, KeyCode::PageDown);
        press(&mut state, KeyCode::PageDown);
        assert_eq!(state.result_selected, 24);
        press(&mut state, KeyCode::PageUp);
        assert_eq!(state.result_selected, 14);
    }

    #[test]
    fn home_and_end_select_the_first_and_last_document() {
        let mut state = with_many_results(25);

        press(&mut state, KeyCode::End);
        assert_eq!(state.result_selected, 24);
        press(&mut state, KeyCode::Home);
        assert_eq!(state.result_selected, 0);
    }

    fn press_ctrl(state: &mut AppState, c: char) -> Vec<Effect> {
        let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL);
        update(state, Event::Key(key))
    }

    #[test]
    fn ctrl_d_and_ctrl_u_move_the_selection_half_a_page() {
        let mut state = with_many_results(25);

        press_ctrl(&mut state, 'd');
        assert_eq!(state.result_selected, 5);
        press_ctrl(&mut state, 'd');
        assert_eq!(state.result_selected, 10);
        press_ctrl(&mut state, 'u');
        assert_eq!(state.result_selected, 5);
        press_ctrl(&mut state, 'u');
        press_ctrl(&mut state, 'u');
        assert_eq!(state.result_selected, 0);
    }

    #[test]
    fn gg_selects_the_first_document_and_shift_g_the_last() {
        let mut state = with_many_results(25);

        press(&mut state, KeyCode::Char('G'));
        assert_eq!(state.result_selected, 24);
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.result_selected, 24);
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.result_selected, 0);
    }

    #[test]
    fn a_g_followed_by_another_key_does_not_jump() {
        let mut state = with_many_results(25);
        press(&mut state, KeyCode::PageDown);

        press(&mut state, KeyCode::Char('g'));
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Char('g'));

        assert_eq!(state.result_selected, 11);
    }

    #[test]
    fn jumping_to_another_document_shows_it_from_the_top() {
        let mut state = with_many_results(25);
        state.doc_scroll = 5;

        press(&mut state, KeyCode::PageDown);

        assert_eq!(state.doc_scroll, 0);
    }

    #[test]
    fn new_results_start_at_the_first_document() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Down);

        press(&mut state, KeyCode::Char('r'));
        query_done(&mut state, 2, Ok(cart_docs()));

        assert_eq!(state.result_selected, 0);
    }

    #[test]
    fn opening_another_container_clears_the_results() {
        let mut state = with_cart_results();
        state.focus = Focus::Tree;

        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);

        assert!(state.results.is_empty());
    }

    #[test]
    fn the_document_pane_scrolls_and_starts_at_the_top_for_another_document() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;
        state.results[0] = long_document(40);
        state.doc_height = 10;

        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Char('j'));
        press(&mut state, KeyCode::PageDown);
        press(&mut state, KeyCode::Up);
        assert_eq!(state.doc_scroll, 11);
        press(&mut state, KeyCode::PageUp);
        assert_eq!(state.doc_scroll, 1);
        press(&mut state, KeyCode::Home);
        assert_eq!(state.doc_scroll, 0);

        press(&mut state, KeyCode::PageDown);
        state.focus = Focus::Results;
        press(&mut state, KeyCode::Down);
        assert_eq!(state.doc_scroll, 0);
    }

    #[test]
    fn question_mark_shows_help_until_the_next_key() {
        let mut state = with_accounts(&["orders", "inventory"]);

        press(&mut state, KeyCode::Char('?'));
        assert!(state.show_help);

        press(&mut state, KeyCode::Down);
        assert!(!state.show_help);
        assert_eq!(state.tree_selected, 0);
    }

    fn page(from: usize, to: usize, more: bool) -> QueryResult {
        QueryResult {
            docs: (from..to)
                .map(|i| json!({ "id": format!("c-{i}") }))
                .collect(),
            more,
            pk_path: "/tenantId".into(),
            elapsed: Duration::from_millis(250),
            stats: QueryStats::default(),
        }
    }

    /// Carts open with the first page of a query that found more.
    fn with_first_page() -> AppState {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        update(
            &mut state,
            Event::Msg(Msg::QueryDone {
                id: 1,
                result: Ok(page(0, 3, true)),
            }),
        );
        state.focus = Focus::Results;
        state
    }

    #[test]
    fn moving_past_the_last_result_loads_more_once() {
        let mut state = with_first_page();
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);

        let effects = press(&mut state, KeyCode::Down);
        let again = press(&mut state, KeyCode::Down);

        assert_eq!(effects, vec![Effect::LoadMore { id: 1 }]);
        assert!(again.is_empty());
        assert_eq!(state.result_selected, 2);
    }

    #[test]
    fn nothing_more_loads_when_the_query_found_everything() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Down);

        assert!(press(&mut state, KeyCode::Down).is_empty());
    }

    fn more_loaded(state: &mut AppState, id: u64, result: Result<QueryResult, String>) {
        update(state, Event::Msg(Msg::MoreLoaded { id, result }));
    }

    #[test]
    fn more_documents_follow_the_results_and_the_first_of_them_is_selected() {
        let mut state = with_first_page();
        state.result_selected = 2;
        press(&mut state, KeyCode::Down);

        more_loaded(&mut state, 1, Ok(page(3, 5, false)));

        assert_eq!(state.results.len(), 5);
        assert_eq!(state.selected_document(), Some(&json!({ "id": "c-3" })));
        assert!(!state.more);
        assert!(!state.loading_more);
        assert_eq!(state.status, Status::Info("5 documents in 0.25s".into()));
    }

    #[test]
    fn more_documents_of_a_superseded_query_are_ignored() {
        let mut state = with_first_page();
        press(&mut state, KeyCode::Char('r'));

        more_loaded(&mut state, 1, Ok(page(3, 5, false)));

        assert_eq!(state.results.len(), 3);
    }

    #[test]
    fn failing_to_load_more_suggests_running_the_query_again() {
        let mut state = with_first_page();
        state.result_selected = 2;
        press(&mut state, KeyCode::Down);

        more_loaded(&mut state, 1, Err("throttled".into()));

        assert_eq!(
            state.status,
            Status::Error("throttled, press r to run the query again".into())
        );
        assert!(press(&mut state, KeyCode::Down).is_empty());
    }

    #[test]
    fn d_on_a_result_asks_to_confirm_the_delete_without_deleting() {
        let mut state = with_cart_results();

        let effects = press(&mut state, KeyCode::Char('d'));

        assert!(effects.is_empty());
        assert!(state.confirm_delete);
    }

    #[test]
    fn any_other_key_cancels_the_delete_and_does_nothing_else() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Char('d'));

        let effects = press(&mut state, KeyCode::Down);

        assert!(effects.is_empty());
        assert!(!state.confirm_delete);
        assert_eq!(state.result_selected, 0);
        assert_eq!(state.results.len(), 2);
    }

    #[test]
    fn y_confirms_the_delete_of_the_marked_documents_only() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Char(' '));
        press(&mut state, KeyCode::Up);

        press(&mut state, KeyCode::Char('d'));
        assert!(state.confirm_delete);
        let effects = press(&mut state, KeyCode::Char('y'));

        assert_eq!(
            effects,
            vec![Effect::DeleteMany {
                target: carts(),
                items: vec![("c-2".into(), Some(json!("fabrikam")))],
            }]
        );
    }

    #[test]
    fn y_confirms_the_delete_of_the_selected_document() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Char('d'));

        let effects = press(&mut state, KeyCode::Char('y'));

        assert_eq!(
            effects,
            vec![Effect::Delete {
                target: carts(),
                id: "c-2".into(),
                partition_key: Some(json!("fabrikam")),
            }]
        );
        assert!(!state.confirm_delete);
        assert_eq!(state.status, Status::Info("Deleting c-2…".into()));
    }

    fn deleted(state: &mut AppState, id: &str, result: Result<(), String>) {
        let id = id.to_string();
        update(state, Event::Msg(Msg::Deleted { id, result }));
    }

    #[test]
    fn a_deleted_document_leaves_the_results() {
        let mut state = with_cart_results();

        deleted(&mut state, "c-1", Ok(()));

        assert_eq!(state.results, vec![cart_docs()[1].clone()]);
        assert_eq!(state.selected_document(), Some(&cart_docs()[1]));
        assert_eq!(state.status, Status::Info("Deleted c-1".into()));
    }

    #[test]
    fn deleting_the_last_result_selects_the_one_before() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Down);

        deleted(&mut state, "c-2", Ok(()));

        assert_eq!(state.result_selected, 0);
        assert_eq!(state.selected_document(), Some(&cart_docs()[0]));
    }

    fn deleted_many(state: &mut AppState, results: &[(&str, Result<(), &str>)]) {
        let results = results
            .iter()
            .map(|(id, result)| {
                let result = result.map_err(String::from);
                (id.to_string(), result)
            })
            .collect();
        update(state, Event::Msg(Msg::DeletedMany { results }));
    }

    #[test]
    fn deleted_marked_documents_leave_the_results_and_the_marks() {
        let mut state = with_cart_results();
        press_ctrl(&mut state, 'a');

        deleted_many(&mut state, &[("c-1", Ok(())), ("c-2", Ok(()))]);

        assert!(state.results.is_empty());
        assert!(state.marked.is_empty());
        assert_eq!(state.status, Status::Info("Deleted 2 documents".into()));
    }

    #[test]
    fn a_partly_failed_delete_keeps_the_failed_documents_and_says_why() {
        let mut state = with_cart_results();
        press_ctrl(&mut state, 'a');

        deleted_many(&mut state, &[("c-1", Ok(())), ("c-2", Err("forbidden"))]);

        assert_eq!(state.results, cart_docs()[1..]);
        assert!(state.marked.is_empty());
        assert_eq!(state.result_selected, 0);
        assert_eq!(
            state.status,
            Status::Error("deleted 1 document, 1 failed: could not delete c-2: forbidden".into())
        );
    }

    #[test]
    fn a_failed_delete_keeps_the_document_and_says_why() {
        let mut state = with_cart_results();

        deleted(&mut state, "c-1", Err("forbidden".into()));

        assert_eq!(state.results, cart_docs());
        assert_eq!(
            state.status,
            Status::Error("could not delete c-1: forbidden".into())
        );
    }

    #[test]
    fn d_in_the_document_pane_asks_to_confirm_the_delete_too() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;

        let effects = press(&mut state, KeyCode::Char('d'));
        assert!(effects.is_empty());
        assert!(state.confirm_delete);

        let effects = press(&mut state, KeyCode::Char('y'));
        assert!(matches!(effects[..], [Effect::Delete { ref id, .. }] if id == "c-1"));
    }

    #[test]
    fn d_in_the_document_pane_without_a_document_asks_nothing() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Document;

        press(&mut state, KeyCode::Char('d'));

        assert!(!state.confirm_delete);
    }

    #[test]
    fn ctrl_d_in_the_document_pane_still_scrolls_without_asking() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;
        state.results[0] = long_document(40);
        state.doc_height = 10;

        press_ctrl(&mut state, 'd');

        assert!(!state.confirm_delete);
        assert_eq!(state.doc_scroll, 5);
    }

    #[test]
    fn d_without_results_asks_nothing() {
        let mut state = with_orders_expanded();
        state.focus = Focus::Results;

        press(&mut state, KeyCode::Char('d'));

        assert!(!state.confirm_delete);
    }

    #[test]
    fn enter_on_a_result_jumps_to_its_document() {
        let mut state = with_cart_results();

        let effects = press(&mut state, KeyCode::Enter);

        assert!(effects.is_empty());
        assert_eq!(state.focus, Focus::Document);
    }

    #[test]
    fn enter_without_results_stays_in_the_results() {
        let mut state = with_orders_expanded();
        state.focus = Focus::Results;

        press(&mut state, KeyCode::Enter);

        assert_eq!(state.focus, Focus::Results);
    }

    /// A document that pretty-prints as one line per field, plus its braces.
    fn long_document(fields: usize) -> Value {
        let fields = (0..fields).map(|i| (format!("f{i:02}"), json!(i)));
        Value::Object(fields.collect())
    }

    #[test]
    fn the_document_scrolls_no_further_than_its_last_line_at_the_bottom() {
        let mut state = with_cart_results();
        state.results[0] = long_document(10);
        state.doc_height = 5;
        state.focus = Focus::Document;

        for _ in 0..20 {
            press(&mut state, KeyCode::Down);
        }
        assert_eq!(state.doc_scroll, 7);

        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::PageDown);
        assert_eq!(state.doc_scroll, 5);
        press(&mut state, KeyCode::PageDown);
        assert_eq!(state.doc_scroll, 7);

        press(&mut state, KeyCode::Home);
        press(&mut state, KeyCode::End);
        assert_eq!(state.doc_scroll, 7);
    }

    #[test]
    fn the_document_scrolls_with_vim_motions_a_page_or_half_a_page_at_a_time() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;
        state.results[0] = long_document(40);
        state.doc_height = 10;

        press_ctrl(&mut state, 'd');
        assert_eq!(state.doc_scroll, 5);
        press_ctrl(&mut state, 'u');
        assert_eq!(state.doc_scroll, 0);
        press(&mut state, KeyCode::Char('G'));
        assert_eq!(state.doc_scroll, state.max_doc_scroll());
        press(&mut state, KeyCode::Char('g'));
        press(&mut state, KeyCode::Char('g'));
        assert_eq!(state.doc_scroll, 0);
    }

    #[test]
    fn a_document_that_fits_does_not_scroll() {
        let mut state = with_cart_results();
        state.doc_height = 10;
        state.focus = Focus::Document;

        press(&mut state, KeyCode::PageDown);

        assert_eq!(state.doc_scroll, 0);
    }

    #[test]
    fn shows_cached_accounts_at_once_while_refreshing_them() {
        let (state, effects) =
            AppState::with_cached_accounts(Some(vec![account("orders"), account("inventory")]));

        assert_eq!(effects, vec![Effect::LoadAccounts]);
        assert_eq!(outline(&state), vec!["orders", "inventory"]);
        assert_eq!(state.status, Status::Info("Refreshing accounts…".into()));
    }

    #[test]
    fn without_cached_accounts_they_load_as_usual() {
        let (state, _) = AppState::with_cached_accounts(None);

        assert_eq!(outline(&state), vec!["loading accounts…"]);
    }

    #[test]
    fn refreshed_accounts_keep_what_was_open_and_selected() {
        let (mut state, _) =
            AppState::with_cached_accounts(Some(vec![account("orders"), account("inventory")]));
        press(&mut state, KeyCode::Enter);
        containers_loaded(&mut state, "orders", Ok(shop_containers()));
        state.tree_selected = 6;
        assert_eq!(selected_label(&state), "inventory");

        let refreshed = vec![account("billing"), account("orders"), account("inventory")];
        update(&mut state, Event::Msg(Msg::AccountsLoaded(Ok(refreshed))));

        assert_eq!(
            outline(&state),
            vec![
                "billing",
                "orders",
                "  shop",
                "    carts",
                "    orders",
                "  audit",
                "    events",
                "inventory"
            ]
        );
        assert_eq!(selected_label(&state), "inventory");
    }

    #[test]
    fn refreshed_accounts_drop_accounts_that_are_gone() {
        let (mut state, _) =
            AppState::with_cached_accounts(Some(vec![account("orders"), account("inventory")]));
        press(&mut state, KeyCode::Down);

        update(
            &mut state,
            Event::Msg(Msg::AccountsLoaded(Ok(vec![account("orders")]))),
        );

        assert_eq!(outline(&state), vec!["orders"]);
        assert_eq!(selected_label(&state), "orders");
    }

    #[test]
    fn a_failed_refresh_keeps_the_cached_accounts() {
        let (mut state, _) = AppState::with_cached_accounts(Some(vec![account("orders")]));

        update(
            &mut state,
            Event::Msg(Msg::AccountsLoaded(Err("az login first".into()))),
        );

        assert_eq!(outline(&state), vec!["orders"]);
        assert_eq!(
            state.status,
            Status::Error("could not refresh accounts: az login first".into())
        );
    }
    fn pick(state: &mut AppState, from: (usize, usize), to: (usize, usize)) {
        let point = |(line, column)| DocPoint { line, column };
        state.doc_selection = Some(Selection {
            anchor: point(from),
            head: point(to),
        });
    }

    #[test]
    fn picked_text_runs_from_the_first_to_the_last_picked_character() {
        let mut state = with_cart_results();

        pick(&mut state, (1, 2), (1, 5));
        assert_eq!(state.selected_text().as_deref(), Some("\"id\""));

        pick(&mut state, (2, 14), (1, 9));
        assert_eq!(
            state.selected_text().as_deref(),
            Some(
                "c-1\",
  \"tenantId\": \""
            )
        );
    }

    #[test]
    fn picked_text_stops_at_the_end_of_each_line() {
        let mut state = with_cart_results();

        pick(&mut state, (0, 0), (1, 80));

        assert_eq!(
            state.selected_text().as_deref(),
            Some(
                "{
  \"id\": \"c-1\","
            )
        );
    }

    #[test]
    fn y_copies_the_picked_text() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;
        pick(&mut state, (1, 2), (1, 5));

        let effects = press(&mut state, KeyCode::Char('y'));

        assert_eq!(effects, vec![Effect::Copy(r#""id""#.into())]);
    }

    #[test]
    fn y_copies_the_whole_document_when_nothing_is_picked() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;

        let effects = press(&mut state, KeyCode::Char('y'));

        let whole = serde_json::to_string_pretty(&cart_docs()[0]).unwrap();
        assert_eq!(effects, vec![Effect::Copy(whole)]);
    }

    #[test]
    fn esc_drops_the_picked_text() {
        let mut state = with_cart_results();
        state.focus = Focus::Document;
        pick(&mut state, (1, 2), (1, 5));

        press(&mut state, KeyCode::Esc);

        assert_eq!(state.doc_selection, None);
    }

    #[test]
    fn y_copies_nothing_without_a_document() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Document;

        assert!(press(&mut state, KeyCode::Char('y')).is_empty());
    }

    #[test]
    fn tells_whether_the_text_was_copied() {
        let (mut state, _) = AppState::new();

        update(&mut state, Event::Msg(Msg::Copied(Ok(()))));
        assert_eq!(state.status, Status::Info("Copied to the clipboard".into()));

        update(
            &mut state,
            Event::Msg(Msg::Copied(Err("no display".into()))),
        );
        assert_eq!(
            state.status,
            Status::Error("could not copy: no display".into())
        );
    }

    #[test]
    fn showing_another_document_drops_the_selection() {
        let mut state = with_cart_results();
        pick(&mut state, (0, 0), (1, 4));

        press(&mut state, KeyCode::Down);

        assert_eq!(state.doc_selection, None);
    }

    #[test]
    fn nothing_is_picked_until_the_selection_reaches_past_where_it_started() {
        let mut state = with_cart_results();
        assert_eq!(state.selected_text(), None);

        pick(&mut state, (1, 3), (1, 3));
        assert_eq!(state.selected_text(), None);
    }

    fn click(state: &mut AppState, pane: Focus, x: u16, y: u16) -> Vec<Effect> {
        let at = Some(Position::new(x, y));
        let action = MouseAction::Click;
        update(state, Event::Mouse(Mouse { action, pane, at }))
    }

    fn drag(state: &mut AppState, pane: Focus, x: u16, y: u16) {
        let at = Some(Position::new(x, y));
        let action = MouseAction::Drag;
        update(state, Event::Mouse(Mouse { action, pane, at }));
    }

    #[test]
    fn dragging_in_the_document_picks_the_text_it_passes_over() {
        let mut state = with_cart_results();
        state.doc_height = 10;

        click(&mut state, Focus::Document, 2, 1);
        drag(&mut state, Focus::Document, 4, 1);
        drag(&mut state, Focus::Document, 5, 1);

        assert_eq!(state.focus, Focus::Document);
        assert_eq!(state.selected_text().as_deref(), Some("\"id\""));
    }

    #[test]
    fn letting_go_of_the_button_picks_the_text_without_any_drag() {
        let mut state = with_cart_results();
        state.doc_height = 10;

        click(&mut state, Focus::Document, 2, 1);
        let at = Some(Position::new(5, 1));
        let action = MouseAction::Release;
        let pane = Focus::Document;
        update(&mut state, Event::Mouse(Mouse { action, pane, at }));

        assert_eq!(state.selected_text().as_deref(), Some("\"id\""));
    }

    #[test]
    fn dragging_counts_the_lines_scrolled_out_of_sight() {
        let mut state = with_cart_results();
        state.results[0] = long_document(10);
        state.doc_height = 5;
        state.doc_scroll = 2;

        click(&mut state, Focus::Document, 2, 0);
        drag(&mut state, Focus::Document, 6, 0);

        assert_eq!(state.selected_text().as_deref(), Some("\"f01\""));
    }

    #[test]
    fn dragging_outside_the_document_keeps_the_selection() {
        let mut state = with_cart_results();
        state.doc_height = 10;
        click(&mut state, Focus::Document, 2, 1);
        drag(&mut state, Focus::Document, 5, 1);

        drag(&mut state, Focus::Results, 1, 1);
        let border = Mouse {
            action: MouseAction::Drag,
            pane: Focus::Document,
            at: None,
        };
        update(&mut state, Event::Mouse(border));

        assert_eq!(state.selected_text().as_deref(), Some("\"id\""));
    }

    fn wheel(state: &mut AppState, pane: Focus, action: MouseAction) -> Vec<Effect> {
        let at = Some(Position::new(0, 0));
        update(state, Event::Mouse(Mouse { action, pane, at }))
    }

    #[test]
    fn clicking_a_pane_focuses_it_even_on_its_border() {
        let mut state = with_cart_results();

        click(&mut state, Focus::Document, 4, 2);
        assert_eq!(state.focus, Focus::Document);

        let border = Mouse {
            action: MouseAction::Click,
            pane: Focus::Tree,
            at: None,
        };
        update(&mut state, Event::Mouse(border));
        assert_eq!(state.focus, Focus::Tree);
    }

    #[test]
    fn clicking_a_tree_row_selects_it_and_clicking_it_again_opens_it() {
        let mut state = with_accounts(&["orders", "inventory"]);
        state.tree_height = 10;

        let effects = click(&mut state, Focus::Tree, 3, 1);
        assert!(effects.is_empty());
        assert_eq!(selected_label(&state), "inventory");

        let effects = click(&mut state, Focus::Tree, 3, 1);
        assert_eq!(effects, vec![Effect::LoadContainers(account("inventory"))]);
    }

    #[test]
    fn clicking_a_container_queries_its_documents() {
        let mut state = with_orders_expanded();
        state.tree_height = 10;

        let effects = click(&mut state, Focus::Tree, 5, 2);

        assert_eq!(selected_label(&state), "carts");
        assert_eq!(state.target, Some(carts()));
        assert!(matches!(effects[..], [Effect::Query { .. }]));
    }

    #[test]
    fn clicking_below_the_last_tree_row_selects_nothing() {
        let mut state = with_accounts(&["orders", "inventory"]);
        state.tree_height = 10;

        click(&mut state, Focus::Tree, 3, 5);

        assert_eq!(selected_label(&state), "orders");
    }

    #[test]
    fn clicking_the_tree_counts_the_rows_scrolled_out_of_sight() {
        let mut state = with_orders_expanded();
        state.tree_height = 3;
        for _ in 0..6 {
            press(&mut state, KeyCode::Down);
        }
        // Showing rows 4 to 6, with the last one selected
        assert_eq!(state.tree_selected, 6);

        click(&mut state, Focus::Tree, 0, 0);

        assert_eq!(state.tree_selected, 4);
    }

    #[test]
    fn clicking_a_result_shows_its_document_from_the_top() {
        let mut state = with_cart_results();
        state.results_height = 10;
        state.doc_scroll = 3;

        click(&mut state, Focus::Results, 2, 2);

        assert_eq!(state.selected_document(), Some(&cart_docs()[1]));
        assert_eq!(state.doc_scroll, 0);
    }

    #[test]
    fn clicking_the_results_header_selects_nothing() {
        let mut state = with_cart_results();
        state.results_height = 10;

        click(&mut state, Focus::Results, 2, 0);

        assert_eq!(state.result_selected, 0);
    }

    #[test]
    fn clicking_the_results_counts_the_rows_scrolled_out_of_sight() {
        let mut state = with_many_results(25);
        press(&mut state, KeyCode::End);

        // Showing documents 15 to 24 below the header
        click(&mut state, Focus::Results, 0, 1);

        assert_eq!(state.result_selected, 15);
    }

    #[test]
    fn clicking_the_search_bar_puts_the_cursor_where_it_was_clicked() {
        let mut state = with_cart_results();
        state.focus = Focus::Search;
        type_text(&mut state, "c.total");

        click(&mut state, Focus::Search, 2, 0);
        assert_eq!(state.search.cursor(), 2);
        click(&mut state, Focus::Search, 40, 0);
        assert_eq!(state.search.cursor(), 7);
    }

    #[test]
    fn the_wheel_scrolls_the_pane_under_it_without_focusing_it() {
        let mut state = with_cart_results();
        state.focus = Focus::Tree;
        state.results[0] = long_document(10);
        state.doc_height = 5;

        wheel(&mut state, Focus::Document, MouseAction::ScrollDown);
        assert_eq!(state.doc_scroll, 3);
        wheel(&mut state, Focus::Document, MouseAction::ScrollDown);
        wheel(&mut state, Focus::Document, MouseAction::ScrollDown);
        assert_eq!(state.doc_scroll, 7);
        wheel(&mut state, Focus::Document, MouseAction::ScrollUp);
        assert_eq!(state.doc_scroll, 4);

        wheel(&mut state, Focus::Results, MouseAction::ScrollDown);
        assert_eq!(state.result_selected, 1);
        assert_eq!(state.focus, Focus::Tree);
    }

    #[test]
    fn the_wheel_past_the_last_result_loads_more() {
        let mut state = with_first_page();
        wheel(&mut state, Focus::Results, MouseAction::ScrollDown);
        wheel(&mut state, Focus::Results, MouseAction::ScrollDown);

        let effects = wheel(&mut state, Focus::Results, MouseAction::ScrollDown);

        assert_eq!(effects, vec![Effect::LoadMore { id: 1 }]);
    }

    #[test]
    fn a_click_only_closes_the_help() {
        let mut state = with_cart_results();
        press(&mut state, KeyCode::Char('?'));

        wheel(&mut state, Focus::Results, MouseAction::ScrollDown);
        assert!(state.show_help);
        assert_eq!(state.result_selected, 0);

        click(&mut state, Focus::Document, 2, 2);
        assert!(!state.show_help);
        assert_eq!(state.focus, Focus::Results);
    }

    #[test]
    fn clicking_the_query_editor_puts_the_cursor_there() {
        let mut state = with_output();
        state.editor_height = 5;
        state.editor.set_text("SELECT *\nFROM c\nWHERE c.n > 1");

        // Past the line numbers, which take two digits and a space
        click(&mut state, Focus::Editor, 5, 1);
        assert_eq!(state.focus, Focus::Editor);
        assert_eq!(state.editor.cursor(), (1, 2));
        click(&mut state, Focus::Editor, 1, 9);
        assert_eq!(state.editor.cursor(), (2, 0));
    }

    #[test]
    fn the_wheel_scrolls_the_query_output() {
        let mut state = with_output();
        state.focus = Focus::Editor;

        wheel(&mut state, Focus::Output, MouseAction::ScrollDown);

        assert_eq!(state.output_scroll, 3);
        assert_eq!(state.focus, Focus::Editor);
    }

    #[test]
    fn y_in_the_output_copies_every_result_as_a_json_array() {
        let mut state = with_output();

        let effects = press(&mut state, KeyCode::Char('y'));

        let json = serde_json::to_string_pretty(&cart_docs()).unwrap();
        assert_eq!(effects, vec![Effect::Copy(json)]);
    }

    fn costing(stats: QueryStats, docs: Vec<Value>) -> Result<QueryResult, String> {
        Ok(QueryResult {
            docs,
            more: true,
            pk_path: "/tenantId".into(),
            elapsed: Duration::from_millis(250),
            stats,
        })
    }

    #[test]
    fn the_status_line_and_stats_add_up_what_the_query_cost() {
        let mut state = with_output();
        press(&mut state, KeyCode::F(5));
        let stats = QueryStats {
            request_charge: 3.5,
            round_trips: 2,
            ..QueryStats::default()
        };
        update(
            &mut state,
            Event::Msg(Msg::QueryDone {
                id: 3,
                result: costing(stats.clone(), cart_docs()),
            }),
        );
        assert_eq!(
            state.status,
            Status::Info("2 documents · 3.50 RU in 0.25s".into())
        );

        update(
            &mut state,
            Event::Msg(Msg::MoreLoaded {
                id: 3,
                result: costing(stats, cart_docs()),
            }),
        );

        assert_eq!(
            state.output.stats,
            QueryStats {
                request_charge: 7.0,
                round_trips: 4,
                ..QueryStats::default()
            }
        );
        assert_eq!(
            state.status,
            Status::Info("4 documents · 7.00 RU in 0.25s".into())
        );
    }

    #[test]
    fn query_stats_add_up_charges_round_trips_and_metrics_by_name() {
        let mut total = QueryStats::default();
        total.add_query_metrics("retrievedDocumentCount=30;totalExecutionTimeInMs=1.50");
        let mut more = QueryStats {
            request_charge: 2.5,
            round_trips: 1,
            ..QueryStats::default()
        };
        more.add_query_metrics("retrievedDocumentCount=10;totalExecutionTimeInMs=0.25;new=1;bad=x");

        total.add(more);

        assert_eq!(total.request_charge, 2.5);
        assert_eq!(total.round_trips, 1);
        assert_eq!(
            total.query_metrics,
            vec![
                ("retrievedDocumentCount".to_string(), 40.0),
                ("totalExecutionTimeInMs".to_string(), 1.75),
                ("new".to_string(), 1.0),
            ]
        );
    }

    #[test]
    fn s_in_the_output_switches_between_the_results_and_the_stats() {
        let mut state = with_output();
        assert_eq!(state.output_tab, OutputTab::Results);

        press(&mut state, KeyCode::Char('s'));
        assert_eq!(state.output_tab, OutputTab::Stats);
        press(&mut state, KeyCode::Char('s'));
        assert_eq!(state.output_tab, OutputTab::Results);
    }

    fn ctrl_s(state: &mut AppState) -> Vec<Effect> {
        let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);
        update(state, Event::Key(key))
    }

    #[test]
    fn ctrl_s_asks_where_to_save_the_query_then_saves_it() {
        let mut state = with_query_editor();
        state.editor.set_text("SELECT VALUE c.id\nFROM c");

        ctrl_s(&mut state);
        assert_eq!(
            state.prompt.as_ref().map(|p| p.path.text()),
            Some("carts.sql")
        );
        press(&mut state, KeyCode::Backspace);
        press(&mut state, KeyCode::Backspace);
        press(&mut state, KeyCode::Backspace);
        type_text(&mut state, "txt");
        let effects = press(&mut state, KeyCode::Enter);

        assert_eq!(
            effects,
            vec![Effect::Save {
                path: "carts.txt".into(),
                contents: "SELECT VALUE c.id\nFROM c\n".into()
            }]
        );
        assert!(state.prompt.is_none());
        assert_eq!(state.editor.text(), "SELECT VALUE c.id\nFROM c");
    }

    #[test]
    fn escape_in_the_save_prompt_saves_nothing() {
        let mut state = with_query_editor();
        ctrl_s(&mut state);

        let effects = press(&mut state, KeyCode::Esc);

        assert!(effects.is_empty());
        assert!(state.prompt.is_none());
        assert_eq!(state.focus, Focus::Editor);
    }

    #[test]
    fn says_where_a_file_was_saved_or_why_not() {
        let mut state = with_query_editor();

        update(&mut state, Event::Msg(Msg::Saved(Ok("carts.sql".into()))));
        assert_eq!(state.status, Status::Info("Saved carts.sql".into()));

        update(&mut state, Event::Msg(Msg::Saved(Err("denied".into()))));
        assert_eq!(state.status, Status::Error("could not save: denied".into()));
    }

    #[test]
    fn w_in_the_output_asks_where_to_save_the_results_then_saves_them() {
        let mut state = with_output();

        press(&mut state, KeyCode::Char('w'));
        assert_eq!(
            state.prompt.as_ref().map(|p| p.path.text()),
            Some("carts.json")
        );
        let effects = press(&mut state, KeyCode::Enter);

        let json = serde_json::to_string_pretty(&cart_docs()).unwrap();
        assert_eq!(
            effects,
            vec![Effect::Save {
                path: "carts.json".into(),
                contents: format!("{json}\n")
            }]
        );
    }

    #[test]
    fn ctrl_s_in_the_output_saves_the_query_too() {
        let mut state = with_output();

        ctrl_s(&mut state);

        assert_eq!(
            state.prompt.as_ref().map(|p| p.path.text()),
            Some("carts.sql")
        );
    }

    #[test]
    fn unbound_ctrl_keys_type_nothing_in_the_query_editor() {
        let mut state = with_query_editor();

        press_ctrl(&mut state, 'a');
        press_ctrl(&mut state, 'u');

        assert_eq!(state.editor.text(), "SELECT * FROM c");
    }

    #[test]
    fn slash_in_query_mode_jumps_to_the_editor() {
        let mut state = with_output();

        press(&mut state, KeyCode::Char('/'));

        assert_eq!(state.focus, Focus::Editor);
    }

    #[test]
    fn the_editor_query_shows_its_results_apart_from_the_browse_results() {
        let mut state = with_query_editor();
        state.editor.set_text("SELECT c.name FROM c");
        press(&mut state, KeyCode::F(5));

        query_done(&mut state, 2, Ok(vec![json!({ "name": "Taylor" })]));

        assert_eq!(state.results, cart_docs());
        assert_eq!(state.output.results, vec![json!({ "name": "Taylor" })]);
    }

    #[test]
    fn the_browse_query_leaves_the_editor_results_alone() {
        let mut state = with_query_editor();
        press(&mut state, KeyCode::F(5));
        query_done(&mut state, 2, Ok(vec![json!(1)]));
        press(&mut state, KeyCode::Esc);
        press(&mut state, KeyCode::Esc);

        press(&mut state, KeyCode::Char('r'));
        query_done(&mut state, 3, Ok(cart_docs()));

        assert_eq!(state.results, cart_docs());
        assert_eq!(state.output.results, vec![json!(1)]);
    }

    #[test]
    fn more_editor_results_follow_its_output_and_leave_the_browse_results_alone() {
        let mut state = with_output();
        state.output.more = true;
        press(&mut state, KeyCode::End);
        press(&mut state, KeyCode::Down);

        more_loaded(&mut state, 2, Ok(page(2, 4, false)));

        assert_eq!(state.output.results.len(), 4);
        assert!(!state.output.more && !state.output.loading_more);
        assert_eq!(state.results, cart_docs());
    }

    #[test]
    fn s_opens_the_settings_of_the_picked_container() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);

        let effects = press(&mut state, KeyCode::Char('s'));

        assert_eq!(effects, vec![Effect::LoadSettings(carts())]);
        assert_eq!(state.mode, Mode::Settings);
        assert_eq!(state.focus, Focus::SettingsForm);
        assert!(matches!(state.settings, Some(Load::Loading)));
    }

    #[test]
    fn s_without_a_container_asks_for_one() {
        let mut state = with_accounts(&["orders"]);

        let effects = press(&mut state, KeyCode::Char('s'));

        assert!(effects.is_empty());
        assert_eq!(state.mode, Mode::Browse);
        assert_eq!(state.status, Status::Error("pick a container first".into()));
    }

    fn with_carts_settings() -> AppState {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        press(&mut state, KeyCode::Char('s'));
        state
    }

    fn settings_loaded(state: &mut AppState, target: Target, result: Result<Value, String>) {
        update(state, Event::Msg(Msg::SettingsLoaded { target, result }));
    }

    fn shown_settings(state: &AppState) -> &ContainerSettings {
        match &state.settings {
            Some(Load::Loaded(settings)) => settings,
            other => panic!("no settings shown: {other:?}"),
        }
    }

    #[test]
    fn shows_the_loaded_settings() {
        let mut state = with_carts_settings();

        settings_loaded(
            &mut state,
            carts(),
            Ok(json!({"id": "carts", "defaultTtl": -1})),
        );

        assert_eq!(shown_settings(&state).ttl, TimeToLive::NoDefault);
    }

    #[test]
    fn shows_why_the_settings_failed_to_load() {
        let mut state = with_carts_settings();

        settings_loaded(&mut state, carts(), Err("forbidden".into()));

        assert!(matches!(&state.settings, Some(Load::Failed(error)) if error == "forbidden"));
    }

    #[test]
    fn ignores_settings_of_another_container() {
        let mut state = with_carts_settings();
        let other = Target {
            container: "orders".into(),
            ..carts()
        };

        settings_loaded(&mut state, other, Ok(json!({"id": "orders"})));

        assert!(matches!(state.settings, Some(Load::Loading)));
    }

    #[test]
    fn tab_cycles_through_the_tree_and_the_settings_tabs() {
        let mut state = with_carts_settings();
        let mut visited = vec![(state.focus, state.settings_tab)];

        for _ in 0..4 {
            press(&mut state, KeyCode::Tab);
            visited.push((state.focus, state.settings_tab));
        }
        press(&mut state, KeyCode::BackTab);

        assert_eq!(
            visited,
            [
                (Focus::SettingsForm, SettingsTab::Settings),
                (Focus::IndexingPolicy, SettingsTab::IndexingPolicy),
                (Focus::ComputedProperties, SettingsTab::ComputedProperties),
                (Focus::Tree, SettingsTab::ComputedProperties),
                (Focus::SettingsForm, SettingsTab::Settings),
            ]
        );
        assert_eq!(state.focus, Focus::Tree);
    }

    fn with_loaded_carts_settings(properties: Value) -> AppState {
        let mut state = with_carts_settings();
        settings_loaded(&mut state, carts(), Ok(properties));
        state
    }

    #[test]
    fn form_keys_change_the_time_to_live_seconds_and_geospatial_type() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));

        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::Char('l'));
        assert_eq!(shown_settings(&state).ttl, TimeToLive::Seconds);
        press(&mut state, KeyCode::Down);
        type_text(&mut state, "905");
        press(&mut state, KeyCode::Backspace);
        assert_eq!(shown_settings(&state).seconds.text(), "90");
        press(&mut state, KeyCode::Char('j'));
        press(&mut state, KeyCode::Char(' '));
        assert_eq!(shown_settings(&state).geospatial, Geospatial::Geometry);
        press(&mut state, KeyCode::Up);
        press(&mut state, KeyCode::Char('k'));
        press(&mut state, KeyCode::Char('h'));
        press(&mut state, KeyCode::Left);

        let settings = shown_settings(&state);
        assert_eq!(settings.field, Field::Ttl);
        assert_eq!(settings.ttl, TimeToLive::Off);
    }

    #[test]
    fn keys_on_the_json_tabs_edit_the_indexing_policy_and_computed_properties() {
        let mut state = with_loaded_carts_settings(json!({"indexingPolicy": {}}));
        press(&mut state, KeyCode::Tab);

        type_text(&mut state, "q/nS");
        press(&mut state, KeyCode::Enter);
        press(&mut state, KeyCode::Tab);
        press(&mut state, KeyCode::End);
        press(&mut state, KeyCode::Backspace);
        type_text(&mut state, "1]");

        let settings = shown_settings(&state);
        assert_eq!(settings.indexing.text(), "q/nS\n{}");
        assert_eq!(settings.computed.text(), "[1]");
        assert_eq!(state.mode, Mode::Settings);
        assert!(!state.quit);
    }

    #[test]
    fn ctrl_s_saves_the_edited_settings() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);

        let effects = ctrl_s(&mut state);

        assert_eq!(
            effects,
            vec![Effect::ReplaceSettings {
                target: carts(),
                properties: json!({"id": "carts", "defaultTtl": -1}),
            }]
        );
        assert_eq!(state.status, Status::Info("Saving the settings…".into()));
    }

    #[test]
    fn saving_settings_that_are_not_valid_shows_why_on_their_tab() {
        let mut state = with_loaded_carts_settings(json!({"indexingPolicy": {}}));
        press(&mut state, KeyCode::Tab);
        press(&mut state, KeyCode::Delete);
        press(&mut state, KeyCode::BackTab);

        let effects = ctrl_s(&mut state);

        assert!(effects.is_empty());
        assert_eq!(state.focus, Focus::IndexingPolicy);
        assert_eq!(state.settings_tab, SettingsTab::IndexingPolicy);
        assert_eq!(
            state.error.as_deref(),
            Some("The indexing policy is not valid JSON: expected value at line 1 column 1")
        );
    }

    fn settings_saved(state: &mut AppState, result: Result<Value, String>) {
        let target = carts();
        update(state, Event::Msg(Msg::SettingsSaved { target, result }));
    }

    #[test]
    fn shows_the_saved_settings_with_no_changes_left() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);
        ctrl_s(&mut state);

        settings_saved(&mut state, Ok(json!({"id": "carts", "defaultTtl": -1})));

        let settings = shown_settings(&state);
        assert_eq!(settings.ttl, TimeToLive::NoDefault);
        assert!(!settings.modified(SettingsTab::Settings));
        assert_eq!(
            state.status,
            Status::Info("Saved the settings of shop/carts".into())
        );
    }

    #[test]
    fn keeps_the_edits_when_saving_the_settings_fails() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);
        ctrl_s(&mut state);

        settings_saved(&mut state, Err("The indexing policy is invalid".into()));

        assert!(shown_settings(&state).modified(SettingsTab::Settings));
        assert_eq!(
            state.error.as_deref(),
            Some("The indexing policy is invalid")
        );
        assert_eq!(
            state.status,
            Status::Error("could not save the settings".into())
        );
    }

    #[test]
    fn esc_leaves_unchanged_settings_for_the_mode_they_were_opened_from() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Tab);

        press(&mut state, KeyCode::Esc);

        assert_eq!(state.mode, Mode::Browse);
        assert_eq!(state.focus, Focus::Results);
        assert!(state.settings.is_none());

        press(&mut state, KeyCode::Char('n'));
        press(&mut state, KeyCode::BackTab);
        press(&mut state, KeyCode::Char('s'));
        press(&mut state, KeyCode::Char('s'));
        assert_eq!(state.mode, Mode::Query);
        assert_eq!(state.focus, Focus::Editor);
    }

    #[test]
    fn leaving_changed_settings_asks_to_discard_the_changes_first() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);

        press(&mut state, KeyCode::Esc);
        assert!(state.confirm_discard);
        press(&mut state, KeyCode::Char('n'));
        assert!(!state.confirm_discard);
        assert_eq!(state.mode, Mode::Settings);
        assert_eq!(shown_settings(&state).ttl, TimeToLive::NoDefault);

        press(&mut state, KeyCode::Char('s'));
        press(&mut state, KeyCode::Char('y'));

        assert!(!state.confirm_discard);
        assert_eq!(state.mode, Mode::Browse);
        assert!(state.settings.is_none());
    }

    #[test]
    fn n_leaves_the_settings_alone() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));

        press(&mut state, KeyCode::Char('n'));

        assert_eq!(state.mode, Mode::Settings);
        assert!(shown_settings(&state).field == Field::Ttl);
    }

    fn shop_orders() -> Target {
        Target {
            container: "orders".into(),
            ..carts()
        }
    }

    #[test]
    fn picking_another_container_shows_its_settings_and_lists_it_when_they_are_left() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::BackTab);

        let effects = press(&mut state, KeyCode::Down);

        assert_eq!(effects, vec![Effect::LoadSettings(shop_orders())]);
        assert_eq!(state.target, Some(shop_orders()));
        assert!(matches!(state.settings, Some(Load::Loading)));

        let effects = press(&mut state, KeyCode::Esc);
        assert!(matches!(
            effects.as_slice(),
            [Effect::Query { target, origin: Origin::Browse, .. }] if *target == shop_orders()
        ));
    }

    #[test]
    fn changed_settings_keep_their_container_picked() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);
        press(&mut state, KeyCode::BackTab);

        let effects = press(&mut state, KeyCode::Down);

        assert!(effects.is_empty());
        assert_eq!(state.target, Some(carts()));
        assert_eq!(shown_settings(&state).ttl, TimeToLive::NoDefault);
        assert_eq!(
            state.status,
            Status::Error(
                "save (Ctrl-S) or discard (Esc) the changes to the settings first".into()
            )
        );
    }

    #[test]
    fn clicking_the_settings_focuses_the_tab_they_show() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::BackTab);
        state.settings_tab = SettingsTab::ComputedProperties;

        click(&mut state, Focus::SettingsForm, 3, 3);

        assert_eq!(state.focus, Focus::ComputedProperties);
    }

    #[test]
    fn slash_keeps_the_focus_in_the_settings() {
        let mut state = with_loaded_carts_settings(json!({"id": "carts"}));

        press(&mut state, KeyCode::Char('/'));

        assert_eq!(state.focus, Focus::SettingsForm);
    }

    #[test]
    fn s_in_the_query_output_switches_its_tab_rather_than_opening_the_settings() {
        let mut state = with_query_editor();
        press(&mut state, KeyCode::Tab);

        press(&mut state, KeyCode::Char('s'));

        assert_eq!(state.mode, Mode::Query);
        assert_eq!(state.output_tab, OutputTab::Stats);
    }

    #[test]
    fn ctrl_s_while_browsing_opens_no_settings() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);

        ctrl_s(&mut state);

        assert_eq!(state.mode, Mode::Browse);
    }
}
