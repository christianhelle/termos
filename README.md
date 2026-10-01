# termos

[![Build](https://github.com/christianhelle/termos/actions/workflows/build.yml/badge.svg)](https://github.com/christianhelle/termos/actions/workflows/build.yml)
![Crates.io Version](https://img.shields.io/crates/v/termos)

A terminal UI for Azure Cosmos DB (NoSQL API). It browses accounts, containers and documents in three panes, like the Data Explorer in the Azure portal.

## Requirements

- Rust 1.88 or later, when installing with `cargo`
- The [Azure CLI](https://learn.microsoft.com/cli/azure/), logged in with `az login`

## Install

### macOS/Linux

```bash
curl -fsSL https://christianhelle.com/termos/install | bash
```

This installs the latest release to `~/.local/bin`. To install somewhere else, or to pin a release:

```bash
curl -fsSL https://christianhelle.com/termos/install | INSTALL_DIR="$HOME/bin" bash
curl -fsSL https://christianhelle.com/termos/install | VERSION="<tag>" bash
```

### Windows PowerShell

```powershell
irm https://christianhelle.com/termos/install.ps1 | iex
```

This installs the latest release to `%LOCALAPPDATA%\Programs\termos`, or to `~\.local\bin` or `~\bin` if one of those already exists, and adds it to your user `PATH`. To install somewhere else, or to pin a release:

```powershell
$install = irm https://christianhelle.com/termos/install.ps1
& ([scriptblock]::Create($install)) -InstallDir "$env:USERPROFILE\bin"
& ([scriptblock]::Create($install)) -Version "<tag>"
```

### Cargo

```bash
cargo install termos
```

To build from a clone of this repository, run `cargo install --path .`.

### Release archives

Download a build from [Releases](https://github.com/christianhelle/termos/releases). Archives are available for Linux, macOS and Windows, on x64 and ARM64.

## Usage

```sh
termos [--subscription <ID>] [--auth auto|entra|key] [--key <KEY>]
```

- **Accounts** (left): a tree of accounts, databases and containers. Open an account with Enter or → to load its containers. Opening a container lists its first 100 documents. The account list from the last run shows at once while a fresh one loads in the background. It is cached in `%LOCALAPPDATA%\termos` on Windows, `~/Library/Caches/termos` on macOS and `~/.cache/termos` on Linux.
- **Results** (middle): the id and partition key of each document found, 100 at a time. When the title says `more ↓`, press ↓ on the last result to load the next 100.
- **Document** (right): the selected document as JSON. Drag the mouse over it to pick text, then press `y` to copy it to the clipboard. With nothing picked, `y` copies the whole document. Esc drops the picked text.
- **Search** (top): press `/` and type a query, then Enter. A `SELECT` statement runs as typed. Clauses like `WHERE c.status = 'open'` or `ORDER BY c._ts DESC` follow `SELECT * FROM c`. A bare condition like `c.total > 10` becomes a `WHERE` clause.

Tab and Shift-Tab move between panes, Ctrl-B hides or shows the accounts to give the other panes more room, `z` zooms the focused pane to fill the screen and shows every pane again, `r` runs the query again, `?` shows every key and `q` quits.

The mouse works too. Click a pane to focus it, or a node or document to select it. Click a selected node again to open or close it, the same as Enter. The wheel scrolls the pane under the mouse. Because termos captures the mouse, drag in the document to pick text there, or hold Shift to select text anywhere in most terminals.

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
cargo test
cargo clippy --all-targets -- -D warnings
cargo fmt --check
```

- Azure access: the `Connector` lists accounts and opens container connections. The adapters in `arm.rs` and `cosmos.rs` are kept thin, and `testing.rs` has in-memory fakes of the control and data planes for tests.
- The terminal UI: keys and finished background work go through `state::update`, which returns the work to start next, so it is tested without a terminal.

## License

[MIT](LICENSE)
