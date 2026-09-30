//! Finds accounts and opens container connections, remembering both.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use crate::management::{Account, Container, Management, resolve_account};
use crate::store::{AuthMode, Credential, DataPlane, Unauthorized};

/// Identifies a container across accounts: account, database and container names.
type ContainerKey = (String, String, String);

/// Where accounts are looked for and how documents are authenticated.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Settings {
    /// Only use this subscription instead of every subscription you can access.
    pub subscription: Option<String>,
    pub auth: AuthMode,
    /// Account key to use instead of fetching one from Resource Manager.
    pub key: Option<String>,
}

/// Connects to accounts and containers on the control and data planes.
pub struct Connector<M, D: DataPlane> {
    pub management: M,
    pub data: D,
    pub settings: Settings,
    /// Accounts found so far, so each name is only looked up once.
    known_accounts: RefCell<Vec<Account>>,
    /// Open container connections, so each container is only connected to once.
    connections: RefCell<HashMap<ContainerKey, Rc<D::Store>>>,
    /// Account keys fetched so far, by account name.
    account_keys: RefCell<HashMap<String, String>>,
}

impl<M: Management, D: DataPlane> Connector<M, D> {
    pub fn new(management: M, data: D, settings: Settings) -> Self {
        Connector {
            management,
            data,
            settings,
            known_accounts: RefCell::default(),
            connections: RefCell::default(),
            account_keys: RefCell::default(),
        }
    }

    /// Lists every account and remembers them, so later lookups by name need no request.
    pub async fn list_accounts(&self) -> anyhow::Result<Vec<Account>> {
        let accounts = self
            .management
            .list_accounts(self.settings.subscription.as_deref())
            .await?;
        self.remember_accounts(&accounts);
        Ok(accounts)
    }

    /// Remembers accounts listed elsewhere, such as ahead of time.
    pub fn remember_accounts(&self, accounts: &[Account]) {
        *self.known_accounts.borrow_mut() = accounts.to_vec();
    }

    /// Finds an account by name, looking each name up only once.
    pub async fn resolve(&self, name: &str) -> anyhow::Result<Account> {
        if let Ok(account) = resolve_account(&self.known_accounts.borrow(), name) {
            return Ok(account.clone());
        }
        let account = self
            .management
            .find_account(name, self.settings.subscription.as_deref())
            .await?;
        self.known_accounts.borrow_mut().push(account.clone());
        Ok(account)
    }

    /// Connects to a container of an already resolved account, reusing an earlier connection.
    pub async fn connect_to(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<Rc<D::Store>> {
        let key = container_key(account, database, container);
        if let Some(store) = self.connections.borrow().get(&key) {
            return Ok(store.clone());
        }
        let store = Rc::new(self.open(account, database, container).await?);
        self.connections.borrow_mut().insert(key, store.clone());
        Ok(store)
    }

    /// Drops the connection to a container, such as after it was deleted.
    pub fn forget_container(&self, account: &Account, database: &str, container: &str) {
        let key = container_key(account, database, container);
        self.connections.borrow_mut().remove(&key);
    }

    /// Opens a new connection to a container, honouring the auth mode.
    async fn open(
        &self,
        account: &Account,
        database: &str,
        container: &str,
    ) -> anyhow::Result<D::Store> {
        let credential = self.first_credential(account).await?;
        let tried_entra_first =
            self.settings.auth == AuthMode::Auto && credential == Credential::Entra;
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

    /// Gets the data plane ready for the account's containers. Failures surface on connect.
    pub async fn prepare(&self, account: &Account) {
        if let Ok(credential) = self.first_credential(account).await {
            self.data.prepare(account, credential).await;
        }
    }

    /// The credential to try first for an account's documents, honouring the auth mode.
    async fn first_credential(&self, account: &Account) -> anyhow::Result<Credential> {
        // In auto mode a key is only fetched after Entra ID was refused for the account
        let known_key = self.account_keys.borrow().get(&account.name).cloned();
        Ok(match (&self.settings.key, self.settings.auth, known_key) {
            (Some(key), _, _) => Credential::Key(key.clone()),
            (None, AuthMode::Key, _) => Credential::Key(self.primary_key(account).await?),
            (None, AuthMode::Auto, Some(key)) => Credential::Key(key),
            (None, AuthMode::Entra | AuthMode::Auto, _) => Credential::Entra,
        })
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

    /// Lists the containers of one database, or of every database, with their database names.
    pub async fn containers_of(
        &self,
        account: &Account,
        database: Option<String>,
    ) -> anyhow::Result<Vec<(String, Container)>> {
        let databases = match database {
            Some(database) => vec![database],
            None => self.management.list_databases(account).await?,
        };
        let listings = databases.iter().map(|database| async move {
            let containers = self.management.list_containers(account, database).await?;
            anyhow::Ok(containers.into_iter().map(|c| (database.clone(), c)))
        });
        // Every database is listed at once, and results keep the database order
        let containers = futures::future::try_join_all(listings).await?;
        Ok(containers.into_iter().flatten().collect())
    }
}

fn container_key(account: &Account, database: &str, container: &str) -> ContainerKey {
    (
        account.name.clone(),
        database.to_string(),
        container.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FakeDataPlane, FakeManagement, account, container};

    type TestConnector = Connector<FakeManagement, FakeDataPlane>;

    fn shop() -> FakeManagement {
        FakeManagement::with_databases(
            account("orders"),
            &[
                ("shop", &[container("carts", "/tenantId")]),
                ("audit", &[container("events", "/tenantId")]),
            ],
        )
    }

    fn connector(auth: AuthMode, entra_allowed: bool) -> TestConnector {
        let mut data = FakeDataPlane::new("/tenantId", vec![]);
        data.entra_allowed = entra_allowed;
        let settings = Settings {
            auth,
            ..Default::default()
        };
        Connector::new(shop(), data, settings)
    }

    async fn connect(
        connector: &TestConnector,
        database: &str,
        container: &str,
    ) -> anyhow::Result<()> {
        let account = connector.resolve("orders").await?;
        connector.connect_to(&account, database, container).await?;
        Ok(())
    }

    #[tokio::test]
    async fn auto_auth_falls_back_to_the_account_key_when_entra_is_forbidden() {
        let connector = connector(AuthMode::Auto, false);

        connect(&connector, "shop", "carts").await.unwrap();

        assert_eq!(
            *connector.data.connections.borrow(),
            vec![Credential::Entra, Credential::Key("primary==".into())]
        );
    }

    #[tokio::test]
    async fn auto_auth_remembers_entra_was_refused_for_the_account() {
        let connector = connector(AuthMode::Auto, false);

        connect(&connector, "shop", "carts").await.unwrap();
        connect(&connector, "audit", "events").await.unwrap();

        assert_eq!(
            *connector.data.connections.borrow(),
            vec![
                Credential::Entra,
                Credential::Key("primary==".into()),
                Credential::Key("primary==".into())
            ]
        );
        assert_eq!(connector.management.key_fetches.get(), 1);
    }

    #[tokio::test]
    async fn entra_auth_does_not_fall_back_to_keys() {
        let connector = connector(AuthMode::Entra, false);

        let result = connect(&connector, "shop", "carts").await;

        assert!(result.is_err());
        assert_eq!(
            *connector.data.connections.borrow(),
            vec![Credential::Entra]
        );
    }

    #[tokio::test]
    async fn key_auth_fetches_the_key_without_trying_entra() {
        let connector = connector(AuthMode::Key, true);

        connect(&connector, "shop", "carts").await.unwrap();

        assert_eq!(
            *connector.data.connections.borrow(),
            vec![Credential::Key("primary==".into())]
        );
    }

    #[tokio::test]
    async fn explicit_key_is_used_as_is() {
        let mut connector = connector(AuthMode::Auto, true);
        connector.settings.key = Some("given==".into());

        connect(&connector, "shop", "carts").await.unwrap();

        assert_eq!(
            *connector.data.connections.borrow(),
            vec![Credential::Key("given==".into())]
        );
    }

    #[tokio::test]
    async fn an_account_is_looked_up_once() {
        let connector = connector(AuthMode::Auto, true);

        connector.resolve("orders").await.unwrap();
        connector.resolve("orders").await.unwrap();

        assert_eq!(connector.management.lookups.get(), 1);
    }

    #[tokio::test]
    async fn listed_accounts_need_no_lookup() {
        let connector = connector(AuthMode::Auto, true);

        connector.list_accounts().await.unwrap();
        connector.resolve("orders").await.unwrap();

        assert_eq!(connector.management.lookups.get(), 0);
    }

    #[tokio::test]
    async fn a_container_is_connected_to_once() {
        let connector = connector(AuthMode::Auto, true);

        connect(&connector, "shop", "carts").await.unwrap();
        connect(&connector, "shop", "carts").await.unwrap();

        assert_eq!(connector.data.connections.borrow().len(), 1);
    }

    #[tokio::test]
    async fn a_forgotten_container_is_connected_to_afresh() {
        let connector = connector(AuthMode::Auto, true);
        let orders = account("orders");

        connect(&connector, "shop", "carts").await.unwrap();
        connector.forget_container(&orders, "shop", "carts");
        connect(&connector, "shop", "carts").await.unwrap();

        assert_eq!(connector.data.connections.borrow().len(), 2);
    }

    #[tokio::test]
    async fn containers_of_every_database_keep_the_database_order() {
        let connector = connector(AuthMode::Auto, true);

        let containers = connector
            .containers_of(&account("orders"), None)
            .await
            .unwrap();

        let names: Vec<_> = containers
            .iter()
            .map(|(database, c)| format!("{database}/{}", c.name))
            .collect();
        assert_eq!(names, vec!["shop/carts", "audit/events"]);
    }
}
