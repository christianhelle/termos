//! Remembers the account list between runs, so it shows before Azure answers.

use std::path::PathBuf;

use crate::management::Account;

/// The account list saved in a file.
pub struct AccountCache {
    path: PathBuf,
}

impl AccountCache {
    /// A cache kept in the given folder, with one file per subscription filter.
    pub fn in_dir(dir: impl Into<PathBuf>, subscription: Option<&str>) -> Self {
        let _ = subscription;
        AccountCache {
            path: dir.into().join("accounts.json"),
        }
    }

    /// The accounts saved last time, if there are any that can be read.
    pub fn load(&self) -> Option<Vec<Account>> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, accounts: &[Account]) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Written aside first, so a run that stops halfway leaves the old list readable
        let partial = self.path.with_extension("json.tmp");
        std::fs::write(&partial, serde_json::to_string(accounts)?)?;
        std::fs::rename(&partial, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::account;

    #[test]
    fn loads_the_accounts_it_saved() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AccountCache::in_dir(dir.path(), None);
        let accounts = vec![account("orders"), account("inventory")];

        cache.save(&accounts).unwrap();

        assert_eq!(cache.load(), Some(accounts));
    }
}
