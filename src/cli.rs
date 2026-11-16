//! Command-line interface definitions.

use std::path::PathBuf;

use clap::{builder::styling::Styles, Parser, Subcommand};

use crate::ui;

/// Catppuccin Mocha styles for command help.
pub fn cli_styles() -> Styles {
    Styles::styled()
        .header(ui::ACCENT.bold())
        .usage(ui::ACCENT.bold())
        .literal(ui::PEACH)
        .placeholder(ui::MUTED)
        .valid(ui::DOCSET)
        .invalid(ui::ERROR)
        .error(ui::ERROR.bold())
}

/// Search, read, and manage offline docsets.
#[derive(Debug, Parser)]
#[command(
    name = "opendoc-cli",
    version,
    about = "Search, read, and manage offline docsets from the terminal",
    long_about = None,
    disable_help_subcommand = true,
    styles = cli_styles()
)]
pub struct Cli {
    /// Directory containing `*.docset` bundles (defaults to OpenDoc settings).
    #[arg(long, global = true, value_name = "DIR")]
    pub docsets: Option<PathBuf>,
    /// Root of the tarix cache (defaults to the OpenDoc data directory).
    #[arg(long, global = true, value_name = "DIR")]
    pub tarix_cache: Option<PathBuf>,
    /// Disable project-based docset prioritization.
    #[arg(long, global = true)]
    pub no_context: bool,
    /// Command to run. An interactive terminal starts the TUI by default.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Available commands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Search entries by name across loaded docsets.
    Search {
        /// Search text, optionally prefixed with `docset:`.
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Limit the search to one docset (ID or name).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Maximum number of results (default 20, maximum 500).
        #[arg(long, value_name = "N", default_value_t = 20)]
        limit: usize,
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Read documentation in the terminal.
    #[command(alias = "cat")]
    Show {
        /// Search text or symbol, such as "Vec::push" or "rust:Vec::push".
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Limit the search to one docset (ID or name).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Show only the signature and code examples.
        #[arg(long)]
        code: bool,
        /// Print plain text without colors or decorative headings.
        #[arg(long)]
        raw: bool,
        /// Copy rendered documentation to the clipboard.
        #[arg(long)]
        copy: bool,
        /// Print the signature, body, and code examples as JSON.
        #[arg(long)]
        json: bool,
        /// Output width (defaults to the terminal width, capped at 110).
        #[arg(long, value_name = "COLS")]
        width: Option<usize>,
    },
    /// Open a result in the default viewer or browser.
    Open {
        /// Search text, optionally prefixed with `docset:`.
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Limit the search to one docset (ID or name).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Skip the interactive picker.
        #[arg(long)]
        non_interactive: bool,
        /// Open the top result without prompting.
        #[arg(long)]
        exact: bool,
        /// Print the resolved file path without opening it.
        #[arg(long)]
        path: bool,
    },
    /// Launch the interactive terminal UI with live search and preview.
    Tui {
        /// Select the initial docset in the TUI.
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Initial search query.
        #[arg(value_name = "QUERY")]
        query: Option<String>,
    },
    /// List installed docsets and pending tarix archives.
    List {
        /// Print machine-readable JSON.
        #[arg(long)]
        json: bool,
    },
    /// Browse, install, update, and uninstall docsets.
    Docsets {
        #[command(subcommand)]
        command: DocsetCommand,
    },
    /// Update OpenDoc CLI from GitHub Releases.
    Update {
        /// Report the installed and latest versions without installing.
        #[arg(long)]
        check: bool,
    },
    /// Generate shell completions for Bash, Zsh, Fish, PowerShell, or Elvish.
    Completions {
        /// Target shell.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}

#[derive(Debug, Subcommand)]
pub enum DocsetCommand {
    /// Browse the catalog and show installed docsets.
    Catalog {
        /// Filter by name or catalog ID.
        query: Option<String>,
    },
    /// Fetch a fresh copy of the catalog.
    Refresh,
    /// Show or configure additional GitHub feed repositories.
    Feeds {
        #[command(subcommand)]
        command: Option<FeedCommand>,
    },
    /// Install a docset by its catalog ID.
    Install { id: String },
    /// Update an installed docset by its catalog ID.
    Update {
        id: String,
        /// Reinstall from the catalog even when versions cannot be compared.
        #[arg(long)]
        force: bool,
    },
    /// Uninstall a docset by catalog or installed ID.
    Uninstall {
        id: String,
        /// Skip the confirmation prompt.
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Debug, Subcommand)]
pub enum FeedCommand {
    /// Set additional GitHub repositories. Omit them to use only DevDocs.
    Set { repos: Vec<String> },
    /// Inherit OpenDoc's configured feeds again.
    Reset,
}
