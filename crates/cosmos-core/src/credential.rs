//! Entra ID token caching.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use azure_core::credentials::{AccessToken, TokenCredential, TokenRequestOptions};
use azure_core::time::{Duration, OffsetDateTime};

/// Scope of Azure Resource Manager tokens.
pub const MANAGEMENT_SCOPE: &str = "https://management.azure.com/.default";

/// Scope the Cosmos DB SDK requests data plane tokens for.
pub const COSMOS_SCOPE: &str = "https://cosmos.azure.com/.default";

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

    const ARM: &str = MANAGEMENT_SCOPE;
    const COSMOS: &str = COSMOS_SCOPE;

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
