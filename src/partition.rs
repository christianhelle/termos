use serde_json::Value;

/// Returns the value found at a partition key path such as `/tenantId`.
pub fn value_at_path<'a>(doc: &'a Value, path: &str) -> Option<&'a Value> {
    path.trim_start_matches('/')
        .split('/')
        .try_fold(doc, |value, segment| value.get(segment))
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
}
