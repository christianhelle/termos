//! Pretty-printed JSON with syntax colours.

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

/// Pretty-prints a document like `serde_json::to_string_pretty`, one styled line per line.
pub fn highlight_json(value: &Value) -> Vec<Line<'static>> {
    let mut writer = Writer::default();
    writer.value(value, 0);
    writer.finish()
}

/// Pretty-prints documents as one JSON array, like [`highlight_json`] would print them in one.
pub fn highlight_json_array(items: &[Value]) -> Vec<Line<'static>> {
    let mut writer = Writer::default();
    writer.array(items, 0);
    writer.finish()
}

/// One line of JSON as typed, which may not parse, coloured like [`highlight_json`]
/// colours documents.
pub fn highlight_json_line(line: &str) -> Line<'static> {
    let mut spans = Vec::new();
    let mut rest = line;
    while let Some(c) = rest.chars().next() {
        let (end, colour) = if c == '"' {
            let end = string_end(rest);
            let key = rest[end..].trim_start().starts_with(':');
            (end, Some(if key { Color::Cyan } else { Color::Green }))
        } else if c == '-' || c.is_ascii_digit() {
            let end = rest[1..]
                .find(|c: char| !(c.is_ascii_digit() || ".eE+-".contains(c)))
                .map_or(rest.len(), |end| end + 1);
            (end, Some(Color::Yellow))
        } else if c.is_alphabetic() {
            let end = rest
                .find(|c: char| !c.is_alphanumeric())
                .unwrap_or(rest.len());
            let literal = matches!(&rest[..end], "true" | "false" | "null");
            (end, literal.then_some(Color::Magenta))
        } else {
            let end = rest
                .find(|c: char| c == '"' || c == '-' || c.is_alphanumeric())
                .unwrap_or(rest.len());
            (end.max(c.len_utf8()), None)
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

/// Where the string at the start of the text ends, past its closing quote,
/// or the end of the text for a string left open.
fn string_end(text: &str) -> usize {
    let mut escaped = false;
    for (index, c) in text.char_indices().skip(1) {
        match c {
            _ if escaped => escaped = false,
            '\\' => escaped = true,
            '"' => return index + 1,
            _ => {}
        }
    }
    text.len()
}

/// Collects spans into lines as the document is walked.
#[derive(Default)]
struct Writer {
    lines: Vec<Line<'static>>,
    current: Vec<Span<'static>>,
}

impl Writer {
    fn push(&mut self, text: impl Into<String>) {
        self.current.push(Span::raw(text.into()));
    }

    fn styled(&mut self, text: String, colour: Color) {
        self.current
            .push(Span::styled(text, Style::new().fg(colour)));
    }

    fn newline(&mut self, indent: usize) {
        let spans = std::mem::take(&mut self.current);
        self.lines.push(Line::from(spans));
        self.push("  ".repeat(indent));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.lines.push(Line::from(self.current));
        self.lines
    }

    fn array(&mut self, items: &[Value], indent: usize) {
        if items.is_empty() {
            self.push("[]");
            return;
        }
        self.push("[");
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                self.push(",");
            }
            self.newline(indent + 1);
            self.value(item, indent + 1);
        }
        self.newline(indent);
        self.push("]");
    }

    fn value(&mut self, value: &Value, indent: usize) {
        match value {
            Value::Object(map) if !map.is_empty() => {
                self.push("{");
                for (index, (key, value)) in map.iter().enumerate() {
                    if index > 0 {
                        self.push(",");
                    }
                    self.newline(indent + 1);
                    self.styled(Value::from(key.as_str()).to_string(), Color::Cyan);
                    self.push(": ");
                    self.value(value, indent + 1);
                }
                self.newline(indent);
                self.push("}");
            }
            Value::Array(items) => self.array(items, indent),
            Value::String(_) => self.styled(value.to_string(), Color::Green),
            Value::Number(_) => self.styled(value.to_string(), Color::Yellow),
            Value::Bool(_) | Value::Null => self.styled(value.to_string(), Color::Magenta),
            Value::Object(_) => self.push("{}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use serde_json::json;

    fn text(lines: &[Line]) -> String {
        let lines: Vec<String> = lines.iter().map(ToString::to_string).collect();
        lines.join("\n")
    }

    #[test]
    fn lays_out_documents_like_pretty_printed_json() {
        let doc = json!({
            "id": "o-1",
            "tenant \"quoted\"": "contoso",
            "total": 12.5,
            "paid": true,
            "note": null,
            "lines": [{ "sku": "a", "qty": 2 }, []],
            "meta": {}
        });

        assert_eq!(
            text(&highlight_json(&doc)),
            serde_json::to_string_pretty(&doc).unwrap()
        );
    }

    #[test]
    fn lays_out_documents_as_one_array() {
        let docs = [json!({ "id": "o-1" }), json!(3)];

        assert_eq!(
            text(&highlight_json_array(&docs)),
            serde_json::to_string_pretty(&json!(docs)).unwrap()
        );
        assert_eq!(text(&highlight_json_array(&[])), "[]");
    }

    fn colour_of(lines: &[Line], content: &str) -> Option<Color> {
        lines
            .iter()
            .flat_map(|line| &line.spans)
            .find(|span| span.content == content)
            .and_then(|span| span.style.fg)
    }

    #[test]
    fn colours_keys_strings_numbers_and_literals() {
        let lines =
            highlight_json(&json!({ "id": "o-1", "total": 3, "paid": false, "note": null }));

        assert_eq!(colour_of(&lines, "\"id\""), Some(Color::Cyan));
        assert_eq!(colour_of(&lines, "\"o-1\""), Some(Color::Green));
        assert_eq!(colour_of(&lines, "3"), Some(Color::Yellow));
        assert_eq!(colour_of(&lines, "false"), Some(Color::Magenta));
        assert_eq!(colour_of(&lines, "null"), Some(Color::Magenta));
        assert_eq!(colour_of(&lines, "{"), None);
    }

    #[test]
    fn colours_one_line_of_json_being_edited() {
        let line = highlight_json_line(r#"  "path": "/\"_etag\"/?", "n": -1.5e3, "on": true, x"#);

        assert_eq!(
            line.to_string(),
            r#"  "path": "/\"_etag\"/?", "n": -1.5e3, "on": true, x"#
        );
        let lines = [line];
        assert_eq!(colour_of(&lines, "\"path\""), Some(Color::Cyan));
        assert_eq!(colour_of(&lines, r#""/\"_etag\"/?""#), Some(Color::Green));
        assert_eq!(colour_of(&lines, "-1.5e3"), Some(Color::Yellow));
        assert_eq!(colour_of(&lines, "true"), Some(Color::Magenta));
        assert_eq!(colour_of(&lines, "x"), None);
    }

    #[test]
    fn colours_an_unfinished_string_to_the_end_of_the_line() {
        let lines = [highlight_json_line(r#"{"open"#)];

        assert_eq!(colour_of(&lines, "\"open"), Some(Color::Green));
    }
}
