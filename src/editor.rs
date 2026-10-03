//! Several lines of editable text, for writing queries.

/// Lines of text with a cursor, counted in lines and characters.
#[derive(Debug)]
pub struct Editor {
    lines: Vec<String>,
    row: usize,
    column: usize,
}

impl Default for Editor {
    fn default() -> Self {
        Editor {
            lines: vec![String::new()],
            row: 0,
            column: 0,
        }
    }
}

impl Editor {
    /// The text, with its lines joined by newlines.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Replaces the text, putting the cursor at its end.
    pub fn set_text(&mut self, text: &str) {
        self.lines = text.split('\n').map(String::from).collect();
        self.row = self.lines.len() - 1;
        self.column = self.len();
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
        self.row = row.min(self.lines.len() - 1);
        self.column = self.column.min(self.len());
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
    fn set_text_replaces_the_lines_and_puts_the_cursor_at_the_end() {
        let mut editor = typed("old");

        editor.set_text("SELECT *\nFROM c");

        assert_eq!(editor.lines(), ["SELECT *", "FROM c"]);
        assert_eq!(editor.cursor(), (1, 6));
    }
}
