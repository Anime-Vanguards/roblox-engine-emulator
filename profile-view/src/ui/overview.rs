//! Overview: the run in a glance: its totals as cards (the mount's work apart from a frame's), the run frame by frame
//! coloured by phase, the issues and the hottest functions.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Sparkline, SparklineBar, Widget},
};

use super::{
    issues::severity_cell,
    kit::{self, Item, col, col_fill, col_r},
};
use crate::{
    app::App,
    fmt,
    keys::Key,
    metric::Unit,
    model::{Kind, Profile},
    theme::{Mark, Theme},
};

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

/// The run's phases in the order the timeline first shows them: a phase's colour is the categorical slot of its place
pub fn phase_order(p: &Profile) -> Vec<u32> {
    let mut order: Vec<u32> = Vec::new();
    for phase in p.raw.timeline.as_ref().and_then(|t| t.values("phase")).unwrap_or_default() {
        let phase = phase as u32;
        if !order.contains(&phase) {
            order.push(phase);
        }
    }
    order
}

/// A phase's colour
pub fn phase_color(order: &[u32], phase: u32, theme: &Theme) -> Color {
    theme.slot(order.iter().position(|p| *p == phase).unwrap_or(0))
}

/// A timeline column's unit
pub fn unit_of(column: &str) -> Unit {
    if column.ends_with("_ms") {
        Unit::Ms
    } else if column.contains("bytes") || column == "gc_freed" {
        Unit::Bytes
    } else {
        Unit::Count
    }
}

/// The frames as columns the area's height (`axis` lays them out), each column coloured by `color` of its index; a
/// gap between columns in a screenshot when frames share them
pub fn frame_columns(buf: &mut Buffer, area: Rect, axis: &kit::FrameAxis, values: &[f64], color: impl Fn(usize) -> Color, theme: &Theme) {
    let columns = axis.values(values);
    let max = columns.iter().copied().fold(0.0, f64::max);
    let eighths = area.height as f64 * 8.0;
    let bars: Vec<SparklineBar> = columns
        .iter()
        .enumerate()
        .map(|(column, &v)| {
            // (a frame above zero shows at least an eighth)
            let value = if max > 0.0 && v > 0.0 { ((v / max) * eighths).round().max(1.0) as u64 } else { 0 };
            SparklineBar::from(value).style(Style::new().fg(color(column)))
        })
        .collect();
    let bg = if axis.stretched() { theme.panel } else { Theme::marked(theme.panel, Mark::Column) };
    Sparkline::default().data(bars).max(eighths as u64).style(Style::new().bg(bg)).render(Rect { width: columns.len() as u16, ..area }, buf);
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let issues = app.issues();
    let hot = hot(app);
    let mut list = app.overview;
    let p = &app.profiles[app.shown];
    let fidelity = p.raw.work.as_ref().map(|w| w.fidelity.trim_start_matches("fidelity: ").to_string()).filter(|f| !f.is_empty());
    let has_timeline = p.raw.timeline.is_some();
    let parts = kit::rows(area, &[4, u16::from(fidelity.is_some()), if has_timeline { 9 } else { 0 }, 0]);
    cards(buf, parts[0], p, theme);
    if let Some(fidelity) = fidelity {
        let line = Line::from(vec![
            Span::styled(" ◇ work clock  ", Style::new().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(fmt::fit(&fidelity, parts[1].width.saturating_sub(26) as usize), Style::new().fg(theme.muted)),
            Span::styled("  ? more", Style::new().fg(theme.ink2)),
        ]);
        Paragraph::new(line).render(parts[1], buf);
    }
    if has_timeline {
        frames(buf, parts[2], app, theme);
    }
    // the issues and the hottest functions, side by side
    let (left, right) = kit::split(parts[3], parts[3].width / 2);
    let (found, unchecked) = (&issues.0, &issues.1);
    let inner = kit::draw_panel(buf, left, kit::panel(theme, "Issues", &fmt::grouped(found.len() as f64), true));
    if found.is_empty() {
        let mut lines = vec![Line::from(Span::styled("✓ none found", Style::new().fg(theme.good).add_modifier(Modifier::BOLD)))];
        lines.extend(unchecked.iter().map(|note| Line::from(Span::styled(format!("○ not checked: {note}"), Style::new().fg(theme.muted)))));
        Paragraph::new(lines).render(inner, buf);
    } else {
        list.fit(found.len(), kit::rows_shown(inner));
        let rows = found.iter().map(|issue| vec![severity_cell(issue.severity, theme), Item::label(&issue.title, Style::new().fg(theme.ink))]).collect();
        let grid = kit::table(buf, inner, theme, &[col("", 10), col_fill("issue", 1)], rows, Some(list.selected), list.offset);
        list.offset = grid.offset;
    }
    app.overview = list;
    // the hottest functions
    let metric = app.metric;
    let p = &app.profiles[app.shown];
    let total = metric.total(p);
    let unit = metric.unit();
    let block = kit::bottom(kit::panel(theme, "Hottest functions", &format!("own {}", metric.label(p)), false), super::legend(app, theme, right.width));
    let inner = kit::draw_panel(buf, right, block);
    let cols = [col_fill("function", 3), col_r("own", 10), col_r("share", 6), col_fill("", 2)];
    let bar_width = kit::widths(&cols, inner.width)[3];
    let max = hot.first().map_or(0.0, |r| r.own);
    let rows = hot
        .iter()
        .take(kit::rows_shown(inner))
        .map(|r| {
            let share = if total > 0.0 { r.own / total } else { 0.0 };
            let style = theme.row(share);
            vec![
                Item::label(p.funcs[r.func as usize].label(), style),
                Item::text(fmt::short(unit, r.own), style),
                Item::text(fmt::share(r.own, total), style),
                Item::Spans(kit::bar(if max > 0.0 { r.own / max } else { 0.0 }, bar_width, theme.heat_of(share))),
            ]
        })
        .collect();
    let grid = kit::table(buf, inner, theme, &cols, rows, None, 0);
    kit::mark(buf, grid.column(3), Mark::Bar);
}

/// A card: its title, its figure, a line under it, and a share drawn as a gauge over that line
type Card = (String, String, Line<'static>, Option<(f64, String, Color)>);

/// The totals as cards: the work (the mount's share of it), a frame, instructions, host time, the allocation, the heap
/// left and the collector
fn cards(buf: &mut Buffer, area: Rect, p: &Profile, theme: &Theme) {
    let mut cards: Vec<Card> = Vec::new();
    let dim = |text: String| Line::from(Span::styled(text, Style::new().fg(theme.muted)));
    if let Some(work) = &p.raw.work {
        let phases = p.phase_work();
        let startup = phases.iter().find(|(phase, _)| *phase == 0).filter(|_| phases.len() > 1).map(|(_, ms)| *ms);
        let gauge = startup.filter(|_| work.total_ms > 0.0).map(|ms| (ms / work.total_ms, format!("{} {}", p.phase_name(0), fmt::share(ms, work.total_ms)), theme.slot(0)));
        cards.push(("Studio work".into(), fmt::ms(work.total_ms), dim("the work clock".into()), gauge));
        if let Some(values) = p.raw.timeline.as_ref().and_then(|t| t.values("work_ms")) {
            let worst = p.frames_past_startup(&values).into_iter().fold(0.0, f64::max);
            cards.push(("a frame".into(), fmt::ms(p.frames_mean(&values)), dim(format!("mean · max {}", fmt::ms(worst))), None));
        }
        let insns = work.script_insns + work.engine_insns;
        let gauge = (insns > 0.0).then(|| (work.script_insns / insns, format!("scripts {}", fmt::share(work.script_insns, insns)), theme.slot(0)));
        cards.push(("instructions".into(), fmt::compact(insns), dim("Luau's".into()), gauge));
    }
    if let Some(time) = &p.raw.time {
        cards.push(("host time".into(), fmt::ms(time.sampled_ms), dim("sampled, noisy".into()), None));
    }
    if let Some(alloc) = &p.raw.alloc {
        cards.push(("allocated".into(), fmt::bytes(alloc.bytes), dim(format!("{} objects", fmt::compact(alloc.objects))), None));
    }
    if let Some(heap) = &p.raw.heap {
        if let Some(last) = heap.snapshots.last() {
            let (reachable, objects, garbage) = last.split();
            let gauge = (reachable + garbage > 0.0).then(|| (garbage / (reachable + garbage), format!("unswept {}", fmt::share(garbage, reachable + garbage)), theme.slot(1)));
            cards.push(("reachable at end".into(), fmt::bytes(reachable), dim(format!("{} objects", fmt::compact(objects))), gauge));
        }
        cards.push(("collector".into(), format!("{} cycles", fmt::grouped(heap.gc.cycles)), dim(format!("{} steps", fmt::grouped(heap.gc.steps))), None));
    }
    if let Some(c) = p.raw.counters.as_ref().filter(|c| c.error.is_none()) {
        if let Some(ipc) = c.run.rates.get("IPC") {
            cards.push(("IPC".into(), format!("{ipc:.2}"), dim("instructions a cycle".into()), None));
        }
    }
    let count = cards.len().min((area.width / 20).max(1) as usize);
    if count == 0 {
        return;
    }
    let width = (area.width + 1) / count as u16;
    for (index, (title, value, under, gauge)) in cards.into_iter().take(count).enumerate() {
        let x = area.x + index as u16 * width;
        let w = if index + 1 == count { area.right() - x } else { width - 1 };
        let under = if gauge.is_some() { Line::default() } else { under };
        let inner = kit::card(buf, Rect { x, width: w, ..area }, theme, &title, &value, under);
        if let Some((ratio, label, color)) = gauge {
            if inner.height >= 2 {
                kit::gauge(buf, Rect { y: inner.y + 1, height: 1, ..inner }, theme, ratio, Line::from(Span::styled(label, Style::new().fg(theme.ink2))), color);
            }
        }
    }
}

/// The run frame by frame in the metric's figure, a column a frame coloured by its phase, the phases named under it
fn frames(buf: &mut Buffer, area: Rect, app: &App, theme: &Theme) {
    let p = &app.profiles[app.shown];
    let Some(timeline) = &p.raw.timeline else { return };
    let column = app.metric.timeline_column(p).filter(|c| timeline.index(c).is_some()).unwrap_or_else(|| "work_ms".into());
    let Some(values) = timeline.values(&column) else { return };
    let unit = unit_of(&column);
    let phases: Vec<u32> = timeline.values("phase").unwrap_or_default().into_iter().map(|v| v as u32).collect();
    let max_row = (0..values.len()).max_by(|&a, &b| values[a].total_cmp(&values[b]).then(b.cmp(&a))).unwrap_or(0);
    let frame_of = |row: usize| timeline.value(row, "frame").unwrap_or(row as f64);
    let note = format!("{} frames · mean {} · max {} at frame {}", fmt::grouped(values.len() as f64), fmt::short(unit, p.frames_mean(&values)), fmt::short(unit, values.get(max_row).copied().unwrap_or(0.0)), frame_of(max_row));
    let inner = kit::draw_panel(buf, area, kit::panel(theme, &format!("{} a frame", series_label(&column)), &note, false));
    if inner.height < 2 || values.is_empty() {
        return;
    }
    let order = phase_order(p);
    let chart = Rect { height: inner.height - 1, ..inner };
    let axis = kit::FrameAxis::new(values.len(), chart.width as usize);
    let phase_of = |column: usize| axis.row(column).and_then(|row| phases.get(row).copied());
    frame_columns(buf, chart, &axis, &values, |column| phase_color(&order, phase_of(column).unwrap_or(0), theme), theme);
    // the phases under the columns, each named where it starts
    let mut last: Option<u32> = None;
    let mut free = inner.x;
    for column in 0..axis.columns() {
        let Some(phase) = phase_of(column) else { continue };
        if last != Some(phase) {
            let x = (inner.x + column as u16).max(free);
            let line = Line::from(vec![Span::styled("■ ", Style::new().fg(phase_color(&order, phase, theme))), Span::styled(p.phase_name(phase).to_string(), Style::new().fg(theme.ink2))]);
            free = x + line.width() as u16 + 2;
            Paragraph::new(line).render(Rect { x, y: inner.bottom() - 1, width: inner.right().saturating_sub(x), height: 1 }, buf);
            last = Some(phase);
        }
    }
}

/// Values in at most `width` columns, each the largest of its frames (a spike stays visible)
pub fn bucket_max(values: &[f64], width: usize) -> Vec<f64> {
    kit::bucket_max(values, width)
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
