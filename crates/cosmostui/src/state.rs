//! What the screen shows, and how keys and finished work change it.

use cosmos_core::management::Account;

/// Work for the runtime to do in the background.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    LoadAccounts,
    LoadContainers(Account),
}

/// Everything the screen shows.
pub struct AppState {}

impl AppState {
    /// The state on startup, with the work to start right away.
    pub fn new() -> (Self, Vec<Effect>) {
        (AppState {}, vec![Effect::LoadAccounts])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_by_loading_accounts() {
        let (_, effects) = AppState::new();

        assert_eq!(effects, vec![Effect::LoadAccounts]);
    }
}
