//! Definición de la interfaz de línea de comandos (clap).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Búsqueda y apertura de docsets de Dash/Zeal fuera de línea.
#[derive(Debug, Parser)]
#[command(
    name = "opendoc-cli",
    version,
    about = "Search and open offline docsets (Dash/Zeal) from the terminal",
    long_about = None,
    disable_help_subcommand = true
)]
pub struct Cli {
    /// Carpeta que contiene los `*.docset` (por defecto: ajustes de OpenDoc).
    #[arg(long, global = true, value_name = "DIR")]
    pub docsets: Option<PathBuf>,
    /// Base de la caché tarix (por defecto: datos de OpenDoc).
    #[arg(long, global = true, value_name = "DIR")]
    pub tarix_cache: Option<PathBuf>,
    /// Subcomando a ejecutar.
    #[command(subcommand)]
    pub command: Command,
}

/// Subcomandos disponibles.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Busca entradas por nombre en todos los docsets cargados.
    Search {
        /// Texto a buscar (una o varias palabras).
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Restringe la búsqueda a un docset (id o nombre).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Número máximo de resultados (por defecto 20, tope 500).
        #[arg(long, value_name = "N", default_value_t = 20)]
        limit: usize,
        /// Salida JSON (una máquina la consume).
        #[arg(long)]
        json: bool,
    },
    /// Abre la mejor coincidencia en el visor del sistema.
    Open {
        /// Texto a buscar (una o varias palabras).
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Restringe la búsqueda a un docset (id o nombre).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
    },
    /// Lista los docsets instalados y los tarix pendientes.
    List {
        /// Salida JSON (una máquina la consume).
        #[arg(long)]
        json: bool,
    },
}
