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

#[cfg(test)]
mod tests {
    use super::*;

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
