//! Installs DevDocs entries as standard docsets.
//!
//! Resumable downloads fetch the index and page database from DevDocs. The
//! shared converter stages and validates the result before replacing an
//! existing installation. The returned `InstalledDocset` goes through the
//! same library and search integration as a classic archive.

use std::path::Path;

use opendoc_core::devdocs::{self, DevDocsError, DevDocsMeta};

use super::install::{
    download_url_patient, part_path_for, sweep_stale_temps, InstallError, ProgressReport,
    ProgressStage,
};
use crate::catalog::feed::FeedEntry;
use opendoc_core::docset::{slugify, TarixLimits};

/// Convierte un error del conversor en el error de instalación de la app.
fn map_convert_error(id: &str, error: DevDocsError) -> InstallError {
    match error {
        DevDocsError::Io(_) | DevDocsError::Sqlite(_) | DevDocsError::Plist(_) => {
            InstallError::InstallFailed {
                id: id.to_string(),
                message: error.to_string(),
            }
        }
        other => InstallError::InvalidPackage {
            id: id.to_string(),
            message: other.to_string(),
        },
    }
}

/// Descarga, convierte y coloca la documentación de `entry`.
///
/// Bloqueante: solo desde `spawn_blocking`. `entry.devdocs` debe estar
/// presente (si no, `UnsupportedPackage`).
pub fn install_devdocs(
    client: &reqwest::blocking::Client,
    entry: &FeedEntry,
    docsets_dir: &Path,
    final_dir: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<(), InstallError> {
    // Las URLs salen del slug, nunca de `entry.urls`: un `catalog.json`
    // manipulado no puede apuntar la descarga a otro servidor.
    let slug = entry.devdocs.as_ref().map_or("", |m| m.slug.as_str());
    let urls = (devdocs::index_url(slug), devdocs::db_url(slug));
    install_from(
        client,
        entry,
        &urls,
        docsets_dir,
        final_dir,
        limits,
        on_progress,
    )
}

/// Núcleo de la instalación con las URLs ya decididas (los tests usan un
/// servidor local).
#[allow(clippy::too_many_arguments)]
fn install_from(
    client: &reqwest::blocking::Client,
    entry: &FeedEntry,
    urls: &(String, String),
    docsets_dir: &Path,
    final_dir: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<(), InstallError> {
    let meta: &DevDocsMeta =
        entry
            .devdocs
            .as_ref()
            .ok_or_else(|| InstallError::UnsupportedPackage {
                id: entry.id.clone(),
            })?;
    if !docsets_dir.is_dir() {
        return Err(InstallError::NoDocsetsDir {
            path: docsets_dir.to_path_buf(),
        });
    }
    sweep_stale_temps(docsets_dir);
    let slug = slugify(&entry.id);
    let (index_url, db_url) = (&urls.0, &urls.1);
    let index_part = part_path_for(docsets_dir, &slug, index_url);
    let db_part = part_path_for(docsets_dir, &slug, db_url);
    let cleanup = || {
        let _ = std::fs::remove_file(&index_part);
        let _ = std::fs::remove_file(&db_part);
    };
    let download_failed = |message: String| InstallError::DownloadFailed {
        id: entry.id.clone(),
        message,
    };

    // `index.json` es pequeño: sin progreso propio. `db.json` marca la barra.
    let silent = |_: ProgressReport| {};
    download_url_patient(client, index_url, &index_part, limits, &silent)
        .map_err(download_failed)?;
    if let Err(message) = download_url_patient(client, db_url, &db_part, limits, on_progress) {
        // El parcial de `db.json` se conserva: el reintento lo reanuda.
        let _ = std::fs::remove_file(&index_part);
        return Err(download_failed(message));
    }

    on_progress(ProgressReport {
        stage: ProgressStage::Extracting,
        received_bytes: 0,
        total_bytes: None,
        files: 0,
    });
    let read = |path: &Path| {
        std::fs::read(path).map_err(|e| InstallError::InstallFailed {
            id: entry.id.clone(),
            message: format!("cannot read {}: {e}", path.display()),
        })
    };
    let (index_json, db_json) = match (read(&index_part), read(&db_part)) {
        (Ok(i), Ok(d)) => (i, d),
        (Err(e), _) | (_, Err(e)) => {
            cleanup();
            return Err(e);
        }
    };
    on_progress(ProgressReport {
        stage: ProgressStage::Verifying,
        received_bytes: 0,
        total_bytes: None,
        files: 0,
    });
    let converted = devdocs::convert(&index_json, &db_json, meta, final_dir);
    cleanup();
    converted.map_err(|e| map_convert_error(&entry.id, e))?;
    Ok(())
}
