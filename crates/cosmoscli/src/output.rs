use comfy_table::{Table, presets::ASCII_FULL};
use serde_json::Value;

use crate::partition::value_at_path;

/// Renders documents as a table with the id and partition key columns.
pub fn render_table(docs: &[Value], pk_path: &str) -> String {
    if docs.is_empty() {
        return "No documents found.".to_string();
    }
    let mut table = Table::new();
    table.load_style(ASCII_FULL).set_header(["id", pk_path]);
    for doc in docs {
        table.add_row([cell(doc.get("id")), cell(value_at_path(doc, pk_path))]);
    }
    table.to_string()
}

/// Renders arbitrary rows as a table with the given headers.
pub fn render_rows<const N: usize>(headers: [&str; N], rows: Vec<[String; N]>) -> String {
    let mut table = Table::new();
    table.load_style(ASCII_FULL).set_header(headers);
    table.add_rows(rows);
    table.to_string()
}

/// Renders documents as a pretty-printed JSON array.
pub fn render_json(docs: &[Value]) -> String {
    serde_json::to_string_pretty(docs).expect("JSON values always serialize")
}

fn cell(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn table_shows_id_and_partition_key() {
        let docs = vec![
            json!({ "id": "1", "tenantId": "contoso", "name": "a" }),
            json!({ "id": "2", "tenantId": 42, "name": "b" }),
        ];
        let expected = "\
+----+-----------+
| id | /tenantId |
+================+
| 1  | contoso   |
|----+-----------|
| 2  | 42        |
+----+-----------+";
        assert_eq!(render_table(&docs, "/tenantId"), expected);
    }

    #[test]
    fn empty_result_says_no_documents() {
        assert_eq!(render_table(&[], "/tenantId"), "No documents found.");
    }

    #[test]
    fn json_renders_pretty_array() {
        let docs = vec![json!({ "id": "1" }), json!({ "id": "2" })];
        let expected = "[\n  {\n    \"id\": \"1\"\n  },\n  {\n    \"id\": \"2\"\n  }\n]";
        assert_eq!(render_json(&docs), expected);
    }
}
