//! Diff: two profiles, before and after. The headline figures, then the functions, lines or allocation sites that
//! moved (tab switches), each with its before, after and change: red ▲ for more, blue ▼ for less, the numbers in ink.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::widgets::{Col, MARGIN, cell, diverging_bar, header, layout, section, select_row, spans, text};
use crate::{
    app::{App, DiffSort, DiffTab, Focus, View},
    diff::{self, Pair},
    fmt,
    keys::Key,
    metric::{Metric, Unit},
    theme::Theme,
};

fn rows(app: &App) -> (Vec<Pair>, Unit, String) {
    let p = &app.profiles[app.shown];
    let metric = app.metric;
    let (mut rows, unit, what) = match app.diff.tab {
        DiffTab::Functions => (diff::functions(app, metric), metric.unit(), format!("own {}", metric.label(p))),
        DiffTab::Lines => (diff::lines(app, metric), metric.unit(), format!("own {}", metric.label(p))),
        DiffTab::Sites => {
            let by_count = metric == Metric::AllocCount;
            (diff::sites(app, metric), if by_count { Unit::Count } else { Unit::Bytes }, if by_count { "objects allocated".into() } else { "bytes allocated".into() })
        }
    };
    match app.diff.sort {
        DiffSort::Change => rows.sort_by(|a, b| b.delta().abs().total_cmp(&a.delta().abs()).then(a.label.cmp(&b.label))),
        DiffSort::Increase => rows.sort_by(|a, b| b.delta().total_cmp(&a.delta()).then(a.label.cmp(&b.label))),
        DiffSort::Decrease => rows.sort_by(|a, b| a.delta().total_cmp(&b.delta()).then(a.label.cmp(&b.label))),
        DiffSort::Name => rows.sort_by(|a, b| a.label.cmp(&b.label)),
    }
    (rows, unit, what)
}

/// The ▲ / ▼ mark of a change in a cost
fn mark(delta: f64, theme: &Theme) -> (&'static str, Style) {
    if delta > 0.0 {
        ("▲", Style::new().fg(theme.more).bg(theme.surface))
    } else if delta < 0.0 {
        ("▼", Style::new().fg(theme.less).bg(theme.surface))
    } else {
        (" ", theme.dim())
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    if app.profiles.len() < 2 {
        text(buf, area.x, area.y, "Diff needs two profiles: ree profile view <before> <after>", area.width, theme.dim());
        return;
    }
    let (figures, notes) = diff::headline(&app.profiles[0], &app.profiles[1]);
    let mut y = area.y;
    let (before, after) = (app.profiles[0].name(), app.profiles[1].name());
    let names = if before.eq_ignore_ascii_case("before") && after.eq_ignore_ascii_case("after") { String::new() } else { format!("{before} → {after}") };
    section(buf, area, y, "Before → after", &names, theme);
    y += 1;
    for note in &notes {
        text(buf, area.x, y, &format!("note: {note}"), area.width, theme.dim());
        y += 1;
    }
    // the headline, two columns of figures when the screen is wide
    let per_column = (figures.len() as u16).div_ceil(if area.width >= 150 { 2 } else { 1 });
    let column_width = if area.width >= 150 { area.width / 2 } else { area.width };
    for (index, figure) in figures.iter().enumerate() {
        let col = index as u16 / per_column;
        let row = index as u16 % per_column;
        let x = area.x + col * column_width;
        let delta = figure.after - figure.before;
        let (m, style) = if figure.cost { mark(delta, theme) } else { (" ", theme.dim()) };
        spans(buf, x, y + row, x + column_width - 1, &[
            (&format!("{:<34}", fmt::fit(&figure.name, 34)), theme.dim()),
            (&format!("{:>12} → {:<12}", fmt::short(figure.unit, figure.before), fmt::short(figure.unit, figure.after)), theme.text()),
            (m, style),
            (&format!(" {:>9}", fmt::change(figure.before, figure.after)), if figure.exact { theme.text() } else { theme.secondary() }),
        ]);
    }
    y += per_column + 1;
    // the tabs
    let tabs = [(DiffTab::Functions, "Functions"), (DiffTab::Lines, "Lines"), (DiffTab::Sites, "Allocation sites")];
    let mut x = area.x;
    for (tab, name) in tabs {
        let style = if app.diff.tab == tab { theme.selected() } else { theme.secondary() };
        let label = format!(" {name} ");
        x += text(buf, x, y, &label, area.right().saturating_sub(x), style) + 1;
    }
    if app.diff.tab == DiffTab::Lines {
        text(buf, x + 1, y, "where the source is the same", area.right().saturating_sub(x + 1), theme.dim());
    }
    y += 1;
    let (rows, unit, what) = rows(app);
    let sort = match app.diff.sort {
        DiffSort::Change => "the largest change",
        DiffSort::Increase => "what grew most",
        DiffSort::Decrease => "what shrank most",
        DiffSort::Name => "name",
    };
    section(buf, area, y, "Moved", &format!("{} · {what} · by {sort}", rows.len()), theme);
    y += 1;
    let show_calls = app.diff.tab == DiffTab::Functions;
    let mut cols = vec![Col::left("", 28), Col::right("before", 11), Col::right("after", 11), Col::left("", 1), Col::right("change", 11), Col::right("", 8)];
    if show_calls {
        cols.push(Col::right("calls", 19));
    }
    cols.push(Col::left("", 16));
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    header(buf, &cols, &xs, y, theme);
    y += 1;
    let height = area.bottom().saturating_sub(y) as usize;
    let mut list = app.diff.list;
    list.fit(rows.len(), height);
    let max = rows.iter().map(|r| r.delta().abs()).fold(0.0, f64::max);
    for (screen, row) in rows.iter().enumerate().skip(list.offset).take(height) {
        let row_y = y + (screen - list.offset) as u16;
        let delta = row.delta();
        let (m, mark_style) = mark(delta, theme);
        cell(buf, &cols[0], xs[0], row_y, &row.label, theme.text());
        cell(buf, &cols[1], xs[1], row_y, &fmt::short(unit, row.before), theme.secondary());
        cell(buf, &cols[2], xs[2], row_y, &fmt::short(unit, row.after), theme.text());
        cell(buf, &cols[3], xs[3], row_y, m, mark_style);
        cell(buf, &cols[4], xs[4], row_y, &fmt::signed(unit, delta), theme.text());
        cell(buf, &cols[5], xs[5], row_y, &fmt::change(row.before, row.after), theme.secondary());
        let mut c = 6;
        if show_calls {
            if let Some((before, after)) = row.calls {
                cell(buf, &cols[c], xs[c], row_y, &format!("{} → {}", fmt::compact(before as f64), fmt::compact(after as f64)), theme.secondary());
            }
            c += 1;
        }
        diverging_bar(buf, xs[c], row_y, cols[c].width, if max > 0.0 { delta / max } else { 0.0 }, theme);
        if screen == list.selected {
            select_row(buf, area, row_y, theme);
        }
    }
    if rows.is_empty() {
        text(buf, area.x, y, "nothing moved", area.width, theme.dim());
    }
    app.diff.list = list;
}

pub fn key(app: &mut App, key: Key) {
    if app.profiles.len() < 2 {
        return;
    }
    let (rows, _, _) = rows(app);
    if app.diff.list.key(key, rows.len(), app.page) {
        return;
    }
    match key {
        Key::Tab | Key::Right => {
            app.diff.tab = match app.diff.tab {
                DiffTab::Functions => DiffTab::Lines,
                DiffTab::Lines => DiffTab::Sites,
                DiffTab::Sites => DiffTab::Functions,
            };
            app.diff.list = Default::default();
        }
        Key::BackTab | Key::Left => {
            app.diff.tab = match app.diff.tab {
                DiffTab::Functions => DiffTab::Sites,
                DiffTab::Lines => DiffTab::Functions,
                DiffTab::Sites => DiffTab::Lines,
            };
            app.diff.list = Default::default();
        }
        Key::Char('s') => {
            app.diff.sort = match app.diff.sort {
                DiffSort::Change => DiffSort::Increase,
                DiffSort::Increase => DiffSort::Decrease,
                DiffSort::Decrease => DiffSort::Name,
                DiffSort::Name => DiffSort::Change,
            };
            app.diff.list.selected = 0;
        }
        Key::Enter => {
            // the after profile's function or line
            let Some(row) = rows.get(app.diff.list.selected) else { return };
            let Some(func) = row.func else { return };
            if app.shown != 1 {
                app.swap();
            }
            app.focus = Some(Focus { func, line: row.line });
            match (app.diff.tab, row.line) {
                (DiffTab::Functions, _) | (_, None) => {
                    app.calls.center = Some(func);
                    app.set_view(View::Calls);
                }
                (_, Some(line)) => {
                    app.source.module = None;
                    app.source.line = line;
                    app.set_view(View::Source);
                }
            }
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let (rows, unit, _) = rows(app);
    let row = rows.get(app.diff.list.selected)?;
    Some(format!("{}: {} -> {} ({})", row.label, fmt::short(unit, row.before), fmt::short(unit, row.after), fmt::change(row.before, row.after)))
}

pub fn find(app: &mut App, query: &str, forward: bool, from_start: bool) -> bool {
    let (rows, _, _) = rows(app);
    match super::next_match(rows.len(), app.diff.list.selected, forward, from_start, |i| rows[i].label.to_lowercase().contains(query)) {
        Some(found) => {
            app.diff.list.selected = found;
            true
        }
        None => false,
    }
}
