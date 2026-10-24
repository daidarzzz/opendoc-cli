//! Paleta y utilidades de dibujo para la terminal.
//!
//! Paleta tipo Catppuccin (Mocha) expresada en 256 colores para que
//! funcione también en terminales sin color verdadero. `anstream` ya
//! elimina los códigos cuando la salida no es una terminal.

use anstyle::{Ansi256Color, Color, Style};

/// Malva (`#cba6f7`): encabezados y acentos.
pub const ACCENT: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(183))));
/// Azul (`#89b4fa`): nombres de entradas.
pub const NAME: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(111))));
/// Verde (`#a6e3a1`): docsets.
pub const DOCSET: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(151))));
/// Turquesa (`#94e2d5`): tipos.
pub const KIND: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(116))));
/// Texto tenue (`#9399b2`): metadatos y rutas.
pub const MUTED: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(103))));
/// Melocotón (`#fab387`): literales y valores.
pub const PEACH: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(216))));
/// Rojo (`#f38ba8`): errores.
pub const ERROR: Style = Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(211))));
/// Negrita.
pub const BOLD: Style = Style::new().bold();

/// Aplica un estilo a un texto (o lo deja plano si `anstream` lo pide).
pub fn paint(style: Style, text: &str) -> String {
    format!("{style}{text}{style:#}")
}

/// Encabezado de sección (negrita + malva).
pub fn heading(text: &str) -> String {
    format!("{ACCENT}{BOLD}{text}{BOLD:#}{ACCENT:#}")
}

/// Recorta por el centro con puntos suspensivos, respetando caracteres
/// UTF-8. Un ancho de 0 o 1 deja solo la elipsis.
pub fn truncate_middle(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width <= 1 {
        return "…".to_string();
    }
    let keep = width - 1;
    // Conserva algo más del inicio (los nombres de API suelen empezar por
    // el identificador más informativo).
    let left = keep.div_ceil(2);
    let right = keep - left;
    let head: String = text.chars().take(left).collect();
    let tail: String = text
        .chars()
        .rev()
        .take(right)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("{head}…{tail}")
}

/// Ancho visible en caracteres de una pantalla dada.
pub fn visible_width(text: &str) -> usize {
    text.chars().count()
}

/// Devuelve una etiqueta corta y estilizada para el tipo de entrada de Dash/Zeal
/// (`[fn]`, `[cls]`, `[prop]`, `[guide]`, `[var]`, etc.).
pub fn compact_kind_badge(kind: &str) -> String {
    let lower = kind.trim().to_lowercase();
    let tag = match lower.as_str() {
        "function" | "func" | "method" | "clm" | "instm" | "procedure" | "callback"
        | "operator" => "fn",
        "class" | "cl" => "cls",
        "property" | "instp" | "clp" | "prop" | "field" | "attribute" | "attr" => "prop",
        "guide" | "section" | "category" | "cat" | "sec" | "topic" => "guide",
        "variable" | "var" | "global" | "constant" | "const" | "value" => "var",
        "interface" | "intf" => "intf",
        "type" | "tdef" | "typedef" => "type",
        "module" | "package" | "mod" | "pkg" | "namespace" | "framework" => "mod",
        "macro" => "macro",
        "struct" => "struct",
        "trait" => "trait",
        "enum" => "enum",
        "union" => "union",
        "constructor" | "ctor" => "ctor",
        "event" | "hook" => "event",
        other => {
            if other.chars().count() <= 5 {
                other
            } else {
                let end = other
                    .char_indices()
                    .nth(5)
                    .map(|(i, _)| i)
                    .unwrap_or(other.len());
                &other[..end]
            }
        }
    };
    format!("[{tag}]")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compact_kind_badge_maps_known_types() {
        assert_eq!(compact_kind_badge("Function"), "[fn]");
        assert_eq!(compact_kind_badge("Method"), "[fn]");
        assert_eq!(compact_kind_badge("Class"), "[cls]");
        assert_eq!(compact_kind_badge("Property"), "[prop]");
        assert_eq!(compact_kind_badge("Guide"), "[guide]");
        assert_eq!(compact_kind_badge("Variable"), "[var]");
        assert_eq!(compact_kind_badge("Constant"), "[var]");
        assert_eq!(compact_kind_badge("Struct"), "[struct]");
        assert_eq!(compact_kind_badge("Trait"), "[trait]");
        assert_eq!(compact_kind_badge("Interface"), "[intf]");
        assert_eq!(compact_kind_badge("Type"), "[type]");
        assert_eq!(compact_kind_badge("UnknownLongType"), "[unkno]");
        assert_eq!(compact_kind_badge("Tag"), "[tag]");
    }

    #[test]
    fn truncate_keeps_short_text() {
        assert_eq!(truncate_middle("grid", 10), "grid");
        assert_eq!(truncate_middle("", 3), "");
    }

    #[test]
    fn truncate_middle_marks_the_cut() {
        assert_eq!(truncate_middle("abcdefghij", 7), "abc…hij");
        assert_eq!(truncate_middle("abcdef", 5), "ab…ef");
        assert_eq!(truncate_middle("abcdef", 2), "a…");
        assert_eq!(truncate_middle("abcdef", 1), "…");
        assert_eq!(truncate_middle("abcdef", 0), "…");
    }

    #[test]
    fn truncate_respects_unicode_boundaries() {
        let out = truncate_middle("áéíóúñ", 5);
        assert_eq!(out.chars().count(), 5);
        assert!(out.contains('…'));
        assert!(out.is_char_boundary(out.len()));
    }

    #[test]
    fn paint_wraps_with_ansi() {
        let out = paint(NAME, "hola");
        assert!(out.starts_with('\u{1b}'));
        assert!(out.contains("hola"));
        assert!(out.len() > "hola".len());
    }
}
