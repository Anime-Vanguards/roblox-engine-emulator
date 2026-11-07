//! Compare: a what-if comparison's variants side by side (`ree profile try`): each variant's figure at each input size
//! in the chosen metric, how it grows with n, the winner of each size, and the selected run's figures, table shapes and
//! hot functions. Enter opens a run's own profile in the other views; D diffs it against the baseline's.

use ratatui_core::{buffer::Buffer, layout::Rect, style::Style};

use super::{
    plot::{Series, log_log},
    widgets::{Col, MARGIN, cell, header, layout, section, select_row, spans, text},
    wrap,
};
use crate::{
    app::{App, View},
    compare::{Compare, MetricInfo, RunInfo},
    fmt,
    keys::Key,
    model::Profile,
    theme::Theme,
};

/// The metric shown: the one picked with m, else the work clock, else the first measured
pub fn metric(app: &App) -> Option<MetricInfo> {
    let compare = app.compare.as_ref()?;
    let measured = compare.measured();
    let picked = app.compare_view.metric.as_ref().and_then(|key| measured.iter().find(|m| &m.key == key));
    picked.or_else(|| measured.iter().find(|m| m.key == "work_ms")).or(measured.first()).map(|m| (*m).clone())
}

/// `m` / `M` in this view: the comparison's metrics
pub fn cycle_metric(app: &mut App, forward: bool) {
    let Some(compare) = app.compare.as_ref() else { return };
    let keys: Vec<String> = compare.measured().iter().map(|m| m.key.clone()).collect();
    if keys.is_empty() {
        return;
    }
    let current = metric(app).map(|m| m.key).unwrap_or_default();
    let at = keys.iter().position(|k| *k == current).unwrap_or(0);
    let next = if forward { (at + 1) % keys.len() } else { (at + keys.len() - 1) % keys.len() };
    app.compare_view.metric = Some(keys[next].clone());
    if let Some(m) = metric(app) {
        app.status = format!("metric: {}{}", m.label, if m.exact { " (exact)" } else { " (measured)" });
    }
}

fn size_label(n: f64) -> String {
    format!("n={}", fmt::compact(n))
}

/// The ratio of a value to the baseline's, oriented so that above 1 is worse
fn worse_ratio(m: &MetricInfo, value: f64, base: f64) -> Option<f64> {
    if base <= 0.0 || value <= 0.0 {
        return None;
    }
    Some(if m.lower_is_better() { value / base } else { base / value })
}

fn selected(app: &App, compare: &Compare) -> (usize, usize) {
    let variant = app.compare_view.list.selected.min(compare.variants.len().saturating_sub(1));
    let last = compare.sizes.len().saturating_sub(1);
    let size = app.compare_view.size.unwrap_or(last).min(last);
    (variant, size)
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let Some(compare) = app.compare.as_ref() else {
        text(buf, area.x, area.y, "no comparison: open a folder ree profile try wrote (it holds compare.json)", area.width, theme.dim());
        return;
    };
    let Some(m) = metric(app) else {
        text(buf, area.x, area.y, "the comparison measured nothing", area.width, theme.dim());
        return;
    };
    let (sel_variant, sel_size) = selected(app, compare);
    let mut y = area.y;
    section(buf, area, y, &compare.title, &compare.about, theme);
    y += 1;
    let sizes: Vec<String> = compare.sizes.iter().map(|n| fmt::compact(*n)).collect();
    let calls = if compare.frames > 1.0 { format!(" · {} calls each", fmt::grouped(compare.frames)) } else { String::new() };
    let line = format!("baseline {} · n {}{calls} · {} ({})", compare.baseline, sizes.join(", "), m.label, if m.exact { "exact" } else { "measured, medians" });
    text(buf, area.x, y, &line, area.width, theme.secondary());
    y += 1;
    for note in compare.notes.iter().take(2) {
        text(buf, area.x, y, &format!("note: {note}"), area.width, theme.dim());
        y += 1;
    }
    y += 1;
    // the table: a row a variant, a column a size
    let largest = compare.sizes.iter().copied().fold(0.0, f64::max);
    let mut cols = vec![Col::left("variant", 22)];
    for n in &compare.sizes {
        cols.push(Col::right(&size_label(*n), 11));
    }
    cols.push(Col::right("scaling", 9));
    cols.push(Col::right(&format!("n={} vs baseline", fmt::compact(largest)), 18));
    cols.push(Col::left("", 1));
    let xs = layout(&mut cols, area.x + MARGIN, area.width - MARGIN);
    header(buf, &cols, &xs, y, theme);
    y += 1;
    let table_rows = (compare.variants.len() as u16).min(area.height / 3).max(1);
    let mut list = app.compare_view.list;
    list.fit(compare.variants.len(), table_rows as usize);
    for (index, variant) in compare.variants.iter().enumerate().skip(list.offset).take(table_rows as usize) {
        let row_y = y + (index - list.offset) as u16;
        let swatch = Style::new().fg(theme.series[index % theme.series.len()]).bg(theme.surface);
        let name = if variant.name == compare.baseline { format!("{} (baseline)", variant.name) } else { variant.name.clone() };
        let x = spans(buf, xs[0], row_y, xs[0] + cols[0].width, &[("■ ", swatch)]);
        text(buf, x, row_y, &name, cols[0].width.saturating_sub(2), theme.text());
        for (s, n) in compare.sizes.iter().enumerate() {
            let col = &cols[1 + s];
            let run = compare.run(&variant.name, *n);
            let value = run.and_then(|r| r.value(&m.key));
            let shown = match (run, value) {
                (Some(r), _) if r.status != "ok" && !r.status.is_empty() => r.status.clone(),
                (_, Some(v)) => m.show(v),
                _ => "-".into(),
            };
            let winner = compare.winner(&m.key, *n).filter(|w| w.winner == variant.name);
            let mark = match winner {
                Some(w) if w.clear => "◆",
                Some(_) => "≈",
                None => " ",
            };
            let failed = run.is_some_and(|r| r.check.as_ref().is_some_and(|c| !c.ok));
            let style = if failed { Style::new().fg(theme.critical).bg(theme.surface) } else { theme.text() };
            cell(buf, col, xs[1 + s], row_y, &format!("{shown}{mark}"), style);
            if s == sel_size && index == sel_variant {
                let width = col.width;
                for dx in 0..width {
                    let c = &mut buf[(xs[1 + s] + dx, row_y)];
                    c.modifier.insert(ratatui_core::style::Modifier::UNDERLINED);
                }
            }
        }
        let c = 1 + compare.sizes.len();
        if let Some(scaling) = compare.scaling(&variant.name, &m.key) {
            if let Some(exponent) = scaling.exponent {
                let style = if exponent >= 1.3 && m.lower_is_better() { Style::new().fg(theme.more).bg(theme.surface) } else { theme.text() };
                cell(buf, &cols[c], xs[c], row_y, &format!("n^{exponent:.2}"), style);
            }
        }
        let base = compare.run(&compare.baseline, largest).and_then(|r| r.value(&m.key));
        let value = compare.run(&variant.name, largest).and_then(|r| r.value(&m.key));
        if variant.name != compare.baseline {
            if let (Some(base), Some(value)) = (base, value) {
                if let Some(ratio) = worse_ratio(&m, value, base) {
                    let (arrow, color) = if ratio > 1.0005 { ("▲ worse", theme.more) } else if ratio < 0.9995 { ("▼ better", theme.less) } else { ("  same", theme.muted) };
                    let times = if ratio >= 1.0 { ratio } else { 1.0 / ratio };
                    spans(buf, xs[c + 1], row_y, xs[c + 1] + cols[c + 1].width, &[(&format!("{times:>6.2}x "), theme.text()), (arrow, Style::new().fg(color).bg(theme.surface))]);
                }
            }
        }
        if index == list.selected {
            select_row(buf, area, row_y, theme);
        }
    }
    app.compare_view.list = list;
    y += table_rows + 1;
    text(buf, area.x, y, "◆ best · ≈ best, within noise · underlined: the run below", area.width, theme.dim());
    y += 2;
    // below: the scaling plot and the selected run
    let below = Rect { x: area.x, y, width: area.width, height: area.bottom().saturating_sub(y) };
    if below.height < 6 {
        return;
    }
    let plot_width = (below.width * 9 / 20).max(30).min(below.width);
    let plot_area = Rect { x: below.x, y: below.y, width: plot_width, height: below.height };
    let detail = Rect { x: below.x + plot_width + 2, y: below.y, width: below.width.saturating_sub(plot_width + 2), height: below.height };
    section(buf, plot_area, plot_area.y, &format!("{} by n", m.label), "log scales", theme);
    let series: Vec<Series> = compare
        .variants
        .iter()
        .enumerate()
        .map(|(index, variant)| Series {
            points: compare.sizes.iter().filter_map(|n| compare.run(&variant.name, *n).and_then(|r| r.value(&m.key)).map(|v| (*n, v))).collect(),
            color: theme.series[index % theme.series.len()],
        })
        .collect();
    let legend_rows = compare.variants.len().min(8) as u16;
    let chart = Rect { x: plot_area.x, y: plot_area.y + 1, width: plot_area.width, height: plot_area.height.saturating_sub(1 + legend_rows) };
    if compare.sizes.len() >= 2 {
        let fmt_y = |v: f64| m.show(v);
        log_log(buf, chart, &series, &|x| fmt::compact(x), &fmt_y, theme);
    } else {
        text(buf, chart.x, chart.y + 1, "one input size: nothing to scale", chart.width, theme.dim());
    }
    // the legend: a swatch in the series' colour, the name in ink
    let mut ly = chart.bottom();
    for (index, variant) in compare.variants.iter().enumerate().take(legend_rows as usize) {
        let swatch = Style::new().fg(theme.series[index % theme.series.len()]).bg(theme.surface);
        let exponent = compare.scaling(&variant.name, &m.key).and_then(|s| s.exponent).map(|e| format!("  n^{e:.2}")).unwrap_or_default();
        spans(buf, plot_area.x + 11, ly, plot_area.right(), &[("━━ ", swatch), (&variant.name, theme.text()), (&exponent, theme.dim())]);
        ly += 1;
    }
    draw_detail(buf, detail, app, compare, &m, sel_variant, sel_size, theme);
}

fn draw_detail(buf: &mut Buffer, area: Rect, _app: &App, compare: &Compare, m: &MetricInfo, variant: usize, size: usize, theme: &Theme) {
    let (Some(info), Some(&n)) = (compare.variants.get(variant), compare.sizes.get(size)) else { return };
    let Some(run) = compare.run(&info.name, n) else {
        section(buf, area, area.y, &format!("{} at n={}", info.name, fmt::grouped(n)), "no run", theme);
        return;
    };
    let base = compare.run(&compare.baseline, n);
    let check = match &run.check {
        Some(c) if c.ok => "check ✓".to_string(),
        Some(c) => format!("check ✗ {}", c.message.clone().unwrap_or_else(|| "a different result".into())),
        None => String::new(),
    };
    let status = if run.status.is_empty() || run.status == "ok" { String::new() } else { format!("{} · ", run.status) };
    section(buf, area, area.y, &format!("{} at n={}", info.name, fmt::grouped(n)), &format!("{status}{check}"), theme);
    let mut y = area.y + 1;
    if let Some(error) = &run.error {
        for chunk in wrap(error, area.width as usize).into_iter().take(2) {
            text(buf, area.x, y, &chunk, area.width, Style::new().fg(theme.critical).bg(theme.surface));
            y += 1;
        }
    }
    // every figure, a call's, with the baseline beside it
    let figures: Vec<(&MetricInfo, f64)> = compare.measured().into_iter().filter_map(|metric| run.value(&metric.key).map(|v| (metric, v))).collect();
    let column_width = 36u16;
    let per_column = ((figures.len() as u16).div_ceil((area.width / column_width).max(1))).max(1);
    let rows_for_figures = per_column.min(area.height.saturating_sub(8).max(3));
    for (index, (metric, value)) in figures.iter().enumerate() {
        let col = index as u16 / rows_for_figures;
        let row = index as u16 % rows_for_figures;
        let x = area.x + col * column_width;
        if x + 10 > area.right() {
            break;
        }
        // (the arrow is the figure's direction against the baseline; red is worse, blue better)
        let base_value = base.and_then(|b| b.value(&metric.key));
        let ratio = base_value.and_then(|b| worse_ratio(metric, *value, b));
        let (arrow, color) = match (ratio, base_value) {
            (Some(r), Some(b)) if info.name != compare.baseline && (r - 1.0).abs() > 0.0005 => (if *value > b { "▲" } else { "▼" }, if r > 1.0 { theme.more } else { theme.less }),
            _ => (" ", theme.muted),
        };
        let label_style = if metric.key == m.key { theme.bold() } else { theme.dim() };
        spans(buf, x, y + row, (x + column_width - 1).min(area.right()), &[
            (&format!("{:<17}", fmt::fit(&metric.label, 17)), label_style),
            (&format!("{:>11}", metric.show(*value)), theme.text()),
            (&format!(" {arrow}"), Style::new().fg(color).bg(theme.surface)),
        ]);
    }
    y += rows_for_figures + 1;
    // table shapes: what the run's result keeps, by allocation site
    if let Some(tables) = &run.tables {
        if y < area.bottom() {
            section(buf, area, y, "Tables kept", &format!("{} · {}", fmt::grouped(tables.count), fmt::bytes(tables.bytes)), theme);
            y += 1;
        }
        for site in tables.sites.iter().take(3) {
            if y >= area.bottom() {
                break;
            }
            let each = if site.count > 0.0 { site.bytes / site.count } else { 0.0 };
            let shape = shape_text(site);
            let line = format!("{} × {shape} · {} each · {}", fmt::grouped(site.count), fmt::bytes(each), site.site);
            text(buf, area.x + 1, y, &line, area.width - 1, theme.text());
            y += 1;
            if site.rehashes > 0.0 && y < area.bottom() {
                text(buf, area.x + 3, y, &format!("grew {} times, moving {} keys", fmt::grouped(site.rehashes), fmt::grouped(site.rehash_keys)), area.width - 3, theme.dim());
                y += 1;
            }
        }
    }
    // the hot functions of the run phase
    if !run.hot.is_empty() && y + 1 < area.bottom() {
        y += 1;
        section(buf, area, y, "Hottest", "", theme);
        y += 1;
        for hot in run.hot.iter().take((area.bottom() - y) as usize) {
            text(buf, area.x + 1, y, &format!("{:>10}  {:>8} insns  {}  {}:{}", fmt::ms(hot.work_ms), fmt::compact(hot.insns), hot.name, hot.module, hot.line), area.width - 1, theme.secondary());
            y += 1;
        }
    }
}

/// `{hash 2, array 0} slots, 1 level`: a site's typical table
pub fn shape_text(site: &crate::compare::TableSite) -> String {
    let part = |name: &str, p: &crate::compare::Part| -> Option<String> {
        if p.max <= 0.0 {
            return None;
        }
        Some(if p.min == p.max { format!("{name} {}", fmt::grouped(p.max)) } else { format!("{name} {}-{} (median {})", fmt::grouped(p.min), fmt::grouped(p.max), fmt::grouped(p.median)) })
    };
    let parts: Vec<String> = [part("hash", &site.hash), part("array", &site.array)].into_iter().flatten().collect();
    let slots = if parts.is_empty() { "empty".to_string() } else { format!("{{{}}} slots", parts.join(", ")) };
    let levels = if site.levels.max > 1.0 { format!(", {} levels deep", fmt::grouped(site.levels.max)) } else { String::new() };
    format!("{slots}{levels}")
}

fn open_profile(compare: &Compare, run: &RunInfo) -> Result<Profile, String> {
    let dir = compare.profile_dir(run).ok_or_else(|| format!("{} at n={}: no profile folder", run.variant, fmt::compact(run.size)))?;
    let mut profile = Profile::load(&dir)?;
    profile.label = Some(format!("{} n={}", run.variant, fmt::compact(run.size)));
    Ok(profile)
}

pub fn key(app: &mut App, key: Key) {
    let Some(compare) = app.compare.as_ref() else { return };
    let variants = compare.variants.len();
    let sizes = compare.sizes.len();
    if app.compare_view.list.key(key, variants, app.page) {
        return;
    }
    let (variant, size) = selected(app, compare);
    match key {
        Key::Left => app.compare_view.size = Some(size.saturating_sub(1)),
        Key::Right => app.compare_view.size = Some((size + 1).min(sizes.saturating_sub(1))),
        Key::Enter => {
            let compare = app.compare.as_ref().unwrap();
            let Some(run) = compare.variants.get(variant).and_then(|v| compare.sizes.get(size).and_then(|n| compare.run(&v.name, *n))) else { return };
            match open_profile(compare, run) {
                Ok(profile) => {
                    let label = format!("{} at n={}", run.variant, fmt::compact(run.size));
                    app.open_profiles(vec![profile]);
                    app.set_view(View::Overview);
                    app.status = format!("{label} · c returns to Compare");
                }
                Err(error) => app.status = error,
            }
        }
        Key::Char('D') => {
            let compare = app.compare.as_ref().unwrap();
            let Some(&n) = compare.sizes.get(size) else { return };
            let Some(info) = compare.variants.get(variant) else { return };
            if info.name == compare.baseline {
                app.status = "the baseline: pick another variant to diff against it".into();
                return;
            }
            let (Some(before), Some(after)) = (compare.run(&compare.baseline, n), compare.run(&info.name, n)) else { return };
            match (open_profile(compare, before), open_profile(compare, after)) {
                (Ok(b), Ok(a)) => {
                    let label = format!("{} → {} at n={}", compare.baseline, info.name, fmt::compact(n));
                    app.open_profiles(vec![b, a]);
                    app.set_view(View::Diff);
                    app.status = format!("{label} · c returns to Compare");
                }
                (Err(error), _) | (_, Err(error)) => app.status = error,
            }
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let compare = app.compare.as_ref()?;
    let (variant, size) = selected(app, compare);
    let run = compare.variants.get(variant).and_then(|v| compare.sizes.get(size).and_then(|n| compare.run(&v.name, *n)))?;
    compare.profile_dir(run).map(|dir| dir.display().to_string())
}
