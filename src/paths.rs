//! Resolución de la carpeta de docsets y de la caché tarix.
//!
//! Orden (de más explícito a menos):
//! 1. `--docsets` / `--tarix-cache`.
//! 2. `OPENDOC_DOCSETS` / `OPENDOC_TARIX_CACHE`.
//! 3. `docsets_dir` de los ajustes de la app.
//! 4. `<datos>/com.opendoc.app/{docsets,tarix-cache}`.

use std::path::{Path, PathBuf};

use crate::error::CliError;

/// Carpeta dentro del directorio de datos del sistema.
pub const APP_DIR: &str = "com.opendoc.app";

/// Rutas efectivas del CLI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locations {
    /// Carpeta que contiene los `*.docset`.
    pub docsets: PathBuf,
    /// Base de `tarix-cache/<id>/`.
    pub tarix_cache: PathBuf,
}

/// Directorio de datos de OpenDoc (`<data>/com.opendoc.app`).
pub fn app_data_dir() -> Result<PathBuf, CliError> {
    dirs::data_dir()
        .map(|dir| dir.join(APP_DIR))
        .ok_or(CliError::NoDataDir)
}

/// Lee `docsets_dir` de los ajustes sin efectos secundarios (no rota el
/// archivo corrupto ni escribe defaults: eso es cosa de la app).
pub fn settings_docsets(data_dir: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(data_dir.join("settings.json")).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value
        .get("docsets_dir")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
}

/// Resuelve ambas rutas aplicando la precedencia anterior.
pub fn resolve(
    explicit_docsets: Option<&Path>,
    explicit_cache: Option<&Path>,
) -> Result<Locations, CliError> {
    let data_dir = app_data_dir()?;
    let docsets = explicit_docsets
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("OPENDOC_DOCSETS").map(PathBuf::from))
        .or_else(|| settings_docsets(&data_dir))
        .unwrap_or_else(|| data_dir.join("docsets"));
    let tarix_cache = explicit_cache
        .map(Path::to_path_buf)
        .or_else(|| std::env::var_os("OPENDOC_TARIX_CACHE").map(PathBuf::from))
        .unwrap_or_else(|| data_dir.join("tarix-cache"));
    Ok(Locations {
        docsets,
        tarix_cache,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_docsets_reads_only_valid_strings() {
        let dir = tempfile::tempdir().expect("tempdir");
        // Ausente → None.
        assert_eq!(settings_docsets(dir.path()), None);
        // Cadena válida.
        std::fs::write(
            dir.path().join("settings.json"),
            r#"{"version":1,"docsets_dir":"C:/docs"}"#,
        )
        .expect("write");
        assert_eq!(settings_docsets(dir.path()), Some(PathBuf::from("C:/docs")));
        // Cadena vacía o tipo raro → None.
        std::fs::write(dir.path().join("settings.json"), r#"{"docsets_dir":""}"#).expect("write");
        assert_eq!(settings_docsets(dir.path()), None);
        std::fs::write(dir.path().join("settings.json"), r#"{"docsets_dir":42}"#).expect("write");
        assert_eq!(settings_docsets(dir.path()), None);
        // Archivo corrupto → None sin tocar el archivo.
        std::fs::write(dir.path().join("settings.json"), b"{roto").expect("write");
        assert_eq!(settings_docsets(dir.path()), None);
        assert_eq!(
            std::fs::read(dir.path().join("settings.json")).expect("read"),
            b"{roto"
        );
    }
}
