//! Downloads and installs catalog entries distributed as classic docset archives.
//!
//! A partial download and a staging directory keep incomplete packages outside
//! the installed library. The archive and its index are validated before the
//! replacement step, which uses an `.old` directory for recovery.
//!
//! Feed matching uses the installed folder name without the `.docset` suffix,
//! compared case-insensitively. Ambiguous matches are reported to the caller.
//! Only dotted numeric versions can be ordered reliably. Progress callbacks
//! report actual bytes or file counts as available.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::catalog::feed::FeedEntry;

use opendoc_core::docset::slugify;
use opendoc_core::docset::tarix::unpack_validated;
use opendoc_core::docset::{read_index, read_info_plist};
use opendoc_core::docset::{Docset, TarixError, TarixLimits};

/// Etapa del progreso (sin porcentajes: bytes y conteos reales).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgressStage {
    /// Descargando el `.tgz` (`received_bytes`/`total_bytes` si se conoce).
    Downloading,
    /// Extrayendo el `.tgz` en temporal (`files` procesados).
    Extracting,
    /// Validando layout e índice del extraído.
    Verifying,
}

/// Informe de progreso para el callback (el comando lo emite a la UI).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ProgressReport {
    /// Etapa actual.
    pub stage: ProgressStage,
    /// Bytes descargados (solo en `Downloading`).
    pub received_bytes: u64,
    /// Total anunciado por el servidor, si lo hay.
    pub total_bytes: Option<u64>,
    /// Entradas del tar procesadas (solo en `Extracting`).
    pub files: u64,
}

/// Error de la operación de descarga/instalación. Las precondiciones
/// (catálogo ausente, feed desconocido, ya instalado, ambigüedad) se
/// comprueban antes en `service` con `ApiError`.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// La carpeta de docsets no existe o no es un directorio.
    #[error("docsets folder does not exist or is not a directory: {}", path.display())]
    NoDocsetsDir {
        /// Ruta rechazada.
        path: PathBuf,
    },
    /// La entrada no trae ninguna URL.
    #[error("entry has no usable download URL: {id}")]
    NoDownloadUrl {
        /// Id del feed.
        id: String,
    },
    /// Ninguna URL es un `.tgz` (único formato de F2).
    #[error("unsupported format (only .tgz): {id}")]
    UnsupportedPackage {
        /// Id del feed.
        id: String,
    },
    /// Todas las URLs fallaron (red o HTTP).
    #[error("download failed for {id}: {message}")]
    DownloadFailed {
        /// Id del feed.
        id: String,
        /// Último error (incluye el HTTP si lo hubo).
        message: String,
    },
    /// El `.tgz` no se pudo extraer (corrupto, malicioso, límites...).
    #[error("extraction failed for {id}: {message}")]
    ExtractFailed {
        /// Id del feed.
        id: String,
        /// Detalle.
        message: String,
    },
    /// Extraído pero sin estructura de docset clásico.
    #[error("invalid package (not a classic docset): {id}: {message}")]
    InvalidPackage {
        /// Id del feed.
        id: String,
        /// Detalle.
        message: String,
    },
    /// Fallo de disco o en los renombrados atómicos.
    #[error("install failed for {id}: {message}")]
    InstallFailed {
        /// Id del feed.
        id: String,
        /// Detalle (incluye si la restauración del `.old` falló).
        message: String,
    },
}

/// Coincidencia estricta entre un feed y los docsets instalados.
#[derive(Debug)]
pub enum FeedMatch<'a> {
    /// Ninguna carpeta coincide.
    None,
    /// Una sola carpeta coincide.
    One(&'a Docset),
    /// Varias carpetas coinciden (variantes de mayúsculas en Linux):
    /// no se elige ninguna.
    Ambiguous(Vec<&'a Docset>),
}

/// Busca la carpeta instalada de un feed: el stem de `root_path` (sin
/// `.docset`) igual al id del feed, insensible a mayúsculas. Solo eso:
/// ni slugs, ni nombres del plist, ni substrings.
pub fn match_feed<'a>(feed_id: &str, docsets: &'a [Docset]) -> FeedMatch<'a> {
    let wanted = feed_id.to_lowercase();
    let mut hits = Vec::new();
    for docset in docsets {
        if folder_stem(&docset.root_path).as_deref() == Some(wanted.as_str()) {
            hits.push(docset);
        }
    }
    match hits.len() {
        0 => FeedMatch::None,
        1 => FeedMatch::One(hits[0]),
        _ => FeedMatch::Ambiguous(hits),
    }
}

/// Stem de la carpeta sin la extensión `.docset` (insensible a
/// mayúsculas), en minúsculas. `None` si no es una carpeta `.docset`.
pub(crate) fn folder_stem(root_path: &Path) -> Option<String> {
    let name = root_path.file_name()?.to_string_lossy();
    if !name.to_lowercase().ends_with(".docset") {
        return None;
    }
    let stem_len = name.len().checked_sub(".docset".len())?;
    Some(name[..stem_len].to_lowercase())
}

/// Compara versiones numéricas punteadas (`1.2.10` > `1.2.3`, los
/// segmentos que faltan valen 0). `None` si alguna no es numérica
/// punteada (no comparable con seguridad).
pub fn compare_versions(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    let pa = parse_numeric_version(a)?;
    let pb = parse_numeric_version(b)?;
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let x = pa.get(i).copied().unwrap_or(0);
        let y = pb.get(i).copied().unwrap_or(0);
        match x.cmp(&y) {
            std::cmp::Ordering::Equal => continue,
            other => return Some(other),
        }
    }
    Some(std::cmp::Ordering::Equal)
}

/// Segmentos numéricos separados por `.`, `-` o `_`. Cualquier otra
/// cosa (vacío, letras, segmentos vacíos, desborde) → `None`.
fn parse_numeric_version(version: &str) -> Option<Vec<u64>> {
    let version = version.trim();
    if version.is_empty() {
        return None;
    }
    let parts: Option<Vec<u64>> = version
        .split(['.', '-', '_'])
        .map(|segment| {
            if segment.is_empty() || !segment.bytes().all(|b| b.is_ascii_digit()) {
                None
            } else {
                segment.parse::<u64>().ok()
            }
        })
        .collect();
    parts.filter(|parts| !parts.is_empty())
}

/// ¿Hay actualización disponible?
/// - `Some(true)`: ambas numéricas y la disponible es más reciente.
/// - `Some(false)`: iguales, o la instalada es más reciente.
/// - `None`: no comparable (instalada desconocida o no numéricas).
pub fn update_available(installed: Option<&str>, available: &str) -> Option<bool> {
    let installed = installed?.trim();
    let available_trimmed = available.trim();
    if installed == available_trimmed {
        return Some(false);
    }
    match compare_versions(installed, available_trimmed) {
        Some(std::cmp::Ordering::Less) => Some(true),
        Some(_) => Some(false),
        None => None,
    }
}

/// `true` si la URL apunta a un `.tgz` (ignora query/fragmento y
/// mayúsculas). Único formato que instala F2.
fn is_tgz_url(url: &str) -> bool {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    path.to_lowercase().ends_with(".tgz")
}

/// Host de una URL en minúsculas (`None` si no hay esquema/host).
/// Sin dependencias: corte léxico en `://`, fin en `/`, `?`, `#` o `:`.
pub(crate) fn url_host(url: &str) -> Option<String> {
    let after_scheme = url.split("://").nth(1)?;
    let host = after_scheme.split(['/', '?', '#', ':']).next()?;
    (!host.is_empty()).then(|| host.to_lowercase())
}

/// Ordena mirrors poniendo primero el recordado (si sigue en la lista);
/// el resto conserva su orden. Estable y sin red.
pub(crate) fn order_mirrors(urls: &[String], prefer_host: Option<&str>) -> Vec<String> {
    let mut ordered: Vec<String> = urls.to_vec();
    if let Some(want) = prefer_host {
        ordered.sort_by_key(|u| url_host(u).as_deref() != Some(want));
    }
    ordered
}

/// Descarga, extrae, valida y coloca atómicamente el docset.
/// - `on_progress`: callback de progreso (descarga real, sin porcentajes
///   inventados).
///   La instalación anterior (si la hay) se conserva hasta que la nueva
///   está validada; si el reemplazo falla, se restaura.
pub fn download_and_install(
    client: &reqwest::blocking::Client,
    entry: &FeedEntry,
    docsets_dir: &Path,
    final_dir: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<(), InstallError> {
    if !docsets_dir.is_dir() {
        return Err(InstallError::NoDocsetsDir {
            path: docsets_dir.to_path_buf(),
        });
    }
    if entry.urls.is_empty() {
        return Err(InstallError::NoDownloadUrl {
            id: entry.id.clone(),
        });
    }
    if !entry.urls.iter().any(|url| is_tgz_url(url)) {
        return Err(InstallError::UnsupportedPackage {
            id: entry.id.clone(),
        });
    }
    sweep_stale_temps(docsets_dir);
    let slug = slugify(&entry.id);
    if reusable_install(entry, final_dir) {
        return Ok(());
    }
    let tmp_dir = unique_temp_path(docsets_dir, &slug);
    let result = download_extract_replace(
        client,
        entry,
        final_dir,
        &tmp_dir,
        docsets_dir,
        &slug,
        limits,
        on_progress,
    );
    // Nunca dejar el temporal de extracción atrás. Los parciales de
    // descarga se conservan a propósito (reanudan reintentos); el
    // ganador se borra al instalar y el resto los barre el sweep.
    if tmp_dir.exists() {
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
    result
}

/// ¿Son la misma versión? Igualdad exacta o numérica (`1.0` = `1.0.0`).
/// `None` instalada → no comparable → nunca reutiliza (se descarga).
fn same_version(installed: Option<&str>, available: &str) -> bool {
    let Some(installed) = installed.map(str::trim).filter(|s| !s.is_empty()) else {
        return false;
    };
    let available = available.trim();
    installed == available
        || compare_versions(installed, available) == Some(std::cmp::Ordering::Equal)
}

/// Reutiliza lo instalado sin descargar cuando coincide en versión y
/// verifica estrictamente (layout + índice legible con entradas).
/// Cualquier duda → `None` y se sigue el flujo normal de descarga.
fn reusable_install(entry: &FeedEntry, final_dir: &Path) -> bool {
    let contents = final_dir.join("Contents");
    let version = read_info_plist(&contents)
        .ok()
        .flatten()
        .and_then(|info| info.version);
    same_version(version.as_deref(), &entry.version)
        && contents.join("Resources/Documents").is_dir()
        && read_index(&contents.join("Resources/docSet.dsidx"), &entry.id)
            .is_ok_and(|data| !data.entries.is_empty())
}

#[allow(clippy::too_many_arguments)]
fn download_extract_replace(
    client: &reqwest::blocking::Client,
    entry: &FeedEntry,
    final_dir: &Path,
    tmp_dir: &Path,
    docsets_dir: &Path,
    slug: &str,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<(), InstallError> {
    let (_bytes, url) = download_mirrors(client, entry, docsets_dir, slug, limits, on_progress)?;
    let dl_path = part_path_for(docsets_dir, slug, &url);
    let result = apply_archive(entry, &dl_path, final_dir, tmp_dir, limits, on_progress);
    let _ = std::fs::remove_file(dl_path);
    result
}

fn apply_archive(
    entry: &FeedEntry,
    package: &Path,
    final_dir: &Path,
    tmp_dir: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<(), InstallError> {
    on_progress(ProgressReport {
        stage: ProgressStage::Extracting,
        received_bytes: 0,
        total_bytes: None,
        files: 0,
    });
    unpack_validated(package, tmp_dir, limits).map_err(|e| InstallError::ExtractFailed {
        id: entry.id.clone(),
        message: tarix_message(&e),
    })?;
    on_progress(ProgressReport {
        stage: ProgressStage::Verifying,
        received_bytes: 0,
        total_bytes: None,
        files: 0,
    });
    let contents = tmp_dir.join("Contents");
    let dsidx = contents.join("Resources/docSet.dsidx");
    let documents = contents.join("Resources/Documents");
    if !dsidx.is_file() || !documents.is_dir() {
        let tarix_inside = contents.join("Resources/tarix.tgz").is_file();
        return Err(InstallError::InvalidPackage {
            id: entry.id.clone(),
            message: if tarix_inside {
                "nested tarix package (not supported)".to_string()
            } else {
                "missing Contents/Resources/docSet.dsidx or Documents/".to_string()
            },
        });
    }
    read_index(&dsidx, &entry.id).map_err(|e| InstallError::InvalidPackage {
        id: entry.id.clone(),
        message: format!("unreadable index: {e}"),
    })?;
    if let Some(backup) = atomic_replace(&entry.id, final_dir, tmp_dir)? {
        let _ = std::fs::remove_dir_all(backup);
    }
    Ok(())
}

/// Mensaje legible de un error de extracción (conserva el detalle del
/// motivo: seguridad, límites o corrupción).
fn tarix_message(error: &TarixError) -> String {
    error.to_string()
}

/// Prueba las URLs `.tgz` en orden (mirrors): la primera que descargue
/// completa gana. Si todas fallan, error con el último motivo.
/// Devuelve los bytes y la URL ganadora (para recordar el mejor espejo).
/// Pasada rápida con failover por espejo lento; si todo falla habiendo
/// espejos lentos, una pasada paciente final sin límite de velocidad.
fn download_mirrors(
    client: &reqwest::blocking::Client,
    entry: &FeedEntry,
    docsets_dir: &Path,
    slug: &str,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<(u64, String), InstallError> {
    let urls: Vec<&String> = entry.urls.iter().filter(|url| is_tgz_url(url)).collect();
    if urls.is_empty() {
        return Err(InstallError::UnsupportedPackage {
            id: entry.id.clone(),
        });
    }
    let mut last_error = String::from("no usable mirrors");
    let mut slow_seen = false;
    for url in &urls {
        match download_one(
            client,
            url,
            &part_path_for(docsets_dir, slug, url),
            limits,
            on_progress,
            false,
        ) {
            Ok(bytes) => return Ok((bytes, (*url).to_string())),
            Err(fail) => {
                slow_seen = slow_seen || fail.slow;
                last_error = fail.message;
            }
        }
    }
    // Reintento paciente (sin corte por lentitud) sobre los espejos que
    // progresaron, reanudando sus parciales. Sin espejos lentos, fallar
    // ya (errores duros como 404 no mejoran reintentando).
    if slow_seen {
        for url in &urls {
            match download_one(
                client,
                url,
                &part_path_for(docsets_dir, slug, url),
                limits,
                on_progress,
                true,
            ) {
                Ok(bytes) => return Ok((bytes, (*url).to_string())),
                Err(fail) => last_error = fail.message,
            }
        }
    }
    Err(InstallError::DownloadFailed {
        id: entry.id.clone(),
        message: last_error,
    })
}

/// Fichero parcial propio de una URL (un espejo nunca mezcla bytes de
/// otro): `.dl-<slug>-<hash16(url)>.part`. Determinista entre intentos.
pub(super) fn part_path_for(docsets_dir: &Path, slug: &str, url: &str) -> PathBuf {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    url.hash(&mut hasher);
    docsets_dir.join(format!(".odc-dl-{slug}-{:016x}.part", hasher.finish()))
}

/// Fallo de un intento de descarga (mensaje + si fue por lentitud).
struct AttemptFail {
    message: String,
    slow: bool,
}

/// Velocidad mínima sostenida para no saltar de espejo (64 KB/s tras
/// 15 s de gracia). Solo en pasada rápida; la paciente no corta.
const SLOW_GRACE_SECS: u64 = 15;
const SLOW_MIN_BPS: u64 = 65_536;

/// `true` si con ese ritmo no merece seguir esperando otro espejo.
fn is_slow(elapsed_ms: u128, bytes: u64) -> bool {
    elapsed_ms >= u128::from(SLOW_GRACE_SECS * 1000)
        && (bytes as u128).saturating_mul(1000) / elapsed_ms.max(1) < u128::from(SLOW_MIN_BPS)
}

/// Descarga una URL a su parcial con tope de tamaño y progreso real.
/// Reanuda (`Range`) si hay parcial previo de la MISMA url y el servidor
/// colabora (206); si lo ignora (200) o lo rechaza (416), empieza de cero.
/// `patient=false` corta espejos lentos; `true` los tolera.
fn download_one(
    client: &reqwest::blocking::Client,
    url: &str,
    dl_path: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
    patient: bool,
) -> Result<u64, AttemptFail> {
    let fail = |message: String, slow: bool| AttemptFail { message, slow };
    let from = match std::fs::symlink_metadata(dl_path) {
        Ok(meta) if meta.is_file() => meta.len().min(limits.max_bytes),
        Ok(_) => {
            return Err(fail(
                format!("unsafe partial file: {}", dl_path.display()),
                false,
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => 0,
        Err(e) => return Err(fail(e.to_string(), false)),
    };
    let mut request = client.get(url);
    if from > 0 {
        request = request.header("Range", format!("bytes={from}-"));
    }
    let response = request
        .send()
        .map_err(|e| fail(format!("network error for {url}: {e}"), false))?;
    let status = response.status();
    // 206 = reanuda; 200 = servidor sin rangos (o sin parcial): de cero.
    // 416 = el parcial no encaja: se borra y se reintenta en limpio
    // (sin Range ya no puede repetirse el 416).
    if status.as_u16() == 206 && from > 0 {
        download_one_body(response, url, dl_path, limits, on_progress, patient, from)
    } else if status.as_u16() == 416 && from > 0 {
        std::fs::remove_file(dl_path).map_err(|e| fail(e.to_string(), false))?;
        download_one(client, url, dl_path, limits, on_progress, patient)
    } else if !status.is_success() {
        Err(fail(format!("HTTP {} for {url}", status.as_u16()), false))
    } else {
        download_one_body(response, url, dl_path, limits, on_progress, patient, 0)
    }
}

/// Cuerpo de la descarga: vuelca el flujo a `dl_path` (anexa si
/// `resume_from > 0`, trunca si es 0), con tope, progreso y corte por
/// espejo lento (salvo pasada paciente). Al terminar exige el total
/// anunciado: si no cuadra, el parcial se borra (no se mezcla basura).
#[allow(clippy::too_many_arguments)]
fn download_one_body(
    mut response: reqwest::blocking::Response,
    url: &str,
    dl_path: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
    patient: bool,
    resume_from: u64,
) -> Result<u64, AttemptFail> {
    let t_dl = std::time::Instant::now();
    let fail = |message: String, slow: bool| AttemptFail { message, slow };
    // Total real esperado (en 206, lo anunciado es solo lo restante).
    let total = response
        .content_length()
        .map(|rest| rest.saturating_add(resume_from));
    if let Some(announced) = total {
        if announced > limits.max_bytes {
            return Err(fail(
                format!(
                    "announced size {announced} exceeds limit {} for {url}",
                    limits.max_bytes
                ),
                false,
            ));
        }
    }
    let mut out = if resume_from > 0 {
        std::fs::OpenOptions::new()
            .append(true)
            .open(dl_path)
            .map_err(|e| {
                fail(
                    format!("cannot append to {}: {e}", dl_path.display()),
                    false,
                )
            })?
    } else {
        std::fs::File::create(dl_path)
            .map_err(|e| fail(format!("cannot write to {}: {e}", dl_path.display()), false))?
    };
    on_progress(ProgressReport {
        stage: ProgressStage::Downloading,
        received_bytes: resume_from,
        total_bytes: total,
        files: 0,
    });
    let mut received: u64 = resume_from;
    let mut attempt_bytes: u64 = 0;
    let mut last_emit: u64 = resume_from;
    let mut buffer = [0_u8; 65536];
    loop {
        let n = response
            .read(&mut buffer)
            .map_err(|e| fail(format!("network cut off for {url}: {e}"), false))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        attempt_bytes += n as u64;
        if received > limits.max_bytes {
            return Err(fail(
                format!(
                    "downloaded size exceeds limit {} for {url}",
                    limits.max_bytes
                ),
                false,
            ));
        }
        out.write_all(&buffer[..n])
            .map_err(|e| fail(format!("disk failure at {}: {e}", dl_path.display()), false))?;
        if !patient && is_slow(t_dl.elapsed().as_millis(), attempt_bytes) {
            let kb_s =
                (attempt_bytes as u128).saturating_mul(1000) / t_dl.elapsed().as_millis().max(1);
            return Err(fail(
                format!(
                    "espejo lento ({} KB/s) en {url}, probando siguiente",
                    kb_s / 1024
                ),
                true,
            ));
        }
        if received - last_emit >= 262_144 {
            last_emit = received;
            on_progress(ProgressReport {
                stage: ProgressStage::Downloading,
                received_bytes: received,
                total_bytes: total,
                files: 0,
            });
        }
    }
    drop(out);
    // Si el total anunciado no cuadra, el parcial no sirve: fuera.
    if let Some(announced) = total {
        if received != announced {
            let _ = std::fs::remove_file(dl_path);
            return Err(fail(
                format!("descarga incompleta en {url}: {received} de {announced} bytes"),
                false,
            ));
        }
    }
    on_progress(ProgressReport {
        stage: ProgressStage::Downloading,
        received_bytes: received,
        total_bytes: total.or(Some(received)),
        files: 0,
    });
    Ok(received)
}

/// Descarga `url` a `dl_path` (reanuda si hay parcial) sin cortar por
/// lentitud y con hasta 3 intentos. Para fuentes de un solo origen
/// (DevDocs), donde no hay otro espejo al que saltar.
pub(super) fn download_url_patient(
    client: &reqwest::blocking::Client,
    url: &str,
    dl_path: &Path,
    limits: TarixLimits,
    on_progress: &(dyn Fn(ProgressReport) + Send + Sync),
) -> Result<u64, String> {
    let mut last = String::new();
    for _ in 0..3 {
        match download_one(client, url, dl_path, limits, on_progress, true) {
            Ok(bytes) => return Ok(bytes),
            Err(fail) => last = fail.message,
        }
    }
    Err(last)
}

/// Sustitución segura: `final→old`, `tmp→final`. Devuelve el respaldo
/// `.old` pendiente de borrar FUERA del camino crítico (el llamador lo
/// elimina en fondo o lo barre al arrancar; la versión nueva ya está
/// viva tras el rename). Si el rename final falla, restaura el `.old`.
/// También recupera un `.old` huérfano de un crash anterior (final ausente).
fn atomic_replace(
    id: &str,
    final_dir: &Path,
    tmp_dir: &Path,
) -> Result<Option<PathBuf>, InstallError> {
    let fail = |message: String| InstallError::InstallFailed {
        id: id.to_string(),
        message,
    };
    let old_dir = final_dir.with_extension("docset.old");
    if !final_dir.exists() && old_dir.exists() {
        std::fs::rename(&old_dir, final_dir)
            .map_err(|e| fail(format!("restoring previous backup: {e}")))?;
    }
    if old_dir.exists() {
        std::fs::remove_dir_all(&old_dir)
            .map_err(|e| fail(format!("cleaning up stale backup: {e}")))?;
    }
    let had_previous = final_dir.exists();
    if had_previous {
        std::fs::rename(final_dir, &old_dir)
            .map_err(|e| fail(format!("moving previous install aside: {e}")))?;
    }
    match std::fs::rename(tmp_dir, final_dir) {
        Ok(()) => Ok(had_previous.then_some(old_dir)),
        Err(rename_error) => {
            if had_previous && old_dir.exists() && !final_dir.exists() {
                if let Err(restore_error) = std::fs::rename(&old_dir, final_dir) {
                    return Err(fail(format!(
                        "placing the new version: {rename_error}; \
                         previous version could not be restored: {restore_error}"
                    )));
                }
            }
            Err(fail(format!("placing the new version: {rename_error}")))
        }
    }
}

/// Ruta temporal única dentro de la carpeta de docsets (mismo sistema
/// de ficheros: los renames son atómicos). Solo para extracción (las
/// descargas usan un `.part` por URL para poder reanudar).
fn unique_temp_path(docsets_dir: &Path, slug: &str) -> PathBuf {
    let pid = std::process::id();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    for attempt in 0..100_u32 {
        let candidate =
            docsets_dir.join(format!(".odc-tmp-install-{slug}-{pid}-{nanos}-{attempt}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    docsets_dir.join(format!(".odc-tmp-install-{slug}-{pid}-{nanos}-99"))
}

/// Limpieza conservadora al empezar: borra temporales de descargas,
/// extracciones o respaldos `.old` con más de una hora (nunca los
/// recientes: podría haber otra instalación en curso en este u otro
/// proceso). Un `.old` solo se borra si su final existe y es válido.
pub(super) fn sweep_stale_temps(docsets_dir: &Path) {
    let entries = match std::fs::read_dir(docsets_dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let old_enough = |path: &Path| {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|modified| SystemTime::now().duration_since(modified).ok())
            .is_some_and(|age| age.as_secs() > 3600)
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() && !is_temp_file(&path) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        // `.tmp-install-*` (extracción tarix) y `.<X>.docset.tmp-<pid>`
        // (conversión de DevDocs, ver `opendoc_core::devdocs::convert`).
        let is_tmp_dir = entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false)
            && (name.starts_with(".odc-tmp-install-"));
        // Respaldo de una instalación anterior: solo si el final sigue
        // en su sitio (el borrado diferido pudo no llegar a correr).
        let is_stale_old =
            name.ends_with(".docset.old") && path.is_dir() && sibling_final_exists(&path);
        if (is_tmp_dir || is_temp_file(&path) || is_stale_old) && old_enough(&path) {
            if path.is_dir() {
                let _ = std::fs::remove_dir_all(&path);
            } else {
                let _ = std::fs::remove_file(&path);
            }
        }
    }
}

/// `true` si junto a `<X>.docset.old` existe `<X>.docset` como
/// directorio (el respaldo ya no protege nada y puede barrerse).
fn sibling_final_exists(old_path: &Path) -> bool {
    let name = old_path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    let Some(stem) = name.strip_suffix(".docset.old") else {
        return false;
    };
    let Some(parent) = old_path.parent() else {
        return false;
    };
    parent.join(format!("{stem}.docset")).is_dir()
}

/// Ficheros temporales propios (descargas a medias).
fn is_temp_file(path: &Path) -> bool {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    (name.starts_with(".odc-dl-") && name.ends_with(".part")) && path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn valid_archive_replaces_existing_docset_and_corrupt_archive_preserves_it() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let root = scratch.path();
        let old = root.join("Example.docset");
        let new = root.join("new");
        super::super::tests::write_docset(&old, "1.0");
        super::super::tests::write_docset(&new, "2.0");
        let package = root.join("Example.tgz");

        let entry = FeedEntry {
            id: "Example".into(),
            name: "Example".into(),
            version: "2.0".into(),
            urls: Vec::new(),
            devdocs: None,
        };
        let html = old.join("Contents/Resources/Documents/index.html");
        fs::write(&package, b"broken archive").unwrap();
        let failed = apply_archive(
            &entry,
            &package,
            &old,
            &root.join("invalid"),
            TarixLimits::default(),
            &|_| {},
        );
        assert!(failed.is_err());
        assert_eq!(fs::read_to_string(&html).unwrap(), "<h1>1.0</h1>");

        let encoder = flate2::write::GzEncoder::new(
            fs::File::create(&package).unwrap(),
            flate2::Compression::fast(),
        );
        let mut archive = tar::Builder::new(encoder);
        archive.append_dir_all("Example.docset", &new).unwrap();
        archive.into_inner().unwrap().finish().unwrap();
        apply_archive(
            &entry,
            &package,
            &old,
            &root.join("staging"),
            TarixLimits::default(),
            &|_| {},
        )
        .unwrap();
        assert_eq!(fs::read_to_string(&html).unwrap(), "<h1>2.0</h1>");
        assert!(!root.join("Example.docset.old").exists());
    }
}
