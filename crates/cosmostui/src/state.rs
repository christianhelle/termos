//! What the screen shows, and how keys and finished work change it.

use cosmos_core::management::{Account, Container};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Work for the runtime to do in the background.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    LoadAccounts,
    LoadContainers(Account),
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
}

impl AppState {
    /// The state on startup, with the work to start right away.
    pub fn new() -> (Self, Vec<Effect>) {
        let state = AppState {
            accounts: Load::Loading,
            status: Status::Info("Loading accounts…".into()),
            quit: false,
            tree_selected: 0,
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
        _ => false,
    };
    if expanded {
        collapse(state);
        Vec::new()
    } else {
        expand(state)
    }
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
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_core::testing::{account, container};

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
}
