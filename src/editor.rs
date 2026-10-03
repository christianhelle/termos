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

    pub fn left(&mut self) {
        self.column = self.column.saturating_sub(1);
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
}
