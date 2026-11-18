use std::io;

use crate::grid::{DEFAULT_BG, Grid, WIDE_TAIL, width};
use crate::term::write_all;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

impl ColorMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "truecolor" | "24bit" => Some(ColorMode::TrueColor),
            "256" => Some(ColorMode::Ansi256),
            "16" => Some(ColorMode::Ansi16),
            "mono" | "none" => Some(ColorMode::Mono),
            _ => None,
        }
    }

    pub fn detect() -> Self {
        let var = |k| std::env::var(k).unwrap_or_default();
        let (colorterm, term) = (var("COLORTERM"), var("TERM"));
        if !var("NO_COLOR").is_empty() || term == "dumb" {
            ColorMode::Mono
        } else if colorterm == "truecolor" || colorterm == "24bit" {
            ColorMode::TrueColor
        } else if term == "linux" || term.starts_with("vt") || term.contains("16color") {
            ColorMode::Ansi16
        } else if var("TERM_PROGRAM") == "Apple_Terminal" {
            ColorMode::Ansi256
        } else {
            ColorMode::TrueColor
        }
    }

    /// The colour as this mode sends it: a palette index, or 0 for "set" in mono.
    fn quantize(self, c: u32) -> u32 {
        match self {
            _ if c == DEFAULT_BG => c,
            ColorMode::TrueColor => c,
            ColorMode::Ansi256 => to_256(c),
            ColorMode::Ansi16 => to_16(c),
            ColorMode::Mono => 0,
        }
    }

    fn push_fg(self, out: &mut Vec<u8>, c: u32) {
        match self {
            ColorMode::TrueColor => push_rgb(out, b"38;2;", c),
            ColorMode::Ansi256 => push_indexed(out, b"38;5;", c),
            ColorMode::Ansi16 => push_num(out, if c < 8 { 30 + c } else { 82 + c }),
            ColorMode::Mono => {}
        }
    }

    fn push_bg(self, out: &mut Vec<u8>, c: u32) {
        match self {
            ColorMode::Mono if c == DEFAULT_BG => out.extend_from_slice(b"27"),
            ColorMode::Mono => out.push(b'7'),
            _ if c == DEFAULT_BG => out.extend_from_slice(b"49"),
            ColorMode::TrueColor => push_rgb(out, b"48;2;", c),
            ColorMode::Ansi256 => push_indexed(out, b"48;5;", c),
            ColorMode::Ansi16 => push_num(out, if c < 8 { 40 + c } else { 92 + c }),
        }
    }
}

/// Remembers the last colour it converted; neighbouring cells mostly share one.
struct Quantizer {
    mode: ColorMode,
    last: Option<(u32, u32)>,
}

impl Quantizer {
    fn new(mode: ColorMode) -> Self {
        Quantizer { mode, last: None }
    }

    fn get(&mut self, c: u32) -> u32 {
        if self.mode == ColorMode::TrueColor {
            return c;
        }
        match self.last {
            Some((from, to)) if from == c => to,
            _ => {
                let to = self.mode.quantize(c);
                self.last = Some((c, to));
                to
            }
        }
    }
}

fn channels(c: u32) -> [u32; 3] {
    [(c >> 16) & 255, (c >> 8) & 255, c & 255]
}

fn distance(a: [u32; 3], b: [u32; 3]) -> u32 {
    (0..3).map(|i| a[i].abs_diff(b[i]).pow(2)).sum()
}

fn to_256(c: u32) -> u32 {
    let rgb = channels(c);
    let step = |v: u32| match v {
        0..48 => 0,
        48..115 => 1,
        _ => (v - 35) / 40,
    };
    let level = |s: u32| if s == 0 { 0 } else { 55 + 40 * s };
    let cube = rgb.map(step);
    let gray = (rgb.iter().sum::<u32>() / 3).saturating_sub(3).min(238) / 10;
    if distance(rgb, [8 + 10 * gray; 3]) < distance(rgb, cube.map(level)) {
        232 + gray
    } else {
        16 + 36 * cube[0] + 6 * cube[1] + cube[2]
    }
}

const ANSI16: [[u32; 3]; 16] = [
    [0, 0, 0],
    [205, 0, 0],
    [0, 205, 0],
    [205, 205, 0],
    [0, 0, 238],
    [205, 0, 205],
    [0, 205, 205],
    [229, 229, 229],
    [127, 127, 127],
    [255, 0, 0],
    [0, 255, 0],
    [255, 255, 0],
    [92, 92, 255],
    [255, 0, 255],
    [0, 255, 255],
    [255, 255, 255],
];

fn to_16(c: u32) -> u32 {
    let rgb = channels(c);
    (0..16)
        .min_by_key(|&i| distance(rgb, ANSI16[i]))
        .unwrap_or(0) as u32
}

/// Never a glyph or a colour any mode produces.
const UNKNOWN: u32 = u32::MAX - 1;
const UNSET: char = '\0';

#[derive(Clone, Copy, PartialEq)]
struct Pen {
    fg: u32,
    bg: u32,
}

impl Pen {
    const UNKNOWN: Pen = Pen {
        fg: UNKNOWN,
        bg: UNKNOWN,
    };
    const PLAIN: Pen = Pen {
        fg: UNKNOWN,
        bg: DEFAULT_BG,
    };

    /// Switches to a cell's colours in one escape sequence; the foreground only
    /// matters under a glyph.
    fn switch(&mut self, out: &mut Vec<u8>, mode: ColorMode, fg: u32, bg: u32, glyph: bool) {
        let set_bg = bg != self.bg;
        let set_fg = glyph && fg != self.fg && mode != ColorMode::Mono;
        if !set_bg && !set_fg {
            return;
        }
        out.extend_from_slice(b"\x1b[");
        if set_bg {
            mode.push_bg(out, bg);
            self.bg = bg;
        }
        if set_fg {
            if set_bg {
                out.push(b';');
            }
            mode.push_fg(out, fg);
            self.fg = fg;
        }
        out.push(b'm');
    }
}

/// What the terminal shows: every cell, the cursor, and the pen.
pub struct Screen {
    mode: ColorMode,
    /// Sends `.` for `·`, for terminals that draw it two columns wide.
    ascii: bool,
    glyph: Vec<char>,
    fg: Vec<u32>,
    bg: Vec<u32>,
    has_wide: bool,
    cursor: usize,
    pen: Pen,
    out: Vec<u8>,
}

impl Screen {
    pub fn new(mode: ColorMode, ascii: bool) -> Self {
        Screen {
            mode,
            ascii,
            glyph: Vec::new(),
            fg: Vec::new(),
            bg: Vec::new(),
            has_wide: false,
            cursor: usize::MAX,
            pen: Pen::UNKNOWN,
            out: Vec::with_capacity(1 << 16),
        }
    }

    /// Forgets everything, so the next flush repaints every cell.
    pub fn reset(&mut self, cells: usize) {
        self.glyph = vec![UNSET; cells];
        self.fg = vec![UNKNOWN; cells];
        self.bg = vec![UNKNOWN; cells];
        self.has_wide = false;
        self.cursor = usize::MAX;
        self.pen = Pen::UNKNOWN;
    }

    pub fn flush(&mut self, grid: &Grid) -> io::Result<()> {
        let out = self.encode(grid);
        if out.is_empty() {
            Ok(())
        } else {
            write_all(out)
        }
    }

    /// The bytes that bring the terminal up to date with `grid`, as one synchronized
    /// update; empty when nothing changed.
    pub fn encode(&mut self, grid: &Grid) -> &[u8] {
        let cols = grid.cols;
        if grid.has_wide || self.has_wide {
            self.forget_around_wide_changes(grid);
        }
        let (mut fgq, mut bgq) = (Quantizer::new(self.mode), Quantizer::new(self.mode));
        let out = &mut self.out;
        out.clear();
        out.extend_from_slice(b"\x1b[?2026h");
        let header = out.len();

        for i in 0..grid.glyph.len() {
            let g = grid.glyph[i];
            let blank = g == ' ' || g == WIDE_TAIL;
            let b = bgq.get(grid.bg[i]);
            let f = if blank {
                self.fg[i]
            } else {
                fgq.get(grid.fg[i])
            };
            if g == self.glyph[i] && b == self.bg[i] && f == self.fg[i] {
                continue;
            }
            (self.glyph[i], self.fg[i], self.bg[i]) = (g, f, b);
            if g == WIDE_TAIL {
                continue;
            }

            let cursor = self.cursor;
            // A cursor at a row start means the last write filled the row above.
            if cursor != i || i.is_multiple_of(cols) {
                out.extend_from_slice(b"\x1b[");
                if cursor < i && cursor / cols == i / cols && !cursor.is_multiple_of(cols) {
                    push_num(out, (i - cursor) as u32);
                    out.push(b'C');
                } else {
                    push_num(out, (i / cols + 1) as u32);
                    out.push(b';');
                    push_num(out, (i % cols + 1) as u32);
                    out.push(b'H');
                }
            }
            self.pen.switch(out, self.mode, f, b, !blank);
            push_glyph(out, g, self.ascii);
            self.cursor = i + width(g).max(1);
        }
        self.has_wide = grid.has_wide;

        if out.len() == header {
            out.clear();
        } else {
            out.extend_from_slice(b"\x1b[?2026l");
        }
        out
    }

    /// A terminal redraws a wide glyph as a whole, so where one appears or goes, the
    /// cells beside it are sent again too.
    fn forget_around_wide_changes(&mut self, grid: &Grid) {
        let is_wide = |c: char| c == WIDE_TAIL || width(c) == 2;
        let n = self.glyph.len();
        for i in 0..n {
            let (old, new) = (self.glyph[i], grid.glyph[i]);
            if old != new && (is_wide(old) || is_wide(new)) {
                for j in [i.wrapping_sub(1), i + 1] {
                    if j < n {
                        self.bg[j] = UNKNOWN;
                    }
                }
            }
        }
    }
}

/// One frame as plain rows, for printing rather than painting a screen.
pub fn print(grid: &Grid, mode: ColorMode, ascii: bool) -> Vec<u8> {
    let mut out = Vec::new();
    let (mut fgq, mut bgq) = (Quantizer::new(mode), Quantizer::new(mode));
    for row in 0..grid.rows {
        let start = row * grid.cols;
        let end = (start..start + grid.cols)
            .rev()
            .find(|&i| grid.glyph[i] != ' ' || grid.bg[i] != DEFAULT_BG)
            .map_or(start, |i| i + 1);
        let mut pen = Pen::PLAIN;
        for i in start..end {
            let g = grid.glyph[i];
            if g == WIDE_TAIL {
                continue;
            }
            let glyph = g != ' ';
            let f = if glyph { fgq.get(grid.fg[i]) } else { pen.fg };
            pen.switch(&mut out, mode, f, bgq.get(grid.bg[i]), glyph);
            push_glyph(&mut out, g, ascii);
        }
        if pen != Pen::PLAIN {
            out.extend_from_slice(b"\x1b[0m");
        }
        out.push(b'\n');
    }
    out
}

fn push_glyph(out: &mut Vec<u8>, g: char, ascii: bool) {
    match g {
        '\u{b7}' if ascii => out.push(b'.'),
        g if g.is_ascii() => out.push(g as u8),
        g => out.extend_from_slice(g.encode_utf8(&mut [0; 4]).as_bytes()),
    }
}

fn push_rgb(out: &mut Vec<u8>, prefix: &[u8], c: u32) {
    out.extend_from_slice(prefix);
    let [r, g, b] = channels(c);
    push_num(out, r);
    out.push(b';');
    push_num(out, g);
    out.push(b';');
    push_num(out, b);
}

fn push_indexed(out: &mut Vec<u8>, prefix: &[u8], n: u32) {
    out.extend_from_slice(prefix);
    push_num(out, n);
}

fn push_num(out: &mut Vec<u8>, mut n: u32) {
    let mut digits = [0u8; 10];
    let mut i = digits.len();
    loop {
        i -= 1;
        digits[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    out.extend_from_slice(&digits[i..]);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::grid::rgb;

    fn grid(cols: usize, rows: usize) -> Grid {
        let mut g = Grid::new();
        g.resize(cols, rows);
        g
    }

    fn frame(screen: &mut Screen, g: &Grid) -> String {
        let s = String::from_utf8(screen.encode(g).to_vec()).unwrap();
        s.trim_start_matches("\x1b[?2026h")
            .trim_end_matches("\x1b[?2026l")
            .to_string()
    }

    #[test]
    fn sends_only_changes_with_short_moves_and_merged_colours() {
        let mut g = grid(10, 2);
        let mut s = Screen::new(ColorMode::TrueColor, false);
        s.reset(20);
        frame(&mut s, &g);
        assert_eq!(frame(&mut s, &g), "");

        g.text(2, 'a', rgb(1, 2, 3), 1.0);
        g.text(6, 'b', rgb(1, 2, 3), 1.0);
        g.block(13, rgb(9, 9, 9));
        g.text(13, 'c', rgb(4, 5, 6), 1.0);
        assert_eq!(
            frame(&mut s, &g),
            "\x1b[1;3H\x1b[38;2;1;2;3ma\x1b[3Cb\x1b[2;4H\x1b[48;2;9;9;9;38;2;4;5;6mc"
        );
    }

    #[test]
    fn colour_modes() {
        let mut g = grid(3, 1);
        g.block(0, rgb(255, 0, 0));
        g.text(1, 'x', rgb(0, 0, 238), 1.0);
        for (mode, want) in [
            (
                ColorMode::Ansi256,
                "\x1b[1;1H\x1b[48;5;196m \x1b[49;38;5;21mx ",
            ),
            (ColorMode::Ansi16, "\x1b[1;1H\x1b[101m \x1b[49;34mx "),
            (ColorMode::Mono, "\x1b[1;1H\x1b[7m \x1b[27mx "),
        ] {
            let mut s = Screen::new(mode, false);
            s.reset(3);
            assert_eq!(frame(&mut s, &g), want, "{mode:?}");
        }
    }

    #[test]
    fn wide_glyphs_take_two_columns_and_repaint_their_neighbours() {
        let mut g = grid(6, 1);
        let mut s = Screen::new(ColorMode::Mono, false);
        s.reset(6);
        g.put(0, 1, "虎x", 0, None);
        assert_eq!(frame(&mut s, &g), "\x1b[1;1H\x1b[27m 虎x  ");
        g.clear();
        g.put(0, 2, "y", 0, None);
        assert_eq!(frame(&mut s, &g), "\x1b[1;1H  y ");
        g.clear();
        g.put(0, 0, "鯊魚", 0, None);
        frame(&mut s, &g);
        g.put(0, 0, "虎", 0, None);
        assert_eq!(
            frame(&mut s, &g),
            "\x1b[1;1H虎",
            "the unchanged 魚 is not resent"
        );
    }

    #[test]
    fn ascii_mode_sends_a_plain_dot() {
        let mut g = grid(2, 1);
        g.text(0, '\u{b7}', 0, 1.0);
        let mut s = Screen::new(ColorMode::Mono, true);
        s.reset(2);
        assert_eq!(frame(&mut s, &g), "\x1b[1;1H\x1b[27m. ");
    }

    #[test]
    fn print_keeps_rows_tidy() {
        let mut g = grid(6, 3);
        g.put(1, 1, "虎", rgb(9, 9, 9), None);
        let printed = String::from_utf8(print(&g, ColorMode::TrueColor, false)).unwrap();
        assert_eq!(printed, "\n \x1b[38;2;9;9;9m虎\x1b[0m\n\n");
        assert_eq!(print(&g, ColorMode::Mono, false), "\n 虎\n\n".as_bytes());
    }

    #[test]
    fn palette_mapping() {
        assert_eq!(to_256(rgb(255, 0, 0)), 196);
        assert_eq!(to_256(rgb(128, 128, 128)), 244);
        assert_eq!(to_16(rgb(10, 10, 10)), 0);
        assert_eq!(to_16(rgb(250, 250, 250)), 15);
    }
}
