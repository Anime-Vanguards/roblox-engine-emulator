//! Timeline: a column a frame (bucketed to the screen's width, each column the worst of its frames) in the metric's
//! per-frame figure, coloured by phase; pick a frame to see all of its figures beside its phase's median.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::widgets::{columns, section, spans, text};
use crate::{app::App, fmt, keys::Key, metric::Unit, model::Profile, theme::Theme};

/// The series a timeline can show: the timeline's columns, then the hardware counters' a frame
pub fn series(p: &Profile) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    if let Some(timeline) = &p.raw.timeline {
        for column in &timeline.columns {
            if !matches!(column.as_str(), "frame" | "seconds" | "phase") {
                out.push(column.clone());
            }
        }
    }
    if let Some(counters) = p.raw.counters.as_ref().filter(|c| c.error.is_none()) {
        for name in &counters.names {
            out.push(format!("hw:{name}"));
        }
    }
    out
}

/// A series' values, row by row, with each row's frame and phase
pub fn values(p: &Profile, series: &str) -> Option<(Vec<f64>, Vec<f64>, Vec<u32>)> {
    if let Some(name) = series.strip_prefix("hw:") {
        let frames = &p.raw.counters.as_ref()?.frames;
        let values = frames.values(name)?;
        let numbers = frames.values("frame").unwrap_or_default();
        // (the counters' rows name their phase)
        let phase_index = frames.index("phase");
        let phases = frames.rows.iter().map(|row| phase_index.and_then(|i| row.get(i)).and_then(|v| v.as_str().map(|name| p.raw.phases.iter().position(|ph| ph == name).unwrap_or(0) as u32).or_else(|| v.as_u64().map(|n| n as u32))).unwrap_or(0)).collect();
        return Some((values, numbers, phases));
    }
    let timeline = p.raw.timeline.as_ref()?;
    let values = timeline.values(series)?;
    Some((values, timeline.values("frame").unwrap_or_default(), timeline.values("phase").unwrap_or_default().into_iter().map(|v| v as u32).collect()))
}

fn unit_of(series: &str) -> Unit {
    if series.ends_with("_ms") {
        Unit::Ms
    } else if series.contains("bytes") || series == "gc_freed" {
        Unit::Bytes
    } else {
        Unit::Count
    }
}

/// The series shown: one picked with [ ], else the metric's, else work
pub fn current(app: &App) -> Option<String> {
    let p = &app.profiles[app.shown];
    let all = series(p);
    if let Some(column) = app.timeline.column.as_ref().filter(|c| all.contains(c)) {
        return Some(column.clone());
    }
    let metric = app.metric.timeline_column(p).map(|c| if matches!(app.metric, crate::metric::Metric::Counter(_)) { format!("hw:{c}") } else { c });
    metric.filter(|c| all.contains(c)).or_else(|| all.iter().find(|c| *c == "work_ms").cloned()).or_else(|| all.first().cloned())
}

fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(f64::total_cmp);
    sorted[sorted.len() / 2]
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let Some(name) = current(app) else {
        text(buf, area.x, area.y, "no timeline in this profile (--profile timeline)", area.width, theme.dim());
        return;
    };
    let p = &app.profiles[app.shown];
    let Some((all, frames, phases)) = values(p, &name) else { return };
    if all.is_empty() {
        return;
    }
    let (from, to) = app.timeline.range.map(|(a, b)| (a.min(all.len() - 1), b.min(all.len()))).unwrap_or((0, all.len()));
    let shown = &all[from..to];
    let selected = app.timeline.frame.unwrap_or_else(|| (0..all.len()).max_by(|&a, &b| all[a].total_cmp(&all[b]).then(b.cmp(&a))).unwrap_or(0)).min(all.len() - 1);
    let unit = unit_of(&name);
    let frame_at = |row: usize| frames.get(row).copied().unwrap_or(row as f64);
    let range = if app.timeline.range.is_some() { format!("frames {}-{} of {}", frame_at(from), frame_at(to - 1), fmt::grouped(all.len() as f64)) } else { format!("{} frames", fmt::grouped(all.len() as f64)) };
    section(buf, area, area.y, &format!("{} a frame", super::overview::series_label(&name)), &range, theme);
    let chart_height = (area.height / 2).clamp(6, 18);
    let chart = Rect { x: area.x + 11, y: area.y + 1, width: area.width - 11, height: chart_height };
    let width = chart.width as usize;
    let buckets = super::overview::bucket_max(shown, width);
    let per = shown.len() as f64 / buckets.len().max(1) as f64;
    let max = buckets.iter().copied().fold(0.0, f64::max);
    // the axis: the maximum at the top, zero at the bottom
    text(buf, area.x, chart.y, &fmt::right(&fmt::short(unit, max), 10), 10, theme.dim());
    text(buf, area.x, chart.bottom() - 1, &fmt::right(&fmt::short(unit, 0.0), 10), 10, theme.dim());
    let bucket_of = |row: usize| (((row.saturating_sub(from)) as f64 / per) as usize).min(buckets.len().saturating_sub(1));
    let selected_bucket = (selected >= from && selected < to).then(|| bucket_of(selected));
    let phase_of_bucket = |b: usize| phases.get(from + ((b as f64 * per) as usize).min(shown.len() - 1)).copied().unwrap_or(0);
    // (a phase's colour: the categorical slots in the order the phases first appear)
    let mut order: Vec<u32> = Vec::new();
    for &phase in phases.iter() {
        if !order.contains(&phase) {
            order.push(phase);
        }
    }
    let color_of = |phase: u32| theme.series[order.iter().position(|p| *p == phase).unwrap_or(0) % theme.series.len()];
    columns(buf, chart, &buckets, max, |b| if Some(b) == selected_bucket { theme.ink } else { color_of(phase_of_bucket(b)) }, theme);
    // the cursor, the frame numbers and the phases under the chart
    let y = chart.bottom();
    if let Some(b) = selected_bucket {
        text(buf, chart.x + b as u16, y, "▲", 1, theme.bold());
    }
    let ticks = 6.min(buckets.len());
    let mut last_end = chart.x;
    for tick in 0..ticks {
        let b = if ticks <= 1 { 0 } else { tick * (buckets.len() - 1) / (ticks - 1) };
        let label = format!("{}", frame_at(from + ((b as f64 * per) as usize).min(shown.len() - 1)));
        let width = label.chars().count() as u16;
        let x = (chart.x + b as u16).saturating_sub(if tick + 1 == ticks { width - 1 } else { 0 }).max(last_end);
        if Some(b) != selected_bucket && x + width <= chart.right() {
            text(buf, x, y, &label, width, theme.dim());
            last_end = x + width + 1;
        }
    }
    let mut last: Option<u32> = None;
    for b in 0..buckets.len() {
        let phase = phase_of_bucket(b);
        if last != Some(phase) {
            let x = chart.x + b as u16;
            spans(buf, x, y + 1, chart.right(), &[("▏", Style::new().fg(color_of(phase)).bg(theme.surface)), (p.phase_name(phase), theme.secondary())]);
            last = Some(phase);
        }
    }
    // the selected frame beside its phase's median frame
    let y = y + 3;
    let phase = phases.get(selected).copied().unwrap_or(0);
    let same_phase: Vec<usize> = (0..all.len()).filter(|&i| phases.get(i).copied().unwrap_or(0) == phase).collect();
    section(buf, area, y, &format!("Frame {}", frame_at(selected)), &format!("{} · beside its median frame", p.phase_name(phase)), theme);
    let mut rows: Vec<(String, f64, f64, Unit)> = Vec::new();
    for column in series(p) {
        let Some((values, _, _)) = values(p, &column) else { continue };
        let Some(&value) = values.get(selected) else { continue };
        let typical = median(&same_phase.iter().filter_map(|&i| values.get(i).copied()).collect::<Vec<_>>());
        rows.push((super::overview::series_label(&column), value, typical, unit_of(&column)));
    }
    let col_width = 52u16;
    let per_column = area.bottom().saturating_sub(y + 2).max(1) as usize;
    for column_index in 0..rows.len().div_ceil(per_column) {
        let x = area.x + column_index as u16 * col_width;
        if x + col_width > area.right() + 2 {
            break;
        }
        spans(buf, x, y + 1, x + col_width - 1, &[(&format!("{:<17}{:>11}  {:>11} {:>6}", "", "frame", "median", ""), theme.dim())]);
    }
    for (index, (column, value, typical, unit)) in rows.iter().enumerate() {
        let x = area.x + (index / per_column) as u16 * col_width;
        let row_y = y + 2 + (index % per_column) as u16;
        if x + col_width > area.right() + 2 || row_y >= area.bottom() {
            continue;
        }
        let ratio = if *typical > 0.0 { value / typical } else if *value > 0.0 { f64::INFINITY } else { 1.0 };
        let mark = if ratio >= 2.0 && *value > 0.0 { ("▲", Style::new().fg(theme.more).bg(theme.surface)) } else if ratio <= 0.5 && *typical > 0.0 { ("▼", Style::new().fg(theme.less).bg(theme.surface)) } else { (" ", theme.dim()) };
        let ratio_text = if ratio.is_finite() { format!("{ratio:.1}x") } else { "new".into() };
        spans(buf, x, row_y, x + col_width - 1, &[
            (&format!("{:<17}", fmt::fit(column, 16)), theme.dim()),
            (&format!("{:>11}", fmt::short(*unit, *value)), theme.text()),
            (mark.0, mark.1),
            (&format!(" {:>11} {:>6}", fmt::short(*unit, *typical), ratio_text), theme.secondary()),
        ]);
    }
}

pub fn key(app: &mut App, key: Key) {
    let Some(name) = current(app) else { return };
    let p = &app.profiles[app.shown];
    let Some((all, _, _)) = values(p, &name) else { return };
    if all.is_empty() {
        return;
    }
    let len = all.len();
    let selected = app.timeline.frame.unwrap_or_else(|| (0..len).max_by(|&a, &b| all[a].total_cmp(&all[b]).then(b.cmp(&a))).unwrap_or(0));
    let series = series(p);
    match key {
        Key::Left => app.timeline.frame = Some(selected.saturating_sub(1)),
        Key::Right => app.timeline.frame = Some((selected + 1).min(len - 1)),
        Key::PageUp | Key::Up => app.timeline.frame = Some(selected.saturating_sub(10)),
        Key::PageDown | Key::Down => app.timeline.frame = Some((selected + 10).min(len - 1)),
        Key::Home => app.timeline.frame = Some(0),
        Key::End => app.timeline.frame = Some(len - 1),
        Key::Enter => {
            // zoom to the frames around the selected one, a column each
            let window = (app.page * 2).max(60).min(len);
            let from = selected.saturating_sub(window / 2).min(len - window);
            app.timeline.range = Some((from, from + window));
        }
        Key::Esc | Key::Backspace => app.timeline.range = None,
        Key::Char(']') | Key::Char('[') => {
            if let Some(at) = series.iter().position(|s| *s == name) {
                let next = if key == Key::Char(']') { (at + 1) % series.len() } else { (at + series.len() - 1) % series.len() };
                app.timeline.column = Some(series[next].clone());
                app.status = format!("series: {}", series[next].replace('_', " "));
            }
        }
        Key::Char('s') => {
            // the worst frame of the series
            app.timeline.frame = (0..len).max_by(|&a, &b| all[a].total_cmp(&all[b]).then(b.cmp(&a)));
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let name = current(app)?;
    let (all, frames, _) = values(app.profile(), &name)?;
    let selected = app.timeline.frame.unwrap_or(0).min(all.len().saturating_sub(1));
    Some(format!("frame {}", frames.get(selected).copied().unwrap_or(0.0)))
}
