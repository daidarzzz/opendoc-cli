//! Sintaxis de consulta estilo Dash: `rust:Vec::push`, `fn:filter`,
//! `py:fn:dumps`. Los prefijos pueden ser un docset o un tipo de símbolo.

use std::collections::HashSet;

use opendoc_core::library::Library;

use crate::render::{kind_tag, sym_kind};

/// Consulta ya interpretada.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedQuery {
    /// Docset al que se restringe (id resuelto).
    pub docset: Option<String>,
    /// Etiqueta de tipo (`fn`, `struct`…) a la que se restringe.
    pub kind: Option<String>,
    /// Texto que se busca realmente.
    pub text: String,
}

/// Etiquetas de tipo presentes en la biblioteca (para validar prefijos).
pub fn known_kinds(library: &Library) -> HashSet<String> {
    library
        .entries
        .iter()
        .map(|e| sym_kind(&e.kind, &e.path))
        .collect()
}

/// Resuelve un token a docset (id, nombre o plataforma, sin distinguir caja).
pub fn resolve_docset(library: &Library, token: &str) -> Option<String> {
    let lower = token.to_lowercase();
    library
        .docsets
        .iter()
        .find(|d| {
            d.id.to_lowercase() == lower
                || d.name.to_lowercase() == lower
                || d.platform.as_deref().map(str::to_lowercase).as_deref() == Some(lower.as_str())
                || d.id.to_lowercase().strip_prefix("devdocs-") == Some(lower.as_str())
        })
        .map(|d| d.id.clone())
        .or_else(|| {
            library
                .pending
                .iter()
                .find(|p| p.id.to_lowercase() == lower || p.name.to_lowercase() == lower)
                .map(|p| p.id.clone())
        })
}

/// Alias cortos habituales de docsets (`py`, `js`, `ts`…).
fn alias(token: &str) -> &str {
    match token {
        "py" => "python",
        "js" => "javascript",
        "ts" => "typescript",
        "rs" => "rust",
        "c++" => "cpp",
        "sh" => "bash",
        other => other,
    }
}

/// Interpreta `raw` contra la biblioteca. `explicit_docset` (flag
/// `--docset`) tiene prioridad y deja el texto intacto.
pub fn parse(
    library: &Library,
    kinds: &HashSet<String>,
    raw: &str,
    explicit_docset: Option<&str>,
) -> ParsedQuery {
    let mut out = ParsedQuery {
        text: raw.trim().to_string(),
        ..ParsedQuery::default()
    };
    if let Some(d) = explicit_docset {
        out.docset = Some(d.to_string());
    }
    for _ in 0..2 {
        let Some((head, rest)) = out.text.split_once(':') else {
            break;
        };
        // `std::vector` no es un prefijo.
        if rest.starts_with(':') || head.is_empty() || head.contains(char::is_whitespace) {
            break;
        }
        let head_l = head.trim().to_lowercase();
        if out.docset.is_none() && explicit_docset.is_none() {
            if let Some(id) =
                resolve_docset(library, &head_l).or_else(|| resolve_docset(library, alias(&head_l)))
            {
                out.docset = Some(id);
                out.text = rest.trim_start().to_string();
                continue;
            }
        }
        if out.kind.is_none() {
            let tag = kind_tag(&head_l);
            if kinds.contains(&tag) {
                out.kind = Some(tag);
                out.text = rest.trim_start().to_string();
                continue;
            }
        }
        break;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use opendoc_core::docset::{Docset, Entry};
    use std::path::PathBuf;

    fn lib() -> Library {
        let d = |id: &str, name: &str| Docset {
            id: id.into(),
            name: name.into(),
            platform: None,
            version: None,
            bundle_id: None,
            home_path: None,
            icon: None,
            root_path: PathBuf::new(),
            contents_path: PathBuf::new(),
        };
        Library {
            source_dir: PathBuf::new(),
            docsets: vec![d("devdocs-rust", "Rust"), d("devdocs-python", "Python")],
            pending: vec![],
            issues: vec![],
            entries: vec![Entry {
                docset_id: "devdocs-rust".into(),
                name: "push".into(),
                kind: "Function".into(),
                path: "a".into(),
            }],
        }
    }

    #[test]
    fn prefixes() {
        let l = lib();
        let k = known_kinds(&l);
        let p = parse(&l, &k, "rust:fn:push", None);
        assert_eq!(p.docset.as_deref(), Some("devdocs-rust"));
        assert_eq!(p.kind.as_deref(), Some("fn"));
        assert_eq!(p.text, "push");
        let p = parse(&l, &k, "py:json", None);
        assert_eq!(p.docset.as_deref(), Some("devdocs-python"));
        let p = parse(&l, &k, "std::vector", None);
        assert_eq!(
            p,
            ParsedQuery {
                text: "std::vector".into(),
                ..Default::default()
            }
        );
        let p = parse(&l, &k, "rust:x", Some("python"));
        assert_eq!(p.text, "rust:x");
    }
}
