//! Errores del CLI. Todos se imprimen limpios y devuelven código 1.

use std::path::PathBuf;

/// Fallo en tiempo de ejecución (no de uso: clap cubre el 2).
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    /// No se pudo resolver el directorio de datos de la app.
    #[error("cannot locate the OpenDoc data directory; pass --docsets explicitly")]
    NoDataDir,
    /// La carpeta de docsets no existe o no es un directorio.
    #[error(
        "docsets folder not found: {path}\n\
         Set it with --docsets, the OPENDOC_DOCSETS variable or the OpenDoc settings."
    )]
    DocsetsMissing {
        /// Ruta comprobada.
        path: PathBuf,
    },
    /// El docset pedido con `--docset` no existe.
    #[error("docset not found: {0}")]
    UnknownDocset(String),
    /// La búsqueda no devolvió ninguna coincidencia (solo en `open`).
    #[error("no match for: {0}")]
    NoMatch(String),
    /// El docset no tiene `Documents/` (tarix pendiente de extraer).
    #[error("docset '{id}' has no extracted documents (tarix not installed)")]
    NoDocuments {
        /// Id del docset.
        id: String,
    },
    /// No se pudo resolver la página dentro del docset.
    #[error("cannot resolve '{path}' in docset '{id}': {reason}")]
    ResolveFailed {
        /// Id del docset.
        id: String,
        /// Ruta relativa pedida.
        path: String,
        /// Motivo (del núcleo).
        reason: String,
    },
    /// Fallo de carga del escaneo o de los índices.
    #[error("cannot load docsets: {0}")]
    Load(String),
    /// No se pudo abrir el fichero con la aplicación por defecto.
    #[error("cannot open {}: {source}", path.display())]
    OpenFailed {
        /// Fichero que se intentó abrir.
        path: PathBuf,
        /// Error del sistema.
        #[source]
        source: std::io::Error,
    },
}
