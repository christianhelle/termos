//! What the screen shows, and how keys and finished work change it.

use crate::management::{Account, Container};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Position;
use serde_json::Value;

use crate::input::TextInput;
use crate::query::{DEFAULT_QUERY, build_query};

/// Work for the runtime to do in the background.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    LoadAccounts,
    LoadContainers(Account),
    /// Runs a query on a container. Only the latest query's results are shown.
    Query {
        id: u64,
        target: Target,
        sql: String,
    },
    /// Reads the next page of the latest query.
    LoadMore {
        id: u64,
    },
}

/// A container to query.
#[derive(Debug, Clone, PartialEq)]
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
}

/// Something that happened, for [`update`] to act on.
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Mouse(Mouse),
    Msg(Msg),
}

/// A click or turn of the wheel over a pane.
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

/// The pane that keys go to.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Focus {
    Tree,
    Search,
    Results,
    Document,
}

impl Focus {
    const ORDER: [Focus; 4] = [Focus::Tree, Focus::Search, Focus::Results, Focus::Document];

    fn next(self) -> Focus {
        let index = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(index + 1) % Self::ORDER.len()]
    }

    fn previous(self) -> Focus {
        let index = Self::ORDER.iter().position(|f| *f == self).unwrap_or(0);
        Self::ORDER[(index + Self::ORDER.len() - 1) % Self::ORDER.len()]
    }
}

/// Everything the screen shows.
pub struct AppState {
    pub accounts: Load<Vec<AccountNode>>,
    pub status: Status,
    pub quit: bool,
    /// Whether the key help covers the screen.
    pub show_help: bool,
    pub focus: Focus,
    /// What is typed in the search bar.
    pub search: TextInput,
    /// Index of the selected tree row.
    pub tree_selected: usize,
    /// The container that queries run against.
    pub target: Option<Target>,
    /// The id of the latest query.
    pub query_id: u64,
    /// The documents the latest query found.
    pub results: Vec<Value>,
    /// The partition key path of the container the results came from.
    pub pk_path: String,
    /// Index of the selected document in the results.
    pub result_selected: usize,
    /// The SQL of the latest query, to run again on refresh.
    pub last_sql: String,
    /// How many lines the document pane is scrolled down.
    pub doc_scroll: u16,
    /// How many lines of a document the document pane shows at once.
    pub doc_height: u16,
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
}

impl AppState {
    /// The state on startup, with the work to start right away.
    pub fn new() -> (Self, Vec<Effect>) {
        let state = AppState {
            accounts: Load::Loading,
            status: Status::Info("Loading accounts…".into()),
            quit: false,
            show_help: false,
            focus: Focus::Tree,
            search: TextInput::default(),
            tree_selected: 0,
            target: None,
            query_id: 0,
            results: Vec::new(),
            pk_path: String::new(),
            result_selected: 0,
            last_sql: String::new(),
            doc_scroll: 0,
            doc_height: 0,
            results_height: 0,
            tree_height: 0,
            pending_g: false,
            more: false,
            loading_more: false,
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
    if state.show_help && !ctrl_c {
        // Any other key closes the help, and only that
        state.show_help = false;
        return Vec::new();
    }
    match key.code {
        KeyCode::Char('c') if ctrl => state.quit = true,
        KeyCode::Tab => state.focus = state.focus.next(),
        KeyCode::BackTab => state.focus = state.focus.previous(),
        _ => {
            return match state.focus {
                Focus::Search => on_search_key(state, key),
                _ => on_pane_key(state, key),
            };
        }
    }
    Vec::new()
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
    match mouse.action {
        MouseAction::Click => on_click(state, mouse.pane, mouse.at),
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
/// and a click on the selected tree row opens or closes it like Enter.
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
            }
        }
        // The first line is the header
        Focus::Results if line > 0 => {
            let index = first_visible(state.result_selected, state.results_height) + line - 1;
            if index < state.results.len() && index != state.result_selected {
                state.result_selected = index;
                state.doc_scroll = 0;
            }
        }
        Focus::Search => state.search.move_to(usize::from(at.x)),
        Focus::Results | Focus::Document => {}
    }
    Vec::new()
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
        Focus::Search => Vec::new(),
    }
}

/// Keys that work in every pane but the search bar, where they are typed instead.
fn on_pane_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Char('q') => state.quit = true,
        KeyCode::Char('/') => state.focus = Focus::Search,
        KeyCode::Char('?') => state.show_help = true,
        _ => {
            return match state.focus {
                Focus::Tree => on_tree_key(state, key),
                Focus::Results => on_results_key(state, key),
                Focus::Document => on_document_key(state, key),
                Focus::Search => Vec::new(),
            };
        }
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
            state.doc_scroll = 0;
        }
        return Vec::new();
    }
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            let last = state.results.len().saturating_sub(1);
            if state.result_selected == last && state.more && !state.loading_more {
                state.loading_more = true;
                state.status = Status::Info("Loading more documents…".into());
                return vec![Effect::LoadMore { id: state.query_id }];
            }
            state.result_selected = (state.result_selected + 1).min(last);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.result_selected = state.result_selected.saturating_sub(1);
        }
        KeyCode::Enter if state.selected_document().is_some() => state.focus = Focus::Document,
        KeyCode::Char('r') => return run_query(state, state.last_sql.clone()),
        _ => {}
    }
    if state.result_selected != selected {
        state.doc_scroll = 0;
    }
    Vec::new()
}

fn on_document_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
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
            return run_query(state, build_query(state.search.text()));
        }
        _ => {}
    }
    Vec::new()
}

fn on_tree_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            let last = state.tree_rows().len().saturating_sub(1);
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
    Vec::new()
}

/// Opens a closed tree node, or closes an open one.
fn toggle(state: &mut AppState) -> Vec<Effect> {
    let expanded = match state.selected_node() {
        Some(Node::Account(a)) => state.account_mut(a).is_some_and(|node| node.expanded),
        Some(Node::Database(a, d)) => state.database_mut(a, d).is_some_and(|node| node.expanded),
        Some(Node::Container(a, d, c)) => return open_container(state, a, d, c),
        _ => false,
    };
    if expanded {
        collapse(state);
        Vec::new()
    } else {
        expand(state)
    }
}

/// Makes a container the one queries run against, and lists its documents.
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
    state.target = Some(target);
    state.results.clear();
    state.result_selected = 0;
    run_query(state, DEFAULT_QUERY.to_string())
}

/// Queries the current container, if there is one.
fn run_query(state: &mut AppState, sql: String) -> Vec<Effect> {
    let Some(target) = state.target.clone() else {
        state.status = Status::Error("pick a container first".into());
        return Vec::new();
    };
    state.query_id += 1;
    state.more = false;
    state.loading_more = false;
    state.last_sql.clone_from(&sql);
    state.status = Status::Info(format!(
        "Querying {}/{}…",
        target.database, target.container
    ));
    vec![Effect::Query {
        id: state.query_id,
        target,
        sql,
    }]
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
fn containers_loaded(
    state: &mut AppState,
    account: &str,
    result: Result<Vec<(String, Container)>, String>,
) {
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
    let mut databases: Vec<DatabaseNode> = Vec::new();
    for (database, container) in containers {
        match databases.iter_mut().find(|d| d.name == database) {
            Some(node) => node.containers.push(container),
            None => databases.push(DatabaseNode {
                name: database,
                expanded: true,
                containers: vec![container],
            }),
        }
    }
    node.databases = Some(Load::Loaded(databases));
}

/// Shows the documents a query found.
fn query_done(state: &mut AppState, id: u64, result: Result<QueryResult, String>) {
    if id != state.query_id {
        return;
    }
    match result {
        Ok(result) => {
            state.status = Status::Info(format!(
                "{} in {:.2}s",
                documents(result.docs.len()),
                result.elapsed.as_secs_f64()
            ));
            state.results = result.docs;
            state.more = result.more;
            state.loading_more = false;
            state.result_selected = 0;
            state.doc_scroll = 0;
            state.pk_path = result.pk_path;
        }
        Err(error) => state.status = Status::Error(error),
    }
}

/// Adds the next page of a query to the results, selecting its first document.
fn more_loaded(state: &mut AppState, id: u64, result: Result<QueryResult, String>) {
    if id != state.query_id {
        return;
    }
    state.loading_more = false;
    match result {
        Ok(result) => {
            if !result.docs.is_empty() {
                state.result_selected = state.results.len();
                state.doc_scroll = 0;
            }
            state.results.extend(result.docs);
            state.more = result.more;
            state.status = Status::Info(format!(
                "{} in {:.2}s",
                documents(state.results.len()),
                result.elapsed.as_secs_f64()
            ));
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
    state.status = Status::Info("Pick a container".into());
    state.reselect(selected);
}

/// Counts documents in words, such as "1 document" or "3 documents".
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
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn enter_toggles_a_loaded_account_without_reloading_it() {
        let mut state = with_orders_expanded();

        let effects = press(&mut state, KeyCode::Enter);
        assert_eq!(outline(&state), vec!["orders", "inventory"]);

        let effects = [effects, press(&mut state, KeyCode::Enter)].concat();
        assert!(effects.is_empty());
        assert_eq!(outline(&state).len(), 7);
    }

    fn carts() -> Target {
        Target {
            account: account("orders"),
            database: "shop".into(),
            container: "carts".into(),
        }
    }

    /// Opens orders/shop/carts, returning the work that starts.
    fn open_carts(state: &mut AppState) -> Vec<Effect> {
        press(state, KeyCode::Down);
        press(state, KeyCode::Down);
        press(state, KeyCode::Enter)
    }

    #[test]
    fn opening_a_container_queries_every_document() {
        let mut state = with_orders_expanded();

        let effects = open_carts(&mut state);

        assert_eq!(
            effects,
            vec![Effect::Query {
                id: 1,
                target: carts(),
                sql: "SELECT * FROM c".into()
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
        press(&mut state, KeyCode::Enter);

        query_done(&mut state, 1, Ok(cart_docs()));

        assert!(state.results.is_empty());
        assert_eq!(state.status, Status::Info("Querying shop/carts…".into()));
    }

    #[test]
    fn reports_a_failed_query() {
        let mut state = with_orders_expanded();
        open_carts(&mut state);

        query_done(&mut state, 1, Err("syntax error".into()));

        assert_eq!(state.status, Status::Error("syntax error".into()));
    }

    #[test]
    fn tab_and_shift_tab_cycle_through_the_panes() {
        let (mut state, _) = AppState::new();
        assert_eq!(state.focus, Focus::Tree);

        let order: Vec<Focus> = (0..4)
            .map(|_| {
                press(&mut state, KeyCode::Tab);
                state.focus
            })
            .collect();
        assert_eq!(
            order,
            vec![Focus::Search, Focus::Results, Focus::Document, Focus::Tree]
        );

        press(&mut state, KeyCode::BackTab);
        assert_eq!(state.focus, Focus::Document);
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
                sql: "SELECT * FROM c WHERE c.qty > 1".into()
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

    fn with_cart_results() -> AppState {
        let mut state = with_orders_expanded();
        open_carts(&mut state);
        query_done(&mut state, 1, Ok(cart_docs()));
        state.focus = Focus::Results;
        state
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
    fn click(state: &mut AppState, pane: Focus, x: u16, y: u16) -> Vec<Effect> {
        let at = Some(Position::new(x, y));
        let action = MouseAction::Click;
        update(state, Event::Mouse(Mouse { action, pane, at }))
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
}
