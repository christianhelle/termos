//! Remembers the account list between runs, so it shows before Azure answers.

use std::path::PathBuf;

use crate::management::Account;

/// The account list saved in a file.
pub struct AccountCache {
    path: PathBuf,
}

impl AccountCache {
    /// The cache in the user's cache folder, such as `%LOCALAPPDATA%smoscli` on Windows.
    pub fn for_user(subscription: Option<&str>) -> Option<Self> {
        let dir = dirs::cache_dir()?.join("cosmoscli");
        Some(Self::in_dir(dir, subscription))
    }

    /// A cache kept in the given folder, with one file per subscription filter.
    pub fn in_dir(dir: impl Into<PathBuf>, subscription: Option<&str>) -> Self {
        let name = match subscription {
            // Kept to characters that are safe in a file name on every platform
            Some(id) => {
                let id: String = id
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                format!("accounts-{id}.json")
            }
            None => "accounts.json".to_string(),
        };
        AccountCache {
            path: dir.into().join(name),
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

    #[test]
    fn has_nothing_before_the_first_save() {
        let dir = tempfile::tempdir().unwrap();

        assert_eq!(AccountCache::in_dir(dir.path(), None).load(), None);
    }

    #[test]
    fn ignores_a_file_it_cannot_read() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("accounts.json"), "not json").unwrap();

        assert_eq!(AccountCache::in_dir(dir.path(), None).load(), None);
    }

    #[test]
    fn keeps_a_separate_list_per_subscription() {
        let dir = tempfile::tempdir().unwrap();
        let every = AccountCache::in_dir(dir.path(), None);
        let one = AccountCache::in_dir(dir.path(), Some("sub-2"));
        every
            .save(&[account("orders"), account("inventory")])
            .unwrap();

        one.save(&[account("inventory")]).unwrap();

        assert_eq!(every.load().unwrap().len(), 2);
        assert_eq!(one.load(), Some(vec![account("inventory")]));
    }

    #[test]
    fn creates_its_folder_on_first_save() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AccountCache::in_dir(dir.path().join("cosmoscli"), None);

        cache.save(&[account("orders")]).unwrap();

        assert_eq!(cache.load(), Some(vec![account("orders")]));
    }
}
