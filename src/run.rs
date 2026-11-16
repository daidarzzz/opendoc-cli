//! Implementación de los subcomandos.

use std::collections::HashMap;
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde::Serialize;

use opendoc_core::browse::docset_url;
use opendoc_core::library::{self, Library, LoadOptions};
use opendoc_core::resolve::resolve_request;
use opendoc_core::search::{search, SearchIndex, SearchResult};

use crate::cli::{Cli, Command, DocsetCommand};
use crate::clip;
use crate::context;
use crate::docsets::{self, Action, Manager};
use crate::error::CliError;
use crate::pager;
use crate::paths::{self, Locations};
use crate::query::{self, ParsedQuery};
use crate::render;
use crate::tui;
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
    if let Some(Command::Update { check }) = &cli.command {
        return crate::update::run(*check);
    }
    let locations = paths::resolve(cli.docsets.as_deref(), cli.tarix_cache.as_deref())?;
    match &cli.command {
        Some(Command::Search {
            query,
            docset,
            limit,
            json,
        }) => search_command(
            &locations,
            &join_query(query),
            docset.as_deref(),
            *limit,
            *json,
            cli.no_context,
        ),
        Some(Command::Show {
            query,
            docset,
            code,
            raw,
            copy,
            json,
            width,
        }) => show_command(
            &locations,
            &join_query(query),
            docset.as_deref(),
            ShowOpts {
                only_code: *code,
                raw: *raw,
                copy: *copy,
                json: *json,
                width: *width,
                no_context: cli.no_context,
            },
        ),
        Some(Command::Open {
            query,
            docset,
            non_interactive,
            exact,
            path,
        }) => open_command(
            &locations,
            &join_query(query),
            docset.as_deref(),
            *non_interactive,
            *exact,
            *path,
            cli.no_context,
        ),
        Some(Command::Tui { docset, query }) => tui_command(
            &locations,
            docset.as_deref(),
            query.as_deref(),
            cli.no_context,
        ),
        Some(Command::Completions { shell }) => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            clap_complete::generate(*shell, &mut cmd, "odc", &mut std::io::stdout());
            Ok(())
        }
        Some(Command::Docsets { command }) => docsets_command(&locations, command),
        Some(Command::Update { .. }) => unreachable!("update is dispatched before path resolution"),
        Some(Command::List { json }) => list_command(&locations, *json),
        None => {
            if std::io::stdout().is_terminal() {
                tui_command(&locations, None, None, cli.no_context)
            } else {
                use clap::CommandFactory;
                let mut cmd = Cli::command();
                cmd.print_help()
                    .map_err(|e| CliError::Load(e.to_string()))?;
                println!();
                Ok(())
            }
        }
    }
}

fn docsets_command(locations: &Locations, command: &DocsetCommand) -> Result<(), CliError> {
    use crate::cli::FeedCommand;

    let manager = Manager::new(locations)?;
    let last_progress = std::sync::Mutex::new(None);
    let progress = |report: docsets::ProgressReport| {
        if !std::io::stderr().is_terminal() {
            return;
        }
        if let Ok(mut last) = last_progress.lock() {
            let now = std::time::Instant::now();
            if last.is_none_or(|(when, stage)| {
                stage != report.stage || now.duration_since(when).as_millis() >= 250
            }) {
                eprintln!("{report}");
                *last = Some((now, report.stage));
            }
        }
    };
    match command {
        DocsetCommand::Catalog { query } => {
            let (mut rows, ready) = manager.rows()?;
            if !ready {
                match manager.refresh() {
                    Ok(message) => {
                        eprintln!("{message}");
                        (rows, _) = manager.rows()?;
                    }
                    Err(e) if !rows.is_empty() => eprintln!("Catalog unavailable: {e}"),
                    Err(e) => return Err(e),
                }
            }
            let rows: Vec<_> = rows
                .iter()
                .filter(|r| r.matches(query.as_deref().unwrap_or("")))
                .collect();
            if rows.is_empty() {
                println!("No matching docsets.");
            }
            for row in rows {
                println!("{}", row.description());
            }
        }
        DocsetCommand::Refresh => println!("{}", manager.refresh()?),
        DocsetCommand::Feeds { command } => {
            match command {
                Some(FeedCommand::Set { repos }) => manager.set_feeds(&repos.join(" "))?,
                Some(FeedCommand::Reset) => manager.reset_feeds()?,
                None => {}
            }
            println!("DevDocs");
            for repo in manager.feeds()? {
                println!("{}", repo.path());
            }
        }
        DocsetCommand::Install { id } => {
            if !manager.has_catalog()? {
                eprintln!("{}", manager.refresh()?);
            }
            println!("{}", manager.change(Action::Install, id, &progress)?);
        }
        DocsetCommand::Update { id, force } => {
            if !manager.has_catalog()? {
                eprintln!("{}", manager.refresh()?);
            }
            let action = if *force {
                Action::Reinstall
            } else {
                Action::Update
            };
            println!("{}", manager.change(action, id, &progress)?);
        }
        DocsetCommand::Uninstall { id, yes } => {
            if !yes {
                if !std::io::stdin().is_terminal() || !std::io::stderr().is_terminal() {
                    return Err(docsets::error("pass --yes to uninstall without a terminal"));
                }
                eprint!("Uninstall {id}? [y/N] ");
                std::io::stderr().flush().map_err(docsets::error)?;
                let mut answer = String::new();
                std::io::stdin()
                    .read_line(&mut answer)
                    .map_err(docsets::error)?;
                if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
                    println!("Uninstall cancelled.");
                    return Ok(());
                }
            }
            println!("{}", manager.change(Action::Uninstall, id, &progress)?);
        }
    }
    Ok(())
}

/// Une las palabras del texto de búsqueda.
fn join_query(words: &[String]) -> String {
    words.join(" ")
}

/// Carga el inventario compartido. La carga nunca extrae tarix: reutiliza
/// las cachés que la app ya haya creado.
pub(crate) fn load(locations: &Locations) -> Result<Library, CliError> {
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

/// Interpreta la consulta: prefijos `docset:` y `tipo:` (ver `query`).
pub fn interpret(library: &Library, raw_query: &str, explicit_docset: Option<&str>) -> ParsedQuery {
    let kinds = query::known_kinds(library);
    query::parse(library, &kinds, raw_query, explicit_docset)
}

/// Ids de docsets que encajan con el proyecto del directorio actual.
fn project_docsets(library: &Library, no_context: bool) -> Vec<String> {
    if no_context {
        return Vec::new();
    }
    std::env::current_dir()
        .map(|cwd| context::context_docsets(library, &cwd))
        .unwrap_or_default()
}

/// Búsqueda completa: docset, tipo y prioridad por contexto de proyecto.
fn run_parsed(
    library: &Library,
    parsed: &ParsedQuery,
    limit: usize,
    no_context: bool,
) -> Result<Vec<SearchResult>, CliError> {
    let fetch = if parsed.kind.is_some() {
        MAX_LIMIT * 4
    } else {
        limit.clamp(1, MAX_LIMIT) * 2
    };
    let mut results = run_search(library, &parsed.text, parsed.docset.as_deref(), fetch)?;
    if let Some(k) = &parsed.kind {
        results.retain(|r| &render::sym_kind(&r.kind, &r.path) == k);
    }
    if parsed.docset.is_none() {
        let ctx = project_docsets(library, no_context);
        if !ctx.is_empty() {
            // Partición estable: el orden relativo se conserva.
            let (mut first, rest): (Vec<_>, Vec<_>) = results
                .into_iter()
                .partition(|r| ctx.contains(&r.docset_id));
            first.extend(rest);
            results = first;
        }
    }
    results.truncate(limit.clamp(1, MAX_LIMIT));
    Ok(results)
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
    no_context: bool,
) -> Result<(), CliError> {
    let library = load(locations)?;
    let parsed = interpret(&library, raw_query, explicit_docset);
    let results = run_parsed(&library, &parsed, limit, no_context)?;
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
    let badge = format!("[{}]", render::sym_kind(&hit.kind, &hit.path));
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
    no_context: bool,
) -> Result<(), CliError> {
    let library = load(locations)?;
    let parsed = interpret(&library, raw_query, explicit_docset);
    let results = run_parsed(&library, &parsed, OPEN_CANDIDATES_LIMIT, no_context)?;

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

#[derive(Serialize)]
struct ShowJsonView<'a> {
    /// Versión del contrato JSON.
    version: u8,
    name: &'a str,
    kind: &'a str,
    docset: &'a str,
    signature: Option<&'a str>,
    file_path: &'a Path,
    code_blocks: &'a [String],
    content: &'a str,
}

/// Opciones de `show`.
struct ShowOpts {
    only_code: bool,
    raw: bool,
    copy: bool,
    json: bool,
    width: Option<usize>,
    no_context: bool,
}

/// `show` / `cat`: extrae y muestra la documentación directamente en la terminal.
fn show_command(
    locations: &Locations,
    raw_query: &str,
    explicit_docset: Option<&str>,
    opts: ShowOpts,
) -> Result<(), CliError> {
    let library = load(locations)?;
    let parsed = interpret(&library, raw_query, explicit_docset);
    let results = run_parsed(&library, &parsed, OPEN_CANDIDATES_LIMIT, opts.no_context)?;

    if results.is_empty() {
        return Err(CliError::NoMatch(raw_query.to_string()));
    }

    // Coincidencia exacta primero (en el último segmento), o la mejor del ranking.
    let q = parsed.text.trim();
    let hit = results
        .iter()
        .find(|r| r.name.eq_ignore_ascii_case(q))
        .unwrap_or(&results[0]);

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
    let file_part = hit.path.split('#').next().unwrap_or(&hit.path);
    let resolved = resolve_request(&documents, file_part).map_err(|e| CliError::ResolveFailed {
        id: docset.id.clone(),
        path: hit.path.clone(),
        reason: e.to_string(),
    })?;
    let html_content =
        std::fs::read_to_string(&resolved.path).map_err(|e| CliError::ReadFailed {
            path: resolved.path.clone(),
            source: e,
        })?;

    let anchor = hit.path.split_once('#').map(|(_, a)| a);
    let sk = render::sym_kind(&hit.kind, &hit.path);
    let doc = render::parse_doc(&html_content, anchor, &hit.name, &sk, &hit.docset_id);
    let width = opts
        .width
        .or_else(|| crossterm::terminal::size().ok().map(|(w, _)| w as usize))
        .unwrap_or(90)
        .clamp(30, 110);
    let page_err = |e: std::io::Error| CliError::ReadFailed {
        path: resolved.path.clone(),
        source: e,
    };

    if opts.copy {
        let text = if opts.only_code {
            doc.code_blocks.join("\n\n")
        } else {
            doc.plain_text(width)
        };
        let via = clip::copy(&text).map_err(CliError::Clipboard)?;
        write_stdout(&format!(
            "{} Copied {} [{}] to clipboard ({via}).\n",
            ui::paint(ui::ACCENT, "✓"),
            hit.name,
            hit.docset_id
        ))?;
        return Ok(());
    }

    if opts.json {
        let content = doc.plain_text(width);
        let view = ShowJsonView {
            version: 1,
            name: &hit.name,
            kind: &hit.kind,
            docset: &hit.docset_id,
            signature: doc.signature.as_deref(),
            file_path: &resolved.path,
            code_blocks: &doc.code_blocks,
            content: &content,
        };
        let json_str =
            serde_json::to_string_pretty(&view).map_err(|e| CliError::Load(e.to_string()))?;
        return write_stdout(&format!("{json_str}\n"));
    }

    if opts.only_code {
        let mut blocks: Vec<&String> = doc.code_blocks.iter().collect();
        if blocks.is_empty() {
            if let Some(sig) = doc.signature.as_ref() {
                blocks.push(sig);
            } else {
                return write_stdout(&format!(
                    "{} No code examples found for {}.\n",
                    ui::paint(ui::MUTED, "●"),
                    hit.name
                ));
            }
        }
        let mut out = String::new();
        for b in blocks {
            out.push_str(&render::highlight_ansi(b, &doc.lang));
            out.push('\n');
        }
        return pager::page_output(&out).map_err(page_err);
    }

    if opts.raw {
        return write_stdout(&doc.plain_text(width));
    }

    let layout = doc.layout(width);
    pager::page_output(&render::to_ansi(&layout.lines)).map_err(page_err)
}

/// `tui`: lanza la interfaz interactiva.
fn tui_command(
    locations: &Locations,
    explicit_docset: Option<&str>,
    initial_query: Option<&str>,
    no_context: bool,
) -> Result<(), CliError> {
    let library = match load(locations) {
        Err(CliError::DocsetsMissing { .. }) if !locations.docsets.exists() => Library {
            source_dir: locations.docsets.clone(),
            docsets: Vec::new(),
            pending: Vec::new(),
            issues: Vec::new(),
            entries: Vec::new(),
        },
        result => result?,
    };
    let initial_id = match explicit_docset {
        Some(token) => Some(filter_id(&library, token)?),
        // Sin docset explícito: si el proyecto actual tiene un único docset
        // evidente (Cargo.toml → Rust…), se empieza filtrando por él.
        None => {
            let ctx = project_docsets(&library, no_context);
            if ctx.len() == 1 {
                ctx.into_iter().next()
            } else {
                None
            }
        }
    };
    tui::run_tui(
        library,
        initial_id,
        initial_query.map(str::to_string),
        Manager::new(locations)?,
    )
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
        .map(|r| format!("[{}]", render::sym_kind(&r.kind, &r.path)))
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
pub(crate) fn open_in_default(path: &Path) -> Result<(), CliError> {
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
    fn interpret_supports_docset_prefix() {
        let lib = library();
        let p = interpret(&lib, "css:grid", None);
        assert_eq!(p.docset.as_deref(), Some("css"));
        assert_eq!(p.text, "grid");
        let p = interpret(&lib, "Alpha:grid", None);
        assert_eq!(p.docset.as_deref(), Some("a"));
        let p = interpret(&lib, "std::vector", None);
        assert_eq!(p.docset, None);
        assert_eq!(p.text, "std::vector");
        let p = interpret(&lib, "prop:grid", None);
        assert_eq!(p.kind.as_deref(), Some("prop"));
        assert_eq!(p.text, "grid");
        let p = interpret(&lib, "css:grid", Some("a"));
        assert_eq!(p.docset.as_deref(), Some("a"));
        assert_eq!(p.text, "css:grid");
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
