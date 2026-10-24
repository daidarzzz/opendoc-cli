//! OpenDoc CLI: búsqueda y apertura de docsets fuera de línea.
//!
//! Reutiliza el mismo núcleo que la app (`opendoc-core`): escaneo,
//! índices SQLite, cachés tarix y búsqueda difusa. La app nunca se toca:
//! este binario solo lee su carpeta de docsets y su caché.

mod cli;
mod error;
mod paths;
mod run;
mod ui;

use std::io::Write;

use clap::Parser;

use crate::cli::Cli;

fn main() {
    let cli = Cli::parse();
    if let Err(error) = run::run(&cli) {
        let mut stderr = anstream::stderr();
        let _ = writeln!(stderr, "{} {error}", ui::paint(ui::ERROR, "error:"));
        std::process::exit(1);
    }
}
