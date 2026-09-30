//! Draws the state on the terminal.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, List, ListItem, ListState, Paragraph};

use crate::state::{AppState, Focus, Status};

/// Draws the tree, search bar, results, document and status line.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let [main, status] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    let [tree, right] =
        Layout::horizontal([Constraint::Percentage(25), Constraint::Fill(1)]).areas(main);
    let [search, body] =
        Layout::vertical([Constraint::Length(3), Constraint::Fill(1)]).areas(right);
    let [results, document] =
        Layout::horizontal([Constraint::Percentage(45), Constraint::Fill(1)]).areas(body);

    draw_tree(frame, state, tree);
    frame.render_widget(pane("Search", state, Focus::Search), search);
    frame.render_widget(pane("Results", state, Focus::Results), results);
    frame.render_widget(pane("Document", state, Focus::Document), document);
    draw_status(frame, state, status);
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

/// A bordered pane, highlighted when it has focus.
fn pane<'a>(title: &'a str, state: &AppState, focus: Focus) -> Block<'a> {
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
    let line = match &state.status {
        Status::Info(text) => Line::raw(text.as_str()),
        Status::Error(text) => Line::styled(format!("error: {text}"), Style::new().fg(Color::Red)),
    };
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Event, Msg, update};
    use cosmos_core::testing::{account, container};
    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
}
