//! Caches: Luau's inline caches by kind and by call site (hit, repredicted, slow path), and ree's own caches.

use ratatui_core::{buffer::Buffer, layout::Rect};

use super::widgets::{Col, MARGIN, bar, cell, header, layout, section, select_row, text};
use crate::{
    app::{App, CacheSort, Focus, View},
    fmt,
    keys::Key,
    model::Kind,
    theme::Theme,
};

/// The call sites in the order shown (indexes into `luau_caches.sites`)
pub fn order(app: &App) -> Vec<usize> {
    let p = &app.profiles[app.shown];
    let Some(caches) = &p.raw.luau_caches else { return Vec::new() };
    let mut order: Vec<usize> = (0..caches.sites.len())
        .filter(|&i| {
            let site = &caches.sites[i];
            let kind = p.func_of_id(site.function).map(|f| p.funcs[f].kind);
            // (the miss rankings leave out sites that never missed; the rate's, sites with under 100 lookups)
            let ranked = match app.caches.sort {
                CacheSort::Misses => site.missed() > 0.0,
                CacheSort::Rate => site.missed() > 0.0 && site.runs() >= 100.0,
                CacheSort::Runs => true,
            };
            ranked && kind != Some(Kind::Engine) && (!app.scripts_only || kind == Some(Kind::Script))
        })
        .collect();
    let key = |i: usize| -> f64 {
        let site = &caches.sites[i];
        match app.caches.sort {
            CacheSort::Misses => site.missed(),
            CacheSort::Rate => {
                if site.runs() > 0.0 {
                    site.missed() / site.runs()
                } else {
                    0.0
                }
            }
            CacheSort::Runs => site.runs(),
        }
    };
    order.sort_by(|&a, &b| key(b).total_cmp(&key(a)).then(a.cmp(&b)));
    order
}

fn site_label(app: &App, index: usize) -> String {
    let p = &app.profiles[app.shown];
    let Some(site) = p.raw.luau_caches.as_ref().and_then(|c| c.sites.get(index)) else { return String::new() };
    match p.func_of_id(site.function) {
        Some(func) => {
            let f = &p.funcs[func];
            format!("{}  {}:{}", f.name, f.short, site.line)
        }
        None => format!("function {} line {}", site.function, site.line),
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let order = order(app);
    let mut list = app.caches.list;
    let p = &app.profiles[app.shown];
    let Some(caches) = &p.raw.luau_caches else {
        text(buf, area.x, area.y, "no cache counts in this profile (--profile caches)", area.width, theme.dim());
        return;
    };
    let mut y = area.y;
    section(buf, area, y, "Luau inline caches", "miss: predicted wrong · slow: a metatable or an unsafe environment · other: not a table", theme);
    y += 1;
    let mut cols = vec![Col::left("cache", 20), Col::right("hit rate", 9), Col::left("", 16), Col::right("runs", 11), Col::right("misses", 10), Col::right("slow", 10), Col::right("other", 10)];
    let xs = layout(&mut cols, area.x + MARGIN, area.width.min(110) - MARGIN);
    header(buf, &cols, &xs, y, theme);
    y += 1;
    for kind in &caches.by_kind {
        let table = kind.hits + kind.misses + kind.slow;
        let rate = if table > 0.0 { kind.hits / table } else { 0.0 };
        cell(buf, &cols[0], xs[0], y, &kind.kind, theme.text());
        cell(buf, &cols[1], xs[1], y, &if table > 0.0 { format!("{:.1}%", rate * 100.0) } else { "-".into() }, theme.text());
        if table > 0.0 {
            bar(buf, xs[2], y, cols[2].width, rate, theme.ink2, theme);
        }
        cell(buf, &cols[3], xs[3], y, &fmt::grouped(kind.hits + kind.misses + kind.slow + kind.other), theme.secondary());
        cell(buf, &cols[4], xs[4], y, &fmt::grouped(kind.misses), theme.secondary());
        cell(buf, &cols[5], xs[5], y, &fmt::grouped(kind.slow), theme.secondary());
        cell(buf, &cols[6], xs[6], y, &fmt::grouped(kind.other), theme.secondary());
        y += 1;
    }
    y += 1;
    // ree's caches take the bottom
    let ree: Vec<&crate::model::ReeCache> = p.raw.ree_caches.iter().flatten().filter(|c| c.hits + c.misses + c.evictions > 0.0).collect();
    let ree_height = if ree.is_empty() { 0 } else { (ree.len() as u16 + 2).min(area.height / 3) };
    let sort = match app.caches.sort {
        CacheSort::Misses => "by misses",
        CacheSort::Rate => "by miss rate, 100 lookups or more",
        CacheSort::Runs => "by lookups",
    };
    let all_missed: f64 = caches.sites.iter().map(|s| s.missed()).sum();
    section(buf, area, y, "Call sites", &format!("{} · {sort}", fmt::grouped(order.len() as f64)), theme);
    y += 1;
    let mut cols = vec![Col::left("op", 11), Col::left("", 26), Col::right("lookups", 11), Col::right("hits", 11), Col::right("misses", 10), Col::right("slow", 10), Col::right("missed", 7), Col::left("", 12)];
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    header(buf, &cols, &xs, y, theme);
    y += 1;
    let height = area.bottom().saturating_sub(y + ree_height) as usize;
    list.fit(order.len(), height);
    let max = order.first().map_or(0.0, |&i| caches.sites[i].missed());
    for (screen, &index) in order.iter().enumerate().skip(list.offset).take(height) {
        let site = &caches.sites[index];
        let row_y = y + (screen - list.offset) as u16;
        let missed = site.missed();
        let share = if all_missed > 0.0 { missed / all_missed } else { 0.0 };
        let style = theme.row(share);
        cell(buf, &cols[0], xs[0], row_y, &site.op, theme.secondary());
        cell(buf, &cols[1], xs[1], row_y, &site_label(app, index), style);
        cell(buf, &cols[2], xs[2], row_y, &fmt::grouped(site.runs()), theme.secondary());
        cell(buf, &cols[3], xs[3], row_y, &fmt::grouped(site.hits), theme.secondary());
        cell(buf, &cols[4], xs[4], row_y, &fmt::grouped(site.misses), style);
        cell(buf, &cols[5], xs[5], row_y, &fmt::grouped(site.slow), style);
        cell(buf, &cols[6], xs[6], row_y, &fmt::share(missed, site.runs()), style);
        bar(buf, xs[7], row_y, cols[7].width, if max > 0.0 { missed / max } else { 0.0 }, theme.heat_of(share), theme);
        if screen == list.selected {
            select_row(buf, area, row_y, theme);
        }
    }
    if order.is_empty() {
        text(buf, area.x + MARGIN, y, if app.caches.sort == CacheSort::Runs { "none" } else { "no misses (s: by lookups)" }, area.width, theme.dim());
    }
    app.caches.list = list;
    if ree_height == 0 {
        return;
    }
    let y = area.bottom() - ree_height;
    section(buf, area, y, "ree's caches", "the emulator's own", theme);
    let mut cols = vec![Col::left("cache", 22), Col::right("hit rate", 9), Col::right("lookups", 12), Col::right("misses", 10), Col::right("evicted", 10), Col::left("", 20)];
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    for (i, cache) in ree.iter().take(ree_height as usize - 1).enumerate() {
        let row_y = y + 1 + i as u16;
        let lookups = cache.hits + cache.misses;
        cell(buf, &cols[0], xs[0], row_y, &cache.name, theme.text());
        cell(buf, &cols[1], xs[1], row_y, &fmt::share(cache.hits, lookups), theme.text());
        cell(buf, &cols[2], xs[2], row_y, &fmt::grouped(lookups), theme.secondary());
        cell(buf, &cols[3], xs[3], row_y, &fmt::grouped(cache.misses), theme.secondary());
        cell(buf, &cols[4], xs[4], row_y, &fmt::grouped(cache.evictions), theme.secondary());
        cell(buf, &cols[5], xs[5], row_y, &cache.about, theme.dim());
    }
}

pub fn key(app: &mut App, key: Key) {
    let order = order(app);
    if app.caches.list.key(key, order.len(), app.page) {
        return;
    }
    match key {
        Key::Char('s') => {
            app.caches.sort = match app.caches.sort {
                CacheSort::Misses => CacheSort::Rate,
                CacheSort::Rate => CacheSort::Runs,
                CacheSort::Runs => CacheSort::Misses,
            };
            app.caches.list.selected = 0;
        }
        Key::Enter => {
            let p = &app.profiles[app.shown];
            let Some(site) = order.get(app.caches.list.selected).and_then(|&i| p.raw.luau_caches.as_ref().map(|c| c.sites[i].clone())) else { return };
            if let Some(func) = p.func_of_id(site.function) {
                app.focus = Some(Focus { func: func as u32, line: Some(site.line) });
                app.source.module = None;
                app.source.line = site.line;
                app.set_view(View::Source);
            }
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let order = order(app);
    let p = &app.profiles[app.shown];
    let site = order.get(app.caches.list.selected).and_then(|&i| p.raw.luau_caches.as_ref().map(|c| &c.sites[i]))?;
    let func = p.func_of_id(site.function)?;
    super::calls::place_text(app, func as u32, Some(site.line))
}

pub fn find(app: &mut App, query: &str, forward: bool, from_start: bool) -> bool {
    let order = order(app);
    match super::next_match(order.len(), app.caches.list.selected, forward, from_start, |i| site_label(app, order[i]).to_lowercase().contains(query)) {
        Some(found) => {
            app.caches.list.selected = found;
            true
        }
        None => false,
    }
}
