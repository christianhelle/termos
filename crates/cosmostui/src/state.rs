//! What the screen shows, and how keys and finished work change it.

use cosmos_core::management::Account;
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
}

/// An account in the tree.
#[derive(Debug)]
pub struct AccountNode {
    pub account: Account,
}

/// One visible line of the tree.
#[derive(Debug, Clone, PartialEq)]
pub struct TreeRow {
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
}

impl AppState {
    /// The state on startup, with the work to start right away.
    pub fn new() -> (Self, Vec<Effect>) {
        let state = AppState {
            accounts: Load::Loading,
            status: Status::Info("Loading accounts…".into()),
            quit: false,
        };
        (state, vec![Effect::LoadAccounts])
    }

    /// The tree lines that are visible, top to bottom.
    pub fn tree_rows(&self) -> Vec<TreeRow> {
        match &self.accounts {
            Load::Loading => vec![TreeRow {
                depth: 0,
                label: "loading accounts…".into(),
            }],
            Load::Loaded(accounts) => accounts
                .iter()
                .map(|node| TreeRow {
                    depth: 0,
                    label: node.account.name.clone(),
                })
                .collect(),
        }
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
        _ => {}
    }
    Vec::new()
}

fn on_msg(state: &mut AppState, msg: Msg) -> Vec<Effect> {
    match msg {
        Msg::AccountsLoaded(Ok(accounts)) => {
            let nodes = accounts
                .into_iter()
                .map(|account| AccountNode { account })
                .collect();
            state.accounts = Load::Loaded(nodes);
            state.status = Status::Info("Pick a container".into());
        }
        Msg::AccountsLoaded(Err(error)) => {
            state.accounts = Load::Loaded(Vec::new());
            state.status = Status::Error(error);
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_core::testing::account;

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
}
