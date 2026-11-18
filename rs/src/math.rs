//! Port of upstream src/math.ts, plus the random source standing in for Math.random().

use std::cell::Cell;
use std::time::{SystemTime, UNIX_EPOCH};

pub const PI: f64 = std::f64::consts::PI;
pub const TAU: f64 = std::f64::consts::TAU;

thread_local! {
    static RNG: Cell<u32> = Cell::new(clock_seed());
}

fn clock_seed() -> u32 {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    (t ^ (t >> 32) ^ ((std::process::id() as u64) << 16)) as u32
}

#[cfg(test)]
pub fn seed(s: u32) {
    RNG.with(|r| r.set(s));
}

/// Math.random(), as mulberry32: small, fast, and easy to reproduce in JS for diffing.
pub fn random() -> f64 {
    RNG.with(|r| {
        let a = r.get().wrapping_add(0x6D2B_79F5);
        r.set(a);
        let mut t = (a ^ (a >> 15)).wrapping_mul(1 | a);
        t = t.wrapping_add((t ^ (t >> 7)).wrapping_mul(61 | t)) ^ t;
        (t ^ (t >> 14)) as f64 / 4_294_967_296.0
    })
}

/// JS Math.round: halves round toward +∞ (f64::round rounds them away from zero).
pub fn js_round(x: f64) -> f64 {
    (x + 0.5).floor()
}

/// JS ToInt32 (`x | 0`): truncate, then wrap into 32 bits.
pub fn to_i32(x: f64) -> i32 {
    x as i64 as i32
}

pub fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

pub fn rand(lo: f64, hi: f64) -> f64 {
    lo + random() * (hi - lo)
}

pub fn rand_int(lo: i32, hi: i32) -> i32 {
    rand(lo as f64, hi as f64 + 1.0).floor() as i32
}

pub fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Shortest signed angle from a to b, in (-PI, PI].
pub fn angle_diff(a: f64, b: f64) -> f64 {
    let mut d = (b - a) % TAU;
    if d > PI {
        d -= TAU;
    }
    if d < -PI {
        d += TAU;
    }
    d
}

/// Cheap integer hash → [0, 1). Deterministic, so textures stay stable.
pub fn hash3(x: i32, y: i32, z: i32) -> f64 {
    let mut h = x.wrapping_mul(374_761_393) ^ y.wrapping_mul(668_265_263) ^ z.wrapping_mul(1_442_695_041);
    h = (h ^ ((h as u32) >> 13) as i32).wrapping_mul(1_274_126_177);
    h ^= ((h as u32) >> 16) as i32;
    h as u32 as f64 / 4_294_967_296.0
}

const K1: f64 = 1.0 / 260.0;
const K2: f64 = 1.0 / 310.0;
const K3: f64 = 1.0 / 190.0;
const CURRENT_SCALE: f64 = 3400.0;

/// Divergence-free ocean current: the curl of a slowly drifting stream
/// function ψ(x, y, t), so it moves schools around without bunching them up.
pub fn current(x: f64, y: f64, t: f64) -> (f64, f64) {
    let a = x * K1 + t * 0.07;
    let b = y * K2 - t * 0.05;
    let c = (x + y) * K3 + t * 0.11;
    // u = ∂ψ/∂y, v = -∂ψ/∂x with ψ = sin(a)cos(b) + 0.5 sin(c)
    (
        (-K2 * a.sin() * b.sin() + 0.5 * K3 * c.cos()) * CURRENT_SCALE,
        (-K1 * a.cos() * b.cos() - 0.5 * K3 * c.cos()) * CURRENT_SCALE,
    )
}
