# termos

A terminal UI for Azure Cosmos DB (NoSQL API). It browses accounts, containers and documents in three panes, like the Data Explorer in the Azure portal.

## Requirements

- Rust 1.88 or later
- The [Azure CLI](https://learn.microsoft.com/cli/azure/), logged in with `az login`

## Install

```sh
cargo install --path crates/termos
```

## Usage

```sh
termos [--subscription <ID>] [--auth auto|entra|key] [--key <KEY>]
```

- **Accounts** (left): a tree of accounts, databases and containers. Open an account with Enter or → to load its containers. Opening a container lists its first 100 documents. The account list from the last run shows at once while a fresh one loads in the background. It is cached in `%LOCALAPPDATA%\termos` on Windows, `~/Library/Caches/termos` on macOS and `~/.cache/termos` on Linux.
- **Results** (middle): the id and partition key of each document found, 100 at a time. When the title says `more ↓`, press ↓ on the last result to load the next 100.
- **Document** (right): the selected document as JSON.
- **Search** (top): press `/` and type a query, then Enter. A `SELECT` statement runs as typed. Clauses like `WHERE c.status = 'open'` or `ORDER BY c._ts DESC` follow `SELECT * FROM c`. A bare condition like `c.total > 10` becomes a `WHERE` clause.

Tab and Shift-Tab move between panes, `r` runs the query again, `?` shows every key and `q` quits.

## Authentication

Accounts, databases and containers are found through Azure Resource Manager, using your `az login` identity. Accounts come from every subscription you can access; add `--subscription <ID>` to list just one.

Documents are read through the Cosmos DB data plane. `--auth` picks how to authenticate:

| `--auth` | Behaviour |
| --- | --- |
| `auto` (default) | Try Entra ID first. If you have no Cosmos DB data plane role, fall back to the account key fetched from Resource Manager. |
| `entra` | Only use Entra ID. |
| `key` | Only use the account key fetched from Resource Manager. |

Pass `--key <KEY>` to supply an account key yourself.

## Development

```sh
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The workspace has two crates:
- `crates/cosmos-core`: Azure access. The `Connector` finds accounts and opens container connections. The adapters in `arm.rs` and `cosmos.rs` are kept thin, and `testing.rs` has in-memory fakes of the control and data planes (feature `test-support`).
- `crates/termos`: the terminal UI. Keys and finished background work go through `state::update`, which returns the work to start next, so it is tested without a terminal.
