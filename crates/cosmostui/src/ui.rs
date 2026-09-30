//! Draws the state on the terminal.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Paragraph};

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

    frame.render_widget(pane("Accounts", state, Focus::Tree), tree);
    frame.render_widget(pane("Search", state, Focus::Search), search);
    frame.render_widget(pane("Results", state, Focus::Results), results);
    frame.render_widget(pane("Document", state, Focus::Document), document);
    draw_status(frame, state, status);
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
}
