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
        let end = if is_word(c) {
            rest.find(|c| !is_word(c)).unwrap_or(rest.len())
        } else {
            rest.find(is_word).unwrap_or(rest.len())
        };
        let (token, after) = rest.split_at(end);
        spans.push(span(token));
        rest = after;
    }
    Line::from(spans)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '_' | '.' | '$')
}

/// A token, coloured by what kind it is.
fn span(token: &str) -> Span<'static> {
    let keyword = KEYWORDS.iter().any(|k| k.eq_ignore_ascii_case(token));
    if keyword {
        Span::styled(token.to_string(), Style::new().fg(Color::Blue))
    } else {
        Span::raw(token.to_string())
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
}
