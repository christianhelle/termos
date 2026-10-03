//! Runs the background work that updates ask for.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::time::Instant;

use crate::cache::AccountCache;
use crate::clipboard::Clipboard;
use crate::connector::Connector;
use crate::management::Management;
use crate::store::{DataPlane, DataStore, Documents};
use futures::StreamExt;
use serde_json::Value;

use crate::state::{Effect, Msg, Origin, QueryResult, QueryStats, Target};

/// Does background work against the control and data planes.
pub struct Runner<M, D: DataPlane> {
    pub connector: Connector<M, D>,
    /// The latest query of each origin, kept open while it has documents left to show.
    open_queries: RefCell<Vec<OpenQuery>>,
    /// Where listed accounts are saved for the next run.
    account_cache: Option<AccountCache>,
    clipboard: RefCell<Clipboard>,
}

/// A query with documents left to read.
struct OpenQuery {
    id: u64,
    origin: Origin,
    pk_path: String,
    pages: Documents,
    /// Documents read with a page but not handed out yet.
    unread: VecDeque<Value>,
    /// What the pages read since the documents were last handed out cost.
    stats: QueryStats,
}

impl OpenQuery {
    /// Reads pages until there are documents to hand out, returning false when
    /// the query has none left.
    async fn fill(&mut self) -> anyhow::Result<bool> {
        while self.unread.is_empty() {
            match self.pages.next().await {
                Some(page) => {
                    let page = page?;
                    self.stats.request_charge += page.request_charge;
                    self.stats.round_trips += 1;
                    if let Some(metrics) = &page.query_metrics {
                        self.stats.add_query_metrics(metrics);
                    }
                    self.unread.extend(page.docs);
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }
}

impl<M: Management, D: DataPlane> Runner<M, D> {
    pub fn new(connector: Connector<M, D>) -> Self {
        Runner {
            connector,
            open_queries: RefCell::default(),
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
            Effect::Query {
                id,
                target,
                sql,
                origin,
            } => Msg::QueryDone {
                id,
                result: self
                    .query(id, origin, &target, &sql)
                    .await
                    .map_err(describe),
            },
            Effect::LoadMore { id } => Msg::MoreLoaded {
                id,
                result: self.load_more(id).await.map_err(describe),
            },
            Effect::Delete {
                target,
                id,
                partition_key,
            } => {
                let result = self.delete(&target, &id, partition_key.as_ref()).await;
                Msg::Deleted {
                    id,
                    result: result.map_err(describe),
                }
            }
            Effect::DeleteMany { target, items } => Msg::DeletedMany {
                results: self.delete_many(&target, items).await,
            },
            Effect::LoadSettings(target) => {
                let result = self.properties(&target).await;
                Msg::SettingsLoaded {
                    target,
                    result: result.map_err(describe),
                }
            }
            Effect::Save { path, contents } => Msg::Saved(
                std::fs::write(&path, contents)
                    .map(|()| path)
                    .map_err(|error| error.to_string()),
            ),
            Effect::Copy(text) => {
                Msg::Copied(self.clipboard.borrow_mut().copy(text).map_err(describe))
            }
        }
    }

    /// Runs a query and reads its first page, keeping it open for more.
    async fn query(
        &self,
        id: u64,
        origin: Origin,
        target: &Target,
        sql: &str,
    ) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        // The query it replaces has nothing more to show
        self.open_queries
            .borrow_mut()
            .retain(|open| open.origin != origin);
        let store = self
            .connector
            .connect_to(&target.account, &target.database, &target.container)
            .await?;
        let open = OpenQuery {
            id,
            origin,
            pk_path: store.partition_key_path().to_string(),
            pages: store.documents(sql).await?,
            unread: VecDeque::new(),
            stats: QueryStats::default(),
        };
        self.read_page(open, started).await
    }

    /// Reads the properties of a container.
    async fn properties(&self, target: &Target) -> anyhow::Result<Value> {
        let store = self
            .connector
            .connect_to(&target.account, &target.database, &target.container)
            .await?;
        store.properties().await
    }

    /// Deletes a document from a container.
    async fn delete(
        &self,
        target: &Target,
        id: &str,
        partition_key: Option<&Value>,
    ) -> anyhow::Result<()> {
        let store = self
            .connector
            .connect_to(&target.account, &target.database, &target.container)
            .await?;
        store.delete(id, partition_key).await
    }

    /// Deletes documents from a container, one at a time, keeping each outcome.
    async fn delete_many(
        &self,
        target: &Target,
        items: Vec<(String, Option<Value>)>,
    ) -> Vec<(String, Result<(), String>)> {
        let store = self
            .connector
            .connect_to(&target.account, &target.database, &target.container)
            .await;
        let mut results = Vec::with_capacity(items.len());
        for (id, partition_key) in items {
            let result = match &store {
                Ok(store) => store.delete(&id, partition_key.as_ref()).await,
                Err(error) => Err(anyhow::anyhow!("{error:#}")),
            };
            results.push((id, result.map_err(describe)));
        }
        results
    }

    /// Reads the next page of an open query.
    async fn load_more(&self, id: u64) -> anyhow::Result<QueryResult> {
        let started = Instant::now();
        let open = {
            let mut open_queries = self.open_queries.borrow_mut();
            let index = open_queries.iter().position(|open| open.id == id);
            index.map(|index| open_queries.remove(index))
        };
        match open {
            Some(open) => self.read_page(open, started).await,
            None => anyhow::bail!("the query has no more documents to load"),
        }
    }

    async fn read_page(
        &self,
        mut open: OpenQuery,
        started: Instant,
    ) -> anyhow::Result<QueryResult> {
        let mut docs = Vec::with_capacity(PAGE_SIZE);
        while docs.len() < PAGE_SIZE && open.fill().await? {
            let count = (PAGE_SIZE - docs.len()).min(open.unread.len());
            docs.extend(open.unread.drain(..count));
        }
        let more = docs.len() == PAGE_SIZE && open.fill().await?;
        let pk_path = open.pk_path.clone();
        let stats = std::mem::take(&mut open.stats);
        if more {
            self.open_queries.borrow_mut().push(open);
        }
        Ok(QueryResult {
            docs,
            more,
            pk_path,
            elapsed: started.elapsed(),
            stats,
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
                origin: Origin::Browse,
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
                origin: Origin::Browse,
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
            origin: Origin::Browse,
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
    async fn hands_out_a_hundred_documents_at_a_time_whatever_the_page_size() {
        let runner = with_carts(130);
        runner.connector.data.container.borrow_mut().page_size = 30;

        let Msg::QueryDone {
            result: Ok(first), ..
        } = runner.run(query_carts(1)).await
        else {
            panic!("the query failed");
        };
        let Msg::MoreLoaded {
            result: Ok(second), ..
        } = runner.run(Effect::LoadMore { id: 1 }).await
        else {
            panic!("loading more failed");
        };

        assert_eq!(
            ids(&first.docs),
            (0..100).map(|i| format!("c-{i}")).collect::<Vec<_>>()
        );
        assert!(first.more);
        assert_eq!(
            ids(&second.docs),
            (100..130).map(|i| format!("c-{i}")).collect::<Vec<_>>()
        );
        assert!(!second.more);
    }

    #[tokio::test]
    async fn counts_the_request_units_and_round_trips_each_read_took() {
        let runner = with_carts(130);
        {
            let mut container = runner.connector.data.container.borrow_mut();
            container.page_size = 30;
            container.page_charge = 2.5;
            container.page_metrics = Some("retrievedDocumentCount=30".into());
        }

        let Msg::QueryDone {
            result: Ok(first), ..
        } = runner.run(query_carts(1)).await
        else {
            panic!("the query failed");
        };
        let Msg::MoreLoaded {
            result: Ok(second), ..
        } = runner.run(Effect::LoadMore { id: 1 }).await
        else {
            panic!("loading more failed");
        };

        // Four pages hold the first hundred, and the fifth the last ten
        assert_eq!(first.stats.round_trips, 4);
        assert_eq!(first.stats.request_charge, 10.0);
        assert_eq!(
            first.stats.query_metrics,
            vec![("retrievedDocumentCount".to_string(), 120.0)]
        );
        assert_eq!(second.stats.round_trips, 1);
        assert_eq!(second.stats.request_charge, 2.5);
    }

    #[tokio::test]
    async fn saves_text_to_a_file_and_says_where() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("carts.sql").display().to_string();

        let msg = runner()
            .run(Effect::Save {
                path: path.clone(),
                contents: "SELECT * FROM c\n".into(),
            })
            .await;

        let Msg::Saved(Ok(saved)) = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(saved, path);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "SELECT * FROM c\n");
    }

    #[tokio::test]
    async fn reports_a_file_it_could_not_save() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("missing").join("carts.sql");

        let msg = runner()
            .run(Effect::Save {
                path: path.display().to_string(),
                contents: String::new(),
            })
            .await;

        assert!(matches!(msg, Msg::Saved(Err(_))), "{msg:?}");
    }

    #[tokio::test]
    async fn the_editor_query_leaves_the_browse_query_open_for_more() {
        let runner = with_carts(250);
        runner.run(query_carts(1)).await;
        runner
            .run(Effect::Query {
                id: 2,
                target: carts(),
                sql: "SELECT VALUE c.id FROM c".into(),
                origin: Origin::Editor,
            })
            .await;

        let browse = runner.run(Effect::LoadMore { id: 1 }).await;
        let editor = runner.run(Effect::LoadMore { id: 2 }).await;

        for (msg, first) in [(browse, "c-100"), (editor, "c-100")] {
            let Msg::MoreLoaded {
                result: Ok(result), ..
            } = msg
            else {
                panic!("unexpected {msg:?}");
            };
            assert_eq!(ids(&result.docs)[0], first);
        }
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

    fn delete_cart(id: &str) -> Effect {
        Effect::Delete {
            target: carts(),
            id: id.into(),
            partition_key: Some(json!("contoso")),
        }
    }

    #[tokio::test]
    async fn deletes_a_document_by_id_and_partition_key() {
        let mut runner = runner();
        let docs = vec![json!({ "id": "c-1" }), json!({ "id": "c-2" })];
        runner.connector.data = FakeDataPlane::new("/tenantId", docs);

        let msg = runner.run(delete_cart("c-1")).await;

        let Msg::Deleted { id, result: Ok(()) } = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(id, "c-1");
        let container = runner.connector.data.container.borrow();
        assert_eq!(container.docs, vec![json!({ "id": "c-2" })]);
        assert_eq!(
            container.deletes,
            vec![("c-1".to_string(), Some(json!("contoso")))]
        );
    }

    #[tokio::test]
    async fn deletes_many_documents_and_reports_each_outcome() {
        let mut runner = runner();
        let docs = vec![json!({ "id": "c-1" }), json!({ "id": "c-2" })];
        runner.connector.data = FakeDataPlane::new("/tenantId", docs);

        let msg = runner
            .run(Effect::DeleteMany {
                target: carts(),
                items: vec![
                    ("c-1".into(), Some(json!("contoso"))),
                    ("gone".into(), Some(json!("contoso"))),
                    ("c-2".into(), Some(json!("contoso"))),
                ],
            })
            .await;

        let Msg::DeletedMany { results } = msg else {
            panic!("unexpected {msg:?}");
        };
        let outcomes: Vec<(&str, bool)> = results
            .iter()
            .map(|(id, result)| (id.as_str(), result.is_ok()))
            .collect();
        assert_eq!(outcomes, [("c-1", true), ("gone", false), ("c-2", true)]);
        assert!(runner.connector.data.container.borrow().docs.is_empty());
    }

    #[tokio::test]
    async fn reports_a_document_it_could_not_delete() {
        let mut runner = runner();
        runner.connector.data = FakeDataPlane::new("/tenantId", vec![]);

        let msg = runner.run(delete_cart("gone")).await;

        let Msg::Deleted {
            result: Err(error), ..
        } = msg
        else {
            panic!("unexpected {msg:?}");
        };
        assert!(error.contains("gone"), "{error}");
    }

    #[tokio::test]
    async fn saves_the_listed_accounts_for_the_next_run() {
        let dir = tempfile::tempdir().unwrap();
        let runner = runner().with_account_cache(AccountCache::in_dir(dir.path(), None));

        runner.run(Effect::LoadAccounts).await;

        let cached = AccountCache::in_dir(dir.path(), None).load();
        assert_eq!(cached, Some(vec![account("orders")]));
    }

    #[tokio::test]
    async fn loads_the_properties_of_a_container_for_its_settings() {
        let runner = runner();
        let properties = json!({ "id": "carts", "defaultTtl": -1 });
        runner.connector.data.container.borrow_mut().properties = properties.clone();

        let msg = runner.run(Effect::LoadSettings(carts())).await;

        let Msg::SettingsLoaded { target, result } = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(target, carts());
        assert_eq!(result, Ok(properties));
    }
}
