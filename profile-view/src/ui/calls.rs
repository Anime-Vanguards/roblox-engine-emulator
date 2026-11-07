//! Calls: a function between its callers (above) and its callees (below), with what each cost under it in the metric
//! and how often it called; its hottest lines at the bottom. Enter re-centres on the selected caller or callee, esc
//! goes back.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::{
    overview::{hot, ranked},
    widgets::{Col, MARGIN, bar, cell, header, hline, layout, section, select_row, spans, text},
};
use crate::{
    app::{App, Focus},
    fmt,
    keys::Key,
    metric::Metric,
    theme::Theme,
    tree::{EdgeStat, Frame},
};

#[derive(Clone, Copy, Debug)]
enum Row {
    Caller(Frame, EdgeStat),
    Callee(u32, EdgeStat),
}

/// The function in the middle: the one the views follow (picked here or in another view), else the hottest
pub fn center(app: &App) -> Option<u32> {
    app.focus.map(|f| f.func).or(app.calls.center).or_else(|| hot(app).first().map(|r| r.func))
}

fn rows(app: &App, func: u32) -> (Vec<Row>, usize) {
    let tree = app.tree();
    let mut callers: Vec<Row> = tree.callers(func).into_iter().map(|(frame, stat)| Row::Caller(frame, stat)).collect();
    let mut callees: Vec<Row> = tree.callees(func).into_iter().map(|(callee, stat)| Row::Callee(callee, stat)).collect();
    if app.calls.by_calls {
        let calls = |row: &Row| match row {
            Row::Caller(_, stat) | Row::Callee(_, stat) => stat.calls,
        };
        callers.sort_by_key(|row| std::cmp::Reverse(calls(row)));
        callees.sort_by_key(|row| std::cmp::Reverse(calls(row)));
    }
    let split = callers.len();
    callers.extend(callees);
    (callers, split)
}

fn row_name(app: &App, row: &Row) -> String {
    let p = &app.profiles[app.shown];
    match row {
        Row::Caller(Frame::Func(func), _) | Row::Callee(func, _) => p.funcs[*func as usize].label(),
        Row::Caller(Frame::Phase(phase), _) => format!("(the engine, in {})", p.phase_name(*phase)),
        Row::Caller(Frame::Root, _) => "(the engine)".into(),
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let Some(func) = center(app) else {
        text(buf, area.x, area.y, "no functions", area.width, theme.dim());
        return;
    };
    let tree = app.tree();
    let (rows, split) = rows(app, func);
    let mut list = app.calls.list;
    let p = &app.profiles[app.shown];
    let metric = tree.metric;
    let unit = metric.unit();
    let run_total = tree.total();
    let stat = tree.funcs.get(&func).copied().unwrap_or_default();
    let f = &p.funcs[func as usize];
    let mut lines: Vec<(i64, f64, &crate::model::LineFig)> = p.lines_of(func as usize).map(|(line, fig)| (line, app.metric.line_self(fig), fig)).filter(|(_, v, _)| *v > 0.0).collect();
    lines.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    // rows for each part: what it needs, the lines taking what is left
    let (callers_len, callees_len) = (split, rows.len() - split);
    let fixed = 2 + 3 + 1 + if lines.is_empty() { 0 } else { 1 };
    let room = area.height.saturating_sub(fixed) as usize;
    let callers_height = callers_len.max(1).min((room / 3).max(1));
    let callees_height = callees_len.max(1).min((room.saturating_sub(callers_height) / 2).max(1));
    let lines_height = room.saturating_sub(callers_height + callees_height).min(lines.len());
    list.fit(rows.len(), rows.len().max(1));
    // each half scrolls on its own
    let sel = list.selected;
    let callers_offset = if sel < split { sel.saturating_sub(callers_height.saturating_sub(1)) } else { 0 };
    let callees_offset = if sel >= split { (sel - split).saturating_sub(callees_height.saturating_sub(1)) } else { 0 };
    let mut cols = vec![Col::left("", 24), Col::right("calls", 10), Col::right(&metric.label(p), 12), Col::right("share", 7), Col::left("", 14)];
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    let mut y = area.y;
    section(buf, area, y, "Callers", &callers_len.to_string(), theme);
    y += 1;
    header(buf, &cols, &xs, y, theme);
    y += 1;
    let draw_row = |buf: &mut Buffer, row: &Row, y: u16, selected: bool, whole: f64| {
        let stat = match row {
            Row::Caller(_, stat) | Row::Callee(_, stat) => stat,
        };
        let share = if run_total > 0.0 { stat.total / run_total } else { 0.0 };
        let style = theme.row(share);
        cell(buf, &cols[0], xs[0], y, &row_name(app, row), style);
        cell(buf, &cols[1], xs[1], y, &if tree.has_calls { fmt::grouped(stat.calls as f64) } else { "-".into() }, style);
        cell(buf, &cols[2], xs[2], y, &fmt::short(unit, stat.total), style);
        cell(buf, &cols[3], xs[3], y, &fmt::share(stat.total, whole), style);
        bar(buf, xs[4], y, cols[4].width, if whole > 0.0 { stat.total / whole } else { 0.0 }, theme.heat_of(share), theme);
        if selected {
            select_row(buf, area, y, theme);
        }
    };
    for (index, row) in rows[..split].iter().enumerate().skip(callers_offset).take(callers_height) {
        draw_row(buf, row, y + (index - callers_offset) as u16, index == list.selected, stat.total);
    }
    if split == 0 {
        text(buf, xs[0], y, "none", area.width, theme.dim());
    }
    y += callers_height as u16;
    // the function itself
    hline(buf, area.x, y, area.width, Style::new().fg(theme.grid).bg(theme.surface));
    let share = if run_total > 0.0 { stat.own / run_total } else { 0.0 };
    let calls = if tree.has_calls && stat.calls > 0 { format!(" · {} calls, {} each", fmt::grouped(stat.calls as f64), fmt::short(unit, stat.total / stat.calls as f64)) } else { String::new() };
    let x = spans(buf, area.x, y + 1, area.right(), &[("● ", Style::new().fg(theme.heat_of(share)).bg(theme.surface)), (&f.label(), theme.bold())]);
    text(buf, x + 2, y + 1, &format!("own {} ({}) · total {} ({}){calls}", fmt::value(unit, stat.own), fmt::share(stat.own, run_total), fmt::value(unit, stat.total), fmt::share(stat.total, run_total)), area.right().saturating_sub(x + 2), theme.text());
    hline(buf, area.x, y + 2, area.width, Style::new().fg(theme.grid).bg(theme.surface));
    y += 3;
    section(buf, area, y, "Callees", &callees_len.to_string(), theme);
    y += 1;
    for (index, row) in rows[split..].iter().enumerate().skip(callees_offset).take(callees_height) {
        draw_row(buf, row, y + (index - callees_offset) as u16, split + index == list.selected, stat.total);
    }
    if callees_len == 0 {
        text(buf, xs[0], y, "none", area.width, theme.dim());
    }
    y += callees_height as u16;
    if lines_height > 0 && y < area.bottom() {
        section(buf, area, y, "Hottest lines", &format!("own {}", app.metric.label(p)), theme);
        y += 1;
        let line_total = Metric::total(app.metric, p);
        let max = lines.first().map_or(0.0, |l| l.1);
        for (line, value, fig) in lines.iter().take(lines_height) {
            if y >= area.bottom() {
                break;
            }
            let share = if line_total > 0.0 { value / line_total } else { 0.0 };
            let mut ops: Vec<(&String, &u64)> = fig.ops.iter().collect();
            ops.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let ops: Vec<String> = ops.iter().take(3).map(|(op, n)| format!("{op} {}", fmt::compact(**n as f64))).collect();
            let x = area.x + MARGIN;
            text(buf, x, y, &format!("{}:{line}", f.short), 16, theme.row(share));
            text(buf, x + 17, y, &fmt::right(&fmt::short(app.metric.unit(), *value), 10), 10, theme.row(share));
            text(buf, x + 28, y, &fmt::right(&fmt::share(*value, line_total), 6), 6, theme.row(share));
            bar(buf, x + 36, y, 14, if max > 0.0 { value / max } else { 0.0 }, theme.heat_of(share), theme);
            text(buf, x + 52, y, &ops.join(" · "), area.width.saturating_sub(52 + MARGIN), theme.dim());
            y += 1;
        }
    }
    app.calls.list = list;
}

pub fn key(app: &mut App, key: Key) {
    let Some(func) = center(app) else { return };
    let (rows, _) = rows(app, func);
    if app.calls.list.key(key, rows.len(), app.page) {
        return;
    }
    match key {
        Key::Enter | Key::Right => {
            let target = match rows.get(app.calls.list.selected) {
                Some(Row::Caller(Frame::Func(f), _)) | Some(Row::Callee(f, _)) => Some(*f),
                _ => None,
            };
            if let Some(target) = target {
                app.calls.history.push(func);
                app.calls.center = Some(target);
                app.calls.list.selected = 0;
                app.focus = Some(Focus { func: target, line: None });
            }
        }
        Key::Esc | Key::Backspace | Key::Left => {
            if let Some(previous) = app.calls.history.pop() {
                app.calls.center = Some(previous);
                app.calls.list.selected = 0;
                app.focus = Some(Focus { func: previous, line: None });
            }
        }
        Key::Char('s') => {
            app.calls.by_calls = !app.calls.by_calls;
            app.status = if app.calls.by_calls { "by calls".into() } else { "by cost".into() };
        }
        _ => {}
    }
}

/// `path:line` when the profile names the module's file, else `Module:line`
pub fn place_text(app: &App, func: u32, line: Option<i64>) -> Option<String> {
    let p = &app.profiles[app.shown];
    let f = p.funcs.get(func as usize)?;
    if f.module.is_empty() {
        return Some(f.name.clone());
    }
    let line = line.unwrap_or(f.line);
    Some(match p.module_path(&f.module) {
        Some(path) => format!("{path}:{line}"),
        None => format!("{}:{line}", f.module),
    })
}

pub fn copy(app: &App) -> Option<String> {
    let func = center(app)?;
    let (rows, _) = rows(app, func);
    match rows.get(app.calls.list.selected) {
        Some(Row::Caller(Frame::Func(f), _)) | Some(Row::Callee(f, _)) => place_text(app, *f, None),
        _ => place_text(app, func, None),
    }
}

pub fn find(app: &mut App, query: &str) -> bool {
    // (a search re-centres on the hottest function whose name matches)
    let p = &app.profiles[app.shown];
    let hit = ranked(app).into_iter().find(|r| p.funcs[r.func as usize].label().to_lowercase().contains(query)).map(|r| r.func);
    if let Some(func) = hit {
        if let Some(current) = center(app) {
            app.calls.history.push(current);
        }
        app.calls.center = Some(func);
        app.calls.list.selected = 0;
        app.focus = Some(Focus { func, line: None });
        return true;
    }
    false
}
