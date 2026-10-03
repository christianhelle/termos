//! In-memory fakes of the control and data planes, for tests.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde_json::Value;

use crate::management::{Account, Container, Management};
use crate::store::{Credential, DataPlane, DataStore, Documents, Page, Unauthorized};
use futures::StreamExt;

pub fn account(name: &str) -> Account {
    Account {
        name: name.into(),
        subscription_id: "sub-1".into(),
        resource_group: "rg-data".into(),
        location: "West Europe".into(),
        endpoint: format!("https://{name}.documents.azure.com:443/"),
    }
}

pub fn container(name: &str, pk_path: &str) -> Container {
    Container {
        name: name.into(),
        partition_key_paths: vec![pk_path.into()],
    }
}

#[derive(Default)]
pub struct FakeManagement {
    pub accounts: Vec<Account>,
    /// Databases and their containers, shared by every account.
    pub databases: Vec<(String, Vec<Container>)>,
    /// How many times an account key was fetched.
    pub key_fetches: Cell<usize>,
}

impl FakeManagement {
    pub fn with_databases(account: Account, databases: &[(&str, &[Container])]) -> Self {
        FakeManagement {
            accounts: vec![account],
            databases: databases
                .iter()
                .map(|(name, containers)| (name.to_string(), containers.to_vec()))
                .collect(),
            ..Default::default()
        }
    }
}

impl Management for FakeManagement {
    async fn list_accounts(&self, subscription: Option<&str>) -> anyhow::Result<Vec<Account>> {
        Ok(self
            .accounts
            .iter()
            .filter(|account| subscription.is_none_or(|id| account.subscription_id == id))
            .cloned()
            .collect())
    }

    async fn list_databases(&self, _account: &Account) -> anyhow::Result<Vec<String>> {
        Ok(self
            .databases
            .iter()
            .map(|(name, _)| name.clone())
            .collect())
    }

    async fn list_containers(
        &self,
        _account: &Account,
        database: &str,
    ) -> anyhow::Result<Vec<Container>> {
        Ok(self
            .databases
            .iter()
            .find(|(name, _)| name == database)
            .map(|(_, containers)| containers.clone())
            .unwrap_or_default())
    }

    async fn primary_key(&self, _account: &Account) -> anyhow::Result<String> {
        self.key_fetches.set(self.key_fetches.get() + 1);
        Ok("primary==".into())
    }
}

/// Shared state behind a fake container.
#[derive(Default)]
pub struct FakeContainer {
    pub docs: Vec<Value>,
    pub queries: Vec<String>,
    /// The id and partition key value of each delete asked for.
    pub deletes: Vec<(String, Option<Value>)>,
    /// How many documents each page of a query holds.
    pub page_size: usize,
    /// The request units each page of a query costs.
    pub page_charge: f64,
}

pub struct FakeDataPlane {
    pub pk_path: String,
    /// When false, Entra ID connections are rejected as unauthorized.
    pub entra_allowed: bool,
    pub container: Rc<RefCell<FakeContainer>>,
    pub connections: RefCell<Vec<Credential>>,
    /// Credentials the data plane was asked to prepare clients for.
    pub prepared: RefCell<Vec<Credential>>,
}

impl FakeDataPlane {
    pub fn new(pk_path: &str, docs: Vec<Value>) -> Self {
        FakeDataPlane {
            pk_path: pk_path.into(),
            entra_allowed: true,
            container: Rc::new(RefCell::new(FakeContainer {
                docs,
                page_size: 1000,
                ..Default::default()
            })),
            connections: RefCell::default(),
            prepared: RefCell::default(),
        }
    }
}

impl DataPlane for FakeDataPlane {
    type Store = FakeStore;

    async fn connect(
        &self,
        _account: &Account,
        _database: &str,
        _container: &str,
        credential: Credential,
    ) -> anyhow::Result<FakeStore> {
        self.connections.borrow_mut().push(credential.clone());
        if credential == Credential::Entra && !self.entra_allowed {
            return Err(Unauthorized("Entra ID principal lacks a data plane role".into()).into());
        }
        Ok(FakeStore {
            pk_path: self.pk_path.clone(),
            container: self.container.clone(),
        })
    }

    async fn prepare(&self, _account: &Account, credential: Credential) {
        self.prepared.borrow_mut().push(credential);
    }
}

pub struct FakeStore {
    pk_path: String,
    container: Rc<RefCell<FakeContainer>>,
}

impl DataStore for FakeStore {
    fn partition_key_path(&self) -> &str {
        &self.pk_path
    }

    /// Records the SQL and returns every document, since the fake cannot run SQL,
    /// in pages of the container's page size.
    async fn documents(&self, sql: &str) -> anyhow::Result<Documents> {
        let mut container = self.container.borrow_mut();
        container.queries.push(sql.to_string());
        let size = container.page_size.max(1);
        let charge = container.page_charge;
        let pages: Vec<anyhow::Result<Page>> = container
            .docs
            .chunks(size)
            .map(|docs| {
                Ok(Page {
                    docs: docs.to_vec(),
                    request_charge: charge,
                })
            })
            .collect();
        Ok(futures::stream::iter(pages).boxed_local())
    }

    async fn delete(&self, id: &str, partition_key: Option<&Value>) -> anyhow::Result<()> {
        let mut container = self.container.borrow_mut();
        container
            .deletes
            .push((id.to_string(), partition_key.cloned()));
        let before = container.docs.len();
        container.docs.retain(|doc| doc["id"] != id);
        anyhow::ensure!(container.docs.len() < before, "document {id} not found");
        Ok(())
    }
}
