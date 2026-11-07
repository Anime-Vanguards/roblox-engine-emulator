//! Allocations: what the run allocated by site (the Luau line), with each site's share, object size, rate a frame in
//! the steady state and types; the run's totals by type; the selected site's stacks.

use ratatui_core::{buffer::Buffer, layout::Rect};

use super::widgets::{Col, MARGIN, bar, cell, header, layout, section, select_row, text};
use crate::{
    app::{AllocSort, App, Focus, View},
    fmt,
    keys::Key,
    metric::Metric,
    model::Kind,
    sites::{AllocSite, ENGINE, site_label},
    theme::Theme,
};

/// The sites in the order shown
pub fn order(app: &App, sites: &[AllocSite]) -> Vec<usize> {
    let p = &app.profiles[app.shown];
    let steady = p.steady_phase();
    let mut order: Vec<usize> = (0..sites.len()).filter(|&i| !app.scripts_only || (sites[i].func != ENGINE && p.funcs[sites[i].func as usize].kind == Kind::Script)).collect();
    let key = |site: &AllocSite| -> f64 {
        match app.alloc.sort {
            AllocSort::Bytes => {
                if app.metric == Metric::AllocCount {
                    site.count
                } else {
                    site.bytes
                }
            }
            AllocSort::Count => site.count,
            AllocSort::SteadyBytes => steady.map_or(0.0, |(phase, frames)| site.per_frame(phase, frames).0),
            AllocSort::Size => {
                if site.count > 0.0 {
                    site.bytes / site.count
                } else {
                    0.0
                }
            }
        }
    };
    order.sort_by(|&a, &b| key(&sites[b]).total_cmp(&key(&sites[a])).then(a.cmp(&b)));
    order
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let sites = app.sites();
    let order = order(app, &sites);
    let mut list = app.alloc.list;
    let p = &app.profiles[app.shown];
    let Some(alloc) = &p.raw.alloc else {
        text(buf, area.x, area.y, "no allocations in this profile (--profile alloc)", area.width, theme.dim());
        return;
    };
    let steady = p.steady_phase();
    let by_count = app.metric == Metric::AllocCount;
    let mut y = area.y;
    // totals
    let mut line = format!("{}, {} objects", fmt::bytes(alloc.bytes), fmt::grouped(alloc.objects));
    if let (Some((phase, frames)), Some(timeline)) = (steady, &p.raw.timeline) {
        let phases = timeline.values("phase").unwrap_or_default();
        let bytes = timeline.values("allocated_bytes").unwrap_or_default();
        let counts = timeline.values("allocations").unwrap_or_default();
        let rows: Vec<usize> = (0..phases.len()).filter(|&i| phases[i] as u32 == phase).collect();
        if !rows.is_empty() {
            let mean = |values: &[f64]| rows.iter().map(|&i| values.get(i).copied().unwrap_or(0.0)).sum::<f64>() / rows.len() as f64;
            line.push_str(&format!(" · {} ({} frames): {}, {} objects a frame", p.phase_name(phase), fmt::grouped(frames as f64), fmt::bytes(mean(&bytes)), fmt::grouped(mean(&counts))));
        }
    }
    if alloc.step > 1.0 {
        line.push_str(&format!(" · stacks sampled every {} bytes", fmt::grouped(alloc.step)));
    }
    text(buf, area.x, y, &line, area.width, theme.secondary());
    y += 1;
    // by type: the largest groups
    let mut groups: Vec<(String, f64, f64)> = Vec::new();
    for row in &alloc.by_type {
        let group = if row.group.is_empty() { row.ty.clone() } else { row.group.clone() };
        match groups.iter_mut().find(|g| g.0 == group) {
            Some(g) => {
                g.1 += row.bytes;
                g.2 += row.count;
            }
            None => groups.push((group, row.bytes, row.count)),
        }
    }
    groups.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    let parts: Vec<String> = groups.iter().take(6).map(|(name, bytes, _)| format!("{name} {}", fmt::share(*bytes, alloc.bytes))).collect();
    text(buf, area.x, y, &parts.join(" · "), area.width, theme.dim());
    y += 2;
    // the sites
    let detail_height = 5u16;
    let sort = match app.alloc.sort {
        AllocSort::Bytes => {
            if by_count {
                "objects"
            } else {
                "bytes"
            }
        }
        AllocSort::Count => "objects",
        AllocSort::SteadyBytes => "bytes a frame",
        AllocSort::Size => "object size",
    };
    section(buf, area, y, "Sites", &format!("{} · by {sort}", fmt::grouped(order.len() as f64)), theme);
    y += 1;
    let steady_label = steady.map(|(phase, _)| format!("a frame, {}", p.phase_name(phase))).unwrap_or_else(|| "a frame".into());
    let mut cols = vec![
        Col::left("", 28),
        Col::right("bytes", 10),
        Col::right("objects", 9),
        Col::right("size", 7),
        Col::right(&fmt::fit(&steady_label, 18), 18),
        Col::left("types", 26),
        Col::left("", 12),
    ];
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    header(buf, &cols, &xs, y, theme);
    y += 1;
    let height = area.bottom().saturating_sub(y + detail_height) as usize;
    list.fit(order.len(), height);
    let total = if by_count { alloc.objects } else { alloc.bytes };
    let max = order.first().map_or(0.0, |&i| if by_count { sites[i].count } else { sites[i].bytes });
    for (screen, &index) in order.iter().enumerate().skip(list.offset).take(height) {
        let site = &sites[index];
        let row_y = y + (screen - list.offset) as u16;
        let value = if by_count { site.count } else { site.bytes };
        let share = if total > 0.0 { value / total } else { 0.0 };
        let style = theme.row(share);
        cell(buf, &cols[0], xs[0], row_y, &site_label(p, site), style);
        cell(buf, &cols[1], xs[1], row_y, &fmt::bytes(site.bytes), style);
        cell(buf, &cols[2], xs[2], row_y, &fmt::compact(site.count), style);
        cell(buf, &cols[3], xs[3], row_y, &if site.count > 0.0 { fmt::bytes(site.bytes / site.count) } else { "-".into() }, theme.secondary());
        if let Some((phase, frames)) = steady {
            let (bytes, count) = site.per_frame(phase, frames);
            if count > 0.0 {
                cell(buf, &cols[4], xs[4], row_y, &format!("{}, {}", fmt::bytes(bytes), fmt::compact(count)), style);
            }
        }
        let types: Vec<String> = site.types.iter().take(2).map(|(ty, bytes, _)| format!("{ty} {}", fmt::share(*bytes, site.bytes.max(1.0)))).collect();
        cell(buf, &cols[5], xs[5], row_y, &types.join(", "), theme.secondary());
        bar(buf, xs[6], row_y, cols[6].width, if max > 0.0 { value / max } else { 0.0 }, theme.heat_of(share), theme);
        if screen == list.selected {
            select_row(buf, area, row_y, theme);
        }
    }
    super::widgets::scroll_note(buf, area, y - 2, list.offset, height, order.len(), theme);
    app.alloc.list = list;
    // the selected site's stacks
    let y = area.bottom() - detail_height;
    let Some(site) = order.get(list.selected).map(|&i| &sites[i]) else { return };
    section(buf, area, y, "Stacks", &site_label(p, site), theme);
    if site.stacks.is_empty() {
        text(buf, area.x, y + 1, "not among the 500 stacks the profile kept", area.width, theme.dim());
    }
    for (row, &stack_index) in site.stacks.iter().take(detail_height as usize - 1).enumerate() {
        let stack = &alloc.by_stack[stack_index];
        let frames: Vec<String> = stack.frames.iter().flatten().filter_map(|(id, line)| {
            let func = p.func_of_id(*id)?;
            let f = &p.funcs[func];
            Some(if f.module.is_empty() { f.name.clone() } else { format!("{}:{line}", f.short) })
        }).take(8).collect();
        text(buf, area.x, y + 1 + row as u16, &format!("{:>10} {:>8} {:<16} {}", fmt::bytes(stack.bytes), fmt::compact(stack.count), fmt::fit(&stack.ty, 16), frames.join(" < ")), area.width, theme.secondary());
    }
}

pub fn key(app: &mut App, key: Key) {
    let sites = app.sites();
    let order = order(app, &sites);
    if app.alloc.list.key(key, order.len(), app.page) {
        if let Some(site) = order.get(app.alloc.list.selected).map(|&i| &sites[i]) {
            if site.func != ENGINE {
                app.focus = Some(Focus { func: site.func, line: (site.line >= 0).then_some(site.line) });
            }
        }
        return;
    }
    match key {
        Key::Char('s') => {
            app.alloc.sort = match app.alloc.sort {
                AllocSort::Bytes => AllocSort::Count,
                AllocSort::Count => AllocSort::SteadyBytes,
                AllocSort::SteadyBytes => AllocSort::Size,
                AllocSort::Size => AllocSort::Bytes,
            };
            app.alloc.list.selected = 0;
        }
        Key::Enter => {
            if let Some(site) = order.get(app.alloc.list.selected).map(|&i| &sites[i]) {
                if site.func != ENGINE {
                    let (func, line) = (site.func, site.line);
                    app.focus = Some(Focus { func, line: (line >= 0).then_some(line) });
                    app.source.module = None;
                    app.source.line = if line >= 0 { line } else { app.profile().funcs[func as usize].line };
                    app.set_view(View::Source);
                }
            }
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let sites = app.sites();
    let order = order(app, &sites);
    let site = order.get(app.alloc.list.selected).map(|&i| &sites[i])?;
    if site.func == ENGINE {
        return None;
    }
    super::calls::place_text(app, site.func, (site.line >= 0).then_some(site.line))
}

pub fn find(app: &mut App, query: &str, forward: bool, from_start: bool) -> bool {
    let sites = app.sites();
    let order = order(app, &sites);
    let p = &app.profiles[app.shown];
    match super::next_match(order.len(), app.alloc.list.selected, forward, from_start, |i| site_label(p, &sites[order[i]]).to_lowercase().contains(query)) {
        Some(found) => {
            app.alloc.list.selected = found;
            true
        }
        None => false,
    }
}
