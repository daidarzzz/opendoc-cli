//! TUI de pantalla completa: búsqueda difusa en vivo, lector de
//! documentación con scroll, enlaces navegables e historial.
//!
//! Dos zonas de foco: la **búsqueda** (escribes, ↑/↓ mueven la lista) y
//! el **lector** (j/k, saltar secciones, seguir enlaces). La vista previa
//! se carga solo cuando el teclado queda en reposo para que mantener
//! pulsada una tecla nunca bloquee la interfaz.

use std::collections::HashMap;
use std::io::stdout;
use std::path::{Component, Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
    KeyModifiers, MouseButton, MouseEventKind,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use opendoc_core::library::Library;
use opendoc_core::resolve::resolve_request;
use opendoc_core::search::{search, SearchIndex, SearchResult};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line as RLine, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
    ScrollbarOrientation, ScrollbarState,
};
use ratatui::Terminal;
use unicode_width::UnicodeWidthStr;

use crate::clip;
use crate::error::CliError;
use crate::query;
use crate::render::{self, Doc, Layout as DocLayout, Seg};
use crate::state::{Item, State};
use crate::ui;

const READ_WIDTH: usize = 100;
const MAX_RESULTS: usize = 300;
const HISTORY_MAX: usize = 64;

fn rgb(c: render::Rgb) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

fn seg_style(s: &render::Sty) -> Style {
    let mut st = Style::default();
    if let Some(c) = s.fg {
        st = st.fg(rgb(c));
    }
    if let Some(c) = s.bg {
        st = st.bg(rgb(c));
    }
    if s.bold {
        st = st.add_modifier(Modifier::BOLD);
    }
    if s.italic {
        st = st.add_modifier(Modifier::ITALIC);
    }
    if s.underline {
        st = st.add_modifier(Modifier::UNDERLINED);
    }
    if s.dim {
        st = st.add_modifier(Modifier::DIM);
    }
    st
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Search,
    Reader,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PickKind {
    Docset,
    Kind,
    Outline,
    Recent,
    Related,
}

struct Picker {
    kind: PickKind,
    title: String,
    /// (etiqueta visible, valor; `None` = «todos»).
    items: Vec<(String, Option<String>)>,
    filter: String,
    sel: usize,
}

impl Picker {
    fn visible(&self) -> Vec<&(String, Option<String>)> {
        let f = self.filter.to_lowercase();
        self.items
            .iter()
            .filter(|(l, _)| f.is_empty() || l.to_lowercase().contains(&f))
            .collect()
    }
}

enum Overlay {
    None,
    Help,
    Pick(Picker),
}

/// Lo que se está leyendo.
#[derive(Clone)]
struct View {
    docset: String,
    path: String,
    name: String,
    doc: Rc<Doc>,
    scroll: usize,
    link: Option<usize>,
}

struct App<'a> {
    library: &'a Library,
    index: SearchIndex,
    kinds: std::collections::HashSet<String>,
    query: String,
    cursor: usize,
    docset: Option<String>,
    kind: Option<String>,
    results: Vec<SearchResult>,
    list: ListState,
    view: Option<View>,
    history: Vec<View>,
    cache: HashMap<(String, String), Rc<Doc>>,
    layout_cache: Option<(Rc<Doc>, usize, Rc<DocLayout>)>,
    entry_index: Option<HashMap<(String, String), (String, String)>>,
    focus: Focus,
    overlay: Overlay,
    status: Option<(String, Instant, bool)>,
    pending_load: bool,
    quit: bool,
    // Geometría de la última pasada de dibujo (para ratón y paginación).
    list_area: Rect,
    reader_area: Rect,
    reader_height: usize,
    narrow: bool,
    /// Selección con el ratón en coordenadas del documento (línea, columna).
    sel: Option<Selection>,
    state: State,
    find: Option<Find>,
    pending_g: bool,
    last_click: Option<(Instant, usize, usize, u8)>,
}

/// Búsqueda dentro de la página.
struct Find {
    input: String,
    editing: bool,
    /// (línea, columna inicial, columna final exclusiva).
    matches: Vec<(usize, usize, usize)>,
    cur: usize,
    /// Documento y ancho para los que se calcularon las coincidencias.
    stamp: Option<(Rc<Doc>, usize)>,
}

fn fold(c: char, smart: bool) -> char {
    if smart {
        c
    } else {
        c.to_lowercase().next().unwrap_or(c)
    }
}

/// Coincidencias de `q` (minúsculas/mayúsculas inteligentes) en el layout.
fn compute_matches(lay: &DocLayout, q: &str) -> Vec<(usize, usize, usize)> {
    let smart = q.chars().any(|c| c.is_uppercase());
    let qc: Vec<char> = q.chars().map(|c| fold(c, smart)).collect();
    let mut out = Vec::new();
    if qc.is_empty() {
        return out;
    }
    for li in 0..lay.lines.len() {
        let chars = line_text(lay, li);
        let folded: Vec<char> = chars.iter().map(|(c, _)| fold(*c, smart)).collect();
        let mut i = 0;
        while i + qc.len() <= folded.len() {
            if folded[i..i + qc.len()] == qc[..] {
                let (last, lcol) = chars[i + qc.len() - 1];
                let w = unicode_width::UnicodeWidthChar::width(last).unwrap_or(1);
                out.push((li, chars[i].1, lcol + w));
                i += qc.len();
                if out.len() >= 5000 {
                    return out;
                }
            } else {
                i += 1;
            }
        }
    }
    out
}

fn enc(it: &Item) -> String {
    format!("{}\x1f{}\x1f{}\x1f{}", it.docset, it.path, it.name, it.kind)
}

fn dec(s: &str) -> Option<Item> {
    let mut p = s.split('\x1f');
    Some(Item {
        docset: p.next()?.to_string(),
        path: p.next()?.to_string(),
        name: p.next()?.to_string(),
        kind: p.next()?.to_string(),
    })
}

#[derive(Clone, Copy)]
struct Selection {
    anchor: (usize, usize),
    head: (usize, usize),
    dragging: bool,
}

impl Selection {
    /// Extremos ordenados (inicio, fin).
    fn ordered(&self) -> ((usize, usize), (usize, usize)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }

    /// Columnas `[desde, hasta)` seleccionadas en `line`.
    fn range_on(&self, line: usize) -> Option<(usize, usize)> {
        let (a, b) = self.ordered();
        if line < a.0 || line > b.0 {
            return None;
        }
        let from = if line == a.0 { a.1 } else { 0 };
        let to = if line == b.0 { b.1 + 1 } else { usize::MAX };
        Some((from, to))
    }
}

impl<'a> App<'a> {
    fn new(
        library: &'a Library,
        docset: Option<String>,
        kind: Option<String>,
        query: Option<String>,
    ) -> Self {
        let q = query.unwrap_or_default();
        let mut app = Self {
            library,
            index: SearchIndex::build_for_search(library.entries.clone()),
            kinds: query::known_kinds(library),
            cursor: q.chars().count(),
            query: q,
            docset,
            kind,
            results: Vec::new(),
            list: ListState::default(),
            view: None,
            history: Vec::new(),
            cache: HashMap::new(),
            layout_cache: None,
            entry_index: None,
            focus: Focus::Search,
            overlay: Overlay::None,
            status: None,
            pending_load: false,
            quit: false,
            list_area: Rect::default(),
            reader_area: Rect::default(),
            reader_height: 20,
            narrow: false,
            sel: None,
            state: State::load(),
            find: None,
            pending_g: false,
            last_click: None,
        };
        app.refresh();
        app
    }

    fn say(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), false));
    }
    fn warn(&mut self, msg: impl Into<String>) {
        self.status = Some((msg.into(), Instant::now(), true));
    }

    // ───────── búsqueda ─────────

    fn refresh(&mut self) {
        let parsed = query::parse(self.library, &self.kinds, &self.query, None);
        let docset = parsed.docset.or_else(|| self.docset.clone());
        let kind = parsed.kind.or_else(|| self.kind.clone());
        let ids = docset.map(|d| vec![d]);
        let text = parsed.text.trim().to_string();

        let mut results: Vec<SearchResult> = if text.is_empty() {
            self.library
                .entries
                .iter()
                .filter(|e| ids.as_ref().is_none_or(|v| v.contains(&e.docset_id)))
                .filter(|e| {
                    kind.as_ref()
                        .is_none_or(|k| &render::sym_kind(&e.kind, &e.path) == k)
                })
                .take(MAX_RESULTS)
                .map(|e| SearchResult {
                    name: e.name.clone(),
                    path: e.path.clone(),
                    kind: e.kind.clone(),
                    docset_id: e.docset_id.clone(),
                    score: 0,
                })
                .collect()
        } else {
            let limit = if kind.is_some() { 2000 } else { MAX_RESULTS };
            let mut r = search(&mut self.index, &text, ids.as_deref(), limit);
            if let Some(k) = &kind {
                r.retain(|h| &render::sym_kind(&h.kind, &h.path) == k);
            }
            r.truncate(MAX_RESULTS);
            r
        };
        results.shrink_to_fit();
        self.results = results;
        self.list.select(if self.results.is_empty() {
            None
        } else {
            Some(0)
        });
        self.queue_load();
    }

    fn queue_load(&mut self) {
        self.pending_load = true;
    }

    fn selected_hit(&self) -> Option<&SearchResult> {
        self.list.selected().and_then(|i| self.results.get(i))
    }

    fn move_sel(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let cur = self.list.selected().unwrap_or(0) as isize;
        let next = (cur + delta).clamp(0, self.results.len() as isize - 1) as usize;
        if Some(next) != self.list.selected() {
            self.list.select(Some(next));
            self.queue_load();
        }
    }

    // ───────── carga de documentos ─────────

    fn load_doc(&mut self, docset_id: &str, path: &str, name: &str, kind: &str) -> Option<Rc<Doc>> {
        let key = (docset_id.to_string(), path.to_string());
        if let Some(d) = self.cache.get(&key) {
            return Some(d.clone());
        }
        let docset = self.library.docset(docset_id)?;
        let documents = docset.contents_path.join("Resources/Documents");
        let (file, anchor) = match path.split_once('#') {
            Some((f, a)) => (f, Some(a)),
            None => (path, None),
        };
        let resolved = resolve_request(&documents, file).ok()?;
        let html = std::fs::read_to_string(&resolved.path).ok()?;
        let doc = Rc::new(render::parse_doc(&html, anchor, name, kind, docset_id));
        if self.cache.len() >= 48 {
            self.cache.clear();
        }
        self.cache.insert(key, doc.clone());
        Some(doc)
    }

    fn load_selected(&mut self) {
        self.pending_load = false;
        let Some(hit) = self.selected_hit().cloned() else {
            self.view = None;
            self.history.clear();
            return;
        };
        let sk = render::sym_kind(&hit.kind, &hit.path);
        match self.load_doc(&hit.docset_id, &hit.path, &hit.name, &sk) {
            Some(doc) => {
                self.find = None;
                self.view = Some(View {
                    docset: hit.docset_id,
                    path: hit.path,
                    name: hit.name,
                    doc,
                    scroll: 0,
                    link: None,
                });
                self.history.clear();
            }
            None => {
                self.view = None;
                self.warn(format!(
                    "No se pudo leer «{}» (¿tarix sin extraer?)",
                    hit.name
                ));
            }
        }
    }

    fn layout_for(&mut self, width: usize) -> Option<Rc<DocLayout>> {
        let view = self.view.as_ref()?;
        if let Some((doc, w, l)) = &self.layout_cache {
            // `ptr_eq` sobre un Rc retenido: la dirección no se puede reutilizar.
            if Rc::ptr_eq(doc, &view.doc) && *w == width {
                return Some(l.clone());
            }
        }
        let l = Rc::new(view.doc.layout(width));
        self.layout_cache = Some((view.doc.clone(), width, l.clone()));
        Some(l)
    }

    fn content_width(&self) -> usize {
        (self.reader_area.width.saturating_sub(5) as usize).clamp(24, READ_WIDTH)
    }

    // ───────── navegación en el lector ─────────

    fn scroll_by(&mut self, delta: isize) {
        let w = self.content_width();
        let Some(lay) = self.layout_for(w) else {
            return;
        };
        let max = lay.lines.len().saturating_sub(self.reader_height.max(1)) as isize;
        if let Some(v) = self.view.as_mut() {
            v.scroll = (v.scroll as isize + delta).clamp(0, max.max(0)) as usize;
        }
    }

    fn scroll_to(&mut self, line: usize) {
        let w = self.content_width();
        let Some(lay) = self.layout_for(w) else {
            return;
        };
        let max = lay.lines.len().saturating_sub(self.reader_height.max(1));
        if let Some(v) = self.view.as_mut() {
            v.scroll = line.min(max);
        }
    }

    fn jump_heading(&mut self, forward: bool) {
        let w = self.content_width();
        let Some(lay) = self.layout_for(w) else {
            return;
        };
        let Some(cur) = self.view.as_ref().map(|v| v.scroll) else {
            return;
        };
        let target = if forward {
            lay.headings.iter().find(|(l, _, _)| *l > cur)
        } else {
            lay.headings.iter().rev().find(|(l, _, _)| *l < cur)
        };
        if let Some((l, _, _)) = target {
            self.scroll_to(*l);
        }
    }

    fn cycle_link(&mut self, forward: bool) {
        let w = self.content_width();
        let Some(lay) = self.layout_for(w) else {
            return;
        };
        let Some(v) = self.view.as_ref() else { return };
        let n = v.doc.links.len();
        // Solo enlaces que aparecen en el texto maquetado.
        let visible: Vec<usize> = (0..n).filter(|i| lay.link_lines[*i].is_some()).collect();
        if visible.is_empty() {
            self.warn("No hay enlaces en este documento");
            return;
        }
        let cur_pos = v.link.and_then(|l| visible.iter().position(|x| *x == l));
        let next = match (cur_pos, forward) {
            (None, true) => {
                // Primer enlace visible a partir del scroll actual.
                visible
                    .iter()
                    .position(|i| lay.link_lines[*i].unwrap_or(0) >= v.scroll)
                    .unwrap_or(0)
            }
            (None, false) => visible
                .iter()
                .rposition(|i| lay.link_lines[*i].unwrap_or(0) < v.scroll + self.reader_height)
                .unwrap_or(visible.len() - 1),
            (Some(p), true) => (p + 1) % visible.len(),
            (Some(p), false) => (p + visible.len() - 1) % visible.len(),
        };
        let link = visible[next];
        let line = lay.link_lines[link].unwrap_or(0);
        let (scroll, height) = (v.scroll, self.reader_height);
        if let Some(v) = self.view.as_mut() {
            v.link = Some(link);
        }
        if line < scroll || line >= scroll + height.saturating_sub(2) {
            self.scroll_to(line.saturating_sub(height / 3));
        }
    }

    fn build_entry_index(&mut self) {
        if self.entry_index.is_some() {
            return;
        }
        let mut m = HashMap::with_capacity(self.library.entries.len());
        for e in &self.library.entries {
            m.entry((e.docset_id.clone(), e.path.clone()))
                .or_insert_with(|| (e.name.clone(), e.kind.clone()));
        }
        self.entry_index = Some(m);
    }

    fn follow_link(&mut self) {
        let Some(view) = self.view.clone() else {
            return;
        };
        let Some(li) = view.link else {
            self.warn("Selecciona un enlace con Tab");
            return;
        };
        let href = view.doc.links[li].clone();
        if href.contains("://") || href.starts_with("mailto:") {
            match crate::run::open_in_default(Path::new(&href)) {
                Ok(()) => self.say(format!("Abierto en el navegador: {href}")),
                Err(e) => self.warn(e.to_string()),
            }
            return;
        }
        let (file_part, anchor) = match href.split_once('#') {
            Some((f, a)) => (f.to_string(), Some(a.to_string())),
            None => (href.clone(), None),
        };
        let cur_file = view.path.split('#').next().unwrap_or("");
        let target_file = if file_part.is_empty() {
            cur_file.to_string()
        } else {
            let base = Path::new(cur_file).parent().unwrap_or(Path::new(""));
            normalize(&base.join(&file_part))
        };
        let target = match &anchor {
            Some(a) if !a.is_empty() => format!("{target_file}#{a}"),
            _ => target_file.clone(),
        };
        self.build_entry_index();
        let idx = self.entry_index.as_ref().unwrap();
        let known = idx
            .get(&(view.docset.clone(), target.clone()))
            .or_else(|| idx.get(&(view.docset.clone(), target_file.clone())))
            .cloned();
        let (name, kind) = known
            .map(|(n, k)| (n, render::sym_kind(&k, &target)))
            .unwrap_or_else(|| {
                let stem = Path::new(&target_file)
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| target.clone());
                (stem, "guide".into())
            });
        match self.load_doc(&view.docset, &target, &name, &kind) {
            Some(doc) => {
                if self.history.len() >= HISTORY_MAX {
                    self.history.remove(0);
                }
                self.history.push(view.clone());
                self.find = None;
                self.view = Some(View {
                    docset: view.docset,
                    path: target,
                    name,
                    doc,
                    scroll: 0,
                    link: None,
                });
                self.record_recent();
            }
            None => self.warn(format!("No se pudo abrir el enlace: {href}")),
        }
    }

    fn go_back(&mut self) -> bool {
        match self.history.pop() {
            Some(prev) => {
                self.find = None;
                self.view = Some(prev);
                true
            }
            None => false,
        }
    }

    // ───────── favoritos, recientes, relacionados, índice ─────────

    fn current_item(&self) -> Option<Item> {
        let v = self.view.as_ref()?;
        Some(Item {
            docset: v.docset.clone(),
            path: v.path.clone(),
            name: v.name.clone(),
            kind: v.doc.kind.clone(),
        })
    }

    fn record_recent(&mut self) {
        if let Some(it) = self.current_item() {
            self.state.push_recent(it);
        }
    }

    /// Pasa el foco al lector registrando la lectura.
    fn enter_reader(&mut self) {
        // Si la vista previa aún no se cargó (teclado muy rápido), cárgala ya.
        if self.pending_load {
            self.load_selected();
        }
        if self.view.is_some() {
            self.focus = Focus::Reader;
            self.record_recent();
        }
    }

    fn toggle_favorite(&mut self) {
        let Some(it) = self.current_item() else {
            return;
        };
        let name = it.name.clone();
        if self.state.toggle_favorite(it) {
            self.say(format!("★ Añadido a favoritos: {name}"));
        } else {
            self.say(format!("Quitado de favoritos: {name}"));
        }
    }

    fn is_favorite(&self) -> bool {
        self.current_item()
            .is_some_and(|i| self.state.is_favorite(&i))
    }

    /// Abre un símbolo guardado o relacionado sin pasar por la búsqueda.
    fn open_item(&mut self, it: Item) {
        if self.library.docset(&it.docset).is_none() {
            self.warn(format!("El docset «{}» ya no está instalado", it.docset));
            return;
        }
        let Some(doc) = self.load_doc(&it.docset, &it.path, &it.name, &it.kind) else {
            self.warn(format!("No se pudo abrir «{}»", it.name));
            return;
        };
        if let Some(prev) = self.view.take() {
            if self.history.len() >= HISTORY_MAX {
                self.history.remove(0);
            }
            self.history.push(prev);
        }
        self.view = Some(View {
            docset: it.docset,
            path: it.path,
            name: it.name,
            doc,
            scroll: 0,
            link: None,
        });
        self.find = None;
        self.focus = Focus::Reader;
        self.record_recent();
    }

    fn recent_items(&self) -> Vec<(String, Option<String>)> {
        let mut items = Vec::new();
        let label = |star: &str, it: &Item| {
            format!(
                "{star} {:<6} {}  ·  {}",
                ui::truncate_end(&it.kind, 6),
                it.name,
                short_docset(&it.docset)
            )
        };
        for it in &self.state.favorites {
            items.push((label("★", it), Some(enc(it))));
        }
        for it in self
            .state
            .recent
            .iter()
            .filter(|r| !self.state.is_favorite(r))
        {
            items.push((label(" ", it), Some(enc(it))));
        }
        items
    }

    fn open_recent(&mut self) {
        let items = self.recent_items();
        if items.is_empty() {
            self.warn("Aún no hay recientes ni favoritos (m marca un símbolo)");
            return;
        }
        self.overlay = Overlay::Pick(Picker {
            kind: PickKind::Recent,
            title: " Favoritos y recientes · Supr olvida ".into(),
            items,
            filter: String::new(),
            sel: 0,
        });
    }

    fn open_outline(&mut self) {
        let width = self.content_width();
        let Some(lay) = self.layout_for(width) else {
            return;
        };
        if lay.headings.is_empty() {
            self.warn("Esta página no tiene secciones");
            return;
        }
        let scroll = self.view.as_ref().map_or(0, |v| v.scroll);
        let sel = lay
            .headings
            .iter()
            .rposition(|(l, _, _)| *l <= scroll + 1)
            .unwrap_or(0);
        let min_lvl = lay.headings.iter().map(|h| h.1).min().unwrap_or(1);
        let items = lay
            .headings
            .iter()
            .map(|(line, lvl, text)| {
                let indent = "  ".repeat(usize::from(lvl - min_lvl).min(4));
                (format!("{indent}{text}"), Some(line.to_string()))
            })
            .collect();
        self.overlay = Overlay::Pick(Picker {
            kind: PickKind::Outline,
            title: " Índice de la página ".into(),
            items,
            filter: String::new(),
            sel,
        });
    }

    /// Otros símbolos de la misma página o del mismo «padre» (Vec → sus métodos).
    fn open_related(&mut self) {
        let Some(v) = self.view.as_ref() else { return };
        let (docset, path, name) = (v.docset.clone(), v.path.clone(), v.name.clone());
        let file = path.split('#').next().unwrap_or("").to_string();
        let parent = name
            .rsplit_once("::")
            .or_else(|| name.rsplit_once('.'))
            .map(|(p, _)| p.to_string());
        let mut seen = std::collections::HashSet::new();
        let mut items = Vec::new();
        for e in &self.library.entries {
            if e.docset_id != docset || e.path == path {
                continue;
            }
            let same_file = e.path.split('#').next() == Some(file.as_str());
            let same_parent = parent.as_ref().is_some_and(|p| {
                e.name
                    .strip_prefix(p.as_str())
                    .is_some_and(|r| r.starts_with("::") || r.starts_with('.'))
            });
            if (same_file || same_parent) && seen.insert(e.path.clone()) {
                let it = Item {
                    docset: e.docset_id.clone(),
                    path: e.path.clone(),
                    name: e.name.clone(),
                    kind: render::sym_kind(&e.kind, &e.path),
                };
                items.push((
                    format!("{:<6} {}", ui::truncate_end(&it.kind, 6), it.name),
                    Some(enc(&it)),
                ));
                if items.len() >= 800 {
                    break;
                }
            }
        }
        if items.is_empty() {
            self.warn("Sin símbolos relacionados");
            return;
        }
        self.overlay = Overlay::Pick(Picker {
            kind: PickKind::Related,
            title: format!(
                " Relacionados con {} ({}) ",
                ui::truncate_start(&name, 30),
                items.len()
            ),
            items,
            filter: String::new(),
            sel: 0,
        });
    }

    // ───────── búsqueda en la página ─────────

    fn find_start(&mut self) {
        if self.view.is_some() {
            self.find = Some(Find {
                input: String::new(),
                editing: true,
                matches: Vec::new(),
                cur: 0,
                stamp: None,
            });
        }
    }

    fn find_refresh(&mut self, jump: bool) {
        let width = self.content_width();
        let Some(lay) = self.layout_for(width) else {
            return;
        };
        let Some(view) = self.view.as_ref() else {
            return;
        };
        let (doc, scroll) = (view.doc.clone(), view.scroll);
        if let Some(f) = self.find.as_mut() {
            f.matches = compute_matches(&lay, &f.input);
            f.stamp = Some((doc, width));
            if jump {
                f.cur = f.matches.iter().position(|m| m.0 >= scroll).unwrap_or(0);
            } else {
                f.cur = f.cur.min(f.matches.len().saturating_sub(1));
            }
        }
        if jump {
            self.find_show_current();
        }
    }

    fn find_show_current(&mut self) {
        let Some(line) = self
            .find
            .as_ref()
            .and_then(|f| f.matches.get(f.cur).map(|m| m.0))
        else {
            return;
        };
        let (scroll, h) = (
            self.view.as_ref().map_or(0, |v| v.scroll),
            self.reader_height,
        );
        if line < scroll || line >= scroll + h.saturating_sub(1) {
            self.scroll_to(line.saturating_sub(h / 3));
        }
    }

    fn find_step(&mut self, forward: bool) {
        let Some(f) = self.find.as_mut() else { return };
        if f.matches.is_empty() {
            return;
        }
        let n = f.matches.len();
        f.cur = if forward {
            (f.cur + 1) % n
        } else {
            (f.cur + n - 1) % n
        };
        self.find_show_current();
    }

    // ───────── acciones ─────────

    fn open_external(&mut self) {
        let Some(v) = self.view.as_ref() else { return };
        let Some(docset) = self.library.docset(&v.docset) else {
            return;
        };
        let documents = docset.contents_path.join("Resources/Documents");
        let file = v.path.split('#').next().unwrap_or("");
        match resolve_request(&documents, file) {
            Ok(r) => match crate::run::open_in_default(&r.path) {
                Ok(()) => self.say(format!("Abierto en el navegador: {}", v.name)),
                Err(e) => self.warn(e.to_string()),
            },
            Err(e) => self.warn(e.to_string()),
        }
    }

    fn copy(&mut self, what: char) {
        let Some(v) = self.view.as_ref() else { return };
        let (label, text) = match what {
            'y' => match v.doc.code_blocks.first().or(v.doc.signature.as_ref()) {
                Some(t) => ("ejemplo", t.clone()),
                None => ("texto", v.doc.plain_text(100)),
            },
            's' => match &v.doc.signature {
                Some(t) => ("firma", t.clone()),
                None => {
                    self.warn("Este símbolo no tiene firma");
                    return;
                }
            },
            _ => ("documentación", v.doc.plain_text(100)),
        };
        let name = v.name.clone();
        match clip::copy(&text) {
            Ok(_) => self.say(format!("Copiado ({label}): {name}")),
            Err(e) => self.warn(format!("No se pudo copiar: {e}")),
        }
    }

    fn open_picker(&mut self, kind: PickKind) {
        let mut items: Vec<(String, Option<String>)> = Vec::new();
        match kind {
            PickKind::Docset => {
                items.push(("Todos los docsets".into(), None));
                let mut counts: HashMap<&str, usize> = HashMap::new();
                for e in &self.library.entries {
                    *counts.entry(e.docset_id.as_str()).or_default() += 1;
                }
                for d in &self.library.docsets {
                    let n = counts.get(d.id.as_str()).copied().unwrap_or(0);
                    items.push((format!("{}  ·  {} entradas", d.name, n), Some(d.id.clone())));
                }
            }
            _ => {
                items.push(("Todos los tipos".into(), None));
                let mut counts: HashMap<String, usize> = HashMap::new();
                for e in &self.library.entries {
                    if self.docset.as_ref().is_none_or(|d| d == &e.docset_id) {
                        *counts
                            .entry(render::sym_kind(&e.kind, &e.path))
                            .or_default() += 1;
                    }
                }
                let mut v: Vec<_> = counts.into_iter().collect();
                v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                for (k, n) in v {
                    items.push((format!("{k}  ·  {n}"), Some(k)));
                }
            }
        }
        let current = match kind {
            PickKind::Docset => &self.docset,
            _ => &self.kind,
        };
        let sel = items.iter().position(|(_, v)| v == current).unwrap_or(0);
        let title = match kind {
            PickKind::Docset => " Elegir docset ",
            _ => " Elegir tipo de símbolo ",
        };
        self.overlay = Overlay::Pick(Picker {
            kind,
            title: title.into(),
            items,
            filter: String::new(),
            sel,
        });
    }

    fn apply_picker(&mut self) {
        let Overlay::Pick(p) = std::mem::replace(&mut self.overlay, Overlay::None) else {
            return;
        };
        let visible = p.visible();
        let Some((_, val)) = visible.get(p.sel) else {
            return;
        };
        let val = val.clone();
        match p.kind {
            PickKind::Docset => {
                self.docset = val;
                self.kind = None;
                self.refresh();
            }
            PickKind::Kind => {
                self.kind = val;
                self.refresh();
            }
            PickKind::Outline => {
                if let Some(line) = val.and_then(|v| v.parse::<usize>().ok()) {
                    self.scroll_to(line);
                    self.focus = Focus::Reader;
                }
            }
            PickKind::Recent | PickKind::Related => {
                if let Some(it) = val.as_deref().and_then(dec) {
                    self.open_item(it);
                }
            }
        }
    }

    fn cycle_docset(&mut self, forward: bool) {
        let ids: Vec<&String> = self.library.docsets.iter().map(|d| &d.id).collect();
        if ids.is_empty() {
            return;
        }
        let pos = self
            .docset
            .as_ref()
            .and_then(|d| ids.iter().position(|i| *i == d));
        // Posiciones: None → 0 → 1 … → None
        let n = ids.len() as isize + 1;
        let cur = pos.map_or(0, |p| p as isize + 1);
        let next = (cur + if forward { 1 } else { -1 }).rem_euclid(n);
        self.docset = if next == 0 {
            None
        } else {
            Some(ids[next as usize - 1].clone())
        };
        self.kind = None;
        self.refresh();
    }

    // ───────── edición de la consulta ─────────

    fn byte_at(&self, chars: usize) -> usize {
        self.query
            .char_indices()
            .nth(chars)
            .map_or(self.query.len(), |(i, _)| i)
    }

    fn insert(&mut self, c: char) {
        let at = self.byte_at(self.cursor);
        self.query.insert(at, c);
        self.cursor += 1;
        self.refresh();
    }

    fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let at = self.byte_at(self.cursor - 1);
        self.query.remove(at);
        self.cursor -= 1;
        self.refresh();
    }

    fn delete_word(&mut self) {
        let chars: Vec<char> = self.query.chars().collect();
        let mut i = self.cursor;
        while i > 0 && chars[i - 1].is_whitespace() {
            i -= 1;
        }
        while i > 0 && !chars[i - 1].is_whitespace() {
            i -= 1;
        }
        let new: String = chars[..i]
            .iter()
            .chain(chars[self.cursor..].iter())
            .collect();
        self.query = new;
        self.cursor = i;
        self.refresh();
    }
}

/// Normaliza `a/b/../c` sin tocar el disco.
fn normalize(p: &Path) -> String {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out.to_string_lossy().replace('\\', "/")
}

// ───────────────────────── Bucle principal ─────────────────────────

/// Ejecuta la interfaz TUI.
pub fn run_tui(
    library: &Library,
    initial_docset: Option<String>,
    initial_query: Option<String>,
) -> Result<(), CliError> {
    let tui_err = |e: std::io::Error| CliError::Tui(e.to_string());
    enable_raw_mode().map_err(tui_err)?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture).map_err(tui_err)?;

    // Restaura la terminal incluso ante un pánico.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(std::io::stdout(), DisableMouseCapture, LeaveAlternateScreen);
        default_hook(info);
    }));

    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).map_err(tui_err)?;
    let mut app = App::new(library, initial_docset, None, initial_query);

    let res = event_loop(&mut terminal, &mut app);

    let _ = disable_raw_mode();
    let _ = execute!(
        terminal.backend_mut(),
        DisableMouseCapture,
        LeaveAlternateScreen
    );
    let _ = terminal.show_cursor();
    res
}

fn event_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<(), CliError> {
    let tui_err = |e: std::io::Error| CliError::Tui(e.to_string());
    app.load_selected();
    loop {
        terminal
            .draw(|f| draw(f, app))
            .map_err(|e| CliError::Tui(e.to_string()))?;
        if app.quit {
            return Ok(());
        }
        // Espera el primer evento y vacía la cola antes de cargar nada.
        if !event::poll(Duration::from_millis(400)).map_err(tui_err)? {
            continue;
        }
        loop {
            match event::read().map_err(tui_err)? {
                Event::Key(k) if k.kind != KeyEventKind::Release => handle_key(app, k),
                Event::Mouse(m) => handle_mouse(app, m),
                _ => {}
            }
            if app.quit || !event::poll(Duration::ZERO).map_err(tui_err)? {
                break;
            }
        }
        if app.pending_load {
            app.load_selected();
        }
    }
}

/// Línea y columna del documento bajo una posición de pantalla.
fn doc_pos(app: &mut App, x: u16, y: u16, clamp: bool) -> Option<(usize, usize)> {
    let area = app.reader_area;
    let width = app.content_width();
    let lay = app.layout_for(width)?;
    let scroll = app.view.as_ref()?.scroll;
    let row = if y <= area.y {
        0
    } else {
        (y - area.y - 1) as usize
    };
    let row = if clamp {
        row.min(app.reader_height.saturating_sub(1))
    } else {
        row
    };
    let line = (scroll + row).min(lay.lines.len().saturating_sub(1));
    let col = x.saturating_sub(area.x + 2) as usize;
    Some((line, col))
}

/// Texto de la selección (columnas de pantalla, sin relleno final).
fn selected_text(lay: &DocLayout, sel: &Selection) -> String {
    let (a, b) = sel.ordered();
    let mut out = Vec::new();
    for i in a.0..=b.0.min(lay.lines.len().saturating_sub(1)) {
        let Some((from, to)) = sel.range_on(i) else {
            continue;
        };
        let mut col = 0usize;
        let mut line = String::new();
        for seg in &lay.lines[i] {
            for ch in seg.text.chars() {
                let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
                if col + w > from && col < to {
                    line.push(ch);
                }
                col += w;
            }
        }
        out.push(line.trim_end().to_string());
    }
    out.join("\n")
}

fn line_text(lay: &DocLayout, line: usize) -> Vec<(char, usize)> {
    let mut col = 0usize;
    let mut v = Vec::new();
    for seg in lay.lines.get(line).into_iter().flatten() {
        for ch in seg.text.chars() {
            v.push((ch, col));
            col += unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
        }
    }
    v
}

fn finish_selection(app: &mut App) {
    let width = app.content_width();
    let (Some(sel), Some(lay)) = (app.sel, app.layout_for(width)) else {
        return;
    };
    let text = selected_text(&lay, &sel);
    if text.trim().is_empty() {
        app.sel = None;
        return;
    }
    let n = text.chars().count();
    match clip::copy(&text) {
        Ok(_) => app.say(format!("Copiado: {n} caracteres")),
        Err(e) => app.warn(format!("No se pudo copiar: {e}")),
    }
}

fn handle_mouse(app: &mut App, m: crossterm::event::MouseEvent) {
    let pos = ratatui::layout::Position::new(m.column, m.row);
    if !matches!(app.overlay, Overlay::None) {
        return;
    }
    match m.kind {
        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp => {
            let down = m.kind == MouseEventKind::ScrollDown;
            if app.list_area.contains(pos) {
                app.move_sel(if down { 3 } else { -3 });
            } else if app.reader_area.contains(pos) {
                app.scroll_by(if down { 3 } else { -3 });
            }
        }
        MouseEventKind::Down(MouseButton::Left) => {
            app.sel = None;
            if app.list_area.contains(pos) {
                let row = pos.y.saturating_sub(app.list_area.y + 1) as usize;
                let idx = app.list.offset() + row;
                if idx < app.results.len() {
                    app.list.select(Some(idx));
                    app.focus = Focus::Search;
                    app.queue_load();
                }
            } else if app.reader_area.contains(pos) {
                if app.focus != Focus::Reader {
                    app.enter_reader();
                }
                let Some(p) = doc_pos(app, m.column, m.row, true) else {
                    return;
                };
                // Doble clic: palabra · triple clic: línea.
                let count = match app.last_click {
                    Some((t, l, c, n))
                        if t.elapsed() < Duration::from_millis(450)
                            && l == p.0
                            && c.abs_diff(p.1) <= 1 =>
                    {
                        n + 1
                    }
                    _ => 1,
                };
                app.last_click = Some((Instant::now(), p.0, p.1, count));
                if count >= 2 {
                    let width = app.content_width();
                    let Some(lay) = app.layout_for(width) else {
                        return;
                    };
                    let chars = line_text(&lay, p.0);
                    if chars.is_empty() {
                        return;
                    }
                    let (from, to) = if count >= 3 {
                        (0, chars.last().map_or(0, |c| c.1))
                    } else {
                        let word = |c: char| c.is_alphanumeric() || c == '_';
                        let at = chars
                            .iter()
                            .position(|(_, col)| *col >= p.1)
                            .unwrap_or(chars.len() - 1);
                        if !word(chars[at].0) {
                            return;
                        }
                        let mut a = at;
                        while a > 0 && word(chars[a - 1].0) {
                            a -= 1;
                        }
                        let mut b = at;
                        while b + 1 < chars.len() && word(chars[b + 1].0) {
                            b += 1;
                        }
                        (chars[a].1, chars[b].1)
                    };
                    app.sel = Some(Selection {
                        anchor: (p.0, from),
                        head: (p.0, to),
                        dragging: false,
                    });
                    finish_selection(app);
                } else {
                    app.sel = Some(Selection {
                        anchor: p,
                        head: p,
                        dragging: true,
                    });
                }
            }
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            if app.sel.is_some_and(|s| s.dragging) {
                // Autoscroll al arrastrar fuera del lector.
                let area = app.reader_area;
                if m.row <= area.y {
                    app.scroll_by(-1);
                } else if m.row >= area.bottom().saturating_sub(1) {
                    app.scroll_by(1);
                }
                if let Some(p) = doc_pos(app, m.column, m.row, true) {
                    if let Some(sel) = app.sel.as_mut() {
                        sel.head = p;
                    }
                }
            }
        }
        MouseEventKind::Up(MouseButton::Left) => {
            if let Some(sel) = app.sel.as_mut() {
                if sel.dragging {
                    sel.dragging = false;
                    if sel.anchor == sel.head {
                        app.sel = None;
                    } else {
                        finish_selection(app);
                    }
                }
            }
        }
        _ => {}
    }
}

fn handle_key(app: &mut App, key: KeyEvent) {
    app.sel = None;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if ctrl && key.code == KeyCode::Char('c') {
        app.quit = true;
        return;
    }
    if !matches!(app.overlay, Overlay::None) {
        handle_overlay_key(app, key);
        return;
    }
    if key.code == KeyCode::F(1) {
        app.overlay = Overlay::Help;
        return;
    }
    match app.focus {
        Focus::Search => handle_search_key(app, key),
        Focus::Reader => handle_reader_key(app, key),
    }
}

fn handle_overlay_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    if matches!(app.overlay, Overlay::Help) {
        app.overlay = Overlay::None;
        return;
    }
    let Overlay::Pick(p) = &mut app.overlay else {
        return;
    };
    let n = p.visible().len();
    match key.code {
        KeyCode::Esc => app.overlay = Overlay::None,
        KeyCode::Enter => app.apply_picker(),
        KeyCode::Delete if p.kind == PickKind::Recent => {
            let target = p
                .visible()
                .get(p.sel)
                .and_then(|(_, v)| v.as_deref().and_then(dec));
            let (filter, sel) = (p.filter.clone(), p.sel);
            if let Some(it) = target {
                app.state.forget(&it);
                let items = app.recent_items();
                if let Overlay::Pick(p) = &mut app.overlay {
                    p.items = items;
                    p.filter = filter;
                    p.sel = sel.min(p.visible().len().saturating_sub(1));
                }
            }
        }
        KeyCode::Up => p.sel = p.sel.saturating_sub(1),
        KeyCode::Char('p') if ctrl => p.sel = p.sel.saturating_sub(1),
        KeyCode::Down => p.sel = (p.sel + 1).min(n.saturating_sub(1)),
        KeyCode::Char('n') if ctrl => p.sel = (p.sel + 1).min(n.saturating_sub(1)),
        KeyCode::Backspace => {
            p.filter.pop();
            p.sel = 0;
        }
        KeyCode::Char(c) if !ctrl => {
            p.filter.push(c);
            p.sel = 0;
        }
        _ => {}
    }
}

fn handle_search_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Esc => {
            if !app.query.is_empty() {
                app.query.clear();
                app.cursor = 0;
                app.refresh();
            } else if app.docset.is_some() || app.kind.is_some() {
                app.docset = None;
                app.kind = None;
                app.refresh();
            } else {
                app.quit = true;
            }
        }
        KeyCode::Tab | KeyCode::Enter => app.enter_reader(),
        KeyCode::BackTab => app.cycle_docset(false),
        KeyCode::Down => app.move_sel(1),
        KeyCode::Up => app.move_sel(-1),
        KeyCode::Char('n') | KeyCode::Char('j') if ctrl => app.move_sel(1),
        KeyCode::Char('p') | KeyCode::Char('k') if ctrl => app.move_sel(-1),
        KeyCode::PageDown => app.scroll_by(app.reader_height as isize - 2),
        KeyCode::PageUp => app.scroll_by(-(app.reader_height as isize - 2)),
        KeyCode::Char('d') if ctrl => app.open_picker(PickKind::Docset),
        KeyCode::Char('t') if ctrl => app.open_picker(PickKind::Kind),
        KeyCode::Char('b') if ctrl => app.open_external(),
        KeyCode::Char('o') if ctrl => app.open_outline(),
        KeyCode::Char('h') if ctrl => app.open_recent(),
        KeyCode::F(2) => app.open_recent(),
        KeyCode::Char('h') if alt => app.open_recent(),
        KeyCode::Char('f') if ctrl => app.toggle_favorite(),
        KeyCode::Char('r') if ctrl => app.open_related(),
        KeyCode::Char('y') if ctrl => app.copy('y'),
        KeyCode::Char('/') if ctrl => app.overlay = Overlay::Help,
        KeyCode::Char('w') if ctrl => app.delete_word(),
        KeyCode::Char('u') if ctrl => {
            app.query.clear();
            app.cursor = 0;
            app.refresh();
        }
        KeyCode::Char('a') if ctrl => app.cursor = 0,
        KeyCode::Char('e') if ctrl => app.cursor = app.query.chars().count(),
        KeyCode::Home => app.cursor = 0,
        KeyCode::End => app.cursor = app.query.chars().count(),
        KeyCode::Left => app.cursor = app.cursor.saturating_sub(1),
        KeyCode::Right => {
            if app.cursor < app.query.chars().count() {
                app.cursor += 1;
            } else {
                app.enter_reader();
            }
        }
        KeyCode::Backspace => app.backspace(),
        KeyCode::Char(c) if !ctrl && !alt => app.insert(c),
        _ => {}
    }
}

fn handle_find_key(app: &mut App, key: KeyEvent) {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let Some(f) = app.find.as_mut() else { return };
    match key.code {
        KeyCode::Esc => app.find = None,
        KeyCode::Enter => {
            f.editing = false;
            if f.matches.is_empty() {
                let q = f.input.clone();
                app.find = None;
                if !q.is_empty() {
                    app.warn(format!("Sin coincidencias para «{q}»"));
                }
            }
        }
        KeyCode::Backspace => {
            f.input.pop();
            app.find_refresh(true);
        }
        KeyCode::Char('u') if ctrl => {
            f.input.clear();
            app.find_refresh(true);
        }
        KeyCode::Down | KeyCode::Tab => app.find_step(true),
        KeyCode::Up | KeyCode::BackTab => app.find_step(false),
        KeyCode::Char(c) if !ctrl => {
            f.input.push(c);
            app.find_refresh(true);
        }
        _ => {}
    }
}

fn handle_reader_key(app: &mut App, key: KeyEvent) {
    if app.find.as_ref().is_some_and(|f| f.editing) {
        handle_find_key(app, key);
        return;
    }
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    let half = (app.reader_height / 2).max(1) as isize;
    let page = (app.reader_height.saturating_sub(2)).max(1) as isize;

    // Acorde estilo vim: gg (inicio) · gO (índice).
    if std::mem::take(&mut app.pending_g) {
        match key.code {
            KeyCode::Char('g') => return app.scroll_to(0),
            KeyCode::Char('O') => return app.open_outline(),
            _ => {}
        }
    }
    let has_find = app.find.as_ref().is_some_and(|f| !f.matches.is_empty());
    match key.code {
        KeyCode::Char('q') => app.quit = true,
        KeyCode::Esc => {
            if app.find.is_some() {
                app.find = None;
            } else if !app.go_back() {
                app.focus = Focus::Search;
            }
        }
        KeyCode::Char('i') => app.focus = Focus::Search,
        KeyCode::Char('/') => app.find_start(),
        KeyCode::Char('n') if has_find => app.find_step(true),
        KeyCode::Char('N') if has_find => app.find_step(false),
        KeyCode::Char('j') | KeyCode::Down => app.scroll_by(1),
        KeyCode::Char('k') | KeyCode::Up => app.scroll_by(-1),
        KeyCode::Char('d') if ctrl => app.scroll_by(half),
        KeyCode::Char('u') if ctrl => app.scroll_by(-half),
        KeyCode::Char('f') if ctrl => app.scroll_by(page),
        KeyCode::Char('b') if ctrl => app.scroll_by(-page),
        KeyCode::Char('o') if ctrl => app.open_outline(),
        KeyCode::Char('h') if ctrl => app.open_recent(),
        KeyCode::Char('h') if alt => app.open_recent(),
        KeyCode::F(2) => app.open_recent(),
        KeyCode::PageDown | KeyCode::Char(' ') => app.scroll_by(page),
        KeyCode::PageUp | KeyCode::Char('b') => app.scroll_by(-page),
        KeyCode::Char('g') => app.pending_g = true,
        KeyCode::Home => app.scroll_to(0),
        KeyCode::Char('G') | KeyCode::End => app.scroll_to(usize::MAX),
        KeyCode::Char(']') | KeyCode::Char('n') => app.jump_heading(true),
        KeyCode::Char('[') | KeyCode::Char('N') => app.jump_heading(false),
        KeyCode::Tab => app.cycle_link(true),
        KeyCode::BackTab => app.cycle_link(false),
        KeyCode::Enter => app.follow_link(),
        KeyCode::Backspace | KeyCode::Char('h') | KeyCode::Left => {
            if !app.go_back() {
                app.focus = Focus::Search;
            }
        }
        KeyCode::Char('J') | KeyCode::Char('K') if shift => {
            app.move_sel(if key.code == KeyCode::Char('J') {
                1
            } else {
                -1
            });
        }
        KeyCode::Char('y') => app.copy('y'),
        KeyCode::Char('Y') => app.copy('Y'),
        KeyCode::Char('s') => app.copy('s'),
        KeyCode::Char('m') => app.toggle_favorite(),
        KeyCode::Char('H') => app.open_recent(),
        KeyCode::Char('O') => app.open_outline(),
        KeyCode::Char('r') => app.open_related(),
        KeyCode::Char('o') => app.open_external(),
        KeyCode::Char('d') => app.open_picker(PickKind::Docset),
        KeyCode::Char('t') => app.open_picker(PickKind::Kind),
        KeyCode::Char('?') => app.overlay = Overlay::Help,
        _ => {}
    }
}

// ───────────────────────── Dibujo ─────────────────────────

const C_BORDER: Color = Color::Rgb(69, 71, 90);
const C_ACCENT: Color = Color::Rgb(203, 166, 247);
const C_TEXT: Color = Color::Rgb(205, 214, 244);
const C_MUTED: Color = Color::Rgb(127, 132, 156);
const C_SELECT: Color = Color::Rgb(88, 91, 112);
const C_SURFACE: Color = Color::Rgb(49, 50, 68);
const C_BASE: Color = Color::Rgb(30, 30, 46);
const C_MANTLE: Color = Color::Rgb(24, 24, 37);
const C_GREEN: Color = Color::Rgb(166, 227, 161);
const C_RED: Color = Color::Rgb(243, 139, 168);

fn chip(text: &str, fg: Color, bg: Color) -> Span<'static> {
    Span::styled(
        format!(" {text} "),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

fn key_hint(key: &str, label: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled(
            format!(" {key} "),
            Style::default()
                .fg(C_MANTLE)
                .bg(C_MUTED)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!(" {label}  "), Style::default().fg(C_MUTED)),
    ]
}

fn draw(f: &mut ratatui::Frame, app: &mut App) {
    let area = f.area();
    f.render_widget(Block::default().style(Style::default().bg(C_BASE)), area);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(4),
            Constraint::Length(1),
        ])
        .split(area);

    draw_search(f, app, rows[0]);

    app.narrow = area.width < 90;
    let body = if app.narrow {
        // Pantalla estrecha: una sola columna según el foco.
        if app.focus == Focus::Reader && app.view.is_some() {
            vec![Rect::default(), rows[1]]
        } else {
            vec![rows[1], Rect::default()]
        }
    } else {
        let lw = (rows[1].width * 36 / 100).clamp(30, 52);
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(lw), Constraint::Min(30)])
            .split(rows[1]);
        vec![cols[0], cols[1]]
    };
    if body[0].width > 0 {
        draw_results(f, app, body[0]);
    } else {
        app.list_area = Rect::default();
    }
    if body[1].width > 0 {
        draw_reader(f, app, body[1]);
    } else {
        app.reader_area = Rect::default();
    }
    draw_footer(f, app, rows[2]);

    match &app.overlay {
        Overlay::None => {}
        Overlay::Help => draw_help(f, area),
        Overlay::Pick(p) => draw_picker(f, p, area),
    }
}

fn draw_search(f: &mut ratatui::Frame, app: &App, area: Rect) {
    let active = app.focus == Focus::Search && matches!(app.overlay, Overlay::None);
    let border = if active { C_ACCENT } else { C_BORDER };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border))
        .title(Line::styled(
            " odc ",
            Style::default()
                .fg(C_MANTLE)
                .bg(C_ACCENT)
                .add_modifier(Modifier::BOLD),
        ))
        .title_top(
            RLine::from(vec![Span::styled(
                format!(
                    " {} resultado{} ",
                    app.results.len(),
                    if app.results.len() == 1 { "" } else { "s" }
                ),
                Style::default().fg(C_MUTED),
            )])
            .alignment(Alignment::Right),
        );
    let inner = block.inner(area);
    f.render_widget(block, area);

    let mut spans = vec![Span::styled(" ", Style::default())];
    let parsed = query::parse(app.library, &app.kinds, &app.query, None);
    let docset_name = parsed
        .docset
        .as_ref()
        .or(app.docset.as_ref())
        .and_then(|id| app.library.docset(id))
        .map(|d| d.name.clone());
    match &docset_name {
        Some(n) => spans.push(chip(n, C_MANTLE, C_GREEN)),
        None => spans.push(chip("todos", C_TEXT, C_SURFACE)),
    }
    if let Some(k) = parsed.kind.as_ref().or(app.kind.as_ref()) {
        spans.push(Span::raw(" "));
        spans.push(chip(k, C_MANTLE, rgb(render::kind_color(k))));
    }
    spans.push(Span::raw(" "));
    let prefix_w: usize = spans.iter().map(|s| s.content.width()).sum();
    if app.query.is_empty() {
        spans.push(Span::styled(
            "escribe para buscar…  (rust:Vec::push · fn:map)",
            Style::default().fg(C_MUTED),
        ));
    } else {
        spans.push(Span::styled(app.query.clone(), Style::default().fg(C_TEXT)));
    }
    f.render_widget(Paragraph::new(RLine::from(spans)), inner);

    if active {
        let before: String = app.query.chars().take(app.cursor).collect();
        let x = inner.x + prefix_w as u16 + before.width() as u16;
        f.set_cursor_position((x.min(inner.right().saturating_sub(1)), inner.y));
    }
}

/// Posiciones de `name` que casan (subsecuencia) con el texto buscado.
fn match_positions(name: &str, query: &str) -> Vec<usize> {
    let q: Vec<char> = query
        .to_lowercase()
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if q.is_empty() {
        return Vec::new();
    }
    // Prioriza una coincidencia contigua (más natural al leer).
    let lower: Vec<char> = name.to_lowercase().chars().collect();
    let qs: String = q.iter().collect();
    let ls: String = lower.iter().collect();
    if let Some(b) = ls.find(&qs) {
        let start = ls[..b].chars().count();
        return (start..start + q.len()).collect();
    }
    let mut pos = Vec::new();
    let mut qi = 0;
    for (i, c) in lower.iter().enumerate() {
        if qi < q.len() && *c == q[qi] {
            pos.push(i);
            qi += 1;
        }
    }
    if qi == q.len() {
        pos
    } else {
        Vec::new()
    }
}

fn draw_results(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.list_area = area;
    let parsed = query::parse(app.library, &app.kinds, &app.query, None);
    let show_docset = app.docset.is_none() && parsed.docset.is_none();
    let inner_w = area.width.saturating_sub(2) as usize;
    let items: Vec<ListItem> = app
        .results
        .iter()
        .map(|hit| {
            let tag = render::sym_kind(&hit.kind, &hit.path);
            let kc = rgb(render::kind_color(&tag));
            let docset_label = if show_docset {
                short_docset(&hit.docset_id)
            } else {
                String::new()
            };
            let right_w = docset_label.width() + usize::from(!docset_label.is_empty());
            let name_w = inner_w.saturating_sub(8 + 2 + right_w).max(8);
            let name = ui::truncate_start(&hit.name, name_w);
            let pos = match_positions(&name, &parsed.text);
            let mut spans = vec![Span::styled(
                format!("{:<6}", ui::truncate_end(&tag, 6)),
                Style::default().fg(kc),
            )];
            spans.push(Span::raw(" "));
            for (i, ch) in name.chars().enumerate() {
                let st = if pos.contains(&i) {
                    Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(C_TEXT)
                };
                spans.push(Span::styled(ch.to_string(), st));
            }
            if show_docset {
                let used = 7 + name.width();
                let pad = inner_w.saturating_sub(used + 2 + docset_label.width());
                spans.push(Span::raw(" ".repeat(pad + 1)));
                spans.push(Span::styled(docset_label, Style::default().fg(C_MUTED)));
            }
            ListItem::new(RLine::from(spans))
        })
        .collect();

    let active = app.focus == Focus::Search && matches!(app.overlay, Overlay::None);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if active { C_ACCENT } else { C_BORDER }))
        .title(Span::styled(" Resultados ", Style::default().fg(C_MUTED)));
    if app.results.is_empty() {
        let msg = if app.query.trim().is_empty() {
            "No hay entradas."
        } else {
            "Sin coincidencias."
        };
        f.render_widget(
            Paragraph::new(msg)
                .style(Style::default().fg(C_MUTED))
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
        return;
    }
    let list = List::new(items)
        .block(block)
        .highlight_style(Style::default().bg(C_SURFACE))
        .highlight_symbol("▌");
    f.render_stateful_widget(list, area, &mut app.list);
    // Barra de desplazamiento de la lista.
    if app.results.len() > area.height.saturating_sub(2) as usize {
        let mut sb =
            ScrollbarState::new(app.results.len()).position(app.list.selected().unwrap_or(0));
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None)
                .thumb_style(Style::default().fg(C_BORDER)),
            area.inner(Margin::new(0, 1)),
            &mut sb,
        );
    }
}

fn short_docset(id: &str) -> String {
    id.strip_prefix("devdocs-").unwrap_or(id).to_string()
}

fn draw_reader(f: &mut ratatui::Frame, app: &mut App, area: Rect) {
    app.reader_area = area;
    let active = app.focus == Focus::Reader && matches!(app.overlay, Overlay::None);
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if active { C_ACCENT } else { C_BORDER }));
    let inner = block.inner(area);
    app.reader_height = inner.height as usize;
    let width = (inner.width.saturating_sub(3) as usize).clamp(24, READ_WIDTH);
    let lay = app.layout_for(width);
    // Si cambió el ancho o el documento, recalcula las coincidencias.
    let stale = match (&app.find, &app.view) {
        (Some(f), Some(v)) => f
            .stamp
            .as_ref()
            .is_none_or(|(d, w)| !Rc::ptr_eq(d, &v.doc) || *w != width),
        _ => false,
    };
    if stale {
        app.find_refresh(false);
    }
    let favorite = app.is_favorite();

    let (Some(lay), Some(view)) = (lay, app.view.as_ref()) else {
        block = block.title(Span::styled(" Lector ", Style::default().fg(C_MUTED)));
        let hint = if app.results.is_empty() {
            "Nada que mostrar".to_string()
        } else {
            "Cargando…".to_string()
        };
        f.render_widget(
            Paragraph::new(hint)
                .style(Style::default().fg(C_MUTED))
                .alignment(Alignment::Center)
                .block(block),
            area,
        );
        return;
    };

    let total = lay.lines.len();
    let height = inner.height as usize;
    let max_scroll = total.saturating_sub(height);
    let scroll = view.scroll.min(max_scroll);
    let pct = (scroll * 100).checked_div(max_scroll).unwrap_or(100);
    let star = if favorite { "★ " } else { "" };
    let crumb = if app.history.is_empty() {
        format!(" {star}{} ", view.name)
    } else {
        format!(" ‹{} · {star}{} ", app.history.len(), view.name)
    };
    block = block
        .title(Span::styled(
            ui::truncate_end(&crumb, area.width.saturating_sub(12) as usize),
            Style::default().fg(C_TEXT).add_modifier(Modifier::BOLD),
        ))
        .title_bottom(
            RLine::from(Span::styled(
                format!(" {pct}% "),
                Style::default().fg(C_MUTED),
            ))
            .alignment(Alignment::Right),
        );

    let sel_link = view.link;
    let sel = app.sel;
    let find = app.find.as_ref();
    let lines: Vec<RLine> = lay
        .lines
        .iter()
        .enumerate()
        .skip(scroll)
        .take(height)
        .map(|(i, l)| {
            let marks: Vec<(usize, usize, bool)> = find
                .map(|f| {
                    let start = f.matches.partition_point(|m| m.0 < i);
                    f.matches[start..]
                        .iter()
                        .take_while(|m| m.0 == i)
                        .enumerate()
                        .map(|(k, m)| (m.1, m.2, start + k == f.cur))
                        .collect()
                })
                .unwrap_or_default();
            seg_line(l, sel_link, sel.and_then(|s| s.range_on(i)), &marks)
        })
        .collect();
    let para = Paragraph::new(lines).block(block);
    f.render_widget(para, area);
    // Margen de lectura: relleno de fondo uniforme.
    if total > height {
        let mut sb = ScrollbarState::new(max_scroll).position(scroll);
        f.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .track_style(Style::default().fg(C_SURFACE))
                .thumb_style(Style::default().fg(C_ACCENT)),
            area.inner(Margin::new(0, 1)),
            &mut sb,
        );
    }
}

const C_MATCH: Color = Color::Rgb(249, 226, 175);
const C_CURRENT: Color = Color::Rgb(250, 179, 135);

/// Una línea del lector con enlace, selección y coincidencias de búsqueda.
fn seg_line(
    l: &[Seg],
    sel_link: Option<usize>,
    sel: Option<(usize, usize)>,
    marks: &[(usize, usize, bool)],
) -> RLine<'static> {
    let mut spans = Vec::with_capacity(l.len() + 1);
    spans.push(Span::raw(" "));
    let mut col = 0usize;
    for s in l {
        let mut st = seg_style(&s.sty);
        if s.link.is_some() && s.link == sel_link {
            st = st
                .bg(C_ACCENT)
                .fg(C_MANTLE)
                .remove_modifier(Modifier::UNDERLINED)
                .add_modifier(Modifier::BOLD);
        }
        if sel.is_none() && marks.is_empty() {
            spans.push(Span::styled(s.text.clone(), st));
            continue;
        }
        // Clase por carácter: 0 normal · 1 selección · 2 coincidencia · 3 actual.
        let class_at = |c: usize, w: usize| -> u8 {
            if marks.iter().any(|m| m.2 && c + w > m.0 && c < m.1) {
                3
            } else if marks.iter().any(|m| c + w > m.0 && c < m.1) {
                2
            } else if sel.is_some_and(|(from, to)| c + w > from && c < to) {
                1
            } else {
                0
            }
        };
        let style_of = |class: u8| match class {
            3 => st.bg(C_CURRENT).fg(C_MANTLE).add_modifier(Modifier::BOLD),
            2 => st.bg(C_MATCH).fg(C_MANTLE),
            1 => st.bg(C_SELECT).fg(C_TEXT),
            _ => st,
        };
        let mut run = String::new();
        let mut run_class = 0u8;
        for ch in s.text.chars() {
            let w = unicode_width::UnicodeWidthChar::width(ch).unwrap_or(0);
            let class = class_at(col, w);
            if class != run_class && !run.is_empty() {
                spans.push(Span::styled(std::mem::take(&mut run), style_of(run_class)));
            }
            run_class = class;
            run.push(ch);
            col += w;
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, style_of(run_class)));
        }
    }
    RLine::from(spans)
}

fn draw_footer(f: &mut ratatui::Frame, app: &App, area: Rect) {
    if let Some((msg, at, warn)) = &app.status {
        if at.elapsed() < Duration::from_secs(3) {
            let (fg, icon) = if *warn {
                (C_RED, "✗")
            } else {
                (C_GREEN, "✓")
            };
            f.render_widget(
                Paragraph::new(RLine::from(vec![
                    Span::styled(
                        format!(" {icon} "),
                        Style::default().fg(fg).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(msg.clone(), Style::default().fg(fg)),
                ])),
                area,
            );
            return;
        }
    }
    if let Some(fd) = app.find.as_ref().filter(|f| f.editing) {
        let info = if fd.input.is_empty() {
            String::new()
        } else if fd.matches.is_empty() {
            "  sin coincidencias".to_string()
        } else {
            format!("  {}/{}", fd.cur + 1, fd.matches.len())
        };
        f.render_widget(
            Paragraph::new(RLine::from(vec![
                Span::styled(
                    " / ",
                    Style::default()
                        .fg(C_MANTLE)
                        .bg(C_MATCH)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(format!(" {}", fd.input), Style::default().fg(C_TEXT)),
                Span::styled("▏", Style::default().fg(C_ACCENT)),
                Span::styled(info, Style::default().fg(C_MUTED)),
                Span::styled(
                    "   Enter fijar · Esc cancelar · ↑↓ siguiente",
                    Style::default().fg(C_MUTED),
                ),
            ])),
            area,
        );
        return;
    }
    let mut spans: Vec<Span> = vec![Span::raw(" ")];
    if let Some(fd) = app.find.as_ref().filter(|f| !f.matches.is_empty()) {
        spans.push(chip(
            &format!(
                "/{} {}/{}",
                ui::truncate_end(&fd.input, 16),
                fd.cur + 1,
                fd.matches.len()
            ),
            C_MANTLE,
            C_MATCH,
        ));
        spans.push(Span::raw(" "));
    }
    match app.focus {
        Focus::Search => {
            for (k, l) in [
                ("↑↓", "navegar"),
                ("Enter", "leer"),
                ("^D", "docset"),
                ("^T", "tipo"),
                ("^H/F2", "recientes"),
                ("^F", "★"),
                ("^Y", "copiar"),
                ("F1", "ayuda"),
                ("Esc", "limpiar/salir"),
            ] {
                spans.extend(key_hint(k, l));
            }
        }
        Focus::Reader => {
            for (k, l) in [
                ("/", "buscar"),
                ("O", "índice"),
                ("r", "relacionados"),
                ("m", "★"),
                ("H", "recientes"),
                ("Tab", "enlaces"),
                ("⌫", "atrás"),
                ("y", "copiar"),
                ("i", "símbolos"),
                ("?", "ayuda"),
            ] {
                spans.extend(key_hint(k, l));
            }
        }
    }
    f.render_widget(Paragraph::new(RLine::from(spans)), area);
}

fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width);
    let h = h.min(area.height);
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

fn draw_help(f: &mut ratatui::Frame, area: Rect) {
    let rect = centered(area, 78, 38);
    f.render_widget(Clear, rect);
    let h = |s: &str| {
        RLine::from(Span::styled(
            s.to_string(),
            Style::default().fg(C_ACCENT).add_modifier(Modifier::BOLD),
        ))
    };
    let k = |key: &str, desc: &str| {
        RLine::from(vec![
            Span::styled(
                format!("  {key:<19}"),
                Style::default().fg(Color::Rgb(250, 179, 135)),
            ),
            Span::styled(desc.to_string(), Style::default().fg(C_TEXT)),
        ])
    };
    let lines = vec![
        h("Búsqueda"),
        k("texto", "búsqueda difusa en vivo"),
        k("rust:Vec::push", "restringe al docset (py:, js:, ts:…)"),
        k("fn:map  struct:", "restringe al tipo de símbolo"),
        k("↑↓  ^N ^P", "mover selección"),
        k("Enter  Tab  →", "leer el símbolo"),
        k("^D  ^T  S-Tab", "elegir docset / tipo · ciclar docset"),
        k("^H  F2  Alt+H", "favoritos y recientes (Supr olvida)"),
        k("^F  ^O  ^R", "★ favorito · índice · relacionados"),
        k("^Y  ^B", "copiar ejemplo · abrir en navegador"),
        k("^W  ^U", "borrar palabra / línea"),
        RLine::from(""),
        h("Lector"),
        k("j k  ↑↓  Space b", "desplazar línea / página"),
        k("^D ^U  gg  G", "media página · inicio · fin"),
        k("/  n  N", "buscar en la página · siguiente · anterior"),
        k("O  gO  ^O", "índice de secciones (Enter salta)"),
        k("r", "relacionados (otros métodos del mismo tipo)"),
        k("m", "marcar / quitar favorito ★"),
        k("H  ^H  F2", "favoritos y recientes"),
        k("[ ]", "sección anterior / siguiente"),
        k(
            "Tab  S-Tab  Enter",
            "enlaces: siguiente / anterior / seguir",
        ),
        k("⌫  h  Esc", "volver atrás en el historial"),
        k("y  Y  s", "copiar ejemplo / todo / firma"),
        k("o  d  t", "navegador · docset · tipo"),
        k("i", "volver a la búsqueda"),
        k("q  ^C", "salir"),
        RLine::from(""),
        RLine::from(Span::styled(
            "  Ratón: arrastra para copiar · doble clic palabra · triple línea",
            Style::default().fg(C_MUTED),
        )),
        RLine::from(Span::styled(
            "  Pulsa cualquier tecla para cerrar",
            Style::default().fg(C_MUTED),
        )),
    ];
    f.render_widget(
        Paragraph::new(lines)
            .style(Style::default().bg(C_MANTLE))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(C_ACCENT))
                    .title(" Atajos ")
                    .style(Style::default().bg(C_MANTLE)),
            ),
        rect,
    );
}

fn draw_picker(f: &mut ratatui::Frame, p: &Picker, area: Rect) {
    let visible = p.visible();
    let h = (visible.len() as u16 + 4).clamp(6, area.height.saturating_sub(4));
    let wide = !matches!(p.kind, PickKind::Docset | PickKind::Kind);
    let rect = centered(area, if wide { 84 } else { 56 }, h);
    f.render_widget(Clear, rect);
    let title = p.title.as_str();
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(C_ACCENT))
        .title(title)
        .style(Style::default().bg(C_MANTLE));
    let inner = block.inner(rect);
    f.render_widget(block, rect);
    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(inner);
    f.render_widget(
        Paragraph::new(RLine::from(vec![
            Span::styled("› ", Style::default().fg(C_ACCENT)),
            Span::styled(p.filter.clone(), Style::default().fg(C_TEXT)),
            Span::styled("▏", Style::default().fg(C_ACCENT)),
        ])),
        parts[0],
    );
    let items: Vec<ListItem> = visible
        .iter()
        .map(|(l, _)| ListItem::new(Span::styled(l.clone(), Style::default().fg(C_TEXT))))
        .collect();
    let mut st = ListState::default();
    st.select(if visible.is_empty() {
        None
    } else {
        Some(p.sel.min(visible.len() - 1))
    });
    f.render_stateful_widget(
        List::new(items)
            .highlight_style(
                Style::default()
                    .bg(C_SURFACE)
                    .fg(C_ACCENT)
                    .add_modifier(Modifier::BOLD),
            )
            .highlight_symbol("▌"),
        parts[1],
        &mut st,
    );
}

// Alias para evitar confusión con `render::Line` en las firmas anteriores.
use ratatui::text::Line;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_resolves_parent_dirs() {
        assert_eq!(
            normalize(Path::new("std/vec/../macro.vec.html")),
            "std/macro.vec.html"
        );
        assert_eq!(normalize(Path::new("./a/b.html")), "a/b.html");
    }

    #[test]
    fn find_matches_smart_case_and_columns() {
        let doc = render::parse_doc(
            "<main><p>Vec y vec, VEC</p><p>otra vec</p></main>",
            None,
            "T",
            "guide",
            "x",
        );
        let lay = doc.layout(60);
        // Minúsculas: ignora la caja.
        let m = compute_matches(&lay, "vec");
        assert_eq!(m.len(), 4);
        assert!(m.windows(2).all(|w| w[0] <= w[1]));
        let (l, from, to) = m[0];
        let line = line_text(&lay, l);
        let txt: String = line
            .iter()
            .filter(|(_, c)| *c >= from && *c < to)
            .map(|(c, _)| *c)
            .collect();
        assert_eq!(txt, "Vec");
        // Con mayúsculas: distingue la caja.
        assert_eq!(compute_matches(&lay, "VEC").len(), 1);
        assert!(compute_matches(&lay, "").is_empty());
    }

    #[test]
    fn item_encoding_roundtrip() {
        let it = Item {
            docset: "d".into(),
            path: "a/b.html#x".into(),
            name: "Vec::push".into(),
            kind: "fn".into(),
        };
        assert_eq!(dec(&enc(&it)), Some(it));
    }

    #[test]
    fn match_positions_prefers_contiguous() {
        assert_eq!(match_positions("Vec::push", "push"), vec![5, 6, 7, 8]);
        assert_eq!(match_positions("hashmap", "hmp"), vec![0, 4, 6]);
        assert!(match_positions("abc", "zzz").is_empty());
    }
}
