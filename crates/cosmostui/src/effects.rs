//! Runs the background work that updates ask for.

use cosmos_core::connector::Connector;
use cosmos_core::management::Management;
use cosmos_core::store::DataPlane;

use crate::state::{Effect, Msg};

/// Does the work, reporting how it went as a message for the state.
pub async fn run<M: Management, D: DataPlane>(connector: &Connector<M, D>, effect: Effect) -> Msg {
    match effect {
        Effect::LoadAccounts => {
            Msg::AccountsLoaded(connector.list_accounts().await.map_err(describe))
        }
        Effect::LoadContainers(account) => {
            // Set up the data plane client while the containers are listed
            let (result, ()) = futures::join!(
                connector.containers_of(&account, None),
                connector.prepare(&account)
            );
            Msg::ContainersLoaded {
                account: account.name,
                result: result.map_err(describe),
            }
        }
        Effect::Query { .. } => todo!(),
    }
}

/// An error with its causes, as one line for the status bar.
fn describe(error: anyhow::Error) -> String {
    format!("{error:#}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use cosmos_core::connector::Settings;
    use cosmos_core::store::Credential;
    use cosmos_core::testing::{FakeDataPlane, FakeManagement, account, container};

    type TestConnector = Connector<FakeManagement, FakeDataPlane>;

    fn connector() -> TestConnector {
        let management = FakeManagement::with_databases(
            account("orders"),
            &[("shop", &[container("carts", "/tenantId")])],
        );
        let data = FakeDataPlane::new("/tenantId", vec![]);
        Connector::new(management, data, Settings::default())
    }

    #[tokio::test]
    async fn loads_the_accounts() {
        let msg = run(&connector(), Effect::LoadAccounts).await;

        let Msg::AccountsLoaded(Ok(accounts)) = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(accounts, vec![account("orders")]);
    }

    #[tokio::test]
    async fn loads_containers_and_prepares_the_data_plane_meanwhile() {
        let connector = connector();

        let msg = run(&connector, Effect::LoadContainers(account("orders"))).await;

        let Msg::ContainersLoaded { account, result } = msg else {
            panic!("unexpected {msg:?}");
        };
        assert_eq!(account, "orders");
        assert_eq!(
            result.unwrap(),
            vec![("shop".to_string(), container("carts", "/tenantId"))]
        );
        assert_eq!(*connector.data.prepared.borrow(), vec![Credential::Entra]);
    }
}
