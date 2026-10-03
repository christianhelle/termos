//! Cosmos DB SQL with syntax colours.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

/// Words with a meaning of their own in Cosmos DB SQL.
const KEYWORDS: [&str; 26] = [
    "SELECT", "VALUE", "TOP", "DISTINCT", "FROM", "WHERE", "JOIN", "IN", "AND", "OR", "NOT",
    "ORDER", "BY", "ASC", "DESC", "GROUP", "OFFSET", "LIMIT", "AS", "BETWEEN", "LIKE", "EXISTS",
    "ARRAY", "IS", "ESCAPE", "UDF",
];

/// A line of SQL, with keywords, literals and comments coloured.
pub fn highlight_sql(line: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let (end, colour) = if rest.starts_with("--") {
            (rest.len(), Some(Color::DarkGray))
        } else if c == '\'' || c == '"' {
            (string_end(rest, c), Some(Color::Green))
        } else if c == '@' {
            (1 + word_end(&rest[1..]), Some(Color::Cyan))
        } else if is_word(c) {
            let end = word_end(rest);
            (end, word_colour(&rest[..end]))
        } else {
            (
                rest.find(|c| is_word(c) || "'\"@-".contains(c))
                    .unwrap_or(rest.len())
                    .max(1),
                None,
            )
        };
        let (token, after) = rest.split_at(end);
        spans.push(match colour {
            Some(colour) => Span::styled(token.to_string(), Style::new().fg(colour)),
            None => Span::raw(token.to_string()),
        });
        rest = after;
    }
    Line::from(spans)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$')
}

/// Where the word at the start of the text ends.
fn word_end(text: &str) -> usize {
    text.find(|c| !is_word(c)).unwrap_or(text.len())
}

/// Where the string opened by the quote at the start of the text ends,
/// after its closing quote, or at the end of the text when it is not closed.
fn string_end(text: &str, quote: char) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            _ if c == quote => return index + c.len_utf8(),
            _ => {}
        }
    }
    text.len()
}

/// The colour of a keyword, literal or number, or none for a name.
fn word_colour(word: &str) -> Option<Color> {
    let is = |names: &[&str]| names.iter().any(|name| name.eq_ignore_ascii_case(word));
    if is(&KEYWORDS) {
        Some(Color::Blue)
    } else if is(&["true", "false", "null", "undefined"]) {
        Some(Color::Magenta)
    } else if word.starts_with(|c: char| c.is_ascii_digit()) {
        Some(Color::Yellow)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use ratatui::style::Color;

    fn colour_of(line: &Line, content: &str) -> Option<Color> {
        line.spans
            .iter()
            .find(|span| span.content == content)
            .and_then(|span| span.style.fg)
    }

    #[test]
    fn colours_keywords_in_any_case_and_keeps_the_text() {
        let line = highlight_sql("select c.id FROM c WHERE c.total > 1");

        assert_eq!(line.to_string(), "select c.id FROM c WHERE c.total > 1");
        assert_eq!(colour_of(&line, "select"), Some(Color::Blue));
        assert_eq!(colour_of(&line, "FROM"), Some(Color::Blue));
        assert_eq!(colour_of(&line, "WHERE"), Some(Color::Blue));
        assert_eq!(colour_of(&line, "c.id"), None);
    }

    #[test]
    fn colours_strings_numbers_literals_params_and_comments() {
        let line = highlight_sql(
            "WHERE c.name = 'O\\'Brien w' AND c.n > 90.5 AND c.ok = true AND c.x = @x -- note 'a'",
        );

        assert_eq!(colour_of(&line, "'O\\'Brien w'"), Some(Color::Green));
        assert_eq!(colour_of(&line, "90.5"), Some(Color::Yellow));
        assert_eq!(colour_of(&line, "true"), Some(Color::Magenta));
        assert_eq!(colour_of(&line, "@x"), Some(Color::Cyan));
        assert_eq!(colour_of(&line, "-- note 'a'"), Some(Color::DarkGray));
    }

    #[test]
    fn colours_double_quoted_strings_and_unclosed_strings_to_the_end() {
        let line = highlight_sql("c[\"name\"] = 'open");

        assert_eq!(colour_of(&line, "\"name\""), Some(Color::Green));
        assert_eq!(colour_of(&line, "'open"), Some(Color::Green));
    }

    #[test]
    fn keeps_operators_and_other_characters_as_typed() {
        let line = highlight_sql("c.n - 1 → ½ != -2");

        assert_eq!(line.to_string(), "c.n - 1 → ½ != -2");
    }
}
