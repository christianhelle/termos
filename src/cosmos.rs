//! Data plane adapter backed by the Azure Cosmos DB SDK.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use azure_core::credentials::{Secret, TokenCredential};
use azure_core::http::StatusCode;
use azure_data_cosmos::{
    AccountEndpoint, AccountReference, ContainerClient, CosmosClient, CosmosError, FeedScope,
    PartitionKey, Query, RoutingStrategy,
};
use futures::StreamExt;
use serde_json::Value;

use crate::management::Account;
use crate::store::{Credential, DataPlane, DataStore, Documents, Page, Unauthorized};

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
pub(crate) fn classify(error: CosmosError) -> anyhow::Error {
    let message = short_message(&error.to_string()).to_string();
    match error.status().status_code() {
        StatusCode::Unauthorized | StatusCode::Forbidden => Unauthorized(message).into(),
        _ => anyhow::anyhow!(message),
    }
}

/// The partition key of a document, from the value at the container's partition key path.
fn partition_key_of(value: Option<&Value>) -> anyhow::Result<PartitionKey> {
    Ok(match value {
        None => PartitionKey::UNDEFINED.into(),
        Some(Value::Null) => PartitionKey::NULL.into(),
        Some(Value::String(s)) => PartitionKey::from(s.clone()),
        Some(Value::Bool(b)) => PartitionKey::from(*b),
        Some(Value::Number(n)) => match (n.as_i64(), n.as_f64()) {
            (Some(i), _) => PartitionKey::from(i),
            (None, Some(f)) => PartitionKey::from(f),
            _ => anyhow::bail!("unsupported partition key {n}"),
        },
        Some(other) => anyhow::bail!("unsupported partition key {other}"),
    })
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
        let pages = items.into_pages().map(|page| {
            let page = page.map_err(classify)?;
            let request_charge = page.headers().request_charge().map_or(0.0, |c| c.value());
            Ok(Page {
                docs: page.into_items(),
                request_charge,
            })
        });
        Ok(pages.boxed_local())
    }

    async fn delete(&self, id: &str, partition_key: Option<&Value>) -> anyhow::Result<()> {
        let key = partition_key_of(partition_key)?;
        self.client
            .delete_item(key, id, None)
            .await
            .map_err(classify)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
