use clap::{Args, Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(
    name = "cosmoscli",
    version,
    about = "Command line tool for Azure Cosmos DB"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Args, Debug, Clone, PartialEq, Default)]
pub struct GlobalArgs {
    /// Only use this subscription id instead of every subscription you can access
    #[arg(long, global = true)]
    pub subscription: Option<String>,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum Command {
    /// Manage Cosmos DB accounts
    Accounts {
        #[command(subcommand)]
        command: AccountsCommand,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum AccountsCommand {
    /// List Cosmos DB accounts
    List,
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(std::iter::once("cosmoscli").chain(args.iter().copied())).unwrap()
    }

    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_accounts_list_with_subscription() {
        let cli = parse(&["accounts", "list", "--subscription", "sub-1"]);

        assert_eq!(
            cli.command,
            Command::Accounts {
                command: AccountsCommand::List
            }
        );
        assert_eq!(cli.global.subscription.as_deref(), Some("sub-1"));
    }
}
