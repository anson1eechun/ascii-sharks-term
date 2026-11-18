//! Port of upstream src/world.ts, statement for statement, with f32 wherever
//! upstream uses Float32Array, so verify/ can compare the two exactly.

use crate::math::{PI, TAU, angle_diff, clamp, current, js_round, rand, rand_int, random};
use crate::species::{SPECIES, pick_species};

pub const SHARK_NODES: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SharkKind {
    Classic,
    Hammerhead,
    GreatHammer,
    Tiger,
    Mako,
    Thresher,
    Nurse,
    Zebra,
}

impl SharkKind {
    pub const ALL: [SharkKind; 8] = [
        SharkKind::Classic,
        SharkKind::Hammerhead,
        SharkKind::GreatHammer,
        SharkKind::Tiger,
        SharkKind::Mako,
        SharkKind::Thresher,
        SharkKind::Nurse,
        SharkKind::Zebra,
    ];

    pub fn name(self) -> &'static str {
        match self {
            SharkKind::Classic => "classic",
            SharkKind::Hammerhead => "hammerhead",
            SharkKind::GreatHammer => "greathammer",
            SharkKind::Tiger => "tiger",
            SharkKind::Mako => "mako",
            SharkKind::Thresher => "thresher",
            SharkKind::Nurse => "nurse",
            SharkKind::Zebra => "zebra",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.name() == s)
    }

    pub fn label(self, zh: bool) -> &'static str {
        let (en, zh_name) = match self {
            SharkKind::Classic => ("classic (requiem shark)", "經典鯊（真鯊）"),
            SharkKind::Hammerhead => ("smooth hammerhead", "雙髻鯊（錘頭雙髻鯊）"),
            SharkKind::GreatHammer => ("great hammerhead", "無溝雙髻鯊"),
            SharkKind::Tiger => ("tiger shark", "虎鯊"),
            SharkKind::Mako => ("shortfin mako", "尖吻鯖鯊"),
            SharkKind::Thresher => ("thresher shark", "長尾鯊"),
            SharkKind::Nurse => ("nurse shark", "鉸口鯊"),
            SharkKind::Zebra => ("zebra shark", "豹紋鯊"),
        };
        if zh { zh_name } else { en }
    }
}

#[derive(Clone, Copy)]
pub struct SharkSpec {
    pub kind: SharkKind,
    pub size: f64,
}

#[derive(Clone, Copy)]
pub struct Fish {
    pub x: f64,
    pub y: f64,
    pub vx: f64,
    pub vy: f64,
    pub species: usize,
    pub alarm: f64,
    pub alive: bool,
    pub respawn_at: f64,
    pub turn: f64,
    pub depth: f64,
    pub mill_dir: f64,
    /// Not in upstream: dropped by a feed; never respawns once eaten.
    pub bait: bool,
}

#[derive(Clone, Copy)]
pub struct Shark {
    pub id: u32,
    pub kind: SharkKind,
    pub length: f64,
    pub nodes: [f32; SHARK_NODES * 2],
    pub heading: f64,
    pub speed: f64,
    pub phase: f64,
    pub turn: f64,
    pub hunger: f64,
    pub wander: f64,
    pub orbit_dir: f64,
    pub orbit_radius: f64,
    pub target: Option<usize>,
    pub retarget_at: f64,
    pub depth: f64,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    Eat,
    Splash,
    /// Not in upstream.
    Disturb,
    Drop,
    Feed,
}

pub struct WorldEvent {
    pub kind: EventKind,
    pub x: f64,
    pub y: f64,
}

/// Not in upstream.
struct Disturbance {
    x: f64,
    y: f64,
    at: f64,
}

const SCARE_SECONDS: f64 = 4.0;
const SCARE_K: f64 = 6.0;
const SCARE_FLEE: f64 = 0.15;

#[derive(Default)]
pub struct Pointer {
    pub x: f64,
    pub y: f64,
    pub active: bool,
}

const FISH_SEPARATION_K: f64 = 420.0;
const FISH_ALIGN_K: f64 = 1.4;
const FISH_COHESION_K: f64 = 1.0;
const FISH_FLEE_K: f64 = 1100.0;
const BLIND_COS: f64 = -0.6; // ~127° either side of the heading is visible

pub struct World {
    pub width: f64,
    pub height: f64,
    pub time: f64,
    pub fish: Vec<Fish>,
    pub sharks: Vec<Shark>,
    pub events: Vec<WorldEvent>,
    pub pointer: Pointer,

    perception: f64,
    sep_x: [f64; SPECIES.len()],
    sep_y: [f64; SPECIES.len()],
    foreign_sep: f64,
    bucket_cols: usize,
    bucket_rows: usize,
    bucket_head: Vec<i32>,
    bucket_count: Vec<u16>,
    bucket_next: Vec<i32>,
    last_spawn: f64,
    shark_specs: Vec<SharkSpec>,
    fish_count: Option<usize>,
    disturbances: Vec<Disturbance>,
    due: Vec<usize>,
}

impl World {
    pub fn new(shark_specs: Vec<SharkSpec>, fish_count: Option<usize>) -> Self {
        World {
            width: 0.0,
            height: 0.0,
            time: 0.0,
            fish: Vec::new(),
            sharks: Vec::new(),
            events: Vec::new(),
            pointer: Pointer::default(),
            perception: 60.0,
            sep_x: [0.0; SPECIES.len()],
            sep_y: [0.0; SPECIES.len()],
            foreign_sep: 40.0,
            bucket_cols: 0,
            bucket_rows: 0,
            bucket_head: Vec::new(),
            bucket_count: Vec::new(),
            bucket_next: Vec::new(),
            last_spawn: 0.0,
            shark_specs,
            fish_count,
            disturbances: Vec::new(),
            due: Vec::new(),
        }
    }

    pub fn resize(&mut self, width: f64, height: f64, cell_w: f64, cell_h: f64) {
        let first = self.width == 0.0;
        self.width = width;
        self.height = height;
        self.sep_x = SPECIES.map(|sp| sp.spacing[0] * cell_w);
        self.sep_y = SPECIES.map(|sp| sp.spacing[1] * cell_h);
        self.perception = (cell_h * 3.0).max(cell_w * 5.5);
        self.foreign_sep = cell_w * 3.0;

        if first || self.fish.is_empty() {
            self.seed();
            return;
        }
        for f in &mut self.fish {
            f.x = clamp(f.x, 0.0, width);
            f.y = clamp(f.y, 0.0, height);
        }
        self.repopulate();
    }

    fn population(&self) -> usize {
        self.fish_count.unwrap_or_else(|| {
            clamp(js_round((self.width * self.height) / 4400.0), 85.0, 340.0) as usize
        })
    }

    /// Not in upstream: after a resize, grows or shrinks the school population to fit.
    /// New fish start out due to respawn, so they swim in from the edges as schools.
    fn repopulate(&mut self) {
        let want = self.population();
        let have = self.fish.iter().filter(|f| !f.bait).count();
        if want > have {
            let template = Fish {
                alive: false,
                respawn_at: self.time,
                bait: false,
                ..self.fish[0]
            };
            self.fish.extend(std::iter::repeat_n(template, want - have));
        } else if want < have {
            let mut extra = have - want;
            for i in (0..self.fish.len()).rev() {
                if extra == 0 {
                    break;
                }
                if !self.fish[i].bait {
                    self.fish.remove(i);
                    extra -= 1;
                }
            }
            for sh in &mut self.sharks {
                sh.target = None;
            }
        }
    }

    fn seed(&mut self) {
        let (w, h) = (self.width, self.height);
        let min_side = w.min(h);
        let base_length = clamp(min_side * 0.44, 170.0, 430.0);
        let shark_count = self.shark_specs.len();

        self.sharks.clear();
        for i in 0..shark_count {
            let SharkSpec { kind, size } = self.shark_specs[i];
            let length = base_length * size;
            let orbit_radius = min_side * rand(0.28, 0.4);
            let a = (i as f64 / shark_count as f64) * TAU + rand(-0.4, 0.4);
            let orbit_dir = if i % 2 == 0 { 1.0 } else { -1.0 };
            let x = w / 2.0 + a.cos() * orbit_radius * (w / min_side) * 0.9;
            let y = h / 2.0 + a.sin() * orbit_radius;
            let heading = a + (orbit_dir * PI) / 2.0;
            let mut nodes = [0f32; SHARK_NODES * 2];
            let seg = length / (SHARK_NODES - 1) as f64;
            for n in 0..SHARK_NODES {
                nodes[n * 2] = (x - heading.cos() * seg * n as f64) as f32;
                nodes[n * 2 + 1] = (y - heading.sin() * seg * n as f64) as f32;
            }
            self.sharks.push(Shark {
                id: i as u32 + 1,
                kind,
                length,
                nodes,
                heading,
                speed: 30.0,
                phase: rand(0.0, TAU),
                turn: 0.0,
                hunger: rand(0.25, 0.75),
                wander: 0.0,
                orbit_dir,
                orbit_radius,
                target: None,
                retarget_at: 0.0,
                depth: size,
            });
        }
        self.sharks.sort_by(|a, b| a.length.total_cmp(&b.length));

        let total = self.population();
        self.fish.clear();
        while self.fish.len() < total {
            let species = pick_species();
            let sp = &SPECIES[species];
            let size = (rand_int(sp.school_size[0], sp.school_size[1]) as usize)
                .min(total - self.fish.len());
            let mill_dir = if random() < 0.5 { 1.0 } else { -1.0 };
            let cx = rand(w * 0.06, w * 0.94);
            let cy = rand(h * 0.08, h * 0.92);
            let dir = (if random() < 0.5 { 0.0 } else { PI }) + rand(-0.5, 0.5);
            let spread = (size as f64).sqrt();
            for _ in 0..size {
                let x = cx + rand(-1.0, 1.0) * self.sep_x[species] * spread;
                let y = cy + rand(-1.0, 1.0) * self.sep_y[species] * spread * 0.6;
                // Upstream draws a tail-beat phase it never reads; keep its random sequence.
                rand(0.0, TAU);
                let depth = rand(0.55, 1.0);
                self.fish.push(Fish {
                    x,
                    y,
                    vx: dir.cos() * sp.cruise,
                    vy: dir.sin() * sp.cruise,
                    species,
                    alarm: 0.0,
                    alive: true,
                    respawn_at: 0.0,
                    turn: 0.0,
                    depth,
                    mill_dir,
                    bait: false,
                });
            }
        }
        self.bucket_next = vec![0; self.fish.len()];
    }

    pub fn splash(&mut self, x: f64, y: f64) {
        self.events.push(WorldEvent {
            kind: EventKind::Splash,
            x,
            y,
        });
        self.scatter(x, y, 240.0, 260.0, 1.0);
    }

    fn scatter(&mut self, x: f64, y: f64, r: f64, push: f64, alarm: f64) {
        for f in &mut self.fish {
            if !f.alive {
                continue;
            }
            let dx = f.x - x;
            let dy = f.y - y;
            let d = dx.hypot(dy);
            if d > r || d < 1e-3 {
                continue;
            }
            let k = (1.0 - d / r) * push;
            f.vx += (dx / d) * k;
            f.vy += (dy / d) * k;
            f.alarm = f.alarm.max(alarm);
        }
    }

    /// Not in upstream: a raindrop; fish close by flinch.
    pub fn drop(&mut self, x: f64, y: f64) {
        self.events.push(WorldEvent {
            kind: EventKind::Drop,
            x,
            y,
        });
        self.scatter(x, y, 90.0, 110.0, 0.6);
    }

    /// Not in upstream: drops a ball of sardines at (x, y) and makes nearby sharks hungry.
    pub fn feed(&mut self, x: f64, y: f64) {
        const SCHOOL: usize = 24;
        const MAX_BAIT: usize = 120;
        self.events.push(WorldEvent {
            kind: EventKind::Feed,
            x,
            y,
        });
        let sp = &SPECIES[0];
        let mill_dir = if random() < 0.5 { 1.0 } else { -1.0 };
        let spread = (SCHOOL as f64).sqrt();
        for _ in 0..SCHOOL {
            let dir = rand(0.0, TAU);
            let fish = Fish {
                x: x + rand(-1.0, 1.0) * self.sep_x[0] * spread,
                y: y + rand(-1.0, 1.0) * self.sep_y[0] * spread * 0.6,
                vx: dir.cos() * sp.cruise,
                vy: dir.sin() * sp.cruise,
                species: 0,
                alarm: 0.0,
                alive: true,
                respawn_at: 0.0,
                turn: 0.0,
                depth: rand(0.55, 1.0),
                mill_dir,
                bait: true,
            };
            match self.fish.iter().position(|f| f.bait && !f.alive) {
                Some(i) => self.fish[i] = fish,
                None if self.fish.iter().filter(|f| f.bait).count() < MAX_BAIT => {
                    self.fish.push(fish)
                }
                None => break,
            }
        }
        for sh in &mut self.sharks {
            let (dx, dy) = (sh.nodes[0] as f64 - x, sh.nodes[1] as f64 - y);
            if dx.hypot(dy) < sh.length * 1.5 + 300.0 {
                sh.hunger = sh.hunger.max(0.85);
                sh.retarget_at = self.time;
            }
        }
    }

    /// Not in upstream.
    pub fn disturb(&mut self, x: f64, y: f64) {
        let time = self.time;
        self.disturbances.retain(|d| time - d.at < SCARE_SECONDS);
        if self.disturbances.len() >= 16 {
            self.disturbances.remove(0);
        }
        self.disturbances.push(Disturbance { x, y, at: time });
        self.events.push(WorldEvent {
            kind: EventKind::Disturb,
            x,
            y,
        });
    }

    pub fn step(&mut self, dt: f64) {
        self.time += dt;
        self.rebuild_hash();
        self.step_fish(dt);
        self.step_sharks(dt);
        self.respawn();
    }

    fn rebuild_hash(&mut self) {
        let size = self.perception;
        let pad = 2;
        self.bucket_cols = (self.width / size).ceil() as usize + pad * 2;
        self.bucket_rows = (self.height / size).ceil() as usize + pad * 2;
        let n = self.bucket_cols * self.bucket_rows;
        if self.bucket_head.len() != n {
            self.bucket_head = vec![-1; n];
            self.bucket_count = vec![0; n];
        }
        self.bucket_head.fill(-1);
        self.bucket_count.fill(0);
        if self.bucket_next.len() != self.fish.len() {
            self.bucket_next = vec![0; self.fish.len()];
        }
        for i in 0..self.fish.len() {
            let f = &self.fish[i];
            if !f.alive {
                continue;
            }
            let b = self.bucket_of(f.x, f.y);
            self.bucket_next[i] = self.bucket_head[b];
            self.bucket_head[b] = i as i32;
            self.bucket_count[b] += 1;
        }
    }

    fn bucket_cell(&self, x: f64, y: f64) -> (usize, usize) {
        let size = self.perception;
        let c = clamp((x / size).floor() + 2.0, 0.0, (self.bucket_cols - 1) as f64) as usize;
        let r = clamp((y / size).floor() + 2.0, 0.0, (self.bucket_rows - 1) as f64) as usize;
        (c, r)
    }

    fn bucket_of(&self, x: f64, y: f64) -> usize {
        let (c, r) = self.bucket_cell(x, y);
        r * self.bucket_cols + c
    }

    fn step_fish(&mut self, dt: f64) {
        let (w, h, time) = (self.width, self.height, self.time);
        let r2 = self.perception * self.perception;
        let foreign_sep = self.foreign_sep;
        let cols = self.bucket_cols;
        let rows = self.bucket_rows;
        let head = &self.bucket_head;
        let next = &self.bucket_next;
        let calm_decay = (-dt * 0.7).exp();
        let contagion = (dt * 6.0).min(1.0);

        for i in 0..self.fish.len() {
            let mut f = self.fish[i];
            if !f.alive {
                continue;
            }
            let sp = &SPECIES[f.species];
            let spread = 1.0 + 1.8 * f.alarm;
            let sep_x = self.sep_x[f.species] * spread;
            let sep_y = self.sep_y[f.species] * spread;

            let speed = or_one(f.vx.hypot(f.vy));
            let hx = f.vx / speed;
            let hy = f.vy / speed;

            let mut ax = 0.0;
            let mut ay = 0.0;
            let mut sx = 0.0;
            let mut sy = 0.0;
            let mut avx = 0.0;
            let mut avy = 0.0;
            let mut cx = 0.0;
            let mut cy = 0.0;
            let mut n = 0u32;
            let mut neighbour_alarm: f64 = 0.0;

            let (bc, br) = self.bucket_cell(f.x, f.y);
            for r in br.saturating_sub(1)..=(br + 1).min(rows - 1) {
                for c in bc.saturating_sub(1)..=(bc + 1).min(cols - 1) {
                    let mut j = head[r * cols + c];
                    while j != -1 {
                        let ju = j as usize;
                        if ju != i {
                            let o = &self.fish[ju];
                            let dx = o.x - f.x;
                            let dy = o.y - f.y;
                            let d2 = dx * dx + dy * dy;
                            if d2 < r2 && d2 > 1e-6 {
                                let d = d2.sqrt();
                                if (dx * hx + dy * hy) / d > BLIND_COS {
                                    let qx = dx / sep_x;
                                    let qy = dy / sep_y;
                                    let q2 = qx * qx + qy * qy;
                                    if q2 < 1.0 {
                                        let q = q2.sqrt();
                                        let q = if q == 0.0 { 1e-3 } else { q };
                                        let k = (1.0 - q) / q;
                                        sx -= qx * k;
                                        sy -= qy * k;
                                    }
                                    if o.species == f.species {
                                        avx += o.vx;
                                        avy += o.vy;
                                        cx += dx;
                                        cy += dy;
                                        n += 1;
                                    } else if d < foreign_sep {
                                        let k = ((1.0 - d / foreign_sep) / d) * 0.6;
                                        sx -= dx * k;
                                        sy -= dy * k;
                                    }
                                    if o.alarm > neighbour_alarm {
                                        neighbour_alarm = o.alarm;
                                    }
                                }
                            }
                        }
                        j = next[ju];
                    }
                }
            }

            ax += sx * FISH_SEPARATION_K;
            ay += sy * FISH_SEPARATION_K;
            if n > 0 {
                let n = n as f64;
                let align_k = FISH_ALIGN_K * sp.align;
                let cohesion_k = FISH_COHESION_K * sp.cohesion * (1.0 - 0.8 * f.alarm);
                ax += (avx / n - f.vx) * align_k + (cx / n) * cohesion_k;
                ay += (avy / n - f.vy) * align_k + (cy / n) * cohesion_k;
                if sp.mill > 0.0 {
                    let cl = or_one(cx.hypot(cy));
                    ax += (-cy / cl) * f.mill_dir * sp.mill * (1.0 - f.alarm);
                    ay += (cx / cl) * f.mill_dir * sp.mill * (1.0 - f.alarm);
                }
            }

            let mut fear: f64 = 0.0;
            for sh in &self.sharks {
                let fear_r = sh.length * 0.6 + 60.0;
                let mut best = f64::INFINITY;
                let mut bx = 0.0;
                let mut by = 0.0;
                for k in (0..=12).step_by(3) {
                    let nx = sh.nodes[k * 2] as f64;
                    let ny = sh.nodes[k * 2 + 1] as f64;
                    let d2 = (f.x - nx) * (f.x - nx) + (f.y - ny) * (f.y - ny);
                    if d2 < best {
                        best = d2;
                        bx = nx;
                        by = ny;
                    }
                }
                let d = best.sqrt();
                if d >= fear_r || d < 1e-3 {
                    continue;
                }
                let mut wt = 1.0 - d / fear_r;
                wt *= wt;
                let away_x = (f.x - bx) / d;
                let away_y = (f.y - by) / d;
                let shx = sh.heading.cos();
                let shy = sh.heading.sin();
                let side =
                    if shx * (f.y - sh.nodes[1] as f64) - shy * (f.x - sh.nodes[0] as f64) >= 0.0 {
                        1.0
                    } else {
                        -1.0
                    };
                ax += (away_x * 0.7 - shy * side * 0.9) * FISH_FLEE_K * wt;
                ay += (away_y * 0.7 + shx * side * 0.9) * FISH_FLEE_K * wt;
                fear = fear.max(wt.sqrt());
            }

            if self.pointer.active {
                let pr = 130.0;
                let dx = f.x - self.pointer.x;
                let dy = f.y - self.pointer.y;
                let d = dx.hypot(dy);
                if d < pr && d > 1e-3 {
                    let wt = (1.0 - d / pr) * (1.0 - d / pr);
                    ax += (dx / d) * FISH_FLEE_K * wt;
                    ay += (dy / d) * FISH_FLEE_K * wt;
                    fear = fear.max(wt.sqrt() * 0.8);
                }
            }

            let mut alarm = (f.alarm * calm_decay).max(fear);
            let relayed = neighbour_alarm * 0.86;
            if relayed > alarm {
                alarm += (relayed - alarm) * contagion;
            }
            f.alarm = alarm;

            let m = 36.0;
            if f.x < m {
                ax += (m - f.x) * 7.0;
            } else if f.x > w - m {
                ax -= (f.x - (w - m)) * 7.0;
            }
            if f.y < m {
                ay += (m - f.y) * 7.0;
            } else if f.y > h - m {
                ay -= (f.y - (h - m)) * 7.0;
            }

            let (cur_x, cur_y) = current(f.x, f.y, time);
            ax += cur_x * 0.5 + (random() - 0.5) * sp.jitter;
            ay += cur_y * 0.5 + (random() - 0.5) * sp.jitter;
            ay -= f.vy * sp.vertical_drag * (1.0 - 0.7 * f.alarm);

            let preferred = sp.cruise * (1.0 + 1.3 * f.alarm);
            ax += (preferred - speed) * hx * 1.6;
            ay += (preferred - speed) * hy * 1.6;

            let before = f.vy.atan2(f.vx);
            f.vx += ax * dt;
            f.vy += ay * dt;
            let max_speed = sp.max_speed * (1.0 + 1.8 * f.alarm);
            let s = f.vx.hypot(f.vy);
            if s > max_speed {
                f.vx *= max_speed / s;
                f.vy *= max_speed / s;
            } else if s < sp.min_speed && s > 1e-3 {
                f.vx *= sp.min_speed / s;
                f.vy *= sp.min_speed / s;
            }
            f.turn += (angle_diff(before, f.vy.atan2(f.vx)) / dt - f.turn) * (dt * 8.0).min(1.0);
            f.x = clamp(f.x + f.vx * dt, -60.0, w + 60.0);
            f.y = clamp(f.y + f.vy * dt, -60.0, h + 60.0);
            self.fish[i] = f;
        }
    }

    fn step_sharks(&mut self, dt: f64) {
        let (w, h, time) = (self.width, self.height, self.time);
        let min_side = w.min(h);
        let cx_w = w / 2.0;
        let cy_w = h / 2.0;

        for si in 0..self.sharks.len() {
            let mut sh = self.sharks[si];
            let l = sh.length;
            let hx = sh.nodes[0] as f64;
            let hy = sh.nodes[1] as f64;

            sh.hunger = (sh.hunger + dt * 0.03).min(1.0);

            let mut dx = sh.heading.cos();
            let mut dy = sh.heading.sin();

            sh.wander = clamp(sh.wander + (random() - 0.5) * dt * 3.2, -1.1, 1.1);
            dx += (sh.heading + sh.wander).cos() * 0.45;
            dy += (sh.heading + sh.wander).sin() * 0.45;

            let scare = self.flee_clicks(&sh, &mut dx, &mut dy);
            let fleeing = scare > SCARE_FLEE;

            if time >= sh.retarget_at {
                sh.target = self.find_prey(&sh);
                sh.retarget_at = time + 0.45;
            }
            let mut hunting = false;
            let mut closing = 0.0;
            match sh.target.map(|t| self.fish[t]) {
                Some(t) if t.alive && sh.hunger > 0.3 && !fleeing => {
                    let tx = t.x + t.vx * 0.35 - hx;
                    let ty = t.y + t.vy * 0.35 - hy;
                    let d = or_one(tx.hypot(ty));
                    let wt = 1.4 + 2.6 * sh.hunger;
                    dx += (tx / d) * wt;
                    dy += (ty / d) * wt;
                    hunting = true;
                    closing = clamp(1.0 - d / (l * 0.9), 0.0, 1.0);
                }
                _ => {
                    let rx = (hx - cx_w) / (w / min_side);
                    let ry = hy - cy_w;
                    let r = or_one(rx.hypot(ry));
                    let radius =
                        sh.orbit_radius * (1.0 + 0.15 * (time * 0.05 + sh.id as f64).sin());
                    let radial = clamp((radius - r) / radius, -1.0, 1.0);
                    dx += ((-ry / r) * sh.orbit_dir + (rx / r) * radial * 0.9) * 0.55;
                    dy += ((rx / r) * sh.orbit_dir + (ry / r) * radial * 0.9) * 0.55;
                }
            }

            for (oi, other) in self.sharks.iter().enumerate() {
                if oi == si {
                    continue;
                }
                for k in (0..SHARK_NODES).step_by(4) {
                    let ox = hx - other.nodes[k * 2] as f64;
                    let oy = hy - other.nodes[k * 2 + 1] as f64;
                    let d = ox.hypot(oy);
                    let range = (l + other.length) * 0.45;
                    if d < range && d > 1e-3 {
                        let wt = (1.0 - d / range) * 3.0;
                        dx += (ox / d) * wt;
                        dy += (oy / d) * wt;
                    }
                }
            }

            let ex = (hx - cx_w) / (w * 0.3);
            let ey = (hy - cy_w) / (h * 0.16);
            let e2 = ex * ex + ey * ey;
            if e2 < 1.0 {
                let e = e2.sqrt();
                let e = if e == 0.0 { 1e-3 } else { e };
                let wt = (1.0 - e) * (if hunting { 1.0 } else { 2.5 });
                dx += (ex / e) * wt;
                dy += (ey / e) * wt;
            }

            let over = (-hx).max(hx - w).max(-hy).max(hy - h);
            if over > -l * 0.05 {
                let wt = clamp((over + l * 0.05) / (l * 0.35), 0.0, 1.0) * 3.5;
                let to_x = cx_w - hx;
                let to_y = cy_w - hy;
                let d = or_one(to_x.hypot(to_y));
                dx += (to_x / d) * wt;
                dy += (to_y / d) * wt;
            }

            let desired = dy.atan2(dx);
            let agility = if fleeing {
                (1.2 + 1.8 * scare).min(sh.speed / (l * 0.1))
            } else {
                (if hunting { 1.7 + closing } else { 0.8 }).min(sh.speed / (l * 0.22))
            };
            let max_turn = agility * dt;
            let turned = clamp(angle_diff(sh.heading, desired), -max_turn, max_turn);
            sh.heading += turned;
            sh.turn += (turned / dt - sh.turn) * (dt * 3.0).min(1.0);

            let cruise = 30.0 + l * 0.04;
            let target_speed = if hunting {
                cruise * (1.45 + closing * 1.5)
            } else if fleeing {
                cruise * (1.0 + 1.4 * scare)
            } else {
                cruise
            };
            let accel = if fleeing { 2.5 } else { 1.1 };
            sh.speed += (target_speed - sh.speed) * (dt * accel).min(1.0);
            sh.phase += dt * (2.0 + sh.speed / 24.0);

            let nodes = &mut sh.nodes;
            nodes[0] = (nodes[0] as f64 + sh.heading.cos() * sh.speed * dt) as f32;
            nodes[1] = (nodes[1] as f64 + sh.heading.sin() * sh.speed * dt) as f32;

            let seg = l / (SHARK_NODES - 1) as f64;
            let max_bend = 0.16;
            let mut prev_angle = sh.heading + PI;
            for k in 1..SHARK_NODES {
                let px = nodes[(k - 1) * 2] as f64;
                let py = nodes[(k - 1) * 2 + 1] as f64;
                let mut angle = (nodes[k * 2 + 1] as f64 - py).atan2(nodes[k * 2] as f64 - px);
                angle = prev_angle + clamp(angle_diff(prev_angle, angle), -max_bend, max_bend);
                nodes[k * 2] = (px + angle.cos() * seg) as f32;
                nodes[k * 2 + 1] = (py + angle.sin() * seg) as f32;
                prev_angle = angle;
            }

            if sh.hunger > 0.15 {
                let mouth_r = l * 0.05 + 8.0;
                let mx = sh.nodes[0] as f64;
                let my = sh.nodes[1] as f64;
                for f in &mut self.fish {
                    if !f.alive {
                        continue;
                    }
                    if (f.x - mx) * (f.x - mx) + (f.y - my) * (f.y - my) < mouth_r * mouth_r {
                        f.alive = false;
                        f.respawn_at = if f.bait {
                            f64::INFINITY
                        } else {
                            time + rand(3.0, 8.0)
                        };
                        sh.hunger = (sh.hunger - 0.16).max(0.0);
                        self.events.push(WorldEvent {
                            kind: EventKind::Eat,
                            x: f.x,
                            y: f.y,
                        });
                    }
                }
            }
            self.sharks[si] = sh;
        }
    }

    /// Not in upstream. Returns the fright, fading with time and distance.
    fn flee_clicks(&self, sh: &Shark, dx: &mut f64, dy: &mut f64) -> f64 {
        let (hx, hy) = (sh.nodes[0] as f64, sh.nodes[1] as f64);
        let range = sh.length * 0.9 + 220.0;
        let mut scare: f64 = 0.0;
        for d in &self.disturbances {
            let age = self.time - d.at;
            if !(0.0..SCARE_SECONDS).contains(&age) {
                continue;
            }
            let near = (0..SHARK_NODES)
                .step_by(4)
                .map(|k| (sh.nodes[k * 2] as f64 - d.x).hypot(sh.nodes[k * 2 + 1] as f64 - d.y))
                .fold(f64::INFINITY, f64::min);
            if near >= range {
                continue;
            }
            let wt = (1.0 - near / range) * (1.0 - age / SCARE_SECONDS);
            let (ox, oy) = (hx - d.x, hy - d.y);
            let od = ox.hypot(oy).max(1e-3);
            *dx += (ox / od) * wt * SCARE_K;
            *dy += (oy / od) * wt * SCARE_K;
            scare = scare.max(wt);
        }
        scare
    }

    fn find_prey(&self, sh: &Shark) -> Option<usize> {
        let range = sh.length * 0.9 + 160.0;
        let hx = sh.nodes[0] as f64;
        let hy = sh.nodes[1] as f64;
        let fx = sh.heading.cos();
        let fy = sh.heading.sin();
        let mut best = None;
        let mut best_score = f64::INFINITY;
        let mut current_score = f64::INFINITY;

        for (i, f) in self.fish.iter().enumerate() {
            if !f.alive {
                continue;
            }
            let dx = f.x - hx;
            let dy = f.y - hy;
            let d = dx.hypot(dy);
            if d > range || d < 1e-3 {
                continue;
            }
            let cos = (dx * fx + dy * fy) / d;
            if cos < 0.2 {
                continue;
            }
            let crowd = self.bucket_count[self.bucket_of(f.x, f.y)] as f64;
            let score = d * (1.0 + (1.0 - cos) * 0.8) * (1.0 + crowd * 0.07);
            if score < best_score {
                best_score = score;
                best = Some(i);
            }
            if Some(i) == sh.target {
                current_score = score;
            }
        }
        if sh.target.is_some() && current_score < best_score * 1.3 {
            return sh.target;
        }
        best
    }

    fn respawn(&mut self) {
        let (time, w, h) = (self.time, self.width, self.height);
        let fish = &self.fish;
        self.due.clear();
        self.due
            .extend((0..fish.len()).filter(|&i| !fish[i].alive && fish[i].respawn_at <= time));
        if self.due.is_empty() {
            return;
        }
        if self.due.len() < 6 && time - self.last_spawn < 9.0 {
            return;
        }
        self.last_spawn = time;

        let side = rand_int(0, 3);
        let along = rand(0.15, 0.85);
        let x = match side {
            0 => -20.0,
            1 => w + 20.0,
            _ => along * w,
        };
        let y = match side {
            2 => -20.0,
            3 => h + 20.0,
            _ => along * h,
        };
        let inward = (h / 2.0 - y).atan2(w / 2.0 - x) + rand(-0.5, 0.5);
        let species = pick_species();
        let sp = &SPECIES[species];
        let mill_dir = if random() < 0.5 { 1.0 } else { -1.0 };
        for &i in self.due.iter().take(sp.school_size[1] as usize) {
            let f = &mut self.fish[i];
            f.alive = true;
            f.species = species;
            f.alarm = 0.0;
            f.turn = 0.0;
            f.mill_dir = mill_dir;
            f.x = x + rand(-1.0, 1.0) * self.perception;
            f.y = y + rand(-1.0, 1.0) * self.perception;
            f.vx = inward.cos() * sp.cruise * 1.4;
            f.vy = inward.sin() * sp.cruise * 1.4;
        }
    }
}

/// JS `v || 1` for a computed length: zero (or NaN) falls back to 1.
fn or_one(v: f64) -> f64 {
    if v == 0.0 || v.is_nan() { 1.0 } else { v }
}
