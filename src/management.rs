use serde::{Deserialize, de::DeserializeOwned};

/// A Cosmos DB account as returned by Azure Resource Manager.
#[derive(Debug, Clone, PartialEq)]
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
}
