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
    /// Manage SQL databases on an account
    Databases {
        #[command(subcommand)]
        command: DatabasesCommand,
    },
    /// Manage containers on an account
    Containers {
        #[command(subcommand)]
        command: ContainersCommand,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum AccountsCommand {
    /// List Cosmos DB accounts
    List,
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum DatabasesCommand {
    /// List SQL databases on an account
    List {
        /// Cosmos DB account name
        #[arg(short, long)]
        account: String,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum ContainersCommand {
    /// List containers on an account
    List {
        /// Cosmos DB account name
        #[arg(short, long)]
        account: String,
        /// Only list containers in this database
        #[arg(short, long)]
        database: Option<String>,
    },
    /// Show a container and its partition key
    Show {
        #[command(flatten)]
        target: ContainerRef,
    },
    /// Create a container
    Create {
        #[command(flatten)]
        target: ContainerRef,
        /// Partition key path, for example /tenantId
        #[arg(long)]
        partition_key: String,
        /// Provisioned throughput in RU/s
        #[arg(long)]
        throughput: Option<u32>,
    },
    /// Delete a container and all of its documents
    Delete {
        #[command(flatten)]
        target: ContainerRef,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

/// Identifies a container on a Cosmos DB account.
#[derive(Args, Debug, Clone, PartialEq)]
pub struct ContainerRef {
    /// Cosmos DB account name
    #[arg(short, long)]
    pub account: String,
    /// Database name
    #[arg(short, long)]
    pub database: String,
    /// Container name
    #[arg(short, long)]
    pub container: String,
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

    #[test]
    fn parses_databases_list() {
        let cli = parse(&["databases", "list", "-a", "shop-acct"]);

        assert_eq!(
            cli.command,
            Command::Databases {
                command: DatabasesCommand::List {
                    account: "shop-acct".into()
                }
            }
        );
    }

    #[test]
    fn parses_containers_list_with_optional_database() {
        let cli = parse(&["containers", "list", "--account", "shop-acct", "-d", "shop"]);

        assert_eq!(
            cli.command,
            Command::Containers {
                command: ContainersCommand::List {
                    account: "shop-acct".into(),
                    database: Some("shop".into()),
                }
            }
        );
    }

    fn target() -> ContainerRef {
        ContainerRef {
            account: "shop-acct".into(),
            database: "shop".into(),
            container: "orders".into(),
        }
    }

    #[test]
    fn parses_containers_create() {
        let cli = parse(&[
            "containers",
            "create",
            "-a",
            "shop-acct",
            "-d",
            "shop",
            "-c",
            "orders",
            "--partition-key",
            "/tenantId",
            "--throughput",
            "400",
        ]);

        assert_eq!(
            cli.command,
            Command::Containers {
                command: ContainersCommand::Create {
                    target: target(),
                    partition_key: "/tenantId".into(),
                    throughput: Some(400),
                }
            }
        );
    }

    #[test]
    fn parses_containers_show_and_delete() {
        let show = parse(&[
            "containers",
            "show",
            "-a",
            "shop-acct",
            "-d",
            "shop",
            "-c",
            "orders",
        ]);
        let delete = parse(&[
            "containers",
            "delete",
            "-a",
            "shop-acct",
            "-d",
            "shop",
            "-c",
            "orders",
            "--yes",
        ]);

        assert_eq!(
            show.command,
            Command::Containers {
                command: ContainersCommand::Show { target: target() }
            }
        );
        assert_eq!(
            delete.command,
            Command::Containers {
                command: ContainersCommand::Delete {
                    target: target(),
                    yes: true
                }
            }
        );
    }
}
