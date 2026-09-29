//! In-memory fakes for testing command handlers.

use std::cell::RefCell;

use crate::management::{Account, Container, Management};

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
}
