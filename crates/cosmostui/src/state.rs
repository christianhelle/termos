//! What the screen shows, and how keys and finished work change it.

use cosmos_core::management::{Account, Container};
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::Value;

use crate::query::DEFAULT_QUERY;

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
}

/// The documents a query found.
#[derive(Debug)]
pub struct QueryResult {
    pub docs: Vec<Value>,
    /// The partition key path of the queried container.
    pub pk_path: String,
    pub elapsed: Duration,
}

/// Something that happened, for [`update`] to act on.
#[derive(Debug)]
pub enum Event {
    Key(KeyEvent),
    Msg(Msg),
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

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
    pub node: Node,
    pub depth: usize,
    pub label: String,
}

/// The message on the status line.
#[derive(Debug, Clone, PartialEq)]
pub enum Status {
    Info(String),
    Error(String),
}

/// Everything the screen shows.
pub struct AppState {
    pub accounts: Load<Vec<AccountNode>>,
    pub status: Status,
    pub quit: bool,
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
}

impl AppState {
    /// The state on startup, with the work to start right away.
    pub fn new() -> (Self, Vec<Effect>) {
        let state = AppState {
            accounts: Load::Loading,
            status: Status::Info("Loading accounts…".into()),
            quit: false,
            tree_selected: 0,
            target: None,
            query_id: 0,
            results: Vec::new(),
            pk_path: String::new(),
        };
        (state, vec![Effect::LoadAccounts])
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
            rows.push(row(Node::Account(a), 0, &node.account.name));
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
                        rows.push(row(Node::Database(a, d), 1, &database.name));
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
    }
}

/// Applies an event to the state, returning the background work it starts.
pub fn update(state: &mut AppState, event: Event) -> Vec<Effect> {
    match event {
        Event::Key(key) => on_key(state, key),
        Event::Msg(msg) => on_msg(state, msg),
    }
}

fn on_key(state: &mut AppState, key: KeyEvent) -> Vec<Effect> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Char('c') if ctrl => state.quit = true,
        KeyCode::Char('q') => state.quit = true,
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
    run_query(state, DEFAULT_QUERY.to_string())
}

/// Queries the current container, if there is one.
fn run_query(state: &mut AppState, sql: String) -> Vec<Effect> {
    let Some(target) = state.target.clone() else {
        state.status = Status::Error("pick a container first".into());
        return Vec::new();
    };
    state.query_id += 1;
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
    if let Ok(result) = result {
        state.status = Status::Info(format!(
            "{} in {:.2}s",
            documents(result.docs.len()),
            result.elapsed.as_secs_f64()
        ));
        state.results = result.docs;
        state.pk_path = result.pk_path;
    }
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
        Msg::AccountsLoaded(Ok(accounts)) => {
            let nodes = accounts
                .into_iter()
                .map(|account| AccountNode {
                    account,
                    expanded: false,
                    databases: None,
                })
                .collect();
            state.accounts = Load::Loaded(nodes);
            state.status = Status::Info("Pick a container".into());
        }
        Msg::AccountsLoaded(Err(error)) => {
            state.accounts = Load::Loaded(Vec::new());
            state.status = Status::Error(error);
        }
        Msg::ContainersLoaded { account, result } => containers_loaded(state, &account, result),
        Msg::QueryDone { id, result } => query_done(state, id, result),
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_core::testing::{account, container};
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
}
