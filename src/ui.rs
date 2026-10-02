//! Draws the state on the terminal.

use crate::partition::{display_value, value_at_path};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};

use crate::json::highlight_json;
use crate::query::DEFAULT_QUERY;
use crate::state::{AppState, Focus, Selection, Status};

/// Room for the query the search bar completes, and a space after it.
const QUERY_WIDTH: u16 = DEFAULT_QUERY.len() as u16 + 1;

/// Where each part of the screen goes.
struct Panes {
    tree: Rect,
    query: Rect,
    search: Rect,
    results: Rect,
    document: Rect,
    status: Rect,
}

/// Lays out the panes, leaving all the room to the zoomed pane if there is one.
fn panes(area: Rect, tree_hidden: bool, zoomed: Option<Focus>) -> Panes {
    let [main, status] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(area);
    let tree_width = if tree_hidden {
        Constraint::Length(0)
    } else {
        Constraint::Percentage(25)
    };
    let [tree, right] = Layout::horizontal([tree_width, Constraint::Fill(1)]).areas(main);
    let [search_row, body] =
        Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(right);
    let [query, search] =
        Layout::horizontal([Constraint::Length(QUERY_WIDTH), Constraint::Fill(1)])
            .areas(search_row);
    let [results, document] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(body);
    let mut panes = Panes {
        tree,
        query,
        search,
        results,
        document,
        status,
    };
    if let Some(focus) = zoomed {
        for (pane, area) in [
            (Focus::Tree, &mut panes.tree),
            (Focus::Search, &mut panes.search),
            (Focus::Results, &mut panes.results),
            (Focus::Document, &mut panes.document),
        ] {
            *area = if pane == focus { main } else { Rect::default() };
        }
        panes.query = Rect::default();
    }
    panes
}

/// How many tree rows the tree pane shows on a terminal of this size.
pub fn tree_height(size: Size) -> u16 {
    let tree = panes(Rect::from((Position::ORIGIN, size)), false, None).tree;
    // Less the top and bottom borders
    tree.height.saturating_sub(2)
}

/// The pane at a point on a terminal of this size, with the point inside its
/// borders, or `None` for the point when it is on a border.
pub fn pane_at(
    size: Size,
    at: Position,
    tree_hidden: bool,
    zoomed: Option<Focus>,
) -> Option<(Focus, Option<Position>)> {
    let panes = panes(Rect::from((Position::ORIGIN, size)), tree_hidden, zoomed);
    let (focus, area) = [
        (Focus::Tree, panes.tree),
        (Focus::Search, panes.search),
        (Focus::Results, panes.results),
        (Focus::Document, panes.document),
    ]
    .into_iter()
    .find(|(_, area)| area.contains(at))?;
    let inner = area.inner(Margin::new(1, 1));
    let inside = inner
        .contains(at)
        .then(|| Position::new(at.x - inner.x, at.y - inner.y));
    Some((focus, inside))
}

/// How many document lines the document pane shows on a terminal of this size,
/// or none when another pane is zoomed.
pub fn document_height(size: Size, zoomed: Option<Focus>) -> u16 {
    let document = panes(Rect::from((Position::ORIGIN, size)), false, zoomed).document;
    // Less the top and bottom borders
    document.height.saturating_sub(2)
}

/// How many documents the results table shows on a terminal of this size,
/// or none when another pane is zoomed.
pub fn results_height(size: Size, zoomed: Option<Focus>) -> u16 {
    let results = panes(Rect::from((Position::ORIGIN, size)), false, zoomed).results;
    // Less the borders and the header row
    results.height.saturating_sub(3)
}

/// Draws the tree, search bar, results, document and status line.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let panes = panes(frame.area(), state.tree_hidden, state.zoomed_pane());
    draw_tree(frame, state, panes.tree);
    draw_query(frame, panes.query);
    draw_search(frame, state, panes.search);
    draw_results(frame, state, panes.results);
    draw_document(frame, state, panes.document);
    draw_status(frame, state, panes.status);
    if state.show_help {
        draw_help(frame);
    }
    if state.confirm_delete {
        draw_confirm_delete(frame, state);
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
    if state.target.is_none() {
        return "Results".to_string();
    }
    let mut details = count.to_string();
    if !state.marked.is_empty() {
        details.push_str(&format!(", {} marked", state.marked.len()));
    }
    if state.loading_more {
        details.push_str(", loading…");
    } else if state.more {
        details.push_str(", more ↓");
    }
    format!("Results ({details})")
}

fn draw_results(frame: &mut Frame, state: &AppState, area: Rect) {
    let header = Row::new([Cell::from("id"), Cell::from(state.pk_path.as_str())])
        .style(Style::new().add_modifier(Modifier::BOLD));
    let rows = state.results.iter().enumerate().map(|(index, doc)| {
        let id = display_value(doc.get("id"));
        let id = if state.marked.contains(&index) {
            format!("● {id}")
        } else {
            id
        };
        Row::new([id, display_value(value_at_path(doc, &state.pk_path))])
    });
    let table = Table::new(rows, [Constraint::Fill(1), Constraint::Fill(1)])
        .header(header)
        .block(pane(results_title(state), state, Focus::Results))
        .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut table_state = TableState::default().with_selected(Some(state.result_selected));
    frame.render_stateful_widget(table, area, &mut table_state);
}

/// Shows the query the search bar text completes, level with the text.
fn draw_query(frame: &mut Frame, area: Rect) {
    let middle = Rect {
        y: area.y + 1,
        height: 1,
        ..area
    };
    frame.render_widget(Paragraph::new(DEFAULT_QUERY), middle.intersection(area));
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
    let widths: Vec<usize> = lines.iter().map(Line::width).collect();
    let document = Paragraph::new(lines)
        .block(pane("Document", state, Focus::Document))
        .scroll((scroll, 0));
    frame.render_widget(document, area);
    if let Some(selection) = state.doc_selection.filter(|s| s.anchor != s.head) {
        draw_selection(frame, selection, &widths, scroll, area);
    }
}

/// Reverses the picked text of the document, as far as each line reaches.
fn draw_selection(
    frame: &mut Frame,
    selection: Selection,
    widths: &[usize],
    scroll: u16,
    area: Rect,
) {
    let inner = area.inner(Margin::new(1, 1));
    let (start, end) = selection.range();
    for row in 0..inner.height {
        let line = usize::from(scroll) + usize::from(row);
        if line < start.line || line > end.line {
            continue;
        }
        let width = widths.get(line).copied().unwrap_or(0);
        let from = if line == start.line { start.column } else { 0 };
        let to = if line == end.line {
            (end.column + 1).min(width)
        } else {
            width
        };
        for column in from..to.min(usize::from(inner.width)) {
            let x = inner.x + u16::try_from(column).unwrap_or(u16::MAX);
            if let Some(cell) = frame.buffer_mut().cell_mut((x, inner.y + row)) {
                cell.modifier.insert(Modifier::REVERSED);
            }
        }
    }
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
    let hint = if state.zoomed_pane().is_some() {
        "z unzoom · Tab next pane · / search · ? help · q quit"
    } else {
        "Tab next pane · / search · ? help · q quit"
    };
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
const HELP: [(&str, &str); 20] = [
    ("Tab / Shift-Tab", "Next pane / previous pane"),
    ("Ctrl-B", "Hide or show the accounts"),
    ("z", "Zoom the pane, or show every pane again"),
    ("/", "Search: type SQL, a clause or a condition"),
    ("Enter", "Open a node, search, go to the documents"),
    ("↑ ↓  j k", "Move, scroll, or load more at the end"),
    ("→ ←  l h", "Open or close a node"),
    ("PgUp PgDn Home End", "Scroll the results or document"),
    ("Ctrl-U Ctrl-D", "Scroll half a page up or down"),
    ("g g  G", "Jump to the first or last"),
    ("Click, again", "Pick a pane or row, then open it"),
    ("Wheel", "Scroll the pane under the mouse"),
    ("Drag, y", "Pick text in the document, copy it"),
    ("Esc", "Leave the search bar, drop the picked text"),
    ("r", "Run the query again"),
    (
        "Space, Ctrl-A",
        "Mark a result, mark all results (Esc clears)",
    ),
    (
        "d",
        "Delete the marked or selected documents, after confirming",
    ),
    ("?", "Show this help"),
    ("q, Ctrl-C", "Quit"),
    ("", "Press any key to close"),
];

fn draw_help(frame: &mut Frame) {
    let lines: Vec<Line> = HELP
        .iter()
        .map(|(keys, action)| {
            Line::from(vec![
                Span::styled(format!("{keys:<20}"), Style::new().fg(Color::Cyan)),
                Span::raw(*action),
            ])
        })
        .collect();
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2;
    let area = frame
        .area()
        .centered(Constraint::Length(80), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(Block::bordered().title("Keys")),
        area,
    );
}

/// A dialog asking whether to delete the selected document.
fn draw_confirm_delete(frame: &mut Frame, state: &AppState) {
    let (title, question) = match state.marked.len() {
        0 => {
            let id = state
                .selected_document()
                .map(|doc| display_value(doc.get("id")))
                .unwrap_or_default();
            ("Delete document", format!("Delete {id}?"))
        }
        1 => ("Delete document", "Delete 1 marked document?".to_string()),
        count => ("Delete documents", format!("Delete {count} documents?")),
    };
    let lines = vec![
        Line::raw(question),
        Line::raw("This cannot be undone."),
        Line::raw(""),
        Line::from(vec![
            Span::styled("y / Enter", Style::new().fg(Color::Red)),
            Span::raw(" delete   "),
            Span::styled("Esc", Style::new().fg(Color::Cyan)),
            Span::raw(" or any other key cancel"),
        ]),
    ];
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2;
    let area = frame
        .area()
        .centered(Constraint::Length(52), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(Color::Red)),
        ),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{DocPoint, Event, Focus, Msg, QueryResult, update};
    use crate::testing::{account, container};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Size;

    use serde_json::json;
    use std::time::Duration;

    /// Draws the state on a small terminal and returns its lines.
    fn screen(state: &AppState) -> Vec<String> {
        screen_with_height(state, 20)
    }

    fn screen_with_height(state: &AppState, height: u16) -> Vec<String> {
        let mut terminal = Terminal::new(TestBackend::new(100, height)).unwrap();
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

    #[test]
    fn a_hidden_tree_leaves_its_room_to_the_other_panes() {
        let (mut state, _) = AppState::new();
        state.tree_hidden = true;

        let screen = screen(&state);

        assert!(!shows(&screen, "Accounts"), "{}", screen.join("\n"));
        // The query before the search bar starts at the left edge
        assert!(
            screen[1].starts_with("SELECT * FROM c"),
            "{}",
            screen.join("\n")
        );
    }

    #[test]
    fn a_zoomed_pane_takes_the_whole_screen_above_the_status_line() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Document;
        state.zoomed = true;

        let screen = screen(&state);

        for title in ["Accounts", "Search", "Results"] {
            assert!(!shows(&screen, title), "{}", screen.join("\n"));
        }
        assert!(screen[0].starts_with("┌Document"), "{}", screen.join("\n"));
        assert!(screen[18].starts_with("└"), "{}", screen.join("\n"));
        assert!(screen[0].ends_with("┐"), "{}", screen.join("\n"));
        assert!(shows(&screen, "Loading accounts…"), "{}", screen.join("\n"));
    }

    #[test]
    fn the_status_line_tells_how_to_leave_the_zoom() {
        let (mut state, _) = AppState::new();
        state.focus = Focus::Document;
        assert!(!shows(&screen(&state), "z unzoom"));

        state.zoomed = true;
        assert!(shows(&screen(&state), "z unzoom · Tab next pane"));
    }

    fn send(state: &mut AppState, msg: Msg) {
        update(state, Event::Msg(msg));
    }

    fn press(state: &mut AppState, code: KeyCode) {
        update(state, Event::Key(KeyEvent::from(code)));
    }

    #[test]
    fn the_results_table_shows_the_rows_between_its_borders_and_header() {
        // 20 rows less the status line, search bar, borders and header
        assert_eq!(results_height(Size::new(100, 20), None), 13);
    }

    #[test]
    fn the_tree_shows_the_rows_between_its_borders() {
        // 20 rows less the status line and borders
        assert_eq!(tree_height(Size::new(100, 20)), 17);
    }

    #[test]
    fn a_zoomed_pane_shows_the_rows_down_to_the_status_line() {
        let size = Size::new(100, 20);

        // 20 rows less the status line and borders
        assert_eq!(document_height(size, Some(Focus::Document)), 17);
        // and the header
        assert_eq!(results_height(size, Some(Focus::Results)), 16);
    }

    #[test]
    fn finds_the_pane_at_a_point_and_where_the_point_is_inside_it() {
        let size = Size::new(100, 20);
        let at = |x, y| pane_at(size, Position::new(x, y), false, None);

        assert_eq!(at(3, 4), Some((Focus::Tree, Some(Position::new(2, 3)))));
        assert_eq!(at(27, 1), None);
        assert_eq!(at(43, 1), Some((Focus::Search, Some(Position::new(1, 0)))));
        assert_eq!(at(30, 5).map(|(pane, _)| pane), Some(Focus::Results));
        assert_eq!(at(90, 5).map(|(pane, _)| pane), Some(Focus::Document));
    }

    #[test]
    fn finds_the_panes_that_take_the_room_of_a_hidden_tree() {
        let size = Size::new(100, 20);
        let at = |x, y| pane_at(size, Position::new(x, y), true, None);

        assert_eq!(at(3, 1), None);
        assert_eq!(at(19, 1), Some((Focus::Search, Some(Position::new(2, 0)))));
        assert_eq!(at(3, 5).map(|(pane, _)| pane), Some(Focus::Results));
    }

    #[test]
    fn finds_the_zoomed_pane_wherever_it_reaches() {
        let size = Size::new(100, 20);
        let at = |x, y| pane_at(size, Position::new(x, y), false, Some(Focus::Document));

        assert_eq!(at(3, 1), Some((Focus::Document, Some(Position::new(2, 0)))));
        assert_eq!(at(90, 17).map(|(pane, _)| pane), Some(Focus::Document));
        assert_eq!(at(5, 19), None);
    }

    #[test]
    fn a_point_on_a_border_is_in_the_pane_but_not_inside_it() {
        let size = Size::new(100, 20);

        assert_eq!(
            pane_at(size, Position::new(0, 5), false, None),
            Some((Focus::Tree, None))
        );
        assert_eq!(
            pane_at(size, Position::new(30, 3), false, None),
            Some((Focus::Results, None))
        );
    }

    #[test]
    fn the_status_line_is_in_no_pane() {
        assert_eq!(
            pane_at(Size::new(100, 20), Position::new(5, 19), false, None),
            None
        );
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
        state.doc_height = document_height(Size::new(100, 20), None);
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
    fn the_search_bar_follows_the_query_it_completes() {
        let (mut state, _) = AppState::new();
        state.tree_hidden = true;

        let screen = screen(&state);

        assert!(
            screen[0].starts_with("                ┌Search"),
            "{}",
            screen.join("\n")
        );
        assert!(
            screen[1].starts_with("SELECT * FROM c │"),
            "{}",
            screen.join("\n")
        );
    }

    #[test]
    fn draws_the_key_help_over_the_panes() {
        let mut state = browsing();
        press(&mut state, KeyCode::Char('?'));

        let screen = screen_with_height(&state, 24);

        for text in [
            "Keys",
            "Tab",
            "Next pane",
            "Ctrl-B",
            "Hide or show the accounts",
            "Zoom the pane, or show every pane again",
            "Pick text in the document, copy it",
            "Quit",
        ] {
            assert!(
                shows(&screen, text),
                "{text} missing from\n{}",
                screen.join("\n")
            );
        }
    }

    #[test]
    fn marks_the_results_picked_to_delete_together() {
        let mut state = with_results();
        state.focus = Focus::Results;
        press(&mut state, KeyCode::Char(' '));

        let screen = screen(&state);

        assert!(shows(&screen, "● c-1"), "{}", screen.join("\n"));
        assert!(!shows(&screen, "● c-2"), "{}", screen.join("\n"));
        assert!(shows(&screen, "1 marked"), "{}", screen.join("\n"));
    }

    #[test]
    fn the_delete_dialog_counts_the_marked_documents() {
        let mut state = with_results();
        state.focus = Focus::Results;
        let ctrl_a = KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL);
        update(&mut state, Event::Key(ctrl_a));
        press(&mut state, KeyCode::Char('d'));

        let screen = screen(&state);

        for text in ["Delete documents", "Delete 2 documents?"] {
            assert!(
                shows(&screen, text),
                "{text} missing from\n{}",
                screen.join("\n")
            );
        }
    }

    #[test]
    fn draws_a_dialog_asking_to_confirm_the_delete_over_the_panes() {
        let mut state = with_results();
        state.focus = Focus::Results;
        press(&mut state, KeyCode::Char('d'));

        let screen = screen(&state);

        for text in ["Delete document", "c-1", "y / Enter", "Esc"] {
            assert!(
                shows(&screen, text),
                "{text} missing from\n{}",
                screen.join("\n")
            );
        }
    }

    #[test]
    fn the_key_help_lists_the_delete_key() {
        let mut state = browsing();
        press(&mut state, KeyCode::Char('?'));

        assert!(shows(&screen(&state), "Delete the marked or selected"));
    }

    #[test]
    fn the_key_help_is_wide_enough_for_its_longest_line() {
        let mut state = browsing();
        press(&mut state, KeyCode::Char('?'));

        let screen = screen_with_height(&state, 24);

        for (_, action) in HELP {
            assert!(
                shows(&screen, action),
                "{action} cut off in\n{}",
                screen.join("\n")
            );
        }
    }

    #[test]
    fn the_key_help_lists_the_marking_keys() {
        let mut state = browsing();
        press(&mut state, KeyCode::Char('?'));

        assert!(shows(&screen(&state), "Space, Ctrl-A"));
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
        state.doc_height = document_height(Size::new(100, 20), None);
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
    fn draws_the_picked_document_text_reversed() {
        let mut state = with_results();
        let point = |line, column| DocPoint { line, column };
        state.doc_selection = Some(Selection {
            anchor: point(1, 2),
            head: point(1, 5),
        });

        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();

        let row = screen(&state)
            .iter()
            .position(|line| line.contains(r#""id": "c-1""#))
            .unwrap();
        let buffer = terminal.backend().buffer();
        let row = u16::try_from(row).unwrap();
        let document = panes(buffer.area, false, None).document;
        let reversed: String = (document.x..document.right())
            .map(|x| &buffer[(x, row)])
            .filter(|cell| cell.modifier.contains(Modifier::REVERSED))
            .map(|cell| cell.symbol())
            .collect();
        assert_eq!(reversed, r#""id""#);
    }

    #[test]
    fn drawing_never_scrolls_past_the_end_of_the_document() {
        let mut state = with_results();
        state.doc_scroll = 50;

        assert!(shows(&screen(&state), r#""id": "c-1","#));
    }
}
