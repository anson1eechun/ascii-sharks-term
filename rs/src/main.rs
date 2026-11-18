//! sharks — stackedlol/ascii-sharks in the terminal.
//!
//! world.rs and renderer.rs port upstream's world.ts and renderer.ts; this file
//! replaces engine.ts (frame loop, resize, pointer input) and term.rs replaces
//! the canvas.

mod math;
mod renderer;
mod species;
mod term;
mod world;

use std::io;
use std::process::exit;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use math::{clamp, js_round};
use renderer::{Grid, Renderer};
use world::{SharkKind, SharkSpec, World};

const STEP: f64 = 1.0 / 60.0;
const MAX_STEPS: u32 = 4;
/// Height / width of a terminal cell when the terminal won't tell us.
const FALLBACK_ASPECT: f64 = 2.1;

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

/// Chinese messages when SHARKS_LANG or the POSIX locale asks for them.
fn zh() -> bool {
    static ZH: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ZH.get_or_init(|| {
        ["SHARKS_LANG", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
            .is_some_and(|v| v.to_ascii_lowercase().starts_with("zh"))
    })
}

/// Picks the English or Chinese text.
fn t(en: &'static str, zh_text: &'static str) -> &'static str {
    if zh() { zh_text } else { en }
}

/// The smallest shark, relative to the biggest one in the water.
const MIN_SIZE: f64 = 0.55;

struct Options {
    /// From -k; otherwise `count` kinds are picked at random.
    kinds: Option<Vec<SharkKind>>,
    count: usize,
    fps: f64,
    screensaver: bool,
    aspect: Option<f64>,
}

fn fail(msg: &str) -> ! {
    eprintln!("sharks: {msg}");
    exit(2);
}

fn parse_args() -> Options {
    let mut opts = Options { kinds: None, count: 4, fps: 30.0, screensaver: false, aspect: None };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
            _ => (arg.clone(), None),
        };
        let mut value = || {
            inline
                .clone()
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
                    _ => fail(t("--count must be between 1 and 12", "--count 要介於 1 到 12")),
                };
            }
            "-f" | "--fps" => {
                let fps: f64 = value().parse().unwrap_or(f64::NAN);
                if !(1.0..=120.0).contains(&fps) {
                    fail(t("--fps must be between 1 and 120", "--fps 要介於 1 到 120"));
                }
                opts.fps = fps;
            }
            "--aspect" => {
                let aspect: f64 = value().parse().unwrap_or(f64::NAN);
                if !(1.0..=4.0).contains(&aspect) {
                    fail(t("--aspect must be between 1 and 4", "--aspect 要介於 1 到 4"));
                }
                opts.aspect = Some(aspect);
            }
            _ if zh() => fail(&format!("不認得的選項：{arg}（用 -h 看說明）")),
            _ => fail(&format!("unknown option: {arg} (see -h)")),
        }
    }
    opts
}

fn parse_kinds(list: &str) -> Vec<SharkKind> {
    let names: Vec<&str> = list.split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    let unknown: Vec<&str> = names.iter().copied().filter(|n| SharkKind::parse(n).is_none()).collect();
    if !unknown.is_empty() {
        let unknown = unknown.join(", ");
        fail(&if zh() {
            format!("不認得的鯊魚種類：{unknown}（用 -l 看清單）")
        } else {
            format!("unknown shark kind: {unknown} (see -l)")
        });
    }
    names.iter().map(|n| SharkKind::parse(n).unwrap()).collect()
}

/// `n` kinds at random: all different while there are enough kinds to go round.
fn random_kinds(n: usize) -> Vec<SharkKind> {
    let mut kinds = SharkKind::ALL.to_vec();
    // Fisher–Yates.
    for i in (1..kinds.len()).rev() {
        kinds.swap(i, (math::random() * (i + 1) as f64) as usize);
    }
    (0..n)
        .map(|i| kinds.get(i).copied().unwrap_or_else(|| kinds[(math::random() * kinds.len() as f64) as usize]))
        .collect()
}

/// One shark at full size, the rest anywhere from MIN_SIZE up. Upstream reads size
/// as depth too, so the smaller ones also swim deeper and dimmer.
fn random_sizes(kinds: &[SharkKind]) -> Vec<SharkSpec> {
    let biggest = (math::random() * kinds.len() as f64) as usize;
    kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| SharkSpec { kind, size: if i == biggest { 1.0 } else { math::rand(MIN_SIZE, 1.0) } })
        .collect()
}

/// Upstream demo.ts sizing (the first is biggest, the rest step down), for
/// scenes that must match upstream.
#[cfg(test)]
fn demo_sizes(kinds: &[SharkKind]) -> Vec<SharkSpec> {
    kinds.iter().enumerate().map(|(i, &kind)| SharkSpec { kind, size: (1.0 - i as f64 * 0.1).max(0.3) }).collect()
}

/// Upstream sizes cells from the canvas width (width / 124, clamped to 8–13 px).
/// Take the width it would settle on for this many columns, so the world has the
/// same scale as the web version at a comparable size.
fn cell_width(cols: usize) -> f64 {
    for w in (8..=13).rev() {
        let w = w as f64;
        if clamp(js_round(cols as f64 * w / 124.0), 8.0, 13.0) == w {
            return w;
        }
    }
    8.0
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
}

impl App {
    fn layout(&mut self) -> io::Result<()> {
        let (cols, rows) = term::size();
        self.cw = cell_width(cols);
        self.ch = (self.cw + 1.0).max(js_round(self.cw * self.aspect));
        self.renderer.resize(cols, rows, self.cw, self.ch);
        self.world.resize(cols as f64 * self.cw, rows as f64 * self.ch, self.cw, self.ch);
        self.grid.resize(cols, rows);
        self.screen.reset(cols * rows);
        term::write_all(b"\x1b[0m\x1b[2J")
    }

    /// Returns true when the input asks to quit.
    fn handle_input(&mut self, bytes: &[u8]) -> bool {
        if self.screensaver {
            // Any key, but not the terminal's own replies (CSI …).
            return bytes == b"\x1b" || !bytes.starts_with(b"\x1b[");
        }
        if bytes == b"\x1b" {
            return true;
        }
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                0x1b => i = self.escape(bytes, i),
                b'q' | b'Q' | 0x03 => return true,
                b'p' | b'P' => {
                    self.paused = !self.paused;
                    i += 1;
                }
                _ => i += 1,
            }
        }
        false
    }

    /// Consumes one escape sequence starting at `start`; returns the index after it.
    fn escape(&mut self, bytes: &[u8], start: usize) -> usize {
        if bytes.get(start + 1) != Some(&b'[') {
            return start + 1;
        }
        let body_start = start + 2;
        let Some(len) = bytes[body_start..].iter().position(|b| (0x40..=0x7e).contains(b)) else {
            return bytes.len();
        };
        let end = body_start + len;
        let (body, fin) = (&bytes[body_start..end], bytes[end]);
        if fin == b'O' && body.is_empty() {
            // Focus lost.
            self.world.pointer.active = false;
        } else if let [b'<', params @ ..] = body
            && (fin == b'M' || fin == b'm')
        {
            self.mouse(params, fin == b'M');
        }
        end + 1
    }

    /// SGR mouse report `button;col;row`: moving scares fish, a left press splashes.
    fn mouse(&mut self, params: &[u8], press: bool) {
        let Ok(params) = std::str::from_utf8(params) else { return };
        let mut it = params.split(';').map(|p| p.parse::<u32>());
        let (Some(Ok(btn)), Some(Ok(col)), Some(Ok(row))) = (it.next(), it.next(), it.next()) else { return };
        let x = (col as f64 - 0.5) * self.cw;
        let y = (row as f64 - 0.5) * self.ch;
        let p = &mut self.world.pointer;
        p.x = x;
        p.y = y;
        p.active = true;
        // Bit 32 marks motion, 64 the wheel; low bits 0 is the left button.
        if press && btn & (32 | 64) == 0 && btn & 3 == 0 && !self.paused {
            // Upstream's splash scatters the fish; the ripple and the scared sharks are ours.
            self.world.splash(x, y);
            self.world.disturb(x, y);
            self.renderer.ripple(x, y, self.world.time);
        }
    }
}

/// Same fixed-timestep scheme as upstream's engine.ts, paced by poll() on stdin.
fn run(opts: &Options, aspect: f64) -> io::Result<()> {
    let mut app = App {
        world: World::new(random_sizes(&opts.kinds.clone().unwrap_or_else(|| random_kinds(opts.count)))),
        renderer: Renderer::new(),
        grid: Grid::new(),
        screen: term::Screen::new(),
        aspect,
        cw: 13.0,
        ch: 27.0,
        paused: false,
        screensaver: opts.screensaver,
    };
    app.layout()?;

    let frame = Duration::from_secs_f64(1.0 / opts.fps);
    let mut last = Instant::now();
    let mut next = last;
    let mut acc = 0.0;
    let mut input = [0u8; 4096];

    loop {
        if term::TERMINATE.load(Relaxed) {
            return Ok(());
        }
        if term::RESIZED.swap(false, Relaxed) {
            app.layout()?;
        }

        let now = Instant::now();
        if now >= next {
            let dt = (now - last).as_secs_f64().min(0.1);
            last = now;
            if !app.paused {
                acc += dt;
                let mut steps = 0;
                while acc >= STEP && steps < MAX_STEPS {
                    app.world.step(STEP);
                    acc -= STEP;
                    steps += 1;
                }
                if steps == MAX_STEPS {
                    acc = 0.0;
                }
                app.renderer.render(&mut app.world, dt, &mut app.grid);
                app.screen.flush(&app.grid)?;
            }
            next += frame;
            if next < now {
                next = now + frame;
            }
        }

        if term::wait_input(next.saturating_duration_since(Instant::now())) {
            match term::read_input(&mut input) {
                0 => return Ok(()), // terminal went away
                n if n > 0 && app.handle_input(&input[..n as usize]) => return Ok(()),
                _ => {}
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
        fail(&format!("{}: {e}", t("can't set up the terminal", "無法設定終端機")));
    }

    let aspect = opts.aspect.or_else(term::cell_aspect).unwrap_or(FALLBACK_ASPECT);
    if !opts.screensaver {
        let _ = term::write_all(term::MOUSE_ON);
    }
    let result = run(&opts, aspect);
    term::leave();
    if let Err(e) = result {
        eprintln!("sharks: {e}");
        exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use renderer::DEFAULT_BG;

    /// Renders a seeded scene and writes it as text, for diffing against the
    /// TypeScript original driven by the same seed:
    /// `SHARKS_DUMP=out.txt [SHARKS_DUMP_STEPS=240] cargo test dump_frame`
    #[test]
    fn dump_frame() {
        let Ok(path) = std::env::var("SHARKS_DUMP") else { return };
        let steps: usize = std::env::var("SHARKS_DUMP_STEPS").ok().and_then(|s| s.parse().ok()).unwrap_or(240);
        let (cols, rows, cw, ch) = (150, 42, 13.0, 27.0);
        math::seed(42);
        let kinds = ["hammerhead", "tiger", "zebra", "greathammer", "thresher"];
        let mut world = World::new(demo_sizes(&parse_kinds(&kinds.join(","))));
        let mut renderer = Renderer::new();
        let mut grid = Grid::new();
        renderer.resize(cols, rows, cw, ch);
        world.resize(cols as f64 * cw, rows as f64 * ch, cw, ch);
        grid.resize(cols, rows);
        // Forces the eat / heat / respawn paths early, before the two runs drift apart
        // (1-ulp libm differences, amplified by the chaotic sim, after ~2 s).
        let force = std::env::var("SHARKS_DUMP_FORCE").is_ok_and(|v| !v.is_empty());
        let mut trace = String::new();
        for i in 0..steps {
            if i == 100 {
                world.splash(900.0, 500.0);
            }
            if force && i == 50 {
                for (k, sh) in world.sharks.iter_mut().enumerate() {
                    sh.hunger = 1.0;
                    world.fish[k].x = sh.nodes[0] as f64;
                    world.fish[k].y = sh.nodes[1] as f64;
                }
            }
            if force && i == 60 {
                for f in &mut world.fish[10..16] {
                    f.alive = false;
                    f.respawn_at = world.time;
                }
            }
            world.step(STEP);
            let sx: f64 = world.fish.iter().map(|f| f.x).sum();
            let sy: f64 = world.fish.iter().map(|f| f.y).sum();
            let s0 = &world.sharks[0];
            trace += &format!("{i} {sx:.10} {sy:.10} {:.6} {:.6}\n", s0.nodes[0], s0.nodes[1]);
            if i % 2 == 1 {
                renderer.render(&mut world, 1.0 / 30.0, &mut grid);
            }
        }
        let mut out = String::new();
        let alive = world.fish.iter().filter(|f| f.alive).count();
        let sum: f64 = world.fish.iter().map(|f| f.x + f.y).sum();
        out += &format!("world alive={alive} sum={sum:.6}\n");
        for sh in &world.sharks {
            out += &format!("shark {} {:.4} {:.4}\n", sh.id, sh.nodes[0], sh.nodes[1]);
        }
        for i in 0..cols * rows {
            let (g, f, b) = (grid.glyph[i], grid.fg[i], grid.bg[i]);
            if g == ' ' && b == DEFAULT_BG {
                continue;
            }
            let fg = if g == ' ' { "-".to_string() } else { format!("{f:06x}") };
            let bg = if b == DEFAULT_BG { "def".to_string() } else { format!("{b:06x}") };
            out += &format!("{i} {g} {fg} {bg}\n");
        }
        std::fs::write(format!("{path}.trace"), trace).unwrap();
        std::fs::write(path, out).unwrap();
    }
}

#[cfg(test)]
mod click_tests {
    use super::*;
    use world::SharkKind;

    fn scene() -> World {
        math::seed(7);
        let mut world = World::new(vec![SharkSpec { kind: SharkKind::Classic, size: 1.0 }]);
        world.resize(150.0 * 13.0, 42.0 * 27.0, 13.0, 27.0);
        for _ in 0..60 {
            world.step(STEP);
        }
        world
    }

    fn snout_distance(world: &World, x: f64, y: f64) -> f64 {
        (world.sharks[0].nodes[0] as f64 - x).hypot(world.sharks[0].nodes[1] as f64 - y)
    }

    /// A click just ahead of a shark's snout turns it away; without the click it swims on into the spot.
    #[test]
    fn sharks_flee_a_click() {
        let (mut calm, mut scared) = (scene(), scene());
        let sh = &scared.sharks[0];
        let ahead = sh.length * 0.4;
        let (x, y) = (sh.nodes[0] as f64 + sh.heading.cos() * ahead, sh.nodes[1] as f64 + sh.heading.sin() * ahead);
        scared.disturb(x, y);
        for _ in 0..90 {
            calm.step(STEP);
            scared.step(STEP);
        }
        let (d_calm, d_scared) = (snout_distance(&calm, x, y), snout_distance(&scared, x, y));
        eprintln!("snout distance to click after 1.5 s: calm {d_calm:.0} px, scared {d_scared:.0} px (was {ahead:.0})");
        assert!(d_scared > d_calm + 50.0 && d_scared > ahead);
    }

    /// The ripple is one ring of direction-following glyphs at the expected radius.
    #[test]
    fn ripple_draws_a_ring() {
        let (cols, rows, cw, ch) = (90, 30, 13.0, 27.0);
        let mut world = World::new(Vec::new());
        world.resize(cols as f64 * cw, rows as f64 * ch, cw, ch);
        world.fish.clear();
        let mut renderer = Renderer::new();
        let mut grid = Grid::new();
        renderer.resize(cols, rows, cw, ch);
        grid.resize(cols, rows);
        let (x, y) = (45.0 * cw, 15.0 * ch);
        renderer.ripple(x, y, 0.0);
        world.time = 0.6;
        renderer.render(&mut world, 1.0 / 30.0, &mut grid);

        let ring: Vec<usize> = (0..cols * rows).filter(|&i| "()~/\\".contains(grid.glyph[i])).collect();
        for g in ['(', ')', '~', '/', '\\'] {
            assert!(ring.iter().any(|&i| grid.glyph[i] == g), "no {g:?} in the ring");
        }
        // Every ring cell sits on one of the three rings (ages 0.6, 0.38, 0.16 s).
        let radii: Vec<f64> = (0..3).map(|k| cw + (0.6 - k as f64 * 0.22) * 240.0).collect();
        for &i in &ring {
            let d = (((i % cols) as f64 + 0.5) * cw - x).hypot(((i / cols) as f64 + 0.5) * ch - y);
            // A ring cell may sit up to half a cell's diagonal off the exact radius.
            assert!(radii.iter().any(|r| (d - r).abs() <= cw.hypot(ch) / 2.0), "stray ring cell at distance {d:.0}");
        }
        let picture: String = (5..25)
            .map(|r| grid.glyph[r * cols + 15..r * cols + 75].iter().collect::<String>() + "\n")
            .collect();
        eprintln!("{picture}");
    }
}

#[cfg(test)]
mod screenshot {
    use super::*;
    use renderer::DEFAULT_BG;
    use std::fmt::Write;

    /// Renders a real frame to SVG for the README:
    /// `SHARKS_SVG=../docs/screenshot.svg cargo test --release screenshot`
    #[test]
    fn screenshot_svg() {
        let Ok(path) = std::env::var("SHARKS_SVG") else { return };
        let (cols, rows, cw, ch) = (96, 28, 13.0, 27.0);
        let env = |k: &str, d: u32| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
        let (seed, steps) = (env("SHARKS_SVG_SEED", 42), env("SHARKS_SVG_STEPS", 600) as usize);
        math::seed(seed);
        let kinds = "greathammer,tiger,hammerhead,zebra";
        let mut world = World::new(demo_sizes(&parse_kinds(kinds)));
        let mut renderer = Renderer::new();
        let mut grid = Grid::new();
        renderer.resize(cols, rows, cw, ch);
        world.resize(cols as f64 * cw, rows as f64 * ch, cw, ch);
        grid.resize(cols, rows);
        for i in 0..steps {
            if i + 40 == steps {
                renderer.ripple(24.0 * cw, 9.0 * ch, world.time);
            }
            world.step(STEP);
            if i % 2 == 1 {
                renderer.render(&mut world, 1.0 / 30.0, &mut grid);
            }
        }

        // Cell size in the picture; same height/width as a typical terminal cell.
        let (w, h) = (9.0, 19.0);
        let hex = |c: u32| format!("#{c:06x}");
        let mut svg = String::new();
        let _ = write!(
            svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" font-size="15" font-weight="600">"#,
            cols as f64 * w,
            rows as f64 * h
        );
        svg += r##"<rect width="100%" height="100%" fill="#000"/>"##;
        for i in 0..cols * rows {
            if grid.bg[i] != DEFAULT_BG {
                let (x, y) = ((i % cols) as f64 * w, (i / cols) as f64 * h);
                let _ = write!(svg, r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{}"/>"#, hex(grid.bg[i]));
            }
        }
        // One <text> per run of same-coloured glyphs in a row, each glyph placed on its cell.
        for r in 0..rows {
            let mut c = 0;
            while c < cols {
                let i = r * cols + c;
                if grid.glyph[i] == ' ' {
                    c += 1;
                    continue;
                }
                let fg = grid.fg[i];
                let (mut xs, mut text) = (String::new(), String::new());
                while c < cols && grid.glyph[r * cols + c] != ' ' && grid.fg[r * cols + c] == fg {
                    let _ = write!(xs, "{} ", c as f64 * w + w / 2.0);
                    match grid.glyph[r * cols + c] {
                        '&' => text += "&amp;",
                        '<' => text += "&lt;",
                        g => text.push(g),
                    }
                    c += 1;
                }
                let _ = write!(
                    svg,
                    r#"<text x="{}" y="{}" fill="{}" text-anchor="middle" xml:space="preserve">{text}</text>"#,
                    xs.trim_end(),
                    r as f64 * h + h * 0.72,
                    hex(fg)
                );
            }
        }
        svg += "</svg>\n";
        std::fs::write(path, svg).unwrap();
    }
}

#[cfg(test)]
mod lineup_tests {
    use super::*;

    #[test]
    fn random_kinds_are_distinct_until_they_run_out() {
        for n in 1..=8 {
            let mut kinds = random_kinds(n);
            assert_eq!(kinds.len(), n);
            kinds.sort_by_key(|k| k.name());
            kinds.dedup();
            assert_eq!(kinds.len(), n, "repeated a kind among {n}");
        }
        assert_eq!(random_kinds(12).len(), 12);
    }

    #[test]
    fn random_sizes_have_one_full_size_shark() {
        for _ in 0..200 {
            let specs = random_sizes(&random_kinds(5));
            assert_eq!(specs.iter().filter(|s| s.size == 1.0).count(), 1);
            assert!(specs.iter().all(|s| (MIN_SIZE..=1.0).contains(&s.size)));
        }
    }
}
