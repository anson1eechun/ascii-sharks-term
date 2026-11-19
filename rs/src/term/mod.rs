//! The terminal: raw mode, size, input and signals, on libc or the Windows console.

use std::io::{self, IsTerminal};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

#[cfg_attr(unix, path = "unix.rs")]
#[cfg_attr(windows, path = "windows.rs")]
mod sys;

pub use sys::{
    enter, install_signals, leave, read_input, suspend, take_signals, wait_input, write_all,
};

pub const ASPECT: RangeInclusive<f64> = 1.0..=4.0;

const ENTER: &[u8] = b"\x1b[?1049h\x1b[?25l\x1b[2J";
/// Any-motion mouse reports in SGR form, plus focus in/out.
pub const MOUSE_ON: &[u8] = b"\x1b[?1003h\x1b[?1006h\x1b[?1004h";
const LEAVE: &[u8] = b"\x1b[?1003l\x1b[?1006l\x1b[?1004l\x1b[0m\x1b[?25h\x1b[?1049l";

#[derive(Default)]
pub struct Signals {
    pub resized: bool,
    pub terminate: bool,
    pub suspend: bool,
    pub continued: bool,
}

pub fn is_tty() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// The terminal's size, else $COLUMNS × $LINES, else 80 × 24.
pub fn size() -> (usize, usize) {
    let env = |var: &str, fallback| {
        std::env::var(var)
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&n| n > 0)
            .unwrap_or(fallback)
    };
    sys::window_size().unwrap_or_else(|| (env("COLUMNS", 80), env("LINES", 24)))
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
            match read_input(&mut buf) {
                Ok(0) => break,
                Ok(n) => reply.extend_from_slice(&buf[..n]),
                Err(e) if is_transient(&e) => continue,
                Err(_) => break,
            }
            probe = parse_probe(&reply);
            if probe.done {
                break;
            }
        }
    }
    probe.aspect = probe
        .aspect
        .or_else(sys::pixel_aspect)
        .filter(|a| ASPECT.contains(a));
    probe
}

/// A read that found nothing this time: a signal, or console events without text.
pub fn is_transient(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::Interrupted | io::ErrorKind::WouldBlock
    )
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
