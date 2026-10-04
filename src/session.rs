//! The screen as it was when termos quit, saved so the next run can show it at once.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::management::{Account, Container};
use crate::state::{Focus, Mode, OutputTab, Target};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::snapshot;

    #[test]
    fn reads_back_the_snapshot_it_wrote_as_json() {
        let snapshot = snapshot();

        let json = serde_json::to_string(&snapshot).unwrap();

        assert_eq!(serde_json::from_str::<Snapshot>(&json).unwrap(), snapshot);
    }
}
