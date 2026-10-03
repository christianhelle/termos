# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## Unreleased

### Added

- Write SQL in a query editor with `n`, with numbered, coloured lines, and run it with F5, Ctrl-R or Shift-Enter.
- Show the query editor's results as one JSON array, and load more with ↓ at the end.
- Show the request charge, round trips and query metrics in a stats tab, switched to with `s`.
- Copy the query results with `y`, save them to a JSON file with `w`, and save the query with Ctrl-S.
- Show the request charge of each query on the status line.
- Show why a query failed in a dialog, with the service's own message and the line and column it points at, and explain queries such as aggregates that the Azure Cosmos DB SDK for Rust cannot run across partitions yet, or parameters termos cannot set.

## [0.1.5] - 2026-10-02

### Added

- Browse the local Cosmos DB emulator with `--emulator`, without Azure or `az login`.
- Delete the selected document with `d` in the results or document pane, after confirming in a dialog.
- Mark several results with Space or Ctrl-A, and delete them together with `d`.
- Show `SELECT * FROM c` to the left of the search bar, and a dimmed hint in it while it is empty.
- Show a scrollbar on the right border of the accounts, results and document when they hold more than fits.
- Move the tree selection a page with PgUp and PgDn, half a page with Ctrl-D and Ctrl-U, and to the first or last row with `gg` and `G`.

### Changed

- Selecting a container lists its documents at once, and Enter on it moves to the results.
- Tab and Shift-Tab skip the search bar, which is reached with `/`.

## [0.1.3] - 2026-10-01

### Added

- Scroll the results and the document with PgUp, PgDn, Home and End.
- Scroll half a page with Ctrl-D and Ctrl-U, and jump to the top or bottom with `gg` and `G`.
- Select the first or last tree row with Home and End.
- Mouse support: click to pick and open, and scroll with the wheel.
- Toggle the accounts tree with Ctrl-B; the other panes take its room while it is hidden.
- Zoom the focused pane to the whole screen with `z`.
- Pick document text by dragging the mouse over it, and copy the picked text (or the whole document) to the system clipboard with `y`. Esc drops the selection.

### Fixed

- The install script no longer fails after a successful install.

## [0.1.2] - 2026-09-30

First release of termos, a terminal UI for browsing Azure Cosmos DB.

### Added

- Accounts tree listing every Cosmos DB account across all subscriptions, with containers grouped by database.
- Query a container's documents, with a search bar that accepts a full `SELECT` statement, extra clauses or a bare filter condition.
- Results pane showing the id and partition key of each document, loading more pages on demand.
- Document pane with pretty-printed, syntax-coloured JSON.
- Keyboard navigation with arrow and vim keys, and key help on `?`.
- Entra ID authentication with fallback to account keys, with token caching and prefetching.
- Cached account list so the tree shows at once on startup while it refreshes.
- Release builds for Linux, macOS and Windows on x64 and ARM64.
- Install scripts for macOS, Linux and Windows.

[0.1.5]: https://github.com/christianhelle/termos/compare/0.1.3...0.1.5
[0.1.3]: https://github.com/christianhelle/termos/compare/0.1.2...0.1.3
[0.1.2]: https://github.com/christianhelle/termos/releases/tag/0.1.2
