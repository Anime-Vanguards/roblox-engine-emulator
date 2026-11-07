//! Colour, one job each (the data-viz method ree's report page follows): a single-hue heat ramp for how much of the run
//! something costs (warm = cost, in every view), a blue-red pair with a grey middle for less or more than before,
//! categorical slots in a fixed order for the variants of a comparison, and the status colours, always with an icon
//! and a word, for the issues. Text stays in the ink colours; a coloured mark beside it carries the meaning.
//!
//! The heat ramps were checked as ordered ramps (monotone lightness, steps at least 0.06 apart in OKLCH, one hue, the
//! lightest step at least 2:1 on the surface) and each step's label ink chosen by APCA. The categorical, diverging and
//! status values are the method's reference palette.

use ratatui_core::style::{Color, Modifier, Style};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThemeKind {
    Dark,
    Light,
}

impl ThemeKind {
    pub fn parse(name: &str) -> Option<ThemeKind> {
        match name.trim().to_ascii_lowercase().as_str() {
            "dark" => Some(ThemeKind::Dark),
            "light" => Some(ThemeKind::Light),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ThemeKind::Dark => "dark",
            ThemeKind::Light => "light",
        }
    }
}

/// The share of the run's total each heat step starts at; below the first a figure is negligible and dimmed
pub const HEAT_FROM: [f64; 5] = [0.001, 0.01, 0.03, 0.10, 0.30];
/// The legend's words for the steps
pub const HEAT_LABELS: [&str; 5] = ["0.1-1%", "1-3%", "3-10%", "10-30%", ">=30%"];

#[derive(Clone, Debug)]
pub struct Theme {
    pub kind: ThemeKind,
    /// The page: everything is drawn on it
    pub surface: Color,
    /// The header and footer bars
    pub bar: Color,
    pub ink: Color,
    pub ink2: Color,
    pub muted: Color,
    /// Hairlines: borders, separators, empty bar tracks
    pub grid: Color,
    /// A selected row's background
    pub wash: Color,
    pub heat: [Color; 5],
    /// The label colour on each heat step
    pub heat_ink: [Color; 5],
    /// More than before (worse for a cost)
    pub more: Color,
    /// Less than before
    pub less: Color,
    pub series: [Color; 8],
    pub good: Color,
    pub warning: Color,
    pub serious: Color,
    pub critical: Color,
    /// A search match's background
    pub found: Color,
    pub found_ink: Color,
    /// A phase's node in the flame graph (the bars' grey, told apart by the exporter)
    pub phase: Color,
    /// Backgrounds the eye cannot tell from the surface (and from `wash`), which tell the exporter what a cell is: a
    /// horizontal bar's cell (drawn thin, with a gap between rows) or a column chart's (a gap between columns)
    pub bar_mark: Color,
    pub bar_mark_wash: Color,
    pub column_mark: Color,
}

/// What a cell is drawn as by the exporter (Theme::unmark)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mark {
    None,
    Bar,
    Column,
}

/// A colour one step off in one channel
fn nudge(color: Color, channel: usize) -> Color {
    let Color::Rgb(r, g, b) = color else { return color };
    let mut c = [r, g, b];
    c[channel] = if c[channel] == 255 { 254 } else { c[channel] + 1 };
    Color::Rgb(c[0], c[1], c[2])
}

const fn rgb(hex: u32) -> Color {
    Color::Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

impl Theme {
    pub fn new(kind: ThemeKind) -> Theme {
        let mut theme = match kind {
            ThemeKind::Dark => Theme {
                kind,
                surface: rgb(0x1a1a19),
                bar: rgb(0x2c2c2a),
                ink: rgb(0xf0efec),
                ink2: rgb(0xc3c2b7),
                muted: rgb(0x898781),
                grid: rgb(0x383835),
                wash: rgb(0x1f2b3a),
                heat: [rgb(0x7d4425), rgb(0xa3572c), rgb(0xcb6b34), rgb(0xf3813f), rgb(0xffae84)],
                heat_ink: [rgb(0xf0efec), rgb(0xf0efec), rgb(0xf0efec), rgb(0x1a1a19), rgb(0x1a1a19)],
                more: rgb(0xe66767),
                less: rgb(0x6da7ec),
                series: [rgb(0x3987e5), rgb(0xd95926), rgb(0x199e70), rgb(0xc98500), rgb(0xd55181), rgb(0x008300), rgb(0x9085e9), rgb(0xe66767)],
                good: rgb(0x0ca30c),
                warning: rgb(0xfab219),
                serious: rgb(0xec835a),
                critical: rgb(0xd03b3b),
                found: rgb(0x9085e9),
                found_ink: rgb(0x1a1a19),
                phase: rgb(0x2c2c2a),
                bar_mark: rgb(0x1a1a19),
                bar_mark_wash: rgb(0x1f2b3a),
                column_mark: rgb(0x1a1a19),
            },
            ThemeKind::Light => Theme {
                kind,
                surface: rgb(0xfcfcfb),
                bar: rgb(0xe9e8e3),
                ink: rgb(0x0b0b0b),
                ink2: rgb(0x52514e),
                muted: rgb(0x898781),
                grid: rgb(0xd5d4cc),
                wash: rgb(0xe7eff7),
                heat: [rgb(0xeca47f), rgb(0xe28755), rgb(0xd56927), rgb(0xbd5400), rgb(0x9f4500)],
                heat_ink: [rgb(0x0b0b0b), rgb(0xfcfcfb), rgb(0xfcfcfb), rgb(0xfcfcfb), rgb(0xfcfcfb)],
                more: rgb(0xe34948),
                less: rgb(0x256abf),
                series: [rgb(0x2a78d6), rgb(0xeb6834), rgb(0x1baf7a), rgb(0xeda100), rgb(0xe87ba4), rgb(0x008300), rgb(0x6250d6), rgb(0xe34948)],
                good: rgb(0x0ca30c),
                warning: rgb(0xfab219),
                serious: rgb(0xec835a),
                critical: rgb(0xd03b3b),
                found: rgb(0x6250d6),
                found_ink: rgb(0xfcfcfb),
                phase: rgb(0xe9e8e3),
                bar_mark: rgb(0xfcfcfb),
                bar_mark_wash: rgb(0xe7eff7),
                column_mark: rgb(0xfcfcfb),
            },
        };
        theme.phase = nudge(theme.bar, 0);
        theme.bar_mark = nudge(theme.surface, 2);
        theme.bar_mark_wash = nudge(theme.wash, 2);
        theme.column_mark = nudge(theme.surface, 1);
        theme
    }

    /// A cell's background as it should look, and what the exporter draws thinner in it
    pub fn unmark(&self, bg: Color) -> (Color, Mark) {
        if bg == self.bar_mark {
            (self.surface, Mark::Bar)
        } else if bg == self.bar_mark_wash {
            (self.wash, Mark::Bar)
        } else if bg == self.column_mark {
            (self.surface, Mark::Column)
        } else {
            (bg, Mark::None)
        }
    }

    /// A flame graph node's background: the exporter leaves a gap above each row of them
    pub fn node_bg(&self, bg: Color) -> bool {
        self.heat.contains(&bg) || bg == self.grid || bg == self.phase || bg == self.ink || bg == self.found
    }

    /// The heat step of a share of the run's total, None when negligible
    pub fn step(share: f64) -> Option<usize> {
        if !(share >= HEAT_FROM[0]) {
            return None;
        }
        Some(HEAT_FROM.iter().rposition(|from| share >= *from).unwrap_or(0))
    }

    /// The colour of a share of the run's total (muted when negligible)
    pub fn heat_of(&self, share: f64) -> Color {
        Theme::step(share).map_or(self.muted, |step| self.heat[step])
    }

    pub fn text(&self) -> Style {
        Style::new().fg(self.ink).bg(self.surface)
    }

    pub fn secondary(&self) -> Style {
        Style::new().fg(self.ink2).bg(self.surface)
    }

    pub fn dim(&self) -> Style {
        Style::new().fg(self.muted).bg(self.surface)
    }

    pub fn bold(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    pub fn title(&self) -> Style {
        Style::new().fg(self.ink).bg(self.surface).add_modifier(Modifier::BOLD)
    }

    pub fn bar_text(&self) -> Style {
        Style::new().fg(self.ink).bg(self.bar)
    }

    pub fn bar_dim(&self) -> Style {
        Style::new().fg(self.muted).bg(self.bar)
    }

    pub fn selected(&self) -> Style {
        Style::new().fg(self.ink).bg(self.wash).add_modifier(Modifier::BOLD)
    }

    /// Text in a value's own heat (a figure's ink stays ink; this is for marks)
    pub fn mark(&self, color: Color) -> Style {
        Style::new().fg(color).bg(self.surface)
    }

    /// Ink for a row: negligible rows recede
    pub fn row(&self, share: f64) -> Style {
        if Theme::step(share).is_some() { self.text() } else { self.dim() }
    }
}
