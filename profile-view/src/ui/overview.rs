//! Overview: the run in a few lines: its totals, the work clock's fidelity line, the run frame by frame, the top
//! issues and the hottest functions.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::{
    issues::severity_style,
    widgets::{Col, MARGIN, bar, cell, columns, header, layout, section, select_row, spans, text},
    wrap,
};
use crate::{app::App, fmt, keys::Key, model::Kind, theme::Theme};

/// A function in a ranking: own, total, calls
#[derive(Clone, Copy, Debug)]
pub struct Ranked {
    pub func: u32,
    pub own: f64,
    pub total: f64,
    pub calls: u64,
}

/// Every function ranked by its own figure in the current metric (scripts only when asked)
pub fn ranked(app: &App) -> Vec<Ranked> {
    let p = &app.profiles[app.shown];
    if app.metric.has_stacks(p) {
        return app.tree().ranked().into_iter().map(|(func, stat)| Ranked { func, own: stat.own, total: stat.total, calls: stat.calls }).collect();
    }
    // cache misses: by the sites' sums
    let mut rows: Vec<Ranked> = p.miss_funcs.iter().filter(|(func, _)| !app.scripts_only || p.funcs[**func].is_script()).map(|(&func, &(missed, _))| Ranked { func: func as u32, own: missed, total: missed, calls: p.funcs[func].calls }).collect();
    rows.sort_by(|a, b| b.own.total_cmp(&a.own).then(a.func.cmp(&b.func)));
    rows
}

/// The functions worth showing first: their own figure above zero, not the engine's own Luau
pub fn hot(app: &App) -> Vec<Ranked> {
    let p = &app.profiles[app.shown];
    ranked(app).into_iter().filter(|r| r.own > 0.0 && p.funcs[r.func as usize].kind != Kind::Engine).collect()
}

/// `Work`, `Allocated`: a timeline column's name in titles
pub fn series_label(column: &str) -> String {
    let label = match column {
        "work_ms" => "work",
        "host_ms" => "host time",
        "heap_bytes" => "heap",
        "allocated_bytes" => "allocated",
        "allocations" => "allocations",
        "gc_steps" => "collector steps",
        "gc_ms" => "collector time",
        "gc_freed" => "collector freed",
        "cycles" => "CPU cycles",
        other => other.strip_prefix("hw:").unwrap_or(other),
    };
    let mut chars = label.chars();
    chars.next().map_or(String::new(), |first| first.to_uppercase().collect::<String>() + chars.as_str())
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let issues = app.issues();
    let hot = hot(app);
    let mut list = app.overview;
    let p = &app.profiles[app.shown];
    let run = &p.raw.run;
    let mut y = area.y;
    let right = area.right();
    // what the header leaves out
    let line = format!("{}kinds {} · {:.2} s wall", if run.fps > 0.0 { format!("{} fps · ", run.fps) } else { String::new() }, p.raw.kinds.join(", "), run.elapsed);
    text(buf, area.x, y, &line, area.width, theme.secondary());
    y += 1;
    for warning in p.raw.warnings.iter().take(2) {
        spans(buf, area.x, y, right, &[("▲ ", Style::new().fg(theme.serious).bg(theme.surface)), (warning, theme.text())]);
        y += 1;
    }
    y += 1;
    // the tiles: label over value
    let mut tiles: Vec<(String, String)> = Vec::new();
    if let Some(work) = &p.raw.work {
        tiles.push(("Studio work".into(), fmt::ms(work.total_ms)));
        tiles.push(("a frame".into(), fmt::ms(work.per_frame_ms)));
        tiles.push(("instructions".into(), fmt::compact(work.script_insns + work.engine_insns)));
    }
    if let Some(time) = &p.raw.time {
        tiles.push(("host time".into(), fmt::ms(time.sampled_ms)));
    }
    if let Some(alloc) = &p.raw.alloc {
        tiles.push(("allocated".into(), fmt::bytes(alloc.bytes)));
        tiles.push(("objects".into(), fmt::compact(alloc.objects)));
    }
    if let Some(heap) = &p.raw.heap {
        if let Some(last) = heap.snapshots.last() {
            tiles.push(("reachable at end".into(), fmt::bytes(last.split().0)));
        }
        tiles.push(("collections".into(), fmt::grouped(heap.gc.cycles)));
    }
    if let Some(c) = &p.raw.counters {
        if c.error.is_none() {
            if let Some(ipc) = c.run.rates.get("IPC") {
                tiles.push(("IPC".into(), format!("{ipc:.2}")));
            }
        }
    }
    let mut x = area.x;
    for (label, value) in &tiles {
        let width = (label.chars().count().max(value.chars().count()) + 3) as u16;
        if x + width > right {
            break;
        }
        text(buf, x, y, label, width, theme.dim());
        text(buf, x, y + 1, value, width, theme.bold());
        x += width;
    }
    y += 3;
    // the fidelity line: what the work clock's milliseconds are beside a client's
    if let Some(work) = p.raw.work.as_ref().filter(|w| !w.fidelity.is_empty()) {
        let lines = wrap(&work.fidelity, area.width as usize);
        for (index, chunk) in lines.iter().take(2).enumerate() {
            let shown = if index == 1 && lines.len() > 2 { format!("{} (? for the rest)", fmt::fit(chunk, area.width as usize - 18)) } else { chunk.clone() };
            text(buf, area.x, y, &shown, area.width, theme.dim());
            y += 1;
        }
        y += 1;
    }
    // frame by frame
    if let Some(timeline) = &p.raw.timeline {
        let column = app.metric.timeline_column(p).filter(|c| timeline.index(c).is_some()).unwrap_or_else(|| "work_ms".into());
        if let Some(values) = timeline.values(&column) {
            let unit = if column.ends_with("_ms") { crate::metric::Unit::Ms } else if column.contains("bytes") { crate::metric::Unit::Bytes } else { crate::metric::Unit::Count };
            let total: f64 = values.iter().sum();
            let max = values.iter().copied().fold(0.0, f64::max);
            let note = format!("{} frames · mean {} · max {}", values.len(), fmt::short(unit, total / values.len().max(1) as f64), fmt::short(unit, max));
            section(buf, area, y, &format!("{} a frame", series_label(&column)), &note, theme);
            y += 1;
            let buckets = bucket_max(&values, area.width as usize);
            let top = buckets.iter().copied().fold(0.0, f64::max);
            // (one series in one neutral colour: the heights are the figure; heat stays a share of the run)
            columns(buf, Rect { x: area.x, y, width: buckets.len() as u16, height: 3 }, &buckets, top, |_| theme.ink2, theme);
            y += 3;
            // phases under the columns
            let phases = timeline.values("phase").unwrap_or_default();
            let per = values.len() as f64 / buckets.len().max(1) as f64;
            let mut last: Option<u32> = None;
            for column_index in 0..buckets.len() {
                let row = ((column_index as f64 * per) as usize).min(phases.len().saturating_sub(1));
                let phase = phases.get(row).copied().unwrap_or(0.0) as u32;
                if last != Some(phase) {
                    let name = format!("▏{}", p.phase_name(phase));
                    text(buf, area.x + column_index as u16, y, &name, (buckets.len() - column_index) as u16, theme.dim());
                    last = Some(phase);
                }
            }
            y += 2;
        }
    }
    // the issues and the hottest functions, side by side
    let height = area.bottom().saturating_sub(y);
    if height < 3 {
        return;
    }
    let left = Rect { x: area.x, y, width: area.width / 2 - 1, height };
    let right_area = Rect { x: area.x + area.width / 2 + 1, y, width: area.width - area.width / 2 - 1, height };
    let (found, unchecked) = (&issues.0, &issues.1);
    section(buf, left, y, "Issues", &found.len().to_string(), theme);
    let rows = height.saturating_sub(1) as usize;
    list.fit(found.len(), rows);
    for (index, issue) in found.iter().enumerate().skip(list.offset).take(rows) {
        let row_y = y + 1 + (index - list.offset) as u16;
        let x = spans(buf, left.x + MARGIN, row_y, left.right(), &[(&format!("{} ", issue.severity.icon()), severity_style(issue.severity, theme)), (&format!("{:<9}", issue.severity.word()), theme.dim())]);
        text(buf, x, row_y, &issue.title, left.right().saturating_sub(x), theme.text());
        if index == list.selected {
            select_row(buf, left, row_y, theme);
        }
    }
    if found.is_empty() {
        text(buf, left.x + MARGIN, y + 1, "none found", left.width - MARGIN, theme.dim());
        for (index, note) in unchecked.iter().take(rows.saturating_sub(2)).enumerate() {
            text(buf, left.x + MARGIN, y + 2 + index as u16, &format!("not checked: {note}"), left.width - MARGIN, theme.dim());
        }
    }
    app.overview = list;
    // the hottest functions
    let metric = app.metric;
    let p = &app.profiles[app.shown];
    let total = metric.total(p);
    let unit = metric.unit();
    section(buf, right_area, y, "Hottest functions", &format!("own {}", metric.label(p)), theme);
    let mut cols = vec![Col::left("", 20), Col::right("own", 10), Col::right("share", 6), Col::left("", 12)];
    let xs = layout(&mut cols, right_area.x, right_area.width);
    header(buf, &cols, &xs, y + 1, theme);
    let rows = height.saturating_sub(2) as usize;
    let max = hot.first().map_or(0.0, |r| r.own);
    for (index, r) in hot.iter().take(rows).enumerate() {
        let row_y = y + 2 + index as u16;
        let share = if total > 0.0 { r.own / total } else { 0.0 };
        let style = theme.row(share);
        cell(buf, &cols[0], xs[0], row_y, &p.funcs[r.func as usize].label(), style);
        cell(buf, &cols[1], xs[1], row_y, &fmt::short(unit, r.own), style);
        cell(buf, &cols[2], xs[2], row_y, &fmt::share(r.own, total), style);
        bar(buf, xs[3], row_y, cols[3].width, if max > 0.0 { r.own / max } else { 0.0 }, theme.heat_of(share), theme);
    }
}

/// Values in at most `width` columns, each the largest of its frames (a spike stays visible)
pub fn bucket_max(values: &[f64], width: usize) -> Vec<f64> {
    if values.len() <= width || width == 0 {
        return values.to_vec();
    }
    let per = values.len() as f64 / width as f64;
    (0..width)
        .map(|i| {
            let from = (i as f64 * per) as usize;
            let to = (((i + 1) as f64 * per) as usize).max(from + 1).min(values.len());
            values[from..to].iter().copied().fold(0.0, f64::max)
        })
        .collect()
}

pub fn key(app: &mut App, key: Key) {
    let issues = app.issues();
    let len = issues.0.len();
    if app.overview.key(key, len, app.page) {
        return;
    }
    if key == Key::Enter {
        if let Some(issue) = issues.0.get(app.overview.selected) {
            let target = issue.target.clone();
            app.follow(&target);
        }
    }
}
