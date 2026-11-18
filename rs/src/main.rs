//! stackedlol/ascii-sharks in the terminal; this file stands in for upstream's engine.ts.

mod input;
mod math;
mod renderer;
mod species;
mod term;
#[cfg(test)]
mod tests;
mod world;

use std::io;
use std::process::exit;
use std::sync::OnceLock;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use input::{Input, MOUSE_BUTTONS, MOUSE_MOTION, MOUSE_WHEEL};
use math::{clamp, js_round};
use renderer::{Grid, Renderer};
use world::{SharkKind, SharkSpec, World};

const STEP: f64 = 1.0 / 60.0;
const MAX_STEPS: u32 = 4;
const FALLBACK_ASPECT: f64 = 2.1;
const MIN_SIZE: f64 = 0.55;
/// How long a lone Esc waits to rule out being the start of a sequence.
const ESC_WAIT: Duration = Duration::from_millis(25);
const CTRL_C: u8 = 0x03;

const HELP_EN: &str = "sharks — ASCII sharks and fish schools in your terminal

Usage: sharks [options]

  -k, --sharks <kind,...>  Which sharks to show (default: different kinds at random)
  -n, --count <n>          How many random sharks, 1–12 (default 4; ignored with -k)
  -l, --list               List the shark kinds
  -f, --fps <n>            Frames per second (default 30; lower uses less CPU)
  -S, --screensaver        Screensaver mode: any key quits
      --aspect <n>         Cell height / width (default: ask the terminal, else 2.1)
  -V, --version            Show the version
  -h, --help               Show this help

Every shark gets its own random size, so no two runs look alike.

While running: q / Esc quit, p pause, move the mouse to scare the fish,
               click to make a ripple: the fish scatter and nearby sharks turn away.

Messages follow the locale; set SHARKS_LANG=zh-TW for Chinese.";

const HELP_ZH: &str = "sharks — 終端機裡的 ASCII 鯊魚與魚群

用法: sharks [選項]

  -k, --sharks <種類,...>  指定要放哪些鯊魚（預設：隨機挑選不同種類）
  -n, --count <n>          隨機鯊魚的數量，1～12（預設 4；有 -k 時不使用）
  -l, --list               列出所有鯊魚種類（英文代號與中文名）
  -f, --fps <n>            每秒畫面數（預設 30，越低越省資源）
  -S, --screensaver        螢幕保護模式：按任意鍵離開
      --aspect <n>         字元格高寬比（預設自動偵測，失敗時用 2.1）
  -V, --version            顯示版本
  -h, --help               顯示這段說明

每隻鯊魚的大小都是隨機的，每次執行都不一樣。

執行中：q / Esc 離開　p 暫停　滑鼠移動驚動魚群
        點擊泛起漣漪：魚群四散，附近的鯊魚轉向逃開

訊息語言依語系而定；設定 SHARKS_LANG=en 可改用英文。";

fn zh() -> bool {
    static ZH: OnceLock<bool> = OnceLock::new();
    *ZH.get_or_init(|| {
        ["SHARKS_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("zh"))
    })
}

fn t(en: &'static str, zh_text: &'static str) -> &'static str {
    if zh() { zh_text } else { en }
}

fn fail(msg: &str) -> ! {
    eprintln!("sharks: {msg}");
    exit(2);
}

struct Options {
    kinds: Option<Vec<SharkKind>>,
    count: usize,
    fps: f64,
    screensaver: bool,
    aspect: Option<f64>,
}

fn parse_args() -> Options {
    let mut opts = Options {
        kinds: None,
        count: 4,
        fps: 30.0,
        screensaver: false,
        aspect: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, mut inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_owned(), Some(v.to_owned())),
            _ => (arg, None),
        };
        let mut value = || {
            inline
                .take()
                .or_else(|| args.next())
                .unwrap_or_else(|| fail(&format!("{flag} {}", t("needs a value", "需要一個值"))))
        };
        match flag.as_str() {
            "-h" | "--help" => {
                println!("{}", t(HELP_EN, HELP_ZH));
                exit(0);
            }
            "-l" | "--list" => {
                for kind in SharkKind::ALL {
                    println!("{:<12} {}", kind.name(), kind.label(zh()));
                }
                exit(0);
            }
            "-V" | "--version" => {
                println!("sharks {}", env!("CARGO_PKG_VERSION"));
                exit(0);
            }
            "-S" | "--screensaver" => opts.screensaver = true,
            "-k" | "--sharks" => opts.kinds = Some(parse_kinds(&value())),
            "-n" | "--count" => {
                opts.count = match value().parse() {
                    Ok(n @ 1..=12) => n,
                    _ => fail(t(
                        "--count must be between 1 and 12",
                        "--count 要介於 1 到 12",
                    )),
                };
            }
            "-f" | "--fps" => {
                opts.fps = value().parse().unwrap_or(f64::NAN);
                if !(1.0..=120.0).contains(&opts.fps) {
                    fail(t(
                        "--fps must be between 1 and 120",
                        "--fps 要介於 1 到 120",
                    ));
                }
            }
            "--aspect" => {
                let aspect = value().parse().unwrap_or(f64::NAN);
                if !term::ASPECT.contains(&aspect) {
                    fail(t(
                        "--aspect must be between 1 and 4",
                        "--aspect 要介於 1 到 4",
                    ));
                }
                opts.aspect = Some(aspect);
            }
            _ if zh() => fail(&format!("不認得的選項：{flag}（用 -h 看說明）")),
            _ => fail(&format!("unknown option: {flag} (see -h)")),
        }
    }
    opts
}

fn parse_kinds(list: &str) -> Vec<SharkKind> {
    let (kinds, unknown): (Vec<_>, Vec<_>) = list
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|n| SharkKind::parse(n).ok_or(n))
        .partition(Result::is_ok);
    if !unknown.is_empty() {
        let unknown: Vec<&str> = unknown.into_iter().filter_map(Result::err).collect();
        let unknown = unknown.join(", ");
        fail(&if zh() {
            format!("不認得的鯊魚種類：{unknown}（用 -l 看清單）")
        } else {
            format!("unknown shark kind: {unknown} (see -l)")
        });
    }
    kinds.into_iter().flatten().collect()
}

fn random_kinds(n: usize) -> Vec<SharkKind> {
    let mut kinds = SharkKind::ALL.to_vec();
    for i in (1..kinds.len()).rev() {
        kinds.swap(i, (math::random() * (i + 1) as f64) as usize);
    }
    (0..n)
        .map(|i| {
            kinds
                .get(i)
                .copied()
                .unwrap_or_else(|| kinds[(math::random() * kinds.len() as f64) as usize])
        })
        .collect()
}

/// Upstream also reads size as depth.
fn random_sizes(kinds: &[SharkKind]) -> Vec<SharkSpec> {
    let biggest = (math::random() * kinds.len() as f64) as usize;
    kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| SharkSpec {
            kind,
            size: if i == biggest {
                1.0
            } else {
                math::rand(MIN_SIZE, 1.0)
            },
        })
        .collect()
}

/// The cell width upstream's renderer would pick (canvas width / 124, clamped to 8–13 px).
fn cell_width(cols: usize) -> f64 {
    (8..=13)
        .rev()
        .map(f64::from)
        .find(|&w| clamp(js_round(cols as f64 * w / 124.0), 8.0, 13.0) == w)
        .unwrap_or(8.0)
}

struct App {
    world: World,
    renderer: Renderer,
    grid: Grid,
    screen: term::Screen,
    aspect: f64,
    cw: f64,
    ch: f64,
    paused: bool,
    screensaver: bool,
    frame: Duration,
    last: Instant,
    next: Instant,
    acc: f64,
    pending: Vec<u8>,
}

impl App {
    fn new(opts: &Options, aspect: f64) -> Self {
        let kinds = opts
            .kinds
            .clone()
            .unwrap_or_else(|| random_kinds(opts.count));
        let now = Instant::now();
        App {
            world: World::new(random_sizes(&kinds)),
            renderer: Renderer::new(),
            grid: Grid::new(),
            screen: term::Screen::new(),
            aspect,
            cw: 0.0,
            ch: 0.0,
            paused: false,
            screensaver: opts.screensaver,
            frame: Duration::from_secs_f64(1.0 / opts.fps),
            last: now,
            next: now,
            acc: 0.0,
            pending: Vec::new(),
        }
    }

    fn layout(&mut self) -> io::Result<()> {
        let (cols, rows) = term::size();
        self.cw = cell_width(cols);
        self.ch = (self.cw + 1.0).max(js_round(self.cw * self.aspect));
        self.renderer.resize(cols, rows, self.cw, self.ch);
        self.world.resize(
            cols as f64 * self.cw,
            rows as f64 * self.ch,
            self.cw,
            self.ch,
        );
        self.grid.resize(cols, rows);
        self.screen.reset(cols * rows);
        if self.paused {
            self.draw(0.0)?;
        }
        Ok(())
    }

    fn draw(&mut self, dt: f64) -> io::Result<()> {
        self.renderer.render(&mut self.world, dt, &mut self.grid);
        self.screen.flush(&self.grid)
    }

    fn tick(&mut self, now: Instant) -> io::Result<()> {
        let dt = (now - self.last).as_secs_f64().min(0.1);
        self.last = now;
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= STEP && steps < MAX_STEPS {
            self.world.step(STEP);
            self.acc -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS {
            self.acc = 0.0;
        }
        self.draw(dt)
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.last = Instant::now();
            self.next = self.last;
        }
    }

    fn read_input(&mut self) -> io::Result<bool> {
        let mut buf = [0u8; 4096];
        let mut inputs = Vec::new();
        loop {
            match term::read_input(&mut buf) {
                Ok(0) => return Ok(true),
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => return Ok(false),
                Err(e) => return Err(e),
            }
            let used = input::parse(&self.pending, &mut inputs);
            self.pending.drain(..used);
            if self.pending == [0x1b] && !term::wait_input(Some(ESC_WAIT)) {
                self.pending.clear();
                inputs.push(Input::Esc);
            }
            if self.pending.len() > 64 {
                self.pending.clear();
            }
            if self.pending.is_empty() || !term::wait_input(Some(Duration::ZERO)) {
                break;
            }
        }
        Ok(inputs.iter().any(|i| self.apply(i)))
    }

    fn apply(&mut self, input: &Input) -> bool {
        if self.screensaver {
            return matches!(input, Input::Key(_) | Input::Esc);
        }
        match *input {
            Input::Key(b'q' | b'Q' | CTRL_C) | Input::Esc => return true,
            Input::Key(b'p' | b'P') => self.toggle_pause(),
            Input::Mouse {
                button,
                col,
                row,
                press,
            } => self.mouse(button, col, row, press),
            Input::FocusLost => self.world.pointer.active = false,
            _ => {}
        }
        false
    }

    fn mouse(&mut self, button: u32, col: u32, row: u32, press: bool) {
        let (x, y) = ((col as f64 - 0.5) * self.cw, (row as f64 - 0.5) * self.ch);
        let p = &mut self.world.pointer;
        (p.x, p.y, p.active) = (x, y, true);
        let left_press = press && button & (MOUSE_MOTION | MOUSE_WHEEL | MOUSE_BUTTONS) == 0;
        if left_press && !self.paused {
            self.world.splash(x, y);
            self.world.disturb(x, y);
        }
    }

    fn run(&mut self) -> io::Result<()> {
        self.layout()?;
        loop {
            if term::TERMINATE.load(Relaxed) {
                return Ok(());
            }
            if term::RESIZED.swap(false, Relaxed) {
                self.layout()?;
            }
            let now = Instant::now();
            if !self.paused && now >= self.next {
                self.tick(now)?;
                self.next += self.frame;
                if self.next < now {
                    self.next = now + self.frame;
                }
            }
            let timeout =
                (!self.paused).then(|| self.next.saturating_duration_since(Instant::now()));
            if term::wait_input(timeout) && self.read_input()? {
                return Ok(());
            }
        }
    }
}

fn main() {
    let opts = parse_args();
    if !term::is_tty() {
        fail(t("needs to run in a terminal", "需要在終端機裡執行"));
    }
    term::install_signals();
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        term::leave();
        default_hook(info);
    }));
    if let Err(e) = term::enter() {
        fail(&format!(
            "{}: {e}",
            t("can't set up the terminal", "無法設定終端機")
        ));
    }

    let aspect = opts
        .aspect
        .or_else(term::cell_aspect)
        .unwrap_or(FALLBACK_ASPECT);
    if !opts.screensaver {
        let _ = term::write_all(term::MOUSE_ON);
    }
    let result = App::new(&opts, aspect).run();
    term::leave();
    if let Err(e) = result {
        eprintln!("sharks: {e}");
        exit(1);
    }
}
