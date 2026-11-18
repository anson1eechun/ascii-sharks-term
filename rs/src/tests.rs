use std::fmt::Write;

use super::*;
use crate::renderer::DEFAULT_BG;

/// Upstream demo.ts sizing: the first shark is biggest, the rest step down.
fn demo_sizes(kinds: &[SharkKind]) -> Vec<SharkSpec> {
    kinds
        .iter()
        .enumerate()
        .map(|(i, &kind)| SharkSpec {
            kind,
            size: (1.0 - i as f64 * 0.1).max(0.3),
        })
        .collect()
}

/// Renders a seeded scene and writes it as text, for diffing against the
/// TypeScript original driven by the same seed:
/// `SHARKS_DUMP=out.txt [SHARKS_DUMP_STEPS=240] cargo test dump_frame`
#[test]
fn dump_frame() {
    let Ok(path) = std::env::var("SHARKS_DUMP") else {
        return;
    };
    let steps: usize = std::env::var("SHARKS_DUMP_STEPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(240);
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
        trace += &format!(
            "{i} {sx:.10} {sy:.10} {:.6} {:.6}\n",
            s0.nodes[0], s0.nodes[1]
        );
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
        let fg = if g == ' ' {
            "-".to_string()
        } else {
            format!("{f:06x}")
        };
        let bg = if b == DEFAULT_BG {
            "def".to_string()
        } else {
            format!("{b:06x}")
        };
        out += &format!("{i} {g} {fg} {bg}\n");
    }
    std::fs::write(format!("{path}.trace"), trace).unwrap();
    std::fs::write(path, out).unwrap();
}

fn scene() -> World {
    math::seed(7);
    let mut world = World::new(vec![SharkSpec {
        kind: SharkKind::Classic,
        size: 1.0,
    }]);
    world.resize(150.0 * 13.0, 42.0 * 27.0, 13.0, 27.0);
    for _ in 0..60 {
        world.step(STEP);
    }
    world
}

fn snout_distance(world: &World, x: f64, y: f64) -> f64 {
    (world.sharks[0].nodes[0] as f64 - x).hypot(world.sharks[0].nodes[1] as f64 - y)
}

#[test]
fn sharks_flee_a_click() {
    let (mut calm, mut scared) = (scene(), scene());
    let sh = &scared.sharks[0];
    let ahead = sh.length * 0.4;
    let (x, y) = (
        sh.nodes[0] as f64 + sh.heading.cos() * ahead,
        sh.nodes[1] as f64 + sh.heading.sin() * ahead,
    );
    scared.disturb(x, y);
    for _ in 0..90 {
        calm.step(STEP);
        scared.step(STEP);
    }
    let (d_calm, d_scared) = (snout_distance(&calm, x, y), snout_distance(&scared, x, y));
    eprintln!(
        "snout distance to click after 1.5 s: calm {d_calm:.0} px, scared {d_scared:.0} px (was {ahead:.0})"
    );
    assert!(d_scared > d_calm + 50.0 && d_scared > ahead);
}

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

    let ring: Vec<usize> = (0..cols * rows)
        .filter(|&i| "()~/\\".contains(grid.glyph[i]))
        .collect();
    for g in ['(', ')', '~', '/', '\\'] {
        assert!(
            ring.iter().any(|&i| grid.glyph[i] == g),
            "no {g:?} in the ring"
        );
    }
    let radii: Vec<f64> = (0..3)
        .map(|k| cw + (0.6 - k as f64 * 0.22) * 240.0)
        .collect();
    for &i in &ring {
        let d = (((i % cols) as f64 + 0.5) * cw - x).hypot(((i / cols) as f64 + 0.5) * ch - y);
        // A ring cell may sit up to half a cell's diagonal off the exact radius.
        assert!(
            radii.iter().any(|r| (d - r).abs() <= cw.hypot(ch) / 2.0),
            "stray ring cell at distance {d:.0}"
        );
    }
    let picture: String = (5..25)
        .map(|r| {
            grid.glyph[r * cols + 15..r * cols + 75]
                .iter()
                .collect::<String>()
                + "\n"
        })
        .collect();
    eprintln!("{picture}");
}

/// Renders a real frame to SVG for the README:
/// `SHARKS_SVG=../docs/screenshot.svg cargo test --release screenshot`
#[test]
fn screenshot_svg() {
    let Ok(path) = std::env::var("SHARKS_SVG") else {
        return;
    };
    let (cols, rows, cw, ch) = (96, 28, 13.0, 27.0);
    let env = |k: &str, d: u32| {
        std::env::var(k)
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(d)
    };
    let (seed, steps) = (
        env("SHARKS_SVG_SEED", 42),
        env("SHARKS_SVG_STEPS", 600) as usize,
    );
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
            let _ = write!(
                svg,
                r#"<rect x="{x}" y="{y}" width="{w}" height="{h}" fill="{}"/>"#,
                hex(grid.bg[i])
            );
        }
    }
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
