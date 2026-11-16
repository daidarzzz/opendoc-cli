# OpenDoc CLI

[![Release](https://img.shields.io/github/v/release/daidarzzz/opendoc-cli)](https://github.com/daidarzzz/opendoc-cli/releases)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/license-GPL--3.0-or--later-blue)](LICENSE)

[Español](README.es.md)

Search, read, and manage **Dash and Zeal docsets** in your terminal. OpenDoc CLI shares the scanner, SQLite indexes, tarix cache support, and fuzzy search with [OpenDoc](https://github.com/daidarzzz/opendoc) through [opendoc-core](https://github.com/daidarzzz/opendoc-core).

Use `odc` or `opendoc-cli` to run the same commands. Search and reading work offline. The catalog uses DevDocs and optional GitHub feeds, with network access for refreshes and downloads.

## Installation

**Linux and macOS**

```bash
curl -fsSL https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.sh | sh
```

This installs both executables in `~/.local/bin`.

**Windows PowerShell**

```powershell
irm https://raw.githubusercontent.com/daidarzzz/opendoc-cli/master/install.ps1 | iex
```

This installs both executables in `$HOME\.local\bin` and adds the directory to the user PATH if needed.

**Cargo**

```bash
cargo install --git https://github.com/daidarzzz/opendoc-cli --locked
```

**Manual download**

Download your package from [GitHub Releases](https://github.com/daidarzzz/opendoc-cli/releases). Keep both executables and `opendoc-cli.release` together. The marker allows `odc update` to identify an official release installation.

| Platform | Archive suffix |
| --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-gnu.tar.gz` |
| macOS Apple Silicon | `aarch64-apple-darwin.tar.gz` |
| macOS Intel | `x86_64-apple-darwin.tar.gz` |
| Windows x86_64 | `x86_64-pc-windows-msvc.zip` |

## Quick start

```bash
odc                                     # Full-screen terminal UI
odc search rust:Vec::push --limit 5
odc show 'Vec::push'                     # Read in the terminal
odc show 'Vec::push' --code --copy
odc open css:grid                        # Open in the default browser
odc list                                 # Installed docsets and pending tarix archives
```

Queries can start with a docset name, ID, or alias (`py:dumps`), a symbol type (`fn:map`), or both (`py:fn:dumps`). Search, show, and open prioritize docsets for the current project. Use `--no-context` to turn that off.

## Manage docsets

```bash
odc docsets catalog css
odc docsets install devdocs-css
odc docsets refresh
odc docsets update devdocs-css
odc docsets uninstall devdocs-css
```

Use the ID displayed in brackets by the catalog. IDs for versioned documentation depend on the available feeds. The catalog lists available downloads, installed versions, and updates. On first use, it downloads the catalog. Refresh it explicitly to check for newer docsets.

- `odc docsets update <ID> --force` requests a reinstall, which also works when available and installed versions cannot be compared.
- `odc docsets uninstall <ID> --yes` skips confirmation for scripts. If a local docset ID collides with a catalog entry, use `local:<ID>` for the local docset.
- `odc docsets feeds` lists the active sources. `odc docsets feeds set owner/repo another/repo` configures additional feeds. Run `feeds set` with no repositories to use DevDocs only, or `feeds reset` to inherit OpenDoc's configuration again.

The CLI reads OpenDoc's cached catalog when it matches the active feeds. CLI refreshes and feed overrides are stored separately. Both tools use the same docset directory by default. Reload the desktop app's docset library to pick up changes made by the CLI while the app was open.

### Terminal UI

Press **F4** to open the docset catalog. Type to filter and use the arrow keys to select an entry. Downloads, conversions, and refreshes run in the background while the catalog displays progress.

| Key | Action |
| --- | --- |
| **Enter** | Install, update, or reinstall the selected docset |
| **Delete** | Uninstall the selected docset after confirmation |
| **F5** | Refresh the catalog |
| **F6** | Edit the additional GitHub feeds |
| **Esc** | Return to the reader |

The reader supports live search, syntax highlighting, links, a page outline, search within a page, and favorites and recent pages. Press **F1** to see the shortcuts. Use **Ctrl+D** to pick a docset, **Ctrl+T** to pick a symbol type, and **Ctrl+H** or **F2** to see favorites and recent pages.

## Update the CLI

```bash
odc update --check
odc update
```

`--check` shows the installed version and the latest stable release. `odc update` downloads the official package, checks its SHA-256, and updates both executables. On Windows, the updater finishes after the command exits. Update a Cargo installation using `cargo install` again. To move from v0.2.0 to a version with `odc update`, run your original installer or download a new release package.

## Command reference

| Command | Details |
| --- | --- |
| `odc tui [QUERY]` | Start the terminal UI. Use `--docset` to set the initial docset. |
| `odc search QUERY` | Find matching symbols. Supports `--docset`, `--limit`, and `--json`. |
| `odc show QUERY` | Read the selected symbol. Alias `cat`. Supports `--docset`, `--code`, `--copy`, `--raw`, `--json`, and `--width`. |
| `odc open QUERY` | Open a result using the default viewer or browser. In a terminal, multiple matches open a picker. Use `--exact` or `--non-interactive` to open the top result, or `--path` to print its filesystem path. |
| `odc list` | List installed docsets and pending tarix archives. Supports `--json`. |
| `odc completions SHELL` | Print completions for Bash, Zsh, Fish, PowerShell, or Elvish. |
| `odc --help` | List all commands and options. |

`show` supports the pager when output is interactive. Colors turn off when output is redirected or `NO_COLOR` is set. The JSON modes keep their existing formats. Exit codes are `0` for success or voluntary cancellation, `1` for runtime errors, and `2` for invalid command syntax.

## Data locations

The docset directory is resolved in this order.

1. `--docsets DIR`
2. `OPENDOC_DOCSETS`
3. `docsets_dir` in OpenDoc's settings
4. `<system-data>/com.opendoc.app/docsets`

The tarix cache uses `--tarix-cache DIR`, then `OPENDOC_TARIX_CACHE`, then `<system-data>/com.opendoc.app/tarix-cache`. For an existing local tarix archive, the CLI reuses OpenDoc's extracted cache. Open a pending archive once in OpenDoc to make its documents available in the CLI.

Favorites and recent pages are stored in `<system-data>/com.opendoc.app/cli-state.json`. Set `ODC_STATE` to choose a different file.

## Build and test

```bash
cargo build --release --locked --bins
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
cargo fmt --check
```

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
