use std::io::Write;

use crate::cli::{AccountsCommand, Command, DatabasesCommand, GlobalArgs};
use crate::management::{Account, Management, resolve_account};
use crate::output::render_rows;

/// Runs CLI commands against the control and data planes.
pub struct App<M, W> {
    pub management: M,
    pub out: W,
    pub global: GlobalArgs,
}

impl<M: Management, W: Write> App<M, W> {
    pub async fn run(&mut self, command: Command) -> anyhow::Result<()> {
        match command {
            Command::Accounts {
                command: AccountsCommand::List,
            } => self.list_accounts().await,
            Command::Databases {
                command: DatabasesCommand::List { account },
            } => self.list_databases(&account).await,
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

    async fn list_databases(&mut self, account: &str) -> anyhow::Result<()> {
        let account = self.resolve(account).await?;
        for database in self.management.list_databases(&account).await? {
            writeln!(self.out, "{database}")?;
        }
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
    use crate::testing::{FakeManagement, account};

    fn app(management: FakeManagement) -> App<FakeManagement, Vec<u8>> {
        App {
            management,
            out: Vec::new(),
            global: GlobalArgs::default(),
        }
    }

    fn output(app: &App<FakeManagement, Vec<u8>>) -> String {
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
}
