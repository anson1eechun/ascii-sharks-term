//! The animations in docs/: seeded scenes rendered frame by frame, as SVG that
//! paints only what changed. `SHARKS_DEMO=../docs cargo test --release demo` writes
//! them again.

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::app::STEP;
use crate::cli::parse_kinds;
use crate::clock::{ClockStyle, Overlay};
use crate::grid::{DEFAULT_BG, Grid, WIDE_TAIL};
use crate::math;
use crate::renderer::Renderer;
use crate::tests::demo_sizes;
use crate::world::World;

const COLS: usize = 96;
const ROWS: usize = 28;
const CELL: (f64, f64) = (13.0, 27.0);
const FPS: usize = 15;
const WARM_UP: usize = 150;
/// An SVG cell, in px.
const W: usize = 9;
const H: usize = 19;

fn shoot(
    seed: u32,
    kinds: &str,
    frames: usize,
    mut cue: impl FnMut(usize, &mut World),
    mut overlay: impl FnMut(usize, &mut Grid),
) -> Vec<Grid> {
    math::seed(seed);
    let mut world = World::new(demo_sizes(&parse_kinds(kinds).unwrap()), None);
    let mut renderer = Renderer::new();
    let mut grid = Grid::new();
    let (cw, ch) = CELL;
    renderer.resize(COLS, ROWS, cw, ch);
    world.resize(COLS as f64 * cw, ROWS as f64 * ch, cw, ch);
    grid.resize(COLS, ROWS);
    let mut film = Vec::with_capacity(frames);
    for k in 0..WARM_UP + frames {
        let k = k.checked_sub(WARM_UP);
        if let Some(k) = k {
            cue(k, &mut world);
        }
        for _ in 0..60 / FPS {
            world.step(STEP);
        }
        renderer.render_with(&mut world, 1.0 / FPS as f64, &mut grid, |g| {
            if let Some(k) = k {
                overlay(k, g);
            }
        });
        if k.is_some() {
            film.push(grid.clone());
        }
    }
    film
}

/// A point `ahead` px in front of shark `i`'s snout.
fn ahead_of(world: &World, i: usize, ahead: f64) -> (f64, f64) {
    let s = &world.sharks[i];
    (
        s.nodes[0] as f64 + s.heading.cos() * ahead,
        s.nodes[1] as f64 + s.heading.sin() * ahead,
    )
}

fn hunt() -> Vec<Grid> {
    shoot(
        42,
        "greathammer,tiger,hammerhead,zebra",
        6 * FPS,
        |k, world| match k {
            12 => {
                let (x, y) = ahead_of(world, 1, 160.0);
                world.click(x, y);
            }
            48 => {
                let (x, y) = ahead_of(world, 0, 260.0);
                world.feed(x, y);
            }
            _ => {}
        },
        |_, _| {},
    )
}

fn clock() -> Vec<Grid> {
    let mut overlay = Overlay::new(Some(ClockStyle::Block), true, None);
    shoot(
        7,
        "tiger,hammerhead",
        4 * FPS,
        |_, _| {},
        |k, g| {
            let s = 57 + (k / FPS) as u32;
            overlay.draw_at(g, || (12, 34 + s / 60, s % 60));
        },
    )
}

fn hex(c: u32) -> String {
    let c = if c == DEFAULT_BG { 0 } else { c };
    let nibbles = [20, 16, 12, 8, 4, 0].map(|shift| (c >> shift) & 15);
    if nibbles.chunks(2).all(|p| p[0] == p[1]) {
        format!("#{:x}{:x}{:x}", nibbles[0], nibbles[2], nibbles[4])
    } else {
        format!("#{c:06x}")
    }
}

/// Frame k shows from k / n of the loop to its end, over the frames before it.
fn svg(frames: &[Grid]) -> String {
    let (cols, rows, n) = (frames[0].cols, frames[0].rows, frames.len());
    let mut s = String::new();
    let _ = write!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}" font-family="ui-monospace,SFMono-Regular,Menlo,Consolas,monospace" font-size="15" font-weight="600" text-anchor="middle" stroke-width="{H}" shape-rendering="crispEdges"><style>g{{animation:{}s steps(1,end) infinite}}"#,
        cols * W,
        rows * H,
        n as f64 / FPS as f64
    );
    for k in 1..n {
        let _ = write!(
            s,
            ".f{k}{{animation-name:a{k}}}@keyframes a{k}{{0%{{opacity:0}}{}%{{opacity:1}}}}",
            (k * 100_000 / n) as f64 / 1000.0
        );
    }
    s += r##"</style><rect width="100%" height="100%" fill="#000"/>"##;
    let mut blank = Grid::new();
    blank.resize(cols, rows);
    let mut prev = &blank;
    for (k, g) in frames.iter().enumerate() {
        let changed = |i: usize| {
            g.glyph[i] != prev.glyph[i]
                || g.bg[i] != prev.bg[i]
                || (g.glyph[i] != ' ' && g.fg[i] != prev.fg[i])
        };
        let mut fills: BTreeMap<u32, (String, (usize, usize))> = BTreeMap::new();
        let mut inks: BTreeMap<u32, [String; 3]> = BTreeMap::new();
        for r in 0..rows {
            let mut c = 0;
            while c < cols {
                let i = r * cols + c;
                if !changed(i) {
                    c += 1;
                    continue;
                }
                let run = (c..cols)
                    .take_while(|&c| changed(r * cols + c) && g.bg[r * cols + c] == g.bg[i])
                    .count();
                if k > 0 || g.bg[i] != DEFAULT_BG {
                    let (x, y) = (c * W, r * H);
                    let (d, end) = fills.entry(g.bg[i]).or_default();
                    if d.is_empty() {
                        let _ = write!(d, "M{x} {}.5", y + H / 2);
                    } else {
                        let _ = write!(d, "m{} {}", x as isize - end.0 as isize, y - end.1);
                    }
                    let _ = write!(d, "h{}", run * W);
                    *end = (x + run * W, y);
                }
                for c in c..c + run {
                    let j = r * cols + c;
                    if matches!(g.glyph[j], ' ' | WIDE_TAIL) {
                        continue;
                    }
                    let [xs, ys, text] = inks.entry(g.fg[j]).or_default();
                    let _ = write!(xs, "{} ", c * W + W / 2);
                    let _ = write!(ys, "{} ", r * H + H * 18 / 25);
                    match g.glyph[j] {
                        '&' => *text += "&amp;",
                        '<' => *text += "&lt;",
                        ch => text.push(ch),
                    }
                }
                c += run;
            }
        }
        if k == 0 {
            s += "<g>";
        } else {
            let _ = write!(s, r#"<g class="f{k}">"#);
        }
        for (bg, (d, _)) in fills {
            let _ = write!(s, r#"<path stroke="{}" d="{d}"/>"#, hex(bg));
        }
        for (fg, [xs, ys, text]) in inks {
            let _ = write!(
                s,
                r#"<text x="{}" y="{}" fill="{}">{text}</text>"#,
                xs.trim_end(),
                ys.trim_end(),
                hex(fg)
            );
        }
        s += "</g>";
        prev = g;
    }
    s + "</svg>\n"
}

#[test]
fn write_demos() {
    let Ok(dir) = std::env::var("SHARKS_DEMO") else {
        return;
    };
    std::fs::write(format!("{dir}/demo.svg"), svg(&hunt())).unwrap();
    std::fs::write(format!("{dir}/clock.svg"), svg(&clock())).unwrap();
}

#[test]
fn frames_repaint_only_what_changed() {
    let mut a = Grid::new();
    a.resize(4, 1);
    a.put(0, 0, "ab", 0xffffff, None);
    let mut b = a.clone();
    b.put(0, 1, "c", 0xffffff, None);
    let out = svg(&[a.clone(), a.clone(), b]);
    assert!(
        out.contains(r#"<g class="f1"></g>"#),
        "an unchanged frame is empty"
    );
    assert!(out.contains(
        r##"<g class="f2"><path stroke="#000" d="M9 9.5h9"/><text x="13" y="13" fill="#fff">c</text></g>"##
    ));
}
