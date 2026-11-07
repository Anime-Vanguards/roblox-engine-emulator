//! A line plot in braille dots (2x4 a cell) on log scales: the Compare view's cost against input size.

use ratatui_core::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
};

use super::widgets::text;
use crate::theme::Theme;

/// A series: its points (x, y), both above zero, and its colour
pub struct Series {
    pub points: Vec<(f64, f64)>,
    pub color: Color,
}

/// Dot bits of a braille cell by (column, row) (U+2800 + bits)
const BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

/// Plots series on log-log axes in `area`: the y axis's labels in the left `label_width` columns, the x axis's under
/// the plot. `fmt_y` and `fmt_x` print the axes' values.
pub fn log_log(buf: &mut Buffer, area: Rect, series: &[Series], fmt_x: &dyn Fn(f64) -> String, fmt_y: &dyn Fn(f64) -> String, theme: &Theme) {
    let label_width = 10u16;
    if area.width <= label_width + 4 || area.height < 4 {
        return;
    }
    let plot = Rect { x: area.x + label_width + 1, y: area.y, width: area.width - label_width - 1, height: area.height - 2 };
    let points: Vec<(f64, f64)> = series.iter().flat_map(|s| s.points.iter().copied()).filter(|(x, y)| *x > 0.0 && *y > 0.0).collect();
    if points.is_empty() {
        text(buf, plot.x, plot.y, "nothing above zero to plot", plot.width, theme.dim());
        return;
    }
    let (mut x0, mut x1) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.0.log10()), hi.max(p.0.log10())));
    let (mut y0, mut y1) = points.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.1.log10()), hi.max(p.1.log10())));
    if x1 - x0 < 1e-9 {
        x0 -= 0.5;
        x1 += 0.5;
    }
    if y1 - y0 < 1e-9 {
        y0 -= 0.5;
        y1 += 0.5;
    }
    // (a little room above and below the extremes)
    let pad = (y1 - y0) * 0.06;
    let (y0, y1) = (y0 - pad, y1 + pad);
    let (dots_w, dots_h) = (plot.width as i64 * 2, plot.height as i64 * 4);
    let to_dot = |x: f64, y: f64| -> (i64, i64) {
        let dx = ((x.log10() - x0) / (x1 - x0) * (dots_w - 1) as f64).round() as i64;
        let dy = ((y1 - y.log10()) / (y1 - y0) * (dots_h - 1) as f64).round() as i64;
        (dx.clamp(0, dots_w - 1), dy.clamp(0, dots_h - 1))
    };
    // the grid: a faint dot row at each decade of y, and the axes' hairlines
    let grid = Style::new().fg(theme.grid).bg(theme.surface);
    for row in 0..plot.height {
        let cell = &mut buf[(plot.x - 1, plot.y + row)];
        cell.set_symbol("│");
        cell.set_style(grid);
    }
    // cells' dots and the series that drew most of each cell's dots
    let cells = (plot.width as usize) * (plot.height as usize);
    let mut bits = vec![0u8; cells];
    let mut owner: Vec<Option<usize>> = vec![None; cells];
    let mut set = |dx: i64, dy: i64, s: usize| {
        if dx < 0 || dy < 0 || dx >= dots_w || dy >= dots_h {
            return;
        }
        let (cx, cy) = ((dx / 2) as usize, (dy / 4) as usize);
        let index = cy * plot.width as usize + cx;
        bits[index] |= BITS[(dx % 2) as usize][(dy % 4) as usize];
        owner[index] = Some(s);
    };
    for (s, line) in series.iter().enumerate() {
        let pts: Vec<(i64, i64)> = line.points.iter().filter(|(x, y)| *x > 0.0 && *y > 0.0).map(|&(x, y)| to_dot(x, y)).collect();
        for pair in pts.windows(2) {
            // Bresenham between the two points
            let (mut x, mut y) = pair[0];
            let (xe, ye) = pair[1];
            let (dx, dy) = ((xe - x).abs(), -(ye - y).abs());
            let (sx, sy) = (if x < xe { 1 } else { -1 }, if y < ye { 1 } else { -1 });
            let mut err = dx + dy;
            loop {
                set(x, y, s);
                if x == xe && y == ye {
                    break;
                }
                let e2 = 2 * err;
                if e2 >= dy {
                    err += dy;
                    x += sx;
                }
                if e2 <= dx {
                    err += dx;
                    y += sy;
                }
            }
        }
        // the measured points: a 2x2 block of dots each
        for &(x, y) in &pts {
            for (ox, oy) in [(0, 0), (1, 0), (0, 1), (1, 1), (-1, 0), (0, -1)] {
                set(x + ox, y + oy, s);
            }
        }
    }
    for (index, &b) in bits.iter().enumerate() {
        if b == 0 {
            continue;
        }
        let (cx, cy) = ((index % plot.width as usize) as u16, (index / plot.width as usize) as u16);
        let color = owner[index].map_or(theme.ink, |s| series[s].color);
        let cell = &mut buf[(plot.x + cx, plot.y + cy)];
        cell.set_symbol(&char::from_u32(0x2800 + b as u32).unwrap_or(' ').to_string());
        cell.set_style(Style::new().fg(color).bg(theme.surface));
    }
    // the y axis: the top and bottom values, and a middle one when there is room
    let y_at = |row: u16| -> f64 { 10f64.powf(y1 - (row as f64 * 4.0 + 1.5) / (dots_h - 1) as f64 * (y1 - y0)) };
    let rows: Vec<u16> = if plot.height >= 6 { vec![0, plot.height / 2, plot.height - 1] } else { vec![0, plot.height - 1] };
    for row in rows {
        let label = fmt_y(y_at(row));
        super::widgets::text_right(buf, area.x, plot.y + row, &label, label_width, theme.dim());
    }
    // the x axis: a tick under each distinct x, its value under it
    let base = plot.bottom();
    for col in 0..plot.width {
        let cell = &mut buf[(plot.x + col, base)];
        cell.set_symbol("─");
        cell.set_style(grid);
    }
    let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    xs.sort_by(f64::total_cmp);
    xs.dedup();
    let mut last_end = 0u16;
    for x in xs {
        let (dx, _) = to_dot(x, 10f64.powf(y0));
        let col = (dx / 2) as u16;
        let cell = &mut buf[(plot.x + col, base)];
        cell.set_symbol("┬");
        cell.set_style(grid);
        let label = fmt_x(x);
        let width = label.chars().count() as u16;
        let start = (plot.x + col).saturating_sub(width / 2).max(plot.x).min(plot.right().saturating_sub(width));
        if start >= last_end {
            text(buf, start, base + 1, &label, width, theme.dim());
            last_end = start + width + 1;
        }
    }
}
