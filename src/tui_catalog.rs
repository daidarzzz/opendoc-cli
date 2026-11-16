use std::sync::{mpsc, Mutex};
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use opendoc_core::library::Library;
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::docsets::{display_text, Action, Manager, ProgressReport, Row, Status};
use crate::error::CliError;

const ACCENT: Color = Color::Rgb(137, 180, 250);
const TEXT: Color = Color::Rgb(205, 214, 244);
const GREEN: Color = Color::Rgb(166, 227, 161);
const RED: Color = Color::Rgb(243, 139, 168);
const SURFACE: Color = Color::Rgb(49, 50, 68);

enum Job {
    Open,
    Refresh,
    Change(Action, String),
    SaveFeeds(String),
}

#[derive(Default)]
enum Input {
    #[default]
    Browse,
    Confirm(Row),
    Feeds(String),
}

struct Outcome {
    rows: Option<Vec<Row>>,
    library: Option<Library>,
    status: Result<String, CliError>,
}

enum Event {
    Progress(String),
    Done(Box<Outcome>),
}

pub(crate) struct CatalogView {
    manager: Manager,
    rows: Vec<Row>,
    filter: String,
    selection: ListState,
    input: Input,
    status: String,
    failed: bool,
    changing_library: bool,
    worker: Option<mpsc::Receiver<Event>>,
}

impl CatalogView {
    pub(crate) fn open(manager: Manager) -> Self {
        let mut view = Self {
            manager,
            rows: Vec::new(),
            filter: String::new(),
            selection: ListState::default(),
            input: Input::Browse,
            status: String::new(),
            failed: false,
            changing_library: false,
            worker: None,
        };
        view.start(Job::Open);
        view
    }

    pub(crate) fn busy(&self) -> bool {
        self.worker.is_some()
    }

    fn start(&mut self, job: Job) {
        let (send, recv) = mpsc::channel();
        let manager = self.manager.clone();
        self.status = match &job {
            Job::Change(action, id) => format!("{} {id}…", action.label()),
            Job::SaveFeeds(_) | Job::Refresh => "Refreshing the catalog…".into(),
            Job::Open => "Loading the catalog…".into(),
        };
        self.failed = false;
        self.worker = Some(recv);
        self.changing_library = matches!(&job, Job::Change(..));
        let changes_library = self.changing_library;
        let worker = std::thread::Builder::new()
            .name("odc-catalog".into())
            .spawn(move || {
                let last = Mutex::new(None);
                let progress = |report: ProgressReport| {
                    if let Ok(mut last) = last.lock() {
                        let now = Instant::now();
                        if last.is_none_or(|(when, stage)| {
                            stage != report.stage || now.duration_since(when).as_millis() >= 100
                        }) {
                            let _ = send.send(Event::Progress(report.to_string()));
                            *last = Some((now, report.stage));
                        }
                    }
                };
                let status = match job {
                    Job::Open => manager.has_catalog().and_then(|ready| {
                        if ready {
                            Ok("Catalog loaded.".into())
                        } else {
                            manager.refresh()
                        }
                    }),
                    Job::Refresh => manager.refresh(),
                    Job::Change(action, id) => manager.change(action, &id, &progress),
                    Job::SaveFeeds(feeds) => {
                        manager.set_feeds(&feeds).and_then(|()| manager.refresh())
                    }
                };
                let mut outcome = Outcome {
                    rows: None,
                    library: None,
                    status,
                };
                if changes_library && outcome.status.is_ok() {
                    match crate::run::load(&manager.locations) {
                        Ok(library) => outcome.library = Some(library),
                        Err(e) => outcome.status = Err(e),
                    }
                }
                match manager.rows() {
                    Ok((rows, _)) => outcome.rows = Some(rows),
                    Err(e) if outcome.status.is_ok() => outcome.status = Err(e),
                    Err(_) => {}
                }
                let _ = send.send(Event::Done(Box::new(outcome)));
            });
        if let Err(e) = worker {
            self.worker = None;
            self.failed = true;
            self.status = format!("Cannot start catalog worker: {e}");
        }
    }

    pub(crate) fn poll(&mut self) -> Option<Library> {
        while let Some(worker) = &self.worker {
            match worker.try_recv() {
                Ok(Event::Progress(status)) => self.status = status,
                Ok(Event::Done(outcome)) => {
                    self.worker = None;
                    if let Some(rows) = outcome.rows {
                        let id = self.selected().map(|r| r.id.clone());
                        self.rows = rows;
                        let selected = id
                            .as_deref()
                            .and_then(|id| self.visible().position(|r| r.id == id));
                        self.selection.select(selected.or_else(|| self.first()));
                    }
                    self.failed = outcome.status.is_err();
                    self.status = outcome.status.unwrap_or_else(|e| e.to_string());
                    return outcome.library;
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.worker = None;
                    self.failed = true;
                    self.status = "Catalog worker stopped unexpectedly.".into();
                }
            }
        }
        None
    }

    fn visible(&self) -> impl Iterator<Item = &Row> {
        self.rows.iter().filter(|r| r.matches(&self.filter))
    }

    fn first(&self) -> Option<usize> {
        self.visible().next().map(|_| 0)
    }

    fn selected(&self) -> Option<&Row> {
        self.visible().nth(self.selection.selected()?)
    }

    /// Returns true when the reader should take focus again.
    pub(crate) fn key(&mut self, key: KeyEvent) -> bool {
        match &mut self.input {
            Input::Confirm(row) => {
                match key.code {
                    KeyCode::Char('y' | 'Y') => {
                        let id = row.id.clone();
                        self.input = Input::Browse;
                        self.start(Job::Change(Action::Uninstall, id));
                    }
                    KeyCode::Char('n' | 'N') | KeyCode::Esc => self.input = Input::Browse,
                    _ => {}
                }
                return false;
            }
            Input::Feeds(text) => {
                match key.code {
                    KeyCode::Enter => {
                        let feeds = std::mem::take(text);
                        self.input = Input::Browse;
                        self.start(Job::SaveFeeds(feeds));
                    }
                    KeyCode::Esc => self.input = Input::Browse,
                    KeyCode::Backspace => {
                        text.pop();
                    }
                    KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                        text.push(c)
                    }
                    _ => {}
                }
                return false;
            }
            Input::Browse => {}
        }
        match key.code {
            KeyCode::Esc | KeyCode::F(4) if !self.busy() || !self.changing_library => return true,
            KeyCode::F(5) if !self.busy() => self.start(Job::Refresh),
            KeyCode::F(6) if !self.busy() => match self.manager.feeds() {
                Ok(repos) => {
                    self.input = Input::Feeds(
                        repos
                            .iter()
                            .map(|repo| repo.path())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                }
                Err(e) => {
                    self.failed = true;
                    self.status = e.to_string();
                }
            },
            KeyCode::Up | KeyCode::Down | KeyCode::PageUp | KeyCode::PageDown => {
                let delta = match key.code {
                    KeyCode::Up => -1,
                    KeyCode::Down => 1,
                    KeyCode::PageUp => -10,
                    _ => 10,
                };
                let len = self.visible().count();
                if len != 0 {
                    let current = self.selection.selected().unwrap_or(0);
                    self.selection
                        .select(Some(current.saturating_add_signed(delta).min(len - 1)));
                }
            }
            KeyCode::Enter if !self.busy() => {
                if let Some((action, id)) = self
                    .selected()
                    .and_then(|r| Some((r.status.primary_action()?, r.id.clone())))
                {
                    self.start(Job::Change(action, id));
                }
            }
            KeyCode::Delete if !self.busy() => {
                if let Some(row) = self.selected().filter(|r| r.status.can_uninstall()) {
                    self.input = Input::Confirm(row.clone());
                }
            }
            KeyCode::Backspace => {
                self.filter.pop();
                self.selection.select(self.first());
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filter.push(c);
                self.selection.select(self.first());
            }
            _ => {}
        }
        false
    }

    pub(crate) fn draw(&mut self, frame: &mut ratatui::Frame) {
        let areas = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(frame.area());
        let (title, text) = match &self.input {
            Input::Feeds(value) => (
                " Additional GitHub feeds · comma-separated ",
                value.as_str(),
            ),
            _ => (
                " Docset Catalog · filter by name or ID ",
                self.filter.as_str(),
            ),
        };
        let block = |title| {
            Block::default()
                .borders(Borders::ALL)
                .title(title)
                .border_style(Style::default().fg(ACCENT))
        };
        frame.render_widget(
            Paragraph::new(text)
                .style(Style::default().fg(TEXT))
                .block(block(title)),
            areas[0],
        );

        let items: Vec<_> = self
            .visible()
            .map(|row| {
                let color = match row.status {
                    Status::Installed | Status::Local | Status::UpdateAvailable => GREEN,
                    Status::Ambiguous => RED,
                    _ => TEXT,
                };
                ListItem::new(row.description()).style(Style::default().fg(color))
            })
            .collect();
        if items.is_empty() && !self.busy() {
            frame.render_widget(
                Paragraph::new("No matching docsets.").block(block(" Docsets ")),
                areas[1],
            );
        } else {
            frame.render_stateful_widget(
                List::new(items)
                    .block(block(" Docsets "))
                    .highlight_style(Style::default().bg(SURFACE).add_modifier(Modifier::BOLD))
                    .highlight_symbol("› "),
                areas[1],
                &mut self.selection,
            );
        }

        let status = match &self.input {
            Input::Confirm(row) => display_text(&format!(
                "Uninstall {} [{}]? Press Y to confirm.",
                row.name, row.id
            )),
            Input::Feeds(_) => {
                "Enter saves and refreshes the catalog. An empty list uses DevDocs only.".into()
            }
            Input::Browse => self.status.clone(),
        };
        let hints = match &self.input {
            Input::Confirm(_) => "Y Confirm    Esc Cancel".into(),
            Input::Feeds(_) => "Enter Save    Esc Cancel".into(),
            Input::Browse if self.busy() => "↑↓ Browse    Type to filter".into(),
            Input::Browse => {
                let action = self
                    .selected()
                    .and_then(|r| r.status.primary_action())
                    .map(|a| format!("Enter {}    ", a.label()))
                    .unwrap_or_default();
                let uninstall = if self.selected().is_some_and(|r| r.status.can_uninstall()) {
                    "Del Uninstall    "
                } else {
                    ""
                };
                format!("{action}{uninstall}F5 Refresh    F6 Feeds    Esc Back")
            }
        };
        frame.render_widget(
            Paragraph::new(status)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(if self.failed { RED } else { TEXT })),
            areas[2],
        );
        frame.render_widget(
            Paragraph::new(hints).style(Style::default().fg(ACCENT)),
            areas[3],
        );
    }
}
