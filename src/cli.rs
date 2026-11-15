//! Definición de la interfaz de línea de comandos (clap).

use std::path::PathBuf;

use clap::{builder::styling::Styles, Parser, Subcommand};

use crate::ui;

/// Paleta Catppuccin Mocha para la ayuda (--help).
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

/// Búsqueda, visualización y gestión de docsets fuera de línea.
#[derive(Debug, Parser)]
#[command(
    name = "opendoc-cli",
    version,
    about = "Search, view, and browse offline docsets from the terminal",
    long_about = None,
    disable_help_subcommand = true,
    styles = cli_styles()
)]
pub struct Cli {
    /// Carpeta que contiene los `*.docset` (por defecto: ajustes de OpenDoc).
    #[arg(long, global = true, value_name = "DIR")]
    pub docsets: Option<PathBuf>,
    /// Base de la caché tarix (por defecto: datos de OpenDoc).
    #[arg(long, global = true, value_name = "DIR")]
    pub tarix_cache: Option<PathBuf>,
    /// No priorizar el docset del proyecto actual (Cargo.toml, package.json…).
    #[arg(long, global = true)]
    pub no_context: bool,
    /// Subcomando a ejecutar. Si se omite en terminal interactiva, inicia la TUI.
    #[command(subcommand)]
    pub command: Option<Command>,
}

/// Subcomandos disponibles.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Busca entradas por nombre en todos los docsets cargados.
    Search {
        /// Texto a buscar (una o varias palabras, o prefijo `docset:query`).
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
    /// Muestra la documentación directamente en la terminal sin abrir el navegador.
    #[command(alias = "cat")]
    Show {
        /// Texto a buscar o símbolo (ej. "Vec::push" o "rust:Vec::push").
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Restringe la búsqueda a un docset (id o nombre).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Muestra únicamente los bloques de código o ejemplos de la documentación.
        #[arg(long)]
        code: bool,
        /// Muestra el texto plano/crudo sin colores ni cabeceras decorativas.
        #[arg(long)]
        raw: bool,
        /// Copia la documentación renderizada directamente al portapapeles del sistema.
        #[arg(long)]
        copy: bool,
        /// Salida estructurada JSON (firma, cuerpo, ejemplos de código).
        #[arg(long)]
        json: bool,
        /// Ancho de maquetación (por defecto, el del terminal; máx. 110).
        #[arg(long, value_name = "COLS")]
        width: Option<usize>,
    },
    /// Abre la mejor coincidencia en el navegador o visor del sistema.
    Open {
        /// Texto a buscar (una o varias palabras, o prefijo `docset:query`).
        #[arg(required = true, num_args = 1.., value_name = "QUERY")]
        query: Vec<String>,
        /// Restringe la búsqueda a un docset (id o nombre).
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Desactiva el selector interactivo si hay múltiples coincidencias.
        #[arg(long)]
        non_interactive: bool,
        /// Abre la mejor coincidencia directamente sin interactividad.
        #[arg(long)]
        exact: bool,
        /// Imprime solo la ruta del fichero resuelto en disco sin abrir el visor del sistema.
        #[arg(long)]
        path: bool,
    },
    /// Interfaz interactiva de pantalla completa (TUI) con búsqueda en vivo y vista previa.
    Tui {
        /// Restringe la TUI a un docset inicial.
        #[arg(long, value_name = "DOCSET")]
        docset: Option<String>,
        /// Filtro inicial de búsqueda.
        #[arg(value_name = "QUERY")]
        query: Option<String>,
    },
    /// Lista los docsets instalados y los tarix pendientes.
    List {
        /// Salida JSON (una máquina la consume).
        #[arg(long)]
        json: bool,
    },
    /// Genera el autocompletado para tu shell (bash, zsh, fish, powershell, elvish).
    Completions {
        /// Shell de destino.
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },
}
