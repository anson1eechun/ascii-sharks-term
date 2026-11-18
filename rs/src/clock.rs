//! Not in upstream: a clock and a message on the water's surface, over the sharks.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::grid::{DEFAULT_BG, Grid, rgb, str_width};

const INK: u32 = rgb(222, 232, 245);
const SHADE: u32 = rgb(46, 64, 92);

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ClockStyle {
    Block,
    Shadow,
    Hash,
    Half,
    Dots,
    Big,
    Lcd,
}

impl ClockStyle {
    pub const ALL: [ClockStyle; 7] = [
        ClockStyle::Block,
        ClockStyle::Shadow,
        ClockStyle::Hash,
        ClockStyle::Half,
        ClockStyle::Dots,
        ClockStyle::Big,
        ClockStyle::Lcd,
    ];

    pub fn name(self) -> &'static str {
        match self {
            ClockStyle::Block => "block",
            ClockStyle::Shadow => "shadow",
            ClockStyle::Hash => "hash",
            ClockStyle::Half => "half",
            ClockStyle::Dots => "dots",
            ClockStyle::Big => "big",
            ClockStyle::Lcd => "lcd",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.name() == s)
    }

    /// The style to draw where only ASCII is wanted, where block elements come out two
    /// columns wide, or where there is no colour to tell a shadow apart.
    pub fn fallback(self, ascii: bool, wide_blocks: bool, mono: bool) -> Self {
        match self {
            ClockStyle::Half if ascii || wide_blocks => ClockStyle::Block,
            ClockStyle::Dots if ascii => ClockStyle::Hash,
            ClockStyle::Shadow if mono => ClockStyle::Block,
            style => style,
        }
    }
}

const SMALL: [[&str; 5]; 11] = [
    ["###", "# #", "# #", "# #", "###"],
    [" # ", "## ", " # ", " # ", "###"],
    ["###", "  #", "###", "#  ", "###"],
    ["###", "  #", "###", "  #", "###"],
    ["# #", "# #", "###", "  #", "  #"],
    ["###", "#  ", "###", "  #", "###"],
    ["###", "#  ", "###", "# #", "###"],
    ["###", "  #", "  #", "  #", "  #"],
    ["###", "# #", "###", "# #", "###"],
    ["###", "# #", "###", "  #", "###"],
    [" ", "#", " ", "#", " "],
];

const LARGE: [[&str; 7]; 11] = [
    [
        " ### ", "#   #", "#  ##", "# # #", "##  #", "#   #", " ### ",
    ],
    [
        "  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### ",
    ],
    [
        " ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####",
    ],
    [
        "#####", "   # ", "  #  ", "   # ", "    #", "#   #", " ### ",
    ],
    [
        "   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # ",
    ],
    [
        "#####", "#    ", "#### ", "    #", "    #", "#   #", " ### ",
    ],
    [
        "  ## ", " #   ", "#    ", "#### ", "#   #", "#   #", " ### ",
    ],
    [
        "#####", "    #", "   # ", "  #  ", " #   ", " #   ", " #   ",
    ],
    [
        " ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### ",
    ],
    [
        " ### ", "#   #", "#   #", " ####", "    #", "   # ", " ##  ",
    ],
    [" ", " ", "#", " ", "#", " ", " "],
];

const BIG: [[&str; 5]; 11] = [
    ["  ___  ", " / _ \\ ", "| | | |", "| |_| |", " \\___/ "],
    [" _ ", "/ |", "| |", "| |", "|_|"],
    [" ____  ", "|___ \\ ", "  __) |", " / __/ ", "|_____|"],
    [" _____ ", "|___ / ", "  |_ \\ ", " ___) |", "|____/ "],
    [" _  _   ", "| || |  ", "| || |_ ", "|__   _|", "   |_|  "],
    [" ____  ", "| ___| ", "|___ \\ ", " ___) |", "|____/ "],
    ["  __   ", " / /_  ", "| '_ \\ ", "| (_) |", " \\___/ "],
    [" _____ ", "|___  |", "   / / ", "  / /  ", " /_/   "],
    ["  ___  ", " ( _ ) ", " / _ \\ ", "| (_) |", " \\___/ "],
    ["  ___  ", " / _ \\ ", "| (_) |", " \\__, |", "   /_/ "],
    ["   ", " _ ", "(_)", " _ ", "(_)"],
];

/// Seven-segment digits: bits a b c d e f g from the top, clockwise, then the middle.
const SEGMENTS: [u8; 10] = [0x3f, 0x06, 0x5b, 0x4f, 0x66, 0x6d, 0x7d, 0x07, 0x7f, 0x6f];

fn index(c: char) -> usize {
    c.to_digit(10).map_or(10, |d| d as usize)
}

#[derive(Clone, Copy)]
enum Ink {
    Glyph(char),
    Fill(u32),
}

/// A rectangle of cells to lay over the grid; None is see-through.
struct Art {
    w: usize,
    h: usize,
    cells: Vec<Option<Ink>>,
}

impl Art {
    fn new(w: usize, h: usize) -> Self {
        Art {
            w,
            h,
            cells: vec![None; w * h],
        }
    }

    fn fill(&mut self, x: usize, y: usize, w: usize, h: usize, ink: Ink) {
        for row in y..(y + h).min(self.h) {
            for col in x..(x + w).min(self.w) {
                self.cells[row * self.w + col] = Some(ink);
            }
        }
    }

    fn from_rows(rows: &[String]) -> Self {
        let w = rows.iter().map(|r| r.chars().count()).max().unwrap_or(0);
        let mut art = Art::new(w, rows.len());
        for (y, row) in rows.iter().enumerate() {
            for (x, c) in row.chars().enumerate().filter(|&(_, c)| c != ' ') {
                art.cells[y * w + x] = Some(Ink::Glyph(c));
            }
        }
        art
    }

    fn blit(&self, grid: &mut Grid, top: usize, left: usize) {
        for y in 0..self.h.min(grid.rows.saturating_sub(top)) {
            for x in 0..self.w.min(grid.cols.saturating_sub(left)) {
                let i = (top + y) * grid.cols + left + x;
                match self.cells[y * self.w + x] {
                    Some(Ink::Glyph(c)) => grid.text(i, c, INK, 1.0),
                    Some(Ink::Fill(color)) => grid.block(i, color),
                    // Calm water around the strokes: fish and wakes go, sharks stay.
                    None if grid.bg[i] == DEFAULT_BG => grid.glyph[i] = ' ',
                    None => {}
                }
            }
        }
    }
}

/// The text set in a bitmap font, one pixel of gap between glyphs.
fn pixels<const H: usize>(text: &str, font: &[[&str; H]; 11]) -> Vec<Vec<bool>> {
    let mut rows = vec![Vec::new(); H];
    for (n, c) in text.chars().enumerate() {
        for (y, row) in rows.iter_mut().enumerate() {
            if n > 0 {
                row.push(false);
            }
            row.extend(font[index(c)][y].chars().map(|p| p == '#'));
        }
    }
    rows
}

fn scaled(px: &[Vec<bool>], s: usize) -> Vec<Vec<bool>> {
    px.iter()
        .flat_map(|row| {
            std::iter::repeat_n(
                row.iter()
                    .flat_map(|&p| std::iter::repeat_n(p, s))
                    .collect(),
                s,
            )
        })
        .collect()
}

/// Within upstream's headline zone, which the sharks keep out of.
fn fits(w: usize, h: usize, cols: usize, rows: usize) -> bool {
    w * 10 <= cols * 6 && h * 100 <= rows * 32
}

fn largest(
    scales: &[usize],
    cols: usize,
    rows: usize,
    size: impl Fn(usize) -> (usize, usize),
) -> usize {
    scales
        .iter()
        .copied()
        .take_while(|&s| {
            let (w, h) = size(s);
            fits(w, h, cols, rows)
        })
        .last()
        .unwrap_or(1)
}

fn render(style: ClockStyle, text: &str, cols: usize, rows: usize) -> Art {
    match style {
        ClockStyle::Block | ClockStyle::Shadow | ClockStyle::Hash => {
            blocks(style, text, cols, rows)
        }
        ClockStyle::Half => half_blocks(text, cols, rows),
        ClockStyle::Dots => braille(text, cols, rows),
        ClockStyle::Big => lines(text, |c| BIG[index(c)].map(String::from)),
        ClockStyle::Lcd => lines(text, lcd_glyph),
    }
}

/// Square pixels two cells wide: solid, shadowed, or drawn with `#`.
fn blocks(style: ClockStyle, text: &str, cols: usize, rows: usize) -> Art {
    let px = pixels(text, &SMALL);
    let s = largest(&[2, 3, 4, 5, 6], cols, rows, |s| {
        (px[0].len() * 2 * s, 5 * s)
    });
    let shadow = (style == ClockStyle::Shadow) as usize;
    let mut art = Art::new(px[0].len() * 2 * s + 2 * shadow, 5 * s + shadow);
    let ink = if style == ClockStyle::Hash {
        Ink::Glyph('#')
    } else {
        Ink::Fill(INK)
    };
    let passes = [(Ink::Fill(SHADE), 2, 1), (ink, 0, 0)];
    for &(ink, dx, dy) in &passes[1 - shadow..] {
        for (y, row) in px.iter().enumerate() {
            for (x, _) in row.iter().enumerate().filter(|&(_, &p)| p) {
                art.fill(x * 2 * s + dx, y * s + dy, 2 * s, s, ink);
            }
        }
    }
    art
}

/// Two pixels per cell, one above the other.
fn half_blocks(text: &str, cols: usize, rows: usize) -> Art {
    let base = pixels(text, &LARGE);
    // Odd scales only: even ones fill both halves of every cell alike.
    let s = largest(&[3, 5], cols, rows, |s| {
        (base[0].len() * s, (7 * s).div_ceil(2))
    });
    let px = scaled(&base, s);
    let mut art = Art::new(px[0].len(), px.len().div_ceil(2));
    for (y, pair) in px.chunks(2).enumerate() {
        for x in 0..art.w {
            let glyph = match (pair[0][x], pair.get(1).is_some_and(|r| r[x])) {
                (true, true) => '█',
                (true, false) => '▀',
                (false, true) => '▄',
                (false, false) => continue,
            };
            art.cells[y * art.w + x] = Some(Ink::Glyph(glyph));
        }
    }
    art
}

/// Braille patterns: two by four pixels per cell.
fn braille(text: &str, cols: usize, rows: usize) -> Art {
    const DOTS: [(usize, usize, u32); 8] = [
        (0, 0, 0x01),
        (0, 1, 0x02),
        (0, 2, 0x04),
        (1, 0, 0x08),
        (1, 1, 0x10),
        (1, 2, 0x20),
        (0, 3, 0x40),
        (1, 3, 0x80),
    ];
    let base = pixels(text, &LARGE);
    let s = largest(&[2, 3, 4, 5, 6], cols, rows, |s| {
        ((base[0].len() * s).div_ceil(2), (7 * s).div_ceil(4))
    });
    let px = scaled(&base, s);
    let at = |x: usize, y: usize| px.get(y).and_then(|r| r.get(x)).copied().unwrap_or(false);
    let mut art = Art::new(px[0].len().div_ceil(2), px.len().div_ceil(4));
    for y in 0..art.h {
        for x in 0..art.w {
            let bits: u32 = DOTS
                .iter()
                .filter(|&&(dx, dy, _)| at(2 * x + dx, 4 * y + dy))
                .map(|d| d.2)
                .sum();
            if bits != 0 {
                art.cells[y * art.w + x] = char::from_u32(0x2800 + bits).map(Ink::Glyph);
            }
        }
    }
    art
}

fn lines(text: &str, glyph: impl Fn(char) -> [String; 5]) -> Art {
    let glyphs: Vec<[String; 5]> = text.chars().map(glyph).collect();
    let rows: Vec<String> = (0..5)
        .map(|y| {
            glyphs
                .iter()
                .map(|g| g[y].as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect();
    Art::from_rows(&rows)
}

fn lcd_glyph(c: char) -> [String; 5] {
    let Some(d) = c.to_digit(10) else {
        return ["   ", "   ", " o ", "   ", " o "].map(String::from);
    };
    let on = |bit: u8| SEGMENTS[d as usize] & (1 << bit) != 0;
    let bar = |bit| if on(bit) { "____" } else { "    " };
    let side = |bit| if on(bit) { "|" } else { " " };
    [
        format!(" {} ", bar(0)),
        format!("{}    {}", side(5), side(1)),
        format!("{}{}{}", side(5), bar(6), side(1)),
        format!("{}    {}", side(4), side(2)),
        format!("{}{}{}", side(4), bar(3), side(2)),
    ]
}

fn now() -> Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

fn local_time() -> (u32, u32, u32) {
    unsafe {
        let t = libc::time(std::ptr::null_mut());
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&t, &mut tm);
        (tm.tm_hour as u32, tm.tm_min as u32, tm.tm_sec as u32)
    }
}

struct Cached {
    time: (u32, u32, u32),
    cols: usize,
    rows: usize,
    art: Art,
}

pub struct Overlay {
    clock: Option<ClockStyle>,
    seconds: bool,
    message: Option<String>,
    cache: Option<Cached>,
}

impl Overlay {
    pub fn new(clock: Option<ClockStyle>, seconds: bool, message: Option<String>) -> Self {
        Overlay {
            clock,
            seconds,
            message,
            cache: None,
        }
    }

    pub fn until_change(&self) -> Option<Duration> {
        self.clock?;
        let t = now();
        let period = if self.seconds { 1 } else { 60 };
        Some(Duration::from_secs(period) - Duration::new(t.as_secs() % period, t.subsec_nanos()))
    }

    /// Lays the time out again only when it, or the grid's size, changed.
    fn refresh(&mut self, style: ClockStyle, cols: usize, rows: usize, (h, m, s): (u32, u32, u32)) {
        let time = (h, m, if self.seconds { s } else { 0 });
        if matches!(&self.cache, Some(c) if c.time == time && c.cols == cols && c.rows == rows) {
            return;
        }
        let text = if self.seconds {
            format!("{h:02}:{m:02}:{s:02}")
        } else {
            format!("{h:02}:{m:02}")
        };
        self.cache = Some(Cached {
            time,
            cols,
            rows,
            art: render(style, &text, cols, rows),
        });
    }

    pub fn draw(&mut self, grid: &mut Grid) {
        self.draw_at(grid, local_time);
    }

    /// Draws with the time `now` gives, called only when there is a clock.
    pub fn draw_at(&mut self, grid: &mut Grid, now: impl FnOnce() -> (u32, u32, u32)) {
        if let Some(style) = self.clock {
            self.refresh(style, grid.cols, grid.rows, now());
        } else if self.message.is_none() {
            return;
        }
        let art = self.cache.as_ref().map(|c| &c.art);
        let art_h = art.map_or(0, |a| a.h);
        let msg_w = self.message.as_deref().map(str_width);
        let height = art_h + msg_w.map_or(0, |_| if art_h > 0 { 2 } else { 1 });
        let top = grid.rows.saturating_sub(height) / 2;
        if let Some(art) = art {
            art.blit(grid, top, grid.cols.saturating_sub(art.w) / 2);
        }
        if let (Some(msg), Some(w)) = (&self.message, msg_w) {
            let row = if art_h > 0 { top + art_h + 1 } else { top };
            grid.put(row, grid.cols.saturating_sub(w) / 2, msg, INK, None);
        }
    }
}

/// `12:34` in `style`, as it would look in a 110 × 40 window.
pub fn sample(style: ClockStyle) -> Grid {
    let art = render(style, "12:34", 110, 40);
    let mut grid = Grid::new();
    grid.resize(art.w, art.h);
    art.blit(&mut grid, 0, 0);
    grid
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(style: ClockStyle) -> Vec<String> {
        let g = sample(style);
        (0..g.rows)
            .map(|r| {
                g.glyph[r * g.cols..(r + 1) * g.cols]
                    .iter()
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn every_style_draws_every_glyph() {
        for style in ClockStyle::ALL {
            for text in ["01:23", "45:67", "89:00"] {
                let art = render(style, text, 200, 60);
                assert!(art.cells.iter().any(Option::is_some), "{style:?} {text}");
                assert!(
                    art.w <= 120 && art.h <= 20,
                    "{style:?} too big: {}x{}",
                    art.w,
                    art.h
                );
            }
        }
    }

    #[test]
    fn big_and_lcd_read_as_digits() {
        assert_eq!(
            picture(ClockStyle::Lcd)[2],
            "     |  ____|  o   ____| |____|"
        );
        assert_eq!(picture(ClockStyle::Big)[0], " _   ____        _____   _  _");
    }

    #[test]
    fn message_sits_under_the_clock() {
        let mut g = Grid::new();
        g.resize(80, 30);
        Overlay::new(None, false, None).draw(&mut g);
        assert!(
            g.glyph.iter().all(|&c| c == ' '),
            "an empty overlay draws nothing"
        );
        Overlay::new(Some(ClockStyle::Big), false, Some("hi".into())).draw(&mut g);
        let rows: Vec<String> = (0..30)
            .map(|r| g.glyph[r * 80..(r + 1) * 80].iter().collect())
            .collect();
        let art = rows.iter().rposition(|r| r.contains('|')).unwrap();
        assert_eq!(rows[art + 2].trim(), "hi");
    }

    #[test]
    fn scales_with_the_screen() {
        let small = render(ClockStyle::Block, "12:34", 80, 24);
        let large = render(ClockStyle::Block, "12:34", 240, 70);
        assert!(large.w > small.w && large.w * 10 <= 240 * 7);
    }
}
