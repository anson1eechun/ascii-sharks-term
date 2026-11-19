use std::io;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering::Relaxed};
use std::time::Duration;

use super::{ENTER, LEAVE, Signals};

static RESIZED: AtomicBool = AtomicBool::new(false);
static TERMINATE: AtomicBool = AtomicBool::new(false);
static SUSPEND: AtomicBool = AtomicBool::new(false);
static CONTINUED: AtomicBool = AtomicBool::new(false);
/// A signal writes a byte here so a poll() already under way wakes up for it.
static WAKE_WRITE: AtomicI32 = AtomicI32::new(-1);
static WAKE_READ: AtomicI32 = AtomicI32::new(-1);
static SAVED: OnceLock<libc::termios> = OnceLock::new();

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

/// The signals that arrived since the last call.
pub fn take_signals() -> Signals {
    Signals {
        resized: RESIZED.swap(false, Relaxed),
        terminate: TERMINATE.swap(false, Relaxed),
        suspend: SUSPEND.swap(false, Relaxed),
        continued: CONTINUED.swap(false, Relaxed),
    }
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

fn winsize(fd: libc::c_int) -> Option<libc::winsize> {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        (libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0 && ws.ws_row > 0)
            .then_some(ws)
    }
}

/// The size of whichever of stdout, stderr and stdin is a terminal.
pub(super) fn window_size() -> Option<(usize, usize)> {
    [1, 2, 0]
        .into_iter()
        .find_map(winsize)
        .map(|ws| (ws.ws_col as usize, ws.ws_row as usize))
}

pub(super) fn pixel_aspect() -> Option<f64> {
    let ws = winsize(1)?;
    if ws.ws_xpixel == 0 || ws.ws_ypixel == 0 {
        return None;
    }
    Some((ws.ws_ypixel as f64 / ws.ws_row as f64) / (ws.ws_xpixel as f64 / ws.ws_col as f64))
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
