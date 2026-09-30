//! Data plane adapter backed by the Azure Cosmos DB SDK.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use azure_core::credentials::{Secret, TokenCredential};
use azure_core::http::StatusCode;
use azure_data_cosmos::{
    AccountEndpoint, AccountReference, ContainerClient, CosmosClient, CosmosError, FeedScope,
    PartitionKey, Query, RoutingStrategy,
};
use futures::{StreamExt, TryStreamExt};
use serde_json::Value;

use crate::management::Account;
use crate::store::{Credential, DataPlane, DataStore, Documents, Unauthorized};

/// Connects to containers with the Cosmos DB SDK.
pub struct CosmosDataPlane {
    entra: Arc<dyn TokenCredential>,
    /// Clients by account endpoint and credential, so another container of the
    /// same account skips client setup.
    clients: Mutex<HashMap<(String, Credential), CosmosClient>>,
}

impl CosmosDataPlane {
    pub fn new(entra: Arc<dyn TokenCredential>) -> Self {
        CosmosDataPlane {
            entra,
            clients: Mutex::default(),
        }
    }

    async fn client(
        &self,
        account: &Account,
        credential: Credential,
    ) -> anyhow::Result<CosmosClient> {
        let key = (account.endpoint.clone(), credential);
        if let Some(client) = self
            .clients
            .lock()
            .expect("clients lock poisoned")
            .get(&key)
        {
            return Ok(client.clone());
        }
        let endpoint: AccountEndpoint = account.endpoint.parse()?;
        let reference = match &key.1 {
            Credential::Entra => AccountReference::with_credential(endpoint, self.entra.clone()),
            Credential::Key(key) => {
                AccountReference::with_authentication_key(endpoint, Secret::from(key.clone()))
            }
        };
        let client = CosmosClient::builder()
            .build(reference, RoutingStrategy::PreferredRegions(Vec::new()))
            .await
            .map_err(classify)?;
        self.clients
            .lock()
            .expect("clients lock poisoned")
            .insert(key, client.clone());
        Ok(client)
    }
}

impl DataPlane for CosmosDataPlane {
    type Store = CosmosStore;

    async fn connect(
        &self,
        account: &Account,
        database: &str,
        container: &str,
        credential: Credential,
    ) -> anyhow::Result<CosmosStore> {
        let client = self
            .client(account, credential)
            .await?
            .database_client(database)
            .container_client(container, None)
            .await
            .map_err(classify)?;
        let properties = client.read(None).await.map_err(classify)?.into_model()?;
        let pk_path = properties
            .partition_key
            .paths()
            .first()
            .map(|path| path.to_string())
            .unwrap_or_else(|| "/id".into());
        Ok(CosmosStore { client, pk_path })
    }

    async fn prepare(&self, account: &Account, credential: Credential) {
        let _ = self.client(account, credential).await;
    }
}

/// Stops the Cosmos DB SDK probing the Azure VM metadata service, which it only uses
/// for diagnostics. Off Azure the probe waits 2 seconds to time out on the first connection.
///
/// # Safety
///
/// Sets an environment variable, so it must run before any other thread starts.
pub unsafe fn skip_vm_metadata_probe() {
    if std::env::var_os("COSMOS_DISABLE_IMDS").is_none() {
        // SAFETY: the caller guarantees this is the only thread
        unsafe { std::env::set_var("COSMOS_DISABLE_IMDS", "1") };
    }
}

/// Drops the diagnostics JSON the SDK appends to service error messages.
fn short_message(message: &str) -> &str {
    match message.find(", {\"Summary\"") {
        Some(end) => &message[..end],
        None => message,
    }
}

/// Shortens SDK errors and turns authorization failures into [`Unauthorized`]
/// so callers can fall back.
fn classify(error: CosmosError) -> anyhow::Error {
    let message = short_message(&error.to_string()).to_string();
    match error.status().status_code() {
        StatusCode::Unauthorized | StatusCode::Forbidden => Unauthorized(message).into(),
        _ => anyhow::anyhow!(message),
    }
}

pub struct CosmosStore {
    client: ContainerClient,
    pk_path: String,
}

impl DataStore for CosmosStore {
    fn partition_key_path(&self) -> &str {
        &self.pk_path
    }

    async fn documents(&self, sql: &str) -> anyhow::Result<Documents> {
        let items = self
            .client
            .query_items::<Value>(Query::from(sql), FeedScope::full_container(), None)
            .await
            .map_err(classify)?;
        Ok(items.map_err(classify).boxed_local())
    }

    async fn read_item(&self, id: &str, pk: &Value) -> anyhow::Result<Value> {
        let response = self
            .client
            .read_item(to_partition_key(pk)?, id, None)
            .await
            .map_err(classify)?;
        response.into_model().map_err(classify)
    }

    async fn delete_item(&self, id: &str, pk: &Value) -> anyhow::Result<()> {
        self.client
            .delete_item(to_partition_key(pk)?, id, None)
            .await
            .map_err(classify)?;
        Ok(())
    }

    async fn create_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()> {
        let id = document_id(&doc)?;
        self.client
            .create_item(to_partition_key(pk)?, &id, doc, None)
            .await
            .map_err(classify)?;
        Ok(())
    }

    async fn upsert_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()> {
        let id = document_id(&doc)?;
        self.client
            .upsert_item(to_partition_key(pk)?, &id, doc, None)
            .await
            .map_err(classify)?;
        Ok(())
    }

    async fn replace_item(&self, id: &str, pk: &Value, doc: Value) -> anyhow::Result<()> {
        self.client
            .replace_item(to_partition_key(pk)?, id, doc, None)
            .await
            .map_err(classify)?;
        Ok(())
    }

    async fn ids_in_partition(&self, pk: &Value) -> anyhow::Result<Vec<String>> {
        let ids: Vec<String> = self
            .client
            .query_items::<String>(
                Query::from("SELECT VALUE c.id FROM c"),
                FeedScope::partition(to_partition_key(pk)?),
                None,
            )
            .await
            .map_err(classify)?
            .try_collect()
            .await
            .map_err(classify)?;
        Ok(ids)
    }
}

fn document_id(doc: &Value) -> anyhow::Result<String> {
    doc.get("id")
        .and_then(Value::as_str)
        .map(String::from)
        .ok_or_else(|| anyhow::anyhow!("document has no string 'id' property"))
}

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
    fn short_message_drops_diagnostics() {
        let message = r#"409: Cosmos DB returned HTTP 409: Unknown. Details: Entity with the specified id already exists in the system., {"Summary":{"DirectCalls":{"(409, 0)":1}},"name":"HandleDocumentRequest"}"#;

        assert_eq!(
            short_message(message),
            "409: Cosmos DB returned HTTP 409: Unknown. Details: Entity with the specified id already exists in the system."
        );
    }

    #[test]
    fn short_message_keeps_messages_without_diagnostics() {
        let message =
            "403/5301 (RbacUnauthorizedMetadataRequest): AccountProperties fetch returned HTTP 403";

        assert_eq!(short_message(message), message);
    }

    #[test]
    fn rejects_objects_as_partition_keys() {
        assert!(to_partition_key(&json!({ "a": 1 })).is_err());
    }
}
