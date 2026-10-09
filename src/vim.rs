//! Vim-style modal editing of an [`Editor`].

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::editor::{Editor, Pos};

/// How keys act on an editor.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum VimMode {
    /// Keys move the cursor and run commands.
    #[default]
    Normal,
    /// Keys type text.
    Insert,
    /// Keys pick the characters from where it started to the cursor.
    Visual,
    /// Keys pick the lines from where it started to the cursor.
    VisualLine,
}

impl VimMode {
    /// The name shown for the mode.
    pub fn label(self) -> &'static str {
        match self {
            VimMode::Normal => "NORMAL",
            VimMode::Insert => "INSERT",
            VimMode::Visual => "VISUAL",
            VimMode::VisualLine => "V-LINE",
        }
    }
}

/// A command that acts on the text a motion moves over.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Operator {
    Delete,
    Change,
    Yank,
}

impl Operator {
    fn of(c: char) -> Option<Self> {
        match c {
            'd' => Some(Operator::Delete),
            'c' => Some(Operator::Change),
            'y' => Some(Operator::Yank),
            _ => None,
        }
    }
}

/// The mode of an editor, and the keys of a command typed so far.
#[derive(Debug, Clone, Default)]
pub struct Vim {
    pub mode: VimMode,
    /// The count typed before a command.
    count: Option<usize>,
    /// An operator waiting for its motion, with the count typed before it.
    operator: Option<(Operator, usize)>,
    /// Whether a `g` waits for a second one.
    g: bool,
    /// Whether an `r` waits for the character to put in.
    replace: bool,
    /// Whether an `i` after an operator waits for `w`.
    inner: bool,
    /// Where the visual selection started.
    anchor: Pos,
}

impl Vim {
    /// Whether the editor is in normal mode with no command half typed, so keys
    /// it has no use for can go to the pane.
    pub fn is_idle(&self) -> bool {
        self.mode == VimMode::Normal
            && self.count.is_none()
            && self.operator.is_none()
            && !self.g
            && !self.replace
            && !self.inner
    }

    /// The first and last picked places in a visual mode, whole lines reaching
    /// past their last character in visual line mode.
    pub fn selection(&self, cursor: Pos) -> Option<(Pos, Pos)> {
        let (first, last) = (self.anchor.min(cursor), self.anchor.max(cursor));
        match self.mode {
            VimMode::Visual => Some((first, last)),
            VimMode::VisualLine => Some(((first.0, 0), (last.0, usize::MAX))),
            VimMode::Normal | VimMode::Insert => None,
        }
    }

    fn reset(&mut self) {
        self.count = None;
        self.operator = None;
        self.g = false;
        self.replace = false;
        self.inner = false;
    }
}

/// Text that was deleted or yanked, to put back with `p`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Register {
    pub text: String,
    /// Whether it is whole lines, put back above or below the cursor's line.
    pub linewise: bool,
}

/// What a key did.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    Handled,
    /// Text was yanked, to copy to the clipboard too.
    Yanked(String),
    /// Vim has no use for the key in normal mode, so the pane may.
    Unhandled,
    /// Esc in normal mode, with nothing to cancel.
    Leave,
}

/// How far a motion's text reaches.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// Up to the place it moves to, without it.
    Exclusive,
    /// Up to the place it moves to, with it.
    Inclusive,
    /// The whole lines from the cursor's to the one it moves to.
    Linewise,
}

#[derive(Debug, Clone, Copy)]
struct Motion {
    to: Pos,
    kind: Kind,
}

/// Acts on a key the way Vim would, moving a page by this many lines.
pub fn key(editor: &mut Editor, key: KeyEvent, page: usize, register: &mut Register) -> Outcome {
    let mut vim = std::mem::take(&mut editor.vim);
    let outcome = match vim.mode {
        VimMode::Insert => insert_key(&mut vim, editor, key, page.max(1)),
        _ => normal_key(&mut vim, editor, key, page.max(1), register),
    };
    if vim.mode != VimMode::Insert {
        editor.clamp_to_line();
    }
    editor.vim = vim;
    outcome
}

fn insert_key(vim: &mut Vim, editor: &mut Editor, key: KeyEvent, page: usize) -> Outcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    match key.code {
        KeyCode::Esc => {
            vim.mode = VimMode::Normal;
            let (row, column) = editor.cursor();
            editor.move_to(row, column.saturating_sub(1));
        }
        KeyCode::Char(c) if !ctrl => editor.insert(c),
        KeyCode::Enter => editor.newline(),
        KeyCode::Backspace => editor.backspace(),
        KeyCode::Delete => editor.delete(),
        KeyCode::Left => editor.left(),
        KeyCode::Right => editor.right(),
        KeyCode::Up => editor.up(),
        KeyCode::Down => editor.down(),
        KeyCode::Home => editor.home(),
        KeyCode::End => editor.end(),
        KeyCode::PageUp => editor.page_up(page),
        KeyCode::PageDown => editor.page_down(page),
        _ => return Outcome::Unhandled,
    }
    Outcome::Handled
}

fn normal_key(
    vim: &mut Vim,
    editor: &mut Editor,
    key: KeyEvent,
    page: usize,
    register: &mut Register,
) -> Outcome {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let pending = !vim.is_idle() && vim.mode == VimMode::Normal;
    if vim.replace {
        vim.reset();
        if let KeyCode::Char(c) = key.code
            && !ctrl
        {
            editor.checkpoint();
            editor.replace_char(c);
        }
        return Outcome::Handled;
    }
    if vim.inner {
        let operator = vim.operator.take();
        vim.reset();
        if let (Some((operator, _)), KeyCode::Char('w')) = (operator, key.code) {
            let (first, last) = editor.word_around(editor.cursor());
            return on_chars(vim, editor, operator, first, after(editor, last), register);
        }
        return Outcome::Handled;
    }
    match key.code {
        KeyCode::Char(d) if !ctrl && d.is_ascii_digit() && (d != '0' || vim.count.is_some()) => {
            let digit = d.to_digit(10).map_or(0, |d| d as usize);
            vim.count = Some(
                vim.count
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit),
            );
            return Outcome::Handled;
        }
        _ => {}
    }
    let typed = vim.count.take();
    let count = typed.unwrap_or(1) * vim.operator.map_or(1, |(_, before)| before);
    let g = std::mem::take(&mut vim.g);
    // `cw` changes to the end of the word, like `ce`
    let code = match (vim.operator, key.code) {
        (Some((Operator::Change, _)), KeyCode::Char('w'))
            if editor
                .char_at(editor.cursor())
                .is_some_and(|c| !c.is_whitespace()) =>
        {
            KeyCode::Char('e')
        }
        (_, code) => code,
    };
    if let Some(motion) = motion(editor, code, ctrl, g, count, typed, page) {
        let Some((operator, _)) = vim.operator.take() else {
            editor.move_to(motion.to.0, motion.to.1);
            return Outcome::Handled;
        };
        return operate(vim, editor, operator, motion, code, register);
    }
    if g {
        vim.operator = None;
        return Outcome::Handled;
    }
    if matches!(vim.mode, VimMode::Visual | VimMode::VisualLine) {
        return visual_key(vim, editor, code, ctrl, register);
    }
    let (row, column) = editor.cursor();
    let len = editor.line_len(row);
    match code {
        KeyCode::Esc if pending => vim.reset(),
        KeyCode::Esc => return Outcome::Leave,
        KeyCode::Char('g') if !ctrl => {
            vim.g = true;
            vim.count = typed;
        }
        KeyCode::Char(c) if !ctrl && Operator::of(c).is_some() => {
            let Some(operator) = Operator::of(c) else {
                return Outcome::Handled;
            };
            return match vim.operator.take() {
                Some((pending, _)) if pending == operator => {
                    on_lines(vim, editor, operator, row, count, register)
                }
                Some(_) => Outcome::Handled,
                None => {
                    vim.operator = Some((operator, typed.unwrap_or(1)));
                    Outcome::Handled
                }
            };
        }
        KeyCode::Char('i') if !ctrl && vim.operator.is_some() => vim.inner = true,
        _ if vim.operator.take().is_some() => {}
        KeyCode::Char('i') if !ctrl => insert(vim, editor),
        KeyCode::Char('a') if !ctrl => {
            if len > 0 {
                editor.move_to(row, column + 1);
            }
            insert(vim, editor);
        }
        KeyCode::Char('I') => {
            editor.move_to(row, editor.first_non_blank(row));
            insert(vim, editor);
        }
        KeyCode::Char('A') => {
            editor.end();
            insert(vim, editor);
        }
        KeyCode::Char('o') if !ctrl => {
            insert(vim, editor);
            editor.open_below();
        }
        KeyCode::Char('O') => {
            insert(vim, editor);
            editor.open_above();
        }
        KeyCode::Char('x') if !ctrl && len > 0 => {
            let to = (row, (column + count).min(len));
            return on_chars(vim, editor, Operator::Delete, (row, column), to, register);
        }
        KeyCode::Char('X') if column > 0 => {
            let from = (row, column.saturating_sub(count));
            return on_chars(vim, editor, Operator::Delete, from, (row, column), register);
        }
        KeyCode::Char('D') => {
            return on_chars(
                vim,
                editor,
                Operator::Delete,
                (row, column),
                (row, len),
                register,
            );
        }
        KeyCode::Char('C') => {
            return on_chars(
                vim,
                editor,
                Operator::Change,
                (row, column),
                (row, len),
                register,
            );
        }
        KeyCode::Char('Y') => return on_lines(vim, editor, Operator::Yank, row, count, register),
        KeyCode::Char('J') => {
            editor.checkpoint();
            (0..count.saturating_sub(1).max(1)).for_each(|_| editor.join_below());
        }
        KeyCode::Char('r') if !ctrl => vim.replace = true,
        KeyCode::Char('p') if !ctrl => paste(editor, register, true, count),
        KeyCode::Char('P') => paste(editor, register, false, count),
        KeyCode::Char('u') if !ctrl => (0..count).for_each(|_| {
            editor.undo();
        }),
        KeyCode::Char('r') => (0..count).for_each(|_| {
            editor.redo();
        }),
        KeyCode::Char('v') if !ctrl => {
            vim.mode = VimMode::Visual;
            vim.anchor = (row, column);
        }
        KeyCode::Char('V') => {
            vim.mode = VimMode::VisualLine;
            vim.anchor = (row, column);
        }
        _ => return Outcome::Unhandled,
    }
    Outcome::Handled
}

/// Keys in a visual mode that are not motions: operators on the picked text,
/// or leaving the mode.
fn visual_key(
    vim: &mut Vim,
    editor: &mut Editor,
    code: KeyCode,
    ctrl: bool,
    register: &mut Register,
) -> Outcome {
    let line = vim.mode == VimMode::VisualLine;
    let operator = match code {
        KeyCode::Esc => None,
        KeyCode::Char('v') if !ctrl && line => {
            vim.mode = VimMode::Visual;
            return Outcome::Handled;
        }
        KeyCode::Char('V') if !line => {
            vim.mode = VimMode::VisualLine;
            return Outcome::Handled;
        }
        KeyCode::Char('v' | 'V') if !ctrl => None,
        KeyCode::Char('x') if !ctrl => Some(Operator::Delete),
        KeyCode::Char(c) if !ctrl => match Operator::of(c) {
            Some(operator) => Some(operator),
            None => return Outcome::Unhandled,
        },
        _ => return Outcome::Unhandled,
    };
    let Some((first, last)) = vim.selection(editor.cursor()) else {
        return Outcome::Handled;
    };
    vim.mode = VimMode::Normal;
    let Some(operator) = operator else {
        return Outcome::Handled;
    };
    if line {
        return on_lines(
            vim,
            editor,
            operator,
            first.0,
            last.0 - first.0 + 1,
            register,
        );
    }
    on_chars(vim, editor, operator, first, after(editor, last), register)
}

/// Where a motion key moves the cursor, or `None` for keys that are not motions.
fn motion(
    editor: &Editor,
    code: KeyCode,
    ctrl: bool,
    g: bool,
    count: usize,
    typed: Option<usize>,
    page: usize,
) -> Option<Motion> {
    let (row, column) = editor.cursor();
    let last_row = editor.lines().len() - 1;
    let half = (page / 2).max(1);
    let exclusive = |to| {
        Some(Motion {
            to,
            kind: Kind::Exclusive,
        })
    };
    let lines = |row: usize, column: Option<usize>| {
        let row = row.min(last_row);
        let column = column.unwrap_or_else(|| editor.first_non_blank(row));
        Some(Motion {
            to: (row, column),
            kind: Kind::Linewise,
        })
    };
    let repeat =
        |step: fn(&Editor, Pos) -> Pos| (0..count).fold((row, column), |at, _| step(editor, at));
    match code {
        KeyCode::Char('g') if g => lines(typed.map_or(0, |n| n.saturating_sub(1)), None),
        KeyCode::Char('d') if ctrl => lines(row + half * count, Some(column)),
        KeyCode::Char('u') if ctrl => lines(row.saturating_sub(half * count), Some(column)),
        KeyCode::Char('f') if ctrl => lines(row + page * count, Some(column)),
        KeyCode::Char('b') if ctrl => lines(row.saturating_sub(page * count), Some(column)),
        KeyCode::PageDown => lines(row + page * count, Some(column)),
        KeyCode::PageUp => lines(row.saturating_sub(page * count), Some(column)),
        _ if ctrl => None,
        KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
            exclusive((row, column.saturating_sub(count)))
        }
        KeyCode::Char('l' | ' ') | KeyCode::Right => {
            exclusive((row, (column + count).min(editor.line_len(row))))
        }
        KeyCode::Char('j') | KeyCode::Down => lines(row + count, Some(column)),
        KeyCode::Char('k') | KeyCode::Up => lines(row.saturating_sub(count), Some(column)),
        KeyCode::Char('G') => lines(typed.map_or(last_row, |n| n.saturating_sub(1)), None),
        KeyCode::Char('w') => exclusive(repeat(Editor::word_forward)),
        KeyCode::Char('b') => exclusive(repeat(Editor::word_back)),
        KeyCode::Char('e') => Some(Motion {
            to: repeat(Editor::word_end),
            kind: Kind::Inclusive,
        }),
        KeyCode::Char('0') | KeyCode::Home => exclusive((row, 0)),
        KeyCode::Char('^') => exclusive((row, editor.first_non_blank(row))),
        KeyCode::Char('$') | KeyCode::End => {
            let row = (row + count - 1).min(last_row);
            Some(Motion {
                to: (row, editor.line_len(row).saturating_sub(1)),
                kind: Kind::Inclusive,
            })
        }
        _ => None,
    }
}

/// Acts on the text from the cursor to where a motion moves it.
fn operate(
    vim: &mut Vim,
    editor: &mut Editor,
    operator: Operator,
    motion: Motion,
    code: KeyCode,
    register: &mut Register,
) -> Outcome {
    let cursor = editor.cursor();
    let (first, last) = (cursor.min(motion.to), cursor.max(motion.to));
    match motion.kind {
        Kind::Linewise => on_lines(
            vim,
            editor,
            operator,
            first.0,
            last.0 - first.0 + 1,
            register,
        ),
        Kind::Inclusive => on_chars(vim, editor, operator, first, after(editor, last), register),
        Kind::Exclusive => {
            // A word motion that went on to another line stops at the end of the first
            let last = if code == KeyCode::Char('w') && last.0 > first.0 {
                (first.0, editor.line_len(first.0))
            } else {
                last
            };
            on_chars(vim, editor, operator, first, last, register)
        }
    }
}

/// Just after a place, not past the end of its line.
fn after(editor: &Editor, (row, column): Pos) -> Pos {
    (row, (column + 1).min(editor.line_len(row)))
}

/// Deletes, changes or yanks the text from one place up to another.
fn on_chars(
    vim: &mut Vim,
    editor: &mut Editor,
    operator: Operator,
    from: Pos,
    to: Pos,
    register: &mut Register,
) -> Outcome {
    if operator == Operator::Yank {
        let text = editor.text_between(from, to);
        editor.move_to(from.0, from.1);
        return yank(register, text, false);
    }
    editor.checkpoint();
    register.text = editor.delete_between(from, to);
    register.linewise = false;
    if operator == Operator::Change {
        vim.mode = VimMode::Insert;
    }
    Outcome::Handled
}

/// Deletes, changes or yanks this many whole lines from a line on.
fn on_lines(
    vim: &mut Vim,
    editor: &mut Editor,
    operator: Operator,
    row: usize,
    count: usize,
    register: &mut Register,
) -> Outcome {
    let text = match operator {
        Operator::Yank => {
            let text = editor.lines_text(row, count);
            editor.move_to(row, editor.cursor().1);
            return yank(register, text, true);
        }
        Operator::Delete => {
            editor.checkpoint();
            editor.delete_lines(row, count)
        }
        Operator::Change => {
            editor.checkpoint();
            vim.mode = VimMode::Insert;
            editor.clear_lines(row, count)
        }
    };
    *register = Register {
        text,
        linewise: true,
    };
    Outcome::Handled
}

fn yank(register: &mut Register, text: String, linewise: bool) -> Outcome {
    *register = Register {
        text: text.clone(),
        linewise,
    };
    Outcome::Yanked(text)
}

/// Starts typing, as one change to undo.
fn insert(vim: &mut Vim, editor: &mut Editor) {
    editor.checkpoint();
    vim.mode = VimMode::Insert;
}

/// Puts the register back after or before the cursor, or below or above its
/// line when it holds whole lines.
fn paste(editor: &mut Editor, register: &Register, after: bool, count: usize) {
    if register.text.is_empty() && !register.linewise {
        return;
    }
    editor.checkpoint();
    let (row, column) = editor.cursor();
    if register.linewise {
        let text = vec![register.text.as_str(); count].join("\n");
        editor.insert_lines(if after { row + 1 } else { row }, &text);
        return;
    }
    if after && editor.line_len(row) > 0 {
        editor.move_to(row, column + 1);
    }
    editor.insert_text(&register.text.repeat(count));
    let (row, column) = editor.cursor();
    editor.move_to(row, column.saturating_sub(1));
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Typing {
        editor: Editor,
        register: Register,
    }

    impl Typing {
        fn new(text: &str) -> Self {
            let mut editor = Editor::default();
            editor.set_text(text);
            editor.move_to(0, 0);
            Typing {
                editor,
                register: Register::default(),
            }
        }

        fn press(&mut self, key: KeyEvent) -> Outcome {
            key_on(self, key)
        }

        /// Types each character as a key, `⎋` standing for Esc.
        fn keys(&mut self, keys: &str) -> Outcome {
            let mut outcome = Outcome::Handled;
            for c in keys.chars() {
                let code = match c {
                    '⎋' => KeyCode::Esc,
                    c => KeyCode::Char(c),
                };
                outcome = self.press(KeyEvent::from(code));
            }
            outcome
        }

        fn text(&self) -> String {
            self.editor.text()
        }

        fn cursor(&self) -> Pos {
            self.editor.cursor()
        }
    }

    fn key_on(typing: &mut Typing, event: KeyEvent) -> Outcome {
        key(&mut typing.editor, event, 10, &mut typing.register)
    }

    #[test]
    fn starts_in_normal_mode_and_types_only_after_i() {
        let mut typing = Typing::new("FROM c");

        typing.keys("iSELECT * ⎋");

        assert_eq!(typing.text(), "SELECT * FROM c");
        assert_eq!(typing.editor.vim.mode, VimMode::Normal);
        assert_eq!(typing.cursor(), (0, 8));
    }

    #[test]
    fn a_and_capital_a_append_after_the_cursor_and_at_the_end() {
        let mut typing = Typing::new("ac");

        typing.keys("ab⎋Ad⎋");

        assert_eq!(typing.text(), "abcd");
    }

    #[test]
    fn moves_with_counts() {
        let mut typing = Typing::new("a\nb\nc\nd");

        typing.keys("3j");
        assert_eq!(typing.cursor(), (3, 0));
        typing.keys("2k");
        assert_eq!(typing.cursor(), (1, 0));
        typing.keys("G");
        assert_eq!(typing.cursor(), (3, 0));
        typing.keys("gg");
        assert_eq!(typing.cursor(), (0, 0));
        typing.keys("2G");
        assert_eq!(typing.cursor(), (1, 0));
    }

    #[test]
    fn normal_mode_keeps_the_cursor_on_a_character() {
        let mut typing = Typing::new("abc");

        typing.keys("$");
        assert_eq!(typing.cursor(), (0, 2));
        typing.keys("l");
        assert_eq!(typing.cursor(), (0, 2));
    }

    #[test]
    fn dd_deletes_lines_and_p_puts_them_below() {
        let mut typing = Typing::new("a\nb\nc");

        typing.keys("ddp");

        assert_eq!(typing.text(), "b\na\nc");
        assert_eq!(typing.cursor(), (1, 0));
        typing.keys("2dd");
        assert_eq!(typing.text(), "b");
    }

    #[test]
    fn dw_deletes_to_the_next_word_but_not_past_the_line() {
        let mut typing = Typing::new("SELECT c.id\nFROM c");

        typing.keys("dw");
        assert_eq!(typing.text(), "c.id\nFROM c");
        typing.keys("$dw");
        assert_eq!(typing.text(), "c.i\nFROM c");
    }

    #[test]
    fn cw_changes_to_the_end_of_the_word() {
        let mut typing = Typing::new("SELECT * FROM c");

        typing.keys("cwselect⎋");

        assert_eq!(typing.text(), "select * FROM c");
    }

    #[test]
    fn ciw_changes_the_word_under_the_cursor() {
        let mut typing = Typing::new("  \"name\": 1");
        typing.keys("4l");

        typing.keys("ciwtitle⎋");

        assert_eq!(typing.text(), "  \"title\": 1");
    }

    #[test]
    fn yy_yanks_the_line_for_the_clipboard_and_p_puts_it() {
        let mut typing = Typing::new("a\nb");

        assert_eq!(typing.keys("yy"), Outcome::Yanked("a".into()));
        typing.keys("jp");

        assert_eq!(typing.text(), "a\nb\na");
    }

    #[test]
    fn x_deletes_characters_and_p_puts_them_after_the_cursor() {
        let mut typing = Typing::new("abc");

        typing.keys("xp");

        assert_eq!(typing.text(), "bac");
        assert_eq!(typing.cursor(), (0, 1));
    }

    #[test]
    fn d_dollar_and_capital_d_delete_to_the_end_of_the_line() {
        let mut typing = Typing::new("abc\ndef");

        typing.keys("ld$");
        assert_eq!(typing.text(), "a\ndef");
        typing.keys("jlD");
        assert_eq!(typing.text(), "a\nd");
    }

    #[test]
    fn o_and_capital_o_open_lines() {
        let mut typing = Typing::new("b");

        typing.keys("oc⎋Oa⎋");

        assert_eq!(typing.text(), "b\na\nc");
    }

    #[test]
    fn u_undoes_a_whole_insert_and_ctrl_r_redoes_it() {
        let mut typing = Typing::new("a");

        typing.keys("Abc⎋");
        typing.keys("u");
        assert_eq!(typing.text(), "a");
        typing.press(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL));
        assert_eq!(typing.text(), "abc");
    }

    #[test]
    fn r_replaces_a_character_and_j_joins_lines() {
        let mut typing = Typing::new("cat\n  dog");

        typing.keys("lruJ");

        assert_eq!(typing.text(), "cut dog");
    }

    #[test]
    fn visual_mode_deletes_the_picked_characters() {
        let mut typing = Typing::new("SELECT * FROM c");

        typing.keys("vlld");

        assert_eq!(typing.text(), "ECT * FROM c");
        assert_eq!(typing.editor.vim.mode, VimMode::Normal);
    }

    #[test]
    fn visual_line_mode_yanks_whole_lines() {
        let mut typing = Typing::new("a\nb\nc");

        let outcome = typing.keys("Vjy");

        assert_eq!(outcome, Outcome::Yanked("a\nb".into()));
        assert_eq!(typing.cursor(), (0, 0));
        assert_eq!(typing.editor.vim.mode, VimMode::Normal);
    }

    #[test]
    fn the_selection_runs_from_where_visual_mode_started() {
        let mut typing = Typing::new("abc\ndef");
        typing.keys("lvj");

        assert_eq!(
            typing.editor.vim.selection(typing.cursor()),
            Some(((0, 1), (1, 1)))
        );
        typing.keys("V");
        assert_eq!(
            typing.editor.vim.selection(typing.cursor()),
            Some(((0, 0), (1, usize::MAX)))
        );
    }

    #[test]
    fn keys_vim_has_no_use_for_are_left_to_the_pane() {
        let mut typing = Typing::new("a");

        assert_eq!(typing.keys("q"), Outcome::Unhandled);
        assert_eq!(typing.keys("?"), Outcome::Unhandled);
        assert!(typing.editor.vim.is_idle());
    }

    #[test]
    fn esc_cancels_a_half_typed_command_then_leaves() {
        let mut typing = Typing::new("a\nb");

        assert_eq!(typing.keys("d⎋"), Outcome::Handled);
        assert_eq!(typing.text(), "a\nb");
        assert_eq!(typing.keys("⎋"), Outcome::Leave);
    }

    #[test]
    fn a_count_multiplies_the_count_of_the_motion() {
        let mut typing = Typing::new("a b c d e f g");

        typing.keys("2d2w");

        assert_eq!(typing.text(), "e f g");
    }
}
