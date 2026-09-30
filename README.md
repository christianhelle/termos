# cosmoscli

A command line tool for Azure Cosmos DB (NoSQL API). It can:
- list accounts, databases and containers
- query containers
- read, write and delete documents
- delete everything under a partition key
- create and delete containers

It comes with `cosmostui`, a terminal UI for browsing accounts and documents.

## Requirements

- Rust 1.88 or later
- The [Azure CLI](https://learn.microsoft.com/cli/azure/), logged in with `az login`

## Install

```sh
cargo install --path crates/cosmoscli
```

## Authentication

Accounts, databases and containers are managed through Azure Resource Manager, using your `az login` identity.

Documents are read and written through the Cosmos DB data plane. `--auth` picks how to authenticate:

| `--auth` | Behaviour |
| --- | --- |
| `auto` (default) | Try Entra ID first. If you have no Cosmos DB data plane role, fall back to the account key fetched from Resource Manager. |
| `entra` | Only use Entra ID. |
| `key` | Only use the account key fetched from Resource Manager. |

Pass `--key <KEY>` to supply an account key yourself. Accounts are found by name across every subscription you can access; add `--subscription <ID>` to search just one.

## Usage

```sh
# Accounts, databases and containers
cosmoscli accounts list
cosmoscli databases list -a my-account
cosmoscli containers list -a my-account [-d shop]
cosmoscli containers show   -a my-account -d shop -c orders
cosmoscli containers create -a my-account -d shop -c orders --partition-key /tenantId [--throughput 400]
cosmoscli containers delete -a my-account -d shop -c orders [--yes]

# Query: an id + partition key table by default, or full JSON documents
cosmoscli query -a my-account -d shop -c orders
cosmoscli query -a my-account -d shop -c orders "SELECT * FROM c WHERE c.total > 10" -o json --max 20

# Documents. Writes read JSON from --file or stdin.
cosmoscli items get     -a my-account -d shop -c orders --id o-1 --pk contoso
cosmoscli items create  -a my-account -d shop -c orders --file order.json
cosmoscli items upsert  -a my-account -d shop -c orders < order.json
cosmoscli items replace -a my-account -d shop -c orders -f order.json
cosmoscli items delete  -a my-account -d shop -c orders --id o-1 --pk contoso

# Delete every document with a partition key (asks first)
cosmoscli items delete-partition -a my-account -d shop -c orders --pk contoso [--yes]
```

## Interactive mode

Run `cosmoscli` without a command to stay in the tool and run many queries:

```text
$ cosmoscli
Type /help for commands, /exit or Ctrl-C to leave.
> /accounts
Using account my-account
Completed in 812 ms
[my-account]> /containers
Using container shop/orders
Completed in 1.04 s
[my-account/shop/orders]> SELECT * FROM c WHERE c.total > 10
+-----+-----------+
| id  | /tenantId |
+=================+
| o-1 | contoso   |
+-----+-----------+
Completed in 143 ms
```

- `/accounts` and `/containers` show a list you can filter by typing. The choice becomes the current account and container. Pick an account first, then a container. Queries only run once both are picked.
- Anything that doesn't start with `/` is a query against the current container. `/output json` shows full JSON documents and `/output table` switches back.
- Every CLI command also works as a slash command, and `-a`, `-d` and `-c` default to the current account and container, for example `/items get --id o-1 --pk contoso` or `/containers show`. Document writes need `--file`, since stdin is the prompt.
- Each command and query reports how long it took. Time spent choosing from a list isn't counted.
- `/help` lists the commands. `/exit`, `/quit`, Ctrl-C or Ctrl-D leaves. The up and down arrows go through earlier lines.

Partition key values given with `--pk` are strings. Use `--pk-json` for numbers, booleans or null, for example `--pk-json 42`.

Deleting a partition or a container asks for confirmation. When stdin is not a terminal, the tool refuses unless you pass `--yes`.

## Terminal UI

`cosmostui` browses accounts and documents in three panes, like the Data Explorer in the Azure portal:

```sh
cargo install --path crates/cosmostui
cosmostui [--subscription <ID>] [--auth auto|entra|key] [--key <KEY>]
```

- **Accounts** (left): a tree of accounts, databases and containers. Open an account with Enter or → to load its containers. Opening a container lists its first 100 documents. The account list from the last run shows at once while a fresh one loads in the background. It is cached in `%LOCALAPPDATA%\cosmoscli` on Windows, `~/Library/Caches/cosmoscli` on macOS and `~/.cache/cosmoscli` on Linux.
- **Results** (middle): the id and partition key of each document found, 100 at a time. When the title says `more ↓`, press ↓ on the last result to load the next 100.
- **Document** (right): the selected document as JSON.
- **Search** (top): press `/` and type a query, then Enter. A `SELECT` statement runs as typed. Clauses like `WHERE c.status = 'open'` or `ORDER BY c._ts DESC` follow `SELECT * FROM c`. A bare condition like `c.total > 10` becomes a `WHERE` clause.

Tab and Shift-Tab move between panes, `r` runs the query again, `?` shows every key and `q` quits.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The workspace has three crates:
- `crates/cosmos-core`: Azure access shared by the front ends. The `Connector` finds accounts and opens container connections. The adapters in `arm.rs` and `cosmos.rs` are kept thin, and `testing.rs` has in-memory fakes of the control and data planes (feature `test-support`).
- `crates/cosmoscli`: the command line tool and its interactive mode.
- `crates/cosmostui`: the terminal UI. Keys and finished background work go through `state::update`, which returns the work to start next, so it is tested without a terminal.
