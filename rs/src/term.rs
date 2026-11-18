use std::io;
use std::ops::RangeInclusive;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::time::{Duration, Instant};

static RESIZED: AtomicBool = AtomicBool::new(false);
static TERMINATE: AtomicBool = AtomicBool::new(false);
static SUSPEND: AtomicBool = AtomicBool::new(false);
static CONTINUED: AtomicBool = AtomicBool::new(false);
/// A signal writes a byte here so a poll() already under way wakes up for it.
static WAKE_WRITE: AtomicI32 = AtomicI32::new(-1);
static WAKE_READ: AtomicI32 = AtomicI32::new(-1);
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
    flag.store(true, Relaxed);
    let fd = WAKE_WRITE.load(Relaxed);
    if fd >= 0 {
        unsafe { libc::write(fd, [0u8].as_ptr().cast(), 1) };
    }
}

#[derive(Default)]
pub struct Signals {
    pub resized: bool,
    pub terminate: bool,
    pub suspend: bool,
    pub continued: bool,
}

/// The signals that arrived since the last call.
pub fn take_signals() -> Signals {
    Signals {
        resized: RESIZED.swap(false, Relaxed),
        terminate: TERMINATE.swap(false, Relaxed),
        suspend: SUSPEND.swap(false, Relaxed),
        continued: CONTINUED.swap(false, Relaxed),
    }
}

pub fn is_tty() -> bool {
    unsafe { libc::isatty(0) == 1 && libc::isatty(1) == 1 }
}

pub fn install_signals() {
    unsafe {
        let mut fds = [0; 2];
        if libc::pipe(fds.as_mut_ptr()) == 0 {
            for fd in fds {
                libc::fcntl(
                    fd,
                    libc::F_SETFL,
                    libc::fcntl(fd, libc::F_GETFL) | libc::O_NONBLOCK,
                );
                libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
            }
            WAKE_READ.store(fds[0], Relaxed);
            WAKE_WRITE.store(fds[1], Relaxed);
        }
        let handler = on_signal as extern "C" fn(libc::c_int) as libc::sighandler_t;
        for sig in [
            libc::SIGWINCH,
            libc::SIGTERM,
            libc::SIGHUP,
            libc::SIGINT,
            libc::SIGTSTP,
            libc::SIGCONT,
        ] {
            libc::signal(sig, handler);
        }
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

/// Hands the terminal back and stops until resumed. SIGSTOP, because SIGTSTP is
/// caught here and would only ask for this again.
pub fn suspend() -> io::Result<()> {
    leave();
    unsafe { libc::raise(libc::SIGSTOP) };
    CONTINUED.store(false, Relaxed);
    enter()
}

/// The size of whichever of stdout, stderr and stdin is a terminal, else
/// $COLUMNS × $LINES, else 80 × 24.
pub fn size() -> (usize, usize) {
    let ws = [1, 2, 0].into_iter().find_map(|fd| unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        (libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0)
            .then_some(ws)
    });
    let env = |var: &str, fallback| {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(fallback)
    };
    ws.map_or_else(
        || (env("COLUMNS", 80), env("LINES", 24)),
        |ws| (ws.ws_col as usize, ws.ws_row as usize),
    )
}

fn pixel_aspect() -> Option<f64> {
    let ws = unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        libc::ioctl(1, libc::TIOCGWINSZ, &mut ws);
        ws
    };
    if ws.ws_xpixel == 0 || ws.ws_ypixel == 0 || ws.ws_col == 0 || ws.ws_row == 0 {
        return None;
    }
    Some((ws.ws_ypixel as f64 / ws.ws_row as f64) / (ws.ws_xpixel as f64 / ws.ws_col as f64))
}

#[derive(Debug, Default, PartialEq)]
pub struct Probe {
    pub aspect: Option<f64>,
    /// The terminal draws East Asian "ambiguous" glyphs such as `·` two columns wide.
    pub wide_ambiguous: bool,
    done: bool,
}

/// Asks for the cell size (XTWINOPS 16) and where the cursor lands after a `·`. Every
/// terminal answers the device-attributes query sent last, which ends the wait.
pub fn probe() -> Probe {
    let mut probe = Probe::default();
    if write_all("\x1b[16t\x1b[1;1H\u{b7}\x1b[6n\x1b[c".as_bytes()).is_ok() {
        let deadline = Instant::now() + Duration::from_millis(500);
        let mut reply = Vec::new();
        let mut buf = [0u8; 256];
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if !wait_input(Some(left)) {
                continue;
            }
            let Ok(n @ 1..) = read_input(&mut buf) else {
                break;
            };
            reply.extend_from_slice(&buf[..n]);
            probe = parse_probe(&reply);
            if probe.done {
                break;
            }
        }
    }
    probe.aspect = probe
        .aspect
        .or_else(pixel_aspect)
        .filter(|a| ASPECT.contains(a));
    probe
}

fn parse_probe(reply: &[u8]) -> Probe {
    Probe {
        aspect: csi_numbers(reply, b"\x1b[6;", b't').map(|[h, w]| h / w),
        wide_ambiguous: csi_numbers(reply, b"\x1b[", b'R').is_some_and(|[_, col]| col >= 3.0),
        done: reply.windows(3).any(|w| w == b"\x1b[?") && reply.ends_with(b"c"),
    }
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

/// Waits for input on stdin, forever when `timeout` is None. False on a timeout or
/// a signal; the caller then checks take_signals().
pub fn wait_input(timeout: Option<Duration>) -> bool {
    let wake = WAKE_READ.load(Relaxed);
    let mut fds = [
        libc::pollfd {
            fd: 0,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: wake,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    let ms = timeout.map_or(-1, |t| {
        t.as_micros().div_ceil(1000).min(i32::MAX as u128) as libc::c_int
    });
    let n = if wake >= 0 { 2 } else { 1 };
    if unsafe { libc::poll(fds.as_mut_ptr(), n, ms) } <= 0 {
        return false;
    }
    if fds[1].revents != 0 {
        let mut sink = [0u8; 64];
        while unsafe { libc::read(wake, sink.as_mut_ptr().cast(), sink.len()) } > 0 {}
        return false;
    }
    fds[0].revents != 0
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_probe_replies() {
        let full = parse_probe(b"\x1b[6;17;8t\x1b[1;3R\x1b[?62;22c");
        assert_eq!(
            (full.aspect, full.wide_ambiguous, full.done),
            (Some(17.0 / 8.0), true, true)
        );
        let narrow = parse_probe(b"\x1b[1;2R\x1b[?1;2c");
        assert_eq!(
            (narrow.aspect, narrow.wide_ambiguous, narrow.done),
            (None, false, true)
        );
        assert!(!parse_probe(b"\x1b[6;17;8t\x1b[1;").done);
    }
}
