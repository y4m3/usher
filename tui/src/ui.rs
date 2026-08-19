//! Rendering for the kanban board. All colors come from the "Tracer" theme
//! below. This module always paints the background. Thus the background of the
//! terminal never shows through.

use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use std::path::Path;

use ratatui::widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use crate::vault::{STATUSES, Ticket};
use crate::{App, EDIT_FIELDS, Mode, option_label};

// --- Tracer theme (dark only) ---
const BG: Color = Color::Rgb(0x11, 0x1C, 0x18);
const COLUMN_BG: Color = Color::Rgb(0x15, 0x22, 0x1E);
const CARD_BG: Color = Color::Rgb(0x1A, 0x28, 0x23);
const BORDER: Color = Color::Rgb(0x27, 0x39, 0x32);
const TEXT: Color = Color::Rgb(0xB8, 0xC7, 0xBC);
const TEXT_DIM: Color = Color::Rgb(0x7E, 0x94, 0x88);
const SELECTION: Color = Color::Rgb(0x2B, 0x40, 0x38);
const ACCENT: Color = Color::Rgb(0x4F, 0xB8, 0x7C);
const HEADING: Color = Color::Rgb(0x6F, 0xE8, 0x9B);
const URGENT: Color = Color::Rgb(0xCA, 0x68, 0x60);
const HIGH: Color = Color::Rgb(0xD3, 0xAA, 0x64);
const NORMAL: Color = Color::Rgb(0x53, 0x9E, 0xC0);
const LOW: Color = TEXT_DIM;
const OVERDUE: Color = URGENT;

/// Below `columns * MIN_COLUMN_WIDTH` the board shows fewer columns. It does
/// not make the cards narrower. `h` and `l` move the window over the statuses
/// (four, or five with `show_archived`).
const MIN_COLUMN_WIDTH: u16 = 30;

fn priority_color(priority: &str) -> Color {
    match priority {
        "urgent" => URGENT,
        "high" => HIGH,
        "normal" => NORMAL,
        _ => LOW,
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let area = frame.area();
    frame.render_widget(Block::new().style(Style::new().bg(BG)), area);

    let [board, footer] = Layout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let statuses = app.board();
    let (start, count) = column_window(board.width, app.column, statuses.len());
    let rects = Layout::horizontal(vec![Constraint::Ratio(1, count as u32); count]).split(board);
    for (slot, rect) in rects.iter().enumerate() {
        let index = start + slot;
        let hidden = (
            slot == 0 && start > 0,
            slot + 1 == count && index + 1 < statuses.len(),
        );
        draw_column(frame, app, index, *rect, hidden);
    }
    draw_footer(frame, app, footer);

    // The detail view stays below the popup that an edit key opened.
    if let Some((path, raw, scroll)) = &app.detail {
        draw_detail(frame, area, app, path, raw, *scroll);
    }

    match &app.mode {
        Mode::Browse => {}
        Mode::Status(index) => draw_list_popup(frame, area, " move to ", &labels(&STATUSES), *index),
        Mode::ProjectFilter(index) => {
            draw_list_popup(frame, area, " project ", &filter_labels(app, false), *index)
        }
        Mode::TagFilter(index) => {
            draw_list_popup(frame, area, " tag ", &filter_labels(app, true), *index)
        }
        Mode::Note(buffer) => draw_input_popup(
            frame,
            area,
            " log note (enter to append, esc to cancel) ",
            buffer,
        ),
        Mode::Search(buffer) => draw_input_popup(
            frame,
            area,
            " search id/title (enter to apply, esc to clear) ",
            buffer,
        ),
        Mode::Create(buffer) => draw_input_popup(
            frame,
            area,
            " new ticket title (enter to create, esc to cancel) ",
            buffer,
        ),
        Mode::EditField(index) => {
            draw_list_popup(frame, area, " edit ", &labels(&EDIT_FIELDS), *index)
        }
        Mode::EditText { field, buffer } => draw_input_popup(
            frame,
            area,
            &format!(" {field} (enter to save, esc to cancel) "),
            buffer,
        ),
        Mode::EditPick { field, index } => {
            draw_list_popup(frame, area, &format!(" {field} "), &app.pick_options(field), *index)
        }
    }
}

fn labels(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

fn filter_labels(app: &App, tag: bool) -> Vec<String> {
    app.filter_options(tag)
        .iter()
        .map(|o| option_label(o).to_string())
        .collect()
}

/// The columns that fit: the first column to show and the number of columns.
/// The focused column stays visible, near the center. `total` is the number of
/// columns the board has right now (four, or five with `show_archived`).
fn column_window(width: u16, focus: usize, total: usize) -> (usize, usize) {
    let count = ((width / MIN_COLUMN_WIDTH) as usize).clamp(1, total);
    let start = focus.saturating_sub(count / 2).min(total - count);
    (start, count)
}

/// `hidden` tells if columns are off the screen to the left and to the right.
/// The first and the last visible title show an arrow for these columns.
fn draw_column(frame: &mut Frame, app: &App, index: usize, area: Rect, hidden: (bool, bool)) {
    let focused = app.column == index;
    let tickets = &app.columns[index];
    let status = app.board()[index];
    let block = Block::bordered()
        .title(Line::from(vec![
            Span::styled(
                if hidden.0 { " ◂" } else { "" },
                Style::new().fg(TEXT_DIM),
            ),
            Span::styled(
                format!(" {status} "),
                Style::new()
                    .fg(if focused { HEADING } else { TEXT })
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                if status == "done" && app.done_total > tickets.len() {
                    format!("{}/{} ", tickets.len(), app.done_total)
                } else {
                    format!("{} ", tickets.len())
                },
                Style::new().fg(TEXT_DIM),
            ),
            // The window state of the done column, like the web switch. The
            // archived column has no window: its count is enough, and its
            // presence at all already says show_archived is on.
            Span::styled(
                match status {
                    "done" if app.done_all => "all ",
                    "done" => "recent ",
                    _ => "",
                },
                Style::new().fg(ACCENT),
            ),
            Span::styled(if hidden.1 { "▸ " } else { "" }, Style::new().fg(TEXT_DIM)),
        ]))
        .border_style(Style::new().fg(if focused { ACCENT } else { BORDER }))
        .style(Style::new().bg(COLUMN_BG));

    let items: Vec<ListItem> = tickets.iter().map(card).collect();
    let list = List::new(items)
        .block(block)
        .style(Style::new().bg(COLUMN_BG).fg(TEXT))
        .highlight_style(Style::new().bg(SELECTION));

    let mut state = ListState::default();
    if !tickets.is_empty() {
        state.select(Some(app.selected[index].min(tickets.len() - 1)));
    }
    frame.render_stateful_widget(list, area, &mut state);
}

/// One card: id and title, then the due date, then one empty row.
fn card(ticket: &Ticket) -> ListItem<'static> {
    let card_style = Style::new().bg(CARD_BG);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} ", ticket.id),
            card_style
                .fg(priority_color(&ticket.priority))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(ticket.title.clone(), card_style.fg(TEXT)),
    ])
    .style(card_style)];

    let mut meta = Vec::new();
    if !ticket.project.is_empty() {
        meta.push(Span::styled(
            format!("{} ", ticket.project),
            card_style.fg(TEXT_DIM),
        ));
    }
    if !ticket.due.is_empty() {
        meta.push(Span::styled(
            format!("due {}", ticket.due),
            card_style.fg(if ticket.is_overdue() { OVERDUE } else { TEXT_DIM }),
        ));
    }
    if !ticket.tags.is_empty() {
        let tags: Vec<String> = ticket.tags.iter().map(|t| format!("#{t}")).collect();
        meta.push(Span::styled(
            format!(" {}", tags.join(" ")),
            card_style.fg(TEXT_DIM),
        ));
    }
    if !meta.is_empty() {
        lines.push(Line::from(meta).style(card_style));
    }
    lines.push(Line::from(""));
    ListItem::new(lines)
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let line = match &app.message {
        Some(message) => Line::from(Span::styled(
            format!(" {message}"),
            Style::new().bg(BG).fg(HEADING),
        )),
        // Show the active filters first. On a narrow terminal, Paragraph cuts
        // the end of the line and does not wrap it. Thus the key hints go last.
        None => {
            let mut spans = Vec::new();
            if !app.search.is_empty() {
                spans.push(format!(" filter: {}", app.search));
            }
            if let Some(project) = &app.project_filter {
                spans.push(format!(" proj: {}", option_label(&Some(project.clone()))));
            }
            if let Some(tag) = &app.tag_filter {
                spans.push(format!(" #{tag}"));
            }
            Line::from(vec![
                Span::styled(spans.concat(), Style::new().bg(BG).fg(HEADING)),
                Span::styled(
                    " h/l j/k move  n new  e edit  s status  m note  / search  p proj  t tag  . done  a archived  enter open  r  q",
                    Style::new().bg(BG).fg(TEXT_DIM),
                ),
            ])
        }
    };
    frame.render_widget(Paragraph::new(line).style(Style::new().bg(BG)), area);
}

fn draw_list_popup(frame: &mut Frame, area: Rect, title: &str, entries: &[String], index: usize) {
    let popup = centered(area, 32, entries.len() as u16 + 2);
    frame.render_widget(Clear, popup);
    let items: Vec<ListItem> = entries
        .iter()
        .map(|s| ListItem::new(Line::from(format!("  {s}"))))
        .collect();
    let list = List::new(items)
        .block(popup_block(title))
        .style(Style::new().bg(CARD_BG).fg(TEXT))
        .highlight_style(Style::new().bg(SELECTION).fg(HEADING));
    let mut state = ListState::default();
    state.select(Some(index));
    frame.render_stateful_widget(list, popup, &mut state);
}

fn draw_input_popup(frame: &mut Frame, area: Rect, title: &str, buffer: &str) {
    let popup = centered(area, area.width.saturating_sub(20).max(30), 3);
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(buffer, Style::new().fg(TEXT))))
            .block(popup_block(title))
            .style(Style::new().bg(CARD_BG)),
        popup,
    );
    frame.set_cursor_position(Position::new(
        // Add 1 for the border. The limit keeps the cursor visible in a long
        // note.
        popup.x + 1 + (buffer.chars().count() as u16).min(popup.width.saturating_sub(3)),
        popup.y + 1,
    ));
}

/// The detail view: the fields of the ticket, then its prose sections. The board
/// can have no data for a path, for example after a delete. Then this function
/// shows the raw markdown.
fn draw_detail(frame: &mut Frame, area: Rect, app: &App, path: &Path, raw: &str, scroll: u16) {
    let ticket = app.tickets.iter().find(|t| t.path == *path);
    let id = ticket.map_or("detail", |t| t.id.as_str());
    let lines = match ticket {
        Some(ticket) => detail_lines(ticket, raw),
        None => raw.lines().map(|l| Line::from(l.to_string())).collect(),
    };

    let popup = centered(area, area.width.saturating_sub(8), area.height.saturating_sub(4));
    frame.render_widget(Clear, popup);
    frame.render_widget(
        Paragraph::new(lines)
            .block(popup_block(&format!(
                " {id} (enter edit, s status, m note, j/k scroll, esc close) "
            )))
            .style(Style::new().bg(CARD_BG).fg(TEXT))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        popup,
    );
}

fn detail_lines(ticket: &Ticket, raw: &str) -> Vec<Line<'static>> {
    let dim = Style::new().fg(TEXT_DIM);
    let mut lines = vec![Line::from(vec![
        Span::styled(
            format!("{} ", ticket.id),
            Style::new()
                .fg(priority_color(&ticket.priority))
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            ticket.title.clone(),
            Style::new().fg(TEXT).add_modifier(Modifier::BOLD),
        ),
    ])];

    let tags: Vec<String> = ticket.tags.iter().map(|t| format!("#{t}")).collect();
    let fields = [
        ("status", ticket.status.clone(), Style::new().fg(TEXT)),
        (
            "priority",
            ticket.priority.clone(),
            Style::new().fg(priority_color(&ticket.priority)),
        ),
        (
            "due",
            ticket.due.clone(),
            Style::new().fg(if ticket.is_overdue() { OVERDUE } else { TEXT }),
        ),
        ("project", ticket.project.clone(), dim),
        ("tags", tags.join(" "), dim),
    ];
    for (label, value, style) in fields {
        if !value.is_empty() {
            lines.push(Line::from(vec![
                Span::styled(format!("{label:<9}"), dim),
                Span::styled(value, style),
            ]));
        }
    }

    // An empty line before each heading also separates the sections.
    for header in ["Summary", "Notes", "Log"] {
        let Ok(body) = crate::vault::get_section(raw, header) else {
            continue;
        };
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            header.to_string(),
            Style::new().fg(HEADING).add_modifier(Modifier::BOLD),
        )));
        lines.extend(body.lines().map(|l| Line::from(l.to_string())));
    }
    lines
}

fn popup_block(title: &str) -> Block<'_> {
    Block::bordered()
        .title(Span::styled(
            title.to_string(),
            Style::new().fg(HEADING).add_modifier(Modifier::BOLD),
        ))
        .border_style(Style::new().fg(ACCENT))
        .style(Style::new().bg(CARD_BG))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_column_window_slides_to_keep_the_focus_visible() {
        assert_eq!(column_window(120, 0, 4), (0, 4));
        assert_eq!(column_window(120, 3, 4), (0, 4));
        assert_eq!(column_window(100, 0, 4), (0, 3));
        assert_eq!(column_window(100, 3, 4), (1, 3));
        assert_eq!(column_window(50, 2, 4), (2, 1));
        assert_eq!(column_window(0, 0, 4), (0, 1), "never zero columns");
    }

    #[test]
    fn the_column_window_grows_with_a_fifth_archived_column() {
        assert_eq!(column_window(150, 0, 5), (0, 5));
        assert_eq!(column_window(150, 4, 5), (0, 5));
        assert_eq!(column_window(100, 4, 5), (2, 3));
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}
