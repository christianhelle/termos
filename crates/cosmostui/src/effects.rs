//! Runs the background work that updates ask for.

use std::pin::Pin;
use std::time::Instant;

use cosmos_core::connector::Connector;
use cosmos_core::management::Management;
use cosmos_core::store::{DataPlane, DataStore, Documents};
use futures::StreamExt;
use futures::stream::Peekable;
use serde_json::Value;

use crate::state::{Effect, Msg, QueryResult, Target};

/// Does background work against the control and data planes.
pub struct Runner<M, D: DataPlane> {
    pub connector: Connector<M, D>,
}

impl<M: Management, D: DataPlane> Runner<M, D> {
    pub fn new(connector: Connector<M, D>) -> Self {
        Runner { connector }
    }

    /// Does the work, reporting how it went as a message for the state.
    pub async fn run(&self, effect: Effect) -> Msg {
        let connector = &self.connector;
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
}

/// How many documents a query shows at a time.
const PAGE_SIZE: usize = 100;

async fn query<M: Management, D: DataPlane>(
    connector: &Connector<M, D>,
    target: &Target,
    sql: &str,
) -> anyhow::Result<QueryResult> {
    let started = Instant::now();
    let store = connector
        .connect_to(&target.account, &target.database, &target.container)
        .await?;
    let mut docs = store.documents(sql).await?.peekable();
    let (docs, more) = next_page(&mut docs).await?;
    Ok(QueryResult {
        docs,
        more,
        pk_path: store.partition_key_path().to_string(),
        elapsed: started.elapsed(),
    })
}

/// Reads the next page of documents, and whether more follow it.
async fn next_page(docs: &mut Peekable<Documents>) -> anyhow::Result<(Vec<Value>, bool)> {
    let mut page = Vec::with_capacity(PAGE_SIZE);
    while page.len() < PAGE_SIZE {
        match docs.next().await {
            Some(doc) => page.push(doc?),
            None => return Ok((page, false)),
        }
    }
    let more = Pin::new(docs).peek().await.is_some();
    Ok((page, more))
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

    type TestRunner = Runner<FakeManagement, FakeDataPlane>;

    fn runner() -> TestRunner {
        let management = FakeManagement::with_databases(
            account("orders"),
            &[("shop", &[container("carts", "/tenantId")])],
        );
        let data = FakeDataPlane::new("/tenantId", vec![]);
        Runner::new(Connector::new(management, data, Settings::default()))
    }

    #[tokio::test]
    async fn loads_the_accounts() {
        let msg = runner().run(Effect::LoadAccounts).await;

        let Msg::AccountsLoaded(Ok(accounts)) = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(accounts, vec![account("orders")]);
    }

    #[tokio::test]
    async fn loads_containers_and_prepares_the_data_plane_meanwhile() {
        let runner = runner();

        let msg = runner.run(Effect::LoadContainers(account("orders"))).await;

        let Msg::ContainersLoaded { account, result } = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(account, "orders");
        assert_eq!(
            result.unwrap(),
            vec![("shop".to_string(), container("carts", "/tenantId"))]
        );
        assert_eq!(
            *runner.connector.data.prepared.borrow(),
            vec![Credential::Entra]
        );
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
        let mut runner = runner();
        let docs: Vec<_> = (0..150)
            .map(|i| json!({ "id": format!("c-{i}") }))
            .collect();
        runner.connector.data = FakeDataPlane::new("/tenantId", docs);
        let sql = "SELECT * FROM c WHERE c.qty > 1".to_string();

        let msg = runner
            .run(Effect::Query {
                id: 7,
                target: carts(),
                sql: sql.clone(),
            })
            .await;

        let Msg::QueryDone {
            id: 7,
            result: Ok(result),
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(result.docs.len(), 100);
        assert!(result.more);
        assert_eq!(result.pk_path, "/tenantId");
        assert_eq!(runner.connector.data.container.borrow().queries, vec![sql]);
    }

    #[tokio::test]
    async fn reports_a_container_it_could_not_connect_to() {
        let mut runner = runner();
        runner.connector.data.entra_allowed = false;
        runner.connector.settings.auth = AuthMode::Entra;

        let msg = runner
            .run(Effect::Query {
                id: 1,
                target: carts(),
                sql: "SELECT * FROM c".into(),
            })
            .await;

        let Msg::QueryDone {
            result: Err(error), ..
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
        assert!(error.starts_with("not authorized"), "{error}");
    }

    fn with_carts(count: usize) -> TestRunner {
        let mut runner = runner();
        let docs = (0..count)
            .map(|i| json!({ "id": format!("c-{i}") }))
            .collect();
        runner.connector.data = FakeDataPlane::new("/tenantId", docs);
        runner
    }

    fn query_carts(id: u64) -> Effect {
        Effect::Query {
            id,
            target: carts(),
            sql: "SELECT * FROM c".into(),
        }
    }

    #[tokio::test]
    async fn a_query_that_found_everything_has_no_more_documents() {
        let msg = with_carts(100).run(query_carts(1)).await;

        let Msg::QueryDone {
            result: Ok(result), ..
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(result.docs.len(), 100);
        assert!(!result.more);
    }
}
