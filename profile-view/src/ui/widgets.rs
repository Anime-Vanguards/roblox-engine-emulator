//! Drawing helpers: text in a cell grid, bars, sparklines, section titles and table headers. Everything is written
//! straight into the buffer, one character a cell, so a screen is the same wherever it is drawn.

use ratatui_core::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
};

use crate::{fmt, theme::Theme};

/// Fills an area with spaces in a style
pub fn fill(buf: &mut Buffer, area: Rect, style: Style) {
    let area = area.intersection(buf.area);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            cell.set_symbol(" ");
            cell.set_style(style);
        }
    }
}

/// Writes text from (x, y), at most `width` columns, cut with an ellipsis; returns the columns written
pub fn text(buf: &mut Buffer, x: u16, y: u16, s: &str, width: u16, style: Style) -> u16 {
    if width == 0 || y >= buf.area.bottom() || x >= buf.area.right() {
        return 0;
    }
    let width = width.min(buf.area.right() - x);
    let s = fmt::fit(&fmt::cells(s), width as usize);
    let mut written = 0;
    for (i, ch) in s.chars().enumerate() {
        let cell = &mut buf[(x + i as u16, y)];
        let mut tmp = [0u8; 4];
        cell.set_symbol(ch.encode_utf8(&mut tmp));
        cell.set_style(style);
        written += 1;
    }
    written
}

/// Writes text right-aligned in `width` columns ending at x + width
pub fn text_right(buf: &mut Buffer, x: u16, y: u16, s: &str, width: u16, style: Style) {
    let s = fmt::fit(&fmt::cells(s), width as usize);
    let len = s.chars().count() as u16;
    text(buf, x + width.saturating_sub(len), y, &s, len, style);
}

/// Text centred in `width` columns
pub fn text_center(buf: &mut Buffer, x: u16, y: u16, s: &str, width: u16, style: Style) {
    let s = fmt::fit(&fmt::cells(s), width as usize);
    let len = s.chars().count() as u16;
    text(buf, x + (width.saturating_sub(len)) / 2, y, &s, len, style);
}

/// Spans of styled text in a row: returns the next x
pub fn spans(buf: &mut Buffer, mut x: u16, y: u16, right: u16, parts: &[(&str, Style)]) -> u16 {
    for (s, style) in parts {
        if x >= right {
            break;
        }
        x += text(buf, x, y, s, right - x, *style);
    }
    x
}

/// A horizontal hairline
pub fn hline(buf: &mut Buffer, x: u16, y: u16, width: u16, style: Style) {
    for i in 0..width {
        if x + i < buf.area.right() && y < buf.area.bottom() {
            let cell = &mut buf[(x + i, y)];
            cell.set_symbol("─");
            cell.set_style(style);
        }
    }
}

/// A vertical hairline
pub fn vline(buf: &mut Buffer, x: u16, y: u16, height: u16, style: Style) {
    for i in 0..height {
        if x < buf.area.right() && y + i < buf.area.bottom() {
            let cell = &mut buf[(x, y + i)];
            cell.set_symbol("│");
            cell.set_style(style);
        }
    }
}

/// A section's title over a hairline: `Title ───── note`
pub fn section(buf: &mut Buffer, area: Rect, y: u16, title: &str, note: &str, theme: &Theme) {
    let x = area.x;
    let width = area.width;
    let written = text(buf, x, y, title, width, theme.title());
    let mut at = x + written + 1;
    if !note.is_empty() && at < x + width {
        let room = (x + width).saturating_sub(at);
        let n = text(buf, at, y, note, room, theme.dim());
        at += n + 1;
    }
    if at < x + width {
        hline(buf, at, y, x + width - at, Style::new().fg(theme.grid).bg(theme.surface));
    }
}

const EIGHTHS: [&str; 9] = [" ", "▏", "▎", "▍", "▌", "▋", "▊", "▉", "█"];

/// A horizontal bar of `fraction` (0..1) of `width` columns, in eighths of a column, on a faint track
pub fn bar(buf: &mut Buffer, x: u16, y: u16, width: u16, fraction: f64, color: Color, theme: &Theme) {
    let fraction = if fraction.is_finite() { fraction.clamp(0.0, 1.0) } else { 0.0 };
    let eighths = (fraction * width as f64 * 8.0).round() as u32;
    // (a non-zero value always shows at least an eighth)
    let eighths = if fraction > 0.0 { eighths.max(1) } else { 0 };
    for i in 0..width {
        let filled = eighths.saturating_sub(i as u32 * 8).min(8) as usize;
        if x + i >= buf.area.right() || y >= buf.area.bottom() {
            break;
        }
        let cell = &mut buf[(x + i, y)];
        if filled == 0 {
            cell.set_symbol("·");
            cell.set_style(Style::new().fg(theme.grid).bg(theme.bar_mark));
        } else {
            cell.set_symbol(EIGHTHS[filled]);
            cell.set_style(Style::new().fg(color).bg(theme.bar_mark));
        }
    }
}

/// A two-sided bar for a change: blue to the left of a hairline for less, red to the right for more, `fraction` of an
/// arm (-1..1). The right arm ends in eighths of a cell; the left arm in halves (a cell's right half is the finest
/// right-aligned block every terminal font has)
pub fn diverging_bar(buf: &mut Buffer, x: u16, y: u16, width: u16, fraction: f64, theme: &Theme) {
    let half = width / 2;
    let fraction = if fraction.is_finite() { fraction.clamp(-1.0, 1.0) } else { 0.0 };
    for i in 0..width {
        if x + i < buf.area.right() && y < buf.area.bottom() {
            let cell = &mut buf[(x + i, y)];
            cell.set_symbol(if i == half { "│" } else { " " });
            cell.set_style(Style::new().fg(theme.grid).bg(theme.bar_mark));
        }
    }
    if fraction == 0.0 {
        return;
    }
    let eighths = ((fraction.abs() * half as f64 * 8.0).round() as u32).max(1);
    for i in 0..half {
        let filled = eighths.saturating_sub(i as u32 * 8).min(8) as usize;
        if filled == 0 {
            break;
        }
        if fraction > 0.0 {
            if x + half + 1 + i >= buf.area.right() {
                break;
            }
            let cell = &mut buf[(x + half + 1 + i, y)];
            cell.set_symbol(EIGHTHS[filled]);
            cell.set_style(Style::new().fg(theme.more).bg(theme.bar_mark));
        } else {
            let symbol = match filled {
                8 => "█",
                4..=7 => "▐",
                _ if i == 0 => "▐",
                _ => break,
            };
            let cell = &mut buf[(x + half - 1 - i, y)];
            cell.set_symbol(symbol);
            cell.set_style(Style::new().fg(theme.less).bg(theme.bar_mark));
        }
    }
}

const LEVELS: [&str; 9] = [" ", "▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// A sparkline in one row: one value a column, each in eighths of the row's height, coloured by `color`
pub fn sparkline(buf: &mut Buffer, x: u16, y: u16, values: &[f64], max: f64, color: impl Fn(usize) -> Color, theme: &Theme) {
    for (i, &value) in values.iter().enumerate() {
        if x + i as u16 >= buf.area.right() {
            break;
        }
        let level = if max > 0.0 { ((value / max) * 8.0).round().clamp(0.0, 8.0) as usize } else { 0 };
        let level = if value > 0.0 { level.max(1) } else { 0 };
        let cell = &mut buf[(x + i as u16, y)];
        cell.set_symbol(LEVELS[level]);
        cell.set_style(Style::new().fg(color(i)).bg(theme.column_mark));
    }
}

/// Vertical bars `height` rows tall, one value a column, in eighths of a row
pub fn columns(buf: &mut Buffer, area: Rect, values: &[f64], max: f64, color: impl Fn(usize) -> Color, theme: &Theme) {
    let height = area.height as f64;
    for (i, &value) in values.iter().enumerate() {
        let x = area.x + i as u16;
        if x >= area.right() {
            break;
        }
        let eighths = if max > 0.0 { (value / max * height * 8.0).round() as u32 } else { 0 };
        let eighths = if value > 0.0 { eighths.max(1) } else { 0 };
        for row in 0..area.height {
            let filled = eighths.saturating_sub(row as u32 * 8).min(8) as usize;
            let y = area.bottom() - 1 - row;
            let cell = &mut buf[(x, y)];
            cell.set_symbol(LEVELS[filled]);
            cell.set_style(Style::new().fg(color(i)).bg(theme.column_mark));
        }
    }
}

/// A table column: title, width, right-aligned
#[derive(Clone, Debug)]
pub struct Col {
    pub title: String,
    pub width: u16,
    pub right: bool,
}

impl Col {
    pub fn left(title: &str, width: u16) -> Col {
        Col { title: title.into(), width, right: false }
    }

    pub fn right(title: &str, width: u16) -> Col {
        Col { title: title.into(), width, right: true }
    }
}

/// The columns' x positions from `x`, one space apart; the last left-aligned column takes what is left
pub fn layout(cols: &mut [Col], x: u16, width: u16) -> Vec<u16> {
    let fixed: u16 = cols.iter().map(|c| c.width).sum::<u16>() + cols.len().saturating_sub(1) as u16;
    if let Some(flex) = cols.iter().rposition(|c| !c.right) {
        if fixed < width {
            cols[flex].width += width - fixed;
        } else {
            let over = fixed - width;
            cols[flex].width = cols[flex].width.saturating_sub(over).max(4);
        }
    }
    let mut at = x;
    cols.iter().map(|c| {
        let start = at;
        at += c.width + 1;
        start
    }).collect()
}

/// The header row of a table
pub fn header(buf: &mut Buffer, cols: &[Col], xs: &[u16], y: u16, theme: &Theme) {
    let style = Style::new().fg(theme.muted).bg(theme.surface);
    for (col, &x) in cols.iter().zip(xs) {
        if col.right {
            text_right(buf, x, y, &col.title, col.width, style);
        } else {
            text(buf, x, y, &col.title, col.width, style);
        }
    }
}

/// A cell of a table row
pub fn cell(buf: &mut Buffer, col: &Col, x: u16, y: u16, s: &str, style: Style) {
    if col.right {
        text_right(buf, x, y, s, col.width, style);
    } else {
        text(buf, x, y, s, col.width, style);
    }
}

/// The columns a list leaves at its left for the selection's marker
pub const MARGIN: u16 = 2;

/// Marks a row selected: its background washed (a bar's cells keep their mark), a bar at its left edge, where lists
/// leave MARGIN columns
pub fn select_row(buf: &mut Buffer, area: Rect, y: u16, theme: &Theme) {
    for x in area.left()..area.right() {
        if y < buf.area.bottom() && x < buf.area.right() {
            let cell = &mut buf[(x, y)];
            let bg = if cell.bg == theme.bar_mark { theme.bar_mark_wash } else { theme.wash };
            cell.set_bg(bg);
            cell.modifier.insert(Modifier::BOLD);
        }
    }
    if area.width > 0 && y < buf.area.bottom() {
        let cell = &mut buf[(area.x, y)];
        cell.set_symbol("▌");
        cell.set_fg(theme.ink);
    }
}

/// A scroll position note at a list's corner: `12-40 of 312`
pub fn scroll_note(buf: &mut Buffer, area: Rect, y: u16, offset: usize, shown: usize, len: usize, theme: &Theme) {
    if len > shown {
        let note = format!(" {}-{} of {} ", offset + 1, (offset + shown).min(len), len);
        text_right(buf, area.x, y, &note, area.width, theme.dim());
    }
}
