//! The screen: a header (the run, the metric and its total, the views' tabs), the view's body, the heat legend where
//! the view colours by heat, and a footer with the view's keys. `?` lays the help over it.

pub mod alloc;
pub mod caches;
pub mod calls;
pub mod compare;
pub mod diff;
pub mod flame;
pub mod heap;
pub mod issues;
pub mod overview;
pub mod plot;
pub mod source;
pub mod timeline;
pub mod widgets;

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style, terminal::Frame};

use crate::{
    app::{App, View},
    fmt,
    keys::Key,
    theme::{HEAT_LABELS, Theme},
};
use widgets::{fill, spans, text, text_right};

/// The smallest screen the views lay out in
pub const MIN_WIDTH: u16 = 60;
pub const MIN_HEIGHT: u16 = 16;

pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    draw_buffer(frame.buffer_mut(), area, app);
}

/// Draws the whole screen into a buffer
pub fn draw_buffer(buf: &mut Buffer, area: Rect, app: &mut App) {
    let theme = app.theme.clone();
    fill(buf, area, theme.text());
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        text(buf, area.x, area.y, &format!("ree profile view needs {MIN_WIDTH}x{MIN_HEIGHT} at least ({}x{} now)", area.width, area.height), area.width, theme.text());
        return;
    }
    header(buf, Rect { height: 2, ..area }, app, &theme);
    let legend = uses_heat(app);
    let footer_y = area.bottom() - 1;
    let body_bottom = if legend { footer_y - 1 } else { footer_y };
    let body = Rect { x: area.x + 1, y: area.y + 3, width: area.width - 2, height: body_bottom.saturating_sub(area.y + 3) };
    app.page = body.height.saturating_sub(4).max(1) as usize;
    match app.view {
        View::Overview => overview::draw(buf, body, app, &theme),
        View::Flame => flame::draw(buf, body, app, &theme),
        View::Calls => calls::draw(buf, body, app, &theme),
        View::Source => source::draw(buf, body, app, &theme),
        View::Alloc => alloc::draw(buf, body, app, &theme),
        View::Heap => heap::draw(buf, body, app, &theme),
        View::Timeline => timeline::draw(buf, body, app, &theme),
        View::Caches => caches::draw(buf, body, app, &theme),
        View::Issues => issues::draw(buf, body, app, &theme),
        View::Diff => diff::draw(buf, body, app, &theme),
        View::Compare => compare::draw(buf, body, app, &theme),
    }
    if legend {
        heat_legend(buf, Rect { x: area.x + 1, y: footer_y - 1, width: area.width - 2, height: 1 }, app, &theme);
    }
    footer(buf, Rect { y: footer_y, height: 1, ..area }, app, &theme);
    if app.help {
        help(buf, area, app, &theme);
    }
}

/// Whether the view colours by the heat scale (and so shows its legend)
fn uses_heat(app: &App) -> bool {
    matches!(app.view, View::Overview | View::Flame | View::Calls | View::Source | View::Alloc | View::Caches)
}

fn header(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    fill(buf, Rect { height: 1, ..area }, theme.bar_text());
    let right = area.right();
    let bold = theme.bar_text().add_modifier(ratatui_core::style::Modifier::BOLD);
    let mut x = spans(buf, area.x, area.y, right, &[(" ree profile ", bold)]);
    if app.has_profile() {
        let p = &app.profiles[app.shown];
        let run = &p.raw.run;
        let mut what = format!("{} {}", run.command, run.status);
        if let Some(harness) = p.harness() {
            what.push_str(&format!(" · {harness}"));
        }
        what.push_str(&format!(" · {} frames · {:.2} s", fmt::grouped(run.frames as f64), run.seconds));
        if app.profiles.len() == 2 {
            what = format!("{}: {what}", app.side_name(app.shown));
        }
        let metric = app.metric;
        let total = metric.total(p);
        let unit = metric.unit();
        let right_text = format!(" {} {} ", metric.label(p), fmt::value(unit, total));
        let right_width = right_text.chars().count() as u16;
        x = spans(buf, x, area.y, right.saturating_sub(right_width + 1), &[("│ ", theme.bar_dim()), (&what, theme.bar_text())]);
        let _ = x;
        text_right(buf, area.x, area.y, &right_text, area.width, bold);
    } else if let Some(compare) = &app.compare {
        spans(buf, x, area.y, right, &[("│ ", theme.bar_dim()), (&compare.title, theme.bar_text())]);
    }
    // the tabs
    let mut x = area.x + 1;
    let y = area.y + 1;
    for view in View::ALL {
        let label = format!(" {} {} ", view.key(), view.title());
        let width = label.chars().count() as u16;
        if x + width > right {
            break;
        }
        let style = if view == app.view {
            Style::new().fg(theme.ink).bg(theme.wash).add_modifier(ratatui_core::style::Modifier::BOLD)
        } else if app.available(view) {
            theme.secondary()
        } else {
            theme.dim()
        };
        text(buf, x, y, &label, width, style);
        x += width;
    }
    if app.scripts_only && x + 15 < right {
        text_right(buf, area.x, y, "scripts only ", area.width, theme.secondary());
    }
}

/// What a view's heat is a share of
fn heat_subject(app: &App) -> String {
    let p = &app.profiles[app.shown];
    match app.view {
        View::Alloc if app.metric == crate::metric::Metric::AllocCount => "allocations".into(),
        View::Alloc => "allocated bytes".into(),
        View::Caches => "cache misses".into(),
        _ => app.metric.label(p),
    }
}

/// The heat scale: the share of the run's total each colour stands for
fn heat_legend(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    if !app.has_profile() {
        return;
    }
    let mut parts: Vec<(String, Style)> = vec![(format!("share of {}  ", heat_subject(app)), theme.dim()), ("<0.1%  ".into(), theme.dim())];
    for (step, label) in HEAT_LABELS.iter().enumerate() {
        parts.push(("██".into(), Style::new().fg(theme.heat[step]).bg(theme.bar_mark)));
        parts.push((format!(" {label}  "), theme.secondary()));
    }
    let owned: Vec<(&str, Style)> = parts.iter().map(|(s, st)| (s.as_str(), *st)).collect();
    spans(buf, area.x, area.y, area.right(), &owned);
}

/// The keys a view takes, for the footer
fn view_keys(view: View) -> &'static str {
    match view {
        View::Overview => "↑↓ issue  enter open",
        View::Flame => "←→ ↑↓ move  enter zoom  esc out  s order",
        View::Calls => "↑↓ move  enter centre  esc back  s sort",
        View::Source => "↑↓ line  ←→ hot line  s by heat  enter calls",
        View::Alloc => "↑↓ site  s sort  enter source",
        View::Heap => "←→ snapshot  tab pane  ↑↓ row  enter source",
        View::Timeline => "←→ frame  pgup/pgdn ±10  enter zoom  esc out  [ ] series",
        View::Caches => "↑↓ site  s sort  enter source",
        View::Issues => "↑↓ issue  enter open",
        View::Diff => "tab table  ↑↓ row  s sort  enter open  x side",
        View::Compare => "↑↓ variant  ←→ size  enter open  D diff",
    }
}

fn footer(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    fill(buf, area, theme.bar_text());
    if app.searching {
        let line = format!(" /{}▏  enter find  esc cancel", app.query);
        text(buf, area.x, area.y, &line, area.width, theme.bar_text());
        return;
    }
    let keys = format!(" {}  │ m metric  f scripts  / find  y copy  t theme  ? help  q quit", view_keys(app.view));
    let status = if app.status.is_empty() { String::new() } else { format!(" {} ", app.status) };
    let status_width = (status.chars().count() as u16).min(area.width / 2);
    text(buf, area.x, area.y, &keys, area.width.saturating_sub(status_width + 1), theme.bar_dim());
    if status_width > 0 {
        text_right(buf, area.x, area.y, &status, area.width, theme.bar_text());
    }
}

fn help(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    let width = area.width.min(110).saturating_sub(4);
    let height = area.height.saturating_sub(4).min(40);
    let x = area.x + (area.width - width) / 2;
    let y = area.y + (area.height - height) / 2;
    let boxed = Rect { x, y, width, height };
    fill(buf, boxed, Style::new().fg(theme.ink).bg(theme.bar));
    let style = Style::new().fg(theme.ink).bg(theme.bar);
    let dim = Style::new().fg(theme.ink2).bg(theme.bar);
    let bold = style.add_modifier(ratatui_core::style::Modifier::BOLD);
    let mut lines: Vec<(String, Style)> = vec![
        ("Keys".into(), bold),
        ("1-9 views · d Diff (two profiles) · c Compare (a what-if folder)".into(), style),
        ("arrows or hjkl move · enter in · esc or backspace out".into(), style),
        ("m metric · s sort · f scripts only · / search, n N next · y copy · x before/after · t theme · q quit".into(), style),
        (format!("here: {}", view_keys(app.view)), style),
        (String::new(), style),
        ("Colour".into(), bold),
        ("orange heat: share of the run's total, brighter is more; grey: under 0.1%".into(), style),
        ("diff: red ▲ more, blue ▼ less · issues: ■ critical ▲ serious ● warning ○ info".into(), style),
        (String::new(), style),
    ];
    if app.has_profile() {
        let p = &app.profiles[app.shown];
        lines.push(("Metrics".into(), bold));
        for metric in app.metrics() {
            let mark = if metric == app.metric { "▸ " } else { "  " };
            for (index, chunk) in wrap(&format!("{mark}{}: {}", metric.label(p), metric.about()), width as usize - 6).into_iter().enumerate() {
                lines.push((if index == 0 { chunk } else { format!("    {chunk}") }, if metric == app.metric { style } else { dim }));
            }
        }
        if let Some(work) = &p.raw.work {
            if !work.fidelity.is_empty() {
                lines.push((String::new(), style));
                lines.push(("The work clock beside a client".into(), bold));
                for chunk in wrap(work.fidelity.trim_start_matches("fidelity: "), width as usize - 4) {
                    lines.push((chunk, dim));
                }
            }
        }
    }
    for (index, (line, st)) in lines.iter().enumerate() {
        let row = y + 1 + index as u16;
        if row >= boxed.bottom() - 1 {
            break;
        }
        text(buf, x + 2, row, line, width - 4, *st);
    }
    text_right(buf, x, boxed.bottom() - 1, " any key closes ", width - 1, dim);
}

/// Words wrapped to `width` columns
pub fn wrap(s: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut line = String::new();
    for word in s.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            out.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push(line);
    }
    out
}

/// A key the global keys did not take
pub fn key(app: &mut App, key: Key) {
    match app.view {
        View::Overview => overview::key(app, key),
        View::Flame => flame::key(app, key),
        View::Calls => calls::key(app, key),
        View::Source => source::key(app, key),
        View::Alloc => alloc::key(app, key),
        View::Heap => heap::key(app, key),
        View::Timeline => timeline::key(app, key),
        View::Caches => caches::key(app, key),
        View::Issues => issues::key(app, key),
        View::Diff => diff::key(app, key),
        View::Compare => compare::key(app, key),
    }
}

/// What `y` copies in the current view
pub fn copy_text(app: &App) -> Option<String> {
    match app.view {
        View::Flame => flame::copy(app),
        View::Calls => calls::copy(app),
        View::Source => source::copy(app),
        View::Alloc => alloc::copy(app),
        View::Caches => caches::copy(app),
        View::Issues | View::Overview => issues::copy(app),
        View::Diff => diff::copy(app),
        View::Compare => compare::copy(app),
        View::Heap => heap::copy(app),
        View::Timeline => timeline::copy(app),
    }
}

/// `n`, `N` and a search's enter
pub fn find(app: &mut App, forward: bool, from_start: bool) -> bool {
    let query = app.query.to_lowercase();
    match app.view {
        View::Flame => flame::find(app, &query, forward, from_start),
        View::Calls => calls::find(app, &query),
        View::Source => source::find(app, &query, forward, from_start),
        View::Alloc => alloc::find(app, &query, forward, from_start),
        View::Caches => caches::find(app, &query, forward, from_start),
        View::Issues | View::Overview => issues::find(app, &query, forward, from_start),
        View::Diff => diff::find(app, &query, forward, from_start),
        _ => false,
    }
}

/// The next index after (or before) `from` in `0..len` whose item matches, wrapping round
pub fn next_match(len: usize, from: usize, forward: bool, from_start: bool, matches: impl Fn(usize) -> bool) -> Option<usize> {
    if len == 0 {
        return None;
    }
    for step in 0..len {
        let index = if from_start {
            if forward { step } else { len - 1 - step }
        } else if forward {
            (from + 1 + step) % len
        } else {
            (from + len - 1 - step % len) % len
        };
        if matches(index) {
            return Some(index);
        }
    }
    None
}
