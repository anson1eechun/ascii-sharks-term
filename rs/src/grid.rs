//! The cell grid everything draws into, and the terminal writer reads from.

use crate::math::js_round;

pub const DEFAULT_BG: u32 = u32::MAX;
/// The right half of a double-width glyph; the writer skips it.
pub const WIDE_TAIL: char = '\u{E000}';

pub struct Grid {
    pub cols: usize,
    pub rows: usize,
    pub glyph: Vec<char>,
    pub fg: Vec<u32>,
    pub bg: Vec<u32>,
    pub has_wide: bool,
}

impl Grid {
    pub fn new() -> Self {
        Grid {
            cols: 0,
            rows: 0,
            glyph: Vec::new(),
            fg: Vec::new(),
            bg: Vec::new(),
            has_wide: false,
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.glyph = vec![' '; cols * rows];
        self.fg = vec![0; cols * rows];
        self.bg = vec![DEFAULT_BG; cols * rows];
        self.has_wide = false;
    }

    pub fn clear(&mut self) {
        self.glyph.fill(' ');
        self.fg.fill(0);
        self.bg.fill(DEFAULT_BG);
        self.has_wide = false;
    }

    pub fn text(&mut self, i: usize, glyph: char, rgb: u32, alpha: f64) {
        self.fg[i] = over(rgb, alpha, self.bg[i]);
        self.glyph[i] = glyph;
    }

    pub fn block(&mut self, i: usize, rgb: u32) {
        self.bg[i] = rgb;
        self.glyph[i] = ' ';
    }

    /// Writes `s` from (row, col), clipped to the grid; `bg` None keeps each cell's background.
    pub fn put(&mut self, row: usize, col: usize, s: &str, fg: u32, bg: Option<u32>) {
        if row >= self.rows {
            return;
        }
        let mut c = col;
        for ch in s.chars() {
            let w = width(ch);
            if c + w > self.cols {
                break;
            }
            let i = row * self.cols + c;
            if let Some(bg) = bg {
                self.bg[i] = bg;
            }
            self.text(i, ch, fg, 1.0);
            if w == 2 {
                self.bg[i + 1] = self.bg[i];
                self.glyph[i + 1] = WIDE_TAIL;
                self.has_wide = true;
            }
            c += w;
        }
    }
}

/// Source-over onto an opaque colour; the default background counts as black.
pub fn over(rgb: u32, alpha: f64, under: u32) -> u32 {
    if alpha >= 1.0 {
        return rgb;
    }
    let u = if under == DEFAULT_BG { 0 } else { under };
    let mix = |shift: u32| {
        js_round(
            ((rgb >> shift) & 255) as f64 * alpha + ((u >> shift) & 255) as f64 * (1.0 - alpha),
        ) as u32
    };
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

pub const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    (r << 16) | (g << 8) | b
}

/// Columns a glyph takes: 2 for East Asian wide and fullwidth characters.
pub fn width(c: char) -> usize {
    match c as u32 {
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

pub fn str_width(s: &str) -> usize {
    s.chars().map(width).sum()
}
