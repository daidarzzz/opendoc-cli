//! Detección del contexto de proyecto: sube desde el directorio actual
//! buscando marcadores (`Cargo.toml`, `package.json`…) para priorizar el
//! docset del lenguaje en uso.

use std::path::Path;

use opendoc_core::library::Library;

/// Marcador de fichero → palabras clave de docset.
const MARKERS: &[(&str, &[&str])] = &[
    ("Cargo.toml", &["rust"]),
    ("tsconfig.json", &["typescript", "javascript"]),
    ("package.json", &["javascript", "typescript", "node"]),
    ("deno.json", &["typescript", "javascript"]),
    ("go.mod", &["go"]),
    ("pyproject.toml", &["python"]),
    ("requirements.txt", &["python"]),
    ("setup.py", &["python"]),
    ("CMakeLists.txt", &["cpp", "c"]),
    ("Makefile.am", &["cpp", "c"]),
    ("pom.xml", &["java"]),
    ("build.gradle", &["java", "kotlin"]),
    ("build.gradle.kts", &["java", "kotlin"]),
    ("Package.swift", &["swift"]),
    ("Gemfile", &["ruby"]),
    ("composer.json", &["php"]),
    ("init.lua", &["lua"]),
    (".luarc.json", &["lua"]),
];

/// Palabras clave del primer proyecto detectado subiendo desde `start`.
pub fn detect_hints(start: &Path) -> Vec<&'static str> {
    let mut dir = Some(start);
    while let Some(d) = dir {
        let mut hints: Vec<&'static str> = Vec::new();
        for (file, kws) in MARKERS {
            if d.join(file).is_file() {
                for k in *kws {
                    if !hints.contains(k) {
                        hints.push(k);
                    }
                }
            }
        }
        if !hints.is_empty() {
            return hints;
        }
        dir = d.parent();
    }
    Vec::new()
}

/// ¿El docset `id`/`name`/`platform` corresponde a alguna pista?
pub fn docset_matches(library: &Library, docset_id: &str, hints: &[&str]) -> bool {
    let Some(d) = library.docset(docset_id) else {
        return false;
    };
    let hay = format!(
        "{} {} {}",
        d.id.to_lowercase(),
        d.name.to_lowercase(),
        d.platform.as_deref().unwrap_or("").to_lowercase()
    );
    hints.iter().any(|h| {
        let h = h.to_lowercase();
        // `c` y `go` solo como palabra completa para no casar con «cpp» o «django».
        if h.len() <= 2 {
            hay.split(|c: char| !c.is_alphanumeric() && c != '+')
                .any(|w| w == h)
        } else {
            hay.contains(&h)
        }
    })
}

/// Ids de docsets instalados que encajan con el proyecto actual.
pub fn context_docsets(library: &Library, cwd: &Path) -> Vec<String> {
    let hints = detect_hints(cwd);
    if hints.is_empty() {
        return Vec::new();
    }
    library
        .docsets
        .iter()
        .filter(|d| docset_matches(library, &d.id, &hints))
        .map(|d| d.id.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_marker_walking_up() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("Cargo.toml"), "").unwrap();
        let sub = tmp.path().join("src/deep");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(detect_hints(&sub), vec!["rust"]);
    }
}
