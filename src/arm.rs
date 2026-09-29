//! Control plane adapter backed by the Azure Resource Manager REST API.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use azure_core::credentials::TokenCredential;
use reqwest::{Method, Response, StatusCode};
use serde_json::{Value, json};

use crate::management::{
    Account, Container, Management, Page, parse_accounts, parse_container, parse_containers,
    parse_databases, parse_primary_key, parse_subscriptions,
};

const ENDPOINT: &str = "https://management.azure.com";
const SCOPE: &str = "https://management.azure.com/.default";
const COSMOS_API_VERSION: &str = "2024-11-15";
const SUBSCRIPTIONS_API_VERSION: &str = "2022-12-01";

/// Talks to Azure Resource Manager with an Entra ID token.
pub struct Arm {
    http: reqwest::Client,
    credential: Arc<dyn TokenCredential>,
}

impl Arm {
    pub fn new(credential: Arc<dyn TokenCredential>) -> Self {
        Arm {
            http: reqwest::Client::new(),
            credential,
        }
    }

    async fn send(
        &self,
        method: Method,
        url: &str,
        body: Option<&Value>,
    ) -> anyhow::Result<Response> {
        let token = self
            .credential
            .get_token(&[SCOPE], None)
            .await
            .context("getting an Azure Resource Manager token, try `az login`")?;
        let mut request = self
            .http
            .request(method.clone(), url)
            .bearer_auth(token.token.secret());
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request.send().await?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_default();
            anyhow::bail!("{method} {url} failed with {status}: {text}");
        }
        Ok(response)
    }

    async fn get(&self, url: &str) -> anyhow::Result<String> {
        Ok(self.send(Method::GET, url, None).await?.text().await?)
    }

    /// Follows `nextLink` until every page has been read.
    async fn list_all<T>(
        &self,
        url: String,
        parse: fn(&str) -> anyhow::Result<Page<T>>,
    ) -> anyhow::Result<Vec<T>> {
        let mut items = Vec::new();
        let mut next = Some(url);
        while let Some(url) = next {
            let page = parse(&self.get(&url).await?)?;
            items.extend(page.items);
            next = page.next_link;
        }
        Ok(items)
    }

    /// Waits for a long running operation started by `response` to finish.
    async fn wait_for(&self, response: Response) -> anyhow::Result<()> {
        if response.status() != StatusCode::ACCEPTED {
            return Ok(());
        }
        let Some(status_url) = response
            .headers()
            .get("azure-asyncoperation")
            .and_then(|value| value.to_str().ok())
            .map(String::from)
        else {
            return Ok(());
        };
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let status: Value = serde_json::from_str(&self.get(&status_url).await?)?;
            match status["status"].as_str() {
                Some("Succeeded") => return Ok(()),
                Some("Failed" | "Canceled") => anyhow::bail!("operation did not succeed: {status}"),
                _ => continue,
            }
        }
    }

    fn cosmos_url(&self, account: &Account, path: &str) -> String {
        format!(
            "{ENDPOINT}{}{path}?api-version={COSMOS_API_VERSION}",
            account_id(account)
        )
    }

    fn container_url(&self, account: &Account, database: &str, container: &str) -> String {
        self.cosmos_url(
            account,
            &format!("/sqlDatabases/{database}/containers/{container}"),
        )
    }
}

impl Management for Arm {
    async fn list_accounts(&self, subscription: Option<&str>) -> anyhow::Result<Vec<Account>> {
        let subscriptions = match subscription {
            Some(id) => vec![id.to_string()],
            None => {
                let url =
                    format!("{ENDPOINT}/subscriptions?api-version={SUBSCRIPTIONS_API_VERSION}");
                self.list_all(url, parse_subscriptions).await?
            }
        };
        let mut accounts = Vec::new();
        for subscription in subscriptions {
            let url = format!(
                "{ENDPOINT}/subscriptions/{subscription}/providers/Microsoft.DocumentDB/databaseAccounts?api-version={COSMOS_API_VERSION}"
            );
            accounts.extend(self.list_all(url, parse_accounts).await?);
        }
        Ok(accounts)
    }

    async fn list_databases(&self, account: &Account) -> anyhow::Result<Vec<String>> {
        let url = self.cosmos_url(account, "/sqlDatabases");
        self.list_all(url, parse_databases).await
    }

    async fn list_containers(
        &self,
        account: &Account,
        database: &str,
    ) -> anyhow::Result<Vec<Container>> {
        let url = self.cosmos_url(account, &format!("/sqlDatabases/{database}/containers"));
        self.list_all(url, parse_containers).await
    }

    async fn get_container(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<Container> {
        let url = self.container_url(account, database, container);
        parse_container(&self.get(&url).await?)
    }

    async fn create_container(
        &self,
        account: &Account,
        database: &str,
        container: &str,
        partition_key_path: &str,
        throughput: Option<u32>,
    ) -> anyhow::Result<()> {
        let url = self.container_url(account, database, container);
        let body = create_container_body(container, partition_key_path, throughput);
        let response = self.send(Method::PUT, &url, Some(&body)).await?;
        self.wait_for(response).await
    }

    async fn delete_container(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<()> {
        let url = self.container_url(account, database, container);
        let response = self.send(Method::DELETE, &url, None).await?;
        self.wait_for(response).await
    }

    async fn primary_key(&self, account: &Account) -> anyhow::Result<String> {
        let url = self.cosmos_url(account, "/listKeys");
        let response = self.send(Method::POST, &url, None).await?;
        parse_primary_key(&response.text().await?)
    }
}

/// How long to wait before retrying a throttled request.
fn retry_delay(retry_after: Option<&str>) -> Duration {
    let seconds = retry_after
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(5);
    Duration::from_secs(seconds)
}

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
    fn retry_delay_follows_retry_after_seconds() {
        assert_eq!(retry_delay(Some("30")), Duration::from_secs(30));
    }

    #[test]
    fn retry_delay_defaults_to_five_seconds() {
        assert_eq!(retry_delay(None), Duration::from_secs(5));
        assert_eq!(retry_delay(Some("soon")), Duration::from_secs(5));
    }

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
