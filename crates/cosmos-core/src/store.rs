use futures::stream::LocalBoxStream;
use futures::{StreamExt, TryStreamExt};
use serde_json::Value;

use crate::management::Account;

/// Which credentials to use for the Cosmos DB data plane.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum AuthMode {
    /// Try Entra ID first and fall back to the account key
    #[default]
    Auto,
    /// Only use Entra ID
    Entra,
    /// Only use the account key
    Key,
}

/// How to authenticate to the Cosmos DB data plane.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Credential {
    Entra,
    Key(String),
}

/// Returned by a data plane when the credential is not allowed to access the container.
#[derive(Debug, thiserror::Error)]
#[error("not authorized: {0}")]
pub struct Unauthorized(pub String);

/// Opens connections to containers on the Cosmos DB data plane.
#[allow(async_fn_in_trait)]
pub trait DataPlane {
    type Store: DataStore;

    async fn connect(
        &self,
        account: &Account,
        database: &str,
        container: &str,
        credential: Credential,
    ) -> anyhow::Result<Self::Store>;

    /// Gets ready to connect to the account's containers, such as by setting up a client.
    /// Failures are left for [`DataPlane::connect`] to report.
    async fn prepare(&self, _account: &Account, _credential: Credential) {}
}

/// The documents a query finds, read as they are needed.
pub type Documents = LocalBoxStream<'static, anyhow::Result<Value>>;

/// Document operations on a single container.
#[allow(async_fn_in_trait)]
pub trait DataStore {
    /// The container's partition key path, for example `/tenantId`.
    fn partition_key_path(&self) -> &str;

    /// Runs a query, handing out its documents as they are read, page by page.
    async fn documents(&self, sql: &str) -> anyhow::Result<Documents>;

    /// Runs a query and collects up to `max` documents, or every document without a max.
    async fn query(&self, sql: &str, max: Option<usize>) -> anyhow::Result<Vec<Value>> {
        let docs = self.documents(sql).await?;
        docs.take(max.unwrap_or(usize::MAX)).try_collect().await
    }

    async fn read_item(&self, id: &str, pk: &Value) -> anyhow::Result<Value>;

    async fn delete_item(&self, id: &str, pk: &Value) -> anyhow::Result<()>;

    /// Creates a document, failing if one with the same id already exists.
    async fn create_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()>;

    /// Creates a document, or replaces it when the id already exists.
    async fn upsert_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()>;

    /// Replaces an existing document, failing if it does not exist.
    async fn replace_item(&self, id: &str, pk: &Value, doc: Value) -> anyhow::Result<()>;

    /// Lists the ids of every document with the given partition key.
    async fn ids_in_partition(&self, pk: &Value) -> anyhow::Result<Vec<String>>;
}
