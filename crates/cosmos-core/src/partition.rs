use serde_json::Value;

/// Returns the value found at a partition key path such as `/tenantId`.
pub fn value_at_path<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('/')
        .split('/')
        .try_fold(doc, |value, segment| value.get(segment))
}

/// Shows a value in a table cell: strings without quotes, other values as JSON, missing as blank.
pub fn display_value(value: Option<&Value>) -> String {
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
    fn reads_top_level_partition_key() {
        let doc = json!({ "id": "1", "tenantId": "contoso" });
        assert_eq!(value_at_path(&doc, "/tenantId"), Some(&json!("contoso")));
    }

    #[test]
    fn reads_nested_partition_key() {
        let doc = json!({ "id": "1", "address": { "city": "Copenhagen" } });
        assert_eq!(
            value_at_path(&doc, "/address/city"),
            Some(&json!("Copenhagen"))
        );
    }

    #[test]
    fn missing_partition_key_is_none() {
        let doc = json!({ "id": "1", "address": {} });
        assert_eq!(value_at_path(&doc, "/address/city"), None);
    }
}
