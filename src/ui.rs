//! Draws the state on the terminal.

use crate::partition::{display_value, value_at_path};
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin, Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Scrollbar, ScrollbarOrientation,
    ScrollbarState, Table, TableState, Wrap,
};

use crate::editor::Editor;
use crate::json::{highlight_json, highlight_json_array, highlight_json_line};
use crate::query::DEFAULT_QUERY;
use crate::settings::{ContainerSettings, Field, Geospatial, SettingsTab, TimeToLive};
use crate::sql::highlight_sql;
use crate::state::{
    AppState, Focus, Load, Mode, OutputTab, SavePrompt, Saving, Selection, Status, settings_focus,
};

/// Room for the query the search bar completes, and a space after it.
const QUERY_WIDTH: u16 = DEFAULT_QUERY.len() as u16 + 1;

/// Where each part of the screen goes.
struct Panes {
    tree: Rect,
    query: Rect,
    search: Rect,
    results: Rect,
    document: Rect,
    editor: Rect,
    output: Rect,
    /// The settings of the picked container, whichever tab shows.
    settings: Rect,
    status: Rect,
}

/// Lays out the panes of the mode, leaving all the room to the zoomed pane if there is one.
fn panes(area: Rect, mode: Mode, tree_hidden: bool, zoomed: Option<Focus>) -> Panes {
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
    let [editor, output] =
        Layout::vertical([Constraint::Percentage(35), Constraint::Fill(1)]).areas(right);
    let mut panes = match mode {
        Mode::Browse => Panes {
            tree,
            query,
            search,
            results,
            document,
            editor: Rect::default(),
            output: Rect::default(),
            settings: Rect::default(),
            status,
        },
        Mode::Query => Panes {
            tree,
            query: Rect::default(),
            search: Rect::default(),
            results: Rect::default(),
            document: Rect::default(),
            editor,
            output,
            settings: Rect::default(),
            status,
        },
        Mode::Settings => Panes {
            tree,
            query: Rect::default(),
            search: Rect::default(),
            results: Rect::default(),
            document: Rect::default(),
            editor: Rect::default(),
            output: Rect::default(),
            settings: right,
            status,
        },
    };
    if let Some(focus) = zoomed {
        for (pane, area) in [
            (Focus::Tree, &mut panes.tree),
            (Focus::Search, &mut panes.search),
            (Focus::Results, &mut panes.results),
            (Focus::Document, &mut panes.document),
            (Focus::Editor, &mut panes.editor),
            (Focus::Output, &mut panes.output),
        ] {
            *area = if pane == focus { main } else { Rect::default() };
        }
        panes.settings = if focus.settings_tab().is_some() {
            main
        } else {
            Rect::default()
        };
        panes.query = Rect::default();
    }
    panes
}

/// How many tree rows the tree pane shows on a terminal of this size.
pub fn tree_height(size: Size) -> u16 {
    let tree = panes(
        Rect::from((Position::ORIGIN, size)),
        Mode::Browse,
        false,
        None,
    )
    .tree;
    // Less the top and bottom borders
    tree.height.saturating_sub(2)
}

/// The pane at a point on a terminal of this size, with the point inside its
/// borders, or `None` for the point when it is on a border.
pub fn pane_at(
    size: Size,
    at: Position,
    mode: Mode,
    tree_hidden: bool,
    zoomed: Option<Focus>,
) -> Option<(Focus, Option<Position>)> {
    let panes = panes(
        Rect::from((Position::ORIGIN, size)),
        mode,
        tree_hidden,
        zoomed,
    );
    let (focus, area) = [
        (Focus::Tree, panes.tree),
        (Focus::Search, panes.search),
        (Focus::Results, panes.results),
        (Focus::Document, panes.document),
        (Focus::Editor, panes.editor),
        (Focus::Output, panes.output),
        (Focus::SettingsForm, panes.settings),
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
    let document = panes(
        Rect::from((Position::ORIGIN, size)),
        Mode::Browse,
        false,
        zoomed,
    )
    .document;
    // Less the top and bottom borders
    document.height.saturating_sub(2)
}

/// How many lines the query editor shows on a terminal of this size,
/// or none when another pane is zoomed.
pub fn editor_height(size: Size, zoomed: Option<Focus>) -> u16 {
    let editor = panes(
        Rect::from((Position::ORIGIN, size)),
        Mode::Query,
        false,
        zoomed,
    )
    .editor;
    // Less the top and bottom borders
    editor.height.saturating_sub(2)
}

/// How many lines of the query output show on a terminal of this size,
/// or none when another pane is zoomed.
pub fn output_height(size: Size, zoomed: Option<Focus>) -> u16 {
    let output = panes(
        Rect::from((Position::ORIGIN, size)),
        Mode::Query,
        false,
        zoomed,
    )
    .output;
    // Less the top and bottom borders
    output.height.saturating_sub(2)
}

/// How many lines the settings tabs show on a terminal of this size,
/// or none when another pane is zoomed.
pub fn settings_height(size: Size, zoomed: Option<Focus>) -> u16 {
    let settings = panes(
        Rect::from((Position::ORIGIN, size)),
        Mode::Settings,
        false,
        zoomed,
    )
    .settings;
    // Less the top and bottom borders
    settings.height.saturating_sub(2)
}

/// How many documents the results table shows on a terminal of this size,
/// or none when another pane is zoomed.
pub fn results_height(size: Size, zoomed: Option<Focus>) -> u16 {
    let results = panes(
        Rect::from((Position::ORIGIN, size)),
        Mode::Browse,
        false,
        zoomed,
    )
    .results;
    // Less the borders and the header row
    results.height.saturating_sub(3)
}

/// Draws the tree, search bar, results, document and status line.
pub fn draw(frame: &mut Frame, state: &AppState) {
    let panes = panes(
        frame.area(),
        state.mode,
        state.tree_hidden,
        state.zoomed_pane(),
    );
    draw_tree(frame, state, panes.tree);
    match state.mode {
        Mode::Browse => {
            draw_query(frame, panes.query);
            draw_search(frame, state, panes.search);
            draw_results(frame, state, panes.results);
            draw_document(frame, state, panes.document);
        }
        Mode::Query => {
            draw_editor(frame, state, panes.editor);
            draw_output(frame, state, panes.output);
        }
        Mode::Settings => draw_settings(frame, state, panes.settings),
    }
    draw_status(frame, state, panes.status);
    if state.show_help {
        draw_help(frame, state);
    }
    if state.confirm_delete {
        draw_confirm_delete(frame, state);
    }
    if state.confirm_discard {
        draw_confirm_discard(frame, state);
    }
    if let Some(prompt) = &state.prompt {
        draw_save_prompt(frame, prompt);
    }
    if let Some(error) = &state.error {
        draw_error(frame, error);
    }
}

fn draw_tree(frame: &mut Frame, state: &AppState, area: Rect) {
    let items: Vec<ListItem> = state
        .tree_rows()
        .into_iter()
        .map(|row| {
            let marker = match row.expanded {
                Some(true) => "▾ ",
                Some(false) => "▸ ",
                None => "  ",
            };
            let indent = "  ".repeat(row.depth);
            ListItem::new(format!("{indent}{marker}{}", row.label))
        })
        .collect();
    let rows = items.len();
    let list = List::new(items)
        .block(pane("Accounts", state, Focus::Tree))
        .highlight_style(Style::new().add_modifier(Modifier::REVERSED));
    let mut list_state = ListState::default().with_selected(Some(state.tree_selected));
    frame.render_stateful_widget(list, area, &mut list_state);
    draw_scrollbar(
        frame,
        area.inner(Margin::new(0, 1)),
        rows,
        list_state.offset(),
    );
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
    // Below the header row
    let rows = Rect {
        y: area.y.saturating_add(2),
        height: area.height.saturating_sub(3),
        ..area
    };
    draw_scrollbar(frame, rows, state.results.len(), table_state.offset());
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

/// Shown in the empty search bar, like the hint in the Azure portal's Data Explorer.
const SEARCH_HINT: &str = "Type a query predicate (e.g. WHERE c.id = '1' or ORDER BY c._ts DESC)";

fn draw_search(frame: &mut Frame, state: &AppState, area: Rect) {
    let text = match state.search.text() {
        "" => Span::styled(SEARCH_HINT, Style::new().add_modifier(Modifier::DIM)),
        text => Span::raw(text),
    };
    let input = Paragraph::new(text).block(pane("Search", state, Focus::Search));
    frame.render_widget(input, area);
    if state.focus == Focus::Search {
        let cursor = u16::try_from(state.search.cursor()).unwrap_or(u16::MAX);
        frame.set_cursor_position((area.x + 1 + cursor, area.y + 1));
    }
}

fn draw_editor(frame: &mut Frame, state: &AppState, area: Rect) {
    let title = match &state.target {
        Some(target) => format!("Query: {}/{}", target.database, target.container),
        None => "Query".to_string(),
    };
    let block = pane(title, state, Focus::Editor);
    let focused = state.focus == Focus::Editor;
    draw_code(frame, &state.editor, highlight_sql, block, focused, area);
}

/// Draws text being edited with numbered, coloured lines, scrolled to show the
/// cursor, which shows while the pane has the focus.
fn draw_code(
    frame: &mut Frame,
    editor: &Editor,
    highlight: fn(&str) -> Line<'static>,
    block: Block,
    focused: bool,
    area: Rect,
) {
    let lines = editor.lines();
    let gutter = editor.number_width();
    let numbered: Vec<Line> = lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let number = Span::styled(
                format!("{:>gutter$} ", index + 1),
                Style::new().fg(Color::DarkGray),
            );
            let mut spans = vec![number];
            spans.extend(highlight(line).spans);
            Line::from(spans)
        })
        .collect();
    let (row, column) = editor.cursor();
    let scroll = editor.scroll(usize::from(area.height.saturating_sub(2)));
    let paragraph = Paragraph::new(numbered)
        .block(block)
        .scroll((u16::try_from(scroll).unwrap_or(u16::MAX), 0));
    frame.render_widget(paragraph, area);
    draw_scrollbar(frame, area.inner(Margin::new(0, 1)), lines.len(), scroll);
    if focused {
        let x = area.x + 1 + u16::try_from(gutter + 1 + column).unwrap_or(u16::MAX);
        let y = area.y + 1 + u16::try_from(row - scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((x.min(area.right().saturating_sub(2)), y));
    }
}

/// The settings title: the container and the tabs, with the shown one picked out
/// and a star on those with changes.
fn settings_title(state: &AppState) -> Line<'static> {
    let mut spans = Vec::new();
    if let Some(target) = &state.target {
        spans.push(Span::raw(format!(
            "{}/{}: ",
            target.database, target.container
        )));
    }
    let tabs = [
        ("Settings", SettingsTab::Settings),
        ("Indexing Policy", SettingsTab::IndexingPolicy),
        ("Computed Properties", SettingsTab::ComputedProperties),
    ];
    for (index, (name, tab)) in tabs.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw(" │ "));
        }
        let changed = match &state.settings {
            Some(Load::Loaded(settings)) if settings.modified(tab) => "*",
            _ => "",
        };
        let style = if state.settings_tab == tab {
            Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        };
        spans.push(Span::styled(format!("{name}{changed}"), style));
    }
    Line::from(spans)
}

fn draw_settings(frame: &mut Frame, state: &AppState, area: Rect) {
    let focus = settings_focus(state.settings_tab);
    let block = pane(settings_title(state), state, focus);
    let settings = match &state.settings {
        Some(Load::Loaded(settings)) => settings,
        Some(Load::Failed(error)) => {
            let error = Line::styled(format!("error: {error}"), Style::new().fg(Color::Red));
            let paragraph = Paragraph::new(error).wrap(Wrap { trim: false });
            frame.render_widget(paragraph.block(block), area);
            return;
        }
        Some(Load::Loading) | None => {
            frame.render_widget(Paragraph::new("loading settings…").block(block), area);
            return;
        }
    };
    match state.settings_tab {
        SettingsTab::Settings => draw_settings_form(frame, state, settings, block, area),
        SettingsTab::IndexingPolicy | SettingsTab::ComputedProperties => {
            let editor = match state.settings_tab {
                SettingsTab::IndexingPolicy => &settings.indexing,
                _ => &settings.computed,
            };
            let focused = state.focus == focus;
            draw_code(frame, editor, highlight_json_line, block, focused, area);
        }
    }
}

/// Draws the time to live and geospatial choices, and the partition key.
fn draw_settings_form(
    frame: &mut Frame,
    state: &AppState,
    settings: &ContainerSettings,
    block: Block,
    area: Rect,
) {
    let focused = state.focus == Focus::SettingsForm;
    let heading =
        |text: &'static str| Line::styled(text, Style::new().add_modifier(Modifier::BOLD));
    let radio = |label: &'static str, picked: bool, field: Field| {
        let mark = if picked { "(•)" } else { "( )" };
        let mut style = Style::new();
        if picked && focused && settings.field == field {
            style = style.add_modifier(Modifier::REVERSED);
        }
        Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{mark} {label}"), style),
        ])
    };
    let mut lines = vec![
        heading("Time to Live"),
        radio("Off", settings.ttl == TimeToLive::Off, Field::Ttl),
        radio(
            "On (no default)",
            settings.ttl == TimeToLive::NoDefault,
            Field::Ttl,
        ),
        radio("On", settings.ttl == TimeToLive::Seconds, Field::Ttl),
    ];
    let mut cursor = None;
    if settings.ttl == TimeToLive::Seconds {
        let editing = focused && settings.field == Field::Seconds;
        let style = if editing {
            Style::new().fg(Color::Cyan)
        } else {
            Style::new()
        };
        if editing {
            // Inside the border, past the indent and the opening bracket
            let column = u16::try_from(7 + settings.seconds.cursor()).unwrap_or(u16::MAX);
            let row = u16::try_from(lines.len()).unwrap_or(u16::MAX);
            cursor = Some((area.x + 1 + column, area.y + 1 + row));
        }
        lines.push(Line::from(vec![
            Span::raw("      "),
            Span::styled(format!("[{}]", settings.seconds.text()), style),
            Span::raw(" second(s)"),
        ]));
    }
    let (paths, note) = settings.partition_key();
    let dim = Style::new().add_modifier(Modifier::DIM);
    lines.extend([
        Line::raw(""),
        heading("Geospatial Configuration"),
        radio(
            "Geography",
            settings.geospatial == Geospatial::Geography,
            Field::Geospatial,
        ),
        radio(
            "Geometry",
            settings.geospatial == Geospatial::Geometry,
            Field::Geospatial,
        ),
        Line::raw(""),
        heading("Partition key"),
        Line::styled(format!("  {paths}"), dim),
        Line::styled(format!("  {note}"), dim),
    ]);
    frame.render_widget(Paragraph::new(lines).block(block), area);
    if let Some(position) = cursor {
        frame.set_cursor_position(position);
    }
}

/// The output title: its tabs, with the shown one picked out, and the range of
/// documents shown and whether more can load.
fn output_title(state: &AppState) -> Line<'static> {
    let tab = |name: &'static str, tab: OutputTab| {
        if state.output_tab == tab {
            Span::styled(
                name,
                Style::new().add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            )
        } else {
            Span::styled(name, Style::new().add_modifier(Modifier::DIM))
        }
    };
    let mut range = match state.output.results.len() {
        0 => "0".to_string(),
        count => format!("1 - {count}"),
    };
    if state.output.loading_more {
        range.push_str(", loading…");
    } else if state.output.more {
        range.push_str(", more ↓");
    }
    Line::from(vec![
        tab("Results", OutputTab::Results),
        Span::raw(" │ "),
        tab("Stats", OutputTab::Stats),
        Span::raw(format!(" ({range})")),
    ])
}

fn draw_output(frame: &mut Frame, state: &AppState, area: Rect) {
    let block = pane(output_title(state), state, Focus::Output);
    if state.output_tab == OutputTab::Stats {
        frame.render_widget(stats_table(state).block(block), area);
        return;
    }
    let lines = highlight_json_array(&state.output.results);
    let total = lines.len();
    let height = area.height.saturating_sub(2);
    let last = u16::try_from(total)
        .unwrap_or(u16::MAX)
        .saturating_sub(height);
    // The pane may have grown since the output was scrolled
    let scroll = state.output_scroll.min(last);
    let output = Paragraph::new(lines).block(block).scroll((scroll, 0));
    frame.render_widget(output, area);
    draw_scrollbar(
        frame,
        area.inner(Margin::new(0, 1)),
        total,
        usize::from(scroll),
    );
}

/// What the query cost, then each metric the service reported for it.
fn stats_table(state: &AppState) -> Table<'static> {
    let stats = &state.output.stats;
    let mut rows = vec![
        Row::new([
            "Request charge".to_string(),
            format!("{:.2} RU", stats.request_charge),
        ]),
        Row::new([
            "Documents".to_string(),
            state.output.results.len().to_string(),
        ]),
        Row::new(["Round trips".to_string(), stats.round_trips.to_string()]),
    ];
    rows.extend(
        stats
            .query_metrics
            .iter()
            .map(|(name, value)| Row::new([name.clone(), metric(*value)])),
    );
    Table::new(rows, [Constraint::Length(32), Constraint::Fill(1)])
}

/// A metric value, without decimals when it is a whole number.
fn metric(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
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
    draw_scrollbar(
        frame,
        area.inner(Margin::new(0, 1)),
        usize::from(state.document_lines()),
        usize::from(scroll),
    );
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

/// Draws a scrollbar thumb on the right border of `area`, the rows that show
/// `total` lines from `offset` on, when they do not all fit.
fn draw_scrollbar(frame: &mut Frame, area: Rect, total: usize, offset: usize) {
    let viewport = usize::from(area.height);
    if total <= viewport {
        return;
    }
    // One position for each offset the rows can scroll to
    let mut scrollbar = ScrollbarState::new(total - viewport + 1)
        .position(offset)
        .viewport_content_length(viewport);
    let bar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(None);
    frame.render_stateful_widget(bar, area, &mut scrollbar);
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
    let hint = match (state.mode, state.zoomed_pane().is_some()) {
        (Mode::Browse, true) => "z unzoom · Tab next pane · / search · n query · ? help · q quit",
        (Mode::Browse, false) => "Tab next pane · / search · n query · ? help · q quit",
        (Mode::Query, _) => "F5 run · Esc back · Tab next pane · ? help",
        (Mode::Settings, _) => "Ctrl-S save · Esc back · Tab next tab · ? help",
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
const HELP: [(&str, &str); 21] = [
    ("Tab / Shift-Tab", "Next pane / previous pane"),
    ("Ctrl-B", "Hide or show the accounts"),
    ("z", "Zoom the pane, or show every pane again"),
    ("/", "Search: type SQL, a clause or a condition"),
    ("Enter", "Open a node, search, go to the documents"),
    ("↑ ↓  j k", "Move, scroll, or load more at the end"),
    ("→ ←  l h", "Open or close a node"),
    ("PgUp PgDn Home End", "Move or scroll a page, or to an end"),
    ("Ctrl-U Ctrl-D", "Scroll half a page up or down"),
    ("g g  G", "Jump to the first or last"),
    ("Click, again", "Pick a pane or row, then open it"),
    ("Wheel", "Scroll the pane under the mouse"),
    ("Drag, y", "Pick text in the document, copy it"),
    ("Esc", "Leave the search bar, drop the picked text"),
    ("r", "Run the query again"),
    ("n  s", "Write a query, or edit the container's settings"),
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

/// What each key does in query mode, shown with `?` there.
const QUERY_HELP: [(&str, &str); 19] = [
    ("Tab / Shift-Tab", "Next pane / previous pane"),
    ("F5  Ctrl-R", "Run the query"),
    ("Shift-Enter", "Run the query, in terminals that report it"),
    ("Enter", "Start a new line in the query"),
    ("/", "Go to the query editor"),
    ("↑ ↓  j k", "Scroll the output, or load more at the end"),
    (
        "PgUp PgDn Home End",
        "Scroll the output a page, or to an end",
    ),
    ("g g  G", "Jump to the top or bottom of the output"),
    ("s", "Switch the output between results and stats"),
    ("y", "Copy the results as JSON"),
    ("w", "Save the results to a JSON file"),
    ("Ctrl-S", "Save the query to a file"),
    ("r", "Run the query again from the output"),
    ("Esc", "Leave the editor, then the query editor"),
    ("n", "Leave the query editor from the output"),
    ("Ctrl-B", "Hide or show the accounts"),
    ("z", "Zoom the pane, or show every pane again"),
    ("?, Ctrl-C", "Show this help, quit"),
    ("", "Press any key to close"),
];

/// What each key does in settings mode, shown with `?` there.
const SETTINGS_HELP: [(&str, &str); 10] = [
    ("Tab / Shift-Tab", "Next tab / previous tab"),
    (
        "↑ ↓  j k",
        "Move between the settings, or the lines of JSON",
    ),
    ("← →  h l  Space", "Pick another choice"),
    ("0-9", "Type the seconds documents live"),
    ("Ctrl-S", "Save the settings"),
    ("Esc  s", "Leave the settings, asking to discard changes"),
    ("Ctrl-B", "Hide or show the accounts"),
    ("z", "Zoom the pane, outside the JSON tabs"),
    ("?, Ctrl-C", "Show this help, quit"),
    ("", "Press any key to close"),
];

fn draw_help(frame: &mut Frame, state: &AppState) {
    let help: &[(&str, &str)] = match state.mode {
        Mode::Browse => &HELP,
        Mode::Query => &QUERY_HELP,
        Mode::Settings => &SETTINGS_HELP,
    };
    let lines: Vec<Line> = help
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

/// A dialog asking whether to discard the changes to the settings.
fn draw_confirm_discard(frame: &mut Frame, state: &AppState) {
    let container = state
        .target
        .as_ref()
        .map(|target| format!(" of {}/{}", target.database, target.container))
        .unwrap_or_default();
    let lines = vec![
        Line::raw(format!("Discard the changes to the settings{container}?")),
        Line::raw(""),
        Line::from(vec![
            Span::styled("y / Enter", Style::new().fg(Color::Red)),
            Span::raw(" discard   "),
            Span::styled("Esc", Style::new().fg(Color::Cyan)),
            Span::raw(" or any other key keep editing"),
        ]),
    ];
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2;
    let area = frame
        .area()
        .centered(Constraint::Length(60), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title("Discard changes")
                .border_style(Style::new().fg(Color::Red)),
        ),
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

/// A dialog with why the query failed, wrapped to fit.
fn draw_error(frame: &mut Frame, error: &str) {
    let width = frame.area().width.saturating_sub(4).min(90);
    let inner = usize::from(width.saturating_sub(2)).max(1);
    let mut lines: Vec<Line> = error
        .lines()
        .map(|line| Line::raw(line.trim_end()))
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::styled(
        "Press any key to close",
        Style::new().fg(Color::DarkGray),
    ));
    // Rows each line takes wrapped, with room for words that move to the next row
    let rows: usize = lines
        .iter()
        .map(|line| line.width().div_ceil(inner).max(1))
        .sum();
    let height = u16::try_from(rows + rows / 4 + 2).unwrap_or(u16::MAX);
    let area = frame
        .area()
        .centered(Constraint::Length(width), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::bordered()
                .title("Query failed")
                .border_style(Style::new().fg(Color::Red)),
        ),
        area,
    );
}

/// A dialog asking where to save, with the cursor in the path.
fn draw_save_prompt(frame: &mut Frame, prompt: &SavePrompt) {
    let title = match prompt.saving {
        Saving::Query => "Save query",
        Saving::Results => "Save results",
    };
    let lines = vec![
        Line::raw(prompt.path.text()),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Enter", Style::new().fg(Color::Cyan)),
            Span::raw(" save   "),
            Span::styled("Esc", Style::new().fg(Color::Cyan)),
            Span::raw(" cancel"),
        ]),
    ];
    let height = u16::try_from(lines.len()).unwrap_or(u16::MAX) + 2;
    let area = frame
        .area()
        .centered(Constraint::Length(60), Constraint::Length(height));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .title(title)
                .border_style(Style::new().fg(Color::Cyan)),
        ),
        area,
    );
    let cursor = u16::try_from(prompt.path.cursor()).unwrap_or(u16::MAX);
    let x = (area.x + 1 + cursor).min(area.right().saturating_sub(2));
    frame.set_cursor_position((x, area.y + 1));
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
    fn the_query_editor_and_output_split_the_rows_beside_the_tree() {
        let size = Size::new(100, 20);

        // 20 rows less the status line: 7 for the editor and 12 for the output, less borders
        assert_eq!(editor_height(size, None), 5);
        assert_eq!(output_height(size, None), 10);
        assert_eq!(output_height(size, Some(Focus::Output)), 17);
        assert_eq!(editor_height(size, Some(Focus::Output)), 0);
    }

    #[test]
    fn the_settings_take_the_rows_beside_the_tree() {
        let size = Size::new(100, 20);

        // 20 rows less the status line and the borders
        assert_eq!(settings_height(size, None), 17);
        assert_eq!(settings_height(size, Some(Focus::IndexingPolicy)), 17);
        assert_eq!(settings_height(size, Some(Focus::Tree)), 0);
        let at = |x, y| pane_at(size, Position::new(x, y), Mode::Settings, false, None);
        assert_eq!(
            at(30, 2),
            Some((Focus::SettingsForm, Some(Position::new(4, 1))))
        );
        assert_eq!(at(5, 2), Some((Focus::Tree, Some(Position::new(4, 1)))));
    }

    #[test]
    fn finds_the_query_editor_and_output_in_query_mode() {
        let size = Size::new(100, 20);
        let at = |x, y| pane_at(size, Position::new(x, y), Mode::Query, false, None);

        assert_eq!(at(30, 2), Some((Focus::Editor, Some(Position::new(4, 1)))));
        assert_eq!(at(30, 10), Some((Focus::Output, Some(Position::new(4, 2)))));
    }

    #[test]
    fn finds_the_pane_at_a_point_and_where_the_point_is_inside_it() {
        let size = Size::new(100, 20);
        let at = |x, y| pane_at(size, Position::new(x, y), Mode::Browse, false, None);

        assert_eq!(at(3, 4), Some((Focus::Tree, Some(Position::new(2, 3)))));
        assert_eq!(at(27, 1), None);
        assert_eq!(at(43, 1), Some((Focus::Search, Some(Position::new(1, 0)))));
        assert_eq!(at(30, 5).map(|(pane, _)| pane), Some(Focus::Results));
        assert_eq!(at(90, 5).map(|(pane, _)| pane), Some(Focus::Document));
    }

    #[test]
    fn finds_the_panes_that_take_the_room_of_a_hidden_tree() {
        let size = Size::new(100, 20);
        let at = |x, y| pane_at(size, Position::new(x, y), Mode::Browse, true, None);

        assert_eq!(at(3, 1), None);
        assert_eq!(at(19, 1), Some((Focus::Search, Some(Position::new(2, 0)))));
        assert_eq!(at(3, 5).map(|(pane, _)| pane), Some(Focus::Results));
    }

    #[test]
    fn finds_the_zoomed_pane_wherever_it_reaches() {
        let size = Size::new(100, 20);
        let at = |x, y| {
            pane_at(
                size,
                Position::new(x, y),
                Mode::Browse,
                false,
                Some(Focus::Document),
            )
        };

        assert_eq!(at(3, 1), Some((Focus::Document, Some(Position::new(2, 0)))));
        assert_eq!(at(90, 17).map(|(pane, _)| pane), Some(Focus::Document));
        assert_eq!(at(5, 19), None);
    }

    #[test]
    fn a_point_on_a_border_is_in_the_pane_but_not_inside_it() {
        let size = Size::new(100, 20);

        assert_eq!(
            pane_at(size, Position::new(0, 5), Mode::Browse, false, None),
            Some((Focus::Tree, None))
        );
        assert_eq!(
            pane_at(size, Position::new(30, 3), Mode::Browse, false, None),
            Some((Focus::Results, None))
        );
    }

    #[test]
    fn the_status_line_is_in_no_pane() {
        assert_eq!(
            pane_at(
                Size::new(100, 20),
                Position::new(5, 19),
                Mode::Browse,
                false,
                None
            ),
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
            stats: crate::state::QueryStats::default(),
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

    fn with_settings(properties: serde_json::Value) -> AppState {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('s'));
        let target = state.target.clone().unwrap();
        send(
            &mut state,
            Msg::SettingsLoaded {
                target,
                result: Ok(properties),
            },
        );
        state
    }

    #[test]
    fn settings_mode_draws_the_settings_tab_in_place_of_the_other_panes() {
        let state = with_settings(json!({
            "id": "carts",
            "defaultTtl": 3600,
            "partitionKey": {"paths": ["/tenantId"], "kind": "Hash"}
        }));

        let screen = screen(&state);

        for hidden in ["Search", "Document", "Results"] {
            assert!(!shows(&screen, hidden), "{}", screen.join("\n"));
        }
        for shown in [
            "shop/carts: Settings │ Indexing Policy │ Computed Properties",
            "Time to Live",
            "( ) Off",
            "( ) On (no default)",
            "(•) On",
            "[3600] second(s)",
            "Geospatial Configuration",
            "(•) Geography",
            "( ) Geometry",
            "Partition key",
            "/tenantId",
            "Non-hierarchically partitioned container.",
        ] {
            assert!(shows(&screen, shown), "{shown}\n{}", screen.join("\n"));
        }
    }

    #[test]
    fn the_json_settings_tabs_number_their_lines() {
        let mut state = with_settings(json!({
            "indexingPolicy": {"automatic": true},
            "computedProperties": [{"name": "cp"}]
        }));

        press(&mut state, KeyCode::Tab);
        let indexing = screen(&state);
        press(&mut state, KeyCode::Tab);
        let computed = screen(&state);

        assert!(shows(&indexing, " 1 {"), "{}", indexing.join("\n"));
        assert!(
            shows(&indexing, r#" 2   "automatic": true"#),
            "{}",
            indexing.join("\n")
        );
        assert!(
            shows(&computed, r#" 3     "name": "cp""#),
            "{}",
            computed.join("\n")
        );
    }

    #[test]
    fn asks_to_discard_changed_settings_in_a_dialog() {
        let mut state = with_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);

        press(&mut state, KeyCode::Esc);
        let screen = screen(&state);

        for text in [
            "Discard changes",
            "Discard the changes to the settings of shop/carts?",
            "y / Enter discard",
        ] {
            assert!(shows(&screen, text), "{text}\n{}", screen.join("\n"));
        }
    }

    #[test]
    fn the_help_lists_the_settings_keys_in_settings_mode() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('?'));
        assert!(shows(
            &screen_with_height(&state, 40),
            "edit the container's settings"
        ));
        press(&mut state, KeyCode::Esc);

        press(&mut state, KeyCode::Char('s'));
        press(&mut state, KeyCode::Char('?'));
        let screen = screen_with_height(&state, 40);

        for text in [
            "Save the settings",
            "Leave the settings",
            "Next tab / previous tab",
        ] {
            assert!(shows(&screen, text), "{text}\n{}", screen.join("\n"));
        }
    }

    #[test]
    fn the_settings_tabs_mark_the_ones_with_changes() {
        let mut state = with_settings(json!({"id": "carts"}));
        press(&mut state, KeyCode::Right);

        let screen = screen(&state);

        assert!(!shows(&screen, "second(s)"), "{}", screen.join("\n"));
        assert!(
            shows(&screen, "Settings* │ Indexing Policy │"),
            "{}",
            screen.join("\n")
        );
    }

    #[test]
    fn the_settings_say_while_they_load_and_why_they_failed() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('s'));
        assert!(shows(&screen(&state), "loading settings…"));

        let target = state.target.clone().unwrap();
        send(
            &mut state,
            Msg::SettingsLoaded {
                target,
                result: Err("forbidden".into()),
            },
        );

        assert!(shows(&screen(&state), "error: forbidden"));
    }

    #[test]
    fn query_mode_draws_the_editor_and_output_in_place_of_the_other_panes() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));

        let screen = screen(&state);

        for title in ["Search", "Document", "/tenantId"] {
            assert!(!shows(&screen, title), "{}", screen.join("\n"));
        }
        for title in ["Accounts", "Query: shop/carts", "Results"] {
            assert!(shows(&screen, title), "{}", screen.join("\n"));
        }
    }

    #[test]
    fn the_editor_numbers_its_lines_and_colours_the_sql() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));
        state.editor.set_text("SELECT c.id\nFROM c");

        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = screen(&state);

        assert!(shows(&screen, " 1 SELECT c.id"), "{}", screen.join("\n"));
        assert!(shows(&screen, " 2 FROM c"), "{}", screen.join("\n"));
        let buffer = terminal.backend().buffer();
        let row = screen.iter().position(|l| l.contains(" 1 SELECT")).unwrap();
        let x = screen[row].chars().position(|c| c == 'S').unwrap();
        let cell = &buffer[(u16::try_from(x).unwrap(), u16::try_from(row).unwrap())];
        assert_eq!(cell.fg, Color::Blue);
    }

    #[test]
    fn the_editor_scrolls_to_keep_the_cursor_in_view_and_shows_it() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));
        let text: Vec<String> = (1..=30).map(|n| format!("-- line {n}")).collect();
        state.editor.set_text(&text.join("\n"));

        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = screen(&state);

        assert!(shows(&screen, "30 -- line 30"), "{}", screen.join("\n"));
        assert!(!shows(&screen, " 1 -- line 1"), "{}", screen.join("\n"));
        let cursor = terminal.get_cursor_position().unwrap();
        let row = screen
            .iter()
            .position(|l| l.contains("30 -- line 30"))
            .unwrap();
        assert_eq!(usize::from(cursor.y), row);
    }

    #[test]
    fn the_output_shows_the_results_as_one_json_array_with_their_range() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));
        state.output.results = state.results.clone();

        let screen = screen_with_height(&state, 30);

        assert!(
            shows(&screen, "Results │ Stats (1 - 2)"),
            "{}",
            screen.join("\n")
        );
        for line in [
            "[",
            r#"    "id": "c-1","#,
            "  },",
            r#"    "tenantId": "fabrikam""#,
            "]",
        ] {
            assert!(
                shows(&screen, line),
                "{line} missing from\n{}",
                screen.join("\n")
            );
        }

        state.output.more = true;
        assert!(shows(
            &screen_with_height(&state, 30),
            "Stats (1 - 2, more ↓)"
        ));
    }

    #[test]
    fn the_stats_tab_lists_what_the_query_cost_and_its_metrics() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));
        state.output.results = state.results.clone();
        state.output.stats.request_charge = 3.5;
        state.output.stats.round_trips = 2;
        state
            .output
            .stats
            .add_query_metrics("retrievedDocumentCount=40;totalExecutionTimeInMs=1.75");
        state.output_tab = crate::state::OutputTab::Stats;

        let screen = screen_with_height(&state, 30);

        for row in [
            ["Request charge", "3.50 RU"],
            ["Documents", "2"],
            ["Round trips", "2"],
            ["retrievedDocumentCount", "40"],
            ["totalExecutionTimeInMs", "1.75"],
        ] {
            let found = screen.iter().any(|line| {
                line.contains(row[0])
                    && line
                        .split_whitespace()
                        .any(|w| w == row[1].split(' ').next().unwrap())
            });
            assert!(found, "{row:?} missing from\n{}", screen.join("\n"));
        }
        assert!(!shows(&screen, r#""id": "c-1""#), "{}", screen.join("\n"));
    }

    #[test]
    fn the_status_line_hints_at_the_query_editor_and_how_to_run_it() {
        let mut state = with_results();
        assert!(
            shows(&screen(&state), "n query"),
            "{}",
            screen(&state).join("\n")
        );

        press(&mut state, KeyCode::Char('n'));

        let screen = screen(&state);
        assert!(shows(&screen, "F5 run · Esc back"), "{}", screen.join("\n"));
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
    fn an_empty_search_bar_hints_at_what_to_type_in_dim_text() {
        let (mut state, _) = AppState::new();
        state.tree_hidden = true;
        let hint = "Type a query predicate (e.g. WHERE c.id = '1' or ORDER BY c._ts DESC)";

        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();

        let buffer = terminal.backend().buffer();
        let search = panes(buffer.area, Mode::Browse, true, None).search;
        let dimmed: String = (search.x..search.right())
            .map(|x| &buffer[(x, 1)])
            .filter(|cell| cell.modifier.contains(Modifier::DIM))
            .map(|cell| cell.symbol())
            .collect();
        assert_eq!(dimmed.trim_end(), hint);

        state.focus = Focus::Search;
        press(&mut state, KeyCode::Char('c'));
        assert!(!shows(&screen(&state), "Type a query predicate"));
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
    fn draws_a_dialog_asking_where_to_save_with_the_cursor_in_the_path() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));
        press(&mut state, KeyCode::Esc);
        press(&mut state, KeyCode::Char('w'));

        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|frame| draw(frame, &state)).unwrap();
        let screen = screen(&state);

        for text in ["Save results", "carts.json", "Enter", "Esc"] {
            assert!(
                shows(&screen, text),
                "{text} missing from\n{}",
                screen.join("\n")
            );
        }
        let cursor = terminal.get_cursor_position().unwrap();
        let row = screen
            .iter()
            .position(|l| l.contains("carts.json"))
            .unwrap();
        let start = screen[row].find("carts.json").unwrap();
        let column = screen[row][..start].chars().count() + "carts.json".len();
        assert_eq!(
            (usize::from(cursor.x), usize::from(cursor.y)),
            (column, row)
        );
    }

    #[test]
    fn draws_why_the_query_failed_in_a_dialog_wrapping_long_lines() {
        let mut state = with_results();
        let words: Vec<String> = (0..40).map(|n| format!("word{n:02}")).collect();
        state.error = Some(format!("{}\nActivityId: 7ad6", words.join(" ")));

        let screen = screen_with_height(&state, 24);

        for text in [
            "Query failed",
            "word00",
            "word39",
            "ActivityId: 7ad6",
            "any key",
        ] {
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
    fn the_key_help_lists_the_query_editor_key() {
        let mut state = browsing();
        press(&mut state, KeyCode::Char('?'));

        assert!(shows(
            &screen_with_height(&state, 24),
            "Write a query, or edit"
        ));
    }

    #[test]
    fn the_key_help_in_query_mode_lists_the_query_keys_at_full_width() {
        let mut state = with_results();
        press(&mut state, KeyCode::Char('n'));
        press(&mut state, KeyCode::Esc);
        press(&mut state, KeyCode::Char('?'));

        let screen = screen_with_height(&state, 24);

        for (_, action) in QUERY_HELP {
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
        let document = panes(buffer.area, Mode::Browse, false, None).document;
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

    #[test]
    fn a_long_document_shows_a_scrollbar_on_its_right_border() {
        let mut state = with_results();
        assert!(!shows(&screen(&state), "█"));

        let fields = (0..40).map(|i| (format!("f{i:02}"), json!(i)));
        state.results[0] = serde_json::Value::Object(fields.collect());

        let screen = screen(&state);
        assert!(screen[4].ends_with("█"), "{}", screen.join("\n"));
        assert!(!screen[16].ends_with("█"), "{}", screen.join("\n"));
    }

    #[test]
    fn more_results_than_fit_show_a_scrollbar_on_their_right_border() {
        let mut state = with_results();
        let results = Size::new(100, 20);
        let column = usize::from(
            panes(
                Rect::from((Position::ORIGIN, results)),
                Mode::Browse,
                false,
                None,
            )
            .results
            .right()
                - 1,
        );
        let thumb = |screen: &[String]| {
            screen
                .iter()
                .any(|line| line.chars().nth(column) == Some('█'))
        };
        assert!(!thumb(&screen(&state)));

        state.results = (0..40).map(|i| json!({ "id": format!("c-{i}") })).collect();

        assert!(thumb(&screen(&state)), "{}", screen(&state).join("\n"));
    }

    #[test]
    fn more_accounts_than_fit_show_a_scrollbar_on_the_tree_border() {
        let (mut state, _) = AppState::new();
        let size = Rect::from((Position::ORIGIN, Size::new(100, 20)));
        let column = usize::from(panes(size, Mode::Browse, false, None).tree.right() - 1);
        let thumb = |screen: &[String]| {
            screen
                .iter()
                .any(|line| line.chars().nth(column) == Some('█'))
        };
        assert!(!thumb(&screen(&browsing())));

        let accounts = (0..40).map(|i| account(&format!("a{i:02}"))).collect();
        send(&mut state, Msg::AccountsLoaded(Ok(accounts)));

        assert!(thumb(&screen(&state)), "{}", screen(&state).join("\n"));
    }
}
