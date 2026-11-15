//! Estado persistente de la TUI: favoritos y símbolos leídos recientemente.
//!
//! Se guarda como JSON en el directorio de datos de OpenDoc
//! (`cli-state.json`). Los fallos de lectura o escritura nunca rompen la
//! TUI: se parte de un estado vacío y se ignoran los errores al guardar.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const RECENT_MAX: usize = 100;

/// Un símbolo guardado.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Item {
    pub docset: String,
    pub path: String,
    pub name: String,
    pub kind: String,
}

impl Item {
    /// ¿Apunta al mismo documento (docset + ruta con ancla)?
    pub fn same(&self, other: &Item) -> bool {
        self.docset == other.docset && self.path == other.path
    }
}

/// Favoritos y recientes.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub favorites: Vec<Item>,
    #[serde(default)]
    pub recent: Vec<Item>,
    #[serde(skip)]
    file: Option<PathBuf>,
}

/// Ruta por defecto del fichero de estado.
pub fn default_path() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("ODC_STATE") {
        return Some(PathBuf::from(p));
    }
    crate::paths::app_data_dir()
        .ok()
        .map(|d| d.join("cli-state.json"))
}

impl State {
    /// Carga el estado de `path` (vacío si no existe o está corrupto).
    pub fn load_from(path: Option<PathBuf>) -> Self {
        let mut state = path
            .as_deref()
            .and_then(|p| std::fs::read(p).ok())
            .and_then(|b| serde_json::from_slice::<State>(&b).ok())
            .unwrap_or_default();
        state.file = path;
        state
    }

    pub fn load() -> Self {
        Self::load_from(default_path())
    }

    /// Guarda de forma atómica (escribe un temporal y renombra).
    pub fn save(&self) {
        let Some(path) = &self.file else { return };
        let _ = save_atomic(path, self);
    }

    pub fn is_favorite(&self, item: &Item) -> bool {
        self.favorites.iter().any(|f| f.same(item))
    }

    /// Alterna el favorito y devuelve si queda marcado.
    pub fn toggle_favorite(&mut self, item: Item) -> bool {
        let marked = if let Some(pos) = self.favorites.iter().position(|f| f.same(&item)) {
            self.favorites.remove(pos);
            false
        } else {
            self.favorites.insert(0, item);
            true
        };
        self.save();
        marked
    }

    /// Registra una lectura (al principio, sin duplicados).
    pub fn push_recent(&mut self, item: Item) {
        self.recent.retain(|r| !r.same(&item));
        self.recent.insert(0, item);
        self.recent.truncate(RECENT_MAX);
        self.save();
    }

    /// Quita un símbolo de favoritos y recientes.
    pub fn forget(&mut self, item: &Item) {
        self.favorites.retain(|f| !f.same(item));
        self.recent.retain(|r| !r.same(item));
        self.save();
    }
}

fn save_atomic(path: &Path, state: &State) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_vec_pretty(state).map_err(std::io::Error::other)?;
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(n: &str) -> Item {
        Item {
            docset: "d".into(),
            path: format!("{n}.html"),
            name: n.into(),
            kind: "fn".into(),
        }
    }

    #[test]
    fn favorites_recents_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("sub/state.json");
        let mut s = State::load_from(Some(file.clone()));
        assert!(s.toggle_favorite(item("a")));
        s.push_recent(item("a"));
        s.push_recent(item("b"));
        s.push_recent(item("a"));
        assert_eq!(s.recent.len(), 2);
        assert_eq!(s.recent[0].name, "a");
        let s2 = State::load_from(Some(file));
        assert!(s2.is_favorite(&item("a")));
        assert_eq!(s2.recent.len(), 2);
        let mut s3 = s2;
        assert!(!s3.toggle_favorite(item("a")));
        s3.forget(&item("b"));
        assert_eq!(s3.recent.len(), 1);
    }

    #[test]
    fn corrupt_file_gives_empty_state() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("s.json");
        std::fs::write(&file, "{ no json").unwrap();
        assert!(State::load_from(Some(file)).favorites.is_empty());
    }
}
