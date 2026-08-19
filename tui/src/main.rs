//! A terminal kanban board over an Obsidian "Ticket Vault".
//!
//! Usage: usher [vault-root]   (else `$USHER_VAULT`, else the current directory)

mod ui;
mod vault;

use std::path::PathBuf;
use std::time::{Duration, Instant};

use ratatui::crossterm::event::{self, Event, KeyCode};
use ratatui::crossterm::{execute, terminal};

use vault::{BOARD_STATUSES, STATUSES, Ticket};

const AUTO_RELOAD: Duration = Duration::from_secs(30);

pub enum Mode {
    Browse,
    /// Status picker. The payload is an index into `vault::STATUSES`.
    Status(usize),
    /// Single-line note input. The payload is the text typed until now.
    Note(String),
    /// Single-line search input. The payload is the text typed until now.
    Search(String),
    /// Project and tag pickers. The payload is an index into
    /// `App::filter_options`.
    ProjectFilter(usize),
    TagFilter(usize),
    /// Title of the new ticket. All other fields keep their default value.
    Create(String),
    /// Field picker. The payload is an index into `EDIT_FIELDS`.
    EditField(usize),
    /// One-line editor for a field. It starts with the current value.
    EditText { field: &'static str, buffer: String },
    /// Picker for a field with a fixed set of values.
    EditPick { field: &'static str, index: usize },
}

/// Editable frontmatter fields, in picker order. These fields go to
/// `vault::set_field` or `vault::set_tags`. To edit prose, use the detail view.
/// It edits the whole file.
pub const EDIT_FIELDS: [&str; 5] = ["title", "priority", "due", "project", "tags"];

/// Picker entry for "no project". It is equal to the empty value of `set_field`.
const NO_PROJECT: &str = "(none)";

pub struct App {
    vault: PathBuf,
    /// Every ticket on disk. The columns below come from this list.
    tickets: Vec<Ticket>,
    /// One list for each entry of `self.board()`, filtered and sorted.
    columns: Vec<Vec<Ticket>>,
    column: usize,
    /// Sized for the largest board (`show_archived` on). `board()` decides how
    /// many entries are in play at a given moment.
    selected: [usize; STATUSES.len()],
    mode: Mode,
    /// The open detail view: ticket path, raw markdown, and vertical scroll
    /// offset. The detail view is an overlay, not a `Mode`. Thus an edit popup
    /// can open above it, and the detail view stays open.
    detail: Option<(PathBuf, String, u16)>,
    message: Option<String>,
    pub search: String,
    /// `None` = all, `Some("")` = tickets with no project.
    pub project_filter: Option<String>,
    pub tag_filter: Option<String>,
    /// If false, the done column shows only the newest `done_limit()` tickets.
    pub done_all: bool,
    /// If true, the board shows a fifth column with archived tickets. Off by
    /// default: an archived ticket is done with, and should not crowd the board.
    pub show_archived: bool,
    /// How many done tickets the column shows while `done_all` is false.
    pub done_limit: usize,
    /// How many done tickets exist before the limit above cuts the column.
    /// The header names it, so the hidden ones are not a surprise.
    pub done_total: usize,
    /// Ticket that the main loop gives to `$EDITOR` after the next draw. The key
    /// handlers cannot do this. To suspend the terminal you need the `Terminal`,
    /// and the key handlers have no access to it.
    pending_editor: Option<PathBuf>,
    last_reload: Instant,
}

impl App {
    fn new(vault: PathBuf) -> Self {
        Self {
            vault,
            tickets: Vec::new(),
            columns: vec![Vec::new(); BOARD_STATUSES.len()],
            column: 0,
            selected: [0; STATUSES.len()],
            mode: Mode::Browse,
            detail: None,
            message: None,
            search: String::new(),
            project_filter: None,
            tag_filter: None,
            done_all: false,
            show_archived: false,
            done_limit: done_limit_from_env(),
            done_total: 0,
            pending_editor: None,
            last_reload: Instant::now(),
        }
    }

    fn reload(&mut self) {
        self.last_reload = Instant::now();
        match vault::load_tickets(&self.vault) {
            Ok((tickets, warnings)) => {
                self.tickets = tickets;
                self.rebuild();
                // Show this warning only if the operation before it has no
                // message. Each write causes a reload, and this warning is
                // about a different file. It must not replace the reason for
                // the failure of the edit that the user asked for. on_key
                // clears the message at each key press. The warning is thus
                // still visible on a simple view of the board.
                if let Some(first) = warnings.first()
                    && self.message.is_none()
                {
                    self.message = Some(format!("skipped {} unparsable file(s): {first}", warnings.len()));
                }
            }
            Err(e) => self.message = Some(e),
        }
        // An open detail view reads its file again. Thus it shows every write.
        if let Some((path, _, scroll)) = self.detail.take() {
            match vault::read(&path) {
                Ok(text) => self.detail = Some((path, text, scroll)),
                Err(e) => self.message = Some(e),
            }
        }
    }

    /// The statuses that make up the board's columns, in column order. Normally
    /// the four `BOARD_STATUSES`; with `show_archived` on, all five `STATUSES`
    /// (archived is the last entry of `STATUSES`, so it becomes the fifth
    /// column).
    fn board(&self) -> &'static [&'static str] {
        if self.show_archived { &STATUSES } else { &BOARD_STATUSES }
    }

    /// Make the board columns from the tickets in memory. The filters and the
    /// done-window switch use this function. It does not read the disk.
    fn rebuild(&mut self) {
        let limit = self.done_limit;
        let mut done_total = 0;
        self.columns = self
            .board()
            .iter()
            .map(|status| {
                let mut column: Vec<Ticket> = self
                    .tickets
                    .iter()
                    .filter(|t| t.status == *status && self.matches(t))
                    .cloned()
                    .collect();
                if *status == "done" {
                    // Newest finish first. closed holds a date, so a same-day
                    // tie falls back to the id, which grows with time. Priority
                    // order says nothing about work that is already over.
                    column.sort_by(|a, b| b.closed.cmp(&a.closed).then(b.id.cmp(&a.id)));
                    done_total = column.len();
                    // A count, not a time window. A time window shows nothing
                    // after a quiet week and hundreds of cards after a busy one.
                    if !self.done_all {
                        column.truncate(limit);
                    }
                } else if *status == "archived" {
                    // Same order as done: the work is over, so the priority
                    // order says nothing. Newest finish first.
                    column.sort_by(|a, b| b.closed.cmp(&a.closed).then(b.id.cmp(&a.id)));
                } else {
                    column.sort_by(ticket_sort);
                }
                column
            })
            .collect();
        self.done_total = done_total;
        for (i, column) in self.columns.iter().enumerate() {
            self.selected[i] = self.selected[i].min(column.len().saturating_sub(1));
        }
    }

    fn matches(&self, ticket: &Ticket) -> bool {
        let query = self.search.trim().to_lowercase();
        if !query.is_empty()
            && !ticket.id.to_lowercase().contains(&query)
            && !ticket.title.to_lowercase().contains(&query)
        {
            return false;
        }
        if self.project_filter.as_ref().is_some_and(|p| ticket.project != *p) {
            return false;
        }
        if self.tag_filter.as_ref().is_some_and(|t| !ticket.tags.contains(t)) {
            return false;
        }
        true
    }

    /// The picker entries. `None` is "all". `Some("")` is the "(no project)"
    /// entry. The other entries are the values on the loaded tickets.
    pub fn filter_options(&self, tag: bool) -> Vec<Option<String>> {
        let mut values: Vec<String> = if tag {
            self.tickets.iter().flat_map(|t| t.tags.clone()).collect()
        } else {
            self.tickets
                .iter()
                .map(|t| t.project.clone())
                .filter(|p| !p.is_empty())
                .collect()
        };
        values.sort();
        values.dedup();
        let mut options = vec![None];
        if !tag {
            options.push(Some(String::new()));
        }
        options.extend(values.into_iter().map(Some));
        options
    }

    /// Reload on the 30-second timer. Do not reload while a popup or an input
    /// is open.
    fn tick(&mut self) {
        if matches!(self.mode, Mode::Browse) && self.last_reload.elapsed() >= AUTO_RELOAD {
            self.reload();
        }
    }

    fn current(&self) -> Option<&Ticket> {
        self.columns[self.column].get(self.selected[self.column])
    }

    /// The ticket for an operation. If the detail view is open, use its ticket.
    /// If not, use the selected card. A status change moves the ticket to a
    /// different column. Then `current()` gives a different ticket, but the
    /// path of the ticket stays the same.
    fn target(&self) -> Option<&Ticket> {
        match &self.detail {
            Some((path, ..)) => self.tickets.iter().find(|t| t.path == *path),
            None => self.current(),
        }
    }

    /// Return true when the application must stop.
    fn on_key(&mut self, key: KeyCode) -> bool {
        self.message = None;
        if matches!(self.mode, Mode::Browse) && self.detail.is_some() {
            self.on_detail_key(key);
            return false;
        }
        match &mut self.mode {
            Mode::Browse => return self.on_browse_key(key),
            Mode::Status(index) => match key {
                KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = (*index + 1).min(STATUSES.len() - 1)
                }
                KeyCode::Enter => {
                    let status = STATUSES[*index];
                    self.mode = Mode::Browse;
                    self.apply(|path| vault::change_status(path, status, None));
                }
                KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            Mode::ProjectFilter(_) => self.on_filter_key(key, false),
            Mode::TagFilter(_) => self.on_filter_key(key, true),
            Mode::Search(buffer) => match key {
                KeyCode::Char(c) => buffer.push(c),
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Enter => {
                    self.search = std::mem::take(buffer);
                    self.mode = Mode::Browse;
                    self.rebuild();
                }
                // Esc removes the active filter. The web version does the same.
                KeyCode::Esc => {
                    self.search.clear();
                    self.mode = Mode::Browse;
                    self.rebuild();
                }
                _ => {}
            },
            Mode::Note(buffer) => match key {
                KeyCode::Char(c) => buffer.push(c),
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Enter => {
                    let note = std::mem::take(buffer);
                    self.mode = Mode::Browse;
                    self.apply(|path| vault::append_note(path, &note));
                }
                KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            Mode::Create(buffer) => match key {
                KeyCode::Char(c) => buffer.push(c),
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Enter => {
                    let title = std::mem::take(buffer);
                    self.mode = Mode::Browse;
                    self.create(&title);
                }
                KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            Mode::EditField(index) => match key {
                KeyCode::Up | KeyCode::Char('k') => *index = index.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => {
                    *index = (*index + 1).min(EDIT_FIELDS.len() - 1)
                }
                KeyCode::Enter => {
                    let field = EDIT_FIELDS[*index];
                    self.mode = Mode::Browse;
                    self.begin_edit(field);
                }
                KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            Mode::EditText { field, buffer } => match key {
                KeyCode::Char(c) => buffer.push(c),
                KeyCode::Backspace => {
                    buffer.pop();
                }
                KeyCode::Enter => {
                    let field = *field;
                    let value = std::mem::take(buffer);
                    self.mode = Mode::Browse;
                    self.commit_edit(field, value);
                }
                KeyCode::Esc => self.mode = Mode::Browse,
                _ => {}
            },
            Mode::EditPick { .. } => self.on_edit_pick_key(key),
        }
        false
    }

    fn on_browse_key(&mut self, key: KeyCode) -> bool {
        match key {
            KeyCode::Char('q') => return true,
            KeyCode::Left | KeyCode::Char('h') => self.column = self.column.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => {
                self.column = (self.column + 1).min(self.board().len() - 1)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected[self.column] = self.selected[self.column].saturating_sub(1)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let last = self.columns[self.column].len().saturating_sub(1);
                self.selected[self.column] = (self.selected[self.column] + 1).min(last);
            }
            KeyCode::Char('r') => self.reload(),
            KeyCode::Char('s') => {
                if let Some(ticket) = self.target() {
                    let at = STATUSES.iter().position(|s| *s == ticket.status).unwrap_or(0);
                    self.mode = Mode::Status(at);
                }
            }
            KeyCode::Char('m') => {
                if self.target().is_some() {
                    self.mode = Mode::Note(String::new());
                }
            }
            KeyCode::Char('n') => self.mode = Mode::Create(String::new()),
            KeyCode::Char('e') => {
                if self.target().is_some() {
                    self.mode = Mode::EditField(0);
                }
            }
            KeyCode::Char('/') => self.mode = Mode::Search(self.search.clone()),
            KeyCode::Char('p') => self.mode = Mode::ProjectFilter(self.filter_at(false)),
            KeyCode::Char('t') => self.mode = Mode::TagFilter(self.filter_at(true)),
            KeyCode::Char('.') => {
                self.done_all = !self.done_all;
                self.rebuild();
            }
            KeyCode::Char('a') => {
                self.show_archived = !self.show_archived;
                // Turning the fifth column off can leave the cursor past the
                // new last column.
                self.column = self.column.min(self.board().len() - 1);
                self.rebuild();
            }
            KeyCode::Enter => {
                if let Some(path) = self.current().map(|t| t.path.clone()) {
                    match vault::read(&path) {
                        Ok(text) => self.detail = Some((path, text, 0)),
                        Err(e) => self.message = Some(e),
                    }
                }
            }
            _ => {}
        }
        false
    }

    /// The keys for the open detail view. `s` and `m` use the Browse handlers.
    /// Their popup draws above the detail view and then sets `Mode::Browse`.
    /// The detail view stays open. The reload makes its text current.
    fn on_detail_key(&mut self, key: KeyCode) {
        let Some((path, _, scroll)) = &mut self.detail else {
            return;
        };
        match key {
            KeyCode::Up | KeyCode::Char('k') => *scroll = scroll.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => *scroll = scroll.saturating_add(1),
            KeyCode::Enter | KeyCode::Char('e') => self.pending_editor = Some(path.clone()),
            KeyCode::Esc | KeyCode::Char('q') => self.detail = None,
            KeyCode::Char('s') | KeyCode::Char('m') => {
                self.on_browse_key(key);
            }
            _ => {}
        }
    }

    /// Index of the active filter in its picker. The picker opens on the current
    /// choice, like the status picker.
    fn filter_at(&self, tag: bool) -> usize {
        let active = if tag { &self.tag_filter } else { &self.project_filter };
        self.filter_options(tag)
            .iter()
            .position(|o| o == active)
            .unwrap_or(0)
    }

    fn on_filter_key(&mut self, key: KeyCode, tag: bool) {
        let options = self.filter_options(tag);
        let (Mode::ProjectFilter(mut index) | Mode::TagFilter(mut index)) = self.mode else {
            return;
        };
        match key {
            KeyCode::Up | KeyCode::Char('k') => index = index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => index = (index + 1).min(options.len() - 1),
            KeyCode::Enter => {
                let choice = options[index].clone();
                if tag {
                    self.tag_filter = choice;
                } else {
                    self.project_filter = choice;
                }
                self.mode = Mode::Browse;
                self.rebuild();
                return;
            }
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                return;
            }
            _ => return,
        }
        self.mode = if tag {
            Mode::TagFilter(index)
        } else {
            Mode::ProjectFilter(index)
        };
    }

    /// The values for a field that the user picks and does not type.
    pub fn pick_options(&self, field: &str) -> Vec<String> {
        if field == "priority" {
            return vault::PRIORITIES.iter().map(|p| p.to_string()).collect();
        }
        let mut options = vec![NO_PROJECT.to_string()];
        options.extend(vault::list_projects(&self.vault));
        options
    }

    /// Open the correct editor for a field: a one-line input or a picker.
    fn begin_edit(&mut self, field: &'static str) {
        let Some(ticket) = self.target().cloned() else {
            return;
        };
        let mode = match field {
            "title" => Mode::EditText { field, buffer: ticket.title },
            "due" => Mode::EditText { field, buffer: ticket.due },
            "tags" => Mode::EditText { field, buffer: ticket.tags.join(" ") },
            "priority" => Mode::EditPick {
                field,
                index: vault::PRIORITIES
                    .iter()
                    .position(|p| *p == ticket.priority)
                    .unwrap_or(0),
            },
            "project" => Mode::EditPick {
                field,
                index: self
                    .pick_options(field)
                    .iter()
                    .position(|p| *p == ticket.project)
                    .unwrap_or(0),
            },
            other => unreachable!("{other} is not in EDIT_FIELDS"),
        };
        self.mode = mode;
    }

    fn commit_edit(&mut self, field: &'static str, value: String) {
        if field == "tags" {
            let tags: Vec<String> = value.split_whitespace().map(str::to_string).collect();
            self.apply(move |path| vault::set_tags(path, &tags));
            return;
        }
        let vault = self.vault.clone();
        self.apply(move |path| vault::set_field(&vault, path, field, value.trim()));
    }

    fn on_edit_pick_key(&mut self, key: KeyCode) {
        let Mode::EditPick { field, mut index } = self.mode else {
            return;
        };
        let options = self.pick_options(field);
        match key {
            KeyCode::Up | KeyCode::Char('k') => index = index.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => index = (index + 1).min(options.len() - 1),
            KeyCode::Enter => {
                let value = match options[index].as_str() {
                    NO_PROJECT => String::new(),
                    value => value.to_string(),
                };
                self.mode = Mode::Browse;
                self.commit_edit(field, value);
                return;
            }
            KeyCode::Esc => {
                self.mode = Mode::Browse;
                return;
            }
            _ => return,
        }
        self.mode = Mode::EditPick { field, index };
    }

    /// Ticket creation is the only write that does not use `apply`. There is no
    /// file for `apply` to write to yet.
    fn create(&mut self, title: &str) {
        match vault::create_ticket(&self.vault, title, "normal", "", "") {
            Ok(path) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                self.message = Some(format!("created {name}"));
            }
            Err(e) => self.message = Some(e),
        }
        self.reload();
    }

    /// Write to the selected ticket, then read the vault from disk again. This
    /// application keeps no file content between two operations.
    fn apply(&mut self, op: impl FnOnce(&std::path::Path) -> Result<(), String>) {
        let Some(path) = self.target().map(|t| t.path.clone()) else {
            return;
        };
        if let Err(e) = op(&path) {
            self.message = Some(e);
        }
        self.reload();
    }
}

pub fn option_label(option: &Option<String>) -> &str {
    match option.as_deref() {
        None => "all",
        Some("") => "(no project)",
        Some(value) => value,
    }
}

/// How many done tickets the board shows before the user asks for all of them.
/// server.js reads the same variable, so the two boards agree. usher already
/// configures the vault path through the environment.
fn done_limit_from_env() -> usize {
    std::env::var("USHER_DONE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(20)
}

/// Sort by priority, then by due date, then by id. An empty due date goes last.
/// This is the order of the web board.
fn ticket_sort(a: &Ticket, b: &Ticket) -> std::cmp::Ordering {
    let rank = |p: &str| match p {
        "urgent" => 0,
        "high" => 1,
        "normal" => 2,
        _ => 3,
    };
    let due = |t: &Ticket| {
        if t.due.is_empty() {
            "9999-99-99".to_string()
        } else {
            t.due.clone()
        }
    };
    rank(&a.priority)
        .cmp(&rank(&b.priority))
        .then_with(|| due(a).cmp(&due(b)))
        .then_with(|| a.id.cmp(&b.id))
}

/// The vault root from the caller. `None` means: use the current directory.
fn pick_root(arg: Option<String>, env: Option<String>) -> Option<PathBuf> {
    arg.or(env).map(PathBuf::from)
}

fn resolve_vault() -> Result<PathBuf, String> {
    let arg = std::env::args().nth(1);
    let root = match pick_root(arg, std::env::var("USHER_VAULT").ok()) {
        Some(root) => root,
        None => std::env::current_dir().map_err(|e| e.to_string())?,
    };
    if root.join("tickets").is_dir() {
        Ok(root)
    } else {
        Err(format!(
            "usage: usher [vault-root]  (or set USHER_VAULT)\nno tickets/ folder in {}",
            root.display()
        ))
    }
}

/// Give the whole ticket to `$EDITOR` while the TUI is suspended. If `$EDITOR`
/// is not set, use `notepad` on Windows and `vi` on other systems. Then write
/// back the result. The text goes out and comes back byte for byte. Thus only
/// the changes of the user change the file.
fn run_editor(
    terminal: &mut ratatui::DefaultTerminal,
    path: &std::path::Path,
) -> Result<(), String> {
    let name = path.file_stem().unwrap_or_default().to_string_lossy();
    let scratch = std::env::temp_dir().join(format!("{name}.md"));
    std::fs::write(&scratch, vault::read(path)?)
        .map_err(|e| format!("{}: {e}", scratch.display()))?;

    // The code between the exit from the alternate screen and the return to it
    // must not return early. If it returns early, the terminal keeps the state
    // of the editor.
    let fallback = if cfg!(windows) { "notepad" } else { "vi" };
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| fallback.to_string());
    let _ = terminal::disable_raw_mode();
    let _ = execute!(std::io::stdout(), terminal::LeaveAlternateScreen);
    let status = std::process::Command::new(&editor).arg(&scratch).status();
    let _ = execute!(std::io::stdout(), terminal::EnterAlternateScreen);
    let _ = terminal::enable_raw_mode();
    let _ = terminal.clear();
    // A non-zero editor exit means: discard this edit. In vim, `:cq` does this.
    if !status.map_err(|e| format!("{editor}: {e}"))?.success() {
        let _ = std::fs::remove_file(&scratch);
        return Err("editor exited nonzero, edit discarded".to_string());
    }

    let edited =
        std::fs::read_to_string(&scratch).map_err(|e| format!("{}: {e}", scratch.display()))?;
    let _ = std::fs::remove_file(&scratch);
    vault::set_content(path, &edited)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticket(id: &str, status: &str, priority: &str, due: &str, closed: &str) -> Ticket {
        Ticket {
            path: PathBuf::new(),
            id: id.to_string(),
            title: format!("title {id}"),
            status: status.to_string(),
            priority: priority.to_string(),
            project: String::new(),
            due: due.to_string(),
            closed: closed.to_string(),
            tags: vec!["setup".to_string()],
        }
    }

    fn ids(column: &[Ticket]) -> Vec<&str> {
        column.iter().map(|t| t.id.as_str()).collect()
    }

    #[test]
    fn a_reload_warning_does_not_replace_the_message_of_a_failed_edit() {
        let root = std::env::temp_dir().join("tui-main-test-reload-message");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("tickets")).unwrap();
        std::fs::write(root.join("tickets").join("junk.md"), "not a ticket at all").unwrap();

        let mut app = App::new(root.clone());
        app.reload();
        assert!(app.message.is_some(), "control: with nothing to say, the warning shows");

        app.message = Some("bad due date".to_string());
        app.reload();
        assert_eq!(app.message.as_deref(), Some("bad due date"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn columns_are_sorted_filtered_and_windowed() {
        let mut app = App::new(PathBuf::new());
        app.tickets = vec![
            ticket("T-0001", "open", "normal", "", ""),
            ticket("T-0002", "open", "urgent", "", ""),
            ticket("T-0003", "open", "normal", "2026-01-01", ""),
            ticket("T-0004", "done", "low", "", &vault::days_ago(1)),
            ticket("T-0005", "done", "low", "", "2000-01-01"),
            ticket("T-0006", "done", "urgent", "", &vault::days_ago(1)),
        ];
        app.rebuild();
        // Urgent first. A ticket with a due date comes before a ticket without
        // one. Then id.
        assert_eq!(ids(&app.columns[0]), ["T-0002", "T-0003", "T-0001"]);
        // The done column ignores priority: the work is over. Newest finish
        // first, and a same-day tie falls back to the id. T-0006 is urgent and
        // still sorts on its closed date alone.
        assert_eq!(ids(&app.columns[3]), ["T-0006", "T-0004", "T-0005"]);
        assert_eq!(app.done_total, 3, "the total counts what the limit hides");

        // The limit cuts the oldest, and the total still names all of them.
        app.done_limit = 2;
        app.rebuild();
        assert_eq!(ids(&app.columns[3]), ["T-0006", "T-0004"]);
        assert_eq!(app.done_total, 3);

        app.done_all = true;
        app.rebuild();
        assert_eq!(ids(&app.columns[3]), ["T-0006", "T-0004", "T-0005"]);
        app.done_all = false;
        app.done_limit = 20;

        app.search = "t-0003".to_string();
        app.rebuild();
        assert_eq!(ids(&app.columns[0]), ["T-0003"]);

        app.search.clear();
        app.tag_filter = Some("nope".to_string());
        app.rebuild();
        assert!(app.columns[0].is_empty());

        app.tag_filter = None;
        app.project_filter = Some(String::new());
        app.rebuild();
        assert_eq!(app.columns[0].len(), 3, "all fixtures have no project");
    }

    #[test]
    fn archived_column_only_appears_when_toggled_on() {
        let mut app = App::new(PathBuf::new());
        app.tickets = vec![
            ticket("T-0001", "open", "normal", "", ""),
            ticket("T-0002", "archived", "normal", "", &vault::days_ago(1)),
            ticket("T-0003", "archived", "normal", "", "2000-01-01"),
        ];
        app.rebuild();
        assert_eq!(app.columns.len(), 4, "no fifth column while show_archived is false");
        assert!(app.columns.iter().flatten().all(|t| t.status != "archived"));

        app.show_archived = true;
        app.rebuild();
        assert_eq!(app.columns.len(), 5);
        // Newest finish first, like the done column.
        assert_eq!(ids(&app.columns[4]), ["T-0002", "T-0003"]);
    }

    #[test]
    fn the_a_key_toggles_archived_and_clamps_the_column_cursor() {
        let mut app = App::new(PathBuf::new());
        app.tickets = vec![ticket("T-0001", "archived", "normal", "", "2026-01-01")];
        app.on_key(KeyCode::Char('a'));
        assert!(app.show_archived);
        app.column = 4;
        app.on_key(KeyCode::Char('a'));
        assert!(!app.show_archived);
        assert_eq!(app.column, 3, "clamped back onto the last visible column");
    }

    #[test]
    fn target_follows_the_open_detail_not_the_selection() {
        let mut app = App::new(PathBuf::new());
        let mut a = ticket("T-0001", "open", "normal", "", "");
        a.path = PathBuf::from("a.md");
        let mut b = ticket("T-0002", "open", "normal", "", "");
        b.path = PathBuf::from("b.md");
        app.tickets = vec![a, b];
        app.rebuild();
        app.selected[0] = 1;
        assert_eq!(app.current().unwrap().id, "T-0002");

        app.detail = Some((PathBuf::from("a.md"), String::new(), 0));
        assert_eq!(app.target().unwrap().id, "T-0001");

        app.detail = None;
        assert_eq!(app.target().unwrap().id, app.current().unwrap().id);
    }

    #[test]
    fn the_argument_outranks_the_environment() {
        let arg = || Some("from-arg".to_string());
        let env = || Some("from-env".to_string());
        assert_eq!(pick_root(arg(), env()), Some(PathBuf::from("from-arg")));
        assert_eq!(pick_root(None, env()), Some(PathBuf::from("from-env")));
        assert_eq!(pick_root(arg(), None), Some(PathBuf::from("from-arg")));
        assert_eq!(pick_root(None, None), None, "caller falls back to the cwd");
    }
}

fn main() -> std::io::Result<()> {
    let vault = match resolve_vault() {
        Ok(vault) => vault,
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    };
    let mut app = App::new(vault);
    app.reload();

    ratatui::run(|terminal| {
        loop {
            terminal.draw(|frame| ui::draw(frame, &app))?;
            if let Some(path) = app.pending_editor.take() {
                if let Err(e) = run_editor(terminal, &path) {
                    app.message = Some(e);
                }
                app.reload();
                continue;
            }
            if event::poll(Duration::from_secs(1))?
                && let Event::Key(key) = event::read()?
                && key.is_press()
                && app.on_key(key.code)
            {
                return Ok(());
            }
            app.tick();
        }
    })
}
