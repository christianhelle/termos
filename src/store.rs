use serde_json::Value;

use crate::management::Account;

/// How to authenticate to the Cosmos DB data plane.
#[derive(Debug, Clone, PartialEq)]
pub enum Credential {
    Entra,
    Key(String),
}

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
}

/// Document operations on a single container.
#[allow(async_fn_in_trait)]
pub trait DataStore {
    /// The container's partition key path, for example `/tenantId`.
    fn partition_key_path(&self) -> &str;

    async fn query(&self, sql: &str, max: Option<usize>) -> anyhow::Result<Vec<Value>>;

    async fn read_item(&self, id: &str, pk: &Value) -> anyhow::Result<Value>;

    async fn delete_item(&self, id: &str, pk: &Value) -> anyhow::Result<()>;

    /// Creates a document, failing if one with the same id already exists.
    async fn create_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()>;
}
