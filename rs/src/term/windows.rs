//! The Windows console with virtual-terminal sequences turned on. Input records
//! become the bytes a Unix terminal would send, so one parser reads both.

use std::cell::{Cell, RefCell};
use std::io::{self, Write};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering::Relaxed};
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{FALSE, HANDLE, TRUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{
    CONSOLE_SCREEN_BUFFER_INFO, DISABLE_NEWLINE_AUTO_RETURN, ENABLE_EXTENDED_FLAGS,
    ENABLE_MOUSE_INPUT, ENABLE_PROCESSED_OUTPUT, ENABLE_VIRTUAL_TERMINAL_INPUT,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, ENABLE_WINDOW_INPUT, FOCUS_EVENT,
    FROM_LEFT_1ST_BUTTON_PRESSED, GetConsoleMode, GetConsoleScreenBufferInfo,
    GetNumberOfConsoleInputEvents, GetStdHandle, INPUT_RECORD, KEY_EVENT, MOUSE_EVENT,
    MOUSE_EVENT_RECORD, MOUSE_HWHEELED, MOUSE_MOVED, MOUSE_WHEELED, RIGHTMOST_BUTTON_PRESSED,
    ReadConsoleInputW, STD_ERROR_HANDLE, STD_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    SetConsoleCtrlHandler, SetConsoleMode, WINDOW_BUFFER_SIZE_EVENT,
};
use windows_sys::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects,
};

use super::{ENTER, LEAVE, Signals};

static RESIZED: AtomicBool = AtomicBool::new(false);
static TERMINATE: AtomicBool = AtomicBool::new(false);
/// An event the control handler sets so a wait already under way wakes up.
static WAKE: AtomicUsize = AtomicUsize::new(0);
/// The console modes found at start, restored on the way out.
static SAVED: OnceLock<(u32, u32)> = OnceLock::new();
/// Set once the terminal reports the mouse as escape sequences itself, as Windows
/// Terminal does; from then on the console's own mouse records are ignored.
static VT_MOUSE: AtomicBool = AtomicBool::new(false);
static BUTTONS: AtomicU32 = AtomicU32::new(0);

thread_local! {
    /// Bytes translated from input records and not yet read.
    static QUEUE: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
    /// The first half of a UTF-16 surrogate pair, until the second arrives.
    static HIGH: Cell<u16> = const { Cell::new(0) };
}

fn handle(which: STD_HANDLE) -> HANDLE {
    unsafe { GetStdHandle(which) }
}

unsafe extern "system" fn on_ctrl(_: u32) -> windows_sys::core::BOOL {
    TERMINATE.store(true, Relaxed);
    let wake = WAKE.load(Relaxed);
    if wake != 0 {
        unsafe { SetEvent(wake as HANDLE) };
    }
    TRUE
}

/// What happened since the last call. Windows has no job control, so nothing
/// ever asks to suspend.
pub fn take_signals() -> Signals {
    Signals {
        resized: RESIZED.swap(false, Relaxed),
        terminate: TERMINATE.swap(false, Relaxed),
        ..Signals::default()
    }
}

pub fn install_signals() {
    unsafe {
        let event = CreateEventW(std::ptr::null(), FALSE, FALSE, std::ptr::null());
        if !event.is_null() {
            WAKE.store(event as usize, Relaxed);
        }
        SetConsoleCtrlHandler(Some(on_ctrl), TRUE);
    }
}

pub fn enter() -> io::Result<()> {
    let (input, output) = (handle(STD_INPUT_HANDLE), handle(STD_OUTPUT_HANDLE));
    let (mut in_mode, mut out_mode) = (0, 0);
    unsafe {
        if GetConsoleMode(input, &mut in_mode) == 0 || GetConsoleMode(output, &mut out_mode) == 0 {
            return Err(io::Error::last_os_error());
        }
        let (_, out_mode) = *SAVED.get_or_init(|| (in_mode, out_mode));
        let vt_out = ENABLE_PROCESSED_OUTPUT
            | ENABLE_VIRTUAL_TERMINAL_PROCESSING
            | DISABLE_NEWLINE_AUTO_RETURN;
        if SetConsoleMode(output, out_mode | vt_out) == 0 {
            return Err(io::Error::other(
                "this console can't show escape sequences (Windows 10 1607 or later needed)",
            ));
        }
        // Raw input: no line editing, echo, Ctrl-C handling or QuickEdit selection.
        let raw = ENABLE_WINDOW_INPUT | ENABLE_MOUSE_INPUT | ENABLE_EXTENDED_FLAGS;
        if SetConsoleMode(input, raw | ENABLE_VIRTUAL_TERMINAL_INPUT) == 0 {
            SetConsoleMode(input, raw);
        }
    }
    write_all(ENTER)
}

/// Undoes enter() and MOUSE_ON; safe to call more than once.
pub fn leave() {
    let _ = write_all(LEAVE);
    if let Some(&(input, output)) = SAVED.get() {
        unsafe {
            SetConsoleMode(handle(STD_INPUT_HANDLE), input);
            SetConsoleMode(handle(STD_OUTPUT_HANDLE), output);
        }
    }
}

/// Windows consoles can't suspend a program; Ctrl-Z only redraws.
pub fn suspend() -> io::Result<()> {
    Ok(())
}

fn buffer_info(which: STD_HANDLE) -> Option<CONSOLE_SCREEN_BUFFER_INFO> {
    unsafe {
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        (GetConsoleScreenBufferInfo(handle(which), &mut info) != 0).then_some(info)
    }
}

/// The visible window of whichever of stdout and stderr is a console.
pub(super) fn window_size() -> Option<(usize, usize)> {
    [STD_OUTPUT_HANDLE, STD_ERROR_HANDLE]
        .into_iter()
        .find_map(buffer_info)
        .map(|info| {
            let w = info.srWindow;
            (
                (w.Right - w.Left + 1) as usize,
                (w.Bottom - w.Top + 1) as usize,
            )
        })
        .filter(|&(cols, rows)| cols > 0 && rows > 0)
}

pub(super) fn pixel_aspect() -> Option<f64> {
    None
}

/// Waits for input that reads as bytes, forever when `timeout` is None. False on a
/// timeout, a resize or a control event; the caller then checks take_signals().
pub fn wait_input(timeout: Option<Duration>) -> bool {
    let deadline = timeout.map(|t| Instant::now() + t);
    let wake = WAKE.load(Relaxed) as HANDLE;
    let handles = [handle(STD_INPUT_HANDLE), wake];
    let count = if wake.is_null() { 1 } else { 2 };
    loop {
        if QUEUE.with_borrow(|q| !q.is_empty()) {
            return true;
        }
        let ms = deadline.map_or(INFINITE, |d| {
            let left = d.saturating_duration_since(Instant::now());
            left.as_micros().div_ceil(1000).min(INFINITE as u128 - 1) as u32
        });
        if unsafe { WaitForMultipleObjects(count, handles.as_ptr(), FALSE, ms) } != WAIT_OBJECT_0 {
            return false;
        }
        drain_records();
        if RESIZED.load(Relaxed) && QUEUE.with_borrow(|q| q.is_empty()) {
            return false;
        }
    }
}

/// Never Ok(0): a console has no end of input. WouldBlock when nothing is waiting.
pub fn read_input(buf: &mut [u8]) -> io::Result<usize> {
    if QUEUE.with_borrow(|q| q.is_empty()) {
        drain_records();
    }
    QUEUE.with_borrow_mut(|q| {
        let n = q.len().min(buf.len());
        if n == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        buf[..n].copy_from_slice(&q[..n]);
        q.drain(..n);
        Ok(n)
    })
}

/// Reads every input record waiting, without blocking, into QUEUE.
fn drain_records() {
    let input = handle(STD_INPUT_HANDLE);
    let mut records = [INPUT_RECORD::default(); 64];
    loop {
        let (mut waiting, mut read) = (0, 0);
        let ok = unsafe {
            GetNumberOfConsoleInputEvents(input, &mut waiting) != 0
                && waiting > 0
                && ReadConsoleInputW(input, records.as_mut_ptr(), records.len() as u32, &mut read)
                    != 0
        };
        if !ok || read == 0 {
            return;
        }
        QUEUE.with_borrow_mut(|q| {
            let start = q.len();
            for record in &records[..read as usize] {
                translate(record, q);
            }
            if q[start.saturating_sub(2)..]
                .windows(3)
                .any(|w| w == b"\x1b[<")
            {
                VT_MOUSE.store(true, Relaxed);
            }
        });
    }
}

fn translate(record: &INPUT_RECORD, out: &mut Vec<u8>) {
    match record.EventType as u32 {
        KEY_EVENT => {
            let key = unsafe { record.Event.KeyEvent };
            let unit = unsafe { key.uChar.UnicodeChar };
            if key.bKeyDown != 0 && unit != 0 {
                for _ in 0..key.wRepeatCount.max(1) {
                    push_utf16(unit, out);
                }
            }
        }
        MOUSE_EVENT if !VT_MOUSE.load(Relaxed) => mouse(unsafe { &record.Event.MouseEvent }, out),
        WINDOW_BUFFER_SIZE_EVENT => RESIZED.store(true, Relaxed),
        FOCUS_EVENT if unsafe { record.Event.FocusEvent.bSetFocus } == 0 => {
            out.extend_from_slice(b"\x1b[O")
        }
        _ => {}
    }
}

fn push_utf16(unit: u16, out: &mut Vec<u8>) {
    let high = HIGH.replace(0);
    let units: &[u16] = match unit {
        0xD800..=0xDBFF => {
            HIGH.set(unit);
            return;
        }
        0xDC00..=0xDFFF if high != 0 => &[high, unit],
        _ => &[unit],
    };
    for c in char::decode_utf16(units.iter().copied()).flatten() {
        out.extend_from_slice(c.encode_utf8(&mut [0; 4]).as_bytes());
    }
}

/// The console's own mouse record, as the SGR report a terminal would send.
fn mouse(m: &MOUSE_EVENT_RECORD, out: &mut Vec<u8>) {
    if m.dwEventFlags & (MOUSE_WHEELED | MOUSE_HWHEELED) != 0 {
        return;
    }
    let (left, top) = buffer_info(STD_OUTPUT_HANDLE)
        .map_or((0, 0), |info| (info.srWindow.Left, info.srWindow.Top));
    let col = (m.dwMousePosition.X - left).max(0) as u32 + 1;
    let row = (m.dwMousePosition.Y - top).max(0) as u32 + 1;
    let mut report = |button: u32, fin: char| {
        out.extend_from_slice(format!("\x1b[<{button};{col};{row}{fin}").as_bytes())
    };
    let held = m.dwButtonState & (FROM_LEFT_1ST_BUTTON_PRESSED | RIGHTMOST_BUTTON_PRESSED);
    let before = BUTTONS.swap(held, Relaxed);
    for (bit, button) in [
        (FROM_LEFT_1ST_BUTTON_PRESSED, 0),
        (RIGHTMOST_BUTTON_PRESSED, 2),
    ] {
        match (before & bit != 0, held & bit != 0) {
            (false, true) => report(button, 'M'),
            (true, false) => report(button, 'm'),
            _ => {}
        }
    }
    if m.dwEventFlags & MOUSE_MOVED != 0 {
        let drag = if held & FROM_LEFT_1ST_BUTTON_PRESSED != 0 {
            0
        } else if held != 0 {
            2
        } else {
            3
        };
        report(32 + drag, 'M');
    }
}

pub fn write_all(buf: &[u8]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(buf)?;
    out.flush()
}
