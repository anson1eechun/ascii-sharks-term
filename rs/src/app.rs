use std::io;
use std::sync::atomic::Ordering::Relaxed;
use std::time::{Duration, Instant};

use crate::clock::Overlay;
use crate::grid::Grid;
use crate::input::{self, Input, MOUSE_BUTTONS, MOUSE_MOTION, MOUSE_WHEEL};
use crate::math::{self, clamp, js_round};
use crate::renderer::Renderer;
use crate::term::{self, ColorMode, Screen};
use crate::world::World;

pub const STEP: f64 = 1.0 / 60.0;
const MAX_STEPS: u32 = 4;
/// How long a lone Esc waits to rule out being the start of a sequence.
const ESC_WAIT: Duration = Duration::from_millis(25);
/// Terminals don't report the pointer leaving, so a quiet pointer counts as gone.
const POINTER_IDLE: Duration = Duration::from_secs(5);
const RAIN_PER_SECOND: f64 = 1.2;
const CTRL_C: u8 = 0x03;
const CTRL_Z: u8 = 0x1a;

pub struct Scene {
    pub world: World,
    pub renderer: Renderer,
    pub overlay: Overlay,
    pub aspect: f64,
}

impl Scene {
    /// Fixes the cell size for the session: upstream's width / 124 rule, clamped to
    /// 8–13 px, and the terminal's height / width.
    fn cell_size(&self, cols: usize) -> (f64, f64) {
        let cw = (8..=13)
            .rev()
            .map(f64::from)
            .find(|&w| clamp(js_round(cols as f64 * w / 124.0), 8.0, 13.0) == w)
            .unwrap_or(8.0);
        (cw, (cw + 1.0).max(js_round(cw * self.aspect)))
    }

    /// Upstream's reduced-motion still: let the sea settle for four seconds first.
    fn settle(&mut self) {
        while self.world.time < 4.0 {
            self.world.step(STEP);
        }
    }

    fn render(&mut self, dt: f64, grid: &mut Grid) {
        let overlay = &mut self.overlay;
        self.renderer.render_with(&mut self.world, dt, grid, |g| {
            if !overlay.is_empty() {
                overlay.draw(g);
            }
        });
    }
}

/// Prints one settled frame at the terminal's (or $COLUMNS × $LINES) size.
pub fn print(mut scene: Scene, mode: ColorMode, ascii: bool) -> io::Result<()> {
    let (cols, rows) = term::size();
    let (cw, ch) = scene.cell_size(cols);
    let mut grid = Grid::new();
    grid.resize(cols, rows);
    scene.renderer.resize(cols, rows, cw, ch);
    scene
        .world
        .resize(cols as f64 * cw, rows as f64 * ch, cw, ch);
    scene.settle();
    scene.render(STEP, &mut grid);
    term::write_all(&term::print(&grid, mode, ascii))
}

pub struct Settings {
    pub fps: f64,
    pub speed: f64,
    pub rain: bool,
    pub screensaver: bool,
    pub still: bool,
    pub mouse: bool,
}

pub struct App {
    scene: Scene,
    settings: Settings,
    grid: Grid,
    screen: Screen,
    cell: Option<(f64, f64)>,
    paused: bool,
    frame: Duration,
    last: Instant,
    next: Instant,
    acc: f64,
    pending: Vec<u8>,
    last_mouse: Instant,
}

impl App {
    pub fn new(scene: Scene, settings: Settings, screen: Screen) -> Self {
        let now = Instant::now();
        App {
            scene,
            frame: Duration::from_secs_f64(1.0 / settings.fps),
            settings,
            grid: Grid::new(),
            screen,
            cell: None,
            paused: false,
            last: now,
            next: now,
            acc: 0.0,
            pending: Vec::new(),
            last_mouse: now,
        }
    }

    fn layout(&mut self) -> io::Result<()> {
        let (cols, rows) = term::size();
        let (cw, ch) = *self.cell.get_or_insert_with(|| self.scene.cell_size(cols));
        self.scene.renderer.resize(cols, rows, cw, ch);
        self.scene
            .world
            .resize(cols as f64 * cw, rows as f64 * ch, cw, ch);
        self.grid.resize(cols, rows);
        self.screen.reset(cols * rows);
        if self.settings.still {
            self.scene.settle();
        }
        if self.paused || self.settings.still {
            self.draw(0.0)?;
        }
        Ok(())
    }

    fn draw(&mut self, dt: f64) -> io::Result<()> {
        self.scene.render(dt, &mut self.grid);
        self.screen.flush(&self.grid)
    }

    fn tick(&mut self, now: Instant) -> io::Result<()> {
        let dt = (now - self.last).as_secs_f64().min(0.1) * self.settings.speed;
        self.last = now;
        self.acc += dt;
        let mut steps = 0;
        while self.acc >= STEP && steps < MAX_STEPS {
            self.scene.world.step(STEP);
            self.acc -= STEP;
            steps += 1;
        }
        if steps == MAX_STEPS {
            self.acc = 0.0;
        }
        if self.settings.rain && math::random() < RAIN_PER_SECOND * dt {
            let (x, y) = self.random_point();
            self.scene.world.drop(x, y);
        }
        let pointer = &mut self.scene.world.pointer;
        pointer.active &= now - self.last_mouse < POINTER_IDLE;
        self.draw(dt)
    }

    pub fn run(&mut self) -> io::Result<()> {
        if self.settings.mouse {
            term::write_all(term::MOUSE_ON)?;
        }
        self.layout()?;
        loop {
            if term::TERMINATE.load(Relaxed) {
                return Ok(());
            }
            if term::SUSPEND.swap(false, Relaxed) {
                term::suspend()?;
                self.resume()?;
            }
            if term::CONTINUED.swap(false, Relaxed) {
                term::enter()?;
                self.resume()?;
            }
            if term::RESIZED.swap(false, Relaxed) {
                self.layout()?;
            }

            let timeout = if self.settings.still {
                self.scene.overlay.until_change()
            } else if self.paused {
                None
            } else {
                let now = Instant::now();
                if now >= self.next {
                    self.tick(now)?;
                    self.next += self.frame;
                    if self.next < now {
                        self.next = now + self.frame;
                    }
                }
                Some(self.next.saturating_duration_since(Instant::now()))
            };
            if term::wait_input(timeout) {
                if self.read_input()? {
                    return Ok(());
                }
            } else if self.settings.still && timeout.is_some() {
                self.draw(0.0)?;
            }
        }
    }

    fn resume(&mut self) -> io::Result<()> {
        if self.settings.mouse {
            term::write_all(term::MOUSE_ON)?;
        }
        self.layout()
    }

    /// Reads what's waiting on stdin and acts on it; true means quit.
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
        for input in &inputs {
            if self.apply(input)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn apply(&mut self, input: &Input) -> io::Result<bool> {
        if self.settings.screensaver {
            return Ok(matches!(input, Input::Key(_) | Input::Esc));
        }
        match *input {
            Input::Key(b'q' | b'Q' | CTRL_C) | Input::Esc => return Ok(true),
            Input::Key(CTRL_Z) => {
                term::suspend()?;
                self.resume()?;
            }
            Input::Key(b'p' | b'P') => self.toggle_pause(),
            Input::Key(b' ') => self.act(App::click),
            Input::Key(b'f' | b'F') => self.act(App::feed),
            Input::Mouse {
                button,
                col,
                row,
                press,
            } => self.mouse(button, col, row, press),
            Input::FocusLost => self.scene.world.pointer.active = false,
            _ => {}
        }
        Ok(false)
    }

    fn toggle_pause(&mut self) {
        self.paused = !self.paused;
        if !self.paused {
            self.last = Instant::now();
            self.next = self.last;
        }
    }

    fn random_point(&self) -> (f64, f64) {
        let w = &self.scene.world;
        (math::random() * w.width, math::random() * w.height)
    }

    /// At the pointer when it's in the window, else somewhere at random.
    fn act(&mut self, action: fn(&mut World, f64, f64)) {
        let p = &self.scene.world.pointer;
        let (x, y) = if p.active {
            (p.x, p.y)
        } else {
            self.random_point()
        };
        self.act_at(action, x, y);
    }

    fn act_at(&mut self, action: fn(&mut World, f64, f64), x: f64, y: f64) {
        if !self.paused && !self.settings.still {
            action(&mut self.scene.world, x, y);
        }
    }

    fn click(world: &mut World, x: f64, y: f64) {
        world.splash(x, y);
        world.disturb(x, y);
    }

    fn feed(world: &mut World, x: f64, y: f64) {
        world.feed(x, y);
    }

    fn mouse(&mut self, button: u32, col: u32, row: u32, press: bool) {
        let Some((cw, ch)) = self.cell else { return };
        let (cols, rows) = (self.grid.cols as u32, self.grid.rows as u32);
        let (x, y) = ((col as f64 - 0.5) * cw, (row as f64 - 0.5) * ch);
        let p = &mut self.scene.world.pointer;
        (p.x, p.y, p.active) = (x, y, col > 1 && row > 1 && col < cols && row < rows);
        self.last_mouse = Instant::now();
        if press && button & (MOUSE_MOTION | MOUSE_WHEEL) == 0 {
            match button & MOUSE_BUTTONS {
                0 => self.act_at(App::click, x, y),
                2 => self.act_at(App::feed, x, y),
                _ => {}
            }
        }
    }
}
