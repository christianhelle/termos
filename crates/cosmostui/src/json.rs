//! Pretty-printed JSON with syntax colours.

use ratatui::text::{Line, Span};
use serde_json::Value;

/// Pretty-prints a document like `serde_json::to_string_pretty`, one styled line per line.
pub fn highlight_json(value: &Value) -> Vec<Line<'static>> {
    let mut writer = Writer::default();
    writer.value(value, 0);
    writer.finish()
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

    fn newline(&mut self, indent: usize) {
        let spans = std::mem::take(&mut self.current);
        self.lines.push(Line::from(spans));
        self.push("  ".repeat(indent));
    }

    fn finish(mut self) -> Vec<Line<'static>> {
        self.lines.push(Line::from(self.current));
        self.lines
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
                    self.push(Value::from(key.as_str()).to_string());
                    self.push(": ");
                    self.value(value, indent + 1);
                }
                self.newline(indent);
                self.push("}");
            }
            Value::Array(items) if !items.is_empty() => {
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
            other => self.push(other.to_string()),
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
}
