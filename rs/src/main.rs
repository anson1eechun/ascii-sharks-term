//! stackedlol/ascii-sharks in the terminal; app.rs stands in for upstream's engine.ts.

mod app;
mod cli;
mod clock;
mod grid;
mod input;
mod math;
mod renderer;
mod species;
mod term;
#[cfg(test)]
mod tests;
mod world;

use std::process::exit;

use app::{App, Scene, Settings};
use cli::{Mode, Options, fail, t};
use clock::{ClockStyle, Overlay};
use renderer::{Labels, Renderer};
use term::{ColorMode, Screen};
use world::{SharkKind, SharkSpec, World};

const FALLBACK_ASPECT: f64 = 2.1;
const MIN_SIZE: f64 = 0.55;

/// All different while there are enough kinds to go round.
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

fn scene(opts: &Options, color: ColorMode, aspect: f64, wide_ambiguous: bool) -> Scene {
    let kinds = opts
        .kinds
        .clone()
        .unwrap_or_else(|| random_kinds(opts.count));
    let mut renderer = Renderer::new();
    renderer.calm = opts.calm;
    renderer.labels = if cli::zh() && !opts.ascii {
        Labels::Chinese
    } else {
        Labels::English
    };
    Scene {
        world: World::new(random_sizes(&kinds), opts.fish),
        renderer,
        overlay: Overlay::new(
            opts.clock
                .map(|c| c.fallback(opts.ascii, wide_ambiguous, color == ColorMode::Mono)),
            opts.seconds,
            opts.message.clone(),
        ),
        aspect,
    }
}

fn main() {
    let opts = cli::parse();
    let color = opts.color.unwrap_or_else(ColorMode::detect);
    if opts.list_styles {
        for style in ClockStyle::ALL {
            println!("{}", style.name());
            let sample = clock::sample(style.fallback(opts.ascii, false, color == ColorMode::Mono));
            let _ = term::write_all(&term::print(&sample, color, opts.ascii));
            println!();
        }
        return;
    }
    if let Some(seed) = opts.seed {
        math::seed(seed);
    }
    if opts.mode == Mode::Print {
        let scene = scene(&opts, color, opts.aspect.unwrap_or(FALLBACK_ASPECT), false);
        if let Err(e) = app::print(scene, color, opts.ascii) {
            fail(&e.to_string());
        }
        return;
    }
    if !term::is_tty() {
        fail(t(
            "needs to run in a terminal (or use --print)",
            "需要在終端機裡執行（或改用 --print）",
        ));
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
    let probe = term::probe();
    let aspect = opts.aspect.or(probe.aspect).unwrap_or(FALLBACK_ASPECT);
    let settings = Settings {
        fps: opts.fps,
        speed: opts.speed,
        rain: opts.rain,
        screensaver: opts.screensaver,
        still: opts.mode == Mode::Still,
        mouse: opts.mode == Mode::Live && !opts.screensaver,
    };
    let screen = Screen::new(color, opts.ascii || probe.wide_ambiguous);
    let result = App::new(
        scene(&opts, color, aspect, probe.wide_ambiguous),
        settings,
        screen,
    )
    .run();
    term::leave();
    if let Err(e) = result {
        eprintln!("sharks: {e}");
        exit(1);
    }
}
