//! Reading and editing Obsidian "Ticket Vault" notes.
//!
//! This module has no UI dependency. Thus another front end can use it. It
//! reads and writes frontmatter line by line. The schema has a fixed key order
//! and a small set of values. A YAML parser gives no help here, and it would
//! change the format of the file on each write.
//!
//! Each write touches only the lines that must change. It keeps the
//! line-ending style of the file.

use chrono::Local;
use std::fs;
use std::path::{Path, PathBuf};

pub const STATUSES: [&str; 5] = ["open", "doing", "review", "done", "archived"];

pub const PRIORITIES: [&str; 4] = ["urgent", "high", "normal", "low"];

/// The columns of the board, in order. `archived` is not on the board.
pub const BOARD_STATUSES: [&str; 4] = ["open", "doing", "review", "done"];

#[derive(Clone, Debug)]
pub struct Ticket {
    pub path: PathBuf,
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub project: String,
    pub due: String,
    pub closed: String,
    pub tags: Vec<String>,
}

impl Ticket {
    fn parse(path: &Path, text: &str) -> Result<Self, String> {
        let lines = split_lines(text);
        // Every other error branch of this function names the file. Do the
        // same here, so a warning collected across many files (load_tickets)
        // says which one is a plain note with no frontmatter at all.
        let end = frontmatter_end(&lines).map_err(|e| format!("{}: {e}", path.display()))?;
        let get = |key: &str| fm_get(&lines[1..end], key).unwrap_or_default();

        let status = unquote(&get("status"));
        if !STATUSES.contains(&status.as_str()) {
            return Err(format!("{}: bad status \"{status}\"", path.display()));
        }
        let id = unquote(&get("id"));
        if id.is_empty() {
            return Err(format!("{}: missing id", path.display()));
        }
        Ok(Self {
            path: path.to_path_buf(),
            id,
            title: unquote(&get("title")),
            status,
            priority: unquote(&get("priority")),
            project: unlink(&get("project")),
            due: unquote(&get("due")),
            closed: unquote(&get("closed")),
            tags: fm_get_list(&lines[1..end], "tags"),
        })
    }

    pub fn is_overdue(&self) -> bool {
        // A date is YYYY-MM-DD. Thus a text comparison gives the correct order.
        !self.due.is_empty() && self.due < today() && self.status != "done"
    }
}

/// Load every `<vault>/tickets/*.md` note, sorted by id. A file that fails to
/// read or to parse does not stop the load: it is skipped, and its message
/// (which names the file) goes into the second return value. Only a failure
/// to read the `tickets/` directory itself is an `Err`.
pub fn load_tickets(vault: &Path) -> Result<(Vec<Ticket>, Vec<String>), String> {
    let dir = vault.join("tickets");
    let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut tickets = Vec::new();
    let mut warnings = Vec::new();
    for entry in entries {
        let path = match entry {
            Ok(e) => e.path(),
            Err(e) => {
                warnings.push(e.to_string());
                continue;
            }
        };
        if !path.extension().is_some_and(|e| e == "md") {
            continue;
        }
        match read(&path).and_then(|text| Ticket::parse(&path, &text)) {
            Ok(ticket) => tickets.push(ticket),
            Err(e) => warnings.push(e),
        }
    }
    tickets.sort_by(|a, b| a.id.cmp(&b.id));
    Ok((tickets, warnings))
}

/// Move a ticket to `new_status` and apply the transition rules of the vault.
/// Add exactly one `## Log` line. If the ticket already has `new_status`,
/// change nothing.
pub fn change_status(path: &Path, new_status: &str, note: Option<&str>) -> Result<(), String> {
    if !STATUSES.contains(&new_status) {
        return Err(format!("unknown status \"{new_status}\""));
    }
    let original = read(path)?;
    let eol = detect_eol(&original);
    let mut lines = split_lines(&original);
    let end = frontmatter_end(&lines)?;

    let status = unquote(&fm_get(&lines[1..end], "status").unwrap_or_default());
    if status == new_status {
        return Ok(());
    }

    fm_set(&mut lines[1..end], "status", new_status)?;
    if new_status == "done" {
        fm_set(&mut lines[1..end], "closed", &today())?;
    } else if new_status != "archived" {
        // Leaving done clears closed again, except to archived: that keeps
        // the record of when the work finished.
        fm_set(&mut lines[1..end], "closed", "")?;
    }
    append_log_line(&mut lines, &transition_note(new_status, note))?;

    write_verified(path, Some(&original), lines.join(eol))
}

/// Add a free-text note to the `## Log` section. Change nothing else.
pub fn append_note(path: &Path, note: &str) -> Result<(), String> {
    let note = note.trim();
    if note.is_empty() {
        return Err("empty note".to_string());
    }
    let original = read(path)?;
    let eol = detect_eol(&original);
    let mut lines = split_lines(&original);
    frontmatter_end(&lines)?;

    append_log_line(&mut lines, note)?;
    write_verified(path, Some(&original), lines.join(eol))
}

/// Set one frontmatter field. Validate and format it like the `/fields`
/// endpoint of server.js. The schema owns these fields: status, created,
/// closed, branch, repos and id. This function cannot change them.
pub fn set_field(vault: &Path, path: &Path, field: &str, value: &str) -> Result<(), String> {
    let formatted = match field {
        "title" => {
            let title = value.trim();
            if title.is_empty() {
                return Err("title required".to_string());
            }
            quote_yaml(title)
        }
        "priority" => {
            if !PRIORITIES.contains(&value) {
                return Err(format!("bad priority \"{value}\""));
            }
            value.to_string()
        }
        "due" => {
            if !value.is_empty() && !is_date(value) {
                return Err("bad due (want YYYY-MM-DD)".to_string());
            }
            value.to_string()
        }
        "project" if value.is_empty() => String::new(),
        "project" => {
            if !project_exists(vault, value) {
                return Err(format!("unknown project \"{value}\""));
            }
            format!("\"[[{value}]]\"")
        }
        _ => return Err(format!("field \"{field}\" is not editable")),
    };
    let original = read(path)?;
    let eol = detect_eol(&original);
    let mut lines = split_lines(&original);
    let end = frontmatter_end(&lines)?;

    fm_set(&mut lines[1..end], field, &formatted)?;

    write_verified(path, Some(&original), lines.join(eol))
}

/// Replace the full `tags:` block. The block is the key line and its
/// `  - item` lines. Keep the order of the given tags.
pub fn set_tags(path: &Path, tags: &[String]) -> Result<(), String> {
    let mut unique: Vec<String> = Vec::new();
    for tag in tags {
        if !is_valid_tag(tag) {
            return Err(format!("bad tag \"{tag}\""));
        }
        if !unique.contains(tag) {
            unique.push(tag.clone());
        }
    }
    let original = read(path)?;
    let eol = detect_eol(&original);
    let mut lines = split_lines(&original);
    let end = frontmatter_end(&lines)?;

    let at = 1 + lines[1..end]
        .iter()
        .position(|l| parse_kv(l).is_some_and(|(k, _)| k == "tags"))
        .ok_or_else(|| "no `tags:` line in the frontmatter".to_string())?;
    // Use the same continuation rule as fm_get_list. Thus this function
    // replaces exactly the lines that the reader parses.
    let items = lines[at + 1..end]
        .iter()
        .take_while(|l| l.starts_with(char::is_whitespace) && l.trim_start().starts_with("- "))
        .count();

    let block: Vec<String> = if unique.is_empty() {
        vec!["tags: []".to_string()]
    } else {
        std::iter::once("tags:".to_string())
            .chain(unique.iter().map(|t| format!("  - {t}")))
            .collect()
    };
    lines.splice(at..at + 1 + items, block);

    write_verified(path, Some(&original), lines.join(eol))
}

/// The body of a section, without the empty lines around it. This is the part
/// that a user edits. The result always uses LF. The write functions put the
/// line endings of the file back.
pub fn get_section(text: &str, header: &str) -> Result<String, String> {
    let lines = split_lines(text);
    let (start, end) = section_bounds(&lines, header)?;
    Ok(lines[start..end].join("\n").trim().to_string())
}

/// The project note names, sorted. A project note is either
/// `<vault>/projects/<name>.md`, or `<vault>/projects/<name>/<name>.md` when the
/// vault uses one folder per project (an Obsidian folder note). Only the top
/// level is scanned, so the theme folders that may live inside a project folder
/// do not end up in the list.
pub fn list_projects(vault: &Path) -> Vec<String> {
    let mut names = Vec::new();
    let Ok(entries) = fs::read_dir(vault.join("projects")) else {
        return names;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|s| s.to_str())
                && path.join(format!("{name}.md")).is_file()
            {
                names.push(name.to_string());
            }
        } else if path.extension().is_some_and(|e| e == "md")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            names.push(stem.to_string());
        }
    }
    names.sort();
    names
}

/// Checked against the list, not against a built path, so that a name with a
/// separator or ".." in it can never resolve to a file outside `projects/`.
pub fn project_exists(vault: &Path, name: &str) -> bool {
    list_projects(vault).iter().any(|n| n == name)
}

/// The rule of system/scripts/next_ticket_id.js: one more than the highest
/// number in the `id` or in the file name of a ticket.
pub fn next_ticket_id(vault: &Path) -> Result<String, String> {
    let dir = vault.join("tickets");
    let entries = fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut max = 0;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if !path.extension().is_some_and(|e| e == "md") {
            continue;
        }
        let text = read(&path)?;
        let lines = split_lines(&text);
        let id = frontmatter_end(&lines)
            .ok()
            .and_then(|end| fm_get(&lines[1..end], "id"))
            .map(|v| unquote(&v))
            .unwrap_or_default();
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        for candidate in [&id, &name] {
            max = max.max(leading_ticket_number(candidate).unwrap_or(0));
        }
    }
    if max + 1 > 9999 {
        return Err("ticket id space exhausted (T-9999 cap)".to_string());
    }
    Ok(format!("T-{:04}", max + 1))
}

/// The number in a leading `T-<digits>`, like `/^T-(\d+)/` in next_ticket_id.js.
fn leading_ticket_number(value: &str) -> Option<u32> {
    value
        .strip_prefix("T-")?
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// A new ticket. It matches system/templates/ticket.md byte for byte in key
/// order, in quoting and in empty-field spacing. It always uses LF, because
/// Templater writes LF.
fn new_ticket_content(
    id: &str,
    title: &str,
    priority: &str,
    due: &str,
    project: &str,
    basename: &str,
) -> String {
    [
        "---".to_string(),
        format!("id: {id}"),
        format!("title: {}", quote_yaml(title)),
        "status: open".to_string(),
        format!("priority: {priority}"),
        if project.is_empty() {
            "project: ".to_string()
        } else {
            format!("project: \"[[{project}]]\"")
        },
        "repos: []".to_string(),
        "tags: []".to_string(),
        format!("created: {}", today()),
        if due.is_empty() {
            "due: ".to_string()
        } else {
            format!("due: {due}")
        },
        "closed: ".to_string(),
        format!("branch: {basename}"),
        "---".to_string(),
        String::new(),
        "## Summary".to_string(),
        String::new(),
        title.to_string(),
        String::new(),
        "## Notes".to_string(),
        String::new(),
        "- ".to_string(),
        String::new(),
        "## Log".to_string(),
        String::new(),
        format!("- {} \u{2014} created", now_stamp()),
        String::new(),
    ]
    .join("\n")
}

/// Create `tickets/<id>-<slug>.md` and return its path.
pub fn create_ticket(
    vault: &Path,
    title: &str,
    priority: &str,
    due: &str,
    project: &str,
) -> Result<PathBuf, String> {
    let title = title.trim();
    if title.is_empty() {
        return Err("title required".to_string());
    }
    if !PRIORITIES.contains(&priority) {
        return Err(format!("bad priority \"{priority}\""));
    }
    if !due.is_empty() && !is_date(due) {
        return Err("bad due (want YYYY-MM-DD)".to_string());
    }
    if !project.is_empty() && !project_exists(vault, project) {
        return Err(format!("unknown project \"{project}\""));
    }
    let id = next_ticket_id(vault)?;
    let slug = slugify(title);
    let basename = if slug.is_empty() {
        id.clone()
    } else {
        format!("{id}-{slug}")
    };
    let path = vault.join("tickets").join(format!("{basename}.md"));
    // next_ticket_id prevents this. This function refuses it again, because a
    // create that fails deletes the file that it wrote.
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    write_verified(
        &path,
        None,
        new_ticket_content(&id, title, priority, due, project, &basename),
    )?;
    Ok(path)
}

/// The schema checks of the vault, from `system/scripts/check_vault.js`, for one
/// file. This function makes every check of that script. It does not check for
/// duplicate ids, because that check needs the full folder. `next_ticket_id`
/// prevents duplicate ids. This function is pure. Thus it runs before the write,
/// not after it.
pub fn lint_content(file_name: &str, text: &str) -> Vec<String> {
    let mut errors: Vec<String> = Vec::new();
    let lines = split_lines(text);
    let Ok(end) = frontmatter_end(&lines) else {
        return vec![format!("{file_name}: missing frontmatter")];
    };
    let fm = &lines[1..end];
    let raw = |key: &str| fm_get(fm, key);
    let get = |key: &str| raw(key).map(|v| unquote(&v));

    let id = get("id").unwrap_or_default();
    if !is_ticket_id(&id) {
        errors.push(match raw("id") {
            None => "missing id".to_string(),
            Some(_) => format!("bad id \"{id}\" (want T-NNNN, 4 digits)"),
        });
    } else if !file_name.starts_with(&id) {
        errors.push(format!("filename does not start with {id}"));
    }

    // Check the raw value. The quoting is the subject of this check. If the code
    // removes the quotes first, it cannot see an unquoted ":".
    let title = raw("title").unwrap_or_default();
    if title.is_empty() || title == "\"\"" || title == "''" {
        errors.push("title missing".to_string());
    } else if title.starts_with(['"', '\'']) {
        if !is_well_formed_quoted(&title) {
            errors.push(format!("malformed quoted title {title}"));
        }
    } else if title.contains(':') {
        errors.push("title with ':' must be quoted".to_string());
    }

    let status = get("status");
    if !status.as_deref().is_some_and(|s| STATUSES.contains(&s)) {
        errors.push(match &status {
            None => "missing status".to_string(),
            Some(s) => format!("bad status \"{s}\""),
        });
    }
    let priority = get("priority");
    if !priority.as_deref().is_some_and(|p| PRIORITIES.contains(&p)) {
        errors.push(match &priority {
            None => "missing priority".to_string(),
            Some(p) => format!("bad priority \"{p}\""),
        });
    }
    match get("created") {
        None => errors.push("missing created".to_string()),
        Some(created) if created.is_empty() => errors.push("missing created".to_string()),
        Some(created) if !is_date(&created) => {
            errors.push(format!("bad created \"{created}\" (want YYYY-MM-DD)"))
        }
        Some(_) => {}
    }
    for key in ["due", "closed"] {
        let value = get(key).unwrap_or_default();
        if !value.is_empty() && !is_date(&value) {
            errors.push(format!("bad {key} \"{value}\" (want YYYY-MM-DD)"));
        }
    }
    if status.as_deref() == Some("done") && get("closed").unwrap_or_default().is_empty() {
        errors.push("status done but closed is empty".to_string());
    }
    let branch = get("branch").unwrap_or_default();
    if !branch.is_empty() && !branch.starts_with(&id) {
        errors.push(format!("branch \"{branch}\" does not start with {id}"));
    }

    errors
        .iter()
        .map(|e| format!("{file_name}: {e}"))
        .collect()
}

/// Replace the full content of a ticket. Lint and verify it like every other
/// write. If the new content is the same, write nothing.
pub fn set_content(path: &Path, new_text: &str) -> Result<(), String> {
    let original = read(path)?;
    if new_text == original {
        return Ok(());
    }
    write_verified(path, Some(&original), new_text.to_string())
}

pub fn read(path: &Path) -> Result<String, String> {
    fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

pub fn today() -> String {
    Local::now().format("%Y-%m-%d").to_string()
}

/// `today - days`, for the window of the done column.
pub fn days_ago(days: i64) -> String {
    (Local::now() - chrono::Duration::days(days))
        .format("%Y-%m-%d")
        .to_string()
}

fn now_stamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// The only function in this module that writes to the disk. It lints `modified`
/// first. Thus a schema error causes no write. Then it writes the file, reads
/// the file again and compares the bytes. If the bytes differ, it writes
/// `original` back. If there was no original file (`original == None`, a new
/// ticket), it deletes the file.
fn write_verified(path: &Path, original: Option<&str>, modified: String) -> Result<(), String> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let problems = lint_content(name, &modified);
    if !problems.is_empty() {
        return Err(problems.join("; "));
    }
    fs::write(path, &modified).map_err(|e| format!("{}: {e}", path.display()))?;

    if read(path)? == modified {
        return Ok(());
    }
    match original {
        Some(original) => {
            fs::write(path, original).map_err(|e| {
                format!("{}: verification failed AND restore failed: {e}", path.display())
            })?;
            Err(format!(
                "{}: write verification failed, file restored",
                path.display()
            ))
        }
        None => {
            fs::remove_file(path).map_err(|e| {
                format!("{}: verification failed AND cleanup failed: {e}", path.display())
            })?;
            Err(format!(
                "{}: write verification failed, file removed",
                path.display()
            ))
        }
    }
}

/// The same text that `logMessageFor` in server.js writes. Thus the Log of a
/// ticket is the same for each front end.
fn transition_note(status: &str, note: Option<&str>) -> String {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    match status {
        // The operating manual gives this exact word.
        "doing" => "started".to_string(),
        "done" => note.map_or("done".to_string(), |n| format!("done: {n}")),
        "review" => note.unwrap_or("review requested").to_string(),
        "archived" => note.unwrap_or("archived").to_string(),
        _ => note.unwrap_or("reopened").to_string(),
    }
}

/// Read the line ending from the bytes of the file. Do not use the line ending
/// of the platform.
fn detect_eol(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}

/// This function splits on the line ending of the file. A join with the same
/// line ending gives the same bytes. Thus a line that no edit touches stays
/// the same.
fn split_lines(text: &str) -> Vec<String> {
    text.split(detect_eol(text)).map(str::to_string).collect()
}

fn frontmatter_end(lines: &[String]) -> Result<usize, String> {
    if lines.first().map(|l| l.trim_end()) != Some("---") {
        return Err("missing frontmatter".to_string());
    }
    lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, l)| l.trim_end() == "---")
        .map(|(i, _)| i)
        .ok_or_else(|| "unterminated frontmatter".to_string())
}

/// `key: value`. This is the Rust form of `^(\w+):\s*(.*)$` in check_vault.js.
/// An indented list item (`  - "[[repo]]"`) fails the key test. This function
/// skips it.
fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once(':')?;
    if key.is_empty() || !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return None;
    }
    Some((key, value.trim()))
}

fn fm_get(fm: &[String], key: &str) -> Option<String> {
    fm.iter()
        .filter_map(|l| parse_kv(l))
        .find(|(k, _)| *k == key)
        .map(|(_, v)| v.to_string())
}

/// A block-list value: `key:` alone on its line, then indented `  - item` lines.
/// An inline value (`tags: []`) has no items.
fn fm_get_list(fm: &[String], key: &str) -> Vec<String> {
    let Some(at) = fm
        .iter()
        .position(|l| parse_kv(l).is_some_and(|(k, v)| k == key && v.is_empty()))
    else {
        return Vec::new();
    };
    fm[at + 1..]
        .iter()
        .take_while(|l| l.starts_with(char::is_whitespace) && l.trim_start().starts_with("- "))
        .map(|l| unquote(l.trim().trim_start_matches("- ").trim()))
        .collect()
}

fn fm_set(fm: &mut [String], key: &str, value: &str) -> Result<(), String> {
    let line = fm
        .iter_mut()
        .find(|l| parse_kv(l).is_some_and(|(k, _)| k == key))
        .ok_or_else(|| format!("no `{key}:` line in the frontmatter"))?;
    // The vault writes an empty value as "key: ": colon, space, nothing.
    *line = if value.is_empty() {
        format!("{key}: ")
    } else {
        format!("{key}: {value}")
    };
    Ok(())
}

/// The body of a `## <header>` section, as a line range. It starts after the
/// header line. It stops at the next `## ` heading or at the end of the file.
fn section_bounds(lines: &[String], header: &str) -> Result<(usize, usize), String> {
    let heading = format!("## {header}");
    let start = 1 + lines
        .iter()
        .position(|l| *l == heading)
        .ok_or_else(|| format!("missing ## {header} section"))?;
    let end = lines[start..]
        .iter()
        .position(|l| l.starts_with("## "))
        .map_or(lines.len(), |i| start + i);
    Ok((start, end))
}

fn append_log_line(lines: &mut Vec<String>, text: &str) -> Result<(), String> {
    let (start, end) = section_bounds(lines, "Log")?;
    // Insert after the last line with text in the section. Thus the empty line
    // at the end stays. The separator before the next heading also stays.
    let mut at = end;
    while at > start && lines[at - 1].trim().is_empty() {
        at -= 1;
    }
    lines.insert(at, format!("- {} \u{2014} {text}", now_stamp()));
    Ok(())
}

/// The YAML string escaping of the Templater ticket template.
pub fn quote_yaml(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\""))
}

/// The slug rule of the Templater template: lowercase ASCII kebab-case, with a
/// maximum of 40 characters. Each group of other characters becomes one `-`.
pub fn slugify(title: &str) -> String {
    let mut slug = String::new();
    for c in title.to_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    slug.trim_matches('-').chars().take(40).collect()
}

/// check_vault.js's `^T-\d{4}$`.
fn is_ticket_id(value: &str) -> bool {
    value.len() == 6
        && value.starts_with("T-")
        && value[2..].bytes().all(|b| b.is_ascii_digit())
}

/// check_vault.js's `^\d{4}-\d{2}-\d{2}$`.
fn is_date(value: &str) -> bool {
    let b = value.as_bytes();
    b.len() == 10
        && b[4] == b'-'
        && b[7] == b'-'
        && [0, 1, 2, 3, 5, 6, 8, 9].iter().all(|&i| b[i].is_ascii_digit())
}

/// server.js's `^[A-Za-z0-9_][A-Za-z0-9_/-]*$`.
fn is_valid_tag(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '/' | '-'))
}

/// The title check of check_vault.js: `^("([^"\\]|\\.)*"|'[^']*')$`. This is a
/// quoted scalar that YAML accepts.
fn is_well_formed_quoted(value: &str) -> bool {
    let mut chars = value.chars();
    match chars.next() {
        Some('"') => loop {
            match chars.next() {
                None => return false,
                // A backslash escapes the next character, a quote included.
                Some('\\') => {
                    if chars.next().is_none() {
                        return false;
                    }
                }
                Some('"') => return chars.next().is_none(),
                Some(_) => {}
            }
        },
        Some('\'') => {
            let rest: Vec<char> = chars.collect();
            matches!(rest.split_last(), Some(('\'', body)) if !body.contains(&'\''))
        }
        _ => false,
    }
}

/// Same semantics as unquote() in server.js: a well-formed double-quoted
/// scalar is stripped and its `\x` escapes resolved; a well-formed
/// single-quoted scalar (no embedded `'`) is stripped as-is; anything else,
/// malformed quoting included, is returned unchanged.
fn unquote(value: &str) -> String {
    if !is_well_formed_quoted(value) {
        return value.to_string();
    }
    let body = &value[1..value.len() - 1];
    if !value.starts_with('"') {
        return body.to_string();
    }
    let mut out = String::with_capacity(body.len());
    let mut chars = body.chars();
    while let Some(c) = chars.next() {
        // is_well_formed_quoted guarantees a `\` is always followed by
        // another character, so `chars.next()` here is never None.
        out.push(if c == '\\' { chars.next().unwrap() } else { c });
    }
    out
}

fn unlink(value: &str) -> String {
    let v = unquote(value);
    v.strip_prefix("[[")
        .and_then(|v| v.strip_suffix("]]"))
        .unwrap_or(&v)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Read-only source of the fixture vault, checked into the repo. The tests
    /// never write to it.
    fn source() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/vault")
    }

    fn temp_vault(tag: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("tui-vault-test-{tag}"));
        let _ = fs::remove_dir_all(&root);
        for folder in ["tickets", "projects"] {
            let dir = root.join(folder);
            fs::create_dir_all(&dir).unwrap();
            for entry in fs::read_dir(source().join(folder)).unwrap() {
                let src = entry.unwrap().path();
                if src.extension().is_some_and(|e| e == "md") {
                    fs::copy(&src, dir.join(src.file_name().unwrap())).unwrap();
                }
            }
        }
        root
    }

    /// Find the one inserted line. Then report which of the other lines differ
    /// from `before`.
    /// ponytail: O(n^2) scan. It is sufficient for ticket files. Use a real
    /// diff only if these files grow to thousands of lines.
    fn diff(before: &str, after: &str) -> (Vec<String>, String) {
        let b: Vec<&str> = before.split('\n').collect();
        let a: Vec<&str> = after.split('\n').collect();
        assert_eq!(a.len(), b.len() + 1, "expected exactly one added line");
        let (_, best) = (0..a.len())
            .map(|i| {
                let mut rest = a.clone();
                rest.remove(i);
                (rest.iter().zip(&b).filter(|(x, y)| x != y).count(), i)
            })
            .min()
            .unwrap();
        let mut rest = a.clone();
        let inserted = rest.remove(best);
        let changed = rest
            .iter()
            .zip(&b)
            .filter(|(x, y)| x != y)
            .map(|(x, _)| x.to_string())
            .collect();
        (changed, inserted.to_string())
    }

    /// Remove the equal lines at the start and at the end. The result is the
    /// lines that went and the lines that came. This is the span that an edit
    /// touched. `diff` above needs an equal line count. This function accepts a
    /// different count, as tags and sections need.
    fn changed_region(before: &str, after: &str) -> (Vec<String>, Vec<String>) {
        let b: Vec<&str> = before.split('\n').collect();
        let a: Vec<&str> = after.split('\n').collect();
        let mut head = 0;
        while head < a.len() && head < b.len() && a[head] == b[head] {
            head += 1;
        }
        let mut tail = 0;
        while tail < a.len() - head
            && tail < b.len() - head
            && a[a.len() - 1 - tail] == b[b.len() - 1 - tail]
        {
            tail += 1;
        }
        let own = |lines: &[&str]| lines.iter().map(|l| l.to_string()).collect();
        (own(&b[head..b.len() - tail]), own(&a[head..a.len() - tail]))
    }

    #[test]
    fn every_vault_ticket_parses() {
        let root = temp_vault("parse");
        // No warnings: every fixture file is a well-formed ticket.
        let (tickets, warnings) = load_tickets(&root).expect("all tickets must parse");
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(!tickets.is_empty());
        for t in &tickets {
            assert!(t.id.starts_with("T-"), "bad id {}", t.id);
            assert!(!t.title.starts_with('"'), "title still quoted: {}", t.title);
            assert!(!t.project.starts_with("[["), "project still linked");
            assert!(STATUSES.contains(&t.status.as_str()));
        }
    }

    #[test]
    fn block_list_tags_and_closed_parse() {
        let root = temp_vault("tags");
        let (tickets, _) = load_tickets(&root).unwrap();
        let by_id = |id: &str| tickets.iter().find(|t| t.id == id).unwrap();
        assert_eq!(by_id("T-0001").tags, ["setup"]);
        assert_eq!(by_id("T-0001").closed, "");
        assert_eq!(by_id("T-0006").closed, "2026-08-09");
    }

    #[test]
    fn one_unparsable_ticket_does_not_block_the_others() {
        let root = temp_vault("broken-ticket");
        fs::write(root.join("tickets/junk.md"), "not a ticket at all").unwrap();

        let (tickets, warnings) = load_tickets(&root).expect("a bad file must not fail the load");

        assert!(!tickets.is_empty(), "the other tickets must still load");
        assert!(tickets.iter().all(|t| t.id.starts_with("T-")));
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("junk.md"), "{warnings:?}");
    }

    #[test]
    fn inline_empty_tags_parse_as_none() {
        let dir = std::env::temp_dir().join("tui-vault-test-empty-tags");
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("T-0000.md");
        fs::write(
            &path,
            "---\nid: T-0000\ntitle: \"x\"\nstatus: open\ntags: []\ncreated: 2026-08-09\n---\n",
        )
        .unwrap();

        let ticket = Ticket::parse(&path, &read(&path).unwrap()).unwrap();

        assert!(ticket.tags.is_empty());
    }

    #[test]
    fn status_change_touches_only_status_and_one_log_line() {
        let root = temp_vault("status");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        change_status(&path, "review", Some("PR up")).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (changed, inserted) = diff(&before, &after);
        assert_eq!(changed, vec!["status: review".to_string()]);
        assert!(inserted.ends_with(" \u{2014} PR up"), "{inserted}");
    }

    #[test]
    fn done_sets_closed_to_today() {
        let root = temp_vault("done");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        change_status(&path, "done", Some("shipped")).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (mut changed, inserted) = diff(&before, &after);
        changed.sort();
        assert_eq!(
            changed,
            vec![format!("closed: {}", today()), "status: done".to_string()]
        );
        assert!(inserted.ends_with(" \u{2014} done: shipped"), "{inserted}");
    }

    #[test]
    fn leaving_done_clears_closed_but_archiving_keeps_it() {
        let root = temp_vault("leave-done");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");

        change_status(&path, "done", None).unwrap();
        let done = Ticket::parse(&path, &fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(done.closed, today());

        change_status(&path, "open", None).unwrap();
        let reopened = Ticket::parse(&path, &fs::read_to_string(&path).unwrap()).unwrap();
        assert!(reopened.closed.is_empty(), "closed must clear when leaving done for open");
    }

    #[test]
    fn archiving_a_done_ticket_keeps_closed() {
        let root = temp_vault("archive-done");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");

        change_status(&path, "done", None).unwrap();
        let closed = Ticket::parse(&path, &fs::read_to_string(&path).unwrap()).unwrap().closed;

        change_status(&path, "archived", None).unwrap();
        let archived = Ticket::parse(&path, &fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(archived.closed, closed, "archiving a done ticket keeps closed");
    }

    #[test]
    fn appended_log_line_has_the_exact_format() {
        let root = temp_vault("format");
        let path = root.join("tickets/T-0002-unpin-templater.md");
        let before = fs::read_to_string(&path).unwrap();

        append_note(&path, "hello there").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (changed, inserted) = diff(&before, &after);
        assert!(changed.is_empty(), "a note must not change other lines");

        let body = inserted.strip_prefix("- ").expect("must start with `- `");
        let (stamp, text) = body.split_once(" \u{2014} ").expect("em dash separator");
        assert_eq!(text, "hello there");
        assert!(
            chrono::NaiveDateTime::parse_from_str(stamp, "%Y-%m-%d %H:%M").is_ok(),
            "bad timestamp {stamp}"
        );
    }

    #[test]
    fn invalid_status_is_rejected_and_file_untouched() {
        let root = temp_vault("invalid");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        assert!(change_status(&path, "in-progress", None).is_err());

        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn same_status_is_a_true_no_op() {
        let root = temp_vault("same-status");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        // The fixture status of T-0001 is "doing". The same status must not
        // change the file. It must not add a Log line.
        change_status(&path, "doing", Some("whatever")).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn crlf_line_endings_survive_a_write() {
        let root = temp_vault("crlf");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let crlf = fs::read_to_string(&path).unwrap().replace('\n', "\r\n");
        fs::write(&path, &crlf).unwrap();

        change_status(&path, "done", None).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(after.contains("\r\n"));
        assert!(
            !after.replace("\r\n", "").contains('\n'),
            "a bare LF leaked into a CRLF file"
        );
        let (changed, _) = diff(&crlf, &after);
        assert_eq!(changed.len(), 2, "only status and closed may change");
    }

    // --- editing ---

    const SAMPLE_NAME: &str = "T-0042-sample.md";

    /// A ticket without schema errors. The lint tests change copies of it.
    fn sample() -> String {
        [
            "---",
            "id: T-0042",
            "title: \"Sample ticket\"",
            "status: open",
            "priority: normal",
            "project: ",
            "repos: []",
            "tags: []",
            "created: 2026-08-09",
            "due: ",
            "closed: ",
            "branch: T-0042-sample",
            "---",
            "",
            "## Summary",
            "",
            "Sample.",
            "",
            "## Notes",
            "",
            "- ",
            "",
            "## Log",
            "",
            "- 2026-08-09 09:00 \u{2014} created",
            "",
        ]
        .join("\n")
    }

    #[test]
    fn quote_yaml_escapes_backslashes_and_quotes() {
        assert_eq!(quote_yaml("plain"), "\"plain\"");
        assert_eq!(quote_yaml("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(quote_yaml("c:\\dir"), "\"c:\\\\dir\"");
        // The output of quote_yaml must pass the title check of the lint.
        assert!(is_well_formed_quoted(&quote_yaml("a \" b \\ c")));
    }

    #[test]
    fn unquote_resolves_escapes_like_server_js() {
        assert_eq!(unquote("\"He said \\\"hi\\\"\""), "He said \"hi\"");
        assert_eq!(unquote("\"c:\\\\dir\""), "c:\\dir");
        // Single-quoted: stripped as-is, no escaping.
        assert_eq!(unquote("'plain'"), "plain");
        // Malformed quoting: left completely unchanged, not partially stripped.
        assert_eq!(unquote("\"unbalanced"), "\"unbalanced");
    }

    #[test]
    fn quote_yaml_and_unquote_round_trip() {
        for title in ["plain", "say \"hi\"", "c:\\dir", "a \" b \\ c", "He said \\\"hi\\\""] {
            assert_eq!(unquote(&quote_yaml(title)), title);
        }
    }

    #[test]
    fn set_field_title_escapes_do_not_grow_on_repeated_writes() {
        let root = temp_vault("field-title-escape-stable");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");

        set_field(&root, &path, "title", "He said \"hi\"").unwrap();
        let once = fs::read_to_string(&path).unwrap();

        // Parse the stored title back, then write it unchanged. If unquote
        // failed to resolve the escapes, quote_yaml would double the
        // backslashes on this second write.
        let ticket = Ticket::parse(&path, &once).unwrap();
        set_field(&root, &path, "title", &ticket.title).unwrap();
        let twice = fs::read_to_string(&path).unwrap();

        assert_eq!(once, twice, "backslashes must not grow on a round trip");
    }

    #[test]
    fn slugify_matches_the_template_rule() {
        assert_eq!(slugify("Bootstrap the Vault"), "bootstrap-the-vault");
        assert_eq!(slugify("  Fix: robustness!! "), "fix-robustness");
        assert_eq!(slugify("日本語"), "");
        assert_eq!(slugify("---"), "");
        assert_eq!(slugify(&"ab ".repeat(30)).len(), 40);
    }

    #[test]
    fn date_and_tag_validation_boundaries() {
        assert!(is_date("2026-08-09"));
        assert!(!is_date("2026-8-09"));
        assert!(!is_date("2026-08-09 "));
        assert!(!is_date("20260809xx"));
        assert!(is_valid_tag("setup"));
        assert!(is_valid_tag("_a/b-c1"));
        assert!(!is_valid_tag(""));
        assert!(!is_valid_tag("-lead"));
        assert!(!is_valid_tag("has space"));
        assert!(is_ticket_id("T-0042"));
        assert!(!is_ticket_id("T-42"));
    }

    #[test]
    fn lint_accepts_every_real_ticket() {
        let root = temp_vault("lint");
        for entry in fs::read_dir(root.join("tickets")).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let problems = lint_content(&name, &read(&path).unwrap());
            assert!(problems.is_empty(), "{name}: {problems:?}");
        }
        assert!(lint_content(SAMPLE_NAME, &sample()).is_empty());
    }

    #[test]
    fn lint_catches_every_single_file_breakage() {
        let swap = |from: &str, to: &str| sample().replace(from, to);
        let cases: Vec<(&str, String, &str)> = vec![
            (SAMPLE_NAME, sample().replacen("---\n", "", 1), "missing frontmatter"),
            (SAMPLE_NAME, swap("id: T-0042\n", ""), "missing id"),
            // A bad id also fails the branch prefix check. Remove the branch
            // line too. Thus each case has one error.
            (
                SAMPLE_NAME,
                swap("id: T-0042", "id: T-42").replace("branch: T-0042-sample", "branch: "),
                "bad id \"T-42\"",
            ),
            ("T-0043-sample.md", sample(), "filename does not start with T-0042"),
            (SAMPLE_NAME, swap("title: \"Sample ticket\"", "title: "), "title missing"),
            (
                SAMPLE_NAME,
                swap("title: \"Sample ticket\"", "title: \"unbalanced"),
                "malformed quoted title",
            ),
            (
                SAMPLE_NAME,
                swap("title: \"Sample ticket\"", "title: a: b"),
                "title with ':' must be quoted",
            ),
            (SAMPLE_NAME, swap("status: open", "status: wip"), "bad status \"wip\""),
            (SAMPLE_NAME, swap("priority: normal", "priority: soon"), "bad priority \"soon\""),
            (SAMPLE_NAME, swap("created: 2026-08-09\n", ""), "missing created"),
            (SAMPLE_NAME, swap("due: ", "due: 9/8/2026"), "bad due \"9/8/2026\""),
            (SAMPLE_NAME, swap("status: open", "status: done"), "status done but closed is empty"),
            (
                SAMPLE_NAME,
                swap("branch: T-0042-sample", "branch: feature/x"),
                "branch \"feature/x\" does not start with T-0042",
            ),
        ];
        for (name, text, expect) in cases {
            let problems = lint_content(name, &text);
            assert_eq!(problems.len(), 1, "expected only `{expect}`, got {problems:?}");
            assert!(problems[0].contains(expect), "want `{expect}`, got {problems:?}");
            assert!(problems[0].starts_with(name), "error must name the file: {problems:?}");
        }
    }

    #[test]
    fn a_lint_failure_leaves_the_file_untouched() {
        let root = temp_vault("lint-gate");
        let path = root.join("tickets").join(SAMPLE_NAME);
        // The file has an error before the write: status done with no closed
        // date. The module must refuse every write now, also a correct one.
        let broken = sample().replace("status: open", "status: done");
        fs::write(&path, &broken).unwrap();

        let error = append_note(&path, "a note").unwrap_err();

        assert!(error.contains("closed is empty"), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), broken);
    }

    #[test]
    fn set_content_lints_the_whole_file_and_no_ops_are_free() {
        let root = temp_vault("set-content");
        let path = root.join("tickets").join(SAMPLE_NAME);
        fs::write(&path, sample()).unwrap();
        let before = fs::read_to_string(&path).unwrap();

        let broken = sample().replace("status: open", "status: wip");
        let error = set_content(&path, &broken).unwrap_err();

        assert!(error.contains("bad status \"wip\""), "{error}");
        assert_eq!(fs::read_to_string(&path).unwrap(), before);

        set_content(&path, &before).unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn set_field_title_rewrites_exactly_one_line() {
        let root = temp_vault("field-title");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        set_field(&root, &path, "title", "  Say \"hi\"  ").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (gone, came) = changed_region(&before, &after);
        assert_eq!(gone, ["title: \"Bootstrap the vault\""]);
        assert_eq!(came, ["title: \"Say \\\"hi\\\"\""]);
    }

    #[test]
    fn set_field_clears_due_to_key_colon_space() {
        let root = temp_vault("field-due");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        set_field(&root, &path, "due", "").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (gone, came) = changed_region(&before, &after);
        assert_eq!(gone, ["due: 2026-08-14"]);
        // The form of an empty value in the vault: colon, space, nothing.
        assert_eq!(came, ["due: "]);
        assert!(set_field(&root, &path, "due", "9/8/2026").is_err());
    }

    #[test]
    fn set_field_project_wikilinks_known_projects_only() {
        let root = temp_vault("field-project");
        let path = root.join("tickets/T-0002-unpin-templater.md");
        let before = fs::read_to_string(&path).unwrap();

        assert!(set_field(&root, &path, "project", "no-such-project").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before, "untouched");

        set_field(&root, &path, "project", "vault-bootstrap").unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (_, came) = changed_region(&before, &after);
        assert_eq!(came, ["project: \"[[vault-bootstrap]]\""]);
    }

    #[test]
    fn list_projects_covers_flat_and_folder_note_projects() {
        // The fixture vault, read directly: it has a flat project
        // (vault-bootstrap.md), a folder-note project
        // (folder-note-demo/folder-note-demo.md), a theme note one level below
        // that folder, and a project folder with no matching note.
        let vault = source();

        assert_eq!(list_projects(&vault), ["folder-note-demo", "vault-bootstrap"]);
        assert!(project_exists(&vault, "folder-note-demo"));
        assert!(project_exists(&vault, "vault-bootstrap"));
        assert!(!project_exists(&vault, "theme-note"), "one level below a project folder");
        assert!(!project_exists(&vault, "no-note-folder"), "folder with no matching note");
        assert!(!project_exists(&vault, "../../../etc/passwd"));
    }

    #[test]
    fn set_field_refuses_schema_owned_fields() {
        let root = temp_vault("field-refuse");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        for field in ["status", "created", "closed", "branch", "repos", "id"] {
            assert!(set_field(&root, &path, field, "done").is_err(), "{field}");
        }
        assert!(set_field(&root, &path, "priority", "soon").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn set_tags_replaces_only_the_tags_block() {
        let root = temp_vault("tags-set");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        let tags = ["launch", "docs", "launch"].map(str::to_string);
        set_tags(&path, &tags).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        let (gone, came) = changed_region(&before, &after);
        assert_eq!(gone, ["  - setup"]);
        assert_eq!(came, ["  - launch", "  - docs"], "order kept, dupes dropped");

        set_tags(&path, &[]).unwrap();
        let (_, came) = changed_region(&after, &fs::read_to_string(&path).unwrap());
        assert_eq!(came, ["tags: []"]);
    }

    #[test]
    fn set_tags_rejects_malformed_tags() {
        let root = temp_vault("tags-bad");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let before = fs::read_to_string(&path).unwrap();

        assert!(set_tags(&path, &["has space".to_string()]).is_err());
        assert!(set_tags(&path, &["-leading".to_string()]).is_err());

        assert_eq!(fs::read_to_string(&path).unwrap(), before);
    }

    #[test]
    fn crlf_survives_tag_edits() {
        let root = temp_vault("crlf-edit");
        let path = root.join("tickets/T-0001-bootstrap-vault.md");
        let crlf = fs::read_to_string(&path).unwrap().replace('\n', "\r\n");
        fs::write(&path, &crlf).unwrap();

        set_tags(&path, &["docs".to_string(), "launch".to_string()]).unwrap();

        let after = fs::read_to_string(&path).unwrap();
        assert!(
            !after.replace("\r\n", "").contains('\n'),
            "a bare LF leaked into a CRLF file"
        );
        assert!(after.contains("tags:\r\n  - docs\r\n  - launch\r\n"));
    }

    #[test]
    fn next_ticket_id_is_one_past_the_highest() {
        let root = temp_vault("next-id");
        assert_eq!(next_ticket_id(&root).unwrap(), "T-0007");

        // A ticket without a frontmatter id still counts, by its file name.
        fs::write(
            root.join("tickets/T-0031-orphan.md"),
            sample().replace("id: T-0042\n", ""),
        )
        .unwrap();
        assert_eq!(next_ticket_id(&root).unwrap(), "T-0032");
    }

    #[test]
    fn create_ticket_writes_the_template_verbatim() {
        let root = temp_vault("create");
        let started = now_stamp();

        let path = create_ticket(&root, "  Try the TUI: round 2  ", "high", "2026-09-01", "vault-bootstrap")
            .unwrap();

        assert_eq!(path, root.join("tickets/T-0007-try-the-tui-round-2.md"));
        let text = fs::read_to_string(&path).unwrap();
        assert!(!text.contains('\r'), "a new ticket is always LF");

        let expected = |stamp: &str| {
            format!(
                "---\n\
                 id: T-0007\n\
                 title: \"Try the TUI: round 2\"\n\
                 status: open\n\
                 priority: high\n\
                 project: \"[[vault-bootstrap]]\"\n\
                 repos: []\n\
                 tags: []\n\
                 created: {}\n\
                 due: 2026-09-01\n\
                 closed: \n\
                 branch: T-0007-try-the-tui-round-2\n\
                 ---\n\
                 \n\
                 ## Summary\n\
                 \n\
                 Try the TUI: round 2\n\
                 \n\
                 ## Notes\n\
                 \n\
                 - \n\
                 \n\
                 ## Log\n\
                 \n\
                 - {} \u{2014} created\n",
                today(),
                stamp
            )
        };
        // The clock can change the minute between the two stamps. Thus both
        // minutes are correct.
        let stamps = [started, now_stamp()];
        assert!(
            stamps.iter().any(|s| text == expected(s)),
            "got:\n{text}\nwant:\n{}",
            expected(&stamps[1])
        );
        // The board can load this ticket like every other ticket.
        let ticket = Ticket::parse(&path, &text).unwrap();
        assert_eq!(ticket.title, "Try the TUI: round 2");
        assert_eq!(ticket.project, "vault-bootstrap");
    }

    #[test]
    fn create_ticket_rejects_bad_input_without_writing() {
        let root = temp_vault("create-bad");
        let before = fs::read_dir(root.join("tickets")).unwrap().count();

        assert!(create_ticket(&root, "   ", "normal", "", "").is_err());
        assert!(create_ticket(&root, "ok", "soon", "", "").is_err());
        assert!(create_ticket(&root, "ok", "normal", "9/8/2026", "").is_err());
        assert!(create_ticket(&root, "ok", "normal", "", "no-such-project").is_err());

        assert_eq!(fs::read_dir(root.join("tickets")).unwrap().count(), before);
    }
}
