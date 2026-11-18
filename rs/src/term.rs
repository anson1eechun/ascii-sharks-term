use std::io;
use std::ops::RangeInclusive;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::renderer::{DEFAULT_BG, Grid};

pub static RESIZED: AtomicBool = AtomicBool::new(false);
pub static TERMINATE: AtomicBool = AtomicBool::new(false);
static SAVED: OnceLock<libc::termios> = OnceLock::new();

pub const ASPECT: RangeInclusive<f64> = 1.0..=4.0;

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J";
/// Any-motion mouse reports in SGR form, plus focus in/out.
pub const MOUSE_ON: &[u8] = b"\x1b[?1003h\x1b[?1006h\x1b[?1004h";
const LEAVE: &[u8] = b"\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[0m\x1b[?25h\x1b[?1049l";

extern "C" fn on_signal(sig: libc::c_int) {
    if sig == libc::SIGWINCH {
        RESIZED.store(true, Ordering::Relaxed);
    } else {
        TERMINATE.store(true, Ordering::Relaxed);
    }
}

pub fn is_tty() -> bool {
    unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 }
}

pub fn install_signals() {
    let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
    for sig in [libc::SIGWINCH, libc::SIGTERM, libc::SIGHUP, libc::SIGINT] {
        unsafe { libc::signal(sig, handler) };
    }
}

pub fn enter() -> io::Result<()> {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            return Err(io::Error::last_os_error());
        }
        let _ = SAVED.set(t);
        let mut raw = t;
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

fn winsize() -> libc::winsize {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        libc::ioctl(1, libc::TIOCGWINSZ, &mut ws);
        ws
    }
}

pub fn size() -> (usize, usize) {
    let ws = winsize();
    let or = |n: u16, fallback| if n > 0 { n as usize } else { fallback };
    (or(ws.ws_col, 80), or(ws.ws_row, 24))
}

pub fn cell_aspect() -> Option<f64> {
    let sane = |a: f64| ASPECT.contains(&a).then_some(a);

    if write_all(b"\x1b[16t").is_ok() {
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut reply = Vec::new();
        let mut buf = [0u8; 64];
        while wait_input(Some(deadline.saturating_duration_since(Instant::now()))) {
            let Ok(n @ 1..) = read_input(&mut buf) else {
                break;
            };
            reply.extend_from_slice(&buf[..n]);
            if let Some((h, w)) = parse_cell_size(&reply) {
                return sane(h / w);
            }
        }
    }

    let ws = winsize();
    if ws.ws_xpixel > 0 && ws.ws_ypixel > 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        return sane(
            (ws.ws_ypixel as f64 / ws.ws_row as f64) / (ws.ws_xpixel as f64 / ws.ws_col as f64),
        );
    }
    None
}

/// `ESC [ 6 ; height ; width t`
fn parse_cell_size(buf: &[u8]) -> Option<(f64, f64)> {
    let start = buf.windows(4).position(|w| w == b"\x1b[6;")? + 4;
    let end = start + buf[start..].iter().position(|&b| b == b't')?;
    let (h, w) = std::str::from_utf8(&buf[start..end])
        .ok()?
        .split_once(';')?;
    let (h, w): (f64, f64) = (h.parse().ok()?, w.parse().ok()?);
    (h > 0.0 && w > 0.0).then_some((h, w))
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

/// Never a real colour (those fit in 24 bits) nor DEFAULT_BG.
const UNKNOWN: u32 = u32::MAX - 1;

pub struct Screen {
    glyph: Vec<char>,
    fg: Vec<u32>,
    bg: Vec<u32>,
    pen: usize,
    cur_fg: u32,
    cur_bg: u32,
    out: Vec<u8>,
}

impl Screen {
    pub fn new() -> Self {
        Screen {
            glyph: Vec::new(),
            fg: Vec::new(),
            bg: Vec::new(),
            pen: usize::MAX,
            cur_fg: UNKNOWN,
            cur_bg: UNKNOWN,
            out: Vec::with_capacity(1 << 16),
        }
    }

    pub fn reset(&mut self, cells: usize) {
        self.glyph = vec!['\0'; cells];
        self.fg = vec![UNKNOWN; cells];
        self.bg = vec![UNKNOWN; cells];
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

    pub fn encode(&mut self, grid: &Grid) -> &[u8] {
        let cols = grid.cols;
        let out = &mut self.out;
        out.clear();
        out.extend_from_slice(b"\x1b[?2026h");
        let header = out.len();

        for i in 0..grid.glyph.len() {
            let (g, f, b) = (grid.glyph[i], grid.fg[i], grid.bg[i]);
            if g == self.glyph[i] && b == self.bg[i] && (g == ' ' || f == self.fg[i]) {
                continue;
            }
            let pen = self.pen;
            // A pen at a row start means the last write filled the row above.
            if pen != i || i.is_multiple_of(cols) {
                if pen < i && pen / cols == i / cols && !pen.is_multiple_of(cols) {
                    out.extend_from_slice(b"\x1b[");
                    push_num(out, (i - pen) as u32);
                    out.push(b'C');
                } else {
                    out.extend_from_slice(b"\x1b[");
                    push_num(out, (i / cols + 1) as u32);
                    out.push(b';');
                    push_num(out, (i % cols + 1) as u32);
                    out.push(b'H');
                }
            }

            let set_bg = b != self.cur_bg;
            let set_fg = g != ' ' && f != self.cur_fg;
            if set_bg || set_fg {
                out.extend_from_slice(b"\x1b[");
                if set_bg {
                    if b == DEFAULT_BG {
                        out.extend_from_slice(b"49");
                    } else {
                        push_rgb(out, b"48;2;", b);
                    }
                    self.cur_bg = b;
                }
                if set_fg {
                    if set_bg {
                        out.push(b';');
                    }
                    push_rgb(out, b"38;2;", f);
                    self.cur_fg = f;
                }
                out.push(b'm');
            }

            if g.is_ascii() {
                out.push(g as u8);
            } else {
                out.extend_from_slice(g.encode_utf8(&mut [0; 4]).as_bytes());
            }
            self.pen = i + 1;
            self.glyph[i] = g;
            self.fg[i] = f;
            self.bg[i] = b;
        }

        if out.len() == header {
            out.clear();
        } else {
            out.extend_from_slice(b"\x1b[?2026l");
        }
        out
    }
}

fn push_rgb(out: &mut Vec<u8>, prefix: &[u8], c: u32) {
    out.extend_from_slice(prefix);
    push_num(out, (c >> 16) & 255);
    out.push(b';');
    push_num(out, (c >> 8) & 255);
    out.push(b';');
    push_num(out, c & 255);
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
