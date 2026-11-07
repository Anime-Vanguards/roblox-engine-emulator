//! Source: a module's lines with their heat in the metric and, beside it, every kind's figure for the line:
//! instructions, allocations, Luau cache misses; the selected line's opcodes below. `s` lists the lines by heat.

use std::{collections::BTreeMap, rc::Rc};

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::{
    overview::ranked,
    widgets::{Col, MARGIN, bar, cell, header, layout, section, select_row, text},
};
use crate::{
    app::{App, Focus, View},
    fmt,
    keys::Key,
    model::{LineFig, Source},
    theme::Theme,
};

/// A row of the listing: a line (its text, its figures summed over the functions on it, the innermost function that
/// holds it), or a gap in a partial listing (line -1)
#[derive(Clone, Debug)]
pub struct SrcRow {
    pub line: i64,
    pub text: String,
    pub fig: Option<LineFig>,
    pub func: Option<u32>,
}

fn add(into: &mut LineFig, fig: &LineFig) {
    into.work += fig.work;
    into.total_work += fig.total_work;
    into.insns += fig.insns;
    into.total_insns += fig.total_insns;
    into.time += fig.time;
    into.total_time += fig.total_time;
    into.alloc += fig.alloc;
    into.allocs += fig.allocs;
    into.misses += fig.misses;
    into.cache_runs += fig.cache_runs;
    if into.counters.len() < fig.counters.len() {
        into.counters.resize(fig.counters.len(), 0.0);
    }
    for (slot, value) in into.counters.iter_mut().zip(&fig.counters) {
        *slot += value;
    }
    for (op, count) in &fig.ops {
        *into.ops.entry(op.clone()).or_default() += count;
    }
}

/// The module shown: the followed function's, else the one shown last, else the hottest function's
pub fn module(app: &App) -> Option<String> {
    let p = &app.profiles[app.shown];
    if let Some(focus) = app.focus {
        let module = &p.funcs[focus.func as usize].module;
        if !module.is_empty() {
            return Some(module.clone());
        }
    }
    if let Some(module) = app.source.module.as_ref().filter(|m| !m.is_empty()) {
        return Some(module.clone());
    }
    let func = ranked(app).into_iter().map(|r| r.func).find(|f| !p.funcs[*f as usize].module.is_empty())?;
    Some(p.funcs[func as usize].module.clone())
}

/// The listing's rows, in file order
pub fn rows(app: &App, module: &str) -> (Rc<Option<Source>>, Vec<SrcRow>) {
    let p = &app.profiles[app.shown];
    let source = app.source_text(module);
    let mut figs: BTreeMap<i64, LineFig> = BTreeMap::new();
    for (line, _, fig) in p.module_lines(module) {
        add(figs.entry(line).or_default(), fig);
    }
    // the innermost function holding each line
    let funcs: Vec<(u32, i64, i64)> = p.funcs.iter().enumerate().filter(|(_, f)| f.module == module).map(|(i, f)| (i as u32, f.line, f.last_line.max(f.line))).collect();
    let holder = |line: i64| funcs.iter().filter(|(_, from, to)| *from <= line && line <= *to).max_by_key(|(_, from, _)| *from).map(|(f, _, _)| *f);
    let mut numbers: Vec<i64> = figs.keys().copied().collect();
    let whole = source.as_ref().as_ref().is_some_and(|s| s.whole);
    if let Some(s) = source.as_ref() {
        numbers.extend(s.lines.keys().copied());
    }
    numbers.sort();
    numbers.dedup();
    if whole {
        let last = numbers.last().copied().unwrap_or(0);
        numbers = (1..=last).collect();
    }
    let mut out = Vec::with_capacity(numbers.len());
    let mut previous: Option<i64> = None;
    for line in numbers {
        if line <= 0 {
            continue;
        }
        if let Some(prev) = previous {
            if line > prev + 1 && !whole {
                out.push(SrcRow { line: -1, text: "⋯".into(), fig: None, func: None });
            }
        }
        previous = Some(line);
        let text = source.as_ref().as_ref().and_then(|s| s.lines.get(&line).cloned()).unwrap_or_default();
        out.push(SrcRow { line, text, fig: figs.get(&line).cloned(), func: holder(line) });
    }
    (source, out)
}

/// A path's last three parts (`demo/src/Grid.luau`): where the file is, without the machine's folders
pub fn short_path(path: &str) -> String {
    let parts: Vec<&str> = path.split(['/', '\\']).filter(|part| !part.is_empty()).collect();
    let from = parts.len().saturating_sub(3);
    parts[from..].join("/")
}

/// The smallest figure a cell shows (below it, the cell stays empty rather than read 0)
fn shown_floor(unit: crate::metric::Unit) -> f64 {
    match unit {
        crate::metric::Unit::Ms => 5e-7,
        _ => 0.5,
    }
}

/// The rows as shown: in file order, or by heat (`s`)
fn shown(app: &App, rows: &[SrcRow]) -> Vec<usize> {
    let mut order: Vec<usize> = (0..rows.len()).collect();
    if app.source.hot {
        let metric = app.metric;
        order.retain(|&i| rows[i].fig.as_ref().is_some_and(|f| metric.line_self(f) > 0.0 || metric.line_total(f) > 0.0));
        order.sort_by(|&a, &b| {
            let va = rows[a].fig.as_ref().map_or(0.0, |f| metric.line_self(f));
            let vb = rows[b].fig.as_ref().map_or(0.0, |f| metric.line_self(f));
            vb.total_cmp(&va).then(a.cmp(&b))
        });
    }
    order
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let Some(module) = module(app) else {
        text(buf, area.x, area.y, "no Luau source in this profile (work, time and alloc profiles keep it)", area.width, theme.dim());
        return;
    };
    if app.source.module.as_deref() != Some(module.as_str()) {
        // (another module: its line is the followed one's, else its hottest)
        app.source.module = Some(module.clone());
        app.source.line = app.focus.and_then(|f| f.line).unwrap_or(-1);
        app.source.list = Default::default();
    }
    let (source, rows) = rows(app, &module);
    let order = shown(app, &rows);
    // the selected line: the view's, or the focus's, or the hottest
    let metric = app.metric;
    let mut selected = order.iter().position(|&i| rows[i].line == app.source.line);
    if selected.is_none() {
        selected = order.iter().enumerate().filter(|(_, i)| rows[**i].fig.is_some()).max_by(|a, b| {
            let va = rows[*a.1].fig.as_ref().map_or(0.0, |f| metric.line_self(f));
            let vb = rows[*b.1].fig.as_ref().map_or(0.0, |f| metric.line_self(f));
            va.total_cmp(&vb).then(b.0.cmp(&a.0))
        }).map(|(at, _)| at);
    }
    let detail_height = 4u16;
    let list_height = area.height.saturating_sub(2 + detail_height) as usize;
    let mut list = app.source.list;
    list.selected = selected.unwrap_or(0);
    // (a line reached from elsewhere, off the screen: shown a third of the way down, its context above it)
    if list.selected + 1 < list.offset || list.selected > list.offset + list_height.max(1) {
        list.offset = list.selected.saturating_sub(list_height / 3);
    }
    list.fit(order.len(), list_height);
    if let Some(&row) = order.get(list.selected) {
        app.source.line = rows[row].line;
    }
    let p = &app.profiles[app.shown];
    let total = metric.total(p);
    let unit = metric.unit();
    let label = metric.short_label(p);
    let note = match source.as_ref() {
        Some(s) if s.changed => format!("{} changed since the run: the profile's lines", short_path(&s.path)),
        Some(s) if s.whole => short_path(&s.path),
        Some(s) => format!("{}: not here, the profile's lines", short_path(&s.path)),
        None => "no source text: figures only".to_string(),
    };
    section(buf, area, area.y, crate::model::short_module(&module), &note, theme);
    // columns: line, the metric's own and total, the heat bar, then the other kinds the profile has
    let has_total = matches!(metric, crate::metric::Metric::Work | crate::metric::Metric::Insns | crate::metric::Metric::Time);
    let mut cols = vec![Col::right("line", 5), Col::right(&fmt::fit(&label, 10), 10)];
    if has_total {
        cols.push(Col::right("with calls", 10));
    }
    cols.push(Col::left("", 8));
    let wide = area.width >= 120;
    let extra_insns = wide && metric != crate::metric::Metric::Insns && p.raw.work.is_some();
    let extra_alloc = wide && !matches!(metric, crate::metric::Metric::AllocBytes | crate::metric::Metric::AllocCount) && p.raw.alloc.is_some();
    let extra_miss = wide && metric != crate::metric::Metric::CacheMisses && p.raw.luau_caches.is_some();
    if extra_insns {
        cols.push(Col::right("insns", 7));
    }
    if extra_alloc {
        cols.push(Col::right("alloc", 9));
    }
    if extra_miss {
        cols.push(Col::right("misses", 7));
    }
    cols.push(Col::left("", 2));
    cols.push(Col::left("code", 20));
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    header(buf, &cols, &xs, area.y + 1, theme);
    let max = rows.iter().filter_map(|r| r.fig.as_ref()).map(|f| metric.line_self(f)).fold(0.0, f64::max);
    let focus_func = app.focus.map(|f| f.func);
    let query = app.query.to_lowercase();
    for (screen, &row_index) in order.iter().enumerate().skip(list.offset).take(list_height) {
        let row = &rows[row_index];
        let y = area.y + 2 + (screen - list.offset) as u16;
        if row.line < 0 {
            cell(buf, &cols[0], xs[0], y, "⋯", theme.dim());
            continue;
        }
        let own = row.fig.as_ref().map_or(0.0, |f| metric.line_self(f));
        let share = if total > 0.0 { own / total } else { 0.0 };
        let style = theme.row(share);
        let mut c = 0;
        cell(buf, &cols[c], xs[c], y, &row.line.to_string(), theme.dim());
        c += 1;
        if let Some(fig) = &row.fig {
            if own >= shown_floor(unit) {
                cell(buf, &cols[c], xs[c], y, &fmt::short(unit, own), style);
            }
            c += 1;
            if has_total {
                let with = metric.line_total(fig);
                if with > own {
                    cell(buf, &cols[c], xs[c], y, &fmt::short(unit, with), theme.secondary());
                }
                c += 1;
            }
            bar(buf, xs[c], y, cols[c].width, if max > 0.0 { own / max } else { 0.0 }, theme.heat_of(share), theme);
            c += 1;
            if extra_insns {
                if fig.insns > 0.0 {
                    cell(buf, &cols[c], xs[c], y, &fmt::compact(fig.insns), theme.secondary());
                }
                c += 1;
            }
            if extra_alloc {
                if fig.alloc > 0.0 {
                    cell(buf, &cols[c], xs[c], y, &fmt::bytes(fig.alloc), theme.secondary());
                }
                c += 1;
            }
            if extra_miss {
                if fig.misses > 0.0 {
                    cell(buf, &cols[c], xs[c], y, &fmt::compact(fig.misses), Style::new().fg(theme.serious).bg(theme.surface));
                }
                c += 1;
            }
        } else {
            c += if has_total { 3 } else { 2 };
            c += extra_insns as usize + extra_alloc as usize + extra_miss as usize;
        }
        // the focused function's lines get a gutter mark
        if row.func.is_some() && row.func == focus_func {
            cell(buf, &cols[c], xs[c], y, "│", Style::new().fg(theme.ink2).bg(theme.surface));
        }
        c += 1;
        let code_style = if row.fig.is_some() { theme.text() } else { theme.dim() };
        let code_style = if !query.is_empty() && row.text.to_lowercase().contains(&query) { Style::new().fg(theme.found_ink).bg(theme.found) } else { code_style };
        cell(buf, &cols[c], xs[c], y, &row.text, code_style);
        if screen == list.selected {
            select_row(buf, area, y, theme);
        }
    }
    super::widgets::scroll_note(buf, area, area.y + 1, list.offset, list_height, order.len(), theme);
    app.source.list = list;
    // the selected line in full
    let y = area.bottom() - detail_height;
    let Some(row) = order.get(list.selected).map(|&i| &rows[i]) else { return };
    let func_label = row.func.map(|f| p.funcs[f as usize].label()).unwrap_or_default();
    section(buf, area, y, &format!("Line {}", row.line.max(0)), &func_label, theme);
    let Some(fig) = &row.fig else {
        text(buf, area.x, y + 1, "nothing ran here", area.width, theme.dim());
        return;
    };
    let mut parts: Vec<String> = Vec::new();
    if p.raw.work.is_some() {
        let calls = if fig.total_work > fig.work + 1e-9 { format!(", {} with calls", fmt::ms(fig.total_work)) } else { String::new() };
        parts.push(format!("work {}{calls}", fmt::ms(fig.work)));
        parts.push(format!("instructions {}", fmt::grouped(fig.insns)));
    }
    if p.raw.time.is_some() {
        parts.push(format!("host {}", fmt::ms(fig.time)));
    }
    if p.raw.alloc.is_some() && fig.allocs > 0.0 {
        parts.push(format!("allocated {}, {} objects{}", fmt::bytes(fig.alloc), fmt::grouped(fig.allocs), if p.line_alloc_complete { "" } else { " (kept stacks only)" }));
    }
    if fig.cache_runs > 0.0 {
        parts.push(format!("cache misses {} of {}", fmt::grouped(fig.misses), fmt::grouped(fig.cache_runs)));
    }
    if let Some(c) = &p.raw.counters {
        for (name, value) in c.names.iter().zip(&fig.counters) {
            if *value > 0.0 {
                parts.push(format!("{name} {}", fmt::compact(*value)));
            }
        }
    }
    text(buf, area.x, y + 1, &parts.join(" · "), area.width, theme.text());
    let mut ops: Vec<(&String, &u64)> = fig.ops.iter().collect();
    ops.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
    if !ops.is_empty() {
        let line = ops.iter().take(8).map(|(op, n)| format!("{op} {}", fmt::grouped(**n as f64))).collect::<Vec<_>>().join(" · ");
        text(buf, area.x, y + 2, &format!("opcodes: {line}"), area.width, theme.secondary());
    }
}

pub fn key(app: &mut App, key: Key) {
    let Some(module) = module(app) else { return };
    let (_, rows) = rows(app, &module);
    let order = shown(app, &rows);
    let mut list = app.source.list;
    list.selected = order.iter().position(|&i| rows[i].line == app.source.line).unwrap_or(list.selected);
    if list.key(key, order.len(), app.page) {
        if let Some(&i) = order.get(list.selected) {
            app.source.line = rows[i].line;
            if let Some(func) = rows[i].func {
                app.focus = Some(Focus { func, line: Some(rows[i].line) });
            }
        }
        app.source.list = list;
        return;
    }
    match key {
        Key::Left | Key::Right => {
            // the previous or next line that is not negligible
            let metric = app.metric;
            let p = &app.profiles[app.shown];
            let total = metric.total(p);
            let hot = |i: usize| rows[order[i]].fig.as_ref().is_some_and(|f| total > 0.0 && metric.line_self(f) / total >= crate::theme::HEAT_FROM[0]);
            if let Some(at) = super::next_match(order.len(), list.selected, key == Key::Right, false, hot) {
                app.source.line = rows[order[at]].line;
                if let Some(func) = rows[order[at]].func {
                    app.focus = Some(Focus { func, line: Some(rows[order[at]].line) });
                }
            }
        }
        Key::Char('s') => {
            app.source.hot = !app.source.hot;
            app.status = if app.source.hot { "lines by heat".into() } else { "lines in file order".into() };
        }
        Key::Enter => {
            if let Some(func) = order.get(list.selected).and_then(|&i| rows[i].func) {
                app.focus = Some(Focus { func, line: None });
                app.set_view(View::Calls);
            }
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let module = module(app)?;
    let p = &app.profiles[app.shown];
    Some(match p.module_path(&module) {
        Some(path) => format!("{path}:{}", app.source.line),
        None => format!("{module}:{}", app.source.line),
    })
}

pub fn find(app: &mut App, query: &str, forward: bool, from_start: bool) -> bool {
    let Some(module) = module(app) else { return false };
    let (_, rows) = rows(app, &module);
    let order = shown(app, &rows);
    let at = order.iter().position(|&i| rows[i].line == app.source.line).unwrap_or(0);
    match super::next_match(order.len(), at, forward, from_start, |i| rows[order[i]].text.to_lowercase().contains(query)) {
        Some(found) => {
            app.source.line = rows[order[found]].line;
            true
        }
        None => false,
    }
}
