use futures::stream::LocalBoxStream;
use serde_json::Value;

use crate::management::Account;

/// Which credentials to use for the Cosmos DB data plane.
#[derive(Debug, Clone, Copy, PartialEq, Default, clap::ValueEnum)]
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

/// The pages of documents a query finds, read as they are needed.
pub type Documents = LocalBoxStream<'static, anyhow::Result<Page>>;

/// The documents one round trip of a query read, and what it cost.
#[derive(Debug, Default)]
pub struct Page {
    pub docs: Vec<Value>,
    /// The request units the round trip used.
    pub request_charge: f64,
    /// How the service ran the query for this page, as `name=value` pairs
    /// separated by semicolons, when it reports it.
    pub query_metrics: Option<String>,
}

/// Document operations on a single container.
#[allow(async_fn_in_trait)]
pub trait DataStore {
    /// The container's partition key path, for example `/tenantId`.
    fn partition_key_path(&self) -> &str;

    /// Runs a query, handing out its documents as they are read, page by page.
    async fn documents(&self, sql: &str) -> anyhow::Result<Documents>;

    /// The properties the service keeps for the container, such as its indexing policy.
    async fn properties(&self) -> anyhow::Result<Value>;

    /// Replaces the properties of the container, returning them as the service saved them.
    async fn replace_properties(&self, properties: Value) -> anyhow::Result<Value>;

    /// Deletes the document with this id, in the partition with this key value.
    /// A missing key value stands for a document without the partition key property.
    async fn delete(&self, id: &str, partition_key: Option<&Value>) -> anyhow::Result<()>;

    /// Replaces the document with this id, in the partition with this key value,
    /// returning it as the service saved it. With an etag, the service only replaces
    /// the document while it is still the version with that etag.
    async fn replace(
        &self,
        id: &str,
        partition_key: Option<&Value>,
        document: Value,
        etag: Option<&str>,
    ) -> anyhow::Result<Value>;
}
