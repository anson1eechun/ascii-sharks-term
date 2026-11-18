//! The binary driven through a pseudo-terminal, as a user would.

use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BIN: &str = env!("CARGO_BIN_EXE_sharks");
const PATIENCE: Duration = Duration::from_secs(10);
const ENTER: &[u8] = b"\x1b[?1049h";
const LEAVE: &[u8] = b"\x1b[?1049l";
const MOUSE_ON: &[u8] = b"\x1b[?1003h";
const FRAME_END: &[u8] = b"\x1b[?2026l";
const NARROW: &[u8] = b"\x1b[1;2R\x1b[?62;22c";
const WIDE: &[u8] = b"\x1b[1;3R\x1b[?62;22c";

type Output = Arc<(Mutex<Vec<u8>>, Condvar)>;

struct Term {
    child: Child,
    master: File,
    out: Output,
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn window(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

impl Term {
    /// Starts sharks in a `cols` × `rows` terminal that answers its startup probe
    /// with `reply`.
    fn open(args: &[&str], cols: u16, rows: u16, reply: &[u8]) -> Term {
        let (mut master, mut slave) = (0, 0);
        let mut ws = window(cols, rows);
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut ws,
            )
        };
        assert_eq!(opened, 0, "openpty: {}", std::io::Error::last_os_error());
        unsafe { libc::fcntl(master, libc::F_SETFD, libc::FD_CLOEXEC) };
        let master = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { OwnedFd::from_raw_fd(slave) };

        let mut cmd = Command::new(BIN);
        cmd.args(args)
            .env_clear()
            .env("TERM", "xterm-256color")
            .env("COLORTERM", "truecolor")
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave));
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = cmd.spawn().unwrap();
        drop(cmd);

        let out: Output = Arc::default();
        let (sink, mut reader) = (out.clone(), master.try_clone().unwrap());
        thread::spawn(move || {
            let mut buf = [0u8; 16384];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                let (bytes, ready) = &*sink;
                bytes.lock().unwrap().extend_from_slice(&buf[..n]);
                ready.notify_all();
            }
        });
        let mut term = Term { child, master, out };
        term.wait_for(b"\x1b[c", 0);
        term.send(reply);
        term
    }

    fn output(&self) -> Vec<u8> {
        self.out.0.lock().unwrap().clone()
    }

    fn len(&self) -> usize {
        self.out.0.lock().unwrap().len()
    }

    /// Waits for `what` to appear at or after `from`; returns where it ends.
    fn wait_for(&self, what: &[u8], from: usize) -> usize {
        let (bytes, ready) = &*self.out;
        let deadline = Instant::now() + PATIENCE;
        let mut out = bytes.lock().unwrap();
        loop {
            if let Some(i) = out.get(from..).and_then(|o| find(o, what)) {
                return from + i + what.len();
            }
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "no {:?} after byte {from} of {}",
                String::from_utf8_lossy(what),
                out.len()
            );
            out = ready.wait_timeout(out, left).unwrap().0;
        }
    }

    /// The next whole frame after `from`; returns where it ends.
    fn next_frame(&self, from: usize) -> usize {
        self.wait_for(FRAME_END, from)
    }

    fn send(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    fn resize(&self, cols: u16, rows: u16) {
        let ws = window(cols, rows);
        assert_eq!(
            unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &ws) },
            0
        );
    }

    fn pid(&self) -> libc::pid_t {
        self.child.id() as libc::pid_t
    }

    fn exit(&mut self) -> ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return status;
            }
            assert!(Instant::now() < deadline, "sharks did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn quit(&mut self) {
        self.send(b"q");
        assert!(self.exit().success());
    }
}

impl Drop for Term {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Every `ESC [ row ; col H` in `out`.
fn cursor_moves(out: &[u8]) -> Vec<(u32, u32)> {
    let text = String::from_utf8_lossy(out);
    text.split("\x1b[")
        .skip(1)
        .filter_map(|s| {
            let end = s.find(|c: char| !(c.is_ascii_digit() || c == ';'))?;
            if !s[end..].starts_with('H') {
                return None;
            }
            let (row, col) = s[..end].split_once(';')?;
            Some((row.parse().ok()?, col.parse().ok()?))
        })
        .collect()
}

/// The size of each frame after `from`.
fn frame_sizes(out: &[u8], from: usize) -> Vec<usize> {
    let text = &out[from..];
    let mut sizes = Vec::new();
    let mut at = 0;
    while let Some(start) = find(&text[at..], b"\x1b[?2026h") {
        let Some(end) = find(&text[at + start..], FRAME_END) else {
            break;
        };
        sizes.push(end + FRAME_END.len());
        at += start + end + FRAME_END.len();
    }
    sizes
}

#[test]
fn quits_on_q_esc_and_ctrl_c_and_hands_the_terminal_back() {
    for key in [&b"q"[..], b"Q", b"\x1b", b"\x03"] {
        let mut term = Term::open(&["--seed", "1"], 80, 24, NARROW);
        let first = term.next_frame(0);
        term.send(key);
        assert!(term.exit().success(), "{key:?}");
        let out = term.output();
        assert!(find(&out, MOUSE_ON).is_some());
        let leave = find(&out[first..], LEAVE).map(|i| first + i);
        assert!(leave.is_some(), "{key:?} left the alternate screen on");
        assert!(
            find(&out[leave.unwrap()..], FRAME_END).is_none(),
            "drew after leaving"
        );
    }
}

#[test]
fn resizing_redraws_at_the_new_size() {
    let mut term = Term::open(&["--seed", "2"], 80, 24, NARROW);
    let mut at = term.next_frame(0);
    for _ in 0..5 {
        at = term.next_frame(at);
    }
    let before = term.output();
    assert!(
        cursor_moves(&before)
            .iter()
            .all(|&(r, c)| r <= 24 && c <= 80)
    );

    term.resize(120, 36);
    let resized = term.len();
    let deadline = Instant::now() + PATIENCE;
    while !cursor_moves(&term.output()[resized..])
        .iter()
        .any(|&(r, c)| r > 24 || c > 80)
    {
        assert!(Instant::now() < deadline, "never drew beyond 80 × 24");
        at = term.next_frame(at);
    }
    term.quit();
}

#[test]
fn ctrl_z_suspends_and_sigcont_resumes() {
    let mut term = Term::open(&["--seed", "3"], 80, 24, NARROW);
    let first = term.next_frame(0);
    term.send(b"\x1a");

    let deadline = Instant::now() + PATIENCE;
    loop {
        let mut status = 0;
        let pid =
            unsafe { libc::waitpid(term.pid(), &mut status, libc::WUNTRACED | libc::WNOHANG) };
        if pid == term.pid() && libc::WIFSTOPPED(status) {
            break;
        }
        assert!(Instant::now() < deadline, "Ctrl-Z did not stop sharks");
        thread::sleep(Duration::from_millis(10));
    }
    let stopped = term.wait_for(LEAVE, first);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(
        frame_sizes(&term.output(), stopped).len(),
        0,
        "drew while stopped"
    );

    unsafe { libc::kill(term.pid(), libc::SIGCONT) };
    let back = term.wait_for(ENTER, stopped);
    term.next_frame(back);
    term.quit();
}

#[test]
fn pause_stops_the_frames_until_pressed_again() {
    let mut term = Term::open(&["--seed", "4"], 80, 24, NARROW);
    let at = term.next_frame(0);
    term.send(b"p");
    thread::sleep(Duration::from_millis(150));
    let paused = term.len();
    thread::sleep(Duration::from_millis(600));
    assert_eq!(term.len(), paused, "drew while paused");
    term.send(b"p");
    term.next_frame(paused.max(at));
    term.quit();
}

#[test]
fn still_mode_redraws_only_the_clock() {
    let mut term = Term::open(
        &["--still", "--clock", "--seconds", "--seed", "5"],
        100,
        30,
        NARROW,
    );
    let first = term.next_frame(0);
    let full = *frame_sizes(&term.output(), 0).last().unwrap();
    assert!(full > 3000, "the first frame paints the screen: {full} B");
    thread::sleep(Duration::from_millis(2300));
    let ticks = frame_sizes(&term.output(), first);
    assert!(
        (1..=4).contains(&ticks.len()),
        "{} ticks in 2.3 s",
        ticks.len()
    );
    assert!(
        ticks.iter().all(|&n| n < 1000),
        "ticks repaint the sea: {ticks:?}"
    );
    term.quit();

    let mut term = Term::open(&["--still", "--seed", "5"], 100, 30, NARROW);
    term.next_frame(0);
    let painted = term.len();
    thread::sleep(Duration::from_millis(1200));
    assert_eq!(
        term.len(),
        painted,
        "a still sea without a clock never redraws"
    );
    term.quit();
}

#[test]
fn the_screensaver_ignores_the_mouse_and_quits_on_any_key() {
    let mut term = Term::open(&["-S", "--seed", "6"], 80, 24, NARROW);
    term.next_frame(0);
    assert!(find(&term.output(), MOUSE_ON).is_none());
    term.send(b"\x1b[A");
    assert!(term.exit().success());
}

#[test]
fn wide_middle_dots_are_sent_as_full_stops() {
    let dots = |reply: &[u8]| {
        let mut term = Term::open(&["--seed", "42", "--rain"], 100, 30, reply);
        let probed = term.wait_for(b"\x1b[c", 0);
        let mut at = term.next_frame(probed);
        for _ in 0..20 {
            at = term.next_frame(at);
        }
        term.quit();
        find(&term.output()[probed..], "\u{b7}".as_bytes()).is_some()
    };
    assert!(dots(NARROW), "the scene has middle dots to begin with");
    assert!(
        !dots(WIDE),
        "a terminal that draws · two columns wide got one"
    );
}

#[test]
fn print_sizes_itself_from_the_environment() {
    let rows = |lines: &str| {
        let out = Command::new(BIN)
            .args(["--print", "--seed", "7", "--color", "mono"])
            .env_clear()
            .env("COLUMNS", "50")
            .env("LINES", lines)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap().lines().count()
    };
    assert_eq!(rows("7"), 7);
    assert_eq!(rows("0"), 24, "LINES=0 falls back to 24");
}

#[test]
fn bad_options_fail_with_a_message() {
    for args in [&["-k", ","][..], &["--clock=sundial"], &["-f", "0"]] {
        let out = Command::new(BIN).args(args).env_clear().output().unwrap();
        assert!(!out.status.success(), "{args:?} was accepted");
        assert!(
            String::from_utf8_lossy(&out.stderr).starts_with("sharks: "),
            "{args:?}"
        );
    }
}
