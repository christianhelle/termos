//! Several lines of editable text, for writing queries.

use crate::vim::Vim;

/// A place in the text, as the line and the characters from its start.
pub type Pos = (usize, usize);

/// Lines of text with a cursor, counted in lines and characters.
#[derive(Debug)]
pub struct Editor {
    lines: Vec<String>,
    row: usize,
    column: usize,
    /// The first line shown, kept until the cursor leaves the lines shown.
    top: usize,
    /// The text before each change, latest last, to undo them.
    undone: Vec<Snapshot>,
    /// The text before each undo, latest last, to redo them.
    redone: Vec<Snapshot>,
    /// How keys act on the text.
    pub vim: Vim,
}

/// The text and cursor at some moment.
#[derive(Debug, Clone)]
struct Snapshot {
    lines: Vec<String>,
    row: usize,
    column: usize,
}

/// How many changes can be undone.
const UNDO_LIMIT: usize = 200;

impl Default for Editor {
    fn default() -> Self {
        Editor {
            lines: vec![String::new()],
            row: 0,
            column: 0,
            top: 0,
            undone: Vec::new(),
            redone: Vec::new(),
            vim: Vim::default(),
        }
    }
}

impl Editor {
    /// The text, with its lines joined by newlines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replaces the text, putting the cursor at its end, in normal mode with nothing to undo.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(String::from).collect();
        self.row = self.lines.len() - 1;
        self.column = self.len();
        self.undone.clear();
        self.redone.clear();
        self.vim = Vim::default();
    }

    /// Remembers the text as it is, for the change about to be made to be undone.
    pub fn checkpoint(&mut self) {
        let snapshot = self.snapshot();
        self.undone.push(snapshot);
        if self.undone.len() > UNDO_LIMIT {
            self.undone.remove(0);
        }
        self.redone.clear();
    }

    /// Takes the text back to before the latest change, if there is one.
    pub fn undo(&mut self) -> bool {
        let Some(before) = self.undone.pop() else {
            return false;
        };
        let now = self.snapshot();
        self.redone.push(now);
        self.restore(before);
        true
    }

    /// Makes the latest undone change again, if there is one.
    pub fn redo(&mut self) -> bool {
        let Some(after) = self.redone.pop() else {
            return false;
        };
        let now = self.snapshot();
        self.undone.push(now);
        self.restore(after);
        true
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            lines: self.lines.clone(),
            row: self.row,
            column: self.column,
        }
    }

    fn restore(&mut self, snapshot: Snapshot) {
        self.lines = snapshot.lines;
        self.move_to(snapshot.row, snapshot.column);
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Where the cursor is, as the line and the characters from its start.
    pub fn cursor(&self) -> (usize, usize) {
        (self.row, self.column)
    }

    pub fn insert(&mut self, c: char) {
        let at = self.byte_index(self.column);
        self.lines[self.row].insert(at, c);
        self.column += 1;
    }

    /// Breaks the line at the cursor, moving the cursor to the start of the new line.
    pub fn newline(&mut self) {
        let at = self.byte_index(self.column);
        let rest = self.lines[self.row].split_off(at);
        self.row += 1;
        self.column = 0;
        self.lines.insert(self.row, rest);
    }

    /// Deletes the character before the cursor, or joins the line to the one above at its start.
    pub fn backspace(&mut self) {
        if self.column > 0 {
            self.column -= 1;
            let at = self.byte_index(self.column);
            self.lines[self.row].remove(at);
        } else if self.row > 0 {
            let line = self.lines.remove(self.row);
            self.row -= 1;
            self.column = self.len();
            self.lines[self.row].push_str(&line);
        }
    }

    /// Puts the cursor on a line and column, as near as the text allows.
    pub fn move_to(&mut self, row: usize, column: usize) {
        self.row = row.min(self.lines.len() - 1);
        self.column = column.min(self.len());
    }

    /// The first line to show, when this many lines show at once:
    /// where it was, scrolled only as far as it takes to show the cursor.
    pub fn scroll(&self, height: usize) -> usize {
        let height = height.max(1);
        if self.row < self.top {
            self.row
        } else if self.row >= self.top + height {
            self.row + 1 - height
        } else {
            self.top
        }
    }

    /// Keeps the lines shown where they are, unless the cursor left them.
    pub fn follow(&mut self, height: usize) {
        self.top = self.scroll(height);
    }

    /// How many characters the widest line number takes, at least two.
    pub fn number_width(&self) -> usize {
        self.lines.len().to_string().len().max(2)
    }

    /// Deletes the character under the cursor, or joins the line below at the end of a line.
    pub fn delete(&mut self) {
        if self.column < self.len() {
            let at = self.byte_index(self.column);
            self.lines[self.row].remove(at);
        } else if self.row + 1 < self.lines.len() {
            let line = self.lines.remove(self.row + 1);
            self.lines[self.row].push_str(&line);
        }
    }

    /// Moves back a character, to the end of the line above from the start of a line.
    pub fn left(&mut self) {
        if self.column > 0 {
            self.column -= 1;
        } else if self.row > 0 {
            self.row -= 1;
            self.column = self.len();
        }
    }

    /// Moves on a character, to the start of the line below from the end of a line.
    pub fn right(&mut self) {
        if self.column < self.len() {
            self.column += 1;
        } else if self.row + 1 < self.lines.len() {
            self.row += 1;
            self.column = 0;
        }
    }

    pub fn up(&mut self) {
        self.move_to_row(self.row.saturating_sub(1));
    }

    pub fn down(&mut self) {
        self.move_to_row(self.row + 1);
    }

    pub fn home(&mut self) {
        self.column = 0;
    }

    pub fn end(&mut self) {
        self.column = self.len();
    }

    /// Moves up this many lines, stopping at the first.
    pub fn page_up(&mut self, lines: usize) {
        self.move_to_row(self.row.saturating_sub(lines));
    }

    /// Moves down this many lines, stopping at the last.
    pub fn page_down(&mut self, lines: usize) {
        self.move_to_row(self.row + lines);
    }

    /// Puts the cursor on a line, keeping its column within the line.
    fn move_to_row(&mut self, row: usize) {
        self.move_to(row, self.column);
    }

    /// The text from one place up to another, which it leaves out.
    pub fn text_between(&self, from: Pos, to: Pos) -> String {
        let (start, end) = (self.offset(from), self.offset(to));
        self.text()
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect()
    }

    /// Deletes the text from one place up to another, putting the cursor
    /// where it started, and returns it.
    pub fn delete_between(&mut self, from: Pos, to: Pos) -> String {
        let (start, end) = (self.offset(from), self.offset(to));
        let text = self.text();
        let deleted = text
            .chars()
            .skip(start)
            .take(end.saturating_sub(start))
            .collect();
        let kept: String = text
            .chars()
            .take(start)
            .chain(text.chars().skip(end.max(start)))
            .collect();
        self.lines = kept.split('\n').map(String::from).collect();
        self.move_to(from.0, from.1);
        deleted
    }

    /// The text of this many lines from a line on, joined by newlines.
    pub fn lines_text(&self, row: usize, count: usize) -> String {
        let lines: Vec<&str> = self
            .lines
            .iter()
            .skip(row)
            .take(count.max(1))
            .map(String::as_str)
            .collect();
        lines.join("\n")
    }

    /// Deletes this many lines from a line on, putting the cursor on the first
    /// character of the line that takes their place, and returns them.
    pub fn delete_lines(&mut self, row: usize, count: usize) -> String {
        let text = self.lines_text(row, count);
        let end = (row + count.max(1)).min(self.lines.len());
        self.lines.drain(row.min(end)..end);
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        let row = row.min(self.lines.len() - 1);
        self.move_to(row, self.first_non_blank(row));
        text
    }

    /// Empties this many lines from a line on into one, keeping its indent,
    /// with the cursor at its end, and returns them.
    pub fn clear_lines(&mut self, row: usize, count: usize) -> String {
        let text = self.lines_text(row, count);
        let end = (row + count.max(1)).min(self.lines.len());
        let indent = self.indent(row);
        self.lines.splice(row..end, [indent]);
        self.row = row;
        self.end();
        text
    }

    /// Adds lines of text before a line, or after the last, with the cursor
    /// on the first character of the first of them.
    pub fn insert_lines(&mut self, row: usize, text: &str) {
        let row = row.min(self.lines.len());
        self.lines
            .splice(row..row, text.split('\n').map(String::from));
        self.move_to(row, self.first_non_blank(row));
    }

    /// Adds text at the cursor, breaking lines at its newlines, with the cursor after it.
    pub fn insert_text(&mut self, text: &str) {
        for c in text.chars() {
            match c {
                '\n' => self.newline(),
                c => self.insert(c),
            }
        }
    }

    /// Adds an empty line below the cursor's, indented like it, and moves there.
    pub fn open_below(&mut self) {
        let indent = self.indent(self.row);
        self.lines.insert(self.row + 1, indent);
        self.row += 1;
        self.end();
    }

    /// Adds an empty line above the cursor's, indented like it, and moves there.
    pub fn open_above(&mut self) {
        let indent = self.indent(self.row);
        self.lines.insert(self.row, indent);
        self.end();
    }

    /// Joins the line below to the cursor's with a space, without its indent,
    /// with the cursor where they meet.
    pub fn join_below(&mut self) {
        if self.row + 1 >= self.lines.len() {
            return;
        }
        let below = self.lines.remove(self.row + 1);
        let below = below.trim_start();
        let line = &mut self.lines[self.row];
        line.truncate(line.trim_end().len());
        self.column = line.chars().count();
        if !line.is_empty() && !below.is_empty() {
            line.push(' ');
        }
        line.push_str(below);
    }

    /// Replaces the character under the cursor.
    pub fn replace_char(&mut self, c: char) {
        if self.column < self.len() {
            self.delete();
            self.insert(c);
            self.column -= 1;
        }
    }

    /// Keeps the cursor on a character, off the end of its line, as Vim's normal mode does.
    pub fn clamp_to_line(&mut self) {
        self.column = self.column.min(self.len().saturating_sub(1));
    }

    /// The spaces a line starts with.
    fn indent(&self, row: usize) -> String {
        self.lines[row]
            .chars()
            .take_while(|c| c.is_whitespace())
            .collect()
    }

    /// How many characters a place is from the start of the text, newlines included.
    fn offset(&self, (row, column): Pos) -> usize {
        let row = row.min(self.lines.len() - 1);
        let before: usize = self.lines[..row]
            .iter()
            .map(|line| line.chars().count() + 1)
            .sum();
        before + column.min(self.line_len(row) + 1)
    }

    /// The number of characters in a line.
    pub fn line_len(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |line| line.chars().count())
    }

    /// The character at a place, or `None` past the end of its line.
    pub fn char_at(&self, (row, column): Pos) -> Option<char> {
        self.lines.get(row)?.chars().nth(column)
    }

    /// Where the next word starts, on a later line when this one has no more,
    /// stopping at empty lines, or the end of the text.
    pub fn word_forward(&self, from: Pos) -> Pos {
        let start = self.class_at(from);
        let mut at = from;
        if start != Class::Space {
            loop {
                match self.next(at) {
                    None => return self.end_of_text(),
                    Some(next) if next.0 != at.0 => {
                        at = next;
                        break;
                    }
                    Some(next) => at = next,
                }
                if self.class_at(at) != start {
                    break;
                }
            }
        }
        loop {
            if self.class_at(at) != Class::Space {
                return at;
            }
            if at.0 != from.0 && self.line_len(at.0) == 0 {
                return at;
            }
            match self.next(at) {
                Some(next) => at = next,
                None => return self.end_of_text(),
            }
        }
    }

    /// Where the word ends, the one after when already at its end.
    pub fn word_end(&self, from: Pos) -> Pos {
        let Some(mut at) = self.next(from) else {
            return from;
        };
        while self.class_at(at) == Class::Space {
            match self.next(at) {
                Some(next) => at = next,
                None => return at,
            }
        }
        let class = self.class_at(at);
        loop {
            match self.next(at) {
                Some(next) if next.0 == at.0 && self.class_at(next) == class => at = next,
                _ => return at,
            }
        }
    }

    /// Where the word starts, the one before when already at its start.
    pub fn word_back(&self, from: Pos) -> Pos {
        let Some(mut at) = self.previous(from) else {
            return from;
        };
        while self.class_at(at) == Class::Space {
            if at.0 != from.0 && self.line_len(at.0) == 0 {
                return at;
            }
            match self.previous(at) {
                Some(previous) => at = previous,
                None => return at,
            }
        }
        let class = self.class_at(at);
        loop {
            match self.previous(at) {
                Some(previous) if previous.0 == at.0 && self.class_at(previous) == class => {
                    at = previous;
                }
                _ => return at,
            }
        }
    }

    /// The first and last characters of the word or the spaces at a place.
    pub fn word_around(&self, (row, column): Pos) -> (Pos, Pos) {
        let class = self.class_at((row, column));
        let same = |c: usize| self.class_at((row, c)) == class;
        let mut first = column;
        while first > 0 && same(first - 1) {
            first -= 1;
        }
        let mut last = column;
        while last + 1 < self.line_len(row) && same(last + 1) {
            last += 1;
        }
        ((row, first), (row, last))
    }

    /// The column of the first character of a line that is not a space.
    pub fn first_non_blank(&self, row: usize) -> usize {
        self.lines.get(row).map_or(0, |line| {
            line.chars()
                .position(|c| !c.is_whitespace())
                .unwrap_or_else(|| line.chars().count().saturating_sub(1))
        })
    }

    /// Whether a place holds a space, a word character or punctuation.
    fn class_at(&self, at: Pos) -> Class {
        match self.char_at(at) {
            None => Class::Space,
            Some(c) if c.is_whitespace() => Class::Space,
            Some(c) if c.is_alphanumeric() || c == '_' => Class::Word,
            Some(_) => Class::Punctuation,
        }
    }

    /// The character after a place, at the start of the next line after the last.
    fn next(&self, (row, column): Pos) -> Option<Pos> {
        if column + 1 < self.line_len(row) {
            Some((row, column + 1))
        } else if row + 1 < self.lines.len() {
            Some((row + 1, 0))
        } else {
            None
        }
    }

    /// The character before a place, at the end of the line before from the first.
    fn previous(&self, (row, column): Pos) -> Option<Pos> {
        if column > 0 {
            Some((row, column.min(self.line_len(row)) - 1))
        } else if row > 0 {
            Some((row - 1, self.line_len(row - 1).saturating_sub(1)))
        } else {
            None
        }
    }

    /// Just past the last character.
    fn end_of_text(&self) -> Pos {
        let row = self.lines.len() - 1;
        (row, self.line_len(row))
    }

    /// The number of characters in the cursor's line.
    fn len(&self) -> usize {
        self.lines[self.row].chars().count()
    }

    /// The byte offset of a character in the cursor's line, or the line's end.
    fn byte_index(&self, chars: usize) -> usize {
        let line = &self.lines[self.row];
        line.char_indices()
            .nth(chars)
            .map_or(line.len(), |(index, _)| index)
    }
}

/// What kind of character, for moving by words.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Class {
    Space,
    Word,
    Punctuation,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Editor {
        let mut editor = Editor::default();
        text.chars().for_each(|c| editor.insert(c));
        editor
    }

    #[test]
    fn typing_adds_text_at_the_cursor() {
        let editor = typed("SELECT");

        assert_eq!(editor.text(), "SELECT");
        assert_eq!(editor.cursor(), (0, 6));
    }

    #[test]
    fn newline_splits_the_line_at_the_cursor() {
        let mut editor = typed("SELECT * FROM c");
        (0..6).for_each(|_| editor.left());

        editor.newline();

        assert_eq!(editor.text(), "SELECT * \nFROM c");
        assert_eq!(editor.cursor(), (1, 0));
    }

    #[test]
    fn backspace_deletes_before_the_cursor_and_joins_lines_at_the_start() {
        let mut editor = typed("FROM");
        editor.newline();
        "cc".chars().for_each(|c| editor.insert(c));

        editor.backspace();
        assert_eq!(editor.text(), "FROM\nc");
        editor.left();
        editor.backspace();

        assert_eq!(editor.text(), "FROMc");
        assert_eq!(editor.cursor(), (0, 4));
    }

    #[test]
    fn backspace_at_the_very_start_does_nothing() {
        let mut editor = Editor::default();

        editor.backspace();

        assert_eq!(editor.text(), "");
        assert_eq!(editor.cursor(), (0, 0));
    }

    #[test]
    fn left_and_right_move_across_line_ends() {
        let mut editor = typed("ab");
        editor.newline();
        editor.insert('c');
        editor.left();

        editor.left();
        assert_eq!(editor.cursor(), (0, 2));
        editor.right();
        assert_eq!(editor.cursor(), (1, 0));
        editor.right();
        editor.right();
        assert_eq!(editor.cursor(), (1, 1));
    }

    #[test]
    fn up_and_down_keep_the_column_within_the_line() {
        let mut editor = typed("SELECT *");
        editor.newline();
        "FROM c".chars().for_each(|c| editor.insert(c));
        editor.newline();
        editor.insert('x');
        editor.up();
        editor.up();
        editor.right();
        editor.right();
        editor.right();

        editor.down();
        assert_eq!(editor.cursor(), (1, 4));
        editor.down();
        assert_eq!(editor.cursor(), (2, 1));
        editor.down();
        assert_eq!(editor.cursor(), (2, 1));
        editor.up();
        editor.up();
        editor.up();
        assert_eq!(editor.cursor(), (0, 1));
    }

    #[test]
    fn delete_removes_under_the_cursor_and_joins_the_next_line_at_the_end() {
        let mut editor = typed("FROMx");
        editor.newline();
        editor.insert('c');
        editor.left();
        editor.left();
        editor.left();

        editor.delete();
        assert_eq!(editor.text(), "FROM\nc");
        editor.delete();

        assert_eq!(editor.text(), "FROMc");
        assert_eq!(editor.cursor(), (0, 4));
    }

    #[test]
    fn delete_at_the_very_end_does_nothing() {
        let mut editor = typed("c");

        editor.delete();

        assert_eq!(editor.text(), "c");
    }

    #[test]
    fn home_and_end_go_to_the_ends_of_the_line() {
        let mut editor = typed("FROM c");

        editor.home();
        assert_eq!(editor.cursor(), (0, 0));
        editor.end();
        assert_eq!(editor.cursor(), (0, 6));
    }

    #[test]
    fn page_up_and_down_move_lines_at_a_time_within_the_text() {
        let mut editor = typed("a");
        (0..9).for_each(|_| editor.newline());

        editor.page_up(4);
        assert_eq!(editor.cursor(), (5, 0));
        editor.page_up(10);
        assert_eq!(editor.cursor(), (0, 0));
        editor.page_down(4);
        assert_eq!(editor.cursor(), (4, 0));
        editor.page_down(10);
        assert_eq!(editor.cursor(), (9, 0));
    }

    #[test]
    fn the_lines_shown_scroll_only_when_the_cursor_leaves_them() {
        let mut editor = typed("a");
        (0..9).for_each(|_| editor.newline());
        editor.follow(4);
        assert_eq!(editor.scroll(4), 6);

        editor.up();
        editor.follow(4);
        assert_eq!(editor.scroll(4), 6);
        editor.page_up(4);
        editor.follow(4);
        assert_eq!(editor.scroll(4), 4);
    }

    fn text(text: &str) -> Editor {
        let mut editor = Editor::default();
        editor.set_text(text);
        editor
    }

    #[test]
    fn words_start_after_spaces_or_where_punctuation_meets_letters() {
        let editor = text(
            "SELECT c.id

FROM c",
        );

        assert_eq!(editor.word_forward((0, 0)), (0, 7));
        assert_eq!(editor.word_forward((0, 7)), (0, 8));
        assert_eq!(editor.word_forward((0, 8)), (0, 9));
        assert_eq!(editor.word_forward((0, 9)), (1, 0));
        assert_eq!(editor.word_forward((1, 0)), (2, 0));
        assert_eq!(editor.word_forward((2, 5)), (2, 6));
    }

    #[test]
    fn words_end_on_their_last_character() {
        let editor = text(
            "SELECT c.id
FROM",
        );

        assert_eq!(editor.word_end((0, 0)), (0, 5));
        assert_eq!(editor.word_end((0, 5)), (0, 7));
        assert_eq!(editor.word_end((0, 9)), (0, 10));
        assert_eq!(editor.word_end((0, 10)), (1, 3));
    }

    #[test]
    fn moving_back_a_word_goes_to_its_start() {
        let editor = text(
            "SELECT c.id
FROM",
        );

        assert_eq!(editor.word_back((1, 2)), (1, 0));
        assert_eq!(editor.word_back((1, 0)), (0, 9));
        assert_eq!(editor.word_back((0, 9)), (0, 8));
        assert_eq!(editor.word_back((0, 3)), (0, 0));
        assert_eq!(editor.word_back((0, 0)), (0, 0));
    }

    #[test]
    fn the_word_around_a_place_reaches_both_its_ends() {
        let editor = text("  \"name\": 1");

        assert_eq!(editor.word_around((0, 4)), ((0, 3), (0, 6)));
        assert_eq!(editor.word_around((0, 0)), ((0, 0), (0, 1)));
        assert_eq!(editor.first_non_blank(0), 2);
    }

    #[test]
    fn deletes_text_between_places_across_lines() {
        let mut editor = text("SELECT *\nFROM c");

        assert_eq!(editor.text_between((0, 7), (1, 4)), "*\nFROM");
        assert_eq!(editor.delete_between((0, 6), (1, 4)), " *\nFROM");
        assert_eq!(editor.text(), "SELECT c");
        assert_eq!(editor.cursor(), (0, 6));
    }

    #[test]
    fn deletes_whole_lines_and_keeps_one_when_all_go() {
        let mut editor = text("a\n  b\nc");

        assert_eq!(editor.delete_lines(0, 1), "a");
        assert_eq!(editor.cursor(), (0, 2));
        assert_eq!(editor.delete_lines(0, 5), "  b\nc");
        assert_eq!(editor.lines(), [""]);
    }

    #[test]
    fn clearing_lines_keeps_the_indent_of_the_first() {
        let mut editor = text("{\n  \"a\": 1,\n  \"b\": 2\n}");

        assert_eq!(editor.clear_lines(1, 2), "  \"a\": 1,\n  \"b\": 2");
        assert_eq!(editor.text(), "{\n  \n}");
        assert_eq!(editor.cursor(), (1, 2));
    }

    #[test]
    fn inserts_lines_and_text_with_newlines() {
        let mut editor = text("a\nd");
        editor.insert_lines(1, "b\nc");
        assert_eq!(editor.text(), "a\nb\nc\nd");
        assert_eq!(editor.cursor(), (1, 0));

        editor.insert_text("x\ny");
        assert_eq!(editor.text(), "a\nx\nyb\nc\nd");
        assert_eq!(editor.cursor(), (2, 1));
    }

    #[test]
    fn opens_lines_indented_like_the_cursor_line() {
        let mut editor = text("{\n  \"a\": 1\n}");
        editor.move_to(1, 0);

        editor.open_below();
        assert_eq!(editor.cursor(), (2, 2));
        editor.open_above();
        assert_eq!(editor.text(), "{\n  \"a\": 1\n  \n  \n}");
        assert_eq!(editor.cursor(), (2, 2));
    }

    #[test]
    fn joins_the_line_below_without_its_indent() {
        let mut editor = text("SELECT *\n   FROM c");
        editor.move_to(0, 0);

        editor.join_below();

        assert_eq!(editor.text(), "SELECT * FROM c");
        assert_eq!(editor.cursor(), (0, 8));
    }

    #[test]
    fn replaces_the_character_under_the_cursor() {
        let mut editor = text("cat");
        editor.move_to(0, 1);

        editor.replace_char('u');

        assert_eq!(editor.text(), "cut");
        assert_eq!(editor.cursor(), (0, 1));
    }

    #[test]
    fn undoes_and_redoes_changes_back_to_each_checkpoint() {
        let mut editor = text("a");
        editor.checkpoint();
        editor.insert('b');
        editor.checkpoint();
        editor.insert('c');

        assert!(editor.undo());
        assert_eq!(editor.text(), "ab");
        assert!(editor.undo());
        assert_eq!(editor.text(), "a");
        assert!(!editor.undo());
        assert!(editor.redo());
        assert_eq!(editor.text(), "ab");
        assert_eq!(editor.cursor(), (0, 2));
    }

    #[test]
    fn a_new_change_drops_what_could_be_redone() {
        let mut editor = text("a");
        editor.checkpoint();
        editor.insert('b');
        editor.undo();

        editor.checkpoint();
        editor.insert('c');

        assert!(!editor.redo());
        assert_eq!(editor.text(), "ac");
    }

    #[test]
    fn set_text_replaces_the_lines_and_puts_the_cursor_at_the_end() {
        let mut editor = typed("old");

        editor.set_text("SELECT *\nFROM c");

        assert_eq!(editor.lines(), ["SELECT *", "FROM c"]);
        assert_eq!(editor.cursor(), (1, 6));
    }
}
