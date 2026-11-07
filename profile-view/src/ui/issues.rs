//! Issues: the problems the measured patterns show, most severe first, each with its numbers, what would remove it
//! and a link (enter) to the view and line that show it; what the profile's kinds could not check, at the bottom.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::{
    widgets::{MARGIN, section, select_row, spans, text},
    wrap,
};
use crate::{
    app::App,
    issues::{Severity, Target},
    keys::Key,
    theme::Theme,
};

/// A severity's colour, for its icon (the word beside it says the same)
pub fn severity_style(severity: Severity, theme: &Theme) -> Style {
    let color = match severity {
        Severity::Critical => theme.critical,
        Severity::Serious => theme.serious,
        Severity::Warning => theme.warning,
        Severity::Info => theme.muted,
    };
    Style::new().fg(color).bg(theme.surface)
}

/// Where enter goes, in words
pub fn target_text(app: &App, target: &Target) -> String {
    let p = &app.profiles[app.shown];
    match target {
        Target::None => String::new(),
        Target::Func(func) => format!("Calls: {}", p.funcs[*func as usize].label()),
        Target::Line(func, line) => format!("Source: {}:{line}", p.funcs[*func as usize].short),
        Target::Frame(row) => format!("Timeline: frame {}", p.raw.timeline.as_ref().and_then(|t| t.value(*row, "frame")).unwrap_or(*row as f64)),
        Target::Site(func, line) => format!("Allocations: {}:{line}", p.funcs[*func as usize].short),
        Target::Cache(_) => "Caches: the call site".into(),
        Target::Snapshot(index) => format!("Heap: snapshot {}", p.raw.heap.as_ref().and_then(|h| h.snapshots.get(*index)).map_or(String::new(), |s| s.label.clone())),
        Target::Variant(name) => format!("Compare: {name}"),
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let issues = app.issues();
    let (found, unchecked) = (&issues.0, &issues.1);
    let mut list = app.issues_list;
    let detail_height = 9u16.min(area.height / 2);
    let unchecked_height = if unchecked.is_empty() { 0 } else { (unchecked.len() as u16 + 1).min(5) };
    section(buf, area, area.y, "Issues", &found.len().to_string(), theme);
    let height = area.height.saturating_sub(1 + detail_height + unchecked_height) as usize;
    list.fit(found.len(), height);
    for (index, issue) in found.iter().enumerate().skip(list.offset).take(height) {
        let y = area.y + 1 + (index - list.offset) as u16;
        let x = spans(buf, area.x + MARGIN, y, area.right(), &[(&format!("{} ", issue.severity.icon()), severity_style(issue.severity, theme)), (&format!("{:<9}", issue.severity.word()), theme.dim()), (&format!("{:<18}", issue.kind), theme.dim())]);
        text(buf, x, y, &issue.title, area.right().saturating_sub(x), theme.text());
        if index == list.selected {
            select_row(buf, area, y, theme);
        }
    }
    if found.is_empty() {
        text(buf, area.x + MARGIN, area.y + 1, "none found", area.width, theme.dim());
    }
    super::widgets::scroll_note(buf, area, area.y, list.offset, height, found.len(), theme);
    app.issues_list = list;
    // the selected issue
    let y = area.bottom() - detail_height - unchecked_height;
    if let Some(issue) = found.get(list.selected) {
        let x = spans(buf, area.x, y, area.right(), &[(&format!("{} {} ", issue.severity.icon(), issue.severity.word()), severity_style(issue.severity, theme))]);
        section(buf, Rect { x, width: area.right() - x, ..area }, y, &issue.title, "", theme);
        let mut row = y + 1;
        for line in &issue.evidence {
            for chunk in wrap(line, area.width as usize - 2) {
                if row >= y + detail_height - 1 {
                    break;
                }
                text(buf, area.x + 2, row, &chunk, area.width - 2, theme.text());
                row += 1;
            }
        }
        if !issue.advice.is_empty() && row < y + detail_height - 1 {
            text(buf, area.x + 2, row, &format!("→ {}", issue.advice), area.width - 2, theme.secondary());
            row += 1;
        }
        let target = target_text(app, &issue.target);
        if !target.is_empty() && row < y + detail_height {
            text(buf, area.x + 2, row, &format!("enter → {target}"), area.width - 2, theme.dim());
        }
    }
    if unchecked_height > 0 {
        let y = area.bottom() - unchecked_height;
        section(buf, area, y, "Not checked", "", theme);
        for (i, note) in unchecked.iter().take(unchecked_height as usize - 1).enumerate() {
            text(buf, area.x + 2, y + 1 + i as u16, note, area.width - 2, theme.dim());
        }
    }
}

pub fn key(app: &mut App, key: Key) {
    let issues = app.issues();
    if app.issues_list.key(key, issues.0.len(), app.page) {
        return;
    }
    if key == Key::Enter {
        if let Some(issue) = issues.0.get(app.issues_list.selected) {
            let target = issue.target.clone();
            app.follow(&target);
        }
    }
}

pub fn copy(app: &App) -> Option<String> {
    let issues = app.issues();
    let list = if app.view == crate::app::View::Overview { app.overview } else { app.issues_list };
    let issue = issues.0.get(list.selected)?;
    let mut text = format!("{} {}: {}", issue.severity.icon(), issue.severity.word(), issue.title);
    for line in &issue.evidence {
        text.push_str(&format!("\n  {line}"));
    }
    Some(text)
}

pub fn find(app: &mut App, query: &str, forward: bool, from_start: bool) -> bool {
    let issues = app.issues();
    let from = if app.view == crate::app::View::Overview { app.overview.selected } else { app.issues_list.selected };
    match super::next_match(issues.0.len(), from, forward, from_start, |i| issues.0[i].title.to_lowercase().contains(query)) {
        Some(found) => {
            if app.view == crate::app::View::Overview {
                app.overview.selected = found;
            } else {
                app.issues_list.selected = found;
            }
            true
        }
        None => false,
    }
}
