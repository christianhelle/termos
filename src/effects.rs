//! Runs the background work that updates ask for.

use std::cell::RefCell;
use std::pin::Pin;
use std::time::Instant;

use crate::cache::AccountCache;
use crate::clipboard::Clipboard;
use crate::connector::Connector;
use crate::management::Management;
use crate::store::{DataPlane, DataStore, Documents};
use futures::StreamExt;
use futures::stream::Peekable;

use crate::state::{Effect, Msg, QueryResult, Target};

/// Does background work against the control and data planes.
pub struct Runner<M, D: DataPlane> {
    pub connector: Connector<M, D>,
    /// The latest query, kept open while it has documents left to show.
    open_query: RefCell<Option<OpenQuery>>,
    /// Where listed accounts are saved for the next run.
    account_cache: Option<AccountCache>,
    clipboard: RefCell<Clipboard>,
}

/// A query with documents left to read.
struct OpenQuery {
    id: u64,
    pk_path: String,
    docs: Peekable<Documents>,
}

impl<M: Management, D: DataPlane> Runner<M, D> {
    pub fn new(connector: Connector<M, D>) -> Self {
        Runner {
            connector,
            open_query: RefCell::default(),
            account_cache: None,
            clipboard: RefCell::default(),
        }
    }

    /// Saves every account list to the cache, so the next run can show it at once.
    pub fn with_account_cache(mut self, cache: AccountCache) -> Self {
        self.account_cache = Some(cache);
        self
    }

    /// Does the work, reporting how it went as a message for the state.
    pub async fn run(&self, effect: Effect) -> Msg {
        let connector = &self.connector;
        match effect {
            Effect::LoadAccounts => {
                let result = connector.list_accounts().await;
                if let (Ok(accounts), Some(cache)) = (&result, &self.account_cache) {
                    // A cache that cannot be written only costs the next run its head start
                    let _ = cache.save(accounts);
                }
                Msg::AccountsLoaded(result.map_err(describe))
            }
            Effect::LoadContainers(account) => {
                // Set up the data plane client while the containers are listed
                let (result, ()) = futures::join!(
                    connector.containers_of(&account),
                    connector.prepare(&account)
                );
                Msg::ContainersLoaded {
                    account: account.name,
                    result: result.map_err(describe),
                }
            }
            Effect::Query { id, target, sql } => Msg::QueryDone {
                id,
                result: self.query(id, &target, &sql).await.map_err(describe),
            },
            Effect::LoadMore { id } => Msg::MoreLoaded {
                id,
                result: self.load_more(id).await.map_err(describe),
            },
            Effect::Copy(text) => {
                Msg::Copied(self.clipboard.borrow_mut().copy(text).map_err(describe))
            }
        }
    }

    /// Runs a query and reads its first page, keeping it open for more.
    async fn query(&self, id: u64, target: &Target, sql: &str) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let store = self
            .connector
            .connect_to(&target.account, &target.database, &target.container)
            .await?;
        let open = OpenQuery {
            id,
            pk_path: store.partition_key_path().to_string(),
            docs: store.documents(sql).await?.peekable(),
        };
        self.read_page(open, started).await
    }

    /// Reads the next page of the latest query.
    async fn load_more(&self, id: u64) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let open = self.open_query.borrow_mut().take();
        match open {
            Some(open) if open.id == id => self.read_page(open, started).await,
            _ => anyhow::bail!("the query has no more documents to load"),
        }
    }

    async fn read_page(
        &self,
        mut open: OpenQuery,
        started: Instant,
    ) -> anyhow::Result<QueryResult> {
        let mut docs = Vec::with_capacity(PAGE_SIZE);
        while docs.len() < PAGE_SIZE {
            match open.docs.next().await {
                Some(doc) => docs.push(doc?),
                None => break,
            }
        }
        let more = docs.len() == PAGE_SIZE && Pin::new(&mut open.docs).peek().await.is_some();
        let pk_path = open.pk_path.clone();
        if more {
            *self.open_query.borrow_mut() = Some(open);
        }
        Ok(QueryResult {
            docs,
            more,
            pk_path,
            elapsed: started.elapsed(),
        })
    }
}

/// How many documents a query shows at a time.
const PAGE_SIZE: usize = 100;

/// An error with its causes, as one line for the status bar.
fn describe(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::connector::Settings;
    use crate::store::{AuthMode, Credential};
    use crate::testing::{FakeDataPlane, FakeManagement, account, container};
    use serde_json::{Value, json};

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

    fn ids(docs: &[Value]) -> Vec<String> {
        docs.iter()
            .map(|doc| doc["id"].as_str().unwrap().to_string())
            .collect()
    }

    #[tokio::test]
    async fn loading_more_continues_the_query_where_it_stopped() {
        let runner = with_carts(250);
        runner.run(query_carts(1)).await;

        let second = runner.run(Effect::LoadMore { id: 1 }).await;
        let third = runner.run(Effect::LoadMore { id: 1 }).await;

        let Msg::MoreLoaded {
            id: 1,
            result: Ok(second),
        } = second
        else {
            panic!("unexpected {second:?}");
        };
        let Msg::MoreLoaded {
            id: 1,
            result: Ok(third),
        } = third
        else {
            panic!("unexpected {third:?}");
        };
        assert_eq!(ids(&second.docs)[0], "c-100");
        assert!(second.more);
        assert_eq!(
            ids(&third.docs),
            (200..250).map(|i| format!("c-{i}")).collect::<Vec<_>>()
        );
        assert!(!third.more);
        assert_eq!(runner.connector.data.container.borrow().queries.len(), 1);
    }

    #[tokio::test]
    async fn loading_more_of_a_superseded_query_fails() {
        let runner = with_carts(250);
        runner.run(query_carts(1)).await;
        runner.run(query_carts(2)).await;

        let msg = runner.run(Effect::LoadMore { id: 1 }).await;

        let Msg::MoreLoaded {
            id: 1,
            result: Err(_),
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
    }

    #[tokio::test]
    async fn saves_the_listed_accounts_for_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let runner = runner().with_account_cache(AccountCache::in_dir(dir.path(), None));

        runner.run(Effect::LoadAccounts).await;

        let cached = AccountCache::in_dir(dir.path(), None).load();
        assert_eq!(cached, Some(vec![account("orders")]));
    }
}
