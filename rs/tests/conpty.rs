//! The binary driven through a Windows pseudo-console, as Windows Terminal runs it.
//! ConPTY re-encodes the screen rather than passing the program's bytes through,
//! so these tests watch the output grow instead of reading frames.
#![cfg(windows)]

use std::fs::File;
use std::io::{Read, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{CloseHandle, FALSE, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Console::{COORD, ClosePseudoConsole, CreatePseudoConsole, HPCON};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess,
    InitializeProcThreadAttributeList, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};

const BIN: &str = env!("CARGO_BIN_EXE_sharks");
const PATIENCE: Duration = Duration::from_secs(15);

struct Console {
    hpc: HPCON,
    process: HANDLE,
    input: File,
    out: Arc<Mutex<Vec<u8>>>,
}

fn pipe() -> (HANDLE, HANDLE) {
    let (mut read, mut write) = (std::ptr::null_mut(), std::ptr::null_mut());
    assert_ne!(
        unsafe { CreatePipe(&mut read, &mut write, std::ptr::null(), 0) },
        0
    );
    (read, write)
}

impl Console {
    fn open(args: &[&str]) -> Console {
        let (in_read, in_write) = pipe();
        let (out_read, out_write) = pipe();
        let mut hpc: HPCON = 0;
        let size = COORD { X: 100, Y: 30 };
        assert_eq!(
            unsafe { CreatePseudoConsole(size, in_read, out_write, 0, &mut hpc) },
            0
        );
        unsafe {
            CloseHandle(in_read);
            CloseHandle(out_write);
        }

        let mut bytes = 0;
        unsafe { InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes) };
        let mut list = vec![0u8; bytes];
        let mut info: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
        info.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        info.lpAttributeList = list.as_mut_ptr().cast();
        unsafe {
            assert_ne!(
                InitializeProcThreadAttributeList(info.lpAttributeList, 1, 0, &mut bytes),
                0
            );
            assert_ne!(
                UpdateProcThreadAttribute(
                    info.lpAttributeList,
                    0,
                    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                    hpc as *const _,
                    size_of::<HPCON>(),
                    std::ptr::null_mut(),
                    std::ptr::null(),
                ),
                0
            );
        }

        let line = std::iter::once(format!("\"{BIN}\""))
            .chain(args.iter().map(|a| a.to_string()))
            .collect::<Vec<_>>()
            .join(" ");
        let mut wide: Vec<u16> = line.encode_utf16().chain([0]).collect();
        let mut process: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let created = unsafe {
            CreateProcessW(
                std::ptr::null(),
                wide.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                FALSE,
                EXTENDED_STARTUPINFO_PRESENT,
                std::ptr::null(),
                std::ptr::null(),
                &info.StartupInfo,
                &mut process,
            )
        };
        assert_ne!(
            created,
            0,
            "CreateProcessW: {}",
            std::io::Error::last_os_error()
        );
        unsafe { CloseHandle(process.hThread) };

        let out = Arc::new(Mutex::new(Vec::new()));
        let sink = out.clone();
        let mut reader = unsafe { File::from_raw_handle(out_read) };
        thread::spawn(move || {
            let mut buf = [0u8; 16384];
            while let Ok(n @ 1..) = reader.read(&mut buf) {
                sink.lock().unwrap().extend_from_slice(&buf[..n]);
            }
        });
        Console {
            hpc,
            process: process.hProcess,
            input: unsafe { File::from_raw_handle(in_write) },
            out,
        }
    }

    fn len(&self) -> usize {
        self.out.lock().unwrap().len()
    }

    /// Waits until at least `bytes` more have come out than at `from`.
    fn wait_for_output(&self, from: usize, bytes: usize) -> usize {
        let deadline = Instant::now() + PATIENCE;
        while self.len() < from + bytes {
            assert!(
                Instant::now() < deadline,
                "only {} bytes after {from}",
                self.len() - from
            );
            thread::sleep(Duration::from_millis(20));
        }
        self.len()
    }

    /// Waits until nothing has come out for a moment.
    fn settle(&self) -> usize {
        let mut last = self.len();
        loop {
            thread::sleep(Duration::from_millis(300));
            let now = self.len();
            if now == last {
                return now;
            }
            last = now;
        }
    }

    fn send(&mut self, bytes: &[u8]) {
        self.input.write_all(bytes).unwrap();
        self.input.flush().unwrap();
    }

    fn exit_code(&self) -> Option<u32> {
        let wait = PATIENCE.as_millis() as u32;
        if unsafe { WaitForSingleObject(self.process, wait) } != WAIT_OBJECT_0 {
            return None;
        }
        let mut code = 0;
        unsafe { GetExitCodeProcess(self.process, &mut code) };
        Some(code)
    }

    fn running_after(&self, d: Duration) -> bool {
        unsafe { WaitForSingleObject(self.process, d.as_millis() as u32) != WAIT_OBJECT_0 }
    }
}

impl Drop for Console {
    fn drop(&mut self) {
        unsafe {
            TerminateProcess(self.process, 1);
            CloseHandle(self.process);
            ClosePseudoConsole(self.hpc);
        }
    }
}

#[test]
fn draws_and_quits_on_q_esc_and_ctrl_c() {
    for key in [&b"q"[..], b"\x1b", b"\x03"] {
        let mut console = Console::open(&["--seed", "1"]);
        console.wait_for_output(0, 4000);
        console.send(key);
        assert_eq!(console.exit_code(), Some(0), "{key:?}");
    }
}

#[test]
fn clicks_and_feeding_keep_it_running() {
    let mut console = Console::open(&["--seed", "2"]);
    let at = console.wait_for_output(0, 4000);
    console.send(b"\x1b[<0;40;12M\x1b[<0;40;12m\x1b[<35;41;12M\x1b[<2;20;8M\x1b[<2;20;8mf p p");
    assert!(console.running_after(Duration::from_millis(800)));
    console.wait_for_output(at, 1000);
    console.send(b"q");
    assert_eq!(console.exit_code(), Some(0));
}

#[test]
fn the_still_clock_ticks() {
    let mut console = Console::open(&["--still", "--clock", "--seconds", "--seed", "3"]);
    console.wait_for_output(0, 3000);
    let mut at = console.settle();
    for _ in 0..2 {
        console.wait_for_output(at, 1);
        at = console.settle();
    }
    console.send(b"q");
    assert_eq!(console.exit_code(), Some(0));
}
