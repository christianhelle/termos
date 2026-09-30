//! Control plane adapter backed by the Azure Resource Manager REST API.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use azure_core::credentials::TokenCredential;
use reqwest::{Method, Response, StatusCode};

use crate::credential::MANAGEMENT_SCOPE;
use crate::management::{
    Account, Container, Management, Page, parse_accounts, parse_containers, parse_databases,
    parse_primary_key, parse_subscriptions,
};

const ENDPOINT: &str = "https://management.azure.com";
const COSMOS_API_VERSION: &str = "2024-11-15";
const SUBSCRIPTIONS_API_VERSION: &str = "2022-12-01";
const MAX_ATTEMPTS: u32 = 5;

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

    async fn send(&self, method: Method, url: &str) -> anyhow::Result<Response> {
        let token = self
            .credential
            .get_token(&[MANAGEMENT_SCOPE], None)
            .await
            .context("getting an Azure Resource Manager token, try `az login`")?;
        let mut attempt = 1;
        loop {
            let response = self
                .http
                .request(method.clone(), url)
                .bearer_auth(token.token.secret())
                .send()
                .await?;
            let status = response.status();
            let throttled = matches!(
                status,
                StatusCode::TOO_MANY_REQUESTS | StatusCode::SERVICE_UNAVAILABLE
            );
            if throttled && attempt < MAX_ATTEMPTS {
                let retry_after = response
                    .headers()
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok());
                let delay = retry_delay(retry_after);
                eprintln!(
                    "Azure Resource Manager is throttling, retrying in {}s",
                    delay.as_secs()
                );
                tokio::time::sleep(delay).await;
                attempt += 1;
                continue;
            }
            if !status.is_success() {
                let text = response.text().await.unwrap_or_default();
                anyhow::bail!("{method} {url} failed with {status}: {text}");
            }
            return Ok(response);
        }
    }

    async fn get(&self, url: &str) -> anyhow::Result<String> {
        Ok(self.send(Method::GET, url).await?.text().await?)
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

    fn cosmos_url(&self, account: &Account, path: &str) -> String {
        format!(
            "{ENDPOINT}{}{path}?api-version={COSMOS_API_VERSION}",
            account_id(account)
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
        let listings = subscriptions.iter().map(|subscription| {
            let url = format!(
                "{ENDPOINT}/subscriptions/{subscription}/providers/Microsoft.DocumentDB/databaseAccounts?api-version={COSMOS_API_VERSION}"
            );
            self.list_all(url, parse_accounts)
        });
        // Every subscription is listed at once, and results keep the subscription order
        let accounts = futures::future::try_join_all(listings).await?;
        Ok(accounts.into_iter().flatten().collect())
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

    async fn primary_key(&self, account: &Account) -> anyhow::Result<String> {
        let url = self.cosmos_url(account, "/listKeys");
        let response = self.send(Method::POST, &url).await?;
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
}
