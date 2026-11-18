use std::io;
use std::time::{Duration, Instant};

use crate::cli::Mode;
use crate::clock::Overlay;
use crate::grid::Grid;
use crate::input::{self, Input, MOUSE_BUTTONS, MOUSE_MOTION, MOUSE_WHEEL};
use crate::math::{self, clamp, js_round};
use crate::renderer::Renderer;
use crate::screen::{self, ColorMode, Screen};
use crate::term;
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

/// Upstream's fixed timestep: how many steps `dt` more seconds take. At most
/// MAX_STEPS, so a stall doesn't snowball, unless slow frames or a fast --speed
/// need more every frame.
fn steps_for(acc: &mut f64, dt: f64) -> u32 {
    *acc += dt;
    let max = MAX_STEPS.max((dt / STEP).ceil() as u32);
    let mut steps = 0;
    while *acc >= STEP && steps < max {
        *acc -= STEP;
        steps += 1;
    }
    if steps == max {
        *acc = 0.0;
    }
    steps
}

pub struct Scene {
    pub world: World,
    pub renderer: Renderer,
    pub overlay: Overlay,
    pub aspect: f64,
}

impl Scene {
    /// Upstream's width / 124 rule, clamped to 8–13 px, and the cell's height / width.
    fn cell_size(&self, cols: usize) -> (f64, f64) {
        let cw = (8..=13)
            .rev()
            .map(f64::from)
            .find(|&w| clamp(js_round(cols as f64 * w / 124.0), 8.0, 13.0) == w)
            .unwrap_or(8.0);
        (cw, (cw + 1.0).max(js_round(cw * self.aspect)))
    }

    fn resize(&mut self, cols: usize, rows: usize, (cw, ch): (f64, f64)) {
        self.renderer.resize(cols, rows, cw, ch);
        self.world
            .resize(cols as f64 * cw, rows as f64 * ch, cw, ch);
    }

    /// Upstream's reduced-motion still: four seconds of sea first.
    fn settle(&mut self) {
        if self.world.time < 4.0 {
            for _ in 0..240 {
                self.world.step(STEP);
            }
        }
    }

    fn render(&mut self, dt: f64, grid: &mut Grid) {
        self.renderer
            .render_with(&mut self.world, dt, grid, |g| self.overlay.draw(g));
    }

    /// A settled sea without the overlay, for laying the clock over again and again.
    fn render_still(&mut self, grid: &mut Grid) {
        self.settle();
        self.renderer
            .render_with(&mut self.world, STEP, grid, |_| {});
    }
}

pub fn print(mut scene: Scene, mode: ColorMode, ascii: bool) -> io::Result<()> {
    let (cols, rows) = term::size();
    let cell = scene.cell_size(cols);
    let mut grid = Grid::new();
    grid.resize(cols, rows);
    scene.resize(cols, rows, cell);
    scene.render_still(&mut grid);
    scene.overlay.draw(&mut grid);
    term::write_all(&screen::print(&grid, mode, ascii))
}

pub struct Settings {
    pub fps: f64,
    pub speed: f64,
    pub rain: bool,
    pub screensaver: bool,
    pub mode: Mode,
}

impl Settings {
    fn mouse(&self) -> bool {
        self.mode == Mode::Live && !self.screensaver
    }
}

pub struct App {
    scene: Scene,
    settings: Settings,
    grid: Grid,
    still: Grid,
    screen: Screen,
    cell: Option<(f64, f64)>,
    paused: bool,
    frame: Duration,
    last: Instant,
    next: Instant,
    acc: f64,
    pending: Vec<u8>,
    inputs: Vec<Input>,
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
            still: Grid::new(),
            screen,
            cell: None,
            paused: false,
            last: now,
            next: now,
            acc: 0.0,
            pending: Vec::new(),
            inputs: Vec::new(),
            last_mouse: now,
        }
    }

    fn layout(&mut self) -> io::Result<()> {
        let (cols, rows) = term::size();
        let cell = *self.cell.get_or_insert_with(|| self.scene.cell_size(cols));
        self.scene.resize(cols, rows, cell);
        self.grid.resize(cols, rows);
        self.screen.reset(cols * rows);
        if self.settings.mode == Mode::Still {
            self.still.resize(cols, rows);
            self.scene.render_still(&mut self.still);
            self.draw_still()
        } else if self.paused {
            self.draw(0.0)
        } else {
            Ok(())
        }
    }

    fn draw(&mut self, dt: f64) -> io::Result<()> {
        self.scene.render(dt, &mut self.grid);
        self.screen.flush(&self.grid)
    }

    fn draw_still(&mut self) -> io::Result<()> {
        self.grid.clone_from(&self.still);
        self.scene.overlay.draw(&mut self.grid);
        self.screen.flush(&self.grid)
    }

    fn tick(&mut self, now: Instant) -> io::Result<()> {
        let elapsed = (now - self.last)
            .as_secs_f64()
            .min(self.frame.as_secs_f64().max(0.1));
        let dt = elapsed * self.settings.speed;
        self.last = now;
        for _ in 0..steps_for(&mut self.acc, dt) {
            self.scene.world.step(STEP);
        }
        if self.settings.rain && math::random() < RAIN_PER_SECOND * dt {
            let (x, y) = self.random_point();
            self.scene.world.drop(x, y);
        }
        self.scene.world.pointer.active &= now - self.last_mouse < POINTER_IDLE;
        self.draw(dt)
    }

    pub fn run(&mut self) -> io::Result<()> {
        if self.settings.mouse() {
            term::write_all(term::MOUSE_ON)?;
        }
        self.layout()?;
        loop {
            let signals = term::take_signals();
            if signals.terminate {
                return Ok(());
            }
            if signals.suspend {
                term::suspend()?;
                self.resume()?;
            } else if signals.continued {
                term::enter()?;
                self.resume()?;
            } else if signals.resized {
                self.layout()?;
            }

            let timeout = if self.settings.mode == Mode::Still {
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
            } else if self.settings.mode == Mode::Still && timeout.is_some() {
                self.draw_still()?;
            }
        }
    }

    fn resume(&mut self) -> io::Result<()> {
        if self.settings.mouse() {
            term::write_all(term::MOUSE_ON)?;
        }
        self.layout()
    }

    /// Reads what's waiting on stdin and acts on it; true means quit.
    fn read_input(&mut self) -> io::Result<bool> {
        let mut buf = [0u8; 4096];
        self.inputs.clear();
        loop {
            match term::read_input(&mut buf) {
                Ok(0) => return Ok(true),
                Ok(n) => self.pending.extend_from_slice(&buf[..n]),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => break,
                Err(e) => return Err(e),
            }
            let used = input::parse(&self.pending, &mut self.inputs);
            self.pending.drain(..used);
            if self.pending == [0x1b] && !term::wait_input(Some(ESC_WAIT)) {
                self.pending.clear();
                self.inputs.push(Input::Esc);
            }
            if self.pending.len() > 64 {
                self.pending.clear();
            }
            if self.pending.is_empty() || !term::wait_input(Some(Duration::ZERO)) {
                break;
            }
        }
        for i in 0..self.inputs.len() {
            if self.apply(i)? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn apply(&mut self, i: usize) -> io::Result<bool> {
        let input = &self.inputs[i];
        if self.settings.screensaver {
            return Ok(matches!(input, Input::Key(_) | Input::Esc | Input::Special));
        }
        match *input {
            Input::Key(b'q' | b'Q' | CTRL_C) | Input::Esc => return Ok(true),
            Input::Key(CTRL_Z) => {
                term::suspend()?;
                self.resume()?;
            }
            Input::Key(b'p' | b'P') => self.toggle_pause(),
            Input::Key(b' ') => self.act(World::click),
            Input::Key(b'f' | b'F') => self.act(World::feed),
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
        if !self.paused && self.settings.mode == Mode::Live {
            action(&mut self.scene.world, x, y);
        }
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
                0 => self.act_at(World::click, x, y),
                2 => self.act_at(World::feed, x, y),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sea_keeps_time_at_any_frame_rate_and_speed() {
        for (fps, speed) in [
            (30.0, 1.0),
            (10.0, 1.0),
            (5.0, 1.0),
            (30.0, 3.0),
            (30.0, 5.0),
            (120.0, 0.5),
        ] {
            let (mut acc, mut steps) = (0.0, 0);
            for _ in 0..(fps as usize * 10) {
                steps += steps_for(&mut acc, speed / fps);
            }
            let rate = steps as f64 * STEP / 10.0;
            assert!(
                (rate - speed).abs() < 0.05 * speed,
                "fps {fps} speed {speed}: {rate:.2}×"
            );
        }
    }

    #[test]
    fn still_redraws_change_nothing_but_the_clock() {
        use crate::clock::ClockStyle;
        use crate::world::{SharkKind, SharkSpec};
        math::seed(4);
        let mut scene = Scene {
            world: World::new(
                vec![SharkSpec {
                    kind: SharkKind::Tiger,
                    size: 1.0,
                }],
                None,
            ),
            renderer: Renderer::new(),
            overlay: Overlay::new(Some(ClockStyle::Lcd), false, None),
            aspect: 2.1,
        };
        let (mut still, mut a, mut b) = (Grid::new(), Grid::new(), Grid::new());
        still.resize(120, 40);
        scene.resize(120, 40, scene.cell_size(120));
        scene.render_still(&mut still);
        for g in [&mut a, &mut b] {
            g.clone_from(&still);
            scene.overlay.draw(g);
        }
        assert!(a.glyph == b.glyph && a.fg == b.fg && a.bg == b.bg);
    }

    #[test]
    fn a_stall_does_not_snowball() {
        let mut acc = 5.0;
        assert_eq!(
            steps_for(&mut acc, 0.1),
            6,
            "a tenth of a second is six steps"
        );
        assert_eq!(acc, 0.0);
    }
}
