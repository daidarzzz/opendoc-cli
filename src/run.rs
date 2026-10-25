//! Implementación de los subcomandos.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use opendoc_core::browse::docset_url;
use opendoc_core::library::{self, Library, LoadOptions};
use opendoc_core::resolve::resolve_request;
use opendoc_core::search::{search, SearchIndex, SearchResult};

use crate::cli::{Cli, Command};
use crate::error::CliError;
use crate::paths::{self, Locations};
use crate::ui::{self, paint, truncate_middle, BOLD, DOCSET, KIND, MUTED, NAME};

/// Tope duro de resultados (no listar el índice entero por accidente).
const MAX_LIMIT: usize = 500;
/// Límite de candidatos a evaluar para el subcomando `open`.
const OPEN_CANDIDATES_LIMIT: usize = 25;
/// Ancho máximo de la columna de nombres.
const NAME_WIDTH: usize = 46;
/// Ancho máximo de la columna de tipos.
const KIND_WIDTH: usize = 14;
/// Ancho máximo de la columna de docsets.
const DOCSET_WIDTH: usize = 12;

/// Ejecuta el subcomando pedido.
pub fn run(cli: &Cli) -> Result<(), CliError> {
    let locations = paths::resolve(cli.docsets.as_deref(), cli.tarix_cache.as_deref())?;
    match &cli.command {
        Command::Search {
            query,
            docset,
            limit,
            json,
        } => search_command(
            &locations,
            &join_query(query),
            docset.as_deref(),
            *limit,
            *json,
        ),
        Command::Open {
            query,
            docset,
            non_interactive,
            exact,
            path,
        } => open_command(
            &locations,
            &join_query(query),
            docset.as_deref(),
            *non_interactive,
            *exact,
            *path,
        ),
        Command::List { json } => list_command(&locations, *json),
    }
}

/// Une las palabras del texto de búsqueda.
fn join_query(words: &[String]) -> String {
    words.join(" ")
}

/// Carga el inventario compartido. La carga nunca extrae tarix: reutiliza
/// las cachés que la app ya haya creado.
fn load(locations: &Locations) -> Result<Library, CliError> {
    if !locations.docsets.is_dir() {
        return Err(CliError::DocsetsMissing {
            path: locations.docsets.clone(),
        });
    }
    library::load_with_options(
        &locations.docsets,
        &LoadOptions::new(locations.tarix_cache.clone()),
    )
    .map_err(|e| CliError::Load(e.to_string()))
}

/// Resuelve `--docset` a un id (id, nombre o familia, sin distinguir caja).
fn filter_id(library: &Library, token: &str) -> Result<String, CliError> {
    let lower = token.to_lowercase();
    if let Some(docset) = library.docsets.iter().find(|d| {
        d.id == token
            || d.name.to_lowercase() == lower
            || d.platform.as_deref().map(str::to_lowercase).as_deref() == Some(lower.as_str())
    }) {
        return Ok(docset.id.clone());
    }
    if let Some(pending) = library
        .pending
        .iter()
        .find(|p| p.id == token || p.name.to_lowercase() == lower)
    {
        return Ok(pending.id.clone());
    }
    Err(CliError::UnknownDocset(token.to_string()))
}

/// Resuelve el objetivo de la búsqueda a partir de la sintaxis `docset:query` o del flag `--docset`.
///
/// Si no se proporcionó `--docset`, se inspecciona si `raw_query` contiene `:`.
/// Si el prefijo coincide con un docset cargado (id, nombre o plataforma),
/// se extrae como filtro y se busca el resto de la cadena.
/// Si no coincide, se mantiene la consulta original intacta.
pub fn resolve_query_target<'a>(
    library: &Library,
    raw_query: &'a str,
    explicit_docset: Option<&'a str>,
) -> (Option<String>, &'a str) {
    if let Some(docset) = explicit_docset {
        return (Some(docset.to_string()), raw_query);
    }

    if let Some((posible_docset, rest)) = raw_query.split_once(':') {
        let trimmed_docset = posible_docset.trim();
        let trimmed_rest = rest.trim_start();
        if !trimmed_docset.is_empty() && !trimmed_rest.is_empty() {
            if let Ok(id) = filter_id(library, trimmed_docset) {
                return (Some(id), trimmed_rest);
            }
        }
    }

    (None, raw_query)
}

/// Ejecuta la búsqueda y devuelve los aciertos ya filtrados.
fn run_search(
    library: &Library,
    query: &str,
    docset: Option<&str>,
    limit: usize,
) -> Result<Vec<SearchResult>, CliError> {
    let ids = match docset {
        Some(token) => {
            let id = filter_id(library, token)?;
            Some(vec![id])
        }
        None => None,
    };
    let limit = limit.clamp(1, MAX_LIMIT);
    let mut index = SearchIndex::build_for_search(library.entries.clone());
    Ok(search(&mut index, query, ids.as_deref(), limit))
}

/// `search`: imprime resultados (texto o JSON). Sin aciertos no es error.
fn search_command(
    locations: &Locations,
    raw_query: &str,
    explicit_docset: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<(), CliError> {
    let library = load(locations)?;
    let (docset, query) = resolve_query_target(&library, raw_query, explicit_docset);
    let results = run_search(&library, query, docset.as_deref(), limit)?;
    let mut out = String::new();
    if json {
        out.push_str(
            &serde_json::to_string_pretty(&results).map_err(|e| CliError::Load(e.to_string()))?,
        );
        out.push('\n');
    } else {
        out.push_str(&render_search(raw_query, &results, library.entries.len()));
    }
    write_stdout(&out)
}

/// `list`: docsets instalados y tarix pendientes.
fn list_command(locations: &Locations, json: bool) -> Result<(), CliError> {
    let library = load(locations)?;
    let mut counts: HashMap<&str, usize> = HashMap::new();
    for entry in &library.entries {
        *counts.entry(entry.docset_id.as_str()).or_default() += 1;
    }
    let mut out = String::new();
    if json {
        let view = ListView {
            docsets: library
                .docsets
                .iter()
                .map(|d| ListDocset {
                    id: &d.id,
                    name: &d.name,
                    platform: d.platform.as_deref(),
                    version: d.version.as_deref(),
                    entries: counts.get(d.id.as_str()).copied().unwrap_or(0),
                    home: d.home_path.as_deref(),
                })
                .collect(),
            pending: library
                .pending
                .iter()
                .map(|p| ListPending {
                    id: &p.id,
                    name: &p.name,
                })
                .collect(),
        };
        out.push_str(
            &serde_json::to_string_pretty(&view).map_err(|e| CliError::Load(e.to_string()))?,
        );
        out.push('\n');
    } else {
        out.push_str(&render_list(&library, &counts));
    }
    write_stdout(&out)
}

#[derive(Clone, Debug)]
struct OpenCandidate(SearchResult);

impl std::fmt::Display for OpenCandidate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let badge = ui::compact_kind_badge(&self.0.kind);
        write!(
            f,
            "{:<8} {:<36} [{}]",
            badge,
            truncate_middle(&self.0.name, 36),
            self.0.docset_id
        )
    }
}

fn launch_entry(
    library: &Library,
    hit: &SearchResult,
    print_path_only: bool,
) -> Result<(), CliError> {
    let Some(docset) = library.docset(&hit.docset_id) else {
        return Err(CliError::NoDocuments {
            id: hit.docset_id.clone(),
        });
    };
    let documents = docset.contents_path.join("Resources/Documents");
    if !documents.is_dir() {
        return Err(CliError::NoDocuments {
            id: docset.id.clone(),
        });
    }
    let resolved = resolve_request(&documents, &hit.path).map_err(|e| CliError::ResolveFailed {
        id: docset.id.clone(),
        path: hit.path.clone(),
        reason: e.to_string(),
    })?;
    if print_path_only {
        return write_stdout(&format!("{}\n", resolved.path.display()));
    }
    let url = docset_url(&hit.docset_id, &hit.path);
    let mut out = String::new();
    let badge = ui::compact_kind_badge(&hit.kind);
    out.push_str(&format!(
        "{} {}\n",
        paint(KIND, &badge),
        paint(NAME, &hit.name)
    ));
    out.push_str(&format!(
        "{} {}  {}\n",
        paint(MUTED, "docset:"),
        paint(DOCSET, &hit.docset_id),
        paint(MUTED, &url)
    ));
    out.push_str(&format!(
        "{} {}\n",
        paint(MUTED, "file:  "),
        resolved.path.display()
    ));
    write_stdout(&out)?;
    open_in_default(&resolved.path)
}

/// `open`: resuelve la coincidencia y la abre con el sistema (con selector interactivo si procede).
fn open_command(
    locations: &Locations,
    raw_query: &str,
    explicit_docset: Option<&str>,
    non_interactive: bool,
    exact: bool,
    print_path_only: bool,
) -> Result<(), CliError> {
    let library = load(locations)?;
    let (target_docset, query) = resolve_query_target(&library, raw_query, explicit_docset);
    let results = run_search(
        &library,
        query,
        target_docset.as_deref(),
        OPEN_CANDIDATES_LIMIT,
    )?;

    if results.is_empty() {
        return Err(CliError::NoMatch(raw_query.to_string()));
    }

    if results.len() == 1 {
        return launch_entry(&library, &results[0], print_path_only);
    }

    let is_tty = std::io::stdout().is_terminal();
    if print_path_only || non_interactive || exact || !is_tty {
        return launch_entry(&library, &results[0], print_path_only);
    }

    let options: Vec<OpenCandidate> = results.into_iter().map(OpenCandidate).collect();
    let prompt = "Select document to open:";
    let selection = inquire::Select::new(prompt, options)
        .with_vim_mode(true)
        .prompt();

    match selection {
        Ok(chosen) => launch_entry(&library, &chosen.0, print_path_only),
        Err(
            inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted,
        ) => Ok(()),
        Err(e) => Err(CliError::OpenFailed {
            path: PathBuf::from("<interactive>"),
            source: std::io::Error::other(e.to_string()),
        }),
    }
}

/// Imprime el resultado de búsqueda alineado con badges compactos.
fn render_search(query: &str, results: &[SearchResult], total_entries: usize) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        ui::heading("search"),
        paint(MUTED, &format!("{query:?}"))
    ));
    if results.is_empty() {
        out.push_str(&format!(
            "{}\n",
            paint(MUTED, "no results (the docsets are still indexed)")
        ));
        return out;
    }
    let badges: Vec<String> = results
        .iter()
        .map(|r| ui::compact_kind_badge(&r.kind))
        .collect();
    let name_w = results
        .iter()
        .map(|r| ui::visible_width(&r.name))
        .max()
        .unwrap_or(0)
        .min(NAME_WIDTH);
    let kind_w = badges
        .iter()
        .map(|b| ui::visible_width(b))
        .max()
        .unwrap_or(4)
        .clamp(4, KIND_WIDTH);
    out.push_str(&format!(
        "{} {} {}\n",
        paint(BOLD, &pad("NAME", name_w)),
        paint(BOLD, &pad("KIND", kind_w)),
        paint(BOLD, "DOCSET")
    ));
    for (r, badge) in results.iter().zip(&badges) {
        out.push_str(&format!(
            "{} {} {}\n",
            paint(NAME, &pad(&truncate_middle(&r.name, name_w), name_w)),
            paint(KIND, &pad(badge, kind_w)),
            paint(DOCSET, &truncate_middle(&r.docset_id, DOCSET_WIDTH))
        ));
    }
    out.push_str(&format!(
        "{}\n",
        paint(
            MUTED,
            &format!(
                "{} result(s) · {} entries indexed · use `open` to launch one",
                results.len(),
                total_entries
            )
        )
    ));
    out
}

/// Imprime el inventario alineado.
fn render_list(library: &Library, counts: &HashMap<&str, usize>) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "{} {}\n",
        ui::heading("docsets"),
        paint(MUTED, &format!("({})", library.docsets.len()))
    ));
    if library.docsets.is_empty() {
        out.push_str(&format!("  {}\n", paint(MUTED, "none installed")));
    }
    let id_w = library
        .docsets
        .iter()
        .map(|d| ui::visible_width(&d.id))
        .chain(library.pending.iter().map(|p| ui::visible_width(&p.id)))
        .max()
        .unwrap_or(0)
        .min(DOCSET_WIDTH);
    let name_w = library
        .docsets
        .iter()
        .map(|d| ui::visible_width(&d.name))
        .chain(library.pending.iter().map(|p| ui::visible_width(&p.name)))
        .max()
        .unwrap_or(0)
        .min(NAME_WIDTH);
    for docset in &library.docsets {
        let entries = counts.get(docset.id.as_str()).copied().unwrap_or(0);
        let version = docset.version.as_deref().unwrap_or("-");
        out.push_str(&format!(
            "  {} {} {} {}\n",
            paint(DOCSET, &pad(&truncate_middle(&docset.id, id_w), id_w)),
            paint(NAME, &pad(&truncate_middle(&docset.name, name_w), name_w)),
            paint(MUTED, &pad(version, 10)),
            paint(MUTED, &format!("{entries} entries"))
        ));
    }
    if !library.pending.is_empty() {
        out.push_str(&format!(
            "\n{} {}\n",
            ui::heading("pending tarix"),
            paint(MUTED, &format!("({})", library.pending.len()))
        ));
        for pending in &library.pending {
            out.push_str(&format!(
                "  {} {} {}\n",
                paint(DOCSET, &pad(&truncate_middle(&pending.id, id_w), id_w)),
                paint(NAME, &truncate_middle(&pending.name, name_w)),
                paint(MUTED, "(not extracted: open it once in OpenDoc)")
            ));
        }
    }
    out
}

/// Rellena a la derecha contando caracteres (no bytes).
fn pad(text: &str, width: usize) -> String {
    let len = ui::visible_width(text);
    if len >= width {
        text.to_string()
    } else {
        format!("{text}{}", " ".repeat(width - len))
    }
}

/// Escribe a stdout con `anstream` (elimina color si no es terminal).
/// Un `BrokenPipe` (p. ej. `| head`) no es un error: se ignora.
fn write_stdout(text: &str) -> Result<(), CliError> {
    let mut stdout = anstream::stdout();
    match stdout
        .write_all(text.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(CliError::OpenFailed {
            path: PathBuf::from("<stdout>"),
            source,
        }),
    }
}

/// Abre un fichero con la aplicación por defecto del sistema.
fn open_in_default(path: &Path) -> Result<(), CliError> {
    let result = open_command_for(path);
    result.map_err(|source| CliError::OpenFailed {
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(target_os = "windows")]
fn open_command_for(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    Command::new("cmd")
        .arg("/C")
        .arg("start")
        .arg("")
        .arg(path)
        .spawn()
        .map(|_| ())
}

#[cfg(target_os = "macos")]
fn open_command_for(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    Command::new("open").arg(path).spawn().map(|_| ())
}

#[cfg(all(unix, not(target_os = "macos")))]
fn open_command_for(path: &Path) -> std::io::Result<()> {
    use std::process::Command;
    Command::new("xdg-open").arg(path).spawn().map(|_| ())
}

/// Vista JSON de un docset instalado.
#[derive(Serialize)]
struct ListDocset<'a> {
    id: &'a str,
    name: &'a str,
    platform: Option<&'a str>,
    version: Option<&'a str>,
    entries: usize,
    home: Option<&'a str>,
}

/// Vista JSON de un tarix pendiente.
#[derive(Serialize)]
struct ListPending<'a> {
    id: &'a str,
    name: &'a str,
}

/// Vista JSON completa de `list`.
#[derive(Serialize)]
struct ListView<'a> {
    docsets: Vec<ListDocset<'a>>,
    pending: Vec<ListPending<'a>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendoc_core::docset::{Docset, PendingTarix};

    fn docset(id: &str, name: &str, platform: Option<&str>) -> Docset {
        Docset {
            id: id.to_string(),
            name: name.to_string(),
            platform: platform.map(str::to_string),
            version: None,
            bundle_id: None,
            home_path: Some("index.html".to_string()),
            icon: None,
            root_path: PathBuf::from("x"),
            contents_path: PathBuf::from("x/Contents"),
        }
    }

    fn library() -> Library {
        Library {
            source_dir: PathBuf::from("x"),
            docsets: vec![docset("css", "CSS", None), docset("a", "Alpha", Some("c"))],
            pending: vec![PendingTarix {
                id: "c".to_string(),
                name: "C++".to_string(),
                icon: None,
                root_path: PathBuf::from("x"),
            }],
            issues: Vec::new(),
            entries: vec![
                opendoc_core::docset::Entry {
                    docset_id: "css".to_string(),
                    name: "grid".to_string(),
                    kind: "Property".to_string(),
                    path: "grid.html".to_string(),
                },
                opendoc_core::docset::Entry {
                    docset_id: "css".to_string(),
                    name: "color".to_string(),
                    kind: "Property".to_string(),
                    path: "color.html".to_string(),
                },
            ],
        }
    }

    #[test]
    fn join_query_keeps_words() {
        assert_eq!(
            join_query(&["std".to_string(), "vector".to_string()]),
            "std vector"
        );
    }

    #[test]
    fn filter_resolves_id_name_platform_and_pending() {
        let lib = library();
        assert_eq!(filter_id(&lib, "css").expect("id"), "css");
        assert_eq!(filter_id(&lib, "Alpha").expect("name"), "a");
        assert_eq!(filter_id(&lib, "c").expect("platform"), "a");
        assert_eq!(filter_id(&lib, "c++").expect("pending"), "c");
        assert!(filter_id(&lib, "nope").is_err());
    }

    #[test]
    fn search_only_returns_requested_docset() {
        let lib = library();
        let all = run_search(&lib, "grid", None, 10).expect("buscar");
        assert_eq!(all.len(), 1);
        let filtered = run_search(&lib, "grid", Some("css"), 10).expect("buscar css");
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].docset_id, "css");
        assert!(run_search(&lib, "grid", Some("Alpha"), 10)
            .expect("buscar a")
            .is_empty());
    }

    #[test]
    fn render_search_has_header_and_rows() {
        let lib = library();
        let results = run_search(&lib, "grid", None, 10).expect("buscar");
        let out = render_search("grid", &results, lib.entries.len());
        assert!(out.contains("search"));
        assert!(out.contains("grid"));
        assert!(out.contains("DOCSET"));
        assert!(out.contains("entries indexed"));
    }

    #[test]
    fn render_list_shows_pending_section() {
        let lib = library();
        let mut counts = HashMap::new();
        counts.insert("css", 2usize);
        let out = render_list(&lib, &counts);
        assert!(out.contains("docsets"));
        assert!(out.contains("CSS"));
        assert!(out.contains("pending tarix"));
        assert!(out.contains("C++"));
        assert!(out.contains("2 entries"));
    }

    #[test]
    fn pad_counts_characters() {
        assert_eq!(pad("abc", 5), "abc  ");
        assert_eq!(pad("abcde", 5), "abcde");
        assert_eq!(ui::visible_width(&pad("ñ", 3)), 3);
    }

    #[test]
    fn resolve_query_target_supports_docset_prefix() {
        let lib = library();
        // Prefijo exacto id
        let (docset, query) = resolve_query_target(&lib, "css:grid", None);
        assert_eq!(docset.as_deref(), Some("css"));
        assert_eq!(query, "grid");

        // Mayúsculas/minúsculas y nombre
        let (docset, query) = resolve_query_target(&lib, "Alpha:grid", None);
        assert_eq!(docset.as_deref(), Some("a"));
        assert_eq!(query, "grid");

        // Sin coincidencia de docset (por ejemplo C++ std::vector) -> búsqueda normal
        let (docset, query) = resolve_query_target(&lib, "std::vector", None);
        assert_eq!(docset, None);
        assert_eq!(query, "std::vector");

        // Sin prefijo ':'
        let (docset, query) = resolve_query_target(&lib, "grid", None);
        assert_eq!(docset, None);
        assert_eq!(query, "grid");

        // Flag explícito tiene precedencia
        let (docset, query) = resolve_query_target(&lib, "css:grid", Some("a"));
        assert_eq!(docset.as_deref(), Some("a"));
        assert_eq!(query, "css:grid");

        // Sin término tras ':'
        let (docset, query) = resolve_query_target(&lib, "css:", None);
        assert_eq!(docset, None);
        assert_eq!(query, "css:");
    }

    #[test]
    fn render_search_uses_compact_badges() {
        let lib = library();
        let results = run_search(&lib, "grid", None, 10).expect("buscar");
        let out = render_search("grid", &results, lib.entries.len());
        assert!(out.contains("[prop]"));
    }

    #[test]
    fn open_candidate_displays_badge_name_and_docset() {
        let candidate = OpenCandidate(SearchResult {
            docset_id: "css".to_string(),
            name: "display".to_string(),
            kind: "Property".to_string(),
            path: "display.html".to_string(),
            score: 100,
        });
        let s = candidate.to_string();
        assert!(s.contains("[prop]"));
        assert!(s.contains("display"));
        assert!(s.contains("[css]"));
    }
}
