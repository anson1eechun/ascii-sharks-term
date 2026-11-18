use std::io;
use std::ops::RangeInclusive;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::grid::{DEFAULT_BG, Grid, WIDE_TAIL, width};

pub static RESIZED: AtomicBool = AtomicBool::new(false);
pub static TERMINATE: AtomicBool = AtomicBool::new(false);
pub static SUSPEND: AtomicBool = AtomicBool::new(false);
pub static CONTINUED: AtomicBool = AtomicBool::new(false);
static SAVED: OnceLock<libc::termios> = OnceLock::new();

pub const ASPECT: RangeInclusive<f64> = 1.0..=4.0;

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J";
/// Any-motion mouse reports in SGR form, plus focus in/out.
pub const MOUSE_ON: &[u8] = b"\x1b[?1003h\x1b[?1006h\x1b[?1004h";
const LEAVE: &[u8] = b"\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[0m\x1b[?25h\x1b[?1049l";

extern "C" fn on_signal(sig: libc::c_int) {
    let flag = match sig {
        libc::SIGWINCH => &RESIZED,
        libc::SIGTSTP => &SUSPEND,
        libc::SIGCONT => &CONTINUED,
        _ => &TERMINATE,
    };
    flag.store(true, Ordering::Relaxed);
}

pub fn is_tty() -> bool {
    unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 }
}

pub fn install_signals() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for sig in [
        libc::SIGWINCH,
        libc::SIGTERM,
        libc::SIGHUP,
        libc::SIGINT,
        libc::SIGTSTP,
        libc::SIGCONT,
    ] {
        unsafe { libc::signal(sig, handler) };
    }
}

pub fn enter() -> io::Result<()> {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            return Err(io::Error::last_os_error());
        }
        let mut raw = *SAVED.get_or_init(|| t);
        libc::cfmakeraw(&mut raw);
        if libc::tcsetattr(0, libc::TCSANOW, &raw) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    write_all(ENTER)
}

/// Undoes enter() and MOUSE_ON; safe to call more than once.
pub fn leave() {
    let _ = write_all(LEAVE);
    if let Some(t) = SAVED.get() {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, t) };
    }
}

/// Hands the terminal back and stops, as Ctrl-Z would; returns once resumed.
pub fn suspend() -> io::Result<()> {
    leave();
    unsafe { libc::raise(libc::SIGSTOP) };
    CONTINUED.store(false, Ordering::Relaxed);
    enter()
}

fn winsize() -> libc::winsize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        libc::ioctl(1, libc::TIOCGWINSZ, &mut ws);
        ws
    }
}

/// The terminal's size, else $COLUMNS × $LINES, else 80 × 24.
pub fn size() -> (usize, usize) {
    let ws = winsize();
    let or = |n: u16, var: &str, fallback| {
        if n > 0 {
            n as usize
        } else {
            std::env::var(var)
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(fallback)
        }
    };
    (or(ws.ws_col, "COLUMNS", 80), or(ws.ws_row, "LINES", 24))
}

pub struct Probe {
    pub aspect: Option<f64>,
    /// The terminal draws East Asian "ambiguous" glyphs such as `·` two columns wide.
    pub wide_ambiguous: bool,
}

/// Asks for the cell size (XTWINOPS 16) and where the cursor ends up after a `·`.
/// Every terminal answers the device-attributes query that follows, which ends the wait.
pub fn probe() -> Probe {
    let mut probe = Probe {
        aspect: None,
        wide_ambiguous: false,
    };
    if write_all("\x1b[16t\x1b[1;1H\u{b7}\x1b[6n\x1b[c".as_bytes()).is_ok() {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut reply = Vec::new();
        let mut buf = [0u8; 256];
        while wait_input(Some(deadline.saturating_duration_since(Instant::now()))) {
            let Ok(n @ 1..) = read_input(&mut buf) else {
                break;
            };
            reply.extend_from_slice(&buf[..n]);
            if reply.windows(3).any(|w| w == b"\x1b[?") && reply.ends_with(b"c") {
                break;
            }
        }
        if let Some([h, w]) = csi_numbers(&reply, b"\x1b[6;", b't') {
            probe.aspect = Some(h / w).filter(|a| ASPECT.contains(a));
        }
        if let Some([_, col]) = csi_numbers(&reply, b"\x1b[", b'R') {
            probe.wide_ambiguous = col >= 3.0;
        }
    }
    let ws = winsize();
    if probe.aspect.is_none()
        && ws.ws_xpixel > 0
        && ws.ws_ypixel > 0
        && ws.ws_col > 0
        && ws.ws_row > 0
    {
        let a = (ws.ws_ypixel as f64 / ws.ws_row as f64) / (ws.ws_xpixel as f64 / ws.ws_col as f64);
        probe.aspect = Some(a).filter(|a| ASPECT.contains(a));
    }
    probe
}

/// The two numbers of the first `prefix A ; B fin` in `buf`.
fn csi_numbers(buf: &[u8], prefix: &[u8], fin: u8) -> Option<[f64; 2]> {
    (0..buf.len())
        .filter(|&i| buf[i..].starts_with(prefix))
        .find_map(|i| {
            let body = &buf[i + prefix.len()..];
            let end = body
                .iter()
                .position(|&b| !(b.is_ascii_digit() || b == b';'))?;
            if body[end] != fin {
                return None;
            }
            let (a, b) = std::str::from_utf8(&body[..end]).ok()?.split_once(';')?;
            Some([a.parse().ok()?, b.parse().ok()?])
                .filter(|[a, b]: &[f64; 2]| *a > 0.0 && *b > 0.0)
        })
}

/// Waits for input or hang-up on stdin, forever when `timeout` is None; a signal ends it early.
pub fn wait_input(timeout: Option<Duration>) -> bool {
    let mut pfd = libc::pollfd {
        fd: 0,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.map_or(-1, |t| {
        t.as_micros().div_ceil(1000).min(i32::MAX as u128) as libc::c_int
    });
    unsafe { libc::poll(&mut pfd, 1, ms) > 0 }
}

/// Ok(0) means end of input.
pub fn read_input(buf: &mut [u8]) -> io::Result<usize> {
    let n = unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) };
    if n < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(n as usize)
    }
}

pub fn write_all(mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        let n = unsafe { libc::write(1, buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        buf = &buf[n as usize..];
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    TrueColor,
    Ansi256,
    Ansi16,
    Mono,
}

impl ColorMode {
    /// None for `auto`.
    pub fn parse(s: &str) -> Option<Option<Self>> {
        Some(match s {
            "auto" => None,
            "truecolor" | "24bit" => Some(ColorMode::TrueColor),
            "256" => Some(ColorMode::Ansi256),
            "16" => Some(ColorMode::Ansi16),
            "mono" | "none" => Some(ColorMode::Mono),
            _ => return None,
        })
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

    fn push_sgr(self, out: &mut Vec<u8>, bg: Option<u32>, fg: Option<u32>) {
        if bg.is_none() && fg.is_none() {
            return;
        }
        out.extend_from_slice(b"\x1b[");
        if let Some(b) = bg {
            self.push_bg(out, b);
        }
        if let Some(f) = fg {
            if bg.is_some() {
                out.push(b';');
            }
            self.push_fg(out, f);
        }
        out.push(b'm');
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

/// What the terminal shows: every cell, plus the cursor and the current colours.
pub struct Screen {
    mode: ColorMode,
    /// Sends `.` for `·`, for terminals that draw it two columns wide.
    ascii: bool,
    glyph: Vec<char>,
    fg: Vec<u32>,
    bg: Vec<u32>,
    has_wide: bool,
    pen: usize,
    cur_fg: u32,
    cur_bg: u32,
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
            pen: usize::MAX,
            cur_fg: UNKNOWN,
            cur_bg: UNKNOWN,
            out: Vec::with_capacity(1 << 16),
        }
    }

    /// Forgets everything, so the next flush repaints every cell.
    pub fn reset(&mut self, cells: usize) {
        self.glyph = vec![UNSET; cells];
        self.fg = vec![UNKNOWN; cells];
        self.bg = vec![UNKNOWN; cells];
        self.has_wide = false;
        self.pen = usize::MAX;
        self.cur_fg = UNKNOWN;
        self.cur_bg = UNKNOWN;
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
        let (cols, mode) = (grid.cols, self.mode);
        if grid.has_wide || self.has_wide {
            self.forget_around_wide_changes(grid);
        }
        let out = &mut self.out;
        out.clear();
        out.extend_from_slice(b"\x1b[?2026h");
        let header = out.len();

        for i in 0..grid.glyph.len() {
            let g = grid.glyph[i];
            let (f, b) = (mode.quantize(grid.fg[i]), mode.quantize(grid.bg[i]));
            let blank = g == ' ' || g == WIDE_TAIL;
            if g == self.glyph[i] && b == self.bg[i] && (blank || f == self.fg[i]) {
                continue;
            }
            (self.glyph[i], self.fg[i], self.bg[i]) = (g, f, b);
            if g == WIDE_TAIL {
                continue;
            }

            let pen = self.pen;
            // A pen at a row start means the last write filled the row above.
            if pen != i || i.is_multiple_of(cols) {
                out.extend_from_slice(b"\x1b[");
                if pen < i && pen / cols == i / cols && !pen.is_multiple_of(cols) {
                    push_num(out, (i - pen) as u32);
                    out.push(b'C');
                } else {
                    push_num(out, (i / cols + 1) as u32);
                    out.push(b';');
                    push_num(out, (i % cols + 1) as u32);
                    out.push(b'H');
                }
            }

            let set_bg = (b != self.cur_bg).then_some(b);
            let set_fg = (!blank && f != self.cur_fg && mode != ColorMode::Mono).then_some(f);
            mode.push_sgr(out, set_bg, set_fg);
            self.cur_bg = set_bg.unwrap_or(self.cur_bg);
            self.cur_fg = set_fg.unwrap_or(self.cur_fg);

            push_glyph(out, g, self.ascii);
            self.pen = i + width(g);
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
    /// cells beside it have to be sent again too.
    fn forget_around_wide_changes(&mut self, grid: &Grid) {
        let is_wide = |c: char| c == WIDE_TAIL || width(c) == 2;
        let n = self.glyph.len();
        for i in 0..n {
            let (old, new) = (self.glyph[i], grid.glyph[i]);
            if old != new && (is_wide(old) || is_wide(new)) {
                if i > 0 {
                    self.glyph[i - 1] = UNSET;
                }
                if i + 1 < n {
                    self.glyph[i + 1] = UNSET;
                }
            }
        }
    }
}

/// One frame as plain rows, for printing rather than painting a screen.
pub fn print(grid: &Grid, mode: ColorMode, ascii: bool) -> Vec<u8> {
    let mut out = Vec::new();
    for row in 0..grid.rows {
        let start = row * grid.cols;
        let end = (start..start + grid.cols)
            .rev()
            .find(|&i| grid.glyph[i] != ' ' || grid.bg[i] != DEFAULT_BG)
            .map_or(start, |i| i + 1);
        let (mut cur_fg, mut cur_bg) = (UNKNOWN, DEFAULT_BG);
        for i in start..end {
            let g = grid.glyph[i];
            if g == WIDE_TAIL {
                continue;
            }
            let (f, b) = (mode.quantize(grid.fg[i]), mode.quantize(grid.bg[i]));
            let set_bg = (b != cur_bg).then_some(b);
            let set_fg = (g != ' ' && f != cur_fg && mode != ColorMode::Mono).then_some(f);
            mode.push_sgr(&mut out, set_bg, set_fg);
            cur_bg = set_bg.unwrap_or(cur_bg);
            cur_fg = set_fg.unwrap_or(cur_fg);
            push_glyph(&mut out, g, ascii);
        }
        out.extend_from_slice(b"\x1b[0m\n");
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
    fn reads_replies() {
        let reply = b"\x1b[6;17;8t\x1b[1;3R\x1b[?62;22c";
        assert_eq!(csi_numbers(reply, b"\x1b[6;", b't'), Some([17.0, 8.0]));
        assert_eq!(csi_numbers(reply, b"\x1b[", b'R'), Some([1.0, 3.0]));
    }

    #[test]
    fn palette_mapping() {
        assert_eq!(to_256(rgb(255, 0, 0)), 196);
        assert_eq!(to_256(rgb(128, 128, 128)), 244);
        assert_eq!(to_16(rgb(10, 10, 10)), 0);
        assert_eq!(to_16(rgb(250, 250, 250)), 15);
    }
}
