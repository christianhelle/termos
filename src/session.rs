//! The screen as it was when termos quit, saved so the next run can show it at once.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::management::{Account, Container};
use crate::state::{
    AccountNode, AppState, DatabaseNode, Effect, Focus, Load, Mode, OutputTab, Target,
};

/// The snapshot format, so a file from another version is left alone.
pub const VERSION: u32 = 1;

/// What the screen showed, in a form that can be saved.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub version: u32,
    pub accounts: Vec<AccountSnapshot>,
    pub tree_selected: usize,
    pub tree_hidden: bool,
    pub zoomed: bool,
    pub focus: Focus,
    pub mode: Mode,
    pub output_tab: OutputTab,
    /// What was typed in the search bar, and where its cursor was.
    pub search: String,
    pub search_cursor: usize,
    /// The query in the query editor, and its cursor as line and column.
    pub editor: String,
    pub editor_cursor: (usize, usize),
    pub target: Option<Target>,
    pub last_sql: String,
    /// The documents the results showed.
    pub results: Vec<Value>,
    pub pk_path: String,
    pub more: bool,
    pub result_selected: usize,
    /// Indices of the marked results.
    pub marked: Vec<usize>,
    pub doc_scroll: u16,
}

/// An account in the tree, with its databases if they were listed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    pub account: Account,
    pub expanded: bool,
    pub databases: Option<Vec<DatabaseSnapshot>>,
}

/// A database in the tree, with its containers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DatabaseSnapshot {
    pub name: String,
    pub expanded: bool,
    pub containers: Vec<Container>,
}

impl AppState {
    /// What the screen shows, to save for the next run.
    pub fn snapshot(&self) -> Snapshot {
        let accounts = match &self.accounts {
            Load::Loaded(nodes) => nodes.iter().map(account_snapshot).collect(),
            Load::Loading | Load::Failed(_) => Vec::new(),
        };
        Snapshot {
            version: VERSION,
            accounts,
            tree_selected: self.tree_selected,
            tree_hidden: self.tree_hidden,
            zoomed: self.zoomed,
            focus: self.focus,
            mode: self.mode,
            output_tab: self.output_tab,
            search: self.search.text().to_string(),
            search_cursor: self.search.cursor(),
            editor: self.editor.text(),
            editor_cursor: self.editor.cursor(),
            target: self.target.clone(),
            last_sql: self.last_sql.clone(),
            results: self.results.clone(),
            pk_path: self.pk_path.clone(),
            more: self.more,
            result_selected: self.result_selected,
            marked: self.marked.iter().copied().collect(),
            doc_scroll: self.doc_scroll,
        }
    }

    /// The state on startup, showing the screen an earlier run saved while it refreshes.
    pub fn restore(snapshot: Snapshot) -> (Self, Vec<Effect>) {
        let (mut state, effects) = Self::new();
        let nodes = snapshot.accounts.into_iter().map(account_node).collect();
        state.accounts = Load::Loaded(nodes);
        state.tree_selected = snapshot.tree_selected;
        (state, effects)
    }
}

fn account_snapshot(node: &AccountNode) -> AccountSnapshot {
    let databases = match &node.databases {
        Some(Load::Loaded(databases)) => Some(
            databases
                .iter()
                .map(|database| DatabaseSnapshot {
                    name: database.name.clone(),
                    expanded: database.expanded,
                    containers: database.containers.clone(),
                })
                .collect(),
        ),
        // Databases that were loading or failed are listed again when opened
        Some(Load::Loading | Load::Failed(_)) | None => None,
    };
    AccountSnapshot {
        account: node.account.clone(),
        expanded: node.expanded,
        databases,
    }
}

fn account_node(snapshot: AccountSnapshot) -> AccountNode {
    let databases = snapshot.databases.map(|databases| {
        Load::Loaded(
            databases
                .into_iter()
                .map(|database| DatabaseNode {
                    name: database.name,
                    expanded: database.expanded,
                    containers: database.containers,
                })
                .collect(),
        )
    });
    AccountNode {
        account: snapshot.account,
        expanded: snapshot.expanded && databases.is_some(),
        databases,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::snapshot;

    /// The tree as indented text, one line per row.
    fn outline(state: &AppState) -> Vec<String> {
        state
            .tree_rows()
            .iter()
            .map(|row| format!("{}{}", "  ".repeat(row.depth), row.label))
            .collect()
    }

    #[test]
    fn reads_back_the_snapshot_it_wrote_as_json() {
        let snapshot = snapshot();

        let json = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
    }

    #[test]
    fn restores_the_tree_as_it_was_open_and_selected() {
        let (state, _) = AppState::restore(snapshot());

        assert_eq!(outline(&state), vec!["orders", "  shop", "    carts"]);
        assert_eq!(state.tree_selected, 2);
        assert_eq!(state.snapshot().accounts, snapshot().accounts);
    }

    #[test]
    fn restores_closed_databases_closed() {
        let mut saved = snapshot();
        saved.accounts[0].databases.as_mut().unwrap()[0].expanded = false;
        saved.tree_selected = 1;

        let (state, _) = AppState::restore(saved.clone());

        assert_eq!(outline(&state), vec!["orders", "  shop"]);
        assert_eq!(state.snapshot().accounts, saved.accounts);
    }

    #[test]
    fn restores_an_open_account_without_its_databases_closed_to_list_them_again() {
        let mut saved = snapshot();
        saved.accounts[0].databases = None;
        saved.tree_selected = 0;

        let (state, _) = AppState::restore(saved);

        assert_eq!(outline(&state), vec!["orders"]);
        assert_eq!(state.tree_rows()[0].expanded, Some(false));
    }
}
