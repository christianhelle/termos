use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};

#[derive(Parser, Debug)]
#[command(
    name = "cosmoscli",
    version,
    about = "Command line tool for Azure Cosmos DB"
)]
pub struct Cli {
    #[command(flatten)]
    pub global: GlobalArgs,

    /// Runs interactive mode when omitted
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Args, Debug, Clone, PartialEq, Default)]
pub struct GlobalArgs {
    /// Only use this subscription id instead of every subscription you can access
    #[arg(long, global = true)]
    pub subscription: Option<String>,

    /// How to authenticate to the Cosmos DB data plane
    #[arg(long, global = true, value_enum, default_value_t = AuthMode::Auto)]
    pub auth: AuthMode,

    /// Account key to use instead of fetching one from Resource Manager
    #[arg(long, global = true)]
    pub key: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq, Default)]
pub enum AuthMode {
    /// Try Entra ID first and fall back to the account key
    #[default]
    Auto,
    /// Only use Entra ID
    Entra,
    /// Only use the account key
    Key,
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
    /// Query documents in a container
    Query {
        #[command(flatten)]
        target: ContainerRef,
        /// Cosmos DB SQL query
        #[arg(default_value = "SELECT * FROM c")]
        sql: String,
        /// Output format
        #[arg(short, long, value_enum, default_value_t = OutputFormat::Table)]
        output: OutputFormat,
        /// Stop after this many documents
        #[arg(long)]
        max: Option<usize>,
    },
    /// Read, write and delete documents in a container
    Items {
        #[command(subcommand)]
        command: ItemsCommand,
    },
}

#[derive(Subcommand, Debug, PartialEq)]
pub enum ItemsCommand {
    /// Read a single document
    Get {
        #[command(flatten)]
        target: ContainerRef,
        /// Document id
        #[arg(long)]
        id: String,
        #[command(flatten)]
        pk: PartitionKeyArg,
    },
    /// Create a document from a JSON file or stdin
    Create {
        #[command(flatten)]
        target: ContainerRef,
        /// JSON file to read, stdin when omitted
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    /// Create or replace a document from a JSON file or stdin
    Upsert {
        #[command(flatten)]
        target: ContainerRef,
        /// JSON file to read, stdin when omitted
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    /// Replace an existing document from a JSON file or stdin
    Replace {
        #[command(flatten)]
        target: ContainerRef,
        /// JSON file to read, stdin when omitted
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
    /// Delete a single document
    Delete {
        #[command(flatten)]
        target: ContainerRef,
        /// Document id
        #[arg(long)]
        id: String,
        #[command(flatten)]
        pk: PartitionKeyArg,
    },
    /// Delete every document with the given partition key
    DeletePartition {
        #[command(flatten)]
        target: ContainerRef,
        #[command(flatten)]
        pk: PartitionKeyArg,
        /// Skip the confirmation prompt
        #[arg(short, long)]
        yes: bool,
    },
}

/// A partition key value given as a string or as raw JSON.
#[derive(Args, Debug, Clone, PartialEq)]
#[group(required = true, multiple = false)]
pub struct PartitionKeyArg {
    /// Partition key value as a string
    #[arg(long)]
    pub pk: Option<String>,
    /// Partition key value as JSON, for numbers, booleans or null
    #[arg(long)]
    pub pk_json: Option<String>,
}

#[derive(ValueEnum, Debug, Clone, Copy, PartialEq)]
pub enum OutputFormat {
    /// Table with the id and partition key of each document
    Table,
    /// Full JSON documents
    Json,
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

impl PartitionKeyArg {
    pub fn value(&self) -> anyhow::Result<serde_json::Value> {
        match (&self.pk, &self.pk_json) {
            (Some(pk), _) => Ok(serde_json::Value::String(pk.clone())),
            (None, Some(json)) => Ok(serde_json::from_str(json)?),
            (None, None) => anyhow::bail!("a partition key is required, use --pk or --pk-json"),
        }
    }
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
    fn no_arguments_means_no_command() {
        assert_eq!(parse(&[]).command, None);
    }

    #[test]
    fn parses_accounts_list_with_subscription() {
        let cli = parse(&["accounts", "list", "--subscription", "sub-1"]);

        assert_eq!(
            cli.command,
            Some(Command::Accounts {
                command: AccountsCommand::List
            })
        );
        assert_eq!(cli.global.subscription.as_deref(), Some("sub-1"));
    }

    #[test]
    fn parses_databases_list() {
        let cli = parse(&["databases", "list", "-a", "shop-acct"]);

        assert_eq!(
            cli.command,
            Some(Command::Databases {
                command: DatabasesCommand::List {
                    account: "shop-acct".into()
                }
            })
        );
    }

    #[test]
    fn parses_containers_list_with_optional_database() {
        let cli = parse(&["containers", "list", "--account", "shop-acct", "-d", "shop"]);

        assert_eq!(
            cli.command,
            Some(Command::Containers {
                command: ContainersCommand::List {
                    account: "shop-acct".into(),
                    database: Some("shop".into()),
                }
            })
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
            Some(Command::Containers {
                command: ContainersCommand::Create {
                    target: target(),
                    partition_key: "/tenantId".into(),
                    throughput: Some(400),
                }
            })
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
            Some(Command::Containers {
                command: ContainersCommand::Show { target: target() }
            })
        );
        assert_eq!(
            delete.command,
            Some(Command::Containers {
                command: ContainersCommand::Delete {
                    target: target(),
                    yes: true
                }
            })
        );
    }

    #[test]
    fn query_defaults_to_select_all_as_table() {
        let cli = parse(&["query", "-a", "shop-acct", "-d", "shop", "-c", "orders"]);

        assert_eq!(
            cli.command,
            Some(Command::Query {
                target: target(),
                sql: "SELECT * FROM c".into(),
                output: OutputFormat::Table,
                max: None,
            })
        );
    }

    #[test]
    fn query_accepts_sql_json_output_and_max() {
        let cli = parse(&[
            "query",
            "-a",
            "shop-acct",
            "-d",
            "shop",
            "-c",
            "orders",
            "SELECT * FROM c WHERE c.total > 10",
            "-o",
            "json",
            "--max",
            "5",
        ]);

        assert_eq!(
            cli.command,
            Some(Command::Query {
                target: target(),
                sql: "SELECT * FROM c WHERE c.total > 10".into(),
                output: OutputFormat::Json,
                max: Some(5),
            })
        );
    }

    fn items(args: &[&str]) -> ItemsCommand {
        let target = ["-a", "shop-acct", "-d", "shop", "-c", "orders"];
        let all: Vec<&str> = std::iter::once("items")
            .chain(args.iter().copied())
            .chain(target)
            .collect();
        match parse(&all).command {
            Some(Command::Items { command }) => command,
            other => panic!("expected items command, got {other:?}"),
        }
    }

    fn string_pk(value: &str) -> PartitionKeyArg {
        PartitionKeyArg {
            pk: Some(value.into()),
            pk_json: None,
        }
    }

    #[test]
    fn parses_items_get_and_delete() {
        assert_eq!(
            items(&["get", "--id", "o-1", "--pk", "contoso"]),
            ItemsCommand::Get {
                target: target(),
                id: "o-1".into(),
                pk: string_pk("contoso"),
            }
        );
        assert_eq!(
            items(&["delete", "--id", "o-1", "--pk", "contoso"]),
            ItemsCommand::Delete {
                target: target(),
                id: "o-1".into(),
                pk: string_pk("contoso"),
            }
        );
    }

    #[test]
    fn parses_item_writes_from_file_or_stdin() {
        assert_eq!(
            items(&["create", "--file", "order.json"]),
            ItemsCommand::Create {
                target: target(),
                file: Some("order.json".into()),
            }
        );
        assert_eq!(
            items(&["upsert"]),
            ItemsCommand::Upsert {
                target: target(),
                file: None,
            }
        );
        assert_eq!(
            items(&["replace", "-f", "order.json"]),
            ItemsCommand::Replace {
                target: target(),
                file: Some("order.json".into()),
            }
        );
    }

    #[test]
    fn parses_delete_partition_with_yes() {
        assert_eq!(
            items(&["delete-partition", "--pk", "contoso", "--yes"]),
            ItemsCommand::DeletePartition {
                target: target(),
                pk: string_pk("contoso"),
                yes: true,
            }
        );
    }

    #[test]
    fn partition_key_is_required() {
        let args = [
            "cosmoscli",
            "items",
            "delete-partition",
            "-a",
            "x",
            "-d",
            "y",
            "-c",
            "z",
        ];
        assert!(Cli::try_parse_from(args).is_err());
    }

    #[test]
    fn string_partition_key_value_stays_a_string() {
        assert_eq!(string_pk("42").value().unwrap(), serde_json::json!("42"));
    }

    #[test]
    fn json_partition_key_value_keeps_its_type() {
        let pk = PartitionKeyArg {
            pk: None,
            pk_json: Some("42".into()),
        };
        assert_eq!(pk.value().unwrap(), serde_json::json!(42));
    }

    #[test]
    fn auth_defaults_to_auto() {
        let cli = parse(&["accounts", "list"]);

        assert_eq!(cli.global.auth, AuthMode::Auto);
        assert_eq!(cli.global.key, None);
    }

    #[test]
    fn parses_key_auth_after_the_subcommand() {
        let cli = parse(&[
            "databases",
            "list",
            "-a",
            "x",
            "--auth",
            "key",
            "--key",
            "secret==",
        ]);

        assert_eq!(cli.global.auth, AuthMode::Key);
        assert_eq!(cli.global.key.as_deref(), Some("secret=="));
    }
}
