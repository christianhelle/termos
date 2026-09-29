use std::io::Write;

use serde_json::Value;

use crate::cli::{
    AccountsCommand, Command, ContainerRef, ContainersCommand, DatabasesCommand, GlobalArgs,
    ItemsCommand, OutputFormat, PartitionKeyArg,
};
use crate::management::{Account, Management, resolve_account};
use crate::output::{render_json, render_rows, render_table};
use crate::store::{Credential, DataPlane, DataStore};

/// Runs CLI commands against the control and data planes.
pub struct App<M, D, W> {
    pub management: M,
    pub data: D,
    pub out: W,
    pub global: GlobalArgs,
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
            Command::Containers {
                command: ContainersCommand::List { account, database },
            } => self.list_containers(&account, database).await,
            Command::Query {
                target,
                sql,
                output,
                max,
            } => self.query(&target, &sql, output, max).await,
            Command::Items { command } => self.items(command).await,
            _ => todo!(),
        }
    }

    async fn resolve(&self, name: &str) -> anyhow::Result<Account> {
        let accounts = self
            .management
            .list_accounts(self.global.subscription.as_deref())
            .await?;
        resolve_account(&accounts, name).cloned()
    }

    async fn connect(&self, target: &ContainerRef) -> anyhow::Result<D::Store> {
        let account = self.resolve(&target.account).await?;
        self.data
            .connect(
                &account,
                &target.database,
                &target.container,
                Credential::Entra,
            )
            .await
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
            _ => todo!(),
        }
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
        let databases = match database {
            Some(database) => vec![database],
            None => self.management.list_databases(&account).await?,
        };
        let mut rows = Vec::new();
        for database in databases {
            for container in self.management.list_containers(&account, &database).await? {
                let pk = container.partition_key_paths.join(", ");
                rows.push([database.clone(), container.name, pk]);
            }
        }
        let table = render_rows(["database", "container", "partition key"], rows);
        writeln!(self.out, "{table}")?;
        Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeDataPlane, FakeManagement, account, container};
    use serde_json::json;

    type TestApp = App<FakeManagement, FakeDataPlane, Vec<u8>>;

    fn app(management: FakeManagement) -> TestApp {
        app_with_data(management, FakeDataPlane::new("/tenantId", vec![]))
    }

    fn app_with_data(management: FakeManagement, data: FakeDataPlane) -> TestApp {
        App {
            management,
            data,
            out: Vec::new(),
            global: GlobalArgs::default(),
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
}
