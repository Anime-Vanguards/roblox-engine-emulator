//! Flame (an icicle: the run at the top, callees below): each node as wide as its total in the metric, coloured by its
//! own share of the run (heat). Enter zooms into a node, esc zooms back out; arrows walk along a row, up to the caller
//! and down to the heaviest callee.

use ratatui_core::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
};

use super::widgets::{fill, section, spans, text};
use crate::{
    app::{App, Focus},
    fmt,
    keys::Key,
    model::Profile,
    theme::Theme,
    tree::{Frame, ROOT, Tree},
};

/// A node's place on the screen: [x0, x1) columns at its depth below the zoomed root
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub x0: u16,
    pub x1: u16,
    pub node: u32,
}

/// The rows of the icicle below `root`, `width` columns wide (nodes narrower than a column are left out)
pub fn layout(tree: &Tree, root: u32, width: u16, by_name: bool, name: &dyn Fn(u32) -> String) -> Vec<Vec<Span>> {
    let mut rows: Vec<Vec<Span>> = Vec::new();
    let total = tree.node(root).total;
    if total <= 0.0 || width == 0 {
        return rows;
    }
    let scale = width as f64 / total;
    // (node, x start, depth)
    let mut stack: Vec<(u32, f64, usize)> = vec![(root, 0.0, 0)];
    while let Some((index, start, depth)) = stack.pop() {
        let node = tree.node(index);
        let end = start + node.total * scale;
        let (x0, x1) = (start.round() as u16, (end.round() as u16).min(width));
        if x1 <= x0 {
            continue;
        }
        if rows.len() <= depth {
            rows.resize_with(depth + 1, Vec::new);
        }
        rows[depth].push(Span { x0, x1, node: index });
        let mut children = node.children.clone();
        if by_name {
            children.sort_by_key(|&child| name(child));
        }
        let mut at = start;
        let mut placed = Vec::with_capacity(children.len());
        for child in children {
            placed.push((child, at, depth + 1));
            at += tree.node(child).total * scale;
        }
        for item in placed.into_iter().rev() {
            stack.push(item);
        }
    }
    for row in &mut rows {
        row.sort_by_key(|span| span.x0);
    }
    rows
}

/// A node's label: the function's name and place, or the phase's
pub fn node_name(p: &Profile, tree: &Tree, index: u32) -> String {
    match tree.node(index).frame {
        Frame::Root => "run".to_string(),
        Frame::Phase(phase) => p.phase_name(phase).to_string(),
        Frame::Func(func) => {
            let f = &p.funcs[func as usize];
            if f.module.is_empty() { f.name.clone() } else { format!("{} {}:{}", f.name, f.short, f.line) }
        }
    }
}

fn sync(app: &mut App, tree: &Tree) {
    let key = (app.shown, tree.metric, app.scripts_only);
    if app.flame.built != Some(key) {
        app.flame.zoom.clear();
        app.flame.selected = ROOT;
        app.flame.top = 0;
        app.flame.built = Some(key);
        match app.focus {
            Some(focus) => select_func(app, tree, focus.func),
            // (where the cost is: the node with the most of its own)
            None => {
                let hottest = (0..tree.nodes.len() as u32).filter(|&i| matches!(tree.node(i).frame, Frame::Func(_))).max_by(|&a, &b| tree.node(a).own.total_cmp(&tree.node(b).own).then(b.cmp(&a)));
                if let Some(node) = hottest {
                    app.flame.selected = node;
                }
            }
        }
        return;
    }
    // another view picked a function: show its heaviest node
    if let Some(focus) = app.focus {
        let current = match tree.nodes.get(app.flame.selected as usize).map(|n| n.frame) {
            Some(Frame::Func(func)) => Some(func),
            _ => None,
        };
        if current != Some(focus.func) {
            select_func(app, tree, focus.func);
        }
    }
}

/// Selects a function's heaviest node, zooming so it is wide enough to see
fn select_func(app: &mut App, tree: &Tree, func: u32) {
    let Some(node) = tree.heaviest(func) else { return };
    app.flame.selected = node;
    let root = *app.flame.zoom.last().unwrap_or(&ROOT);
    let path = tree.path(node);
    if !path.contains(&root) {
        app.flame.zoom.clear();
    }
    let root = *app.flame.zoom.last().unwrap_or(&ROOT);
    let share = tree.node(node).total / tree.node(root).total.max(1e-12);
    if share < 0.02 {
        // (zoom to the node's caller, so it and its neighbours fill the width)
        let parent = tree.node(node).parent;
        if parent != u32::MAX {
            app.flame.zoom.push(parent);
        }
    }
}

pub fn draw(buf: &mut Buffer, area: Rect, app: &mut App, theme: &Theme) {
    let tree = app.tree();
    sync(app, &tree);
    let p = &app.profiles[app.shown];
    let metric = app.metric;
    let unit = tree.metric.unit();
    let total = tree.total();
    let root = *app.flame.zoom.last().unwrap_or(&ROOT);
    let note = if tree.metric != metric { format!("{} has no stacks: widths are {}", metric.label(p), tree.metric.label(p)) } else { "width: total · colour: own".to_string() };
    let zoom = if root == ROOT { String::new() } else { format!(" · in {}", node_name(p, &tree, root)) };
    section(buf, area, area.y, &tree.metric.label(p), &format!("{note}{zoom}"), theme);
    let detail_height = 4;
    let flame = Rect { x: area.x, y: area.y + 1, width: area.width, height: area.height.saturating_sub(1 + detail_height) };
    let by_name = app.flame.by_name;
    let name_of = |index: u32| node_name(p, &tree, index);
    let rows = layout(&tree, root, flame.width, by_name, &name_of);
    app.flame.width = flame.width;
    // keep the selection on screen
    let selected_visible = rows.iter().enumerate().find_map(|(depth, row)| row.iter().any(|s| s.node == app.flame.selected).then_some(depth));
    let depth = match selected_visible {
        Some(depth) => depth,
        None => {
            app.flame.selected = root;
            0
        }
    };
    let height = flame.height as usize;
    let mut top = app.flame.top as usize;
    if depth < top {
        top = depth;
    } else if depth >= top + height {
        top = depth + 1 - height;
    }
    top = top.min(rows.len().saturating_sub(height));
    app.flame.top = top as u16;
    let query = app.query.to_lowercase();
    let found = |index: u32| !query.is_empty() && name_of(index).to_lowercase().contains(&query);
    for (row_index, row) in rows.iter().enumerate().skip(top).take(height) {
        let y = flame.y + (row_index - top) as u16;
        for span in row {
            let node = tree.node(span.node);
            let share = if total > 0.0 { node.own / total } else { 0.0 };
            let (bg, fg) = match node.frame {
                Frame::Root | Frame::Phase(_) => (theme.phase, theme.ink),
                Frame::Func(_) => match Theme::step(share) {
                    Some(step) => (theme.heat[step], theme.heat_ink[step]),
                    None => (theme.grid, theme.ink2),
                },
            };
            let (bg, fg) = if found(span.node) { (theme.found, theme.found_ink) } else { (bg, fg) };
            let mut style = Style::new().fg(fg).bg(bg);
            if span.node == app.flame.selected {
                style = Style::new().fg(theme.surface).bg(theme.ink).add_modifier(Modifier::BOLD);
            }
            let width = span.x1 - span.x0;
            // (a one-column surface gap between neighbours, when the node is wide enough to spare it)
            let fill_width = if width >= 3 { width - 1 } else { width };
            fill(buf, Rect { x: flame.x + span.x0, y, width: fill_width, height: 1 }, style);
            if fill_width >= 3 {
                let label = name_of(span.node);
                text(buf, flame.x + span.x0 + 1, y, &label, fill_width - 1, style);
            }
        }
    }
    if rows.len() > top + height {
        text(buf, flame.right().saturating_sub(14), flame.bottom().saturating_sub(1), &format!(" ▼ {} deeper ", rows.len() - top - height), 14, theme.dim());
    }
    // the selected node
    let y = area.bottom() - detail_height;
    let node = tree.node(app.flame.selected);
    let path: Vec<String> = tree.path(app.flame.selected).iter().skip(1).map(|&index| match tree.node(index).frame {
        Frame::Phase(phase) => p.phase_name(phase).to_string(),
        Frame::Func(func) => p.funcs[func as usize].name.clone(),
        Frame::Root => String::new(),
    }).collect();
    section(buf, area, y, &name_of(app.flame.selected), &fmt::fit_left(&path.join(" › "), area.width as usize / 2), theme);
    let calls = if tree.has_calls && node.calls > 0 { format!(" · {} calls, {} each", fmt::grouped(node.calls as f64), fmt::short(unit, node.total / node.calls as f64)) } else { String::new() };
    let line = format!("own {} ({}) · total {} ({}){calls}", fmt::value(unit, node.own), fmt::share(node.own, total), fmt::value(unit, node.total), fmt::share(node.total, total));
    text(buf, area.x, y + 1, &line, area.width, theme.text());
    if let Frame::Func(func) = node.frame {
        let f = &p.funcs[func as usize];
        let kind = match f.kind {
            crate::model::Kind::Script => "Luau",
            crate::model::Kind::Engine => "engine Luau",
            crate::model::Kind::C => "C",
            crate::model::Kind::Step => "engine step",
        };
        let all = tree.funcs.get(&func).filter(|stat| (stat.own - node.own).abs() > 1e-12 || (stat.total - node.total).abs() > 1e-12).map(|stat| format!(" · all its calls: own {}, total {}", fmt::value(unit, stat.own), fmt::value(unit, stat.total))).unwrap_or_default();
        spans(buf, area.x, y + 2, area.right(), &[(kind, theme.dim()), (&all, theme.dim())]);
    }
}

fn visible(app: &App, tree: &Tree, width: u16) -> Vec<Vec<Span>> {
    let root = *app.flame.zoom.last().unwrap_or(&ROOT);
    let p = &app.profiles[app.shown];
    let name_of = |index: u32| node_name(p, tree, index);
    layout(tree, root, width, app.flame.by_name, &name_of)
}

fn set_selected(app: &mut App, tree: &Tree, node: u32) {
    app.flame.selected = node;
    if let Frame::Func(func) = tree.node(node).frame {
        let line = None;
        app.focus = Some(Focus { func, line });
    }
}

pub fn key(app: &mut App, key: Key) {
    let tree = app.tree();
    // (the layout's width only decides which nodes are wide enough to show: the screen's, or a wide default)
    let rows = visible(app, &tree, if app.flame.width == 0 { 200 } else { app.flame.width });
    let selected = app.flame.selected;
    let place = rows.iter().enumerate().find_map(|(depth, row)| row.iter().position(|s| s.node == selected).map(|at| (depth, at)));
    match key {
        Key::Left | Key::Right => {
            if let Some((depth, at)) = place {
                let row = &rows[depth];
                let next = if key == Key::Left { at.checked_sub(1) } else { (at + 1 < row.len()).then_some(at + 1) };
                if let Some(next) = next {
                    set_selected(app, &tree, row[next].node);
                }
            }
        }
        Key::Up => {
            let root = *app.flame.zoom.last().unwrap_or(&ROOT);
            let parent = tree.node(selected).parent;
            if selected != root && parent != u32::MAX {
                set_selected(app, &tree, parent);
            }
        }
        Key::Down => {
            if let Some((depth, _)) = place {
                if let Some(row) = rows.get(depth + 1) {
                    if let Some(child) = row.iter().find(|s| tree.node(s.node).parent == selected) {
                        set_selected(app, &tree, child.node);
                    }
                }
            }
        }
        Key::Enter => {
            if !tree.node(selected).children.is_empty() && app.flame.zoom.last() != Some(&selected) {
                app.flame.zoom.push(selected);
                app.flame.top = 0;
            }
        }
        Key::Esc | Key::Backspace => {
            if app.flame.zoom.pop().is_some() {
                app.flame.top = 0;
            }
        }
        Key::Home => set_selected(app, &tree, *app.flame.zoom.last().unwrap_or(&ROOT)),
        Key::Char('s') => {
            app.flame.by_name = !app.flame.by_name;
            app.status = if app.flame.by_name { "callees by name".into() } else { "callees by cost".into() };
        }
        _ => {}
    }
}

pub fn copy(app: &App) -> Option<String> {
    let tree = app.tree();
    let Frame::Func(func) = tree.node(app.flame.selected).frame else { return None };
    super::calls::place_text(app, func, None)
}

pub fn find(app: &mut App, query: &str, forward: bool, from_start: bool) -> bool {
    let tree = app.tree();
    let order = tree.preorder(ROOT);
    let at = order.iter().position(|&n| n == app.flame.selected).unwrap_or(0);
    let p = &app.profiles[app.shown];
    let found = super::next_match(order.len(), at, forward, from_start, |i| node_name(p, &tree, order[i]).to_lowercase().contains(query));
    let Some(index) = found else { return false };
    let node = order[index];
    // zoom out until the node is under the zoomed root, and in so it is wide enough to see
    while let Some(&root) = app.flame.zoom.last() {
        if tree.path(node).contains(&root) {
            break;
        }
        app.flame.zoom.pop();
    }
    let root = *app.flame.zoom.last().unwrap_or(&ROOT);
    if tree.node(node).total / tree.node(root).total.max(1e-12) < 0.02 {
        let parent = tree.node(node).parent;
        if parent != u32::MAX && parent != root {
            app.flame.zoom.push(parent);
        }
    }
    set_selected(app, &tree, node);
    true
}
