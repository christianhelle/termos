//! Control plane adapter for the local Cosmos DB emulator, which has no Resource Manager.

use azure_core::credentials::Secret;
use azure_core::http::Url;
use azure_data_cosmos::{AccountReference, CosmosClient, RoutingStrategy};
use futures::TryStreamExt;
use tokio::sync::OnceCell;

use crate::cosmos::classify;
use crate::management::{Account, Container, Management};

/// Where the emulator listens unless told otherwise.
pub const DEFAULT_ENDPOINT: &str = "https://localhost:8081/";

/// The key every emulator accepts, published in the emulator documentation.
pub const DEFAULT_KEY: &str =
    "C2y6yDjf5/R+ob0N8A7Cgv30VRDJIWEHLM+4QDU9DAoPhTqgnGdmhIptVNsFvYgoyLCQy68hbWfaiYG3chcdZQ==";

/// Lists the emulator's databases and containers through its data plane.
pub struct Emulator {
    account: Account,
    key: String,
    client: OnceCell<CosmosClient>,
}

impl Emulator {
    pub fn new(endpoint: &str, key: String) -> anyhow::Result<Self> {
        Ok(Emulator {
            account: emulator_account(endpoint)?,
            key,
            client: OnceCell::new(),
        })
    }

    async fn client(&self) -> anyhow::Result<&CosmosClient> {
        self.client
            .get_or_try_init(|| async {
                let endpoint = self.account.endpoint.parse()?;
                let reference = AccountReference::with_authentication_key(
                    endpoint,
                    Secret::from(self.key.clone()),
                );
                CosmosClient::builder()
                    .build(reference, RoutingStrategy::PreferredRegions(Vec::new()))
                    .await
                    .map_err(classify)
            })
            .await
    }
}

impl Management for Emulator {
    async fn list_accounts(&self, _subscription: Option<&str>) -> anyhow::Result<Vec<Account>> {
        Ok(vec![self.account.clone()])
    }

    async fn list_databases(&self, _account: &Account) -> anyhow::Result<Vec<String>> {
        let databases = self
            .client()
            .await?
            .query_databases("SELECT * FROM root", None)
            .await
            .map_err(classify)?;
        let databases: Vec<_> = databases.map_err(classify).try_collect().await?;
        Ok(databases.into_iter().filter_map(|db| db.id).collect())
    }

    async fn list_containers(
        &self,
        _account: &Account,
        database: &str,
    ) -> anyhow::Result<Vec<Container>> {
        let containers = self
            .client()
            .await?
            .database_client(database)
            .query_containers("SELECT * FROM root", None)
            .await
            .map_err(classify)?;
        let containers: Vec<_> = containers.map_err(classify).try_collect().await?;
        Ok(containers
            .into_iter()
            .map(|container| Container {
                name: container.id.to_string(),
                partition_key_paths: container
                    .partition_key
                    .paths()
                    .iter()
                    .map(|path| path.to_string())
                    .collect(),
            })
            .collect())
    }

    async fn primary_key(&self, _account: &Account) -> anyhow::Result<String> {
        Ok(self.key.clone())
    }
}

/// The emulator as an account, named after the host and port it listens on.
fn emulator_account(endpoint: &str) -> anyhow::Result<Account> {
    let url = Url::parse(endpoint)?;
    let host = url
        .host_str()
        .ok_or_else(|| anyhow::anyhow!("the emulator endpoint {endpoint} has no host"))?;
    let name = match url.port_or_known_default() {
        Some(port) => format!("{host}:{port}"),
        None => host.to_string(),
    };
    Ok(Account {
        name,
        subscription_id: String::new(),
        resource_group: String::new(),
        location: "emulator".into(),
        endpoint: url.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_the_account_after_host_and_port() {
        let account = emulator_account(DEFAULT_ENDPOINT).unwrap();

        assert_eq!(account.name, "localhost:8081");
        assert_eq!(account.endpoint, "https://localhost:8081/");
    }

    #[test]
    fn uses_the_scheme_port_when_none_is_given() {
        let account = emulator_account("http://cosmos").unwrap();

        assert_eq!(account.name, "cosmos:80");
        assert_eq!(account.endpoint, "http://cosmos/");
    }

    #[test]
    fn rejects_an_endpoint_that_is_not_a_url() {
        assert!(emulator_account("localhost:8081").is_err());
    }
}
