//! Entra ID token caching.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
use azure_core::time::{Duration, OffsetDateTime};

use crate::cli::{AuthMode, Command, GlobalArgs};

/// Scope of Azure Resource Manager tokens.
pub const MANAGEMENT_SCOPE: &str = "https://management.azure.com/.default";

/// Scope the Cosmos DB SDK requests data plane tokens for.
pub const COSMOS_SCOPE: &str = "https://cosmos.azure.com/.default";

/// The token scopes a run will need, so they can be fetched ahead of time.
///
/// `command` is `None` for interactive mode, where any command may follow.
pub fn scopes_needed(command: Option<&Command>, global: &GlobalArgs) -> Vec<&'static str> {
    let touches_documents = matches!(
        command,
        None | Some(Command::Query { .. } | Command::Items { .. })
    );
    let documents_use_entra = global.key.is_none() && global.auth != AuthMode::Key;
    if touches_documents && documents_use_entra {
        vec![MANAGEMENT_SCOPE, COSMOS_SCOPE]
    } else {
        vec![MANAGEMENT_SCOPE]
    }
}

/// How long before expiry a cached token is replaced, so it never expires mid-request.
const REFRESH_MARGIN: Duration = Duration::minutes(5);

/// The token for one set of scopes. Its lock makes concurrent callers wait for a single fetch.
type Slot = Arc<tokio::sync::Mutex<Option<AccessToken>>>;

/// Keeps Entra ID tokens until shortly before they expire.
///
/// The Azure CLI credential runs `az` for every token, and the Cosmos DB SDK
/// asks for a token on every request, so without this each request waits for `az`.
#[derive(Debug)]
pub struct CachedCredential {
    inner: Arc<dyn TokenCredential>,
    slots: Mutex<HashMap<String, Slot>>,
}

impl CachedCredential {
    pub fn new(inner: Arc<dyn TokenCredential>) -> Self {
        CachedCredential {
            inner,
            slots: Mutex::default(),
        }
    }

    fn slot(&self, scopes: &[&str]) -> Slot {
        let mut slots = self.slots.lock().expect("token slots lock poisoned");
        slots.entry(scopes.join(" ")).or_default().clone()
    }
}

#[async_trait::async_trait]
impl TokenCredential for CachedCredential {
    async fn get_token(
        &self,
        scopes: &[&str],
        options: Option<TokenRequestOptions<'_>>,
    ) -> azure_core::Result<AccessToken> {
        let slot = self.slot(scopes);
        let mut cached = slot.lock().await;
        if let Some(token) = cached.as_ref()
            && token.expires_on - REFRESH_MARGIN > OffsetDateTime::now_utc()
        {
            return Ok(token.clone());
        }
        let token = self.inner.get_token(scopes, options).await?;
        *cached = Some(token.clone());
        Ok(token)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::cli::{AccountsCommand, Cli};
    use clap::Parser;

    const ARM: &str = MANAGEMENT_SCOPE;
    const COSMOS: &str = COSMOS_SCOPE;

    fn scopes_for(args: &[&str]) -> Vec<&'static str> {
        let cli =
            Cli::try_parse_from(std::iter::once("cosmoscli").chain(args.iter().copied())).unwrap();
        scopes_needed(cli.command.as_ref(), &cli.global)
    }

    #[test]
    fn interactive_mode_needs_both_scopes_unless_documents_use_a_key() {
        assert_eq!(scopes_for(&[]), vec![ARM, COSMOS]);
        assert_eq!(scopes_for(&["--auth", "entra"]), vec![ARM, COSMOS]);
        assert_eq!(scopes_for(&["--auth", "key"]), vec![ARM]);
        assert_eq!(scopes_for(&["--key", "secret=="]), vec![ARM]);
    }

    #[test]
    fn document_commands_need_a_cosmos_token_unless_they_use_a_key() {
        let query = ["query", "-a", "x", "-d", "y", "-c", "z"];
        assert_eq!(scopes_for(&query), vec![ARM, COSMOS]);
        assert_eq!(
            scopes_for(&[
                "items", "get", "-a", "x", "-d", "y", "-c", "z", "--id", "1", "--pk", "p"
            ]),
            vec![ARM, COSMOS]
        );
        assert_eq!(
            scopes_for(&[&query[..], &["--auth", "key"]].concat()),
            vec![ARM]
        );
    }

    #[test]
    fn management_commands_only_need_resource_manager() {
        assert_eq!(scopes_for(&["accounts", "list"]), vec![ARM]);
        assert_eq!(scopes_for(&["containers", "list", "-a", "x"]), vec![ARM]);
        let global = GlobalArgs {
            auth: AuthMode::Entra,
            ..Default::default()
        };
        let accounts = Command::Accounts {
            command: AccountsCommand::List,
        };
        assert_eq!(scopes_needed(Some(&accounts), &global), vec![ARM]);
    }

    /// Hands out numbered tokens that expire after a fixed lifetime and counts requests.
    #[derive(Debug)]
    struct CountingCredential {
        lifetime: Duration,
        calls: AtomicUsize,
    }

    impl CountingCredential {
        fn lasting(lifetime: Duration) -> Arc<Self> {
            Arc::new(CountingCredential {
                lifetime,
                calls: AtomicUsize::new(0),
            })
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl TokenCredential for CountingCredential {
        async fn get_token(
            &self,
            scopes: &[&str],
            _options: Option<TokenRequestOptions<'_>>,
        ) -> azure_core::Result<AccessToken> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst) + 1;
            // Like `az`, let other callers run while the token is being fetched
            tokio::task::yield_now().await;
            let token = format!("{} #{call}", scopes.join(" "));
            Ok(AccessToken::new(
                token,
                OffsetDateTime::now_utc() + self.lifetime,
            ))
        }
    }

    async fn token(credential: &CachedCredential, scope: &str) -> String {
        let token = credential.get_token(&[scope], None).await.unwrap();
        token.token.secret().to_string()
    }

    #[tokio::test]
    async fn reuses_a_token_until_it_is_about_to_expire() {
        let inner = CountingCredential::lasting(Duration::hours(1));
        let credential = CachedCredential::new(inner.clone());

        assert_eq!(token(&credential, ARM).await, format!("{ARM} #1"));
        assert_eq!(token(&credential, ARM).await, format!("{ARM} #1"));
        assert_eq!(inner.calls(), 1);
    }

    #[tokio::test]
    async fn fetches_a_new_token_when_the_cached_one_expires_soon() {
        let inner = CountingCredential::lasting(Duration::minutes(2));
        let credential = CachedCredential::new(inner.clone());

        token(&credential, ARM).await;
        assert_eq!(token(&credential, ARM).await, format!("{ARM} #2"));
        assert_eq!(inner.calls(), 2);
    }

    #[tokio::test]
    async fn keeps_a_token_per_scope() {
        let inner = CountingCredential::lasting(Duration::hours(1));
        let credential = CachedCredential::new(inner.clone());

        token(&credential, ARM).await;
        token(&credential, COSMOS).await;

        assert_eq!(token(&credential, ARM).await, format!("{ARM} #1"));
        assert_eq!(token(&credential, COSMOS).await, format!("{COSMOS} #2"));
        assert_eq!(inner.calls(), 2);
    }

    #[tokio::test]
    async fn concurrent_requests_for_a_scope_share_one_fetch() {
        let inner = CountingCredential::lasting(Duration::hours(1));
        let credential = CachedCredential::new(inner.clone());

        let (first, second) = tokio::join!(token(&credential, ARM), token(&credential, ARM));

        assert_eq!(first, second);
        assert_eq!(inner.calls(), 1);
    }
}
