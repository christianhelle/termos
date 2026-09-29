//! Control plane adapter backed by the Azure Resource Manager REST API.

use serde_json::{Value, json};

use crate::management::Account;

/// The Resource Manager id of a Cosmos DB account.
fn account_id(account: &Account) -> String {
    format!(
        "/subscriptions/{}/resourceGroups/{}/providers/Microsoft.DocumentDB/databaseAccounts/{}",
        account.subscription_id, account.resource_group, account.name
    )
}

/// The request body for creating a SQL container.
fn create_container_body(name: &str, partition_key_path: &str, throughput: Option<u32>) -> Value {
    let options = match throughput {
        Some(throughput) => json!({ "throughput": throughput }),
        None => json!({}),
    };
    json!({
        "properties": {
            "resource": {
                "id": name,
                "partitionKey": { "paths": [partition_key_path], "kind": "Hash" }
            },
            "options": options
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_account_resource_id() {
        let account = Account {
            name: "orders".into(),
            subscription_id: "sub-1".into(),
            resource_group: "rg-data".into(),
            location: "West Europe".into(),
            endpoint: "https://orders.documents.azure.com:443/".into(),
        };

        assert_eq!(
            account_id(&account),
            "/subscriptions/sub-1/resourceGroups/rg-data/providers/Microsoft.DocumentDB/databaseAccounts/orders"
        );
    }

    #[test]
    fn create_container_body_sets_partition_key_and_throughput() {
        assert_eq!(
            create_container_body("carts", "/userId", Some(400)),
            json!({
                "properties": {
                    "resource": {
                        "id": "carts",
                        "partitionKey": { "paths": ["/userId"], "kind": "Hash" }
                    },
                    "options": { "throughput": 400 }
                }
            })
        );
    }

    #[test]
    fn create_container_body_omits_throughput_when_not_given() {
        let body = create_container_body("carts", "/userId", None);

        assert_eq!(body["properties"]["options"], json!({}));
    }
}
