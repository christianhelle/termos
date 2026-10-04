//! Remembers things between runs, such as the account list, so they show before Azure answers.

use std::marker::PhantomData;
use std::path::PathBuf;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::management::Account;
use crate::session::{Snapshot, VERSION};

/// The folder termos keeps its caches in, such as `%LOCALAPPDATA%\termos` on Windows.
fn user_dir() -> Option<PathBuf> {
    Some(dirs::cache_dir()?.join("termos"))
}

/// The account list saved in a file.
pub type AccountCache = JsonCache<Vec<Account>>;

/// A value saved in a JSON file, with one file per subscription filter.
pub struct JsonCache<T> {
    path: PathBuf,
    value: PhantomData<T>,
}

impl AccountCache {
    /// The cache in the user's cache folder.
    pub fn for_user(subscription: Option<&str>) -> Option<Self> {
        Some(Self::in_dir(user_dir()?, subscription))
    }

    /// A cache kept in the given folder, with one file per subscription filter.
    pub fn in_dir(dir: impl Into<PathBuf>, subscription: Option<&str>) -> Self {
        Self::in_dir_named(dir, "accounts", subscription)
    }
}

/// The screen as termos last left it, saved in a file.
pub struct SessionCache(JsonCache<Snapshot>);

impl SessionCache {
    /// The session in the user's cache folder, next to the account list.
    pub fn for_user(subscription: Option<&str>) -> Option<Self> {
        Some(Self::in_dir(user_dir()?, subscription))
    }

    /// A session kept in the given folder, with one file per subscription filter.
    pub fn in_dir(dir: impl Into<PathBuf>, subscription: Option<&str>) -> Self {
        SessionCache(JsonCache::in_dir_named(dir, "session", subscription))
    }

    /// The session saved last time, unless another version of termos saved it.
    pub fn load(&self) -> Option<Snapshot> {
        self.0.load().filter(|snapshot| snapshot.version == VERSION)
    }

    pub fn save(&self, snapshot: &Snapshot) -> anyhow::Result<()> {
        self.0.save(snapshot)
    }
}

impl<T: Serialize + DeserializeOwned> JsonCache<T> {
    /// The cache called `name` in the given folder.
    pub fn in_dir_named(dir: impl Into<PathBuf>, name: &str, subscription: Option<&str>) -> Self {
        let file = match subscription {
            // Kept to characters that are safe in a file name on every platform
            Some(id) => {
                let id: String = id
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                    .collect();
                format!("{name}-{id}.json")
            }
            None => format!("{name}.json"),
        };
        JsonCache {
            path: dir.into().join(file),
            value: PhantomData,
        }
    }

    /// The value saved last time, if there is one that can be read.
    pub fn load(&self) -> Option<T> {
        let text = std::fs::read_to_string(&self.path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, value: &T) -> anyhow::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Written aside first, so a run that stops halfway leaves the old value readable
        let partial = self.path.with_extension("json.tmp");
        std::fs::write(&partial, serde_json::to_string(value)?)?;
        std::fs::rename(&partial, &self.path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{account, snapshot};

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
            .save(&vec![account("orders"), account("inventory")])
            .unwrap();

        one.save(&vec![account("inventory")]).unwrap();

        assert_eq!(every.load().unwrap().len(), 2);
        assert_eq!(one.load(), Some(vec![account("inventory")]));
    }

    #[test]
    fn creates_its_folder_on_first_save() {
        let dir = tempfile::tempdir().unwrap();
        let cache = AccountCache::in_dir(dir.path().join("termos"), None);

        cache.save(&vec![account("orders")]).unwrap();

        assert_eq!(cache.load(), Some(vec![account("orders")]));
    }

    #[test]
    fn loads_the_session_it_saved() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SessionCache::in_dir(dir.path(), None);

        cache.save(&snapshot()).unwrap();

        assert_eq!(cache.load(), Some(snapshot()));
    }

    #[test]
    fn ignores_a_session_saved_by_another_version() {
        let dir = tempfile::tempdir().unwrap();
        let cache = SessionCache::in_dir(dir.path(), None);
        let older = Snapshot {
            version: VERSION - 1,
            ..snapshot()
        };

        cache.save(&older).unwrap();

        assert_eq!(cache.load(), None);
    }

    #[test]
    fn keeps_the_session_apart_from_the_accounts() {
        let dir = tempfile::tempdir().unwrap();
        let accounts = AccountCache::in_dir(dir.path(), Some("sub-2"));
        let session = SessionCache::in_dir(dir.path(), Some("sub-2"));

        accounts.save(&vec![account("orders")]).unwrap();
        session.save(&snapshot()).unwrap();

        assert_eq!(accounts.load(), Some(vec![account("orders")]));
        assert_eq!(session.load(), Some(snapshot()));
    }
}
