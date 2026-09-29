use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::Path;
use std::rc::Rc;

use anyhow::Context;
use serde_json::Value;

use crate::cli::{
    AccountsCommand, AuthMode, Command, ContainerRef, ContainersCommand, DatabasesCommand,
    GlobalArgs, ItemsCommand, OutputFormat,
};
use crate::management::{Account, Container, Management, resolve_account};
use crate::output::{render_json, render_rows, render_table};
use crate::partition::value_at_path;
use crate::prompt::Confirm;
use crate::store::{Credential, DataPlane, DataStore, Unauthorized};

/// Identifies a container across accounts: account, database and container names.
type ContainerKey = (String, String, String);

/// Runs CLI commands against the control and data planes.
pub struct App<M, D: DataPlane, W> {
    pub management: M,
    pub data: D,
    /// Where documents are read from when no file is given.
    pub input: Box<dyn Read>,
    pub confirm: Box<dyn Confirm>,
    pub out: W,
    pub global: GlobalArgs,
    /// Accounts found so far, so each name is only looked up once.
    pub known_accounts: RefCell<Vec<Account>>,
    /// Open container connections, so each container is only connected to once.
    pub connections: RefCell<HashMap<ContainerKey, Rc<D::Store>>>,
    /// Account keys fetched so far, by account name.
    pub account_keys: RefCell<HashMap<String, String>>,
}

impl<M: Management, D: DataPlane, W: Write> App<M, D, W> {
    pub async fn run(&mut self, command: Command) -> anyhow::Result<()> {
        match command {
            Command::Accounts {
                command: AccountsCommand::List,
            } => self.list_accounts().await,
            Command::Databases {
                command: DatabasesCommand::List { account },
            } => self.list_databases(&account).await,
            Command::Containers { command } => self.containers(command).await,
            Command::Query {
                target,
                sql,
                output,
                max,
            } => self.query(&target, &sql, output, max).await,
            Command::Items { command } => self.items(command).await,
        }
    }

    async fn containers(&mut self, command: ContainersCommand) -> anyhow::Result<()> {
        match command {
            ContainersCommand::List { account, database } => {
                self.list_containers(&account, database).await
            }
            ContainersCommand::Show { target } => {
                let account = self.resolve(&target.account).await?;
                let container = self
                    .management
                    .get_container(&account, &target.database, &target.container)
                    .await?;
                writeln!(self.out, "database:      {}", target.database)?;
                writeln!(self.out, "container:     {}", container.name)?;
                writeln!(
                    self.out,
                    "partition key: {}",
                    container.partition_key_paths.join(", ")
                )?;
                Ok(())
            }
            ContainersCommand::Create {
                target,
                partition_key,
                throughput,
            } => {
                let account = self.resolve(&target.account).await?;
                self.management
                    .create_container(
                        &account,
                        &target.database,
                        &target.container,
                        &partition_key,
                        throughput,
                    )
                    .await?;
                writeln!(
                    self.out,
                    "Created container {}/{}",
                    target.database, target.container
                )?;
                Ok(())
            }
            ContainersCommand::Delete { target, yes } => self.delete_container(&target, yes).await,
        }
    }

    async fn resolve(&self, name: &str) -> anyhow::Result<Account> {
        if let Ok(account) = resolve_account(&self.known_accounts.borrow(), name) {
            return Ok(account.clone());
        }
        let account = self
            .management
            .find_account(name, self.global.subscription.as_deref())
            .await?;
        self.known_accounts.borrow_mut().push(account.clone());
        Ok(account)
    }

    async fn connect(&self, target: &ContainerRef) -> anyhow::Result<Rc<D::Store>> {
        let account = self.resolve(&target.account).await?;
        self.connect_to(&account, &target.database, &target.container)
            .await
    }

    /// Connects to a container of an already resolved account, reusing an earlier connection.
    pub(crate) async fn connect_to(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<Rc<D::Store>> {
        let key = (
            account.name.clone(),
            database.to_string(),
            container.to_string(),
        );
        if let Some(store) = self.connections.borrow().get(&key) {
            return Ok(store.clone());
        }
        let store = Rc::new(self.open(account, database, container).await?);
        self.connections.borrow_mut().insert(key, store.clone());
        Ok(store)
    }

    /// Opens a new connection to a container, honouring the auth mode.
    async fn open(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<D::Store> {
        // In auto mode a key is only fetched after Entra ID was refused for the account
        let known_key = self.account_keys.borrow().get(&account.name).cloned();
        let credential = match (&self.global.key, self.global.auth, known_key) {
            (Some(key), _, _) => Credential::Key(key.clone()),
            (None, AuthMode::Key, _) => Credential::Key(self.primary_key(account).await?),
            (None, AuthMode::Auto, Some(key)) => Credential::Key(key),
            (None, AuthMode::Entra | AuthMode::Auto, _) => Credential::Entra,
        };
        let tried_entra_first =
            self.global.auth == AuthMode::Auto && credential == Credential::Entra;
        let result = self
            .data
            .connect(account, database, container, credential)
            .await;
        match result {
            Err(error) if tried_entra_first && error.is::<Unauthorized>() => {
                let key = self.primary_key(account).await?;
                self.data
                    .connect(account, database, container, Credential::Key(key))
                    .await
            }
            result => result,
        }
    }

    /// Fetches the account key once per account.
    async fn primary_key(&self, account: &Account) -> anyhow::Result<String> {
        if let Some(key) = self.account_keys.borrow().get(&account.name) {
            return Ok(key.clone());
        }
        let key = self.management.primary_key(account).await?;
        self.account_keys
            .borrow_mut()
            .insert(account.name.clone(), key.clone());
        Ok(key)
    }

    async fn query(
        &mut self,
        target: &ContainerRef,
        sql: &str,
        output: OutputFormat,
        max: Option<usize>,
    ) -> anyhow::Result<()> {
        let store = self.connect(target).await?;
        let docs = store.query(sql, max).await?;
        let rendered = match output {
            OutputFormat::Table => render_table(&docs, store.partition_key_path()),
            OutputFormat::Json => render_json(&docs),
        };
        writeln!(self.out, "{rendered}")?;
        Ok(())
    }

    async fn items(&mut self, command: ItemsCommand) -> anyhow::Result<()> {
        match command {
            ItemsCommand::Get { target, id, pk } => {
                let store = self.connect(&target).await?;
                let doc = store.read_item(&id, &pk.value()?).await?;
                self.print_json(&doc)
            }
            ItemsCommand::Delete { target, id, pk } => {
                let store = self.connect(&target).await?;
                store.delete_item(&id, &pk.value()?).await?;
                writeln!(self.out, "Deleted document '{id}'")?;
                Ok(())
            }
            ItemsCommand::Create { target, file } => {
                let store = self.connect(&target).await?;
                let doc = self.read_document(file.as_deref())?;
                let (id, pk) = identify(&doc, store.partition_key_path())?;
                store.create_item(&pk, doc).await?;
                writeln!(self.out, "Created document '{id}'")?;
                Ok(())
            }
            ItemsCommand::Upsert { target, file } => {
                let store = self.connect(&target).await?;
                let doc = self.read_document(file.as_deref())?;
                let (id, pk) = identify(&doc, store.partition_key_path())?;
                store.upsert_item(&pk, doc).await?;
                writeln!(self.out, "Upserted document '{id}'")?;
                Ok(())
            }
            ItemsCommand::Replace { target, file } => {
                let store = self.connect(&target).await?;
                let doc = self.read_document(file.as_deref())?;
                let (id, pk) = identify(&doc, store.partition_key_path())?;
                store.replace_item(&id, &pk, doc).await?;
                writeln!(self.out, "Replaced document '{id}'")?;
                Ok(())
            }
            ItemsCommand::DeletePartition { target, pk, yes } => {
                self.delete_partition(&target, &pk.value()?, yes).await
            }
        }
    }

    async fn delete_container(&mut self, target: &ContainerRef, yes: bool) -> anyhow::Result<()> {
        let account = self.resolve(&target.account).await?;
        let name = format!("{}/{}", target.database, target.container);
        let question = format!("Delete container {name} and all of its documents?");
        if !yes && !self.confirm.confirm(&question)? {
            writeln!(self.out, "Aborted, nothing was deleted")?;
            return Ok(());
        }
        self.management
            .delete_container(&account, &target.database, &target.container)
            .await?;
        let key = (
            account.name,
            target.database.clone(),
            target.container.clone(),
        );
        self.connections.borrow_mut().remove(&key);
        writeln!(self.out, "Deleted container {name}")?;
        Ok(())
    }

    async fn delete_partition(
        &mut self,
        target: &ContainerRef,
        pk: &Value,
        yes: bool,
    ) -> anyhow::Result<()> {
        let store = self.connect(target).await?;
        let ids = store.ids_in_partition(pk).await?;
        if ids.is_empty() {
            writeln!(
                self.out,
                "No documents with partition key {pk} in {}/{}",
                target.database, target.container
            )?;
            return Ok(());
        }
        let question = format!(
            "Delete {} with partition key {pk} from {}/{}?",
            documents(ids.len()),
            target.database,
            target.container
        );
        if !yes && !self.confirm.confirm(&question)? {
            writeln!(self.out, "Aborted, nothing was deleted")?;
            return Ok(());
        }
        for id in &ids {
            store.delete_item(id, pk).await?;
        }
        writeln!(self.out, "Deleted {}", documents(ids.len()))?;
        Ok(())
    }

    fn read_document(&mut self, file: Option<&Path>) -> anyhow::Result<Value> {
        let text = match file {
            Some(path) => std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?,
            None => {
                let mut text = String::new();
                self.input.read_to_string(&mut text)?;
                text
            }
        };
        serde_json::from_str(&text).context("document is not valid JSON")
    }

    fn print_json(&mut self, doc: &Value) -> anyhow::Result<()> {
        writeln!(self.out, "{}", serde_json::to_string_pretty(doc)?)?;
        Ok(())
    }

    async fn list_databases(&mut self, account: &str) -> anyhow::Result<()> {
        let account = self.resolve(account).await?;
        for database in self.management.list_databases(&account).await? {
            writeln!(self.out, "{database}")?;
        }
        Ok(())
    }

    async fn list_containers(
        &mut self,
        account: &str,
        database: Option<String>,
    ) -> anyhow::Result<()> {
        let account = self.resolve(account).await?;
        let rows = self
            .containers_of(&account, database)
            .await?
            .into_iter()
            .map(|(database, container)| {
                let pk = container.partition_key_paths.join(", ");
                [database, container.name, pk]
            })
            .collect();
        let table = render_rows(["database", "container", "partition key"], rows);
        writeln!(self.out, "{table}")?;
        Ok(())
    }

    /// Lists the containers of one database, or of every database, with their database names.
    pub(crate) async fn containers_of(
        &self,
        account: &Account,
        database: Option<String>,
    ) -> anyhow::Result<Vec<(String, Container)>> {
        let databases = match database {
            Some(database) => vec![database],
            None => self.management.list_databases(account).await?,
        };
        let mut containers = Vec::new();
        for database in databases {
            for container in self.management.list_containers(account, &database).await? {
                containers.push((database.clone(), container));
            }
        }
        Ok(containers)
    }

    async fn list_accounts(&mut self) -> anyhow::Result<()> {
        let accounts = self
            .management
            .list_accounts(self.global.subscription.as_deref())
            .await?;
        let rows = accounts
            .into_iter()
            .map(|a| [a.name, a.resource_group, a.location, a.subscription_id])
            .collect();
        let table = render_rows(["name", "resource group", "location", "subscription"], rows);
        writeln!(self.out, "{table}")?;
        Ok(())
    }
}

/// Counts documents in words, such as "1 document" or "3 documents".
fn documents(count: usize) -> String {
    match count {
        1 => "1 document".to_string(),
        n => format!("{n} documents"),
    }
}

/// Returns the id and partition key value of a document.
fn identify(doc: &Value, pk_path: &str) -> anyhow::Result<(String, Value)> {
    let id = doc
        .get("id")
        .and_then(Value::as_str)
        .context("document has no string 'id' property")?;
    let pk = value_at_path(doc, pk_path)
        .with_context(|| format!("document has no partition key at {pk_path}"))?;
    Ok((id.to_string(), pk.clone()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::PartitionKeyArg;
    use crate::testing::{FakeDataPlane, FakeManagement, ScriptedConfirm, account, container};
    use serde_json::json;

    type TestApp = App<FakeManagement, FakeDataPlane, Vec<u8>>;

    fn app(management: FakeManagement) -> TestApp {
        app_with_data(management, FakeDataPlane::new("/tenantId", vec![]))
    }

    fn app_with_data(management: FakeManagement, data: FakeDataPlane) -> TestApp {
        App {
            management,
            data,
            input: Box::new(std::io::empty()),
            confirm: Box::new(ScriptedConfirm::answering(false)),
            out: Vec::new(),
            global: GlobalArgs::default(),
            known_accounts: RefCell::default(),
            connections: RefCell::default(),
            account_keys: RefCell::default(),
        }
    }

    fn output(app: &TestApp) -> String {
        String::from_utf8(app.out.clone()).unwrap()
    }

    #[tokio::test]
    async fn accounts_list_shows_every_account() {
        let mut app = app(FakeManagement {
            accounts: vec![account("orders"), account("inventory")],
            ..Default::default()
        });

        app.run(Command::Accounts {
            command: AccountsCommand::List,
        })
        .await
        .unwrap();

        let out = output(&app);
        assert!(out.contains("orders"), "{out}");
        assert!(out.contains("inventory"), "{out}");
        assert!(out.contains("rg-data"), "{out}");
    }

    #[tokio::test]
    async fn databases_list_shows_databases_of_the_account() {
        let mut app = app(FakeManagement::with_databases(
            account("orders"),
            &[("shop", &[]), ("audit", &[])],
        ));

        app.run(Command::Databases {
            command: DatabasesCommand::List {
                account: "orders".into(),
            },
        })
        .await
        .unwrap();

        assert_eq!(output(&app), "shop\naudit\n");
    }

    fn shop() -> FakeManagement {
        FakeManagement::with_databases(
            account("orders"),
            &[
                ("shop", &[container("carts", "/userId")]),
                ("audit", &[container("events", "/deviceId")]),
            ],
        )
    }

    #[tokio::test]
    async fn containers_list_covers_every_database_by_default() {
        let mut app = app(shop());

        app.run(Command::Containers {
            command: ContainersCommand::List {
                account: "orders".into(),
                database: None,
            },
        })
        .await
        .unwrap();

        let expected = "\
+----------+-----------+---------------+
| database | container | partition key |
+======================================+
| shop     | carts     | /userId       |
|----------+-----------+---------------|
| audit    | events    | /deviceId     |
+----------+-----------+---------------+
";
        assert_eq!(output(&app), expected);
    }

    #[tokio::test]
    async fn containers_list_can_filter_by_database() {
        let mut app = app(shop());

        app.run(Command::Containers {
            command: ContainersCommand::List {
                account: "orders".into(),
                database: Some("audit".into()),
            },
        })
        .await
        .unwrap();

        let out = output(&app);
        assert!(out.contains("events"), "{out}");
        assert!(!out.contains("carts"), "{out}");
    }

    fn orders() -> ContainerRef {
        ContainerRef {
            account: "orders".into(),
            database: "shop".into(),
            container: "carts".into(),
        }
    }

    fn orders_app(docs: Vec<serde_json::Value>) -> TestApp {
        app_with_data(shop(), FakeDataPlane::new("/tenantId", docs))
    }

    #[tokio::test]
    async fn query_renders_id_and_partition_key_table() {
        let mut app = orders_app(vec![json!({ "id": "c-1", "tenantId": "contoso" })]);

        app.run(Command::Query {
            target: orders(),
            sql: "SELECT * FROM c".into(),
            output: OutputFormat::Table,
            max: None,
        })
        .await
        .unwrap();

        let expected = "\
+-----+-----------+
| id  | /tenantId |
+=================+
| c-1 | contoso   |
+-----+-----------+
";
        assert_eq!(output(&app), expected);
        assert_eq!(app.data.container.borrow().queries, vec!["SELECT * FROM c"]);
    }

    #[tokio::test]
    async fn query_renders_json_documents_up_to_max() {
        let mut app = orders_app(vec![
            json!({ "id": "c-1", "tenantId": "contoso" }),
            json!({ "id": "c-2", "tenantId": "fabrikam" }),
        ]);

        app.run(Command::Query {
            target: orders(),
            sql: "SELECT * FROM c".into(),
            output: OutputFormat::Json,
            max: Some(1),
        })
        .await
        .unwrap();

        let expected = "[\n  {\n    \"id\": \"c-1\",\n    \"tenantId\": \"contoso\"\n  }\n]\n";
        assert_eq!(output(&app), expected);
    }

    fn contoso_pk() -> PartitionKeyArg {
        PartitionKeyArg {
            pk: Some("contoso".into()),
            pk_json: None,
        }
    }

    #[tokio::test]
    async fn items_get_prints_the_document() {
        let mut app = orders_app(vec![
            json!({ "id": "c-1", "tenantId": "fabrikam" }),
            json!({ "id": "c-1", "tenantId": "contoso", "total": 5 }),
        ]);

        app.run(Command::Items {
            command: ItemsCommand::Get {
                target: orders(),
                id: "c-1".into(),
                pk: contoso_pk(),
            },
        })
        .await
        .unwrap();

        let printed: serde_json::Value = serde_json::from_str(&output(&app)).unwrap();
        assert_eq!(
            printed,
            json!({ "id": "c-1", "tenantId": "contoso", "total": 5 })
        );
    }

    #[tokio::test]
    async fn items_delete_removes_only_that_document() {
        let mut app = orders_app(vec![
            json!({ "id": "c-1", "tenantId": "contoso" }),
            json!({ "id": "c-2", "tenantId": "contoso" }),
        ]);

        app.run(Command::Items {
            command: ItemsCommand::Delete {
                target: orders(),
                id: "c-1".into(),
                pk: contoso_pk(),
            },
        })
        .await
        .unwrap();

        assert_eq!(
            app.data.container.borrow().docs,
            vec![json!({ "id": "c-2", "tenantId": "contoso" })]
        );
        assert_eq!(output(&app), "Deleted document 'c-1'\n");
    }

    #[tokio::test]
    async fn items_create_reads_the_document_from_stdin() {
        let mut app = orders_app(vec![]);
        app.input = Box::new(r#"{ "id": "c-9", "tenantId": "contoso" }"#.as_bytes());

        app.run(Command::Items {
            command: ItemsCommand::Create {
                target: orders(),
                file: None,
            },
        })
        .await
        .unwrap();

        assert_eq!(
            app.data.container.borrow().docs,
            vec![json!({ "id": "c-9", "tenantId": "contoso" })]
        );
        assert_eq!(output(&app), "Created document 'c-9'\n");
    }

    #[tokio::test]
    async fn items_upsert_replaces_an_existing_document_from_a_file() {
        let mut app = orders_app(vec![
            json!({ "id": "c-1", "tenantId": "contoso", "total": 1 }),
        ]);
        let file =
            std::env::temp_dir().join(format!("cosmoscli-upsert-{}.json", std::process::id()));
        std::fs::write(
            &file,
            r#"{ "id": "c-1", "tenantId": "contoso", "total": 2 }"#,
        )
        .unwrap();

        let result = app
            .run(Command::Items {
                command: ItemsCommand::Upsert {
                    target: orders(),
                    file: Some(file.clone()),
                },
            })
            .await;
        std::fs::remove_file(&file).unwrap();

        result.unwrap();
        assert_eq!(
            app.data.container.borrow().docs,
            vec![json!({ "id": "c-1", "tenantId": "contoso", "total": 2 })]
        );
        assert_eq!(output(&app), "Upserted document 'c-1'\n");
    }

    #[tokio::test]
    async fn items_replace_updates_an_existing_document() {
        let mut app = orders_app(vec![
            json!({ "id": "c-1", "tenantId": "contoso", "total": 1 }),
        ]);
        app.input = Box::new(r#"{ "id": "c-1", "tenantId": "contoso", "total": 3 }"#.as_bytes());

        app.run(Command::Items {
            command: ItemsCommand::Replace {
                target: orders(),
                file: None,
            },
        })
        .await
        .unwrap();

        assert_eq!(
            app.data.container.borrow().docs,
            vec![json!({ "id": "c-1", "tenantId": "contoso", "total": 3 })]
        );
        assert_eq!(output(&app), "Replaced document 'c-1'\n");
    }

    #[tokio::test]
    async fn items_replace_fails_for_a_missing_document() {
        let mut app = orders_app(vec![]);
        app.input = Box::new(r#"{ "id": "c-1", "tenantId": "contoso" }"#.as_bytes());

        let result = app
            .run(Command::Items {
                command: ItemsCommand::Replace {
                    target: orders(),
                    file: None,
                },
            })
            .await;

        assert!(result.is_err());
        assert!(app.data.container.borrow().docs.is_empty());
    }

    fn contoso_partition() -> Vec<serde_json::Value> {
        vec![
            json!({ "id": "c-1", "tenantId": "contoso" }),
            json!({ "id": "c-2", "tenantId": "contoso" }),
            json!({ "id": "f-1", "tenantId": "fabrikam" }),
        ]
    }

    fn delete_contoso(yes: bool) -> Command {
        Command::Items {
            command: ItemsCommand::DeletePartition {
                target: orders(),
                pk: contoso_pk(),
                yes,
            },
        }
    }

    #[tokio::test]
    async fn delete_partition_asks_first_and_keeps_documents_when_declined() {
        let mut app = orders_app(contoso_partition());
        let confirm = ScriptedConfirm::answering(false);
        app.confirm = Box::new(confirm.clone());

        app.run(delete_contoso(false)).await.unwrap();

        assert_eq!(
            *confirm.asked.borrow(),
            vec!["Delete 2 documents with partition key \"contoso\" from shop/carts?"]
        );
        assert_eq!(app.data.container.borrow().docs, contoso_partition());
        assert_eq!(output(&app), "Aborted, nothing was deleted\n");
    }

    #[tokio::test]
    async fn delete_partition_removes_every_document_when_confirmed() {
        let mut app = orders_app(contoso_partition());
        app.confirm = Box::new(ScriptedConfirm::answering(true));

        app.run(delete_contoso(false)).await.unwrap();

        assert_eq!(
            app.data.container.borrow().docs,
            vec![json!({ "id": "f-1", "tenantId": "fabrikam" })]
        );
        assert_eq!(output(&app), "Deleted 2 documents\n");
    }

    #[tokio::test]
    async fn delete_partition_with_yes_skips_the_prompt() {
        let mut app = orders_app(contoso_partition());
        let confirm = ScriptedConfirm::answering(false);
        app.confirm = Box::new(confirm.clone());

        app.run(delete_contoso(true)).await.unwrap();

        assert!(confirm.asked.borrow().is_empty());
        assert_eq!(app.data.container.borrow().docs.len(), 1);
    }

    #[tokio::test]
    async fn delete_partition_does_not_ask_when_the_partition_is_empty() {
        let mut app = orders_app(vec![json!({ "id": "f-1", "tenantId": "fabrikam" })]);
        let confirm = ScriptedConfirm::answering(true);
        app.confirm = Box::new(confirm.clone());

        app.run(delete_contoso(false)).await.unwrap();

        assert!(confirm.asked.borrow().is_empty());
        assert_eq!(
            output(&app),
            "No documents with partition key \"contoso\" in shop/carts\n"
        );
    }

    fn carts() -> ContainerRef {
        ContainerRef {
            account: "orders".into(),
            database: "shop".into(),
            container: "wishlists".into(),
        }
    }

    #[tokio::test]
    async fn containers_create_adds_a_container_with_partition_key() {
        let mut app = app(shop());

        app.run(Command::Containers {
            command: ContainersCommand::Create {
                target: carts(),
                partition_key: "/userId".into(),
                throughput: None,
            },
        })
        .await
        .unwrap();

        let databases = app.management.databases.borrow();
        assert_eq!(
            databases[0].1,
            vec![
                container("carts", "/userId"),
                container("wishlists", "/userId")
            ]
        );
        assert_eq!(output(&app), "Created container shop/wishlists\n");
    }

    fn delete_carts(yes: bool) -> Command {
        Command::Containers {
            command: ContainersCommand::Delete {
                target: ContainerRef {
                    container: "carts".into(),
                    ..carts()
                },
                yes,
            },
        }
    }

    #[tokio::test]
    async fn containers_delete_asks_first_and_keeps_container_when_declined() {
        let mut app = app(shop());
        let confirm = ScriptedConfirm::answering(false);
        app.confirm = Box::new(confirm.clone());

        app.run(delete_carts(false)).await.unwrap();

        assert_eq!(
            *confirm.asked.borrow(),
            vec!["Delete container shop/carts and all of its documents?"]
        );
        assert_eq!(app.management.databases.borrow()[0].1.len(), 1);
        assert_eq!(output(&app), "Aborted, nothing was deleted\n");
    }

    #[tokio::test]
    async fn containers_delete_removes_the_container_when_confirmed() {
        let mut app = app(shop());
        app.confirm = Box::new(ScriptedConfirm::answering(true));

        app.run(delete_carts(false)).await.unwrap();

        assert!(app.management.databases.borrow()[0].1.is_empty());
        assert_eq!(output(&app), "Deleted container shop/carts\n");
    }

    #[tokio::test]
    async fn containers_show_prints_name_and_partition_key() {
        let mut app = app(shop());

        app.run(Command::Containers {
            command: ContainersCommand::Show {
                target: ContainerRef {
                    container: "carts".into(),
                    ..carts()
                },
            },
        })
        .await
        .unwrap();

        assert_eq!(
            output(&app),
            "database:      shop\ncontainer:     carts\npartition key: /userId\n"
        );
    }

    fn query_orders() -> Command {
        Command::Query {
            target: orders(),
            sql: "SELECT * FROM c".into(),
            output: OutputFormat::Json,
            max: None,
        }
    }

    fn entra_forbidden_app() -> TestApp {
        let mut data = FakeDataPlane::new("/tenantId", vec![]);
        data.entra_allowed = false;
        app_with_data(shop(), data)
    }

    #[tokio::test]
    async fn auto_auth_falls_back_to_the_account_key_when_entra_is_forbidden() {
        let mut app = entra_forbidden_app();

        app.run(query_orders()).await.unwrap();

        assert_eq!(
            *app.data.connections.borrow(),
            vec![Credential::Entra, Credential::Key("primary==".into())]
        );
    }

    #[tokio::test]
    async fn auto_auth_remembers_entra_was_refused_for_the_account() {
        let mut app = entra_forbidden_app();
        let events = ContainerRef {
            account: "orders".into(),
            database: "audit".into(),
            container: "events".into(),
        };

        app.run(query_orders()).await.unwrap();
        app.run(Command::Query {
            target: events,
            sql: "SELECT * FROM c".into(),
            output: OutputFormat::Json,
            max: None,
        })
        .await
        .unwrap();

        assert_eq!(
            *app.data.connections.borrow(),
            vec![
                Credential::Entra,
                Credential::Key("primary==".into()),
                Credential::Key("primary==".into())
            ]
        );
        assert_eq!(app.management.key_fetches.get(), 1);
    }

    #[tokio::test]
    async fn entra_auth_does_not_fall_back_to_keys() {
        let mut app = entra_forbidden_app();
        app.global.auth = AuthMode::Entra;

        let result = app.run(query_orders()).await;

        assert!(result.is_err());
        assert_eq!(*app.data.connections.borrow(), vec![Credential::Entra]);
    }

    #[tokio::test]
    async fn key_auth_fetches_the_key_without_trying_entra() {
        let mut app = orders_app(vec![]);
        app.global.auth = AuthMode::Key;

        app.run(query_orders()).await.unwrap();

        assert_eq!(
            *app.data.connections.borrow(),
            vec![Credential::Key("primary==".into())]
        );
    }

    #[tokio::test]
    async fn explicit_key_is_used_as_is() {
        let mut app = orders_app(vec![]);
        app.global.key = Some("given==".into());

        app.run(query_orders()).await.unwrap();

        assert_eq!(
            *app.data.connections.borrow(),
            vec![Credential::Key("given==".into())]
        );
    }

    #[tokio::test]
    async fn an_account_is_looked_up_once_for_many_commands() {
        let mut app = orders_app(vec![]);

        app.run(query_orders()).await.unwrap();
        app.run(query_orders()).await.unwrap();

        assert_eq!(app.management.lookups.get(), 1);
    }

    #[tokio::test]
    async fn commands_on_the_same_container_share_one_connection() {
        let mut app = orders_app(vec![json!({ "id": "c-1", "tenantId": "contoso" })]);

        app.run(query_orders()).await.unwrap();
        app.run(Command::Items {
            command: ItemsCommand::Get {
                target: orders(),
                id: "c-1".into(),
                pk: contoso_pk(),
            },
        })
        .await
        .unwrap();

        assert_eq!(app.data.connections.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_deleted_container_is_connected_to_afresh() {
        let mut app = orders_app(vec![]);
        app.confirm = Box::new(ScriptedConfirm::answering(true));

        app.run(query_orders()).await.unwrap();
        app.run(delete_carts(true)).await.unwrap();
        app.run(query_orders()).await.unwrap();

        assert_eq!(app.data.connections.borrow().len(), 2);
    }

    #[tokio::test]
    async fn delete_partition_uses_singular_for_one_document() {
        let mut app = orders_app(vec![json!({ "id": "c-1", "tenantId": "contoso" })]);
        let confirm = ScriptedConfirm::answering(true);
        app.confirm = Box::new(confirm.clone());

        app.run(delete_contoso(false)).await.unwrap();

        assert_eq!(
            *confirm.asked.borrow(),
            vec!["Delete 1 document with partition key \"contoso\" from shop/carts?"]
        );
        assert_eq!(output(&app), "Deleted 1 document\n");
    }
}
