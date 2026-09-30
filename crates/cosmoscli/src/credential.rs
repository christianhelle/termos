//! Which Entra ID tokens a run needs.

use cosmos_core::credential::{COSMOS_SCOPE, MANAGEMENT_SCOPE};

use crate::cli::{AuthMode, Command, GlobalArgs};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
