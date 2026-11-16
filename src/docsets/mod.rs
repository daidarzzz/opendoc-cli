//! Catalog and docset operations shared by the command line and the TUI.

mod devdocs;
mod install;

use std::collections::HashSet;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use opendoc_core::docset::{scan_dir, Docset, TarixLimits};
use serde::{Deserialize, Serialize};

use crate::catalog::feed::{self, RepoRef};
use crate::catalog::{self, Catalog, FeedEntry};
use crate::error::CliError;
use crate::paths::{self, Locations};

use install::{folder_stem, match_feed, update_available, FeedMatch};
pub(crate) use install::{ProgressReport, ProgressStage};

#[derive(Clone)]
pub(crate) struct Manager {
    pub(crate) locations: Locations,
    data_dir: PathBuf,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Action {
    Install,
    Update,
    Reinstall,
    Uninstall,
}

impl Action {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Install => "Install",
            Self::Update => "Update",
            Self::Reinstall => "Reinstall",
            Self::Uninstall => "Uninstall",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Available,
    Installed,
    UpdateAvailable,
    VersionUnknown,
    Ambiguous,
    Local,
}

impl Status {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Available => "Available",
            Self::Installed => "Installed",
            Self::UpdateAvailable => "Update available",
            Self::VersionUnknown => "Installed (version unknown)",
            Self::Ambiguous => "Ambiguous installation",
            Self::Local => "Installed (local)",
        }
    }

    pub(crate) fn primary_action(self) -> Option<Action> {
        match self {
            Self::Available => Some(Action::Install),
            Self::UpdateAvailable => Some(Action::Update),
            Self::Installed | Self::VersionUnknown => Some(Action::Reinstall),
            _ => None,
        }
    }

    pub(crate) fn can_uninstall(self) -> bool {
        matches!(
            self,
            Self::Installed | Self::UpdateAvailable | Self::VersionUnknown | Self::Local
        )
    }
}

#[derive(Clone)]
pub(crate) struct Row {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) installed: Option<String>,
    pub(crate) available: Option<String>,
    pub(crate) status: Status,
}

impl Row {
    pub(crate) fn matches(&self, query: &str) -> bool {
        let query = query.to_lowercase();
        self.id.to_lowercase().contains(&query) || self.name.to_lowercase().contains(&query)
    }

    pub(crate) fn description(&self) -> String {
        let version = match (&self.installed, &self.available, self.status) {
            (Some(old), Some(new), Status::UpdateAvailable) => format!(" · {old} → {new}"),
            (Some(version), _, _) | (None, Some(version), Status::Available) => {
                format!(" · {version}")
            }
            _ => String::new(),
        };
        let description = format!(
            "{} [{}] · {}{}",
            self.name,
            self.id,
            self.status.label(),
            version
        );
        display_text(&description)
    }
}

#[derive(Default, Deserialize)]
struct DesktopSettings {
    feed_url: Option<String>,
    fast_mirror: Option<String>,
}

impl Manager {
    pub(crate) fn new(locations: &Locations) -> Result<Self, CliError> {
        Ok(Self {
            locations: locations.clone(),
            data_dir: paths::app_data_dir()?,
        })
    }

    fn desktop_settings(&self) -> DesktopSettings {
        fs::read(self.data_dir.join("settings.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn feeds_file(&self) -> PathBuf {
        self.data_dir.join("odc-feeds.json")
    }

    pub(crate) fn feeds(&self) -> Result<Vec<RepoRef>, CliError> {
        match fs::read(self.feeds_file()) {
            Ok(bytes) => parse_repos(&serde_json::from_slice::<String>(&bytes).map_err(error)?),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                parse_repos(self.desktop_settings().feed_url.as_deref().unwrap_or(""))
            }
            Err(e) => Err(error(e)),
        }
    }

    pub(crate) fn set_feeds(&self, input: &str) -> Result<(), CliError> {
        let normalized = repo_key(&parse_repos(input)?);
        save_json(&self.feeds_file(), &normalized).map_err(error)
    }

    pub(crate) fn reset_feeds(&self) -> Result<(), CliError> {
        match fs::remove_file(self.feeds_file()) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(error(e)),
        }
    }

    fn cached_for(&self, key: &str) -> Option<Catalog> {
        [
            self.data_dir.join("odc-catalog.json"),
            catalog::catalog_file(&self.data_dir),
        ]
        .iter()
        .filter_map(|path| catalog::load_catalog(path))
        .filter(|cached| cached.repo == key)
        .max_by_key(|cached| cached.fetched_at)
    }

    fn cached(&self) -> Result<Option<Catalog>, CliError> {
        Ok(self.cached_for(&repo_key(&self.feeds()?)))
    }

    pub(crate) fn has_catalog(&self) -> Result<bool, CliError> {
        Ok(self.cached()?.is_some())
    }

    pub(crate) fn refresh(&self) -> Result<String, CliError> {
        fs::create_dir_all(&self.data_dir).map_err(error)?;
        let _lock = lock(&self.data_dir.join(".odc-catalog.lock"))?;
        let repos = self.feeds()?;
        let key = repo_key(&repos);
        let client = feed::http_client().map_err(error)?;
        let previous = self.cached_for(&key);
        let mut warnings = Vec::new();
        let mut successes = 0;
        let mut skipped = 0;

        let mut entries = match catalog::devdocs::fetch_devdocs_catalog(&client) {
            Ok((entries, missed)) => {
                successes += 1;
                skipped += missed;
                entries
            }
            Err(e) => {
                warnings.push(format!("DevDocs: {e}"));
                previous
                    .iter()
                    .flat_map(|c| &c.entries)
                    .filter(|e| e.devdocs.is_some())
                    .cloned()
                    .collect()
            }
        };
        let mut feeds_failed = false;
        for repo in &repos {
            match feed::fetch_catalog(&client, repo) {
                Ok((found, missed)) => {
                    successes += 1;
                    skipped += missed;
                    entries.extend(found);
                }
                Err(e) => {
                    feeds_failed = true;
                    warnings.push(format!("{}: {e}", repo.path()));
                }
            }
        }
        if feeds_failed {
            entries.extend(
                previous
                    .iter()
                    .flat_map(|c| &c.entries)
                    .filter(|e| e.devdocs.is_none())
                    .cloned(),
            );
        }
        if successes == 0 {
            return Err(error(warnings.join(" | ")));
        }

        let mut seen = HashSet::new();
        entries.retain(|entry| valid_id(&entry.id) && seen.insert(entry.id.to_lowercase()));
        entries.sort_by_cached_key(|entry| (entry.name.to_lowercase(), entry.name.clone()));
        let summary = format!(
            "Catalog refreshed: {} docsets ({skipped} skipped)",
            entries.len()
        );
        let catalog = Catalog {
            entries,
            repo: key,
            fetched_at: catalog::now_epoch(),
        };
        catalog::save_catalog(&self.data_dir.join("odc-catalog.json"), &catalog).map_err(error)?;
        if warnings.is_empty() {
            Ok(summary)
        } else {
            Ok(format!("{summary}. {}", warnings.join(" | ")))
        }
    }

    fn inventory(&self) -> Result<Vec<Docset>, CliError> {
        let dir = &self.locations.docsets;
        match dir.metadata() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(error(e)),
            Ok(_) => {}
        }
        let scan = scan_dir(dir).map_err(error)?;
        let mut docsets = scan.docsets;
        docsets.extend(scan.pending_tarix.into_iter().map(|pending| Docset {
            id: pending.id,
            name: pending.name,
            contents_path: pending.root_path.join("Contents"),
            root_path: pending.root_path,
            icon: None,
            platform: None,
            version: None,
            bundle_id: None,
            home_path: None,
        }));
        Ok(docsets)
    }

    /// Catalog rows include installed docsets with no current feed.
    pub(crate) fn rows(&self) -> Result<(Vec<Row>, bool), CliError> {
        let cached = self.cached()?;
        let docsets = self.inventory()?;
        let mut matched = HashSet::new();
        let mut rows = Vec::new();
        for entry in cached
            .iter()
            .flat_map(|c| &c.entries)
            .filter(|entry| valid_id(&entry.id))
        {
            let (status, installed) = match match_feed(&entry.id, &docsets) {
                FeedMatch::None => (Status::Available, None),
                FeedMatch::One(docset) => {
                    matched.insert(&docset.id);
                    (
                        match has_update(entry, docset) {
                            Some(true) => Status::UpdateAvailable,
                            Some(false) => Status::Installed,
                            None => Status::VersionUnknown,
                        },
                        docset.version.clone(),
                    )
                }
                FeedMatch::Ambiguous(candidates) => {
                    matched.extend(candidates.into_iter().map(|docset| &docset.id));
                    (Status::Ambiguous, None)
                }
            };
            rows.push(Row {
                id: entry.id.clone(),
                name: entry.name.clone(),
                installed,
                available: Some(entry.version.clone()),
                status,
            });
        }
        let catalog_ids: HashSet<_> = rows.iter().map(|row| row.id.to_ascii_lowercase()).collect();
        rows.extend(
            docsets
                .iter()
                .filter(|d| !matched.contains(&d.id))
                .map(|d| Row {
                    id: if catalog_ids.contains(&d.id.to_ascii_lowercase()) {
                        format!("local:{}", d.id)
                    } else {
                        d.id.clone()
                    },
                    name: d.name.clone(),
                    installed: d.version.clone(),
                    available: None,
                    status: Status::Local,
                }),
        );
        rows.sort_by_cached_key(|row| row.name.to_lowercase());
        Ok((rows, cached.is_some()))
    }

    pub(crate) fn change(
        &self,
        action: Action,
        id: &str,
        progress: &(dyn Fn(ProgressReport) + Send + Sync),
    ) -> Result<String, CliError> {
        let dir = &self.locations.docsets;
        let cached = self.cached()?;
        let local_id = id.strip_prefix("local:");
        let entry = cached
            .as_ref()
            .filter(|_| local_id.is_none())
            .and_then(|catalog| {
                catalog
                    .entries
                    .iter()
                    .find(|entry| entry.id.eq_ignore_ascii_case(id))
            });
        if action == Action::Install && entry.is_some() {
            fs::create_dir_all(dir).map_err(error)?;
        }
        if !dir.is_dir() {
            return Err(error(format!(
                "docsets folder not found: {}",
                dir.display()
            )));
        }
        let _lock = lock(&dir.join(".odc-docsets.lock"))?;
        let docsets = self.inventory()?;
        let installed = if let Some(entry) = entry {
            match match_feed(&entry.id, &docsets) {
                FeedMatch::None => None,
                FeedMatch::One(docset) => Some(docset),
                FeedMatch::Ambiguous(_) => {
                    return Err(error(format!(
                        "more than one installation matches {}",
                        entry.id
                    )))
                }
            }
        } else {
            docsets.iter().find(|d| d.id == local_id.unwrap_or(id))
        };

        if action == Action::Uninstall {
            let docset =
                installed.ok_or_else(|| error(format!("docset is not installed: {id}")))?;
            let path = safe_existing_root(dir, &docset.root_path)?;
            // Publish the removal in one rename, then clean up the hidden directory.
            let trash = tempfile::Builder::new()
                .prefix(".odc-uninstall-")
                .tempdir_in(dir)
                .map_err(error)?;
            fs::rename(path, trash.path().join("removed")).map_err(error)?;
            return match trash.close() {
                Ok(()) => Ok(display_text(&format!("Uninstalled {}", docset.name))),
                Err(e) => Ok(format!(
                    "Uninstalled {}. Could not clean up temporary files: {e}",
                    docset.name
                )),
            };
        }

        let entry = entry.ok_or_else(|| {
            error(if cached.is_none() {
                "catalog is empty; run 'odc docsets refresh' first".to_string()
            } else {
                format!("docset not found in the catalog: {id}")
            })
        })?;
        if !valid_id(&entry.id) {
            return Err(error("invalid catalog ID"));
        }
        let path = match (action, installed) {
            (Action::Install, Some(_)) => return Err(error(format!("already installed: {id}"))),
            (Action::Update | Action::Reinstall, None) => {
                return Err(error(format!("docset is not installed: {id}")));
            }
            (Action::Reinstall, Some(docset)) => safe_existing_root(dir, &docset.root_path)?,
            (Action::Update, Some(docset)) => match has_update(entry, docset) {
                Some(true) => safe_existing_root(dir, &docset.root_path)?,
                Some(false) => return Ok(display_text(&format!("{} is up to date", docset.name))),
                None => {
                    return Err(error(format!(
                        "cannot determine the installed version of {id}"
                    )))
                }
            },
            _ => {
                let path = dir.join(format!("{}.docset", entry.id));
                match fs::symlink_metadata(&path) {
                    Err(e) if e.kind() == io::ErrorKind::NotFound => path,
                    Err(e) => return Err(error(e)),
                    Ok(_) => {
                        return Err(error(format!(
                            "installation path already exists: {}",
                            path.display()
                        )))
                    }
                }
            }
        };
        let client = feed::http_client().map_err(error)?;
        let limits = TarixLimits::default();
        if let Some(meta) = &entry.devdocs {
            if !opendoc_core::devdocs::is_valid_slug(&meta.slug) {
                return Err(error("invalid DevDocs slug"));
            }
            devdocs::install_devdocs(&client, entry, dir, &path, limits, progress)
                .map_err(error)?;
        } else {
            let mut entry = entry.clone();
            entry.urls =
                install::order_mirrors(&entry.urls, self.desktop_settings().fast_mirror.as_deref());
            install::download_and_install(&client, &entry, dir, &path, limits, progress)
                .map_err(error)?;
        }
        let verb = match action {
            Action::Install => "Installed",
            Action::Update => "Updated",
            Action::Reinstall => "Reinstalled",
            Action::Uninstall => unreachable!("uninstall handled before download"),
        };
        Ok(display_text(&format!(
            "{verb} {} ({})",
            entry.name, entry.version
        )))
    }
}

fn has_update(entry: &FeedEntry, docset: &Docset) -> Option<bool> {
    match &entry.devdocs {
        Some(meta) => {
            Some(meta.mtime? > opendoc_core::devdocs::installed_mtime(&docset.root_path)?)
        }
        None => update_available(docset.version.as_deref(), &entry.version),
    }
}

fn parse_repos(input: &str) -> Result<Vec<RepoRef>, CliError> {
    let mut repos = Vec::new();
    for part in input
        .split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .filter(|s| !s.is_empty())
    {
        let repo = feed::normalize_repo(part).map_err(error)?;
        if !repos
            .iter()
            .any(|old: &RepoRef| old.path().eq_ignore_ascii_case(&repo.path()))
        {
            repos.push(repo);
        }
    }
    if repos.len() > 5 {
        return Err(error("too many feed repositories (maximum 5)"));
    }
    Ok(repos)
}

fn repo_key(repos: &[RepoRef]) -> String {
    repos
        .iter()
        .map(RepoRef::path)
        .collect::<Vec<_>>()
        .join(",")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 160
        && id != "."
        && id != ".."
        && !id.starts_with('.')
        && !id.ends_with(['.', ' '])
        && !id
            .chars()
            .any(|c| c.is_control() || "/\\:*?\"<>|".contains(c))
        && !opendoc_core::docset::tarix::has_invalid_windows_component(Path::new(id))
}

fn safe_existing_root(dir: &Path, path: &Path) -> Result<PathBuf, CliError> {
    if !fs::symlink_metadata(path).map_err(error)?.is_dir() || folder_stem(path).is_none() {
        return Err(error(format!("unsafe docset folder: {}", path.display())));
    }
    let root = dir.canonicalize().map_err(error)?;
    let path = path.canonicalize().map_err(error)?;
    if path.parent() != Some(root.as_path()) {
        return Err(error(format!(
            "docset is outside the configured folder: {}",
            path.display()
        )));
    }
    Ok(path)
}

pub(crate) fn lock(path: &Path) -> Result<File, CliError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(error)?;
    file.try_lock()
        .map_err(|e| error(format!("cannot lock {}: {e}", path.display())))?;
    Ok(file)
}

pub(crate) fn save_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::other("missing data directory"))?;
    fs::create_dir_all(dir)?;
    let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer_pretty(tmp.as_file_mut(), value)?;
    tmp.flush()?;
    tmp.persist(path)?;
    Ok(())
}

pub(crate) fn display_text(text: &str) -> String {
    text.chars().filter(|ch| !ch.is_control()).collect()
}

pub(crate) fn error(error: impl std::fmt::Display) -> CliError {
    CliError::Management(error.to_string())
}

impl std::fmt::Display for ProgressReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.stage {
            ProgressStage::Downloading => {
                write!(f, "Downloading: {} B", self.received_bytes)?;
                if let Some(total) = self.total_bytes {
                    write!(f, " / {total} B")?;
                }
                Ok(())
            }
            ProgressStage::Extracting if self.files > 0 => {
                write!(f, "Extracting: {} files", self.files)
            }
            ProgressStage::Extracting => f.write_str("Extracting documentation…"),
            ProgressStage::Verifying => f.write_str("Verifying installation…"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn write_docset(path: &Path, version: &str) {
        let contents = path.join("Contents");
        let resources = contents.join("Resources");
        let documents = resources.join("Documents");
        fs::create_dir_all(&documents).expect("create fixture");
        fs::write(
            contents.join("Info.plist"),
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
                <plist version="1.0"><dict>
                  <key>CFBundleName</key><string>Example</string>
                  <key>CFBundleVersion</key><string>{version}</string>
                </dict></plist>"#
            ),
        )
        .expect("write plist");
        fs::write(documents.join("index.html"), format!("<h1>{version}</h1>")).expect("write HTML");
        let db = rusqlite::Connection::open(resources.join("docSet.dsidx")).expect("open index");
        db.execute_batch(
            "CREATE TABLE searchIndex (id INTEGER PRIMARY KEY, name TEXT, type TEXT, path TEXT);
             INSERT INTO searchIndex (name, type, path) VALUES ('Example', 'Guide', 'index.html');",
        )
        .expect("write index");
    }

    #[test]
    fn local_id_collision_cannot_uninstall_the_catalog_installation() {
        let scratch = tempfile::tempdir().expect("tempdir");
        let root = scratch.path();
        let dir = root.join("docsets");
        let catalog_install = dir.join("some-docs.docset");
        let local_install = dir.join("some docs.docset");
        write_docset(&catalog_install, "1.0");
        write_docset(&local_install, "7.0");

        let manager = Manager {
            locations: Locations {
                docsets: dir,
                tarix_cache: root.join("tarix-cache"),
            },
            data_dir: root.join("data"),
        };
        catalog::save_catalog(
            &manager.data_dir.join("odc-catalog.json"),
            &Catalog {
                repo: String::new(),
                fetched_at: 1,
                entries: vec![FeedEntry {
                    id: "some-docs".into(),
                    name: "Some docs".into(),
                    version: "2.0".into(),
                    urls: Vec::new(),
                    devdocs: None,
                }],
            },
        )
        .expect("seed catalog");
        let (rows, _) = manager.rows().unwrap();
        assert!(rows
            .iter()
            .any(|row| row.id == "some-docs" && row.status == Status::UpdateAvailable));
        assert!(rows
            .iter()
            .any(|row| row.id == "local:some-docs" && row.status == Status::Local));
        manager
            .change(Action::Uninstall, "local:some-docs", &|_| {})
            .unwrap();
        assert!(!local_install.exists());
        assert!(catalog_install.exists());
    }
}
