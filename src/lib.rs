//! OpenDoc CLI (`odc`): búsqueda, visualización y gestión de docsets fuera de línea.
//!
//! Reutiliza el mismo núcleo que la app (`opendoc-core`): escaneo,
//! índices SQLite, cachés tarix y búsqueda difusa.

pub mod cli;
pub mod clip;
pub mod context;
pub mod error;
pub mod pager;
pub mod paths;
pub mod query;
pub mod render;
pub mod run;
pub mod state;
pub mod tui;
pub mod ui;

use crate::cli::Cli;
use clap::Parser;
use std::io::Write;

pub fn main_entry() {
    let cli = Cli::parse();
    if let Err(error) = run::run(&cli) {
        let mut stderr = anstream::stderr();
        let _ = writeln!(stderr, "{} {error}", ui::paint(ui::ERROR, "error:"));
        std::process::exit(1);
    }
}
