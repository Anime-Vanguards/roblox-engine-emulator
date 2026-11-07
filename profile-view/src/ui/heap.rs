//! Heap: the reachable heap at each snapshot (the start, every phase mark, the end), what holds the most of it, its
//! types, the run's own reachable objects by the stack that allocated them, and the collector's work frame by frame.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::widgets::{Col, MARGIN, bar, cell, header, layout, section, select_row, sparkline, spans, text};
use crate::{
    app::{App, Focus, HeapPane, View},
    fmt,
    keys::Key,
    model::{Profile, Snapshot},
    theme::Theme,
};

fn snapshot_index(app: &App) -> Option<usize> {
    let snapshots = &app.profile().raw.heap.as_ref()?.snapshots;
    if snapshots.is_empty() {
        return None;
    }
    Some(app.heap.snapshot.unwrap_or(snapshots.len() - 1).min(snapshots.len() - 1))
}

/// A stack's frames as `Module:line < Module:line`
fn stack_text(p: &Profile, frames: &[(i64, i64)]) -> String {
    let parts: Vec<String> = frames.iter().filter_map(|(id, line)| {
        let func = p.func_of_id(*id)?;
        let f = &p.funcs[func];
        Some(if f.module.is_empty() || *line < 0 { if f.module.is_empty() { f.name.clone() } else { f.short.clone() } } else { format!("{}:{line}", f.short) })
    }).take(6).collect();
    if parts.is_empty() { "(engine)".into() } else { parts.join(" < ") }
}

fn pane_len(snapshot: &Snapshot, pane: HeapPane) -> usize {
    match pane {
        HeapPane::Holders => snapshot.biggest.len(),
        HeapPane::Types => snapshot.by_type.len(),
        HeapPane::Stacks => snapshot.by_stack.len(),
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let Some(index) = snapshot_index(app) else {
        text(buf, area.x, area.y, "no heap snapshots in this profile (--profile heap)", area.width, theme.dim());
        return;
    };
    let mut list = app.heap.list;
    let pane = app.heap.pane;
    let p = &app.profiles[app.shown];
    let heap = p.raw.heap.as_ref().unwrap();
    let snapshot = &heap.snapshots[index];
    let mut y = area.y;
    // the snapshots, left to right
    let mut x = area.x;
    let mut previous: Option<f64> = None;
    for (i, s) in heap.snapshots.iter().enumerate() {
        let (reachable, _, unswept) = s.split();
        let delta = previous.map(|before| format!(" {}", fmt::signed(crate::metric::Unit::Bytes, reachable - before))).unwrap_or_default();
        let label = format!(" {}, frame {}: {}{} ", s.label, s.frame, fmt::bytes(reachable), delta);
        let style = if i == index { theme.selected() } else { theme.secondary() };
        let width = label.chars().count() as u16;
        if x + width + 2 > area.right() {
            break;
        }
        text(buf, x, y, &label, width, style);
        x += width;
        if i + 1 < heap.snapshots.len() {
            text(buf, x, y, "→ ", 2, theme.dim());
            x += 2;
        }
        let _ = unswept;
        previous = Some(reachable);
    }
    y += 1;
    let (reachable, objects, unswept) = snapshot.split();
    text(
        buf,
        area.x,
        y,
        &format!(
            "reachable {}, {} objects · garbage not swept yet {} · allocated by the run and still reachable {} · gcinfo {} KB",
            fmt::bytes(reachable),
            fmt::grouped(objects),
            fmt::bytes(unswept),
            fmt::bytes(snapshot.allocated_since.bytes),
            fmt::grouped(snapshot.gcinfo_kb)
        ),
        area.width,
        theme.secondary(),
    );
    y += 2;
    let collector_height = 4u16;
    let body = area.bottom().saturating_sub(y + collector_height);
    let top_height = body / 2;
    let left = Rect { x: area.x, y, width: area.width * 3 / 5 - 1, height: top_height };
    let right = Rect { x: area.x + area.width * 3 / 5 + 1, y, width: area.width - area.width * 3 / 5 - 1, height: top_height };
    let lower = Rect { x: area.x, y: y + top_height, width: area.width, height: body - top_height };
    let mark = |p: HeapPane| if pane == p { "▸ " } else { "" };
    // what holds the most
    section(buf, left, left.y, &format!("{}Biggest holders", mark(HeapPane::Holders)), "", theme);
    let mut cols = vec![Col::right("keeps", 10), Col::right("own", 9), Col::left("object", 22), Col::left("held by", 20)];
    let xs = layout(&mut cols, left.x + MARGIN, left.width - MARGIN);
    header(buf, &cols, &xs, left.y + 1, theme);
    let rows = left.height.saturating_sub(2) as usize;
    if pane == HeapPane::Holders {
        list.fit(pane_len(snapshot, pane), rows);
    }
    let offset = if pane == HeapPane::Holders { list.offset } else { 0 };
    for (i, held) in snapshot.biggest.iter().enumerate().skip(offset).take(rows) {
        let row_y = left.y + 2 + (i - offset) as u16;
        let share = if reachable > 0.0 { held.retained / reachable } else { 0.0 };
        let style = theme.row(share);
        cell(buf, &cols[0], xs[0], row_y, &fmt::bytes(held.retained), style);
        cell(buf, &cols[1], xs[1], row_y, &fmt::bytes(held.size), theme.secondary());
        let name = if held.name.is_empty() { held.ty.clone() } else { format!("{} \"{}\"", held.ty, held.name) };
        cell(buf, &cols[2], xs[2], row_y, &name, style);
        cell(buf, &cols[3], xs[3], row_y, &held.path, theme.secondary());
        if pane == HeapPane::Holders && i == list.selected {
            select_row(buf, left, row_y, theme);
        }
    }
    // by type
    section(buf, right, right.y, &format!("{}By type", mark(HeapPane::Types)), "", theme);
    let rows_right = right.height.saturating_sub(1) as usize;
    if pane == HeapPane::Types {
        list.fit(pane_len(snapshot, pane), rows_right);
    }
    let offset = if pane == HeapPane::Types { list.offset } else { 0 };
    let max = snapshot.by_type.first().map_or(0.0, |t| t.bytes);
    for (i, ty) in snapshot.by_type.iter().enumerate().skip(offset).take(rows_right) {
        let row_y = right.y + 1 + (i - offset) as u16;
        let share = if reachable > 0.0 { ty.bytes / reachable } else { 0.0 };
        text(buf, right.x + MARGIN, row_y, &fmt::fit(&ty.ty, 22), 22, theme.text());
        text(buf, right.x + MARGIN + 23, row_y, &format!("{:>9} {:>7}", fmt::bytes(ty.bytes), fmt::compact(ty.count)), 17, theme.secondary());
        let bar_x = right.x + MARGIN + 41;
        if bar_x + 4 < right.right() {
            bar(buf, bar_x, row_y, right.right() - bar_x, if max > 0.0 { ty.bytes / max } else { 0.0 }, theme.heat_of(share), theme);
        }
        if pane == HeapPane::Types && i == list.selected {
            select_row(buf, right, row_y, theme);
        }
    }
    // the run's own reachable objects
    section(buf, lower, lower.y, &format!("{}Allocated by the run, still reachable", mark(HeapPane::Stacks)), &format!("{}, {} objects, by stack", fmt::bytes(snapshot.allocated_since.bytes), fmt::grouped(snapshot.allocated_since.count)), theme);
    let mut cols = vec![Col::right("bytes", 10), Col::right("objects", 9), Col::left("type", 18), Col::left("stack, innermost first", 20)];
    let xs = layout(&mut cols, lower.x + MARGIN, lower.width - MARGIN);
    header(buf, &cols, &xs, lower.y + 1, theme);
    let rows_lower = lower.height.saturating_sub(2) as usize;
    if pane == HeapPane::Stacks {
        list.fit(pane_len(snapshot, pane), rows_lower);
    }
    let offset = if pane == HeapPane::Stacks { list.offset } else { 0 };
    if snapshot.by_stack.is_empty() {
        text(buf, lower.x + MARGIN, lower.y + 2, "not followed (needs the alloc kind with heap)", lower.width, theme.dim());
    }
    for (i, row) in snapshot.by_stack.iter().enumerate().skip(offset).take(rows_lower) {
        let row_y = lower.y + 2 + (i - offset) as u16;
        let share = if reachable > 0.0 { row.bytes / reachable } else { 0.0 };
        cell(buf, &cols[0], xs[0], row_y, &fmt::bytes(row.bytes), theme.row(share));
        cell(buf, &cols[1], xs[1], row_y, &fmt::compact(row.count), theme.secondary());
        cell(buf, &cols[2], xs[2], row_y, &row.ty, theme.secondary());
        cell(buf, &cols[3], xs[3], row_y, &stack_text(p, row.frames.as_deref().unwrap_or(&[])), theme.text());
        if pane == HeapPane::Stacks && i == list.selected {
            select_row(buf, lower, row_y, theme);
        }
    }
    // the collector
    let y = area.bottom() - collector_height;
    let gc = &heap.gc;
    section(buf, area, y, "Collector", &format!("{} cycles · {} steps · {} freed · {}", fmt::grouped(gc.cycles), fmt::grouped(gc.steps), fmt::bytes(gc.freed), fmt::ms(gc.ms)), theme);
    let states: Vec<String> = gc.by_state.iter().filter(|s| s.steps > 0.0).map(|s| format!("{} {} ({} steps)", s.state, fmt::ms(s.ms), fmt::grouped(s.steps))).collect();
    text(buf, area.x, y + 1, &states.join(" · "), area.width, theme.secondary());
    if let Some(timeline) = &p.raw.timeline {
        if let Some(gc_ms) = timeline.values("gc_ms") {
            let width = area.width.saturating_sub(30) as usize;
            let values = super::overview::bucket_max(&gc_ms, width);
            let max = values.iter().copied().fold(0.0, f64::max);
            let x = spans(buf, area.x, y + 2, area.right(), &[("time a frame ", theme.dim())]);
            sparkline(buf, x, y + 2, &values, max, |_| theme.ink2, theme);
            text(buf, x + values.len() as u16 + 1, y + 2, &format!("max {}", fmt::ms(max)), 14, theme.dim());
        }
    }
    app.heap.list = list;
    let _ = Style::new();
}

pub fn key(app: &mut App, key: Key) {
    let Some(index) = snapshot_index(app) else { return };
    let count = app.profile().raw.heap.as_ref().map_or(0, |h| h.snapshots.len());
    match key {
        Key::Left => {
            app.heap.snapshot = Some(index.saturating_sub(1));
            app.heap.list = Default::default();
        }
        Key::Right => {
            app.heap.snapshot = Some((index + 1).min(count.saturating_sub(1)));
            app.heap.list = Default::default();
        }
        Key::Tab | Key::Char(' ') => {
            app.heap.pane = match app.heap.pane {
                HeapPane::Holders => HeapPane::Types,
                HeapPane::Types => HeapPane::Stacks,
                HeapPane::Stacks => HeapPane::Holders,
            };
            app.heap.list = Default::default();
        }
        Key::Enter => {
            // a stack's innermost Luau line
            if app.heap.pane == HeapPane::Stacks {
                let p = &app.profiles[app.shown];
                let snapshot = &p.raw.heap.as_ref().unwrap().snapshots[index];
                if let Some(frames) = snapshot.by_stack.get(app.heap.list.selected).and_then(|row| row.frames.clone()) {
                    if let Some((func, line)) = frames.iter().find_map(|(id, line)| p.func_of_id(*id).filter(|f| !p.funcs[*f].module.is_empty()).map(|f| (f as u32, *line))) {
                        app.focus = Some(Focus { func, line: Some(line) });
                        app.source.module = None;
                        app.source.line = line;
                        app.set_view(View::Source);
                    }
                }
            }
        }
        _ => {
            let p = &app.profiles[app.shown];
            let snapshot = &p.raw.heap.as_ref().unwrap().snapshots[index];
            let len = pane_len(snapshot, app.heap.pane);
            let page = app.page;
            app.heap.list.key(key, len, page);
        }
    }
}

pub fn copy(app: &App) -> Option<String> {
    let index = snapshot_index(app)?;
    let p = &app.profiles[app.shown];
    let snapshot = &p.raw.heap.as_ref()?.snapshots[index];
    match app.heap.pane {
        HeapPane::Holders => snapshot.biggest.get(app.heap.list.selected).map(|h| h.path.clone()),
        HeapPane::Types => snapshot.by_type.get(app.heap.list.selected).map(|t| t.ty.clone()),
        HeapPane::Stacks => snapshot.by_stack.get(app.heap.list.selected).map(|row| stack_text(p, row.frames.as_deref().unwrap_or(&[]))),
    }
}
