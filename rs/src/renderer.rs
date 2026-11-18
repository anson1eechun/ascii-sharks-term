//! Port of upstream src/renderer.ts; canvas calls become writes into a Grid of cells.
//! Left out: the grid lines, and the ambient dots and shadows upstream switches off.

use crate::math::{PI, clamp, hash3, js_round, smoothstep, to_i32};
use crate::world::{EventKind, SHARK_NODES, Shark, SharkKind, World};

pub const DEFAULT_BG: u32 = u32::MAX;

pub struct Grid {
    pub cols: usize,
    pub glyph: Vec<char>,
    pub fg: Vec<u32>,
    pub bg: Vec<u32>,
}

impl Grid {
    pub fn new() -> Self {
        Grid {
            cols: 0,
            glyph: Vec::new(),
            fg: Vec::new(),
            bg: Vec::new(),
        }
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.glyph = vec![' '; cols * rows];
        self.fg = vec![0; cols * rows];
        self.bg = vec![DEFAULT_BG; cols * rows];
    }

    fn clear(&mut self) {
        self.glyph.fill(' ');
        self.fg.fill(0);
        self.bg.fill(DEFAULT_BG);
    }

    fn text(&mut self, i: usize, glyph: char, rgb: u32, alpha: f64) {
        self.fg[i] = over(rgb, alpha, self.bg[i]);
        self.glyph[i] = glyph;
    }

    fn block(&mut self, i: usize, rgb: u32) {
        self.bg[i] = rgb;
        self.glyph[i] = ' ';
    }
}

/// Source-over onto an opaque colour; the default background counts as black.
fn over(rgb: u32, alpha: f64, under: u32) -> u32 {
    if alpha >= 1.0 {
        return rgb;
    }
    let u = if under == DEFAULT_BG { 0 } else { under };
    let mix = |shift: u32| {
        js_round(
            ((rgb >> shift) & 255) as f64 * alpha + ((u >> shift) & 255) as f64 * (1.0 - alpha),
        ) as u32
    };
    (mix(16) << 16) | (mix(8) << 8) | mix(0)
}

const fn rgb(r: u32, g: u32, b: u32) -> u32 {
    (r << 16) | (g << 8) | b
}

fn rgb_f(r: f64, g: f64, b: f64) -> u32 {
    let c = |v: f64| js_round(v).min(255.0) as u32;
    rgb(c(r), c(g), c(b))
}

/// Alpha as the canvas saw it after `.toFixed(n)`.
fn fixed(v: f64, decimals: i32) -> f64 {
    let p = 10f64.powi(decimals);
    js_round(v * p) / p
}

const ALPHA_LEVELS: usize = 6;
const FISH_ALPHAS: [f64; 5] = [0.28, 0.42, 0.58, 0.74, 0.9];
const FISH_RGB: u32 = rgb(230, 238, 255);
const WAKE_RGB: u32 = rgb(110, 230, 255);
const DOT_GLYPH: char = '·';

const ACCENTS: [u32; 7] = [
    0xe0892c, 0xe03a32, 0xb44ae0, 0x33c8cc, 0x3cb54a, 0x3150e8, 0xd9c14a,
];

const SKIN_RAMP: &[u8] = b":;t%SX8#@";
const FIN_GLYPHS: &[u8] = b"%;tS:";
const RIDGE_GLYPHS: &[u8] = b"@8%";
const EDGE_HEAVY: &[u8] = b":;";
const EDGE_LIGHT: &[u8] = b".',`";

const FLAG_BODY: u8 = 1;
const FLAG_EDGE: u8 = 2;
const FLAG_FIN: u8 = 3;
const FLAG_RIDGE: u8 = 4;
const FLAG_EYE: u8 = 5;
const FLAG_MARK: u8 = 6;
const MARK_GLYPHS: &[u8] = b"#%@";

const SUBSAMPLES: [(f64, f64); 4] = [(0.25, 0.3), (0.75, 0.3), (0.25, 0.72), (0.75, 0.72)];

// Not in upstream: spine segments are grouped under bounding circles so most
// of them can be skipped per sample. Purely a speed-up; results are identical.
const SEGMENTS: usize = SHARK_NODES - 1;
const CHUNK: usize = 4;
const CHUNKS: usize = SEGMENTS.div_ceil(CHUNK);
/// Slack on the circle bounds, far above f64 rounding at these magnitudes.
const BOUND_EPS: f64 = 1e-6;
/// No species' half_width exceeds this (tiger peaks at 0.114; see the tests),
/// so the outline only needs evaluating closer to the spine than this.
const MAX_HALF_WIDTH: f64 = 0.12;

// Not in upstream.
const RIPPLE_RINGS: usize = 3;
const RIPPLE_GAP: f64 = 0.22;
const RIPPLE_LIFE: f64 = 1.6;
const RIPPLE_SPEED: f64 = 240.0;
const RIPPLE_RGB: u32 = rgb(170, 235, 255);
const RIPPLE_SPAN: f64 = (RIPPLE_RINGS - 1) as f64 * RIPPLE_GAP + RIPPLE_LIFE;

struct Ripple {
    x: f64,
    y: f64,
    born: f64,
}

#[derive(Clone, Copy, Default)]
struct Segment {
    ax: f64,
    ay: f64,
    sx: f64,
    sy: f64,
    /// Squared length, 1 when degenerate (upstream's `|| 1`).
    len2: f64,
    arc: f64,
}

#[derive(Clone, Copy, Default)]
struct Bound {
    cx: f64,
    cy: f64,
    r: f64,
}

struct Head {
    span: f64,
    sweep: f64,
    depth: f64,
    notch: f64,
}

struct SharkShape {
    half_width: fn(f64) -> f64,
    pectoral: [f64; 4],
    falcate: bool,
    pelvic: [f64; 4],
    caudal: f64,
    dorsal: [f64; 2],
    second_dorsal: [f64; 2],
    head: Option<Head>,
    eye: Option<[f64; 2]>,
    gills: [f64; 2],
    tint: [f64; 3],
}

fn classic_half_width(s: f64) -> f64 {
    if s < 0.13 {
        return 0.09 * ((s / 0.13) * PI / 2.0).sin().powf(0.95);
    }
    if s < 0.3 {
        return 0.09 + 0.016 * smoothstep(0.13, 0.3, s);
    }
    if s < 0.84 {
        return 0.019 + 0.087 * (1.0 - (s - 0.3) / 0.54).powf(1.25);
    }
    let k = (s - 0.84) / 0.16;
    0.019 * (1.0 - k) + 0.012 * (k * PI).sin()
}

static CLASSIC: SharkShape = SharkShape {
    half_width: classic_half_width,
    pectoral: [0.15, 0.43, 0.55, 0.26],
    falcate: true,
    pelvic: [0.57, 0.65, 0.69, 0.085],
    caudal: 0.085,
    dorsal: [0.33, 0.47],
    second_dorsal: [0.71, 0.76],
    head: None,
    eye: Some([0.1, 0.14]),
    gills: [0.19, 0.29],
    tint: [0.92, 0.97, 1.05],
};

static HAMMERHEAD: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.06 {
            return 0.045;
        }
        if s < 0.3 {
            return 0.045 + 0.061 * smoothstep(0.06, 0.3, s);
        }
        classic_half_width(s) * 0.92
    },
    head: Some(Head {
        span: 0.17,
        sweep: 0.035,
        depth: 0.09,
        notch: 0.0,
    }),
    pectoral: [0.17, 0.4, 0.5, 0.22],
    falcate: true,
    pelvic: [0.56, 0.63, 0.67, 0.075],
    caudal: 0.11,
    dorsal: [0.28, 0.44],
    second_dorsal: [0.7, 0.74],
    eye: None,
    gills: [0.16, 0.26],
    tint: [0.96, 0.98, 0.9],
};

static TIGER: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.13 {
            return 0.1 * ((s / 0.13) * PI / 2.0).sin().powf(0.45);
        }
        if s < 0.3 {
            return 0.1 + 0.014 * smoothstep(0.13, 0.3, s);
        }
        if s < 0.84 {
            return 0.019 + 0.095 * (1.0 - (s - 0.3) / 0.54).powf(1.2);
        }
        let k = (s - 0.84) / 0.16;
        0.019 * (1.0 - k) + 0.012 * (k * PI).sin()
    },
    pectoral: [0.15, 0.42, 0.53, 0.25],
    falcate: true,
    pelvic: [0.57, 0.65, 0.69, 0.085],
    caudal: 0.095,
    dorsal: [0.33, 0.46],
    second_dorsal: [0.71, 0.75],
    head: None,
    eye: Some([0.08, 0.12]),
    gills: [0.19, 0.28],
    tint: [1.02, 0.95, 0.84],
};

static ZEBRA: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.1 {
            return 0.075 * ((s / 0.1) * PI / 2.0).sin().powf(0.7);
        }
        if s < 0.28 {
            return 0.075 + 0.025 * smoothstep(0.1, 0.28, s);
        }
        if s < 0.55 {
            return 0.022 + 0.078 * (1.0 - (s - 0.28) / 0.27).powf(1.1);
        }
        let k = (s - 0.55) / 0.45;
        0.022 * (1.0 - k * k) + 0.012 * (k * PI).sin()
    },
    pectoral: [0.12, 0.3, 0.36, 0.19],
    falcate: false,
    pelvic: [0.42, 0.48, 0.51, 0.065],
    caudal: 0.0,
    dorsal: [0.36, 0.44],
    second_dorsal: [0.49, 0.53],
    head: None,
    eye: Some([0.06, 0.09]),
    gills: [0.13, 0.2],
    tint: [1.08, 0.97, 0.74],
};

static GREAT_HAMMER: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.05 {
            return 0.04;
        }
        if s < 0.3 {
            return 0.04 + 0.07 * smoothstep(0.05, 0.3, s);
        }
        classic_half_width(s) * 0.95
    },
    head: Some(Head {
        span: 0.21,
        sweep: 0.006,
        depth: 0.085,
        notch: 0.014,
    }),
    pectoral: [0.16, 0.38, 0.49, 0.24],
    falcate: true,
    pelvic: [0.55, 0.62, 0.67, 0.085],
    caudal: 0.13,
    dorsal: [0.24, 0.45],
    second_dorsal: [0.68, 0.74],
    eye: None,
    gills: [0.15, 0.25],
    tint: [0.86, 0.93, 1.02],
};

static MAKO: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.17 {
            return 0.088 * ((s / 0.17) * PI / 2.0).sin().powf(1.5);
        }
        if s < 0.32 {
            return 0.088 + 0.012 * smoothstep(0.17, 0.32, s);
        }
        if s < 0.86 {
            return 0.014 + 0.086 * (1.0 - (s - 0.32) / 0.54).powf(1.45);
        }
        let k = (s - 0.86) / 0.14;
        0.014 * (1.0 - k) + 0.01 * (k * PI).sin()
    },
    pectoral: [0.18, 0.4, 0.5, 0.21],
    falcate: true,
    pelvic: [0.58, 0.64, 0.68, 0.06],
    caudal: 0.13,
    dorsal: [0.34, 0.45],
    second_dorsal: [0.76, 0.79],
    head: None,
    eye: Some([0.11, 0.15]),
    gills: [0.2, 0.3],
    tint: [0.78, 0.9, 1.2],
};

static THRESHER: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.08 {
            return 0.07 * ((s / 0.08) * PI / 2.0).sin().powf(0.75);
        }
        if s < 0.2 {
            return 0.07 + 0.02 * smoothstep(0.08, 0.2, s);
        }
        if s < 0.48 {
            return 0.012 + 0.078 * (1.0 - (s - 0.2) / 0.28).powf(1.3);
        }
        let k = (s - 0.48) / 0.52;
        0.012 * (1.0 - k) + 0.006 * (k * PI * 0.9).sin() + 0.002
    },
    pectoral: [0.12, 0.27, 0.36, 0.2],
    falcate: true,
    pelvic: [0.37, 0.41, 0.44, 0.045],
    caudal: 0.0,
    dorsal: [0.24, 0.32],
    second_dorsal: [0.45, 0.47],
    head: None,
    eye: Some([0.05, 0.1]),
    gills: [0.11, 0.18],
    tint: [1.0, 0.9, 1.08],
};

static NURSE: SharkShape = SharkShape {
    half_width: |s| {
        if s < 0.08 {
            return 0.085 * ((s / 0.08) * PI / 2.0).sin().powf(0.4);
        }
        if s < 0.3 {
            return 0.085 + 0.02 * smoothstep(0.08, 0.3, s);
        }
        if s < 0.72 {
            return 0.022 + 0.083 * (1.0 - (s - 0.3) / 0.42).powf(1.1);
        }
        let k = (s - 0.72) / 0.28;
        0.022 * (1.0 - k * k) + 0.014 * (k * PI).sin()
    },
    pectoral: [0.16, 0.34, 0.4, 0.17],
    falcate: false,
    pelvic: [0.45, 0.52, 0.55, 0.075],
    caudal: 0.0,
    dorsal: [0.5, 0.58],
    second_dorsal: [0.63, 0.69],
    head: None,
    eye: Some([0.05, 0.075]),
    gills: [0.14, 0.22],
    tint: [1.1, 0.96, 0.76],
};

fn shape(kind: SharkKind) -> &'static SharkShape {
    match kind {
        SharkKind::Classic => &CLASSIC,
        SharkKind::Hammerhead => &HAMMERHEAD,
        SharkKind::Tiger => &TIGER,
        SharkKind::Zebra => &ZEBRA,
        SharkKind::GreatHammer => &GREAT_HAMMER,
        SharkKind::Mako => &MAKO,
        SharkKind::Thresher => &THRESHER,
        SharkKind::Nurse => &NURSE,
    }
}

fn in_triangle(
    px: f64,
    py: f64,
    (ax, ay): (f64, f64),
    (bx, by): (f64, f64),
    (cx, cy): (f64, f64),
) -> bool {
    let d1 = (px - bx) * (ay - by) - (ax - bx) * (py - by);
    let d2 = (px - cx) * (by - cy) - (bx - cx) * (py - cy);
    let d3 = (px - ax) * (cy - ay) - (cx - ax) * (py - ay);
    let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
    let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
    !(neg && pos)
}

#[derive(Clone, Copy)]
struct Cells {
    cols: usize,
    rows: usize,
    cw: f64,
    ch: f64,
}

fn stamp(
    field: &mut [f32],
    Cells { cols, rows, cw, ch }: Cells,
    x: f64,
    y: f64,
    radius: i64,
    amount: f64,
) {
    let c0 = (x / cw).floor() as i64;
    let r0 = (y / ch).floor() as i64;
    for r in r0 - radius..=r0 + radius {
        for c in c0 - radius..=c0 + radius {
            if c < 0 || r < 0 || c >= cols as i64 || r >= rows as i64 {
                continue;
            }
            let d = ((c - c0) as f64).hypot((r - r0) as f64 * 1.4);
            if d > radius as f64 + 0.5 {
                continue;
            }
            let i = r as usize * cols + c as usize;
            field[i] =
                (1.6f64).min(field[i] as f64 + amount * (1.0 - d / (radius as f64 + 1.0))) as f32;
        }
    }
}

pub struct Renderer {
    cw: f64,
    ch: f64,
    cols: usize,
    rows: usize,

    energy: Vec<f32>,
    energy_tmp: Vec<f32>,
    heat: Vec<f32>,

    shark_flag: Vec<u8>,
    shark_shade: Vec<f32>,
    shark_key: Vec<u32>,
    shark_depth: Vec<f32>,
    shark_kind: Vec<SharkKind>,
    spine: [f32; SHARK_NODES * 2],
    arc: [f32; SHARK_NODES],
    arc_total: f64,
    segments: [Segment; SEGMENTS],
    bounds: [Bound; CHUNKS],
    max_reach: f64,
    shape: &'static SharkShape,
    kind: SharkKind,
    shark_id: u32,
    fin_tip_left: f64,
    fin_tip_right: f64,
    hit_flag: u8,
    hit_shade: f64,
    hit_s: f64,
    hit_v: f64,
    hit_dist: f64,

    fish_stamp: Vec<u32>,
    frame_id: u32,
    fish_buckets: [Vec<usize>; 5],
    ripples: Vec<Ripple>,
}

impl Renderer {
    pub fn new() -> Self {
        Renderer {
            cw: 12.0,
            ch: 18.0,
            cols: 0,
            rows: 0,
            energy: Vec::new(),
            energy_tmp: Vec::new(),
            heat: Vec::new(),
            shark_flag: Vec::new(),
            shark_shade: Vec::new(),
            shark_key: Vec::new(),
            shark_depth: Vec::new(),
            shark_kind: Vec::new(),
            spine: [0.0; SHARK_NODES * 2],
            arc: [0.0; SHARK_NODES],
            arc_total: 1.0,
            segments: [Segment::default(); SEGMENTS],
            bounds: [Bound::default(); CHUNKS],
            max_reach: 0.0,
            shape: &CLASSIC,
            kind: SharkKind::Classic,
            shark_id: 0,
            fin_tip_left: 0.0,
            fin_tip_right: 0.0,
            hit_flag: 0,
            hit_shade: 0.0,
            hit_s: 0.0,
            hit_v: 0.0,
            hit_dist: 0.0,
            fish_stamp: Vec::new(),
            frame_id: 0,
            fish_buckets: Default::default(),
            ripples: Vec::new(),
        }
    }

    /// Not in upstream.
    pub fn ripple(&mut self, x: f64, y: f64, t: f64) {
        if self.ripples.len() >= 16 {
            self.ripples.remove(0);
        }
        self.ripples.push(Ripple { x, y, born: t });
    }

    /// Upstream derives cw/ch from the canvas size; here the caller picks them to
    /// fit the terminal's character cells.
    pub fn resize(&mut self, cols: usize, rows: usize, cw: f64, ch: f64) {
        self.cw = cw;
        self.ch = ch;
        if cols == self.cols && rows == self.rows {
            return;
        }
        self.cols = cols;
        self.rows = rows;
        let n = cols * rows;
        self.energy = vec![0.0; n];
        self.energy_tmp = vec![0.0; n];
        self.heat = vec![0.0; n];
        self.shark_flag = vec![0; n];
        self.shark_shade = vec![0.0; n];
        self.shark_key = vec![0; n];
        self.shark_depth = vec![0.0; n];
        self.shark_kind = vec![SharkKind::Classic; n];
        self.fish_stamp = vec![0; n];
    }

    pub fn render(&mut self, world: &mut World, dt: f64, grid: &mut Grid) {
        if self.cols == 0 {
            return;
        }
        let t = world.time;
        self.ripples.retain(|r| t - r.born < RIPPLE_SPAN);

        self.consume_events(world);
        self.rasterize_sharks(&world.sharks, t);
        self.deposit_wake(world);
        self.diffuse(dt);

        grid.clear();
        self.mark_fish(world);
        self.draw_dots(grid);
        self.draw_heat(grid);
        self.draw_fish(grid);
        self.draw_ripples(t, grid);
        self.draw_sharks(t, grid);
    }

    /// Not in upstream.
    fn draw_ripples(&self, t: f64, grid: &mut Grid) {
        let (cw, ch, cols, rows) = (self.cw, self.ch, self.cols as i64, self.rows as i64);
        for rp in &self.ripples {
            for k in (0..RIPPLE_RINGS).rev() {
                let age = t - rp.born - k as f64 * RIPPLE_GAP;
                if !(0.0..RIPPLE_LIFE).contains(&age) {
                    continue;
                }
                let r = cw + age * RIPPLE_SPEED;
                let fade = 1.0 - age / RIPPLE_LIFE;
                let alpha = fade * fade.sqrt() * (1.0 - 0.25 * k as f64);
                // The cell test below never accepts a cell further than `band` off the ring.
                let band = 0.5 * cw.max(ch);
                let (outer, inner) = (r + band, (r - band).max(0.0));
                let r0 = (((rp.y - outer) / ch).floor() as i64 - 1).max(0);
                let r1 = (((rp.y + outer) / ch).floor() as i64 + 1).min(rows - 1);
                for row in r0..=r1 {
                    let oy = (row as f64 + 0.5) * ch - rp.y;
                    if oy.abs() > outer {
                        continue;
                    }
                    let out_x = (outer * outer - oy * oy).sqrt();
                    let in_x = if inner > oy.abs() {
                        (inner * inner - oy * oy).sqrt()
                    } else {
                        0.0
                    };
                    let both = [(-out_x, -in_x), (in_x, out_x)];
                    let spans: &[(f64, f64)] = if in_x > 0.0 {
                        &both
                    } else {
                        &[(-out_x, out_x)]
                    };
                    for &(lo, hi) in spans {
                        let c0 = (((rp.x + lo) / cw - 0.5).floor() as i64).max(0);
                        let c1 = (((rp.x + hi) / cw - 0.5).ceil() as i64).min(cols - 1);
                        for col in c0..=c1 {
                            let ox = (col as f64 + 0.5) * cw - rp.x;
                            let d = ox.hypot(oy);
                            if d == 0.0 {
                                continue;
                            }
                            let (ux, uy) = (ox / d, oy / d);
                            if (d - r).abs() > 0.5 * (ux.abs() * cw).max(uy.abs() * ch) {
                                continue;
                            }
                            let glyph = if uy.abs() < 0.38 {
                                if ux < 0.0 { '(' } else { ')' }
                            } else if ux.abs() < 0.38 {
                                '~'
                            } else if ux * uy > 0.0 {
                                '/'
                            } else {
                                '\\'
                            };
                            grid.text(
                                row as usize * self.cols + col as usize,
                                glyph,
                                RIPPLE_RGB,
                                alpha,
                            );
                        }
                    }
                }
            }
        }
    }

    fn cell_index(&self, x: f64, y: f64) -> Option<usize> {
        let c = (x / self.cw).floor();
        let r = (y / self.ch).floor();
        if c < 0.0 || r < 0.0 || c >= self.cols as f64 || r >= self.rows as f64 {
            return None;
        }
        Some(r as usize * self.cols + c as usize)
    }

    fn cells(&self) -> Cells {
        Cells {
            cols: self.cols,
            rows: self.rows,
            cw: self.cw,
            ch: self.ch,
        }
    }

    fn consume_events(&mut self, world: &mut World) {
        let cells = self.cells();
        for e in world.events.drain(..) {
            match e.kind {
                EventKind::Eat => {
                    stamp(&mut self.heat, cells, e.x, e.y, 2, 1.0);
                    stamp(&mut self.energy, cells, e.x, e.y, 3, 0.9);
                }
                EventKind::Splash => stamp(&mut self.energy, cells, e.x, e.y, 6, 1.4),
                EventKind::Disturb => self.ripple(e.x, e.y, world.time),
            }
        }
    }

    fn deposit_wake(&mut self, world: &World) {
        for f in &world.fish {
            if !f.alive {
                continue;
            }
            if let Some(i) = self.cell_index(f.x, f.y) {
                self.energy[i] =
                    (1.6f64).min(self.energy[i] as f64 + 0.012 + f.alarm * 0.03) as f32;
            }
        }
        for sh in &world.sharks {
            for k in SHARK_NODES - 6..SHARK_NODES {
                if let Some(i) = self.cell_index(sh.nodes[k * 2] as f64, sh.nodes[k * 2 + 1] as f64)
                {
                    self.energy[i] =
                        (1.6f64).min(self.energy[i] as f64 + 0.05 + sh.speed * 0.0012) as f32;
                }
            }
        }
        if world.pointer.active {
            let cells = self.cells();
            stamp(
                &mut self.energy,
                cells,
                world.pointer.x,
                world.pointer.y,
                1,
                0.08,
            );
        }
    }

    fn diffuse(&mut self, dt: f64) {
        let (cols, rows) = (self.cols, self.rows);
        let energy = &self.energy;
        let tmp = &mut self.energy_tmp;
        let decay = (-dt * 1.5).exp();
        for r in 0..rows {
            for c in 0..cols {
                let i = r * cols + c;
                let e = energy[i] as f64;
                let l = if c > 0 { energy[i - 1] as f64 } else { e };
                let rt = if c < cols - 1 {
                    energy[i + 1] as f64
                } else {
                    e
                };
                let u = if r > 0 { energy[i - cols] as f64 } else { e };
                let d = if r < rows - 1 {
                    energy[i + cols] as f64
                } else {
                    e
                };
                tmp[i] = ((e * 4.0 + l + rt + u + d) * 0.125 * decay) as f32;
            }
        }
        std::mem::swap(&mut self.energy, &mut self.energy_tmp);
        let heat_decay = (-dt * 2.2).exp();
        for h in &mut self.heat {
            *h = (*h as f64 * heat_decay) as f32;
        }
    }

    fn sample_shark(&mut self, px: f64, py: f64, l: f64) -> bool {
        let mut best = f64::INFINITY;
        let mut best_s = 0.0;
        let mut best_sign = 1.0;
        let mut best_nx = 0.0;
        let mut best_ny = 0.0;

        // Chunk starts are spine points, so the nearest of them bounds the answer;
        // a chunk whose circle lies beyond that can't hold the nearest segment.
        // The survivors run in upstream's order, so ties resolve the same way.
        let mut near = f64::INFINITY;
        for j in 0..CHUNKS {
            let s = &self.segments[j * CHUNK];
            near = near.min((px - s.ax) * (px - s.ax) + (py - s.ay) * (py - s.ay));
        }
        let near = near.sqrt();
        for (j, b) in self.bounds.iter().enumerate() {
            let reach = near + b.r + BOUND_EPS;
            if (px - b.cx) * (px - b.cx) + (py - b.cy) * (py - b.cy) > reach * reach {
                continue;
            }
            for k in j * CHUNK..((j + 1) * CHUNK).min(SEGMENTS) {
                let Segment {
                    ax,
                    ay,
                    sx,
                    sy,
                    len2,
                    arc,
                } = self.segments[k];
                let u = clamp(((px - ax) * sx + (py - ay) * sy) / len2, 0.0, 1.0);
                let qx = ax + sx * u;
                let qy = ay + sy * u;
                let d2 = (px - qx) * (px - qx) + (py - qy) * (py - qy);
                if d2 < best {
                    best = d2;
                    let len = len2.sqrt();
                    best_s = (arc + u * len) / self.arc_total;
                    best_sign = if sx * (py - ay) - sy * (px - ax) >= 0.0 {
                        1.0
                    } else {
                        -1.0
                    };
                    best_nx = (-sy / len) * best_sign;
                    best_ny = (sx / len) * best_sign;
                }
            }
        }

        let a = best.sqrt();
        let s = best_s;
        let u = s * l;
        let shape = self.shape;
        // Beyond MAX_HALF_WIDTH the body test below fails anyway; skip the pow() calls.
        let w = if a <= MAX_HALF_WIDTH * l {
            (shape.half_width)(s) * l
        } else {
            f64::NEG_INFINITY
        };
        self.hit_dist = a;
        self.hit_s = s;
        self.hit_v = a * best_sign;

        if let Some(head) = &shape.head
            && u < l * (head.sweep + head.depth + head.notch)
        {
            let span = l * head.span;
            let r = a / span;
            let front = l * head.sweep * r * r + l * head.notch * (1.0 - r / 0.14).max(0.0);
            let depth = l * head.depth * (1.0 - 0.2 * r * r * r * r);
            if r <= 1.0 && u >= front && u <= front + depth {
                self.hit_flag = if r > 0.84 { FLAG_EYE } else { FLAG_BODY };
                self.hit_shade = if r > 0.84 {
                    0.0
                } else {
                    0.62 - 0.2 * r + if u < front + l * 0.012 { 0.1 } else { 0.0 }
                };
                return true;
            }
        }

        if a <= w {
            let across = a / if w == 0.0 { 1.0 } else { w };
            let facing = (best_nx * -0.6 + best_ny * -0.8) * across;
            let mut shade = 0.58 - across * across * 0.42 + facing * 0.26;
            let mut flag = FLAG_BODY;
            if s > shape.dorsal[0] && s < shape.dorsal[1] && across < 0.3 {
                flag = FLAG_RIDGE;
                shade = 1.0;
            } else if s > shape.second_dorsal[0] && s < shape.second_dorsal[1] && across < 0.4 {
                shade = shade.max(0.82);
            } else if let Some(eye) = shape.eye
                && s > eye[0]
                && s < eye[1]
                && across > 0.6
            {
                flag = FLAG_EYE;
                shade = 0.0;
            } else if s > shape.gills[0] && s < shape.gills[1] && across > 0.58 {
                if (((s - shape.gills[0]) / (shape.gills[1] - shape.gills[0])) * 5.0).floor() as i64
                    % 2
                    == 0
                {
                    shade *= 0.45;
                }
            } else if s < 0.05 {
                shade = shade.max(0.7);
            } else if self.kind == SharkKind::Zebra
                && s > 0.08
                && s < 0.55
                && (across < 0.1 || (across - 0.6).abs() < 0.08)
            {
                shade = shade.max(0.8);
            } else if self.marked(s, a * best_sign, l, across) {
                flag = FLAG_MARK;
            }
            self.hit_flag = flag;
            self.hit_shade = shade;
            return true;
        }

        let tip = if best_sign > 0.0 {
            self.fin_tip_right
        } else {
            self.fin_tip_left
        };
        let [p0, p1, p2, _] = shape.pectoral;
        if in_triangle(u, a, (l * p0, 0.0), (l * p1, 0.0), (l * p2, tip))
            && !(shape.falcate
                && in_triangle(
                    u,
                    a,
                    (l * p1, 0.0),
                    (l * (p1 + (p2 - p1) * 0.33), tip * 0.55),
                    (l * p2, tip),
                ))
        {
            self.hit_flag = if self.marked(s, a * best_sign, l, 0.0) {
                FLAG_MARK
            } else {
                FLAG_FIN
            };
            self.hit_shade = 0.38 + 0.22 * (1.0 - clamp(a / tip, 0.0, 1.0));
            return true;
        }
        let [v0, v1, v2, vr] = shape.pelvic;
        if in_triangle(u, a, (l * v0, 0.0), (l * v1, 0.0), (l * v2, l * vr))
            || (shape.caudal > 0.0
                && in_triangle(
                    u,
                    a,
                    (l * 0.86, 0.0),
                    (l * 0.95, 0.0),
                    (l, l * shape.caudal),
                ))
        {
            self.hit_flag = FLAG_FIN;
            self.hit_shade = 0.36;
            return true;
        }
        false
    }

    fn beyond(&self, px: f64, py: f64, dist: f64) -> bool {
        self.bounds.iter().all(|b| {
            let reach = dist + b.r + BOUND_EPS;
            (px - b.cx) * (px - b.cx) + (py - b.cy) * (py - b.cy) > reach * reach
        })
    }

    fn index_spine(&mut self) {
        for k in 0..SEGMENTS {
            let ax = self.spine[k * 2] as f64;
            let ay = self.spine[k * 2 + 1] as f64;
            let sx = self.spine[(k + 1) * 2] as f64 - ax;
            let sy = self.spine[(k + 1) * 2 + 1] as f64 - ay;
            let len2 = sx * sx + sy * sy;
            let len2 = if len2 == 0.0 { 1.0 } else { len2 };
            self.segments[k] = Segment {
                ax,
                ay,
                sx,
                sy,
                len2,
                arc: self.arc[k] as f64,
            };
        }
        for j in 0..CHUNKS {
            let nodes = j * CHUNK..=((j + 1) * CHUNK).min(SEGMENTS);
            let node = |k: usize| (self.spine[k * 2] as f64, self.spine[k * 2 + 1] as f64);
            let (mut x0, mut y0, mut x1, mut y1) = (
                f64::INFINITY,
                f64::INFINITY,
                f64::NEG_INFINITY,
                f64::NEG_INFINITY,
            );
            for k in nodes.clone() {
                let (x, y) = node(k);
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
            let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
            let r = nodes.map(|k| {
                let (x, y) = node(k);
                ((x - cx) * (x - cx) + (y - cy) * (y - cy)).sqrt()
            });
            self.bounds[j] = Bound {
                cx,
                cy,
                r: r.fold(0.0, f64::max),
            };
        }
    }

    fn marked(&self, s: f64, v: f64, l: f64, across: f64) -> bool {
        match self.kind {
            SharkKind::Tiger => {
                if !(0.2..=0.9).contains(&s) || across > 0.85 {
                    return false;
                }
                let wobble = 0.35 * ((v / l) * 25.0 + s * 5.0).sin();
                let bar = (s * PI * 2.0 * 9.0 + wobble).sin();
                bar > 0.65 - 0.5 * smoothstep(0.2, 0.42, s) + 0.4 * smoothstep(0.8, 0.9, s)
            }
            SharkKind::Zebra => {
                if s < 0.07 {
                    return false;
                }
                let gu = (s * l) / (l * 0.06);
                let gv = v / (l * 0.045);
                let cu = gu.floor();
                let cv = gv.floor();
                let id = self.shark_id as i32;
                let jx = hash3(to_i32(cu), to_i32(cv), id) * 0.5 + 0.25;
                let jy = hash3(to_i32(cu), to_i32(cv), id + 11) * 0.5 + 0.25;
                (gu - cu - jx) * (gu - cu - jx) + (gv - cv - jy) * (gv - cv - jy) < 0.08
            }
            _ => false,
        }
    }

    fn rasterize_sharks(&mut self, sharks: &[Shark], t: f64) {
        let (cw, ch, cols, rows) = (self.cw, self.ch, self.cols, self.rows);
        let n_nodes = SHARK_NODES;
        self.shark_flag.fill(0);
        let flicker_tick = to_i32((t * 6.0).floor());

        for sh in sharks {
            let l = sh.length;
            let n = &sh.nodes;
            let swim = clamp(sh.speed / 60.0, 0.4, 1.4);
            self.shape = shape(sh.kind);
            self.kind = sh.kind;
            self.shark_id = sh.id;

            for k in 0..n_nodes {
                let s = k as f64 / (n_nodes - 1) as f64;
                let a = k.saturating_sub(1);
                let b = (k + 1).min(n_nodes - 1);
                let mut tx = n[a * 2] as f64 - n[b * 2] as f64;
                let mut ty = n[a * 2 + 1] as f64 - n[b * 2 + 1] as f64;
                let tl = tx.hypot(ty);
                let tl = if tl == 0.0 { 1.0 } else { tl };
                tx /= tl;
                ty /= tl;
                let amp = l * 0.05 * (0.1 + 0.9 * s.powf(1.8)) * swim;
                let off = amp * (sh.phase - s * 5.2).sin();
                self.spine[k * 2] = (n[k * 2] as f64 - ty * off) as f32;
                self.spine[k * 2 + 1] = (n[k * 2 + 1] as f64 + tx * off) as f32;
            }
            self.arc[0] = 0.0;
            for k in 1..n_nodes {
                let dx = self.spine[k * 2] as f64 - self.spine[(k - 1) * 2] as f64;
                let dy = self.spine[k * 2 + 1] as f64 - self.spine[(k - 1) * 2 + 1] as f64;
                self.arc[k] = (self.arc[k - 1] as f64 + dx.hypot(dy)) as f32;
            }
            let total = self.arc[n_nodes - 1] as f64;
            self.arc_total = if total == 0.0 { 1.0 } else { total };

            let bank = clamp(sh.turn * 0.45, -0.35, 0.35);
            let flex = 1.0 + 0.06 * (sh.phase * 0.5).sin();
            let reach_fin = self.shape.pectoral[3];
            self.fin_tip_right = l * reach_fin * flex * (1.0 - bank);
            self.fin_tip_left = l * reach_fin * flex * (1.0 + bank);
            self.index_spine();
            // Pectoral tips reach farthest; 0.25 L covers every other part (the widest,
            // a great hammerhead's head, spans 0.21 L), so nothing hits beyond this.
            self.max_reach = self.fin_tip_left.max(self.fin_tip_right).max(l * 0.25);

            let mut min_x = f64::INFINITY;
            let mut min_y = f64::INFINITY;
            let mut max_x = f64::NEG_INFINITY;
            let mut max_y = f64::NEG_INFINITY;
            for k in 0..n_nodes {
                min_x = min_x.min(self.spine[k * 2] as f64);
                max_x = max_x.max(self.spine[k * 2] as f64);
                min_y = min_y.min(self.spine[k * 2 + 1] as f64);
                max_y = max_y.max(self.spine[k * 2 + 1] as f64);
            }
            let reach = l * 0.36;
            let c0 = ((min_x - reach) / cw).floor().max(0.0) as i64;
            let c1 = ((max_x + reach) / cw).floor().min((cols - 1) as f64) as i64;
            let r0 = ((min_y - reach) / ch).floor().max(0.0) as i64;
            let r1 = ((max_y + reach) / ch).floor().min((rows - 1) as f64) as i64;
            if c0 > c1 || r0 > r1 {
                continue;
            }

            let skip_beyond = reach + cw.hypot(ch);
            // Cells provably beyond both upstream's skip distance and the shape's reach
            // would be sampled only to be skipped; leave them out.
            let never = skip_beyond.max(self.max_reach);

            for r in r0..=r1 {
                for c in c0..=c1 {
                    let x = c as f64 * cw;
                    let y = r as f64 * ch;
                    if self.beyond(x + cw / 2.0, y + ch / 2.0, never) {
                        continue;
                    }
                    let center_hit = self.sample_shark(x + cw / 2.0, y + ch / 2.0, l);
                    if !center_hit && self.hit_dist > skip_beyond {
                        continue;
                    }
                    let flag = self.hit_flag;
                    let shade = self.hit_shade;
                    let s = self.hit_s;
                    let v = self.hit_v;

                    let mut hits = if center_hit { 1 } else { 0 };
                    for (fx, fy) in SUBSAMPLES {
                        let (sx, sy) = (x + fx * cw, y + fy * ch);
                        if !self.beyond(sx, sy, self.max_reach) && self.sample_shark(sx, sy, l) {
                            hits += 1;
                        }
                    }
                    if hits == 0 {
                        continue;
                    }
                    let coverage = hits as f64 / 5.0;

                    let i = r as usize * cols + c as usize;
                    let key = (hash3(
                        to_i32(js_round(s * 52.0)),
                        to_i32(js_round(v / (cw * 0.9))),
                        sh.id as i32,
                    ) * 4_294_967_295.0) as u32;
                    self.shark_key[i] = key;
                    self.shark_depth[i] = sh.depth as f32;
                    self.shark_kind[i] = sh.kind;

                    if center_hit && coverage >= 0.6 {
                        self.shark_flag[i] = flag;
                        let flicker = (hash3(c as i32, r as i32, flicker_tick) - 0.5) * 0.06;
                        self.shark_shade[i] = clamp(shade + flicker, 0.0, 1.0) as f32;
                    } else {
                        self.shark_flag[i] = FLAG_EDGE;
                        self.shark_shade[i] = coverage as f32;
                    }
                }
            }
        }
    }

    fn draw_sharks(&self, t: f64, grid: &mut Grid) {
        for i in 0..self.cols * self.rows {
            let flag = self.shark_flag[i];
            if flag == 0 {
                continue;
            }
            let key = self.shark_key[i];
            let shade = self.shark_shade[i] as f64;
            let dim = 0.7 + 0.3 * self.shark_depth[i] as f64;
            let [tr, tg, tb] = shape(self.shark_kind[i]).tint;

            let tick = (t * 2.4 + (key % 997) as f64 / 97.0).floor();
            let roll = hash3(key as i32, to_i32(tick), 7);
            let pick = |set: &[u8]| set[((key >> 8) as usize) % set.len()] as char;

            if flag == FLAG_EDGE {
                let set = if shade >= 0.5 { EDGE_HEAVY } else { EDGE_LIGHT };
                let g = (70.0 + shade * 80.0) * dim;
                let color = if roll < 0.07 {
                    ACCENTS[key as usize % ACCENTS.len()]
                } else {
                    rgb_f(g * tr, g * tg, g * tb + 6.0)
                };
                grid.text(i, pick(set), color, 1.0);
                continue;
            }

            let mut level;
            let glyph;
            if flag == FLAG_RIDGE {
                level = 226.0;
                glyph = pick(RIDGE_GLYPHS);
            } else if flag == FLAG_EYE {
                level = 12.0;
                glyph = 'o';
            } else if flag == FLAG_MARK {
                level = 24.0 + shade * 22.0;
                glyph = pick(MARK_GLYPHS);
            } else if flag == FLAG_FIN {
                level = 40.0 + shade * 120.0;
                glyph = pick(FIN_GLYPHS);
            } else {
                level = 46.0 + shade * 160.0;
                let jitter = ((key >> 4) % 3) as i64 - 1;
                let idx = ((1.0 - shade) * 8.99).floor() as i64 + jitter;
                glyph = SKIN_RAMP[idx.clamp(0, SKIN_RAMP.len() as i64 - 1) as usize] as char;
            }
            level *= dim;
            grid.block(i, rgb_f(level * tr, level * tg, level * tb + 4.0));

            let color = if flag == FLAG_EYE {
                0x9aa0ad
            } else if flag == FLAG_MARK {
                rgb_f(70.0 * tr, 70.0 * tg, 70.0 * tb)
            } else if roll < 0.09 {
                ACCENTS[key as usize % ACCENTS.len()]
            } else if roll < 0.13 {
                0xffffff
            } else if shade > 0.55 {
                0x0e0f13
            } else {
                0x1b1d24
            };
            grid.text(i, glyph, color, 1.0);
        }
    }

    fn mark_fish(&mut self, world: &World) {
        self.frame_id = self.frame_id.wrapping_add(1);
        let frame = self.frame_id;
        for bucket in &mut self.fish_buckets {
            bucket.clear();
        }
        for pass in 0..2 {
            for f in &world.fish {
                if !f.alive || (if f.depth > 0.78 { 0 } else { 1 }) != pass {
                    continue;
                }
                let Some(i) = self.cell_index(f.x, f.y) else {
                    continue;
                };
                if self.fish_stamp[i] == frame || self.shark_flag[i] != 0 {
                    continue;
                }
                self.fish_stamp[i] = frame;
                let glint = clamp(f.turn.abs() / 3.5, 0.0, 1.0);
                let v = 0.3 + (0.4 * (f.depth - 0.55)) / 0.45 + 0.3 * glint.max(f.alarm);
                let b = clamp(
                    (v * FISH_ALPHAS.len() as f64).floor(),
                    0.0,
                    (FISH_ALPHAS.len() - 1) as f64,
                );
                self.fish_buckets[b as usize].push(i);
            }
        }
    }

    fn draw_fish(&self, grid: &mut Grid) {
        for (b, bucket) in self.fish_buckets.iter().enumerate() {
            for &i in bucket {
                grid.text(i, '-', FISH_RGB, FISH_ALPHAS[b]);
            }
        }
    }

    fn draw_heat(&self, grid: &mut Grid) {
        for (i, &h) in self.heat.iter().enumerate() {
            let h = h as f64;
            if h < 0.08 {
                continue;
            }
            let (color, alpha) = if h > 0.5 {
                (rgb(255, 120, 60), fixed(h, 2))
            } else {
                (rgb(220, 40, 40), fixed(h * 1.6, 2))
            };
            grid.text(i, if h > 0.6 { '*' } else { '·' }, color, alpha);
        }
    }

    fn draw_dots(&self, grid: &mut Grid) {
        for i in 0..self.cols * self.rows {
            let e = self.energy[i] as f64;
            if self.shark_flag[i] != 0 || self.fish_stamp[i] == self.frame_id || e <= 0.45 {
                continue;
            }
            let level = clamp(((e - 0.45) * 5.0).floor(), 0.0, (ALPHA_LEVELS - 1) as f64);
            grid.text(
                i,
                DOT_GLYPH,
                WAKE_RGB,
                fixed((level + 1.0) / ALPHA_LEVELS as f64, 3),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn half_widths_stay_under_bound() {
        for kind in SharkKind::ALL {
            let shape = shape(kind);
            for i in 0..=100_000 {
                let s = i as f64 / 100_000.0;
                let w = (shape.half_width)(s);
                assert!(w <= MAX_HALF_WIDTH, "{kind:?} half_width({s}) = {w}");
            }
        }
    }

    /// max_reach assumes 0.25 L covers everything but the pectoral tips.
    #[test]
    fn other_parts_fit_in_quarter_length() {
        for kind in SharkKind::ALL {
            let shape = shape(kind);
            let head = shape.head.as_ref().map_or(0.0, |h| h.span);
            for reach in [head, shape.pelvic[3], shape.caudal, MAX_HALF_WIDTH] {
                assert!(reach <= 0.25, "{kind:?} reaches {reach} L");
            }
        }
    }
}
