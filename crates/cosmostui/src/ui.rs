//! Draws the state on the terminal.

use cosmos_core::partition::{display_value, value_at_path};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};

use crate::json::highlight_json;
use crate::state::{AppState, Focus, Status};

/// Where each part of the screen goes.
struct Panes {
    tree: Rect,
    search: Rect,
    results: Rect,
    document: Rect,
    status: Rect,
}

fn panes(area: Rect) -> Panes {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let [tree, right] =
        Layout::horizontal([Constraint::Percentage(25), Constraint::Fill(1)]).areas(main);
    let [search, body] =
        Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(right);
    let [results, document] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(body);
    Panes {
        tree,
        search,
        results,
        document,
        status,
    }
}

/// How many document lines the document pane shows on a terminal of this size.
pub fn document_height(size: Size) -> u16 {
    let document = panes(Rect::from((Position::ORIGIN, size))).document;
    // Less the top and bottom borders
    document.height.saturating_sub(2)
}

/// Draws the tree, search bar, results, document and status line.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let panes = panes(frame.area());
    draw_tree(frame, state, panes.tree);
    draw_search(frame, state, panes.search);
    draw_results(frame, state, panes.results);
    draw_document(frame, state, panes.document);
    draw_status(frame, state, panes.status);
    if state.show_help {
        draw_help(frame);
    }
}

fn draw_tree(frame: &mut Frame, state: &AppState, area: Rect) {
    let items = state.tree_rows().into_iter().map(|row| {
        let marker = match row.expanded {
            Some(true) => "▾ ",
            Some(false) => "▸ ",
            None => "  ",
        };
        let indent = "  ".repeat(row.depth);
        ListItem::new(format!("{indent}{marker}{}", row.label))
    });
    let list = List::new(items)
        .block(pane("Accounts", state, Focus::Tree))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default().with_selected(Some(state.tree_selected));
    frame.render_stateful_widget(list, area, &mut list_state);
}

/// The results title, with how many documents are shown and whether more can load.
fn results_title(state: &AppState) -> String {
    let count = state.results.len();
    if state.loading_more {
        format!("Results ({count}, loading…)")
    } else if state.more {
        format!("Results ({count}, more ↓)")
    } else if state.target.is_some() {
        format!("Results ({count})")
    } else {
        "Results".to_string()
    }
}

fn draw_results(frame: &mut Frame, state: &AppState, area: Rect) {
    let header = Row::new([Cell::from("id"), Cell::from(state.pk_path.as_str())])
        .style(Style::new().add_modifier(Modifier::BOLD));
    let rows = state.results.iter().map(|doc| {
        Row::new([
            display_value(doc.get("id")),
            display_value(value_at_path(doc, &state.pk_path)),
        ])
    });
    let table = Table::new(rows, [Constraint::Fill(1), Constraint::Fill(1)])
        .header(header)
        .block(pane(results_title(state), state, Focus::Results))
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut table_state = TableState::default().with_selected(Some(state.result_selected));
    frame.render_stateful_widget(table, area, &mut table_state);
}

fn draw_search(frame: &mut Frame, state: &AppState, area: Rect) {
    let input = Paragraph::new(state.search.text()).block(pane("Search", state, Focus::Search));
    frame.render_widget(input, area);
    if state.focus == Focus::Search {
        let cursor = u16::try_from(state.search.cursor()).unwrap_or(u16::MAX);
        frame.set_cursor_position((area.x + 1 + cursor, area.y + 1));
    }
}

fn draw_document(frame: &mut Frame, state: &AppState, area: Rect) {
    // The pane may have grown since the document was scrolled
    let height = area.height.saturating_sub(2);
    let scroll = state
        .doc_scroll
        .min(state.document_lines().saturating_sub(height));
    let lines = state
        .selected_document()
        .map(highlight_json)
        .unwrap_or_default();
    let document = Paragraph::new(lines)
        .block(pane("Document", state, Focus::Document))
        .scroll((scroll, 0));
    frame.render_widget(document, area);
}

/// A bordered pane, highlighted when it has focus.
fn pane<'a>(title: impl Into<Line<'a>>, state: &AppState, focus: Focus) -> Block<'a> {
    let colour = if state.focus == focus {
        Color::Cyan
    } else {
        Color::DarkGray
    };
    Block::bordered()
        .title(title)
        .border_style(Style::new().fg(colour))
}

fn draw_status(frame: &mut Frame, state: &AppState, area: Rect) {
    let hint = "Tab next pane · / search · ? help · q quit";
    let width = u16::try_from(hint.chars().count()).unwrap_or(u16::MAX);
    let [message, keys] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    let line = match &state.status {
        Status::Info(text) => Line::raw(text.as_str()),
        Status::Error(text) => Line::styled(format!("error: {text}"), Style::new().fg(Color::Red)),
    };
    frame.render_widget(Paragraph::new(line), message);
    let hint = Line::styled(hint, Style::new().fg(Color::DarkGray));
    frame.render_widget(Paragraph::new(hint), keys);
}

/// What each key does, shown with `?`.
const HELP: [(&str, &str); 12] = [
    ("Tab / Shift-Tab", "Next pane / previous pane"),
    ("/", "Search: type SQL, a clause or a condition"),
    ("Enter", "Open a node, run the search, show a document"),
    ("↑ ↓  j k", "Move, scroll, or load more at the end"),
    ("→ ←  l h", "Open or close a node"),
    ("PgUp PgDn Home End", "Scroll the document"),
    ("Esc", "Leave the search bar"),
    ("r", "Run the query again"),
    ("?", "Show this help"),
    ("q, Ctrl-C", "Quit"),
    ("", ""),
    ("", "Press any key to close"),
];

fn draw_help(frame: &mut Frame) {
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, action)| {
            Line::from(vec![
                Span::styled(format!("{keys:<16}"), Style::new().fg(Color::Cyan)),
                Span::raw(*action),
            ])
        })
        .collect();
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2;
    let area = frame
        .area()
        .centered(Constraint::Length(64), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title("Keys")),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Event, Focus, Msg, QueryResult, update};
    use cosmos_core::testing::{account, container};
    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Size;

    use serde_json::json;
    use std::time::Duration;

    /// Draws the state on a small terminal and returns its lines.
    fn screen(state: &AppState) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let buffer = terminal.backend().buffer();
        buffer
            .content
            .chunks(buffer.area.width as usize)
            .map(|row| row.iter().map(|cell| cell.symbol()).collect())
            .collect()
    }

    fn shows(screen: &[String], text: &str) -> bool {
        screen.iter().any(|line| line.contains(text))
    }

    #[test]
    fn draws_every_pane_and_the_status_line() {
        let (state, _) = AppState::new();

        let screen = screen(&state);

        for title in [
            "Accounts",
            "Search",
            "Results",
            "Document",
            "Loading accounts…",
        ] {
            assert!(
                shows(&screen, title),
                "{title} missing from\n{}",
                screen.join("\n")
            );
        }
    }

    fn send(state: &mut AppState, msg: Msg) {
        update(state, Event::Msg(msg));
    }

    fn press(state: &mut AppState, code: KeyCode) {
        update(state, Event::Key(KeyEvent::from(code)));
    }

    /// Orders expanded with its carts container open, and inventory collapsed.
    fn browsing() -> AppState {
        let (mut state, _) = AppState::new();
        send(
            &mut state,
            Msg::AccountsLoaded(Ok(vec![account("orders"), account("inventory")])),
        );
        press(&mut state, KeyCode::Enter);
        let containers = vec![("shop".to_string(), container("carts", "/tenantId"))];
        send(
            &mut state,
            Msg::ContainersLoaded {
                account: "orders".into(),
                result: Ok(containers),
            },
        );
        state
    }

    #[test]
    fn draws_the_tree_with_markers_on_nodes_that_open() {
        let screen = screen(&browsing());

        for row in ["▾ orders", "  ▾ shop", "      carts", "▸ inventory"] {
            assert!(
                shows(&screen, row),
                "{row} missing from\n{}",
                screen.join("\n")
            );
        }
    }

    /// Browsing with two carts found in orders/shop/carts.
    fn with_results() -> AppState {
        let mut state = browsing();
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Enter);
        let docs = vec![
            json!({ "id": "c-1", "tenantId": "contoso" }),
            json!({ "id": "c-2", "tenantId": "fabrikam" }),
        ];
        let result = QueryResult {
            docs,
            more: false,
            pk_path: "/tenantId".into(),
            elapsed: Duration::from_millis(40),
        };
        send(
            &mut state,
            Msg::QueryDone {
                id: 1,
                result: Ok(result),
            },
        );
        state
    }

    fn cells(screen: &[String], words: &[&str]) -> bool {
        screen.iter().any(|line| {
            let found: Vec<&str> = line.split([' ', '│']).filter(|w| !w.is_empty()).collect();
            found.windows(words.len()).any(|window| window == words)
        })
    }

    #[test]
    fn draws_the_id_and_partition_key_of_each_result() {
        let screen = screen(&with_results());

        assert!(
            cells(&screen, &["id", "/tenantId"]),
            "{}",
            screen.join("\n")
        );
        assert!(cells(&screen, &["c-1", "contoso"]), "{}", screen.join("\n"));
        assert!(
            cells(&screen, &["c-2", "fabrikam"]),
            "{}",
            screen.join("\n")
        );
    }

    #[test]
    fn draws_the_selected_document_scrolled_to_its_place() {
        let mut state = with_results();
        let mut long = serde_json::Map::new();
        long.insert("id".into(), json!("c-2"));
        for i in 0..30 {
            long.insert(format!("x{i:02}"), json!(i));
        }
        state.results[1] = serde_json::Value::Object(long);
        state.doc_height = document_height(Size::new(100, 20));
        state.focus = Focus::Results;
        press(&mut state, KeyCode::Down);

        let screen = screen(&state);
        assert!(shows(&screen, r#""id": "c-2","#), "{}", screen.join("\n"));

        state.focus = Focus::Document;
        press(&mut state, KeyCode::Down);
        press(&mut state, KeyCode::Down);
        let screen = self::screen(&state);
        assert!(!shows(&screen, r#""id": "c-2","#), "{}", screen.join("\n"));
        assert!(shows(&screen, r#""x00": 0"#));
    }

    #[test]
    fn draws_the_search_text() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('/'));
        for c in "c.qty > 1".chars() {
            press(&mut state, KeyCode::Char(c));
        }

        assert!(shows(&screen(&state), "c.qty > 1"));
    }

    #[test]
    fn draws_the_key_help_over_the_panes() {
        let mut state = browsing();
        press(&mut state, KeyCode::Char('?'));

        let screen = screen(&state);

        for text in ["Keys", "Tab", "Next pane", "Quit"] {
            assert!(
                shows(&screen, text),
                "{text} missing from\n{}",
                screen.join("\n")
            );
        }
    }

    #[test]
    fn the_status_line_hints_at_the_help() {
        assert!(shows(&screen(&browsing()), "? help"));
    }

    #[test]
    fn the_results_title_counts_documents_and_says_when_more_can_load() {
        let mut state = with_results();
        assert!(shows(&screen(&state), "Results (2)"));

        state.more = true;
        assert!(shows(&screen(&state), "Results (2, more ↓)"));

        state.loading_more = true;
        assert!(shows(&screen(&state), "Results (2, loading…)"));
    }

    #[test]
    fn a_document_scrolled_to_the_end_shows_its_last_line_at_the_bottom() {
        let mut state = with_results();
        let fields = (0..40).map(|i| (format!("f{i:02}"), json!(i)));
        state.results[0] = serde_json::Value::Object(fields.collect());
        state.doc_height = document_height(Size::new(100, 20));
        state.focus = Focus::Document;

        press(&mut state, KeyCode::End);

        let screen = screen(&state);
        let above_bottom = &screen[screen.len() - 4];
        assert!(
            above_bottom.contains(r#""f39": 39"#),
            "{}",
            screen.join("\n")
        );
        assert!(!shows(&screen, r#""f00""#), "{}", screen.join("\n"));
    }

    #[test]
    fn drawing_never_scrolls_past_the_end_of_the_document() {
        let mut state = with_results();
        state.doc_scroll = 50;

        assert!(shows(&screen(&state), r#""id": "c-1","#));
    }
}
