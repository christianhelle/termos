//! Runs the background work that updates ask for.

use std::time::Instant;

use cosmos_core::connector::Connector;
use cosmos_core::management::Management;
use cosmos_core::store::{DataPlane, DataStore};

use crate::state::{Effect, Msg, QueryResult, Target};

/// Does the work, reporting how it went as a message for the state.
pub async fn run<M: Management, D: DataPlane>(connector: &Connector<M, D>, effect: Effect) -> Msg {
    match effect {
        Effect::LoadAccounts => {
            Msg::AccountsLoaded(connector.list_accounts().await.map_err(describe))
        }
        Effect::LoadContainers(account) => {
            // Set up the data plane client while the containers are listed
            let (result, ()) = futures::join!(
                connector.containers_of(&account, None),
                connector.prepare(&account)
            );
            Msg::ContainersLoaded {
                account: account.name,
                result: result.map_err(describe),
            }
        }
        Effect::Query { id, target, sql } => Msg::QueryDone {
            id,
            result: query(connector, &target, &sql).await.map_err(describe),
        },
    }
}

/// The most documents a query shows.
const MAX_DOCUMENTS: usize = 100;

async fn query<M: Management, D: DataPlane>(
    connector: &Connector<M, D>,
    target: &Target,
    sql: &str,
) -> anyhow::Result<QueryResult> {
    let started = Instant::now();
    let store = connector
        .connect_to(&target.account, &target.database, &target.container)
        .await?;
    let docs = store.query(sql, Some(MAX_DOCUMENTS)).await?;
    Ok(QueryResult {
        docs,
        pk_path: store.partition_key_path().to_string(),
        elapsed: started.elapsed(),
    })
}

/// An error with its causes, as one line for the status bar.
fn describe(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_core::connector::Settings;
    use cosmos_core::store::{AuthMode, Credential};
    use cosmos_core::testing::{FakeDataPlane, FakeManagement, account, container};
    use serde_json::json;

    type TestConnector = Connector<FakeManagement, FakeDataPlane>;

    fn connector() -> TestConnector {
        let management = FakeManagement::with_databases(
            account("orders"),
            &[("shop", &[container("carts", "/tenantId")])],
        );
        let data = FakeDataPlane::new("/tenantId", vec![]);
        Connector::new(management, data, Settings::default())
    }

    #[tokio::test]
    async fn loads_the_accounts() {
        let msg = run(&connector(), Effect::LoadAccounts).await;

        let Msg::AccountsLoaded(Ok(accounts)) = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(accounts, vec![account("orders")]);
    }

    #[tokio::test]
    async fn loads_containers_and_prepares_the_data_plane_meanwhile() {
        let connector = connector();

        let msg = run(&connector, Effect::LoadContainers(account("orders"))).await;

        let Msg::ContainersLoaded { account, result } = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(account, "orders");
        assert_eq!(
            result.unwrap(),
            vec![("shop".to_string(), container("carts", "/tenantId"))]
        );
        assert_eq!(*connector.data.prepared.borrow(), vec![Credential::Entra]);
    }

    fn carts() -> Target {
        Target {
            account: account("orders"),
            database: "shop".into(),
            container: "carts".into(),
        }
    }

    #[tokio::test]
    async fn queries_up_to_a_hundred_documents_with_the_partition_key_path() {
        let mut connector = connector();
        let docs: Vec<_> = (0..150)
            .map(|i| json!({ "id": format!("c-{i}") }))
            .collect();
        connector.data = FakeDataPlane::new("/tenantId", docs);
        let sql = "SELECT * FROM c WHERE c.qty > 1".to_string();

        let msg = run(
            &connector,
            Effect::Query {
                id: 7,
                target: carts(),
                sql: sql.clone(),
            },
        )
        .await;

        let Msg::QueryDone {
            id: 7,
            result: Ok(result),
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(result.docs.len(), 100);
        assert_eq!(result.pk_path, "/tenantId");
        assert_eq!(connector.data.container.borrow().queries, vec![sql]);
    }

    #[tokio::test]
    async fn reports_a_container_it_could_not_connect_to() {
        let mut connector = connector();
        connector.data.entra_allowed = false;
        connector.settings.auth = AuthMode::Entra;

        let msg = run(
            &connector,
            Effect::Query {
                id: 1,
                target: carts(),
                sql: "SELECT * FROM c".into(),
            },
        )
        .await;

        let Msg::QueryDone {
            result: Err(error), ..
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
        assert!(error.starts_with("not authorized"), "{error}");
    }
}
