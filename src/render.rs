//! Motor de renderizado HTML → terminal.
//!
//! El HTML del docset se convierte primero en una representación
//! intermedia de bloques (`Block`), independiente del ancho. Después
//! `Doc::layout(ancho)` la ajusta a líneas con estilo (`Seg`), que tanto la
//! TUI (ratatui) como la CLI (ANSI de 24 bits) consumen. Así, ambas
//! muestran exactamente lo mismo y la TUI puede re-maquetar al
//! redimensionar sin volver a parsear el HTML.

use std::collections::HashSet;
use std::sync::OnceLock;

use ego_tree::NodeRef;
use scraper::{ElementRef, Html, Node};
use syntect::easy::HighlightLines;
use syntect::highlighting::ThemeSet;
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::ui;

// ───────────────────────── Paleta (Catppuccin Mocha) ─────────────────────────

pub type Rgb = [u8; 3];
pub const TEXT: Rgb = [205, 214, 244];
pub const SUBTEXT: Rgb = [166, 173, 200];
pub const MUTED: Rgb = [127, 132, 156];
pub const MAUVE: Rgb = [203, 166, 247];
pub const BLUE: Rgb = [137, 180, 250];
pub const SKY: Rgb = [137, 220, 235];
pub const TEAL: Rgb = [148, 226, 213];
pub const GREEN: Rgb = [166, 227, 161];
pub const YELLOW: Rgb = [249, 226, 175];
pub const PEACH: Rgb = [250, 179, 135];
pub const RED: Rgb = [243, 139, 168];
pub const PINK: Rgb = [245, 194, 231];
pub const SURFACE: Rgb = [49, 50, 68];
pub const MANTLE: Rgb = [24, 24, 37];

// ───────────────────────── Texto con estilo ─────────────────────────

/// Estilo de un fragmento de línea.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct Sty {
    pub fg: Option<Rgb>,
    pub bg: Option<Rgb>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub dim: bool,
}

impl Sty {
    pub fn fg(c: Rgb) -> Self {
        Self {
            fg: Some(c),
            ..Self::default()
        }
    }
    pub fn bold(mut self) -> Self {
        self.bold = true;
        self
    }
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    pub fn bg(mut self, c: Rgb) -> Self {
        self.bg = Some(c);
        self
    }
}

/// Fragmento de texto con un mismo estilo (y, opcionalmente, un enlace).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Seg {
    pub text: String,
    pub sty: Sty,
    /// Índice en `Doc::links` si el fragmento forma parte de un enlace.
    pub link: Option<usize>,
}

impl Seg {
    pub fn new(text: impl Into<String>, sty: Sty) -> Self {
        Self {
            text: text.into(),
            sty,
            link: None,
        }
    }
}

pub type Line = Vec<Seg>;

fn line_width(line: &Line) -> usize {
    line.iter().map(|s| s.text.width()).sum()
}

// ───────────────────────── Representación intermedia ─────────────────────────

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct InlFlags {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
    link: Option<usize>,
}

#[derive(Clone, Debug)]
struct Inl {
    text: String,
    flags: InlFlags,
}

#[derive(Clone, Debug)]
enum Block {
    Heading(u8, Vec<Inl>),
    Para(Vec<Inl>),
    Code {
        lang: String,
        text: String,
    },
    List {
        ordered: bool,
        items: Vec<Vec<Block>>,
    },
    Quote(Vec<Block>),
    Indent(Vec<Block>),
    Table(Vec<Vec<Vec<Inl>>>),
    Rule,
}

/// Documento ya extraído del HTML, listo para maquetar a cualquier ancho.
#[derive(Debug, Clone)]
pub struct Doc {
    pub title: String,
    pub kind: String,
    pub docset: String,
    /// Firma del símbolo (`pub fn push(&mut self, value: T)`).
    pub signature: Option<String>,
    /// Lenguaje para resaltar (derivado del docset o del HTML).
    pub lang: String,
    /// Nota breve (p. ej. «Stable since Rust version 1.0.0»).
    pub meta: Option<String>,
    blocks: Vec<Block>,
    /// Bloques de código de ejemplo (texto plano, para copiar).
    pub code_blocks: Vec<String>,
    /// Destinos (`href`) de los enlaces, indexados por `Seg::link`.
    pub links: Vec<String>,
}

/// Resultado de maquetar un `Doc`.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub lines: Vec<Line>,
    /// Línea de inicio de cada enlace (índice paralelo a `Doc::links`).
    pub link_lines: Vec<Option<usize>>,
    /// Encabezados `(línea, nivel, texto)` para saltar entre secciones.
    pub headings: Vec<(usize, u8, String)>,
}

// ───────────────────────── Extracción del HTML ─────────────────────────

const SKIP_TAGS: &[&str] = &[
    "script", "style", "head", "nav", "svg", "button", "noscript", "template", "iframe", "form",
    "input", "select", "footer",
];
const SKIP_CLASSES: &[&str] = &[
    "rightside",
    "src",
    "anchor",
    "copy-button",
    "sidebar",
    "toggle-wrapper",
    "collapse-toggle",
    "headerlink",
    "sidebar-elems",
    "_sidebar",
    "mobile-topbar",
    "search-form",
    "edit-on-github",
    "_attribution",
];

/// Extrae y estructura la documentación de un archivo HTML.
pub fn parse_doc(
    html_raw: &str,
    anchor: Option<&str>,
    name: &str,
    kind: &str,
    docset_id: &str,
) -> Doc {
    let html = Html::parse_document(html_raw);
    let mut lang = lang_from_docset(docset_id);
    let mut skip: HashSet<ego_tree::NodeId> = HashSet::new();
    let mut signature = None;
    let mut meta = None;

    let roots = anchor
        .map(|a| anchor_nodes(&html, a))
        .filter(|r| !r.is_empty());

    let nodes: Vec<NodeRef<Node>> = match roots {
        Some(r) => {
            for n in &r {
                if signature.is_none() {
                    if let Some(sig) = find_signature(*n, &mut skip, ".code-header") {
                        signature = Some(sig);
                    }
                }
                if meta.is_none() {
                    meta = find_since(*n);
                }
            }
            r
        }
        None => {
            let main = main_container(&html);
            if let Some(m) = main {
                signature = find_signature(m, &mut skip, ".item-decl");
                meta = find_since(m);
                vec![m]
            } else {
                vec![html.tree.root()]
            }
        }
    };

    // Lenguaje explícito en el HTML (p. ej. `data-language="rust"`).
    if let Some(l) = nodes.iter().find_map(|n| first_pre_lang(*n)) {
        lang = l;
    }

    let mut cx = Cx {
        links: Vec::new(),
        skip,
        lang: lang.clone(),
    };
    let mut blocks = Vec::new();
    let mut inl = Vec::new();
    for n in &nodes {
        // Si el nodo raíz es un contenedor, se recorre su contenido.
        walk(*n, &mut blocks, &mut inl, InlFlags::default(), &mut cx);
    }
    flush(&mut blocks, &mut inl);
    let mut code_blocks = Vec::new();
    collect_code(&blocks, &mut code_blocks);

    Doc {
        title: name.to_string(),
        kind: kind.to_string(),
        docset: docset_id.to_string(),
        signature,
        lang,
        meta,
        blocks,
        code_blocks,
        links: cx.links,
    }
}

fn main_container(html: &Html) -> Option<NodeRef<'_, Node>> {
    const SELECTORS: &[&str] = &[
        "main",
        "article",
        "[role='main']",
        "._page",
        "._content",
        "#main-content",
        ".document",
        ".content",
        "body",
    ];
    for s in SELECTORS {
        if let Ok(sel) = scraper::Selector::parse(s) {
            if let Some(el) = html.select(&sel).next() {
                return Some(*el);
            }
        }
    }
    None
}

fn el(n: NodeRef<Node>) -> Option<ElementRef> {
    ElementRef::wrap(n)
}

fn tag<'a>(n: NodeRef<'a, Node>) -> Option<&'a str> {
    n.value().as_element().map(|e| e.name())
}

fn has_class(n: NodeRef<Node>, class: &str) -> bool {
    n.value()
        .as_element()
        .is_some_and(|e| e.classes().any(|c| c == class))
}

fn heading_level(n: NodeRef<Node>) -> Option<u8> {
    match tag(n)? {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}

/// Localiza el elemento de un fragmento y los hermanos que lo acompañan
/// (la descripción de un método rustdoc o devdocs vive *después* de la
/// firma, como hermano, no dentro).
fn anchor_nodes<'a>(html: &'a Html, anchor: &str) -> Vec<NodeRef<'a, Node>> {
    let frag = percent_decode(anchor.trim_start_matches('#'));
    if frag.is_empty() {
        return Vec::new();
    }
    let found = html.tree.nodes().find(|n| {
        n.value().as_element().is_some_and(|e| {
            e.attr("id") == Some(frag.as_str())
                || (e.name() == "a" && e.attr("name") == Some(frag.as_str()))
        })
    });
    let Some(mut start) = found else {
        return Vec::new();
    };

    // Subir mientras el padre sea un envoltorio que no aporta (summary…).
    while let Some(parent) = start.parent() {
        match tag(parent) {
            Some("summary") | Some("dt") => start = parent,
            Some("section") if tag(start) != Some("section") && has_class(parent, "method") => {
                start = parent
            }
            _ => break,
        }
    }

    let mut out = vec![start];
    let lvl = heading_level(start);
    let is_dt = tag(start) == Some("dt");
    let is_block_head = matches!(tag(start), Some("summary") | Some("section"));
    if lvl.is_none() && !is_dt && !is_block_head {
        // Un id suelto en un párrafo, lista, etc.: solo ese elemento.
        return out;
    }
    for sib in start.next_siblings() {
        if el(sib).is_none() {
            continue;
        }
        let t = tag(sib).unwrap_or("");
        if let Some(l) = lvl {
            if heading_level(sib).is_some_and(|sl| sl <= l) {
                break;
            }
        } else if is_dt {
            if t == "dt" {
                break;
            }
        } else if t == "summary"
            || t == "details"
            || heading_level(sib).is_some_and(|l| l <= 4)
            || (t == "section"
                && sib
                    .value()
                    .as_element()
                    .is_some_and(|e| e.attr("id").is_some()))
        {
            break;
        }
        out.push(sib);
    }
    out
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find_signature(
    root: NodeRef<Node>,
    skip: &mut HashSet<ego_tree::NodeId>,
    selector: &str,
) -> Option<String> {
    let sel = scraper::Selector::parse(selector).ok()?;
    let root_el = el(root)?;
    let found = root_el.select(&sel).next().or_else(|| {
        // Rustdoc: el propio nodo puede ser la firma.
        sel.matches(&root_el).then_some(root_el)
    })?;
    let text: String = found.text().collect();
    let text = text.trim().to_string();
    if text.is_empty() {
        return None;
    }
    skip.insert(found.id());
    Some(text)
}

fn find_since(root: NodeRef<Node>) -> Option<String> {
    let sel = scraper::Selector::parse(".since").ok()?;
    let e = el(root)?;
    let s = e.select(&sel).next()?;
    s.value()
        .attr("title")
        .map(str::to_string)
        .or_else(|| Some(s.text().collect::<String>().trim().to_string()))
        .filter(|t| !t.is_empty())
}

fn first_pre_lang(root: NodeRef<Node>) -> Option<String> {
    let e = el(root)?;
    let sel = scraper::Selector::parse("pre[data-language]").ok()?;
    e.select(&sel)
        .next()
        .and_then(|p| p.value().attr("data-language"))
        .map(str::to_string)
}

struct Cx {
    links: Vec<String>,
    skip: HashSet<ego_tree::NodeId>,
    lang: String,
}

fn flush(out: &mut Vec<Block>, inl: &mut Vec<Inl>) {
    if inl.is_empty() {
        return;
    }
    let mut v = std::mem::take(inl);
    // Recortar espacios en los extremos.
    if let Some(first) = v.first_mut() {
        first.text = first.text.trim_start().to_string();
    }
    if let Some(last) = v.last_mut() {
        last.text = last.text.trim_end().to_string();
    }
    v.retain(|i| !i.text.is_empty());
    if !v.is_empty() {
        out.push(Block::Para(v));
    }
}

fn push_text(inl: &mut Vec<Inl>, text: &str, flags: InlFlags) {
    if text.is_empty() {
        return;
    }
    let mut t = text.to_string();
    if !flags.code {
        // Colapsa espacios en blanco (HTML) salvo en código en línea.
        let mut collapsed = String::with_capacity(t.len());
        let mut prev_space = inl.last().is_some_and(|l| l.text.ends_with(' '));
        for c in t.chars() {
            if c.is_whitespace() && c != '\u{a0}' {
                if !prev_space {
                    collapsed.push(' ');
                }
                prev_space = true;
            } else {
                collapsed.push(c);
                prev_space = false;
            }
        }
        t = collapsed;
        if t.is_empty() {
            return;
        }
    }
    if let Some(last) = inl.last_mut() {
        if last.flags == flags {
            last.text.push_str(&t);
            return;
        }
    }
    inl.push(Inl { text: t, flags });
}

fn inline_text(n: NodeRef<Node>, out: &mut String) {
    match n.value() {
        Node::Text(t) => out.push_str(t),
        Node::Element(e) => {
            if e.name() == "br" {
                out.push('\n');
            } else if !SKIP_TAGS.contains(&e.name()) {
                for c in n.children() {
                    inline_text(c, out);
                }
            }
        }
        _ => {}
    }
}

fn should_skip(n: NodeRef<Node>, cx: &Cx) -> bool {
    if cx.skip.contains(&n.id()) {
        return true;
    }
    let Some(e) = n.value().as_element() else {
        return false;
    };
    SKIP_TAGS.contains(&e.name()) || e.classes().any(|c| SKIP_CLASSES.contains(&c))
}

fn walk(n: NodeRef<Node>, out: &mut Vec<Block>, inl: &mut Vec<Inl>, flags: InlFlags, cx: &mut Cx) {
    match n.value() {
        Node::Text(t) => push_text(inl, t, flags),
        Node::Element(e) => {
            if should_skip(n, cx) {
                return;
            }
            let name = e.name();
            let recurse = |out: &mut Vec<Block>, inl: &mut Vec<Inl>, f: InlFlags, cx: &mut Cx| {
                for c in n.children() {
                    walk(c, out, inl, f, cx);
                }
            };
            match name {
                "b" | "strong" => recurse(
                    out,
                    inl,
                    InlFlags {
                        bold: true,
                        ..flags
                    },
                    cx,
                ),
                "i" | "em" | "cite" | "var" => recurse(
                    out,
                    inl,
                    InlFlags {
                        italic: true,
                        ..flags
                    },
                    cx,
                ),
                "s" | "del" | "strike" => recurse(
                    out,
                    inl,
                    InlFlags {
                        strike: true,
                        ..flags
                    },
                    cx,
                ),
                "code" | "kbd" | "samp" | "tt" => {
                    let f = InlFlags {
                        code: true,
                        ..flags
                    };
                    recurse(out, inl, f, cx)
                }
                "a" => {
                    let href = e.attr("href").unwrap_or("").to_string();
                    if href.is_empty() || href.starts_with("javascript:") {
                        recurse(out, inl, flags, cx);
                    } else {
                        cx.links.push(href);
                        let f = InlFlags {
                            link: Some(cx.links.len() - 1),
                            ..flags
                        };
                        recurse(out, inl, f, cx);
                    }
                }
                "br" => push_text(
                    inl,
                    "\n",
                    InlFlags {
                        code: true,
                        ..flags
                    },
                ),
                "img" => {
                    if let Some(alt) = e.attr("alt").filter(|a| !a.is_empty()) {
                        push_text(
                            inl,
                            &format!("[{alt}]"),
                            InlFlags {
                                italic: true,
                                ..flags
                            },
                        );
                    }
                }
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush(out, inl);
                    let lvl = heading_level(n).unwrap_or(2);
                    let mut h = Vec::new();
                    for c in n.children() {
                        walk(c, out, &mut h, InlFlags::default(), cx);
                    }
                    let mut tmp = Vec::new();
                    flush(&mut tmp, &mut h);
                    if let Some(Block::Para(v)) = tmp.pop() {
                        out.push(Block::Heading(lvl, v));
                    }
                }
                "pre" => {
                    flush(out, inl);
                    let mut text = String::new();
                    inline_text(n, &mut text);
                    let text = text.trim_matches('\n').trim_end().to_string();
                    if !text.trim().is_empty() {
                        let lang = e
                            .attr("data-language")
                            .map(str::to_string)
                            .or_else(|| lang_from_classes(n))
                            .unwrap_or_else(|| cx.lang.clone());
                        out.push(Block::Code { lang, text });
                    }
                }
                "ul" | "ol" => {
                    flush(out, inl);
                    let mut items = Vec::new();
                    for c in n.children() {
                        if tag(c) == Some("li") && !should_skip(c, cx) {
                            let mut b = Vec::new();
                            let mut i2 = Vec::new();
                            for cc in c.children() {
                                walk(cc, &mut b, &mut i2, InlFlags::default(), cx);
                            }
                            flush(&mut b, &mut i2);
                            if !b.is_empty() {
                                items.push(b);
                            }
                        }
                    }
                    if !items.is_empty() {
                        out.push(Block::List {
                            ordered: name == "ol",
                            items,
                        });
                    }
                }
                "dt" => {
                    flush(out, inl);
                    let mut i2 = Vec::new();
                    for c in n.children() {
                        walk(
                            c,
                            out,
                            &mut i2,
                            InlFlags {
                                bold: true,
                                ..flags
                            },
                            cx,
                        );
                    }
                    flush(out, &mut i2);
                }
                "dd" => {
                    flush(out, inl);
                    let mut b = Vec::new();
                    let mut i2 = Vec::new();
                    for c in n.children() {
                        walk(c, &mut b, &mut i2, flags, cx);
                    }
                    flush(&mut b, &mut i2);
                    if !b.is_empty() {
                        out.push(Block::Indent(b));
                    }
                }
                "blockquote" => {
                    flush(out, inl);
                    let mut b = Vec::new();
                    let mut i2 = Vec::new();
                    for c in n.children() {
                        walk(c, &mut b, &mut i2, flags, cx);
                    }
                    flush(&mut b, &mut i2);
                    if !b.is_empty() {
                        out.push(Block::Quote(b));
                    }
                }
                "table" => {
                    flush(out, inl);
                    let mut rows = Vec::new();
                    collect_rows(n, &mut rows, cx);
                    if !rows.is_empty() {
                        out.push(Block::Table(rows));
                    }
                }
                "hr" => {
                    flush(out, inl);
                    out.push(Block::Rule);
                }
                "p" | "div" | "section" | "article" | "main" | "header" | "details" | "summary"
                | "li" | "figure" | "figcaption" | "aside" | "body" | "html" | "address"
                | "center" | "dl" | "tr" | "td" | "th" | "tbody" | "thead" | "caption"
                | "fieldset" | "legend" => {
                    flush(out, inl);
                    recurse(out, inl, flags, cx);
                    flush(out, inl);
                }
                _ => recurse(out, inl, flags, cx),
            }
        }
        Node::Document => {
            for c in n.children() {
                walk(c, out, inl, flags, cx);
            }
        }
        _ => {}
    }
}

fn collect_rows(n: NodeRef<Node>, rows: &mut Vec<Vec<Vec<Inl>>>, cx: &mut Cx) {
    for c in n.children() {
        match tag(c) {
            Some("tr") => {
                let mut row = Vec::new();
                for cell in c.children() {
                    if matches!(tag(cell), Some("td") | Some("th")) {
                        let mut b = Vec::new();
                        let mut i2 = Vec::new();
                        walk(cell, &mut b, &mut i2, InlFlags::default(), cx);
                        flush(&mut b, &mut i2);
                        let mut cell_inl = Vec::new();
                        for blk in b {
                            if let Block::Para(v) = blk {
                                if !cell_inl.is_empty() {
                                    cell_inl.push(Inl {
                                        text: " ".into(),
                                        flags: InlFlags::default(),
                                    });
                                }
                                cell_inl.extend(v);
                            }
                        }
                        row.push(cell_inl);
                    }
                }
                if !row.is_empty() {
                    rows.push(row);
                }
            }
            Some("thead") | Some("tbody") | Some("tfoot") => collect_rows(c, rows, cx),
            _ => {}
        }
    }
}

fn lang_from_classes(n: NodeRef<Node>) -> Option<String> {
    let mut stack = vec![n];
    while let Some(cur) = stack.pop() {
        if let Some(e) = cur.value().as_element() {
            for c in e.classes() {
                for pre in ["language-", "lang-", "highlight-source-", "brush: "] {
                    if let Some(l) = c.strip_prefix(pre) {
                        return Some(l.to_string());
                    }
                }
            }
            if let Some(l) = e.attr("data-lang") {
                return Some(l.to_string());
            }
        }
        stack.extend(cur.children());
        if stack.len() > 8 {
            break;
        }
    }
    None
}

fn collect_code(blocks: &[Block], out: &mut Vec<String>) {
    for b in blocks {
        match b {
            Block::Code { text, .. } => {
                if !out.contains(text) {
                    out.push(text.clone());
                }
            }
            Block::List { items, .. } => items.iter().for_each(|i| collect_code(i, out)),
            Block::Quote(v) | Block::Indent(v) => collect_code(v, out),
            _ => {}
        }
    }
}

// ───────────────────────── Maquetación ─────────────────────────

impl Doc {
    /// Maqueta el documento (cabecera incluida) al ancho indicado.
    pub fn layout(&self, width: usize) -> Layout {
        let width = width.max(20);
        let mut lay = Layout {
            link_lines: vec![None; self.links.len()],
            ..Layout::default()
        };
        self.header(width, &mut lay);
        let mut lines = Vec::new();
        let base = lay.lines.len();
        layout_blocks(&self.blocks, width, &mut lines, &mut lay, 0, base);
        lay.lines.extend(lines);
        // Primera línea de cada enlace (sobre las líneas ya definitivas).
        for (i, l) in lay.lines.iter().enumerate() {
            for s in l {
                if let Some(slot) = s.link.and_then(|k| lay.link_lines.get_mut(k)) {
                    slot.get_or_insert(i);
                }
            }
        }
        // Quita líneas en blanco finales.
        while lay.lines.last().is_some_and(|l| l.is_empty()) {
            lay.lines.pop();
        }
        lay
    }

    fn header(&self, width: usize, lay: &mut Layout) {
        let badge = kind_tag(&self.kind);
        let kind_color = kind_color(&self.kind);
        let top = vec![
            Seg::new(format!(" {badge} "), Sty::fg(MANTLE).bg(kind_color).bold()),
            Seg::new(" ", Sty::default()),
            Seg::new(self.title.clone(), Sty::fg(TEXT).bold()),
        ];
        lay.lines.push(top);
        let mut sub = vec![Seg::new(
            self.docset
                .strip_prefix("devdocs-")
                .unwrap_or(&self.docset)
                .to_string(),
            Sty::fg(GREEN),
        )];
        if let Some(m) = &self.meta {
            sub.push(Seg::new(format!("  ·  {m}"), Sty::fg(MUTED)));
        }
        lay.lines.push(sub);
        if let Some(sig) = &self.signature {
            lay.lines.push(Vec::new());
            lay.lines.extend(code_lines(sig, &self.lang, width));
        }
        lay.lines
            .push(vec![Seg::new("─".repeat(width), Sty::fg(SURFACE))]);
        lay.lines.push(Vec::new());
    }

    /// Texto plano (sin estilos) a un ancho dado, para copiar o `--raw`.
    pub fn plain_text(&self, width: usize) -> String {
        to_plain(&self.layout(width).lines)
    }
}

/// Tipos de símbolo que `sym_kind` puede devolver además de los de Dash.
const EXTRA_KINDS: &[&str] = &[
    "variant", "field", "attr", "derive", "kw", "prim", "item", "doc",
];

/// Etiqueta corta del tipo sin corchetes (`fn`, `struct`…). Idempotente.
pub fn kind_tag(kind: &str) -> String {
    let lower = kind.trim().to_lowercase();
    if EXTRA_KINDS.contains(&lower.as_str()) {
        return lower;
    }
    ui::compact_kind_badge(kind)
        .trim_matches(['[', ']'])
        .to_string()
}

/// Tipo de símbolo «real» de una entrada. Los docsets de DevDocs usan el
/// campo `kind` como *categoría* (`std::vec`, `Array`), no como tipo; en
/// ese caso se infiere de la ruta (convenciones de rustdoc y similares).
pub fn sym_kind(kind: &str, path: &str) -> String {
    let tag = kind_tag(kind);
    const DASH: &[&str] = &[
        "fn", "cls", "prop", "guide", "var", "intf", "type", "mod", "macro", "struct", "trait",
        "enum", "union", "ctor", "event", "const",
    ];
    if DASH.contains(&tag.as_str()) {
        return tag;
    }
    let (file, anchor) = path.split_once('#').unwrap_or((path, ""));
    let a = anchor.split('.').next().unwrap_or("");
    match a {
        "method" | "tymethod" => return "fn".into(),
        "associatedtype" => return "type".into(),
        "associatedconstant" => return "const".into(),
        "variant" => return "variant".into(),
        "structfield" => return "field".into(),
        _ => {}
    }
    let name = file.rsplit('/').next().unwrap_or(file);
    let stem = name.split('.').next().unwrap_or("");
    match stem {
        "struct" => "struct".into(),
        "enum" => "enum".into(),
        "trait" => "trait".into(),
        "fn" => "fn".into(),
        "macro" => "macro".into(),
        "type" | "traitalias" => "type".into(),
        "constant" | "static" => "const".into(),
        "union" => "union".into(),
        "attr" => "attr".into(),
        "derive" => "derive".into(),
        "keyword" => "kw".into(),
        "primitive" => "prim".into(),
        "index" if file.contains('/') => "mod".into(),
        _ => {
            if file.contains("/book/")
                || file.starts_with("book/")
                || file.starts_with("reference/")
            {
                "guide".into()
            } else if tag.chars().count() <= 5 && !tag.contains(':') {
                tag
            } else {
                "item".into()
            }
        }
    }
}

/// Color por tipo de símbolo.
pub fn kind_color(kind: &str) -> Rgb {
    match kind_tag(kind).as_str() {
        "fn" => BLUE,
        "cls" | "struct" => YELLOW,
        "type" | "intf" | "trait" | "enum" => TEAL,
        "prop" | "var" | "const" => PEACH,
        "guide" => GREEN,
        "mod" | "ns" => PINK,
        "macro" => RED,
        _ => MAUVE,
    }
}

fn layout_blocks(
    blocks: &[Block],
    width: usize,
    out: &mut Vec<Line>,
    lay: &mut Layout,
    depth: usize,
    base: usize,
) {
    let mut first = true;
    for b in blocks {
        if !first {
            out.push(Vec::new());
        }
        first = false;
        match b {
            Block::Heading(l, inl) => {
                let (sty, prefix) = match l {
                    1 => (Sty::fg(MAUVE).bold(), "█ "),
                    2 => (Sty::fg(BLUE).bold(), "▌ "),
                    3 => (Sty::fg(TEAL).bold(), "▎ "),
                    _ => (Sty::fg(GREEN).bold(), ""),
                };
                let start = base + out.len();
                let text: String = inl.iter().map(|i| i.text.as_str()).collect();
                lay.headings.push((start, *l, text.trim().to_string()));
                let heading_inl: Vec<Inl> = inl.clone();
                let mut lines = wrap_inline(
                    &heading_inl,
                    width.saturating_sub(prefix.width()),
                    Some(sty),
                );
                for (i, line) in lines.iter_mut().enumerate() {
                    let p = if i == 0 {
                        prefix
                    } else {
                        &" ".repeat(prefix.width())
                    };
                    line.insert(0, Seg::new(p.to_string(), sty));
                }
                out.extend(lines);
                if *l == 1 {
                    out.push(vec![Seg::new("━".repeat(width), Sty::fg(SURFACE))]);
                }
            }
            Block::Para(inl) => {
                let lines = wrap_inline(inl, width, None);
                out.extend(lines);
            }
            Block::Code { lang, text } => out.extend(code_lines(text, lang, width)),
            Block::List { ordered, items } => {
                for (i, item) in items.iter().enumerate() {
                    let marker = if *ordered {
                        format!("{:>2}. ", i + 1)
                    } else if depth.is_multiple_of(2) {
                        " • ".to_string()
                    } else {
                        " ◦ ".to_string()
                    };
                    let mw = marker.width();
                    let mut sub = Vec::new();
                    layout_blocks(
                        item,
                        width.saturating_sub(mw),
                        &mut sub,
                        lay,
                        depth + 1,
                        base + out.len(),
                    );
                    let msty = Sty::fg(PEACH);
                    for (j, mut l) in sub.into_iter().enumerate() {
                        let p = if j == 0 {
                            marker.clone()
                        } else {
                            " ".repeat(mw)
                        };
                        l.insert(0, Seg::new(p, msty));
                        out.push(l);
                    }
                }
            }
            Block::Quote(v) => {
                let mut sub = Vec::new();
                layout_blocks(
                    v,
                    width.saturating_sub(2),
                    &mut sub,
                    lay,
                    depth,
                    base + out.len(),
                );
                for mut l in sub {
                    l.insert(0, Seg::new("│ ", Sty::fg(MAUVE)));
                    for s in l.iter_mut().skip(1) {
                        s.sty.italic = true;
                    }
                    out.push(l);
                }
            }
            Block::Indent(v) => {
                let mut sub = Vec::new();
                layout_blocks(
                    v,
                    width.saturating_sub(4),
                    &mut sub,
                    lay,
                    depth,
                    base + out.len(),
                );
                for mut l in sub {
                    if !l.is_empty() {
                        l.insert(0, Seg::new("    ", Sty::default()));
                    }
                    out.push(l);
                }
            }
            Block::Table(rows) => out.extend(table_lines(rows, width)),
            Block::Rule => out.push(vec![Seg::new("─".repeat(width), Sty::fg(SURFACE))]),
        }
    }
}

fn inl_sty(f: InlFlags, base: Option<Sty>) -> Sty {
    let mut s = base.unwrap_or_else(|| Sty::fg(TEXT));
    if f.bold {
        s.bold = true;
        if base.is_none() {
            s.fg = Some([255, 255, 255]);
        }
    }
    if f.italic {
        s.italic = true;
    }
    if f.strike {
        s.dim = true;
    }
    if f.code {
        s.fg = Some(PEACH);
        s.bg = Some(MANTLE);
    }
    if f.link.is_some() {
        s.fg = Some(BLUE);
        s.underline = true;
    }
    s
}

/// Ajuste de línea voraz sobre texto en línea.
fn wrap_inline(inl: &[Inl], width: usize, base: Option<Sty>) -> Vec<Line> {
    // Divide en palabras (piezas adyacentes sin espacio) y espacios.
    type Piece = (String, Sty, Option<usize>);
    let mut words: Vec<Option<Vec<Piece>>> = Vec::new(); // None = espacio
    let mut cur: Vec<Piece> = Vec::new();
    for i in inl {
        let sty = inl_sty(i.flags, base);
        let mut buf = String::new();
        let flush_buf = |buf: &mut String, cur: &mut Vec<Piece>| {
            if !buf.is_empty() {
                cur.push((std::mem::take(buf), sty, i.flags.link));
            }
        };
        for c in i.text.chars() {
            if c == '\n' {
                flush_buf(&mut buf, &mut cur);
                if !cur.is_empty() {
                    words.push(Some(std::mem::take(&mut cur)));
                }
                words.push(Some(vec![("\n".into(), sty, None)]));
            } else if c == ' ' || c == '\t' {
                flush_buf(&mut buf, &mut cur);
                if !cur.is_empty() {
                    words.push(Some(std::mem::take(&mut cur)));
                }
                words.push(None);
            } else {
                buf.push(c);
            }
        }
        flush_buf(&mut buf, &mut cur);
    }
    if !cur.is_empty() {
        words.push(Some(cur));
    }

    let mut lines: Vec<Line> = Vec::new();
    let mut line: Line = Vec::new();
    let mut w = 0usize;
    let mut pending_space = false;
    for word in words {
        match word {
            None => pending_space = w > 0,
            Some(pieces) => {
                if pieces.len() == 1 && pieces[0].0 == "\n" {
                    lines.push(std::mem::take(&mut line));
                    w = 0;
                    pending_space = false;
                    continue;
                }
                let ww: usize = pieces.iter().map(|p| p.0.width()).sum();
                let need = ww + usize::from(pending_space && w > 0);
                if w > 0 && w + need > width {
                    lines.push(std::mem::take(&mut line));
                    w = 0;
                    pending_space = false;
                }
                if pending_space && w > 0 {
                    line.push(Seg::new(" ", base.unwrap_or_else(|| Sty::fg(TEXT))));
                    w += 1;
                }
                pending_space = false;
                for (text, sty, link) in pieces {
                    // Palabras más largas que la línea: corte duro.
                    let mut chunk = String::new();
                    let mut cw = 0;
                    for ch in text.chars() {
                        let chw = ch.width().unwrap_or(0);
                        if w + cw + chw > width && (w + cw) > 0 {
                            if !chunk.is_empty() {
                                line.push(Seg {
                                    text: std::mem::take(&mut chunk),
                                    sty,
                                    link,
                                });
                            }
                            lines.push(std::mem::take(&mut line));
                            w = 0;
                            cw = 0;
                        }
                        chunk.push(ch);
                        cw += chw;
                    }
                    if !chunk.is_empty() {
                        line.push(Seg {
                            text: chunk,
                            sty,
                            link,
                        });
                    }
                    w += cw;
                }
            }
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn table_lines(rows: &[Vec<Vec<Inl>>], width: usize) -> Vec<Line> {
    let ncols = rows.iter().map(Vec::len).max().unwrap_or(0);
    if ncols == 0 {
        return Vec::new();
    }
    let text_of = |c: &Vec<Inl>| -> String { c.iter().map(|i| i.text.as_str()).collect() };
    let mut colw = vec![3usize; ncols];
    for r in rows {
        for (i, c) in r.iter().enumerate() {
            colw[i] = colw[i].max(text_of(c).width());
        }
    }
    let avail = width.saturating_sub(ncols * 3 + 1);
    while colw.iter().sum::<usize>() > avail {
        let (i, _) = colw.iter().enumerate().max_by_key(|(_, w)| **w).unwrap();
        if colw[i] <= 4 {
            break;
        }
        colw[i] -= 1;
    }
    let border = Sty::fg(SURFACE);
    let rule = |l: &str, m: &str, r: &str| -> Line {
        let mut s = String::from(l);
        for (i, w) in colw.iter().enumerate() {
            s.push_str(&"─".repeat(w + 2));
            s.push_str(if i + 1 == ncols { r } else { m });
        }
        vec![Seg::new(s, border)]
    };
    let mut out = vec![rule("┌", "┬", "┐")];
    for (ri, r) in rows.iter().enumerate() {
        let mut line = vec![Seg::new("│", border)];
        for (i, w) in colw.iter().enumerate() {
            let t = r.get(i).map(text_of).unwrap_or_default();
            let t = ui::truncate_end(&t, *w);
            let pad = w.saturating_sub(t.width());
            let sty = if ri == 0 {
                Sty::fg(MAUVE).bold()
            } else {
                Sty::fg(TEXT)
            };
            line.push(Seg::new(format!(" {t}{} ", " ".repeat(pad)), sty));
            line.push(Seg::new("│", border));
        }
        out.push(line);
        if ri == 0 && rows.len() > 1 {
            out.push(rule("├", "┼", "┤"));
        }
    }
    out.push(rule("└", "┴", "┘"));
    out
}

/// Líneas de un bloque de código resaltado con fondo continuo.
fn code_lines(code: &str, lang: &str, width: usize) -> Vec<Line> {
    let inner = width.saturating_sub(2).max(10);
    let highlighted = highlight(code, lang);
    let mut out = Vec::new();
    for mut hl in highlighted {
        // Corte duro de líneas largas.
        loop {
            let (head, rest) = split_line_at(&mut hl, inner);
            let mut l = vec![Seg::new(" ", Sty::default().bg(MANTLE))];
            let w = line_width(&head);
            l.extend(head);
            l.push(Seg::new(
                " ".repeat(inner.saturating_sub(w) + 1),
                Sty::default().bg(MANTLE),
            ));
            out.push(l);
            match rest {
                Some(r) => hl = r,
                None => break,
            }
        }
    }
    if out.is_empty() {
        out.push(vec![Seg::new(" ".repeat(width), Sty::default().bg(MANTLE))]);
    }
    out
}

fn split_line_at(line: &mut Line, max: usize) -> (Line, Option<Line>) {
    if line_width(line) <= max {
        return (std::mem::take(line), None);
    }
    let mut head: Line = Vec::new();
    let mut rest: Line = Vec::new();
    let mut w = 0;
    let mut overflow = false;
    for seg in line.drain(..) {
        if overflow {
            rest.push(seg);
            continue;
        }
        let mut a = String::new();
        let mut b = String::new();
        for ch in seg.text.chars() {
            let cw = ch.width().unwrap_or(0);
            if !overflow && w + cw <= max {
                a.push(ch);
                w += cw;
            } else {
                overflow = true;
                b.push(ch);
            }
        }
        if !a.is_empty() {
            head.push(Seg {
                text: a,
                ..seg.clone()
            });
        }
        if !b.is_empty() {
            rest.push(Seg { text: b, ..seg });
        }
    }
    (head, Some(rest))
}

// ───────────────────────── Resaltado de sintaxis ─────────────────────────

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<ThemeSet> = OnceLock::new();

fn syntaxes() -> &'static SyntaxSet {
    SYNTAX_SET.get_or_init(SyntaxSet::load_defaults_newlines)
}
fn themes() -> &'static ThemeSet {
    THEME_SET.get_or_init(ThemeSet::load_defaults)
}

/// Deduce el lenguaje a partir del id de docset.
pub fn lang_from_docset(id: &str) -> String {
    let s = id.to_lowercase();
    let s = s.strip_prefix("devdocs-").unwrap_or(&s);
    for (needle, lang) in [
        ("typescript", "ts"),
        ("javascript", "js"),
        ("node", "js"),
        ("cpp", "cpp"),
        ("c++", "cpp"),
        ("rust", "rust"),
        ("python", "python"),
        ("bash", "bash"),
        ("homebrew", "bash"),
        ("java", "java"),
        ("kotlin", "java"),
        ("lua", "lua"),
        ("swift", "rust"),
        ("css", "css"),
        ("html", "html"),
        ("sql", "sql"),
        ("ruby", "ruby"),
        ("php", "php"),
    ] {
        if s.contains(needle) {
            return lang.to_string();
        }
    }
    if s == "go" || s.starts_with("go-") || s.starts_with("golang") {
        return "go".into();
    }
    if s == "c" {
        return "c".into();
    }
    String::new()
}

fn syntax_for(lang: &str) -> &'static SyntaxReference {
    let ps = syntaxes();
    let l = lang.trim().to_lowercase();
    let mapped = match l.as_str() {
        "rs" | "rust" | "no_run" | "ignore" | "should_panic" => "rs",
        "ts" | "typescript" | "tsx" | "jsx" | "javascript" | "js" | "node" | "mjs" => "js",
        "py" | "python" | "python3" => "py",
        "sh" | "shell" | "bash" | "zsh" | "console" | "shell-session" => "sh",
        "c++" | "cpp" | "cxx" | "hpp" => "cpp",
        "golang" | "go" => "go",
        "kotlin" | "kt" | "java" => "java",
        "yml" | "yaml" => "yaml",
        "json" | "jsonc" => "json",
        "toml" | "ini" => "ini",
        other => other,
    };
    ps.find_syntax_by_token(mapped)
        .or_else(|| ps.find_syntax_by_extension(mapped))
        .unwrap_or_else(|| ps.find_syntax_plain_text())
}

/// Resalta `code` y devuelve líneas con fragmentos coloreados (sin fondo).
pub fn highlight(code: &str, lang: &str) -> Vec<Line> {
    let ps = syntaxes();
    let ts = themes();
    let syntax = syntax_for(lang);
    let theme = ts
        .themes
        .get("base16-mocha.dark")
        .or_else(|| ts.themes.values().next())
        .expect("temas por defecto");
    let mut h = HighlightLines::new(syntax, theme);
    let mut out = Vec::new();
    for raw in LinesWithEndings::from(code) {
        let mut line: Line = Vec::new();
        match h.highlight_line(raw, ps) {
            Ok(ranges) => {
                for (style, text) in ranges {
                    let t = text.trim_end_matches(['\n', '\r']).replace('\t', "    ");
                    if t.is_empty() {
                        continue;
                    }
                    let c = style.foreground;
                    let mut sty = Sty::fg([c.r, c.g, c.b]);
                    sty.bg = Some(MANTLE);
                    line.push(Seg::new(t, sty));
                }
            }
            Err(_) => line.push(Seg::new(
                raw.trim_end_matches(['\n', '\r']).replace('\t', "    "),
                Sty::fg(TEXT).bg(MANTLE),
            )),
        }
        // Asegura el fondo en todos los fragmentos.
        for s in &mut line {
            s.sty.bg = Some(MANTLE);
        }
        out.push(line);
    }
    out
}

// ───────────────────────── Salidas ─────────────────────────

/// Texto plano de unas líneas.
pub fn to_plain(lines: &[Line]) -> String {
    let mut s = String::new();
    for l in lines {
        let t: String = l.iter().map(|s| s.text.as_str()).collect();
        s.push_str(t.trim_end());
        s.push('\n');
    }
    s.trim_end().to_string() + "\n"
}

/// Secuencias ANSI de 24 bits (`anstream` las degrada o elimina).
pub fn to_ansi(lines: &[Line]) -> String {
    let mut out = String::new();
    for l in lines {
        for s in l {
            let mut codes: Vec<String> = Vec::new();
            if s.sty.bold {
                codes.push("1".into());
            }
            if s.sty.dim {
                codes.push("2".into());
            }
            if s.sty.italic {
                codes.push("3".into());
            }
            if s.sty.underline {
                codes.push("4".into());
            }
            if let Some([r, g, b]) = s.sty.fg {
                codes.push(format!("38;2;{r};{g};{b}"));
            }
            if let Some([r, g, b]) = s.sty.bg {
                codes.push(format!("48;2;{r};{g};{b}"));
            }
            if codes.is_empty() {
                out.push_str(&s.text);
            } else {
                out.push_str(&format!("\x1b[{}m{}\x1b[0m", codes.join(";"), s.text));
            }
        }
        out.push('\n');
    }
    out
}

/// Resalta código suelto y lo devuelve como ANSI (para `--code`).
pub fn highlight_ansi(code: &str, lang: &str) -> String {
    let lines = highlight(code, lang)
        .into_iter()
        .map(|mut l| {
            for s in &mut l {
                s.sty.bg = None;
            }
            l
        })
        .collect::<Vec<_>>();
    to_ansi(&lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RUSTDOC: &str = r#"<html><body><div class="_page"><h1>Struct Vec</h1>
<p>Intro paragraph.</p>
<summary><section id="method.push" class="method"><span class="rightside"><span class="since" title="Stable since 1.0.0">1.0.0</span><a class="src" href="x">Source</a></span><pre class="code-header" data-language="rust">pub fn push(&amp;mut self, value: T)</pre></section></summary>
<div class="docblock"><p>Appends an element to the back.</p><h5 id="e">Examples</h5>
<div class="example-wrap"><pre data-language="rust">let mut v = vec![1];
v.push(2);</pre></div></div>
<summary><section id="method.pop"><pre class="code-header">pub fn pop(&amp;mut self)</pre></section></summary>
<div class="docblock"><p>Removes the last.</p></div></div></body></html>"#;

    #[test]
    fn anchor_includes_sibling_docblock_and_stops_at_next_method() {
        let d = parse_doc(
            RUSTDOC,
            Some("method.push"),
            "Vec::push",
            "Method",
            "devdocs-rust",
        );
        let txt = d.plain_text(80);
        assert!(txt.contains("Appends an element"), "{txt}");
        assert!(txt.contains("v.push(2);"));
        assert!(!txt.contains("Removes the last"));
        assert_eq!(
            d.signature.as_deref(),
            Some("pub fn push(&mut self, value: T)")
        );
        assert_eq!(d.meta.as_deref(), Some("Stable since 1.0.0"));
        assert_eq!(d.code_blocks.len(), 1);
        assert!(!txt.contains("Source"));
    }

    #[test]
    fn page_without_anchor_renders_headings_and_lists() {
        let html = "<main><h1>T</h1><p>hola <b>mundo</b> <a href='a.html'>enlace</a></p><ul><li>uno</li><li>dos</li></ul></main>";
        let d = parse_doc(html, None, "T", "Guide", "x");
        let l = d.layout(60);
        let txt = to_plain(&l.lines);
        assert!(txt.contains("hola mundo enlace"));
        assert!(txt.contains("• uno"));
        assert_eq!(d.links, vec!["a.html".to_string()]);
        assert!(l.link_lines[0].is_some());
        assert!(!l.headings.is_empty());
    }

    #[test]
    fn link_and_heading_lines_are_absolute_inside_lists() {
        let html =
            "<main><p>a</p><p>b</p><ul><li>x</li><li><a href='z.html'>enlace</a></li></ul></main>";
        let d = parse_doc(html, None, "T", "Guide", "x");
        let l = d.layout(60);
        let idx = l.link_lines[0].expect("enlace");
        let text: String = l.lines[idx].iter().map(|s| s.text.as_str()).collect();
        assert!(text.contains("enlace"), "{text}");
    }

    #[test]
    fn wrapping_respects_width() {
        let html = format!("<p>{}</p>", "palabra ".repeat(40));
        let d = parse_doc(&html, None, "T", "Guide", "x");
        for line in d.layout(30).lines {
            assert!(line_width(&line) <= 30);
        }
    }

    #[test]
    fn table_renders() {
        let html = "<table><tr><th>a</th><th>b</th></tr><tr><td>1</td><td>2</td></tr></table>";
        let d = parse_doc(html, None, "T", "Guide", "x");
        let txt = d.plain_text(40);
        assert!(txt.contains('┌') && txt.contains("│ 1"));
    }

    #[test]
    fn highlight_keeps_text() {
        let lines = highlight("fn main() {}\n", "rust");
        let t: String = lines[0].iter().map(|s| s.text.as_str()).collect();
        assert_eq!(t, "fn main() {}");
    }

    #[test]
    fn lang_detection() {
        assert_eq!(lang_from_docset("devdocs-cpp"), "cpp");
        assert_eq!(lang_from_docset("go"), "go");
        assert_eq!(lang_from_docset("devdocs-javascript"), "js");
    }

    #[test]
    fn symbol_kind_from_rustdoc_paths() {
        assert_eq!(
            sym_kind("std::vec", "std/vec/struct.vec.html#method.push"),
            "fn"
        );
        assert_eq!(sym_kind("std::vec", "std/vec/struct.vec.html"), "struct");
        assert_eq!(sym_kind("Function", "x.html"), "fn");
        assert_eq!(sym_kind("std", "std/macro.vec.html"), "macro");
        assert_eq!(sym_kind("Array", "a/b.html"), "array");
        assert_eq!(kind_tag("variant"), "variant");
    }

    #[test]
    fn percent_decoding() {
        assert_eq!(percent_decode("a%20b"), "a b");
        assert_eq!(percent_decode("x%"), "x%");
    }
}
