//! Data plane adapter backed by the Azure Cosmos DB SDK.

use azure_data_cosmos::PartitionKey;
use serde_json::Value;

/// Converts a JSON partition key value into an SDK partition key.
fn to_partition_key(value: &Value) -> anyhow::Result<PartitionKey> {
    Ok(match value {
        Value::String(s) => PartitionKey::from(s.clone()),
        Value::Number(n) => {
            let n = n
                .as_f64()
                .ok_or_else(|| anyhow::anyhow!("partition key {n} is not a finite number"))?;
            PartitionKey::from(n)
        }
        Value::Bool(b) => PartitionKey::from(*b),
        Value::Null => PartitionKey::from(PartitionKey::NULL),
        other => {
            anyhow::bail!("partition key must be a string, number, boolean or null, got {other}")
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_scalar_partition_key_values() {
        assert_eq!(
            to_partition_key(&json!("contoso")).unwrap(),
            PartitionKey::from("contoso")
        );
        assert_eq!(
            to_partition_key(&json!(42)).unwrap(),
            PartitionKey::from(42.0)
        );
        assert_eq!(
            to_partition_key(&json!(true)).unwrap(),
            PartitionKey::from(true)
        );
        assert_eq!(
            to_partition_key(&json!(null)).unwrap(),
            PartitionKey::from(PartitionKey::NULL)
        );
    }

    #[test]
    fn rejects_objects_as_partition_keys() {
        assert!(to_partition_key(&json!({ "a": 1 })).is_err());
    }
}
