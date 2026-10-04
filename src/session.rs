//! The screen as it was when termos quit, saved so the next run can show it at once.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::management::{Account, Container};
use crate::query::DEFAULT_QUERY;
use crate::state::{
    AccountNode, AppState, DatabaseNode, Effect, Focus, Load, Mode, OutputTab, Status, Target,
    refresh_documents,
};

/// The snapshot format, so a file from another version is left alone.
pub const VERSION: u32 = 1;

/// How many documents of the results are saved, to keep the file small.
const MAX_RESULTS: usize = 1000;

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
        // Settings are loaded afresh when opened, so the run starts where they were opened from
        let (mode, focus) = match self.mode {
            Mode::Settings if self.focus.settings_tab().is_some() => {
                (self.settings_from, Focus::Tree)
            }
            Mode::Settings => (self.settings_from, self.focus),
            mode => (mode, self.focus),
        };
        let results = &self.results[..self.results.len().min(MAX_RESULTS)];
        // A document left out is not shown, so the last one saved is selected instead
        let (result_selected, doc_scroll) = match results.len() {
            0 => (0, 0),
            len if self.result_selected >= len => (len - 1, 0),
            _ => (self.result_selected, self.doc_scroll),
        };
        Snapshot {
            version: VERSION,
            accounts,
            tree_selected: self.tree_selected,
            tree_hidden: self.tree_hidden,
            zoomed: self.zoomed,
            focus,
            mode,
            output_tab: self.output_tab,
            search: self.search.text().to_string(),
            search_cursor: self.search.cursor(),
            editor: self.editor.text(),
            editor_cursor: self.editor.cursor(),
            target: self.target.clone(),
            // Documents of the picked container were not listed yet, so they are listed from the top
            last_sql: if self.browse_stale {
                DEFAULT_QUERY.to_string()
            } else {
                self.last_sql.clone()
            },
            results: results.to_vec(),
            pk_path: self.pk_path.clone(),
            more: self.more || results.len() < self.results.len(),
            result_selected,
            marked: self
                .marked
                .iter()
                .copied()
                .filter(|index| *index < results.len())
                .collect(),
            doc_scroll,
        }
    }

    /// The state on startup, showing the screen an earlier run saved while it refreshes.
    pub fn restore(snapshot: Snapshot) -> (Self, Vec<Effect>) {
        let (mut state, mut effects) = Self::new();
        let nodes = snapshot.accounts.into_iter().map(account_node).collect();
        state.accounts = Load::Loaded(nodes);
        state.tree_selected = snapshot.tree_selected;
        state.tree_hidden = snapshot.tree_hidden;
        state.zoomed = snapshot.zoomed;
        state.focus = snapshot.focus;
        state.mode = snapshot.mode;
        state.output_tab = snapshot.output_tab;
        snapshot.search.chars().for_each(|c| state.search.insert(c));
        state.search.move_to(snapshot.search_cursor);
        state.last_sql = snapshot.last_sql;
        state.editor.set_text(&snapshot.editor);
        let (row, column) = snapshot.editor_cursor;
        state.editor.move_to(row, column);
        state.target = snapshot.target;
        state.results = snapshot.results;
        state.pk_path = snapshot.pk_path;
        state.more = snapshot.more;
        state.result_selected = snapshot.result_selected;
        state.marked = snapshot.marked.into_iter().collect();
        state.doc_scroll = snapshot.doc_scroll;
        if let Load::Loaded(nodes) = &state.accounts {
            let opened = nodes.iter().filter(|node| node.databases.is_some());
            effects.extend(opened.map(|node| Effect::LoadContainers(node.account.clone())));
        }
        effects.extend(refresh_documents(&mut state));
        state.status = Status::Info("Refreshing…".into());
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
    use crate::state::{Event, Msg, Origin, QueryResult, QueryStats, update};
    use crate::testing::{account, container, snapshot};
    use crossterm::event::{KeyCode, KeyEvent};
    use serde_json::json;
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

    #[test]
    fn restores_the_layout() {
        let (state, _) = AppState::restore(snapshot());

        assert!(state.tree_hidden);
        assert!(state.zoomed);
        assert_eq!(state.focus, Focus::Document);
        assert_eq!(state.mode, Mode::Browse);
        assert_eq!(state.output_tab, OutputTab::Stats);
    }

    #[test]
    fn saves_settings_mode_as_the_mode_it_came_from() {
        let (mut state, _) = AppState::restore(snapshot());
        state.mode = Mode::Settings;
        state.settings_from = Mode::Query;
        state.focus = Focus::IndexingPolicy;

        let saved = state.snapshot();

        assert_eq!(saved.mode, Mode::Query);
        assert_eq!(saved.focus, Focus::Tree);
    }

    #[test]
    fn restores_the_search_and_the_query_editor_without_its_output() {
        let (state, _) = AppState::restore(snapshot());

        assert_eq!(state.search.text(), "c.qty > 1");
        assert_eq!(state.search.cursor(), 3);
        assert_eq!(state.last_sql, "SELECT * FROM c WHERE c.qty > 1");
        assert_eq!(
            state.editor.text(),
            "SELECT *
FROM c"
        );
        assert_eq!(state.editor.cursor(), (1, 2));
        assert!(state.output.results.is_empty());
    }

    #[test]
    fn restores_the_documents_found_with_the_selected_one() {
        let (state, _) = AppState::restore(snapshot());

        assert_eq!(state.target, snapshot().target);
        assert_eq!(state.results, snapshot().results);
        assert_eq!(state.pk_path, "/tenantId");
        assert!(state.more);
        assert_eq!(state.result_selected, 0);
        assert_eq!(state.marked.iter().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(state.doc_scroll, 4);
    }

    #[test]
    fn saves_at_most_a_thousand_documents() {
        let (mut state, _) = AppState::restore(snapshot());
        state.results = (0..1500)
            .map(|i| json!({ "id": format!("c-{i}") }))
            .collect();
        state.marked = [10, 1200].into();

        let saved = state.snapshot();

        assert_eq!(saved.results.len(), 1000);
        assert!(saved.more);
        assert_eq!(saved.marked, vec![10]);
    }

    #[test]
    fn a_selected_document_past_the_saved_ones_selects_the_last() {
        let (mut state, _) = AppState::restore(snapshot());
        state.results = (0..1500)
            .map(|i| json!({ "id": format!("c-{i}") }))
            .collect();
        state.result_selected = 1200;
        state.doc_scroll = 7;

        let saved = state.snapshot();

        assert_eq!(saved.result_selected, 999);
        assert_eq!(saved.doc_scroll, 0);
    }

    #[test]
    fn refreshes_the_accounts_the_open_containers_and_the_documents() {
        let (state, effects) = AppState::restore(snapshot());

        assert_eq!(
            effects,
            vec![
                Effect::LoadAccounts,
                Effect::LoadContainers(account("orders")),
                Effect::Query {
                    id: 1,
                    target: snapshot().target.unwrap(),
                    sql: snapshot().last_sql,
                    origin: Origin::Browse,
                },
            ]
        );
        assert_eq!(state.status, Status::Info("Refreshing…".into()));
        assert!(state.more);
    }

    #[test]
    fn without_a_container_only_the_accounts_refresh() {
        let mut saved = snapshot();
        saved.accounts[0].databases = None;
        saved.target = None;

        let (_, effects) = AppState::restore(saved);

        assert_eq!(effects, vec![Effect::LoadAccounts]);
    }

    #[test]
    fn documents_not_listed_yet_for_the_picked_container_list_from_the_top() {
        let (mut state, _) = AppState::restore(snapshot());
        state.browse_stale = true;
        state.results.clear();

        let saved = state.snapshot();

        assert_eq!(saved.last_sql, DEFAULT_QUERY);
    }

    #[test]
    fn more_documents_load_once_the_refreshed_ones_are_in() {
        let mut saved = snapshot();
        saved.focus = Focus::Results;
        saved.zoomed = false;
        let (mut state, _) = AppState::restore(saved);

        let effects = update(&mut state, Event::Key(KeyEvent::from(KeyCode::Down)));

        assert_eq!(effects, vec![]);
        assert!(state.more);
    }

    #[test]
    fn refreshed_containers_keep_closed_databases_closed_and_the_selection() {
        let mut saved = snapshot();
        let databases = saved.accounts[0].databases.as_mut().unwrap();
        databases.insert(
            0,
            DatabaseSnapshot {
                name: "audit".into(),
                expanded: false,
                containers: vec![container("events", "/day")],
            },
        );
        saved.tree_selected = 3;
        let (mut state, _) = AppState::restore(saved);
        assert_eq!(state.tree_rows()[3].label, "carts");

        update(
            &mut state,
            Event::Msg(Msg::ContainersLoaded {
                account: "orders".into(),
                result: Ok(vec![
                    ("billing".into(), container("ledger", "/year")),
                    ("audit".into(), container("events", "/day")),
                    ("shop".into(), container("carts", "/tenantId")),
                ]),
            }),
        );

        assert_eq!(
            outline(&state),
            vec![
                "orders",
                "  billing",
                "    ledger",
                "  audit",
                "  shop",
                "    carts"
            ]
        );
        assert_eq!(state.tree_rows()[state.tree_selected].label, "carts");
    }

    fn cart(id: &str) -> Value {
        json!({ "id": id, "tenantId": "contoso" })
    }

    /// A restored session showing three carts, the second selected and the third marked.
    fn three_carts() -> AppState {
        let mut saved = snapshot();
        saved.results = vec![cart("c-1"), cart("c-2"), cart("c-3")];
        saved.result_selected = 1;
        saved.marked = vec![2];
        let (state, _) = AppState::restore(saved);
        state
    }

    fn found(docs: Vec<Value>, more: bool) -> Result<QueryResult, String> {
        Ok(QueryResult {
            docs,
            more,
            pk_path: "/tenantId".into(),
            elapsed: Duration::from_millis(5),
            stats: QueryStats::default(),
        })
    }

    #[test]
    fn refreshed_documents_keep_the_selected_and_marked_ones() {
        let mut state = three_carts();
        let id = state.query_id;

        let refreshed = vec![cart("c-0"), cart("c-1"), cart("c-2"), cart("c-3")];
        update(
            &mut state,
            Event::Msg(Msg::QueryDone {
                id,
                result: found(refreshed.clone(), false),
            }),
        );

        assert_eq!(state.results, refreshed);
        assert_eq!(state.result_selected, 2);
        assert_eq!(state.marked.iter().copied().collect::<Vec<_>>(), vec![3]);
        assert_eq!(state.doc_scroll, 4);
        assert!(!state.more);
        assert_eq!(state.refreshing_query, None);
    }

    #[test]
    fn a_selected_document_that_is_gone_selects_the_one_in_its_place() {
        let mut state = three_carts();
        let id = state.query_id;

        update(
            &mut state,
            Event::Msg(Msg::QueryDone {
                id,
                result: found(vec![cart("c-1"), cart("c-3")], false),
            }),
        );

        assert_eq!(state.result_selected, 1);
        assert_eq!(state.marked.iter().copied().collect::<Vec<_>>(), vec![1]);
        assert_eq!(state.doc_scroll, 0);
    }
}
