//! Reads Kapeli-compatible XML feeds from a GitHub repository.
//!
//! The repository tree locates XML files. Each feed supplies a version and
//! one or more archive URLs, while its filename stem supplies the feed ID.
//! Fetching and parsing are synchronous and belong on a blocking worker.

use serde::{Deserialize, Serialize};

/// `owner/repo` normalizado para la API de GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoRef {
    /// Dueño (`kapeli`).
    pub owner: String,
    /// Repo (`feeds`).
    pub name: String,
}

impl RepoRef {
    /// Ruta `owner/name` para URLs de API/raw.
    pub fn path(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

/// Error de red, API o parseo de feeds.
#[derive(Debug, thiserror::Error)]
pub enum FeedError {
    /// Fallo HTTP o de conexión.
    #[error("network error for {url}: {message}")]
    Network {
        /// URL que fallaba.
        url: String,
        /// Detalle.
        message: String,
    },
    /// La API devolvió un estado inesperado.
    #[error("GitHub API returned {status} for {url}")]
    Api {
        /// URL pedida.
        url: String,
        /// Código HTTP.
        status: u16,
    },
    /// XML ilegible o sin los campos necesarios.
    #[error("unreadable feed at {url}: {message}")]
    Parse {
        /// URL del XML.
        url: String,
        /// Detalle.
        message: String,
    },
    /// Demasiados ficheros (sanidad, no un repo de feeds normal).
    #[error("too many XML files in repo ({found}, max {max})")]
    TooManyFiles {
        /// Encontrados.
        found: usize,
        /// Tope.
        max: usize,
    },
}

/// Una entrada del catálogo (un docset descargable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeedEntry {
    /// Stem del XML (`Python_3`).
    pub id: String,
    /// Nombre visible (stem del XML).
    pub name: String,
    /// `<version>` del feed (comparación de strings, como Dash).
    pub version: String,
    /// URLs del `.tgz` (mirrors). En DevDocs, la URL de `db.json`.
    pub urls: Vec<String>,
    /// Metadatos si viene de DevDocs (se convierte a docset al instalar);
    /// `None` en los feeds estilo Kapeli.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub devdocs: Option<opendoc_core::devdocs::DevDocsMeta>,
}

/// `<entry>` Kapeli: solo interesa `version` + `url` (el resto se ignora).
#[derive(Debug, Deserialize)]
struct KapeliEntry {
    /// Versión del docset.
    #[serde(default)]
    version: Option<String>,
    /// Una o varias URLs (mirrors).
    #[serde(default)]
    url: Vec<String>,
}

/// Tope de XML por repo (sanidad antes de lanzar cientos de peticiones).
pub const MAX_FEED_FILES: usize = 3000;
/// Hilos de descarga de XML en paralelo.
const FETCH_THREADS: usize = 8;
/// Timeout por petición HTTP.
const REQUEST_TIMEOUT_SECS: u64 = 30;
/// User-Agent exigido por la API de GitHub.
const USER_AGENT: &str = "OpenDoc CLI";

/// Acepta `owner/repo`, URLs `github.com/owner/repo[.git][/...]` y
/// `http(s)://...`. Recorta espacios; vacío o sin dos segmentos → error.
pub fn normalize_repo(raw: &str) -> Result<RepoRef, FeedError> {
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(FeedError::Parse {
            url: raw.to_string(),
            message: "empty repository (use owner/repo or a GitHub URL)".to_string(),
        });
    }
    // Quita esquema y posible `.git` final.
    let no_scheme = trimmed
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let no_git = no_scheme.trim_end_matches(".git");
    let after_host = if no_git.starts_with("github.com/") {
        no_git.trim_start_matches("github.com/")
    } else if no_git.starts_with("www.github.com/") {
        no_git.trim_start_matches("www.github.com/")
    } else {
        no_git
    };
    let mut parts = after_host.split('/').filter(|s| !s.is_empty());
    match (parts.next(), parts.next(), parts.next()) {
        (Some(owner), Some(name), None)
            if owner
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !matches!(name, "." | "..")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')) =>
        {
            Ok(RepoRef {
                owner: owner.to_string(),
                name: name.to_string(),
            })
        }
        _ => Err(FeedError::Parse {
            url: raw.to_string(),
            message: "invalid repository (use owner/repo or a GitHub URL)".to_string(),
        }),
    }
}

/// Cliente HTTP compartido (timeouts + User-Agent).
pub fn http_client() -> Result<reqwest::blocking::Client, FeedError> {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|e| FeedError::Network {
            url: "client".to_string(),
            message: e.to_string(),
        })
}

/// Descarga el catálogo completo: rama por defecto → árbol (`*.xml`) →
/// cada XML en paralelo. Devuelve entradas ordenadas por nombre y cuántos
/// XML se saltaron (ilegibles o sin versión/URLs).
pub fn fetch_catalog(
    client: &reqwest::blocking::Client,
    repo: &RepoRef,
) -> Result<(Vec<FeedEntry>, usize), FeedError> {
    let branch = default_branch(client, repo)?;
    let files = xml_files(client, repo, &branch)?;
    if files.len() > MAX_FEED_FILES {
        return Err(FeedError::TooManyFiles {
            found: files.len(),
            max: MAX_FEED_FILES,
        });
    }
    let mut entries: Vec<FeedEntry> = Vec::new();
    let mut skipped = 0_usize;
    let chunk = files.len().div_ceil(FETCH_THREADS).max(1);
    std::thread::scope(|s| {
        let mut handles = Vec::new();
        for part in files.chunks(chunk) {
            let urls: Vec<String> = part
                .iter()
                .map(|name| {
                    format!(
                        "https://raw.githubusercontent.com/{}/{}/{name}",
                        repo.path(),
                        branch
                    )
                })
                .collect();
            handles.push(s.spawn(move || {
                let mut local = Vec::new();
                let mut missed = 0_usize;
                for url in urls {
                    match fetch_entry(client, &url) {
                        Ok(Some(entry)) => local.push(entry),
                        Ok(None) | Err(_) => missed += 1,
                    }
                }
                (local, missed)
            }));
        }
        for h in handles {
            match h.join() {
                Ok((mut local, missed)) => {
                    entries.append(&mut local);
                    skipped += missed;
                }
                Err(_) => skipped += 1,
            }
        }
    });
    entries.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok((entries, skipped))
}

/// Rama por defecto del repo (también valida que existe).
fn default_branch(client: &reqwest::blocking::Client, repo: &RepoRef) -> Result<String, FeedError> {
    let url = format!("https://api.github.com/repos/{}", repo.path());
    let res = client.get(&url).send().map_err(|e| FeedError::Network {
        url: url.clone(),
        message: e.to_string(),
    })?;
    if !res.status().is_success() {
        return Err(FeedError::Api {
            url,
            status: res.status().as_u16(),
        });
    }
    let body: serde_json::Value = res.json().map_err(|e| FeedError::Parse {
        url: url.clone(),
        message: e.to_string(),
    })?;
    body.get("default_branch")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| FeedError::Parse {
            url,
            message: "missing default_branch".to_string(),
        })
}

/// Rutas `*.xml` del árbol (recursivo).
fn xml_files(
    client: &reqwest::blocking::Client,
    repo: &RepoRef,
    branch: &str,
) -> Result<Vec<String>, FeedError> {
    let url = format!(
        "https://api.github.com/repos/{}/git/trees/{}?recursive=1",
        repo.path(),
        branch
    );
    let res = client.get(&url).send().map_err(|e| FeedError::Network {
        url: url.clone(),
        message: e.to_string(),
    })?;
    if !res.status().is_success() {
        return Err(FeedError::Api {
            url,
            status: res.status().as_u16(),
        });
    }
    let body: serde_json::Value = res.json().map_err(|e| FeedError::Parse {
        url: url.clone(),
        message: e.to_string(),
    })?;
    let tree = body
        .get("tree")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| FeedError::Parse {
            url: url.clone(),
            message: "missing tree".to_string(),
        })?;
    Ok(tree
        .iter()
        .filter_map(|n| n.get("path").and_then(serde_json::Value::as_str))
        .filter(|p| p.ends_with(".xml"))
        .map(str::to_string)
        .collect())
}

/// Un XML → `Some(entry)`; `None` si no trae versión o URLs (no sirve para
/// detectar actualizaciones). Errores de red/parseo suben como `Err`.
fn fetch_entry(
    client: &reqwest::blocking::Client,
    url: &str,
) -> Result<Option<FeedEntry>, FeedError> {
    let res = client.get(url).send().map_err(|e| FeedError::Network {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    if !res.status().is_success() {
        return Err(FeedError::Api {
            url: url.to_string(),
            status: res.status().as_u16(),
        });
    }
    let text = res.text().map_err(|e| FeedError::Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    let entry: KapeliEntry = quick_xml::de::from_str(&text).map_err(|e| FeedError::Parse {
        url: url.to_string(),
        message: e.to_string(),
    })?;
    let stem = url
        .rsplit('/')
        .next()
        .unwrap_or(url)
        .trim_end_matches(".xml");
    match (entry.version, entry.url) {
        (Some(version), urls) if !version.is_empty() && !urls.is_empty() => Ok(Some(FeedEntry {
            id: stem.to_string(),
            name: stem.to_string(),
            version,
            urls,
            devdocs: None,
        })),
        _ => Ok(None),
    }
}
