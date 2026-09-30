use serde::{Deserialize, Serialize, de::DeserializeOwned};

/// A Cosmos DB account as returned by Azure Resource Manager.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub subscription_id: String,
    pub resource_group: String,
    pub location: String,
    pub endpoint: String,
}

/// A SQL container and the paths that make up its partition key.
#[derive(Debug, Clone, PartialEq)]
pub struct Container {
    pub name: String,
    pub partition_key_paths: Vec<String>,
}

/// One page of an Azure Resource Manager list response.
#[derive(Debug, PartialEq)]
pub struct Page<T> {
    pub items: Vec<T>,
    pub next_link: Option<String>,
}

/// Control plane operations backed by Azure Resource Manager.
#[allow(async_fn_in_trait)]
pub trait Management {
    /// Lists accounts in one subscription, or in every accessible subscription.
    async fn list_accounts(&self, subscription: Option<&str>) -> anyhow::Result<Vec<Account>>;

    /// Finds an account by name in one subscription, or in every accessible subscription.
    async fn find_account(
        &self,
        name: &str,
        subscription: Option<&str>,
    ) -> anyhow::Result<Account> {
        let accounts = self.list_accounts(subscription).await?;
        resolve_account(&accounts, name).cloned()
    }

    async fn list_databases(&self, account: &Account) -> anyhow::Result<Vec<String>>;

    async fn list_containers(
        &self,
        account: &Account,
        database: &str,
    ) -> anyhow::Result<Vec<Container>>;

    async fn get_container(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<Container>;

    async fn create_container(
        &self,
        account: &Account,
        database: &str,
        container: &str,
        partition_key_path: &str,
        throughput: Option<u32>,
    ) -> anyhow::Result<()>;

    async fn delete_container(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<()>;

    async fn primary_key(&self, account: &Account) -> anyhow::Result<String>;
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ListResponse<T> {
    value: Vec<T>,
    next_link: Option<String>,
}

#[derive(Deserialize)]
struct ArmAccount {
    id: String,
    name: String,
    location: String,
    properties: ArmAccountProperties,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArmAccountProperties {
    document_endpoint: String,
}

pub fn parse_accounts(json: &str) -> anyhow::Result<Page<Account>> {
    parse_page(json, |arm: ArmAccount| Account {
        subscription_id: id_segment(&arm.id, "subscriptions"),
        resource_group: id_segment(&arm.id, "resourceGroups"),
        name: arm.name,
        location: arm.location,
        endpoint: arm.properties.document_endpoint,
    })
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArmSubscription {
    subscription_id: String,
}

pub fn parse_subscriptions(json: &str) -> anyhow::Result<Page<String>> {
    parse_page(json, |arm: ArmSubscription| arm.subscription_id)
}

#[derive(Deserialize)]
struct ArmNamed {
    name: String,
}

pub fn parse_databases(json: &str) -> anyhow::Result<Page<String>> {
    parse_page(json, |arm: ArmNamed| arm.name)
}

#[derive(Deserialize)]
struct ArmContainer {
    name: String,
    properties: ArmContainerProperties,
}

#[derive(Deserialize)]
struct ArmContainerProperties {
    resource: ArmContainerResource,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArmContainerResource {
    partition_key: ArmPartitionKey,
}

#[derive(Deserialize)]
struct ArmPartitionKey {
    paths: Vec<String>,
}

impl From<ArmContainer> for Container {
    fn from(arm: ArmContainer) -> Self {
        Container {
            name: arm.name,
            partition_key_paths: arm.properties.resource.partition_key.paths,
        }
    }
}

pub fn parse_containers(json: &str) -> anyhow::Result<Page<Container>> {
    parse_page(json, |arm: ArmContainer| Container::from(arm))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ArmKeys {
    primary_master_key: String,
}

pub fn parse_primary_key(json: &str) -> anyhow::Result<String> {
    let keys: ArmKeys = serde_json::from_str(json)?;
    Ok(keys.primary_master_key)
}

pub fn parse_container(json: &str) -> anyhow::Result<Container> {
    let arm: ArmContainer = serde_json::from_str(json)?;
    Ok(arm.into())
}

/// Finds an account by name. Account names are globally unique in Azure.
pub fn resolve_account<'a>(accounts: &'a [Account], name: &str) -> anyhow::Result<&'a Account> {
    accounts
        .iter()
        .find(|account| account.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| anyhow::anyhow!("Cosmos DB account '{name}' not found"))
}

/// Builds a Resource Graph query that finds one Cosmos DB account by name.
pub fn account_query(name: &str) -> anyhow::Result<String> {
    let name = name.to_ascii_lowercase();
    let valid = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
    anyhow::ensure!(valid, "'{name}' is not a valid Cosmos DB account name");
    Ok(format!(
        "resources \
         | where type =~ 'microsoft.documentdb/databaseaccounts' and name =~ '{name}' \
         | project id, name, location, documentEndpoint = tostring(properties.documentEndpoint)"
    ))
}

/// Parses the accounts in a Resource Graph response to [`account_query`].
#[derive(Deserialize)]
struct GraphResponse {
    data: Vec<GraphAccount>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphAccount {
    id: String,
    name: String,
    location: String,
    document_endpoint: String,
}

pub fn parse_resource_graph_accounts(json: &str) -> anyhow::Result<Vec<Account>> {
    let response: GraphResponse = serde_json::from_str(json)?;
    Ok(response
        .data
        .into_iter()
        .map(|graph| Account {
            subscription_id: id_segment(&graph.id, "subscriptions"),
            resource_group: id_segment(&graph.id, "resourceGroups"),
            name: graph.name,
            location: graph.location,
            endpoint: graph.document_endpoint,
        })
        .collect())
}

fn parse_page<A: DeserializeOwned, T>(
    json: &str,
    map: impl FnMut(A) -> T,
) -> anyhow::Result<Page<T>> {
    let response: ListResponse<A> = serde_json::from_str(json)?;
    Ok(Page {
        items: response.value.into_iter().map(map).collect(),
        next_link: response.next_link,
    })
}

/// Returns the segment following `key` in an ARM resource id.
fn id_segment(id: &str, key: &str) -> String {
    let mut segments = id.split('/');
    segments
        .by_ref()
        .find(|segment| segment.eq_ignore_ascii_case(key));
    segments.next().unwrap_or_default().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_account_list() {
        let json = r#"{
            "value": [{
                "id": "/subscriptions/sub-1/resourceGroups/rg-data/providers/Microsoft.DocumentDB/databaseAccounts/orders",
                "name": "orders",
                "location": "West Europe",
                "kind": "GlobalDocumentDB",
                "properties": { "documentEndpoint": "https://orders.documents.azure.com:443/" }
            }]
        }"#;

        let page = parse_accounts(json).unwrap();

        assert_eq!(
            page,
            Page {
                items: vec![Account {
                    name: "orders".into(),
                    subscription_id: "sub-1".into(),
                    resource_group: "rg-data".into(),
                    location: "West Europe".into(),
                    endpoint: "https://orders.documents.azure.com:443/".into(),
                }],
                next_link: None,
            }
        );
    }

    #[test]
    fn keeps_next_link_for_paging() {
        let json = r#"{ "value": [], "nextLink": "https://management.azure.com/next?page=2" }"#;

        let page = parse_accounts(json).unwrap();

        assert_eq!(
            page.next_link.as_deref(),
            Some("https://management.azure.com/next?page=2")
        );
    }

    #[test]
    fn parses_subscription_ids() {
        let json = r#"{ "value": [
            { "id": "/subscriptions/sub-1", "subscriptionId": "sub-1", "displayName": "Production" },
            { "id": "/subscriptions/sub-2", "subscriptionId": "sub-2", "displayName": "Dev" }
        ] }"#;

        let page = parse_subscriptions(json).unwrap();

        assert_eq!(page.items, vec!["sub-1".to_string(), "sub-2".to_string()]);
    }

    #[test]
    fn parses_database_names() {
        let json = r#"{ "value": [
            { "name": "shop", "type": "Microsoft.DocumentDB/databaseAccounts/sqlDatabases",
              "properties": { "resource": { "id": "shop" } } }
        ] }"#;

        let page = parse_databases(json).unwrap();

        assert_eq!(page.items, vec!["shop".to_string()]);
    }

    #[test]
    fn parses_containers_with_partition_key_paths() {
        let json = r#"{ "value": [
            { "name": "orders", "properties": { "resource": {
                "id": "orders",
                "partitionKey": { "paths": ["/tenantId"], "kind": "Hash" }
            } } }
        ] }"#;

        let page = parse_containers(json).unwrap();

        assert_eq!(
            page.items,
            vec![Container {
                name: "orders".into(),
                partition_key_paths: vec!["/tenantId".into()],
            }]
        );
    }

    #[test]
    fn parses_primary_key_from_list_keys() {
        let json = r#"{
            "primaryMasterKey": "primary==",
            "secondaryMasterKey": "secondary==",
            "primaryReadonlyMasterKey": "ro==",
            "secondaryReadonlyMasterKey": "ro2=="
        }"#;

        assert_eq!(parse_primary_key(json).unwrap(), "primary==");
    }

    #[test]
    fn parses_single_container() {
        let json = r#"{ "name": "events", "properties": { "resource": {
            "id": "events",
            "partitionKey": { "paths": ["/deviceId"], "kind": "Hash" }
        } } }"#;

        assert_eq!(
            parse_container(json).unwrap(),
            Container {
                name: "events".into(),
                partition_key_paths: vec!["/deviceId".into()],
            }
        );
    }

    fn account(name: &str) -> Account {
        Account {
            name: name.into(),
            subscription_id: "sub-1".into(),
            resource_group: "rg".into(),
            location: "West Europe".into(),
            endpoint: format!("https://{name}.documents.azure.com:443/"),
        }
    }

    #[test]
    fn resolves_account_by_name_ignoring_case() {
        let accounts = vec![account("orders"), account("inventory")];

        let found = resolve_account(&accounts, "Inventory").unwrap();

        assert_eq!(found.name, "inventory");
    }

    #[test]
    fn unknown_account_is_an_error() {
        let accounts = vec![account("orders")];

        let error = resolve_account(&accounts, "billing").unwrap_err();

        assert_eq!(error.to_string(), "Cosmos DB account 'billing' not found");
    }

    #[test]
    fn account_query_finds_the_named_account() {
        assert_eq!(
            account_query("Orders-EU").unwrap(),
            "resources \
             | where type =~ 'microsoft.documentdb/databaseaccounts' and name =~ 'orders-eu' \
             | project id, name, location, documentEndpoint = tostring(properties.documentEndpoint)"
        );
    }

    #[test]
    fn account_query_rejects_names_that_are_not_valid_account_names() {
        assert!(account_query("x' or name != '").is_err());
        assert!(account_query("").is_err());
    }

    #[test]
    fn parses_resource_graph_accounts() {
        let json = r#"{
            "totalRecords": 1,
            "count": 1,
            "data": [{
                "id": "/subscriptions/sub-1/resourceGroups/rg-data/providers/Microsoft.DocumentDB/databaseAccounts/orders",
                "name": "orders",
                "location": "swedencentral",
                "documentEndpoint": "https://orders.documents.azure.com:443/"
            }],
            "facets": [],
            "resultTruncated": "false"
        }"#;

        assert_eq!(
            parse_resource_graph_accounts(json).unwrap(),
            vec![Account {
                name: "orders".into(),
                subscription_id: "sub-1".into(),
                resource_group: "rg-data".into(),
                location: "swedencentral".into(),
                endpoint: "https://orders.documents.azure.com:443/".into(),
            }]
        );
    }
}
