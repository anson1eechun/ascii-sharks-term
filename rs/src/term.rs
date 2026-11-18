//! Terminal I/O on plain libc: raw mode, size, signals, input polling, and a
//! diffing writer that sends only the cells that changed as 24-bit ANSI.

use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::renderer::{DEFAULT_BG, Grid};

pub static RESIZED: AtomicBool = AtomicBool::new(false);
pub static TERMINATE: AtomicBool = AtomicBool::new(false);
static SAVED: OnceLock<libc::termios> = OnceLock::new();

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

/// Raw mode on the alternate screen, cursor hidden.
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

/// Undo enter() and MOUSE_ON. Safe to call more than once.
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

/// (columns, rows), falling back to 80×24 when the terminal won't say.
pub fn size() -> (usize, usize) {
    let ws = winsize();
    let cols = if ws.ws_col > 0 { ws.ws_col as usize } else { 80 };
    let rows = if ws.ws_row > 0 { ws.ws_row as usize } else { 24 };
    (cols, rows)
}

/// Height / width of one character cell. Asks the terminal (XTWINOPS 16),
/// then falls back to the pixel size in the window-size ioctl.
pub fn cell_aspect() -> Option<f64> {
    let sane = |a: f64| (1.0..=4.0).contains(&a).then_some(a);

    if write_all(b"\x1b[16t").is_ok() {
        let deadline = Instant::now() + Duration::from_millis(250);
        let mut reply = Vec::new();
        let mut buf = [0u8; 64];
        while wait_input(deadline.saturating_duration_since(Instant::now())) {
            let n = read_input(&mut buf);
            if n <= 0 {
                break;
            }
            reply.extend_from_slice(&buf[..n as usize]);
            if let Some((h, w)) = parse_cell_size(&reply) {
                return sane(h / w);
            }
        }
    }

    let ws = winsize();
    if ws.ws_xpixel > 0 && ws.ws_ypixel > 0 && ws.ws_col > 0 && ws.ws_row > 0 {
        return sane((ws.ws_ypixel as f64 / ws.ws_row as f64) / (ws.ws_xpixel as f64 / ws.ws_col as f64));
    }
    None
}

/// Parses `ESC [ 6 ; height ; width t`.
fn parse_cell_size(buf: &[u8]) -> Option<(f64, f64)> {
    let start = buf.windows(4).position(|w| w == b"\x1b[6;")? + 4;
    let end = start + buf[start..].iter().position(|&b| b == b't')?;
    let (h, w) = std::str::from_utf8(&buf[start..end]).ok()?.split_once(';')?;
    let (h, w): (f64, f64) = (h.parse().ok()?, w.parse().ok()?);
    (h > 0.0 && w > 0.0).then_some((h, w))
}

/// Waits up to `timeout` for input (or hang-up) on stdin. Returns early on a signal.
pub fn wait_input(timeout: Duration) -> bool {
    let mut pfd = libc::pollfd { fd: 0, events: libc::POLLIN, revents: 0 };
    let ms = timeout.as_micros().div_ceil(1000).min(i32::MAX as u128) as libc::c_int;
    unsafe { libc::poll(&mut pfd, 1, ms) > 0 }
}

/// read(2) on stdin: bytes read, 0 at end of input, negative on error.
pub fn read_input(buf: &mut [u8]) -> isize {
    unsafe { libc::read(0, buf.as_mut_ptr().cast(), buf.len()) }
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

/// What the terminal currently shows, so each frame sends only the changes.
pub struct Screen {
    glyph: Vec<char>,
    fg: Vec<u32>,
    bg: Vec<u32>,
    out: Vec<u8>,
}

impl Screen {
    pub fn new() -> Self {
        Screen { glyph: Vec::new(), fg: Vec::new(), bg: Vec::new(), out: Vec::with_capacity(1 << 16) }
    }

    /// Forget the screen contents: the next flush repaints every cell.
    pub fn reset(&mut self, cells: usize) {
        self.glyph = vec!['\0'; cells];
        self.fg = vec![UNKNOWN; cells];
        self.bg = vec![UNKNOWN; cells];
    }

    /// Writes the cells that changed since the last flush, as one synchronized update.
    pub fn flush(&mut self, grid: &Grid) -> io::Result<()> {
        let out = &mut self.out;
        out.clear();
        out.extend_from_slice(b"\x1b[?2026h");
        let header = out.len();
        let cols = grid.cols;
        let mut pen = usize::MAX;
        let mut cur_fg = UNKNOWN;
        let mut cur_bg = UNKNOWN;

        for i in 0..grid.glyph.len() {
            let (g, f, b) = (grid.glyph[i], grid.fg[i], grid.bg[i]);
            if g == self.glyph[i] && b == self.bg[i] && (g == ' ' || f == self.fg[i]) {
                continue;
            }
            if pen != i || i % cols == 0 {
                out.extend_from_slice(b"\x1b[");
                push_num(out, (i / cols + 1) as u32);
                out.push(b';');
                push_num(out, (i % cols + 1) as u32);
                out.push(b'H');
            }
            if b != cur_bg {
                if b == DEFAULT_BG {
                    out.extend_from_slice(b"\x1b[49m");
                } else {
                    push_sgr_rgb(out, b"\x1b[48;2;", b);
                }
                cur_bg = b;
            }
            if g != ' ' && f != cur_fg {
                push_sgr_rgb(out, b"\x1b[38;2;", f);
                cur_fg = f;
            }
            let mut utf8 = [0u8; 4];
            out.extend_from_slice(g.encode_utf8(&mut utf8).as_bytes());
            pen = i + 1;
            self.glyph[i] = g;
            self.fg[i] = f;
            self.bg[i] = b;
        }

        if out.len() == header {
            return Ok(());
        }
        out.extend_from_slice(b"\x1b[0m\x1b[?2026l");
        write_all(out)
    }
}

fn push_sgr_rgb(out: &mut Vec<u8>, prefix: &[u8], c: u32) {
    out.extend_from_slice(prefix);
    push_num(out, (c >> 16) & 255);
    out.push(b';');
    push_num(out, (c >> 8) & 255);
    out.push(b';');
    push_num(out, c & 255);
    out.push(b'm');
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
