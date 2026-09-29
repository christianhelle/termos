//! In-memory fakes for testing command handlers.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use serde_json::Value;

use crate::management::{Account, Container, Management};
use crate::partition::value_at_path;
use crate::prompt::{Confirm, LineReader, Picker};
use crate::store::{Credential, DataPlane, DataStore, Unauthorized};

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
    pub databases: RefCell<Vec<(String, Vec<Container>)>>,
}

impl FakeManagement {
    pub fn with_databases(account: Account, databases: &[(&str, &[Container])]) -> Self {
        FakeManagement {
            accounts: vec![account],
            databases: RefCell::new(
                databases
                    .iter()
                    .map(|(name, containers)| (name.to_string(), containers.to_vec()))
                    .collect(),
            ),
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
            .borrow()
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
            .borrow()
            .iter()
            .find(|(name, _)| name == database)
            .map(|(_, containers)| containers.clone())
            .unwrap_or_default())
    }

    async fn get_container(
        &self,
        account: &Account,
        database: &str,
        name: &str,
    ) -> anyhow::Result<Container> {
        self.list_containers(account, database)
            .await?
            .into_iter()
            .find(|c| c.name == name)
            .ok_or_else(|| anyhow::anyhow!("container '{database}/{name}' not found"))
    }

    async fn create_container(
        &self,
        _account: &Account,
        database: &str,
        name: &str,
        partition_key_path: &str,
        _throughput: Option<u32>,
    ) -> anyhow::Result<()> {
        let mut databases = self.databases.borrow_mut();
        let (_, containers) = databases
            .iter_mut()
            .find(|(db, _)| db == database)
            .ok_or_else(|| anyhow::anyhow!("database '{database}' not found"))?;
        containers.push(container(name, partition_key_path));
        Ok(())
    }

    async fn delete_container(
        &self,
        _account: &Account,
        database: &str,
        name: &str,
    ) -> anyhow::Result<()> {
        for (db, containers) in self.databases.borrow_mut().iter_mut() {
            if db == database {
                containers.retain(|c| c.name != name);
            }
        }
        Ok(())
    }

    async fn primary_key(&self, _account: &Account) -> anyhow::Result<String> {
        Ok("primary==".into())
    }
}

/// Shared state behind a fake container.
#[derive(Default)]
pub struct FakeContainer {
    pub docs: Vec<Value>,
    pub queries: Vec<String>,
}

pub struct FakeDataPlane {
    pub pk_path: String,
    /// When false, Entra ID connections are rejected as unauthorized.
    pub entra_allowed: bool,
    pub container: Rc<RefCell<FakeContainer>>,
    pub connections: RefCell<Vec<Credential>>,
}

impl FakeDataPlane {
    pub fn new(pk_path: &str, docs: Vec<Value>) -> Self {
        FakeDataPlane {
            pk_path: pk_path.into(),
            entra_allowed: true,
            container: Rc::new(RefCell::new(FakeContainer {
                docs,
                ..Default::default()
            })),
            connections: RefCell::default(),
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
}

pub struct FakeStore {
    pk_path: String,
    container: Rc<RefCell<FakeContainer>>,
}

impl DataStore for FakeStore {
    fn partition_key_path(&self) -> &str {
        &self.pk_path
    }

    /// Records the SQL and returns every document, since the fake cannot run SQL.
    async fn query(&self, sql: &str, max: Option<usize>) -> anyhow::Result<Vec<Value>> {
        let mut container = self.container.borrow_mut();
        container.queries.push(sql.to_string());
        let limit = max.unwrap_or(usize::MAX);
        Ok(container.docs.iter().take(limit).cloned().collect())
    }

    async fn read_item(&self, id: &str, pk: &Value) -> anyhow::Result<Value> {
        let container = self.container.borrow();
        let doc = container.docs.iter().find(|doc| self.matches(doc, id, pk));
        doc.cloned()
            .ok_or_else(|| anyhow::anyhow!("document '{id}' not found"))
    }

    async fn delete_item(&self, id: &str, pk: &Value) -> anyhow::Result<()> {
        let mut container = self.container.borrow_mut();
        let before = container.docs.len();
        container.docs.retain(|doc| !self.matches(doc, id, pk));
        anyhow::ensure!(container.docs.len() < before, "document '{id}' not found");
        Ok(())
    }

    async fn create_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()> {
        let id = doc["id"].as_str().unwrap_or_default().to_string();
        anyhow::ensure!(
            self.read_item(&id, pk).await.is_err(),
            "document '{id}' already exists"
        );
        self.container.borrow_mut().docs.push(doc);
        Ok(())
    }

    async fn upsert_item(&self, pk: &Value, doc: Value) -> anyhow::Result<()> {
        let id = doc["id"].as_str().unwrap_or_default().to_string();
        let _ = self.delete_item(&id, pk).await;
        self.container.borrow_mut().docs.push(doc);
        Ok(())
    }

    async fn replace_item(&self, id: &str, pk: &Value, doc: Value) -> anyhow::Result<()> {
        self.delete_item(id, pk).await?;
        self.container.borrow_mut().docs.push(doc);
        Ok(())
    }

    async fn ids_in_partition(&self, pk: &Value) -> anyhow::Result<Vec<String>> {
        let container = self.container.borrow();
        Ok(container
            .docs
            .iter()
            .filter(|doc| value_at_path(doc, &self.pk_path) == Some(pk))
            .filter_map(|doc| doc["id"].as_str().map(String::from))
            .collect())
    }
}

impl FakeStore {
    fn matches(&self, doc: &Value, id: &str, pk: &Value) -> bool {
        doc.get("id").and_then(Value::as_str) == Some(id)
            && value_at_path(doc, &self.pk_path) == Some(pk)
    }
}

/// Answers every confirmation the same way and records what was asked.
#[derive(Clone, Default)]
pub struct ScriptedConfirm {
    pub answer: bool,
    pub asked: Rc<RefCell<Vec<String>>>,
}

impl ScriptedConfirm {
    pub fn answering(answer: bool) -> Self {
        ScriptedConfirm {
            answer,
            ..Default::default()
        }
    }
}

impl Confirm for ScriptedConfirm {
    fn confirm(&mut self, message: &str) -> anyhow::Result<bool> {
        self.asked.borrow_mut().push(message.to_string());
        Ok(self.answer)
    }
}

/// Hands out scripted lines, then reports the end of input.
#[derive(Clone, Default)]
pub struct ScriptedLines {
    pub lines: Rc<RefCell<VecDeque<String>>>,
}

impl ScriptedLines {
    pub fn new(lines: &[&str]) -> Self {
        ScriptedLines {
            lines: Rc::new(RefCell::new(lines.iter().map(|l| l.to_string()).collect())),
        }
    }
}

impl LineReader for ScriptedLines {
    fn read_line(&mut self, _prompt: &str) -> anyhow::Result<Option<String>> {
        Ok(self.lines.borrow_mut().pop_front())
    }
}

/// Gives scripted answers to pickers and records the items each one showed.
#[derive(Clone, Default)]
pub struct ScriptedPicker {
    pub answers: Rc<RefCell<VecDeque<Option<usize>>>>,
    pub shown: Rc<RefCell<Vec<Vec<String>>>>,
}

impl ScriptedPicker {
    pub fn answering(answers: &[Option<usize>]) -> Self {
        ScriptedPicker {
            answers: Rc::new(RefCell::new(answers.iter().copied().collect())),
            ..Default::default()
        }
    }
}

impl Picker for ScriptedPicker {
    fn pick(&mut self, _prompt: &str, items: &[String]) -> anyhow::Result<Option<usize>> {
        self.shown.borrow_mut().push(items.to_vec());
        Ok(self.answers.borrow_mut().pop_front().flatten())
    }
}
