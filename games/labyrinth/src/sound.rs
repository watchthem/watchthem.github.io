//! Labyrinth's procedural sound, on `lib/audio`: footsteps per theme (floor + shoe),
//! the exit, Dungeon's key pickup / key turn / door, and each theme's ambience.
//!
//! Every clip is synthesized once at startup (`Sound::load`). Triggers hang off what the
//! viewer sees happen — a footfall landing, a key turning — via `anim::AnimEvent`, and
//! ambience listens to `Fx`'s lamps (a blinking panel's buzz, the nearest torches'
//! burn). quad-snd can't change a playing clip's volume on WASM, so anything whose
//! loudness depends on where the bot stands is a stream of short one-shots with the
//! volume picked at play time; only the steady beds (office hum, forest wind) loop.
//!
//! All randomness is `audio::Rng`, never `macroquad::rand` — drawing from the gameplay
//! stream would break `HCG_SEED` replay.

use crate::maze::{Cell, EdgeKind, Maze};
use crate::view::critters::Call;
use crate::view::fx::Fx;
use crate::view::theme::ThemeKind;
use audio::filter::{one_pole_highpass, one_pole_lowpass};
use audio::{Clip, Rng, SAMPLE_RATE};
use macroquad::prelude::Vec2;
use std::f32::consts::TAU;

const SR: f32 = SAMPLE_RATE as f32;

fn secs(n: f32) -> usize {
    (n * SR) as usize
}

fn noise(n: usize, rng: &mut Rng) -> Vec<f32> {
    (0..n).map(|_| rng.range(-1.0, 1.0)).collect()
}

fn band(buf: &mut [f32], lo: f32, hi: f32) {
    one_pole_highpass(buf, lo, SAMPLE_RATE);
    one_pole_lowpass(buf, hi, SAMPLE_RATE);
}

/// Adds `src * gain` into `dst` starting at `at` seconds, growing `dst` as needed.
fn add_at(dst: &mut Vec<f32>, src: &[f32], at: f32, gain: f32) {
    let o = secs(at);
    if dst.len() < o + src.len() {
        dst.resize(o + src.len(), 0.0);
    }
    for (d, s) in dst[o..].iter_mut().zip(src) {
        *d += s * gain;
    }
}

/// Noise shaped by a fast-attack exponential decay (`tau` seconds), band-limited.
fn burst(len: f32, tau: f32, lo: f32, hi: f32, rng: &mut Rng) -> Vec<f32> {
    let mut b = noise(secs(len), rng);
    for (i, s) in b.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *s *= (t / 0.002).min(1.0) * (-t / tau).exp();
    }
    band(&mut b, lo, hi);
    b
}

/// Sine sweeping `f0 -> f1` Hz, exponential decay `tau`.
fn thump(len: f32, f0: f32, f1: f32, tau: f32) -> Vec<f32> {
    let n = secs(len);
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let f = f0 + (f1 - f0) * (t / len);
            ph += f / SR;
            (ph * TAU).sin() * (t / 0.003).min(1.0) * (-t / tau).exp()
        })
        .collect()
}

/// A decaying sine partial — bells, key rings, chimes.
fn ring(len: f32, f: f32, tau: f32) -> Vec<f32> {
    (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            (t * f * TAU).sin() * (t / 0.002).min(1.0) * (-t / tau).exp()
        })
        .collect()
}

/// Two quiet reflections, low-passed — the "hard walls around you" of a stone corridor.
fn echo(buf: &mut Vec<f32>, taps: &[(f32, f32)], cutoff: f32) {
    let dry = buf.clone();
    let mut wet = dry.clone();
    one_pole_lowpass(&mut wet, cutoff, SAMPLE_RATE);
    for &(at, g) in taps {
        add_at(buf, &wet, at, g);
    }
}

/// Scale to exactly `peak`: every clip leaves here equally loud, so play volumes are
/// the only loudness knob.
fn finish(mut b: Vec<f32>, peak: f32) -> Vec<f32> {
    let p = b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if p > 0.0 {
        b.iter_mut().for_each(|s| *s *= peak / p);
    }
    b
}

/// Scale to unit RMS, so layers can be mixed by gain rather than by guesswork.
fn unit_rms(mut b: Vec<f32>) -> Vec<f32> {
    let r = (b.iter().map(|s| s * s).sum::<f32>() / b.len().max(1) as f32).sqrt();
    if r > 0.0 {
        b.iter_mut().for_each(|s| *s /= r);
    }
    b
}

/// RBJ band-pass biquad (0 dB at the centre). `set` retunes it without resetting state,
/// so it can sweep sample by sample.
#[derive(Default)]
struct Bandpass {
    b0: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Bandpass {
    fn new(f: f32, q: f32) -> Self {
        let mut b = Self::default();
        b.set(f, q);
        b
    }

    fn set(&mut self, f: f32, q: f32) {
        let w = TAU * f / SR;
        let alpha = w.sin() / (2.0 * q);
        let a0 = 1.0 + alpha;
        self.b0 = alpha / a0;
        self.a1 = -2.0 * w.cos() / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    fn tick(&mut self, x: f32) -> f32 {
        let y = self.b0 * (x - self.x2) - self.a1 * self.y1 - self.a2 * self.y2;
        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

/// A struck mode of a solid: `exc` rung through a two-pole resonator at `f` Hz that
/// dies away in `tau` seconds, scaled to peak `gain`.
fn mode(exc: &[f32], f: f32, tau: f32, gain: f32) -> Vec<f32> {
    let r = (-1.0 / (tau * SR)).exp();
    let c = 2.0 * r * (TAU * f / SR).cos();
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    let out: Vec<f32> = (0..exc.len() + secs(tau * 5.0))
        .map(|i| {
            let y = exc.get(i).copied().unwrap_or(0.0) + c * y1 - r * r * y2;
            y2 = y1;
            y1 = y;
            y
        })
        .collect();
    finish(out, gain)
}

/// Smooth random drift in 0..1: a fresh target every ~`step` seconds, cosine-eased in
/// between — the unsteady breath of a flame, a leaf's flutter.
fn drift(n: usize, step: f32, rng: &mut Rng) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    let (mut a, mut b) = (rng.next_f32(), rng.next_f32());
    while out.len() < n {
        let len = secs(step * rng.range(0.6, 1.4)).max(1);
        for i in 0..len {
            let k = 0.5 - 0.5 * (i as f32 / len as f32 * std::f32::consts::PI).cos();
            out.push(a + (b - a) * k);
        }
        a = b;
        b = rng.next_f32();
    }
    out.truncate(n);
    out
}

/// A loop-periodic curve in 0..1 built from several harmonics of `1/len` with random
/// phases and falling weights — repeats every `len` seconds (so a bed stays seamless)
/// but has no single swell period the ear can lock onto.
fn gusts(n: usize, len: f32, harmonics: &[f32], rng: &mut Rng) -> Vec<f32> {
    let parts: Vec<(f32, f32, f32)> = harmonics
        .iter()
        .map(|&k| (k, rng.range(0.0, TAU), rng.range(0.6, 1.0) / k.sqrt()))
        .collect();
    let raw: Vec<f32> = (0..n)
        .map(|i| {
            let w = TAU * i as f32 / SR / len;
            parts.iter().map(|(k, ph, a)| a * (k * w + ph).sin()).sum()
        })
        .collect();
    let period = secs(len).min(n);
    let lo = raw[..period].iter().fold(f32::MAX, |m, &s| m.min(s));
    let hi = raw[..period].iter().fold(f32::MIN, |m, &s| m.max(s));
    raw.iter().map(|s| (s - lo) / (hi - lo)).collect()
}

/// A small stone room: four damped feedback combs into two all-passes (Schroeder),
/// decaying over `rt60` seconds, `wet` mixed on top of the dry signal.
fn room(buf: &mut Vec<f32>, rt60: f32, damp_hz: f32, wet: f32) {
    let dry_len = buf.len();
    buf.resize(dry_len + secs(rt60), 0.0);
    let damp = 1.0 - (-TAU * damp_hz / SR).exp();
    let mut out = vec![0.0f32; buf.len()];
    for d in [0.0297f32, 0.0371, 0.0411, 0.0437] {
        let g = 10f32.powf(-3.0 * d / rt60);
        let mut line = vec![0.0f32; secs(d)];
        let mut lp = 0.0f32;
        for (i, o) in out.iter_mut().enumerate() {
            let j = i % line.len();
            let y = line[j];
            lp += damp * (y - lp);
            line[j] = buf[i] + lp * g;
            *o += y * 0.25;
        }
    }
    for d in [0.005f32, 0.0017] {
        let mut line = vec![0.0f32; secs(d)];
        for (i, o) in out.iter_mut().enumerate() {
            let j = i % line.len();
            let v = line[j];
            let x = *o;
            line[j] = x + v * 0.7;
            *o = v - x * 0.7;
        }
    }
    for (b, o) in buf.iter_mut().zip(&out) {
        *b += o * wet;
    }
}

// ── footsteps: floor x shoe ──────────────────────────────────────────────────
//
// Built like a real footfall: almost nothing in it is pitched. A heel contact and then
// a toe contact, each a very short broadband strike that rings the *floor's* own modes
// for a few ms, plus whatever the floor adds (grit, fibre, leaves) and the room. The
// floor picks the strike's band, its modes, the grit/scuff and the room.
//
// History, all heard by ear: 1.5-6kHz click grains read as Minecraft; a swept-sine knock
// read as a kick drum (96-99% of energy < 200 Hz); low damped "slab" modes (170-520 Hz,
// 9-14 ms) plus a low noise thud read as hitting a drum, not stone (stone: 69% of energy
// < 400 Hz, centroid ~430 Hz). A drum is a decaying ring in 80-400 Hz or a thump; a real
// hard-floor step is the opposite — a < 60 ms broadband transient, 0.5-6 kHz.

/// What a boot landing on a floor sounds like.
struct Floor {
    /// Band of each contact's strike (12 dB/oct both sides) and how fast it dies.
    heel: (f32, f32),
    heel_tau: f32,
    /// The floor's own modes, rung by each contact: (Hz, decay s, level). Each contact
    /// detunes them ±15%, so heel and toe never ring the same pitch twice.
    modes: &'static [(f32, f32, f32)],
    /// Unpitched low weight of the body landing (low-passed noise, < 260 Hz).
    weight: f32,
    /// The toe contact is a flat sole, not a heel edge: it's low-passed here.
    toe_dull: f32,
    /// Sand/grit crunched under each contact: sparse tiny ticks in (lo, hi) Hz, level.
    grit: (f32, f32, f32),
    /// Sole-scuff friction band and level.
    scuff: (f32, f32, f32),
    /// Everything above this is rolled off (carpet swallows the top end).
    cutoff: f32,
    /// Onset ramp of the whole step: a soft sole lands over a few ms, a hard heel doesn't.
    attack: f32,
}

/// `band` at 12 dB/oct — a one-pole's 6 dB/oct leaks too much low end under a strike.
fn band2(buf: &mut [f32], lo: f32, hi: f32) {
    band(buf, lo, hi);
    band(buf, lo, hi);
}

/// Sparse tiny ticks over band noise, dying in `tau` — grains of grit crushed or shifting
/// under a sole. `density` is ticks per ms, roughly.
fn grit(len: f32, tau: f32, density: f32, lo: f32, hi: f32, rng: &mut Rng) -> Vec<f32> {
    let n = secs(len);
    let mut g = noise(n, rng);
    band2(&mut g, lo, hi);
    let p = density / (SR / 1000.0);
    let mut ticks: Vec<f32> = (0..n)
        .map(|_| {
            if rng.next_f32() < p {
                rng.range(0.3, 1.0)
            } else {
                0.0
            }
        })
        .collect();
    one_pole_lowpass(&mut ticks, 400.0, SAMPLE_RATE);
    for (i, (s, k)) in g.iter_mut().zip(&ticks).enumerate() {
        *s *= k * (-(i as f32 / SR) / tau).exp();
    }
    g
}

/// One boot landing on `floor`: heel contact, toe contact a roll later, grit and scuff.
/// Every call is a slightly different step.
fn boot(rng: &mut Rng, floor: &Floor) -> Vec<f32> {
    let mut b = vec![0.0; secs(0.24)];
    // Heel and toe land nearly as hard as each other, a roll apart: two contacts spread
    // over ~60-110ms read as a step; one dominant hit reads as a drum.
    let toe = rng.range(0.06, 0.11);
    for (k, (at, g)) in [(0.0, 0.85), (toe, rng.range(0.55, 0.8))]
        .into_iter()
        .enumerate()
    {
        let mut hit = vec![0.0; secs(0.05)];
        let tau = floor.heel_tau * rng.range(0.8, 1.25);
        let mut strike = burst(0.04, tau, 20.0, 20000.0, rng);
        band2(&mut strike, floor.heel.0, floor.heel.1);
        add_at(&mut hit, &finish(strike, 1.0), 0.0, 1.0);
        let exc = burst(0.004, 0.0008, 60.0, 12000.0, rng);
        for &(f, t, lvl) in floor.modes {
            let m = mode(
                &exc,
                f * rng.range(0.85, 1.15),
                t * rng.range(0.8, 1.2),
                lvl * rng.range(0.6, 1.1),
            );
            add_at(&mut hit, &m, 0.0, 1.0);
        }
        if floor.weight > 0.0 {
            let mut thud = burst(0.05, 0.01, 60.0, 500.0, rng);
            one_pole_lowpass(&mut thud, 260.0, SAMPLE_RATE);
            add_at(&mut hit, &finish(thud, floor.weight), 0.0, 1.0);
        }
        if k == 1 {
            one_pole_lowpass(&mut hit, floor.toe_dull, SAMPLE_RATE);
        }
        add_at(&mut b, &finish(hit, 1.0), at, g);
        let (lo, hi, lvl) = floor.grit;
        if lvl > 0.0 {
            let gr = grit(0.05, 0.015, rng.range(1.5, 3.0), lo, hi, rng);
            add_at(
                &mut b,
                &finish(gr, lvl * rng.range(0.6, 1.2)),
                at + 0.001,
                g,
            );
        }
    }
    // Scuff: friction noise swelling in as the sole rolls forward, then gone — the
    // "shh" under the contacts that makes it a step rather than a hit.
    let (lo, hi, lvl) = floor.scuff;
    let mut scuff = noise(secs(0.14), rng);
    band(&mut scuff, lo, hi);
    let len = scuff.len() as f32;
    for (i, s) in scuff.iter_mut().enumerate() {
        *s *= (std::f32::consts::PI * i as f32 / len).sin().powi(2);
    }
    add_at(
        &mut b,
        &finish(scuff, lvl * rng.range(0.6, 1.2)),
        0.008,
        1.0,
    );
    one_pole_lowpass(&mut b, floor.cutoff, SAMPLE_RATE);
    let ramp = secs(floor.attack).max(1) as f32;
    for (i, s) in b.iter_mut().enumerate().take(ramp as usize) {
        *s *= i as f32 / ramp;
    }
    b
}

/// Backrooms: boots on damp carpet over a hollow subfloor — a soft, dull pat, a brush
/// of fibre, no room. (Its modes are low-Q and gone in ~4 ms: the 150/340 Hz, 10-12 ms
/// modes before rang like a muffled tom.)
fn step_carpet(rng: &mut Rng) -> Vec<f32> {
    const CARPET: Floor = Floor {
        heel: (250.0, 1800.0),
        heel_tau: 0.006,
        modes: &[
            (320.0, 0.004, 0.12),
            (700.0, 0.004, 0.25),
            (1200.0, 0.003, 0.2),
        ],
        weight: 0.05,
        toe_dull: 1200.0,
        grit: (0.0, 0.0, 0.0),
        scuff: (500.0, 2400.0, 0.3),
        cutoff: 1700.0,
        attack: 0.005,
    };
    finish(boot(rng, &CARPET), 0.9)
}

/// One leather sole meeting stone: filtered noise under a rounded 2-4 ms rise (leather
/// and heel rubber give — nothing lands in a single sample), a dull 200-800 Hz body and
/// a short 1-3 kHz slap. The envelope goes on *after* filtering and the filters are
/// one-poles, so nothing rings: no modal pitch at all. `body`/`slap` are the two levels.
fn sole_contact(rng: &mut Rng, body: f32, slap: f32) -> Vec<f32> {
    let n = secs(0.06);
    let rise = rng.range(0.002, 0.004);
    let env = |i: usize, tau: f32| {
        let t = i as f32 / SR;
        let up = if t < rise {
            0.5 - 0.5 * (std::f32::consts::PI * t / rise).cos()
        } else {
            1.0
        };
        up * (-(t - rise).max(0.0) / tau).exp()
    };
    let (tb, ts) = (rng.range(0.007, 0.011), rng.range(0.0025, 0.004));
    let mut lo = noise(n, rng);
    band2(&mut lo, 100.0, 500.0);
    let mut hi = noise(n, rng);
    band2(&mut hi, 500.0, 1300.0);
    let lo: Vec<f32> = lo.iter().enumerate().map(|(i, s)| s * env(i, tb)).collect();
    let hi: Vec<f32> = hi.iter().enumerate().map(|(i, s)| s * env(i, ts)).collect();
    let (lo, hi) = (finish(lo, body), finish(hi, slap));
    lo.iter().zip(&hi).map(|(a, b)| a + b).collect()
}

/// Dungeon: leather boots on stone flagstones in a long stone corridor. Not `boot`:
/// the floor has no modes here at all. Heel then toe (a flatter, slappier contact a roll
/// later), a few grains of grit, a faint scuff — and the *corridor* is what makes it
/// stone: spaced early reflections 20-120 ms out off the walls and vault, then a darker,
/// longer tail at low level.
///
/// History: low damped slab modes + noise thud read as hitting a drum (69% < 400 Hz);
/// the first version's 1.5/3.1 kHz ring read as tapping; six detuned 0.4-5 kHz modes
/// (τ 2.5-6 ms) plus 6-24 ms reflections read as a hammer on an iron pan with a too-fast
/// echo. Even short discrete modes sound like struck metal — keep this noise-only.
fn step_stone(rng: &mut Rng) -> Vec<f32> {
    let mut b = vec![0.0; secs(0.24)];
    let toe = rng.range(0.07, 0.11);
    for (at, g, body, slap) in [
        (0.0, 1.0, 1.0, 0.25),
        (toe, rng.range(0.6, 0.8), 0.75, 0.35),
    ] {
        add_at(&mut b, &sole_contact(rng, body, slap), at, g);
        let gr = grit(0.05, 0.015, rng.range(1.0, 2.0), 900.0, 2500.0, rng);
        add_at(
            &mut b,
            &finish(gr, 0.025 * rng.range(0.6, 1.2)),
            at + 0.002,
            g,
        );
    }
    let mut scuff = noise(secs(0.12), rng);
    band(&mut scuff, 400.0, 1500.0);
    let len = scuff.len() as f32;
    for (i, s) in scuff.iter_mut().enumerate() {
        *s *= (std::f32::consts::PI * i as f32 / len).sin().powi(2);
    }
    add_at(
        &mut b,
        &finish(scuff, 0.05 * rng.range(0.6, 1.2)),
        0.01,
        1.0,
    );
    one_pole_lowpass(&mut b, 2000.0, SAMPLE_RATE);
    // The corridor: walls and vault a few metres off, each answer darker and later.
    let j = rng.range(0.9, 1.1);
    echo(
        &mut b,
        &[
            (0.023 * j, 0.2),
            (0.049 * j, 0.14),
            (0.084 * j, 0.09),
            (0.123 * j, 0.05),
        ],
        // Stone walls give back the body of the step, not its click: dark reflections.
        800.0,
    );
    room(&mut b, 0.8, 900.0, 0.06);
    one_pole_highpass(&mut b, 100.0, SAMPLE_RATE);
    finish(b, 0.9)
}

/// Forest: boots on soft earth — a damped, earthy pat with a light, smooth rustle of
/// leaf litter pushed aside (a brush, not a crunch).
fn step_leaves(rng: &mut Rng) -> Vec<f32> {
    const EARTH: Floor = Floor {
        heel: (180.0, 1800.0),
        heel_tau: 0.005,
        modes: &[(300.0, 0.005, 0.15), (620.0, 0.004, 0.2)],
        weight: 0.08,
        toe_dull: 1500.0,
        grit: (0.0, 0.0, 0.0),
        scuff: (600.0, 2200.0, 0.2),
        cutoff: 2600.0,
        attack: 0.004,
    };
    let mut b = boot(rng, &EARTH);
    let n = secs(0.16);
    let mut rustle = noise(n, rng);
    band(&mut rustle, 550.0, 2300.0);
    let flutter = drift(n, 0.012, rng);
    for (i, (s, f)) in rustle.iter_mut().zip(&flutter).enumerate() {
        let t = i as f32 / SR;
        *s *= (t / 0.012).min(1.0) * (-t / 0.045).exp() * (0.5 + 0.5 * f);
    }
    add_at(&mut b, &finish(rustle, 0.14), 0.004, 1.0);
    finish(b, 0.9)
}

/// Garden: boots on a gravel path — a firm pat and a fine, quiet grit of stones
/// shifting under the sole.
fn step_gravel(rng: &mut Rng) -> Vec<f32> {
    const GRAVEL: Floor = Floor {
        heel: (260.0, 2800.0),
        heel_tau: 0.004,
        modes: &[(380.0, 0.005, 0.25), (800.0, 0.004, 0.2)],
        weight: 0.07,
        toe_dull: 1800.0,
        grit: (0.0, 0.0, 0.0),
        scuff: (800.0, 3200.0, 0.18),
        cutoff: 4500.0,
        attack: 0.003,
    };
    let mut b = boot(rng, &GRAVEL);
    // Stones shifting as the weight settles: denser and longer than a contact's grit.
    add_at(
        &mut b,
        &finish(grit(0.13, 0.05, 0.18, 1100.0, 3800.0, rng), 0.16),
        0.003,
        1.0,
    );
    finish(b, 0.9)
}

// ── events ───────────────────────────────────────────────────────────────────

/// Reaching the exit: a soft rising A-major arpeggio with an airy swell. Rare (once per
/// maze), so tonal is fine.
fn exit_chime(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    for (i, st) in [0.0f32, 4.0, 7.0, 12.0, 16.0].iter().enumerate() {
        let f = 440.0 * 2f32.powf(st / 12.0);
        let mut n = ring(1.4, f, 0.5);
        add_at(&mut n, &ring(1.0, f * 2.0, 0.25), 0.0, 0.18);
        add_at(&mut b, &n, i as f32 * 0.09, 1.0);
    }
    let mut air = noise(secs(1.3), rng);
    for (i, s) in air.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *s *= (t / 0.5).min(1.0) * (-(t - 0.5).max(0.0) / 0.25).exp();
    }
    band(&mut air, 1500.0, 5000.0);
    add_at(&mut b, &air, 0.0, 0.5);
    one_pole_lowpass(&mut b, 4500.0, SAMPLE_RATE);
    finish(b, 0.8)
}

/// Picking up a key: the ring of a small iron key on its ring — two bright inharmonic
/// strikes.
fn key_pickup() -> Vec<f32> {
    let mut b = Vec::new();
    for (at, f) in [(0.0, 2350.0f32), (0.075, 3100.0)] {
        let mut n = ring(0.5, f, 0.12);
        add_at(&mut n, &ring(0.4, f * 2.76, 0.05), 0.0, 0.35);
        add_at(&mut b, &n, at, 1.0);
    }
    finish(b, 0.8)
}

/// Turning a key in an old lock: insert scrape, a ratchet of wards, the bolt's clack.
fn key_turn(rng: &mut Rng) -> Vec<f32> {
    let mut b = burst(0.08, 0.03, 2500.0, 8000.0, rng);
    for i in 0..4 {
        let at = 0.12 + i as f32 * 0.045;
        add_at(&mut b, &burst(0.02, 0.003, 1500.0, 6000.0, rng), at, 0.5);
        add_at(
            &mut b,
            &ring(0.04, 1900.0 + i as f32 * 90.0, 0.008),
            at,
            0.25,
        );
    }
    add_at(&mut b, &burst(0.05, 0.006, 800.0, 5000.0, rng), 0.36, 1.0);
    add_at(&mut b, &thump(0.08, 260.0, 180.0, 0.02), 0.36, 0.8);
    echo(&mut b, &[(0.09, 0.2)], 1500.0);
    finish(b, 0.85)
}

/// A dungeon gate swinging open: iron bars, not planks. A rusty hinge squealing —
/// stick-slip bursts of a pitched grind (400–700 Hz fundamental, bright harmonics,
/// ringing the hinge's metal modes) — the bars rattling in their frame, then the gate
/// clanging against its stop: a struck iron bar (free-bar partial ratios, ringing
/// for most of a second) in the stone corridor. (It was a wooden door's creak and thud.)
/// Played on the key turn: the swing starts `GATE_LEAD` in and lasts `GATE_SWING`,
/// matching the view's `Shown::unlocking` swing, so the clang lands as the gate stops.
fn door_open(rng: &mut Rng) -> Vec<f32> {
    let len = GATE_SWING;
    let n = secs(len);
    let mut squeal = vec![0.0; n];
    let mut ph = 0.0f32;
    let grip = drift(n, 0.05, rng);
    let f0 = rng.range(420.0, 520.0);
    for (i, s) in squeal.iter_mut().enumerate() {
        let t = i as f32 / SR;
        let k = t / len;
        // The pitch wanders up as the hinge frees, with a stick-slip wobble.
        let f = f0 * (1.0 + 0.35 * k + 0.08 * (t * 23.0).sin() + 0.1 * grip[i]);
        ph += f / SR;
        let saw = 2.0 * ph.fract() - 1.0;
        // Sticks and slips: bursts, not a steady tone.
        let bursty = (grip[i] * 1.6 - 0.3).clamp(0.0, 1.0);
        let env = (k / 0.06).min(1.0) * ((1.0 - k) / 0.15).min(1.0);
        *s = saw * bursty * env;
    }
    band(&mut squeal, 700.0, 5000.0);
    let mut b = squeal.clone();
    for (f, tau) in [(1180.0, 0.05), (2390.0, 0.035), (3710.0, 0.02)] {
        add_at(&mut b, &mode(&squeal, f, tau, 0.6), 0.0, 0.5);
    }
    let mut b = finish(b, 0.8);
    // The bars rattling in the frame as it swings.
    for _ in 0..6 {
        let f = rng.range(900.0, 1600.0);
        let c = clink(rng, f);
        let at = rng.range(0.05, len - 0.1);
        add_at(&mut b, &c, at, rng.range(0.1, 0.25));
    }
    // The clang against the stop.
    let hit = len - 0.01;
    let base = rng.range(190.0, 230.0);
    add_at(&mut b, &burst(0.06, 0.01, 300.0, 6000.0, rng), hit, 0.8);
    for (ratio, tau, g) in [
        (1.0, 0.7, 0.9),
        (2.76, 0.5, 0.7),
        (5.40, 0.3, 0.45),
        (8.93, 0.18, 0.3),
        (13.3, 0.1, 0.2),
    ] {
        let f = base * ratio * rng.range(0.99, 1.01);
        add_at(&mut b, &ring(1.2, f, tau), hit, g);
    }
    // The other bars answering, a beat later.
    for _ in 0..4 {
        let f = rng.range(700.0, 1300.0);
        let c = clink(rng, f);
        add_at(
            &mut b,
            &c,
            hit + rng.range(0.02, 0.12),
            rng.range(0.15, 0.3),
        );
    }
    room(&mut b, 0.9, 2500.0, 0.25);
    one_pole_highpass(&mut b, 80.0, SAMPLE_RATE);
    // A one-shot stays under 2 s: let the clang ring ~1.2 s, then fade.
    b.truncate(secs(len + 1.2));
    fade_out(&mut b, 0.35);
    let mut out = vec![0.0; secs(GATE_LEAD)];
    out.extend(b);
    finish(out, 0.85)
}

/// A dungeon gate's swing, as the view plays it (`lib.rs` `door_swing`): the key turns
/// for `GATE_LEAD`, then the gate swings over the rest of `anim::UNLOCK_SECS`.
pub const GATE_LEAD: f32 = 0.15;
const GATE_SWING: f32 = crate::anim::UNLOCK_SECS - GATE_LEAD;

/// An office door: the handle's latch clunking back (a low, damped knock, not a
/// click), the air of a hollow door swinging, a faint low hinge groan. (The first
/// version rang at 1.35 kHz with a 0.7-2.2 kHz creak — it read as a squeak.)
fn office_door(rng: &mut Rng) -> Vec<f32> {
    let mut b = burst(0.05, 0.008, 250.0, 2000.0, rng);
    add_at(&mut b, &burst(0.04, 0.006, 200.0, 1500.0, rng), 0.06, 0.7);
    // The door's own hollow body, knocked by the latch.
    add_at(&mut b, &ring(0.08, 190.0, 0.02), 0.0, 0.35);
    // Swing: a soft swell of low air.
    let n = secs(0.45);
    let mut air = noise(n, rng);
    band(&mut air, 90.0, 500.0);
    for (i, s) in air.iter_mut().enumerate() {
        let k = i as f32 / n as f32;
        *s *= (k * std::f32::consts::PI).sin().powi(2);
    }
    add_at(&mut b, &air, 0.08, 0.5);
    // A faint slow hinge groan, low.
    let len = 0.35;
    let mut groan = vec![0.0; secs(len)];
    let mut ph = 0.0f32;
    for (i, s) in groan.iter_mut().enumerate() {
        let k = i as f32 / SR / len;
        ph += (35.0 + 20.0 * k) / SR;
        let pulse = if ph.fract() < 0.08 { 1.0 } else { 0.0 };
        *s = pulse * (k / 0.2).min(1.0) * ((1.0 - k) / 0.4).min(1.0);
    }
    band(&mut groan, 180.0, 700.0);
    band(&mut groan, 180.0, 700.0);
    add_at(&mut b, &groan, 0.12, 0.25);
    one_pole_lowpass(&mut b, 3000.0, SAMPLE_RATE);
    finish(b, 0.8)
}

/// Fades the last `len` seconds out to silence, so a ringing tail never ends in a click.
fn fade_out(buf: &mut [f32], len: f32) {
    let n = secs(len).min(buf.len());
    let start = buf.len() - n;
    for (i, s) in buf[start..].iter_mut().enumerate() {
        *s *= 1.0 - i as f32 / n as f32;
    }
}

/// Piecewise-linear curve through `(t, value)` keys, held flat past both ends.
fn curve(keys: &[(f32, f32)], t: f32) -> f32 {
    let i = keys.partition_point(|k| k.0 <= t);
    if i == 0 {
        return keys[0].1;
    }
    if i == keys.len() {
        return keys[i - 1].1;
    }
    let ((t0, v0), (t1, v1)) = (keys[i - 1], keys[i]);
    v0 + (v1 - v0) * (t - t0) / (t1 - t0)
}

/// A plucked steel string heard through a single-coil pickup near the bridge: a stack of
/// harmonics, each weighted by where the pick struck (`|sin(πk·0.1)|/k^0.6`) and where the
/// pickup sits (`|sin(πk·0.09)|` — the thin, twangy part), upper ones dying first; plus
/// the pick's own scrape. `bend(t)` multiplies the pitch (slides, vibrato). The fretting
/// hand damps it over the last `damp` seconds.
fn pluck(len: f32, f: f32, bend: &dyn Fn(f32) -> f32, tau: f32, damp: f32) -> Vec<f32> {
    use std::f32::consts::PI;
    let harmonics = ((4500.0 / f) as usize).clamp(4, 20);
    let parts: Vec<(f32, f32)> = (1..=harmonics)
        .map(|k| {
            let k = k as f32;
            let amp = (PI * k * 0.1).sin().abs() / k.powf(0.6) * (PI * k * 0.09).sin().abs();
            (amp, tau / (1.0 + 0.15 * (k - 1.0)))
        })
        .collect();
    let mut ph = 0.0f32;
    (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            ph = (ph + f * bend(t) / SR).fract();
            let x: f32 = parts
                .iter()
                .enumerate()
                .map(|(k, (a, tk))| a * (-t / tk).exp() * (ph * (k + 1) as f32 * TAU).sin())
                .sum();
            let off = ((len - t) / damp).clamp(0.0, 1.0);
            x * (t / 0.002).min(1.0) * off
        })
        .collect()
}

/// A picked bass string: round low harmonics, a thumb-soft attack, a short ring.
fn bass_note(len: f32, f: f32) -> Vec<f32> {
    (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            let x: f32 = [1.0f32, 0.5, 0.3, 0.12]
                .iter()
                .enumerate()
                .map(|(k, a)| a * (t * f * (k + 1) as f32 * TAU).sin())
                .sum();
            x * (t / 0.006).min(1.0) * (-t / 0.35).exp() * ((len - t) / 0.04).clamp(0.0, 1.0)
        })
        .collect()
}

/// The janitor's closet: the seventies-soundtrack funk cliché — "bow, chicka, wow, wow".
/// At 100 BPM (a 16th = 0.15 s): a wah'd double-stop on the beat ("bow"), two muted
/// 16th-note scratches ("chick-a"), then two wah'd chords, the second held with vibrato
/// and sagging flat ("wow ... wowww"). A clean single-coil guitar through a real wah —
/// a resonant band-pass (Q 5) the foot rocks between 400 Hz (heel, closed: "oo") and
/// 2 kHz (toe, open: "ah"), closed at each attack, opened, closed again: the "w-ah-oo"
/// of every "wow". A sparse bass and closed hi-hat underneath.
fn closet_sting(rng: &mut Rng) -> Vec<f32> {
    let st = |n: f32| 2f32.powf(n / 12.0);
    let (e3, g3) = (164.8f32, 196.0f32);
    let flat = |_: f32| 1.0;
    let mut gtr = vec![0.0; secs(1.85)];
    // Double-stops: root + fifth for "bow", minor third + seventh for the "wow"s.
    for (at, len, f, g) in [(0.0, 0.27, e3, 1.0), (0.0, 0.27, e3 * st(7.0), 0.7)] {
        add_at(&mut gtr, &pluck(len, f, &flat, 0.5, 0.04), at, g);
    }
    // Chick-a: all strings muted by the fretting hand — a dead thud of pitch under a
    // bright pick scrape.
    for at in [0.3, 0.45] {
        for f in [e3, e3 * st(7.0), e3 * 2.0] {
            add_at(&mut gtr, &pluck(0.07, f, &flat, 0.015, 0.02), at, 1.0);
        }
        add_at(&mut gtr, &burst(0.05, 0.01, 800.0, 5000.0, rng), at, 1.0);
    }
    add_at(&mut gtr, &pluck(0.28, g3, &flat, 0.6, 0.04), 0.6, 1.0);
    add_at(
        &mut gtr,
        &pluck(0.28, g3 * st(7.0), &flat, 0.6, 0.04),
        0.6,
        0.7,
    );
    // The last "wow": held, finger vibrato after a moment, sagging a semitone at the end.
    let wow = |t: f32| {
        let vib = 1.0 + 0.012 * ((t - 0.2) / 0.15).clamp(0.0, 1.0) * (t * 5.5 * TAU).sin();
        vib * 2f32.powf(-((t - 0.55) / 0.35).clamp(0.0, 1.0) / 12.0)
    };
    add_at(
        &mut gtr,
        &pluck(0.95, e3 * st(3.0), &wow, 1.2, 0.15),
        0.9,
        1.0,
    );
    add_at(
        &mut gtr,
        &pluck(0.95, e3 * st(10.0), &wow, 1.2, 0.15),
        0.9,
        0.7,
    );
    for &at in &[0.0f32, 0.3, 0.45, 0.6, 0.9] {
        add_at(&mut gtr, &burst(0.012, 0.002, 800.0, 5000.0, rng), at, 0.12);
    }

    // The pedal, 0 = heel (closed) .. 1 = toe (open), rocked with the rhythm.
    const PEDAL: [(f32, f32); 16] = [
        (0.0, 0.0),
        (0.1, 1.0),
        (0.27, 0.15),
        (0.3, 0.55),
        (0.4, 0.3),
        (0.45, 0.6),
        (0.56, 0.1),
        (0.6, 0.0),
        (0.72, 1.0),
        (0.88, 0.1),
        (0.9, 0.0),
        (1.06, 1.0),
        (1.2, 0.95),
        (1.45, 0.6),
        (1.7, 0.05),
        (1.85, 0.0),
    ];
    let mut wah = Bandpass::new(400.0, 5.0);
    let mut open = 1.0;
    let mut b = unit_rms(
        gtr.iter()
            .enumerate()
            .map(|(i, x)| {
                if i % 16 == 0 {
                    let p = curve(&PEDAL, i as f32 / SR);
                    wah.set(400.0 * 5f32.powf(p), 5.0);
                    // A real pedal's peak gets louder towards the toe.
                    open = 1.0 + 1.5 * p;
                }
                wah.tick(*x) * open + x * 0.05
            })
            .collect(),
    );

    let mut bass = Vec::new();
    for (at, len, f) in [
        (0.0, 0.26, 82.4),
        (0.45, 0.13, 82.4),
        (0.6, 0.27, 98.0),
        (0.9, 0.8, 82.4),
    ] {
        add_at(&mut bass, &bass_note(len, f), at, 1.0);
    }
    one_pole_lowpass(&mut bass, 700.0, SAMPLE_RATE);
    add_at(&mut b, &unit_rms(bass), 0.0, 0.3);
    let mut hats = Vec::new();
    for &(at, g) in &[
        (0.0f32, 1.0f32),
        (0.3, 0.7),
        (0.6, 1.0),
        (0.9, 0.8),
        (1.2, 0.7),
        (1.5, 0.6),
    ] {
        add_at(&mut hats, &burst(0.05, 0.01, 6000.0, 14000.0, rng), at, g);
    }
    add_at(&mut b, &unit_rms(hats), 0.0, 0.12);
    b.truncate(secs(1.85));
    one_pole_lowpass(&mut b, 7000.0, SAMPLE_RATE);
    fade_out(&mut b, 0.12);
    finish(b, 0.85)
}

/// The Boss beaten: a retro victory jingle on the NES voices — a fast rising arpeggio,
/// two pick-up notes and a held top note with vibrato over a thirds harmony, triangle
/// bass underneath, a snare on the landing.
fn boss_defeated() -> Vec<f32> {
    use audio::chiptune::{pulse_lead, snare, triangle_bass};
    use audio::pitch::note_freq;
    use audio::sfx::{SfxParams, SfxWave};
    use audio::synth::Envelope;
    let sr = SAMPLE_RATE;
    let mut lead = Vec::new();
    // C major up two octaves, a 16th each (C5 = A4 + 3 semitones).
    for (i, n) in [3.0f32, 7.0, 10.0, 15.0, 19.0, 22.0].iter().enumerate() {
        add_at(
            &mut lead,
            &pulse_lead(note_freq(*n), 0.07, 0.25, sr),
            i as f32 * 0.06,
            0.8,
        );
    }
    add_at(
        &mut lead,
        &pulse_lead(note_freq(17.0), 0.1, 0.5, sr),
        0.42,
        1.0,
    );
    add_at(
        &mut lead,
        &pulse_lead(note_freq(19.0), 0.1, 0.5, sr),
        0.54,
        1.0,
    );
    let held = |semi: f32, duty: f32| {
        SfxParams {
            wave: SfxWave::Square { duty },
            envelope: Envelope {
                attack: 0.01,
                decay: 0.08,
                sustain_level: 0.75,
                sustain: 0.55,
                release: 0.3,
            },
            start_freq: note_freq(semi),
            freq_slide: 0.0,
            freq_delta_slide: 0.0,
            arp_semitones: 0.0,
            arp_time_frac: 0.5,
            vibrato_depth: 0.008,
            vibrato_speed: 6.0,
        }
        .render(sr, 1)
    };
    add_at(&mut lead, &held(27.0, 0.25), 0.66, 1.0);
    add_at(&mut lead, &held(22.0, 0.5), 0.66, 0.55);
    let mut bass = Vec::new();
    for (at, n, len) in [
        (0.0, -21.0, 0.34),
        (0.36, -26.0, 0.14),
        (0.5, -24.0, 0.14),
        (0.66, -21.0, 0.9),
    ] {
        add_at(&mut bass, &triangle_bass(note_freq(n), len, sr), at, 1.0);
    }
    let mut b = unit_rms(lead);
    add_at(&mut b, &unit_rms(bass), 0.0, 0.7);
    add_at(&mut b, &unit_rms(snare(sr)), 0.66, 0.5);
    one_pole_lowpass(&mut b, 9000.0, SAMPLE_RATE);
    fade_out(&mut b, 0.05);
    finish(b, 0.8)
}

/// An office lift arriving: a soft mallet on two tuned chime bars, "ding ... dong" a
/// major third down — a free bar's overtones sit at 2.76x and 5.40x, not the harmonic
/// series, which is what makes it a bell and not a beep; a second fundamental 1.3 Hz off
/// slowly beats against the first. Then the doors: a low rumble of rollers in the track
/// swelling as they slide, and the soft bump of them stopping.
fn elevator(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    for (at, f) in [(0.0, 1046.5f32), (0.38, 830.6)] {
        let mut bar = ring(1.4, f, 0.9);
        add_at(&mut bar, &ring(1.4, f + 1.3, 0.9), 0.0, 0.5);
        add_at(&mut bar, &ring(0.6, f * 2.756, 0.22), 0.0, 0.3);
        add_at(&mut bar, &ring(0.3, f * 5.404, 0.06), 0.0, 0.12);
        add_at(
            &mut bar,
            &burst(0.01, 0.0015, 1500.0, 6000.0, rng),
            0.0,
            0.15,
        );
        add_at(&mut b, &bar, at, 1.0);
    }
    let b = unit_rms(b);
    let (from, to) = (0.75f32, 1.55f32);
    let n = secs(to - from);
    let mut acc = 0.0f32;
    let mut roll: Vec<f32> = noise(n, rng)
        .iter()
        .map(|x| {
            acc = acc * 0.99 + x;
            acc
        })
        .collect();
    band(&mut roll, 50.0, 400.0);
    for (i, s) in roll.iter_mut().enumerate() {
        let k = i as f32 / n as f32;
        *s *= (k / 0.3).min(1.0) * ((1.0 - k) / 0.15).min(1.0);
    }
    let mut out = b;
    add_at(&mut out, &unit_rms(roll), from, 0.7);
    let exc = burst(0.01, 0.003, 40.0, 1500.0, rng);
    add_at(&mut out, &mode(&exc, 110.0, 0.03, 1.0), to - 0.02, 0.9);
    add_at(
        &mut out,
        &burst(0.06, 0.012, 60.0, 600.0, rng),
        to - 0.02,
        0.7,
    );
    out.truncate(secs(1.78));
    one_pole_lowpass(&mut out, 6000.0, SAMPLE_RATE);
    fade_out(&mut out, 0.15);
    finish(out, 0.8)
}

// ── voices ───────────────────────────────────────────────────────────────────
//
// Gibberish speech for the Boss argument, built like a real voice: a glottal pulse
// train (the vocal folds) through three formant resonators (the throat and mouth). Each
// syllable glides the formants to a new vowel; consonants are silence + a burst (stops),
// hiss (fricatives) or a hummed low formant (nasals). No words, just the music of a
// man telling you something important.

/// Vowel formants F1-F3 (Hz), an adult male voice: ah, eh, ee, oh, oo, a (cat), uh.
const VOWELS: [[f32; 3]; 7] = [
    [730.0, 1090.0, 2440.0],
    [530.0, 1840.0, 2480.0],
    [270.0, 2290.0, 3010.0],
    [570.0, 840.0, 2410.0],
    [300.0, 870.0, 2240.0],
    [660.0, 1720.0, 2410.0],
    [520.0, 1190.0, 2390.0],
];
/// A nasal murmur ("m", "n"): one strong low resonance, the mouth shut.
const NASAL: [f32; 3] = [250.0, 1100.0, 2300.0];

#[derive(Clone, Copy)]
enum Onset {
    Stop,
    Fricative,
    Nasal,
    Vowel,
}

/// Who is talking.
struct Speaker {
    /// Mean pitch, Hz.
    f0: f32,
    /// Pitch across the line, `k` 0..1 -> multiplier.
    contour: fn(f32) -> f32,
    /// Formant scale: a shorter vocal tract raises every formant.
    tract: f32,
    syllables: (usize, usize),
    syl_len: (f32, f32),
    /// Robot: pitch held flat per syllable and stepped, then ring-modulated and crushed.
    robot: bool,
}

/// Rosenberg glottal pulse at phase `p` (0..1): the folds open slowly, snap shut, stay
/// shut. Its derivative (the lips radiate the change in airflow) is what's heard.
fn glottal(p: f32) -> f32 {
    use std::f32::consts::PI;
    if p < 0.4 {
        0.5 * (1.0 - (PI * p / 0.4).cos())
    } else if p < 0.56 {
        (PI * 0.5 * (p - 0.4) / 0.16).cos()
    } else {
        0.0
    }
}

/// One gibberish line from `who`.
fn babble(rng: &mut Rng, who: &Speaker) -> Vec<f32> {
    // Plan: per sample — voicing level, hiss level, pitch multiplier, formant targets.
    let count = who.syllables.0 + rng.index(who.syllables.1 - who.syllables.0 + 1);
    let mut voiced = Vec::new();
    let mut hiss = Vec::new();
    let mut pitch = Vec::new();
    let mut target: Vec<[f32; 3]> = Vec::new();
    let mut step = 1.0f32;
    for s in 0..count {
        let len = rng.range(who.syl_len.0, who.syl_len.1);
        let vowel = VOWELS[rng.index(VOWELS.len())];
        let onset = if s == 0 {
            [Onset::Stop, Onset::Nasal, Onset::Vowel][rng.index(3)]
        } else {
            [Onset::Stop, Onset::Stop, Onset::Fricative, Onset::Nasal][rng.index(4)]
        };
        // Words: a small breath gap now and then.
        if s > 0 && rng.next_f32() < 0.3 {
            for _ in 0..secs(rng.range(0.04, 0.08)) {
                voiced.push(0.0);
                hiss.push(0.0);
                pitch.push(step);
                target.push(vowel);
            }
        }
        if who.robot {
            step = [1.0f32, 1.12, 0.89, 1.26, 1.0][rng.index(5)];
        } else {
            // Stress: the first syllable of a word pushed up, the rest settling.
            step = if s == 0 || rng.next_f32() < 0.3 {
                1.12
            } else {
                1.0
            };
        }
        let (c_len, c_voice, c_hiss, c_form) = match onset {
            Onset::Stop => (0.035, 0.0, 0.0, vowel),
            Onset::Fricative => (0.07, 0.0, 1.0, vowel),
            Onset::Nasal => (0.05, 0.5, 0.0, NASAL),
            Onset::Vowel => (0.0, 0.0, 0.0, vowel),
        };
        for i in 0..secs(c_len) {
            // A stop's release: a short burst of breath at the end of its closure.
            let burst = matches!(onset, Onset::Stop) && i + secs(0.008) >= secs(c_len);
            voiced.push(c_voice);
            hiss.push(if burst { 0.7 } else { c_hiss });
            pitch.push(step);
            target.push(c_form);
        }
        let n = secs(len - c_len);
        for i in 0..n {
            let k = i as f32 / n as f32;
            voiced.push((k / 0.1).min(1.0) * ((1.0 - k) / 0.25).min(1.0));
            hiss.push(0.0);
            pitch.push(step);
            target.push(vowel);
        }
    }
    let total = voiced.len();
    // Smooth the plan: the tongue glides between vowels (~25 ms), the level ramps.
    let glide = 1.0 - (-1.0 / (0.025 * SR)).exp();
    let ramp = 1.0 - (-1.0 / (0.008 * SR)).exp();
    let pglide = 1.0 - (-1.0 / (0.03 * SR)).exp();
    let mut form = target[0];
    let (mut v, mut h, mut pm) = (0.0f32, 0.0f32, pitch[0]);
    // Formant bandwidths (Hz) and levels: F2/F3 lifted against the source's falling
    // spectrum, or every vowel collapses into "oo".
    const BW: [f32; 3] = [70.0, 100.0, 140.0];
    let mut res = [
        Bandpass::new(500.0, 6.0),
        Bandpass::new(1500.0, 12.0),
        Bandpass::new(2500.0, 16.0),
    ];
    let gains = [1.0f32, 1.5, 1.1];
    let mut sss = noise(total, rng);
    band(&mut sss, 2500.0, 6500.0);
    band(&mut sss, 2500.0, 6500.0);
    let sss = unit_rms(sss);
    let (mut ph, mut last_g) = (0.0f32, 0.0f32);
    let wobble = drift(total, 0.08, rng);
    let air = noise(total, rng);
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let k = i as f32 / total as f32;
        for (f, t) in form.iter_mut().zip(&target[i]) {
            *f += glide * (t * who.tract - *f);
        }
        v += ramp * (voiced[i] - v);
        h += ramp * (hiss[i] - h);
        let p_target = if who.robot {
            pitch[i]
        } else {
            pitch[i] * (0.97 + 0.06 * wobble[i])
        };
        pm += if who.robot { 1.0 } else { pglide } * (p_target - pm);
        ph = (ph + who.f0 * (who.contour)(k) * pm / SR).fract();
        let g = glottal(ph);
        let src = (g - last_g) * 40.0 + air[i] * 0.03;
        last_g = g;
        if i % 32 == 0 {
            for ((r, f), bw) in res.iter_mut().zip(&form).zip(BW) {
                r.set(*f, f / bw);
            }
        }
        let voice: f32 = res
            .iter_mut()
            .zip(gains)
            .map(|(r, g)| r.tick(src * v) * g)
            .sum();
        // Hiss: breath through the teeth.
        out.push(voice + sss[i] * h * 0.05);
    }
    if who.robot {
        // Ring-modulated against a 70 Hz carrier (half and half with the dry voice), then
        // held at 11 kHz and crushed to 6 bits: a cheap speech chip.
        let mut hold = 0.0f32;
        let peak = out.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-9);
        for (i, s) in out.iter_mut().enumerate() {
            let t = i as f32 / SR;
            let x = *s / peak * (0.55 + 0.45 * (t * 70.0 * TAU).sin());
            if i % 4 == 0 {
                hold = (x * 32.0).round() / 32.0;
            }
            *s = hold;
        }
        one_pole_lowpass(&mut out, 5500.0, SAMPLE_RATE);
    }
    one_pole_highpass(&mut out, 70.0, SAMPLE_RATE);
    fade_out(&mut out, 0.02);
    finish(out, 0.9)
}

/// The Boss: a big low voice (F0 ~110 Hz), pompous — up on the first word, falling away
/// to the end of the line.
fn boss_line(rng: &mut Rng) -> Vec<f32> {
    babble(
        rng,
        &Speaker {
            f0: 112.0,
            contour: |k| 1.08 - 0.22 * k,
            tract: 0.95,
            syllables: (4, 5),
            syl_len: (0.11, 0.18),
            robot: false,
        },
    )
}

/// The lady boss: a woman's voice (F0 ~210 Hz, shorter tract), clipped and emphatic —
/// the same falling contour of someone used to having the last word.
fn lady_line(rng: &mut Rng) -> Vec<f32> {
    babble(
        rng,
        &Speaker {
            f0: 210.0,
            contour: |k| 1.12 - 0.28 * k,
            tract: 0.82,
            syllables: (4, 5),
            syl_len: (0.1, 0.16),
            robot: false,
        },
    )
}

/// Someone in an occupied washroom answering the handle: two or three sharp,
/// rising-then-falling syllables ("Oc-cu-pied!"), muffled through the door and the
/// tiled room — a man's voice or a woman's, by the door's sign. Starts after a beat of
/// silence: the handle's rattle goes first.
fn wc_busy(rng: &mut Rng, mens: bool) -> Vec<f32> {
    // Hers well up: at 220 Hz behind the muffling it passed for a man's.
    let (f0, tract, muffle) = if mens {
        (120.0, 0.95, 1800.0)
    } else {
        (290.0, 0.78, 2400.0)
    };
    let mut b = babble(
        rng,
        &Speaker {
            f0,
            contour: |k| 1.05 + 0.25 * (std::f32::consts::PI * k).sin() - 0.15 * k,
            tract,
            syllables: (2, 3),
            syl_len: (0.1, 0.16),
            robot: false,
        },
    );
    one_pole_lowpass(&mut b, muffle, SAMPLE_RATE);
    room(&mut b, 0.5, 2500.0, 0.3);
    let mut out = vec![0.0; secs(0.3)];
    out.extend(b);
    fade_out(&mut out, 0.05);
    finish(out, 0.9)
}

fn wc_busy_man(rng: &mut Rng) -> Vec<f32> {
    wc_busy(rng, true)
}

fn wc_busy_woman(rng: &mut Rng) -> Vec<f32> {
    wc_busy(rng, false)
}

/// The bot: higher (F0 ~200 Hz), a shorter synthetic tract, stepped robotic pitch.
fn bot_line(rng: &mut Rng) -> Vec<f32> {
    babble(
        rng,
        &Speaker {
            f0: 200.0,
            contour: |k| 1.0 - 0.05 * k,
            tract: 1.15,
            syllables: (4, 6),
            syl_len: (0.1, 0.16),
            robot: true,
        },
    )
}

// ── the Dungeon's other inmates ──────────────────────────────────────────────
//
// Voices from somewhere deep in the stone: the same glottal-pulse synth as the Boss,
// then pushed far away — muffled through the walls (low-passed), drowned in a long
// stone reverb, played quiet. Deliberately ambiguous: a groan that might be pain or
// might be something else entirely (a nod to a certain gym-locker-room meme), a woman
// sobbing, a wail. The player should wonder, not know.

/// Far away through stone: muffle, then a long wet vault reverb.
fn far_off(mut b: Vec<f32>, muffle: f32) -> Vec<f32> {
    one_pole_lowpass(&mut b, muffle, SAMPLE_RATE);
    one_pole_lowpass(&mut b, muffle * 1.4, SAMPLE_RATE);
    room(&mut b, 2.2, 900.0, 1.6);
    fade_out(&mut b, 0.3);
    finish(b, 0.9)
}

/// A man's long, low groan — one vowel held, sagging, swelling up at the end. Pain?
/// Effort? Pleasure? Hard to say from here.
fn groan(rng: &mut Rng) -> Vec<f32> {
    let f0 = rng.range(90.0, 115.0);
    let b = babble(
        rng,
        &Speaker {
            f0,
            contour: |k| 0.95 + 0.3 * (k * k * k) - 0.1 * (std::f32::consts::PI * k).sin(),
            tract: 0.95,
            syllables: (1, 2),
            syl_len: (0.8, 1.4),
            robot: false,
        },
    );
    far_off(b, 1200.0)
}

/// Someone sobbing: short catching syllables, high and trembling, falling away.
fn sob(rng: &mut Rng) -> Vec<f32> {
    let f0 = rng.range(250.0, 310.0);
    let b = babble(
        rng,
        &Speaker {
            f0,
            contour: |k| (1.1 - 0.25 * k) * (1.0 + 0.05 * (k * 70.0).sin()),
            tract: 0.84,
            syllables: (4, 7),
            syl_len: (0.14, 0.26),
            robot: false,
        },
    );
    far_off(b, 1500.0)
}

/// A wail: a high voice rising, held, breaking downward.
fn wail(rng: &mut Rng) -> Vec<f32> {
    let f0 = rng.range(230.0, 300.0);
    let b = babble(
        rng,
        &Speaker {
            f0,
            contour: |k| {
                (1.0 + 0.35 * (k * 3.0).min(1.0) - 0.45 * ((k - 0.7) / 0.3).max(0.0))
                    * (1.0 + 0.03 * (k * 40.0).sin())
            },
            tract: 0.86,
            syllables: (1, 1),
            syl_len: (1.1, 1.7),
            robot: false,
        },
    );
    far_off(b, 1300.0)
}

/// One distant voice, of any of the kinds.
fn inmate(rng: &mut Rng) -> Vec<f32> {
    match rng.index(3) {
        0 => groan(rng),
        1 => sob(rng),
        _ => wail(rng),
    }
}

/// One iron link striking another: a hard click and the link's own ring — a few
/// inharmonic partials (a small torus's ratios) from `base`, the higher ones dying
/// first, damped by the links it touches within a few tens of ms.
fn clink(rng: &mut Rng, base: f32) -> Vec<f32> {
    let mut c = burst(0.01, 0.0012, 2500.0, 9000.0, rng);
    for (ratio, tau, g) in [
        (1.0, 0.045, 1.0),
        (2.32, 0.032, 0.8),
        (4.25, 0.02, 0.55),
        (6.63, 0.012, 0.35),
    ] {
        let f = (base * ratio * rng.range(0.97, 1.03)).min(11000.0);
        add_at(
            &mut c,
            &ring(0.2, f, tau * rng.range(0.8, 1.2)),
            0.0,
            g * 0.5,
        );
    }
    c
}

/// The chain's links ring high: each link a cluster of partials around these lines
/// (Hz), measured off a real chain rattle — the metal lives at 6–15 kHz, re-struck and
/// ringing on for a few hundred ms, over each hit's thud and clank. Every version that
/// rang tones at 0.3–3 kHz read as a chime, a dreamcatcher or a xylophone.
const CHAIN_LINES: [f32; 11] = [
    3840.0, 5120.0, 6040.0, 7320.0, 8150.0, 9030.0, 9710.0, 9880.0, 11500.0, 14100.0, 15400.0,
];

/// Walking past a hanging chain or shackles: a burst of links knocking as it's
/// brushed, 8–12 hits in half a second, then the rings dying away. Each hit: a thud
/// (~150 Hz, the links' weight — half the energy), a short clank round 1.5 kHz, a
/// broadband tick, and a few links re-struck — each link its own slightly detuned set
/// of `CHAIN_LINES`, so struck together they beat like a real chain's smeared lines.
fn chain_rattle(rng: &mut Rng) -> Vec<f32> {
    const LINKS: usize = 5;
    // Each link: most lines, detuned a little, each with its own decay and level.
    let mut links: Vec<Vec<(f32, f32, f32)>> = Vec::new();
    for _ in 0..LINKS {
        let mut link = Vec::new();
        for &f in &CHAIN_LINES {
            if rng.next_f32() < 0.7 {
                let f = f * rng.range(0.995, 1.005);
                // Links touching damp each other fast; the top lines are faint.
                let tau = rng.range(0.03, 0.11) * (6000.0 / f).powf(0.3);
                let top = if f > 12000.0 {
                    0.35
                } else if f < 6000.0 {
                    0.6
                } else {
                    1.0
                };
                link.push((f, tau, rng.range(0.3, 1.0) * top));
            }
        }
        links.push(link);
    }
    let mut b = vec![0.0; secs(1.6)];
    let hits = 8 + rng.index(5);
    let mut at = 0.0f32;
    for h in 0..hits {
        let k = h as f32 / hits as f32;
        // The first knocks hardest; the rest settle, unevenly.
        let g = (1.0 - 0.75 * k) * rng.range(0.55, 1.0);
        let thud = thump(0.08, rng.range(140.0, 170.0), 110.0, 0.025);
        add_at(&mut b, &thud, at, g * 0.5);
        let clank = burst(0.05, 0.012, 1100.0, 2000.0, rng);
        add_at(&mut b, &clank, at, g * 1.5);
        let tick = burst(0.01, 0.0015, 3000.0, 16000.0, rng);
        add_at(&mut b, &tick, at, g * 0.35);
        for _ in 0..1 + rng.index(3) {
            let link = &links[rng.index(LINKS)];
            let lg = g * rng.range(0.4, 1.0);
            for &(f, tau, amp) in link {
                add_at(&mut b, &ring(tau * 5.0, f, tau), at, lg * amp * 0.22);
            }
        }
        at += rng.range(0.025, 0.09);
    }
    b.truncate(secs(at + 0.4));
    echo(&mut b, &[(0.023, 0.2), (0.051, 0.1)], 6000.0);
    fade_out(&mut b, 0.15);
    finish(b, 0.9)
}

// ── ambience ─────────────────────────────────────────────────────────────────

/// Makes `buf` loop seamlessly at `loop_len` samples: the rendered tail past the loop
/// point is cross-faded into the head.
fn seamless(mut buf: Vec<f32>, loop_len: usize) -> Vec<f32> {
    let fade = buf.len() - loop_len;
    for i in 0..fade {
        let k = i as f32 / fade as f32;
        buf[i] = buf[i] * k + buf[loop_len + i] * (1.0 - k);
    }
    buf.truncate(loop_len);
    buf
}

/// Backrooms: fluorescent tubes and the room they hang in — a quiet 120 Hz ballast
/// hum with soft low harmonics, a second tube a quarter-hertz off so the two slowly
/// beat, a little unsteadiness, over broadband room tone and air. Every partial and
/// wobble completes whole cycles in the 4 s loop.
fn office_hum(rng: &mut Rng) -> Vec<f32> {
    let len = 4.0;
    let n = secs(len);
    let total = n + secs(0.4);
    let w = TAU / len;
    let tone: Vec<f32> = (0..total)
        .map(|i| {
            let t = i as f32 / SR;
            let wobble = 1.0
                + 0.06 * (w * t + 0.3).sin()
                + 0.04 * (3.0 * w * t + 1.1).sin()
                + 0.03 * (7.0 * w * t + 2.0).sin();
            let x = (t * 120.0 * TAU).sin()
                + 0.4 * (t * 120.25 * TAU + 0.8).sin()
                + 0.3 * (t * 240.0 * TAU + 0.7).sin()
                + 0.1 * (t * 240.5 * TAU).sin()
                + 0.1 * (t * 360.0 * TAU + 1.9).sin()
                + 0.05 * (t * 480.0 * TAU + 0.4).sin();
            x * wobble
        })
        .collect();
    let tone = unit_rms(tone);
    // Room tone: brown-ish (integrated) noise, the low air of a big empty floor.
    let mut acc = 0.0f32;
    let brown: Vec<f32> = noise(total, rng)
        .iter()
        .map(|x| {
            acc = acc * 0.995 + x;
            acc
        })
        .collect();
    let mut roomtone = brown;
    band(&mut roomtone, 40.0, 900.0);
    let roomtone = unit_rms(roomtone);
    let mut air = noise(total, rng);
    band(&mut air, 1500.0, 6000.0);
    let air = unit_rms(air);
    let mut b: Vec<f32> = (0..total)
        .map(|i| tone[i] + roomtone[i] * 0.7 + air[i] * 0.12)
        .collect();
    one_pole_lowpass(&mut b, 3000.0, SAMPLE_RATE);
    finish(seamless(b, n), 0.9)
}

/// A blinking panel catching: starter click, then a stuttering overdriven buzz.
fn ballast_buzz(rng: &mut Rng) -> Vec<f32> {
    let len = rng.range(0.18, 0.35);
    let mut b: Vec<f32> = (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            let x = (t * 120.0 * TAU).sin() + 0.6 * (t * 360.0 * TAU).sin();
            (x * 4.0).tanh() * (1.0 - t / len)
        })
        .collect();
    // Zero-order-held gate: chop the buzz into 15-30 ms on/off segments.
    let mut t = 0usize;
    while t < b.len() {
        let seg = secs(rng.range(0.015, 0.03));
        let on = rng.next_f32() < 0.7;
        for s in b.iter_mut().skip(t).take(seg) {
            *s *= if on { 1.0 } else { 0.1 };
        }
        t += seg;
    }
    one_pole_lowpass(&mut b, 3000.0, SAMPLE_RATE);
    add_at(&mut b, &burst(0.02, 0.002, 2000.0, 8000.0, rng), 0.0, 0.8);
    finish(b, 0.9)
}

/// Forest: wind in the trees — a steady low rumble that never drops out, irregular
/// gusts leaning on it, two howling band-passed voices whose pitch rises with the gust,
/// and a leaf rustle that only wakes up when a gust hits. Gust curves are built from
/// several harmonics of the loop length (seamless, but no single swell period).
fn forest_wind(rng: &mut Rng) -> Vec<f32> {
    let len = 12.0;
    let n = secs(len);
    let total = n + secs(1.0);
    let g1 = gusts(total, len, &[1.0, 2.0, 3.0, 5.0, 7.0, 11.0], rng);
    let g2 = gusts(total, len, &[2.0, 3.0, 4.0, 6.0, 9.0, 13.0], rng);
    let gust = |i: usize| g1[i] * g1[i] * 0.8 + g2[i] * 0.2;

    let mut rumble = noise(total, rng);
    one_pole_lowpass(&mut rumble, 220.0, SAMPLE_RATE);
    one_pole_lowpass(&mut rumble, 220.0, SAMPLE_RATE);
    one_pole_highpass(&mut rumble, 25.0, SAMPLE_RATE);
    let rumble = unit_rms(rumble);
    let mut body = noise(total, rng);
    band(&mut body, 200.0, 1000.0);
    let body = unit_rms(body);

    // Howl: white noise through narrow band-passes that glide with gust strength, the
    // second voice on its own curve so they drift in and out of step.
    let src = noise(total, rng);
    let (mut h1, mut h2) = (Bandpass::new(400.0, 10.0), Bandpass::new(900.0, 14.0));
    let mut howl1 = Vec::with_capacity(total);
    let mut howl2 = Vec::with_capacity(total);
    for (i, x) in src.iter().enumerate() {
        if i % 32 == 0 {
            h1.set(300.0 + 520.0 * gust(i), 10.0);
            h2.set(650.0 + 800.0 * g2[i] * g2[i], 14.0);
        }
        howl1.push(h1.tick(*x));
        howl2.push(h2.tick(*x));
    }
    let (howl1, howl2) = (unit_rms(howl1), unit_rms(howl2));

    let mut leaves = noise(total, rng);
    band(&mut leaves, 1800.0, 6000.0);
    let leaves = unit_rms(leaves);
    let flutter = drift(total, 0.05, rng);

    let b: Vec<f32> = (0..total)
        .map(|i| {
            let g = gust(i);
            rumble[i] * (0.75 + 0.35 * g)
                + body[i] * (0.15 + 0.35 * g)
                + howl1[i] * 0.3 * g.powf(1.5)
                + howl2[i] * 0.15 * (g2[i] * g2[i])
                + leaves[i] * 0.2 * g * g * (0.4 + 0.6 * flutter[i])
        })
        .collect();
    finish(seamless(b, n), 0.9)
}

/// An owl somewhere off in the trees: "hoo ... hoo-hoo ... hooo", breathy, a touch of
/// distance on it.
fn owl(rng: &mut Rng) -> Vec<f32> {
    let f0 = rng.range(300.0, 390.0);
    let calls: [(f32, f32); 4] = [(0.0, 0.28), (0.5, 0.14), (0.7, 0.14), (0.95, 0.55)];
    let mut b = Vec::new();
    for (at, len) in calls {
        let mut ph = 0.0f32;
        let hoo: Vec<f32> = (0..secs(len))
            .map(|i| {
                let t = i as f32 / SR;
                let k = t / len;
                // Scoop up at the onset, sag at the end.
                let f = f0 * (1.0 + 0.06 * (k * 6.0).min(1.0) - 0.08 * k * k);
                ph += f / SR;
                let env = (t / 0.04).min(1.0) * ((len - t) / 0.08).clamp(0.0, 1.0);
                ((ph * TAU).sin() + 0.12 * (ph * 2.0 * TAU).sin()) * env
            })
            .collect();
        add_at(&mut b, &hoo, at, 1.0);
    }
    let mut breath = noise(b.len(), rng);
    one_pole_lowpass(&mut breath, 900.0, SAMPLE_RATE);
    for (s, n) in b.iter_mut().zip(&breath) {
        *s += n * s.abs() * 0.6;
    }
    one_pole_lowpass(&mut b, 1400.0, SAMPLE_RATE);
    echo(&mut b, &[(0.23, 0.18), (0.47, 0.08)], 900.0);
    finish(b, 0.9)
}

/// A crow's "kaaah", modelled on recordings: a hoarse, noisy voice around 550 Hz whose
/// pitch rises into the call then sags, rough from period-doubling (every other pulse
/// weaker — a subharmonic growl) and a 40-70 Hz flutter, shaped by an open "aa" (F1
/// sweeping up and back, F2/F3 fixed) behind a short "k" onset. Three to five calls,
/// each a little lower and shorter, with a crow's steady spacing between them.
fn caw(rng: &mut Rng) -> Vec<f32> {
    let f0 = rng.range(500.0, 620.0);
    let n = 3 + rng.index(3);
    let rough = rng.range(40.0, 70.0);
    let mut b = Vec::new();
    let mut at = 0.0;
    for k in 0..n {
        let len = rng.range(0.3, 0.42) * (1.0 - 0.05 * k as f32);
        let fk = f0 * (1.0 - 0.03 * k as f32);
        let (mut f1, mut f2, mut f3) = (
            Bandpass::new(900.0, 4.0),
            Bandpass::new(rng.range(1800.0, 2200.0), 5.0),
            Bandpass::new(rng.range(2900.0, 3300.0), 6.0),
        );
        let mut ph = 0.0f32;
        let mut jit = 1.0;
        let v: Vec<f32> = (0..secs(len))
            .map(|i| {
                let t = i as f32 / SR;
                let x = t / len;
                // Up into the call over the first quarter, sagging after.
                let contour = if x < 0.25 {
                    0.85 + 0.15 * x / 0.25
                } else {
                    1.0 - 0.28 * (x - 0.25) / 0.75
                };
                let before = ph;
                ph += fk * contour * jit / SR;
                if ph.floor() != before.floor() {
                    // New period: fresh jitter.
                    jit = rng.range(0.94, 1.06);
                }
                let weak = if (ph as u32).is_multiple_of(2) {
                    1.0
                } else {
                    0.55
                };
                let pulse = glottal(ph.fract()) * weak;
                let breath = rng.range(-1.0, 1.0) * (0.25 + 0.5 * pulse);
                let flutter = 1.0 - 0.45 * (0.5 + 0.5 * (TAU * rough * t).sin());
                let src = (pulse + breath * 0.5) * flutter;
                // The "aa": F1 opens up to ~1300 Hz and closes again.
                f1.set(900.0 + 400.0 * (x * std::f32::consts::PI).sin(), 4.0);
                let env =
                    (t / 0.015).min(1.0) * (1.0 - 0.3 * x) * ((len - t) / 0.06).clamp(0.0, 1.0);
                (f1.tick(src) + 0.8 * f2.tick(src) + 0.35 * f3.tick(src)) * env
            })
            .collect();
        let click = burst(0.012, 0.004, 1500.0, 4000.0, rng);
        add_at(&mut b, &click, at, 0.35);
        add_at(&mut b, &v, at + 0.004, 1.0);
        at += len + rng.range(0.22, 0.34);
    }
    one_pole_highpass(&mut b, 350.0, SAMPLE_RATE);
    one_pole_lowpass(&mut b, 4500.0, SAMPLE_RATE);
    echo(&mut b, &[(0.17, 0.12), (0.36, 0.05)], 1800.0);
    finish(b, 0.9)
}

/// A crow taking off: a clatter of wingbeats, fast at first then settling, fading as
/// it climbs away.
fn wingbeats(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    let mut at = 0.0;
    for k in 0..7 {
        let beat = burst(0.09, 0.03, 250.0, 2500.0, rng);
        add_at(&mut b, &beat, at, 0.85f32.powi(k));
        at += 0.085 + 0.012 * k as f32;
    }
    finish(b, 0.9)
}

/// A crow in flight: two or three wingbeats — each a soft push of air, not a hit: dark
/// noise (mostly under 700 Hz) swelling over ~35 ms and easing off, no transient at all.
/// (The first version was a fast-attack noise burst and read as gunshots.)
fn flaps(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    let mut at = 0.0;
    for k in 0..2 + rng.index(2) {
        let len = rng.range(0.09, 0.12);
        let mut beat = noise(secs(len), rng);
        band2(&mut beat, 120.0, 700.0);
        let n = beat.len() as f32;
        for (i, s) in beat.iter_mut().enumerate() {
            let x = i as f32 / n;
            // Rises over the first third (the downstroke), fades over the rest.
            let env = if x < 0.35 {
                (std::f32::consts::PI * x / 0.7).sin().powi(2)
            } else {
                (std::f32::consts::FRAC_PI_2 * (1.0 - x) / 0.65)
                    .sin()
                    .powi(2)
            };
            *s *= env;
        }
        add_at(&mut b, &beat, at, 0.85f32.powi(k as i32));
        at += rng.range(0.14, 0.18);
    }
    finish(b, 0.9)
}

/// A wolf far off in the forest: one long howl — rising from a low start, holding with
/// a slow wobble, falling away at the end — a breathy voiced tone with a couple of
/// harmonics and a nasal formant, lost in distance (low-passed, echoing off the trees).
/// Sometimes a second, higher wolf answers a moment later.
fn wolf(rng: &mut Rng) -> Vec<f32> {
    let one = |rng: &mut Rng, f0: f32| {
        let len = rng.range(2.4, 3.6);
        let mut ph = 0.0f32;
        let mut formant = Bandpass::new(900.0, 3.0);
        let v: Vec<f32> = (0..secs(len))
            .map(|i| {
                let t = i as f32 / SR;
                let x = t / len;
                // Scoop up over the first fifth, hold, sag over the last third.
                let contour = if x < 0.2 {
                    0.7 + 0.3 * (x / 0.2)
                } else if x < 0.66 {
                    1.0
                } else {
                    1.0 - 0.3 * (x - 0.66) / 0.34
                };
                let wobble = 1.0 + 0.012 * (TAU * 4.5 * t).sin();
                ph += f0 * contour * wobble / SR;
                let tone = (ph * TAU).sin()
                    + 0.35 * (2.0 * ph * TAU).sin()
                    + 0.12 * (3.0 * ph * TAU).sin();
                let breath = rng.range(-1.0, 1.0) * 0.15;
                let env = (t / 0.35).min(1.0) * ((len - t) / 0.6).clamp(0.0, 1.0);
                (tone + formant.tick(tone + breath) * 0.8) * env
            })
            .collect();
        v
    };
    let f0 = rng.range(380.0, 460.0);
    let mut b = one(rng, f0);
    if rng.next_f32() < 0.45 {
        let at = rng.range(1.2, 2.2);
        let f1 = rng.range(470.0, 560.0);
        let second = one(rng, f1);
        add_at(&mut b, &second, at, 0.6);
    }
    one_pole_lowpass(&mut b, 1400.0, SAMPLE_RATE);
    echo(&mut b, &[(0.31, 0.22), (0.67, 0.12), (1.1, 0.06)], 900.0);
    room(&mut b, 2.0, 1200.0, 0.2);
    finish(b, 0.9)
}

/// A fountain's trickle, 1.8 s (played back to back near it, so it can grow louder as
/// the bot nears and die away round the hedges): many small drops and bubbles — short
/// upward-gliding sine blips at 0.8–3 kHz — over a soft band of splashing water.
fn trickle(rng: &mut Rng) -> Vec<f32> {
    let len = 1.8;
    let mut b = noise(secs(len), rng);
    band2(&mut b, 400.0, 2500.0);
    let wash = drift(b.len(), 0.02, rng);
    for (s, w) in b.iter_mut().zip(&wash) {
        *s *= 0.25 + 0.15 * w;
    }
    for _ in 0..rng.index(30) + 50 {
        let at = rng.range(0.0, len - 0.05);
        let f = rng.range(800.0, 3000.0);
        let l = rng.range(0.008, 0.03);
        let blip = whistle(l, f, f * rng.range(1.2, 1.8), 0.0, 0.002);
        add_at(&mut b, &blip, at, rng.range(0.2, 0.7));
    }
    b.truncate(secs(len));
    // Fade the ends so back-to-back copies don't click.
    let n = b.len();
    let edge = secs(0.05);
    for i in 0..edge {
        let k = i as f32 / edge as f32;
        b[i] *= k;
        b[n - 1 - i] *= k;
    }
    finish(b, 0.9)
}

/// A rat's squeak: one to three short, thin, very high chirps (4-7 kHz, a quick upward
/// then downward glide, a little rough), with the stone corridor's short echo.
fn squeak(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    let mut at = 0.0;
    for _ in 0..1 + rng.index(3) {
        let len = rng.range(0.05, 0.12);
        let f0 = rng.range(4000.0, 6500.0);
        let mut ph = 0.0f32;
        let v: Vec<f32> = (0..secs(len))
            .map(|i| {
                let t = i as f32 / SR;
                let x = t / len;
                let f =
                    f0 * (1.0 + 0.18 * (std::f32::consts::PI * x).sin()) * rng.range(0.98, 1.02);
                ph += f / SR;
                let env = (std::f32::consts::PI * x).sin().powf(0.6);
                ((ph * TAU).sin() + 0.25 * (2.0 * ph * TAU).sin()) * env
            })
            .collect();
        add_at(&mut b, &v, at, rng.range(0.6, 1.0));
        at += len + rng.range(0.04, 0.1);
    }
    echo(&mut b, &[(0.021, 0.2), (0.047, 0.1)], 4000.0);
    finish(b, 0.9)
}

/// A rat dashing off: a quick patter of tiny claws on stone — sparse sharp ticks at
/// ~30 a second, bright and quiet, over a quarter to half a second.
fn scurry(rng: &mut Rng) -> Vec<f32> {
    let len = rng.range(0.25, 0.45);
    let mut b = vec![0.0; secs(len)];
    let mut at = 0.0;
    while at < len - 0.01 {
        let tick = burst(0.006, 0.0015, 2500.0, 8000.0, rng);
        add_at(
            &mut b,
            &tick,
            at,
            rng.range(0.4, 1.0) * (1.0 - at / len * 0.6),
        );
        at += rng.range(0.022, 0.045);
    }
    b.truncate(secs(len));
    finish(b, 0.9)
}

/// A field cricket in the dark: its wings rasp out a nearly pure 4-5 kHz tone in short
/// pulses, 3-5 pulses to a chirp, a chirp every half second or so, for a bout of a
/// second and a half.
fn cricket(rng: &mut Rng) -> Vec<f32> {
    let f = rng.range(3900.0, 5000.0);
    let pulse = rng.range(0.012, 0.018);
    let gap = pulse + rng.range(0.008, 0.014);
    let period = rng.range(0.38, 0.55);
    let per_chirp = rng.index(3) + 3;
    let bout = rng.range(1.2, 1.6);
    let mut b = Vec::new();
    let mut at = rng.range(0.0, 0.1);
    while at < bout {
        let level = rng.range(0.8, 1.0);
        for p in 0..per_chirp {
            // Each pulse sags a touch in pitch as the wing stroke slows.
            let fp = f * (1.0 - 0.01 * p as f32);
            let mut ph = 0.0f32;
            let tone: Vec<f32> = (0..secs(pulse))
                .map(|i| {
                    let k = i as f32 / secs(pulse) as f32;
                    ph += fp * (1.0 - 0.015 * k) / SR;
                    let env = (std::f32::consts::PI * k).sin().powi(2);
                    ((ph * TAU).sin() + 0.08 * (ph * 2.0 * TAU).sin()) * env
                })
                .collect();
            add_at(&mut b, &tone, at + p as f32 * gap, level);
        }
        at += period * rng.range(0.95, 1.05);
    }
    one_pole_lowpass(&mut b, 7000.0, SAMPLE_RATE);
    echo(&mut b, &[(0.07, 0.12)], 5000.0);
    finish(b, 0.9)
}

/// A dry twig snapping somewhere out in the dark: a sharp double crack with a short
/// woody ring, dulled by distance, the trees throwing a little of it back.
fn twig_snap(rng: &mut Rng) -> Vec<f32> {
    let mut b = burst(0.03, 0.0012, 1200.0, 7000.0, rng);
    let exc = burst(0.004, 0.0008, 500.0, 8000.0, rng);
    add_at(
        &mut b,
        &mode(&exc, rng.range(1300.0, 2000.0), 0.012, 0.5),
        0.0,
        1.0,
    );
    let second = rng.range(0.015, 0.04);
    add_at(
        &mut b,
        &burst(0.02, 0.0010, 1500.0, 7000.0, rng),
        second,
        0.6,
    );
    add_at(&mut b, &burst(0.06, 0.015, 600.0, 3000.0, rng), 0.002, 0.25);
    one_pole_lowpass(&mut b, 4500.0, SAMPLE_RATE);
    echo(&mut b, &[(0.12, 0.2), (0.26, 0.08)], 2000.0);
    finish(b, 0.9)
}

/// A torch burning: the low, fluttering roar of the flame (low-passed noise whose
/// loudness wanders every few tens of ms) and now and then a small crackle of resin —
/// faded at both ends so overlapping copies blend into one continuous burn.
fn torch_crackle(rng: &mut Rng) -> Vec<f32> {
    let len = 0.7;
    let n = secs(len);
    let mut roar = noise(n, rng);
    one_pole_lowpass(&mut roar, 380.0, SAMPLE_RATE);
    one_pole_lowpass(&mut roar, 380.0, SAMPLE_RATE);
    one_pole_highpass(&mut roar, 50.0, SAMPLE_RATE);
    let roar = unit_rms(roar);
    let mut gas = noise(n, rng);
    band(&mut gas, 700.0, 1800.0);
    let gas = unit_rms(gas);
    let flutter = drift(n, 0.05, rng);
    let mut b: Vec<f32> = (0..n)
        .map(|i| {
            let f = 0.35 + 0.65 * flutter[i] * flutter[i];
            (roar[i] + gas[i] * 0.08) * f
        })
        .collect();
    for _ in 0..rng.index(3) + 1 {
        // A crackle is a tight cluster of tiny clicks, not one.
        let at = rng.range(0.1, len - 0.1);
        let g = rng.range(1.2, 2.2);
        for k in 0..rng.index(3) + 1 {
            let pop = burst(0.008, rng.range(0.0004, 0.0012), 500.0, 5000.0, rng);
            add_at(&mut b, &pop, at + k as f32 * rng.range(0.003, 0.012), g);
        }
    }
    b.truncate(n);
    for (i, s) in b.iter_mut().enumerate() {
        let edge = (i.min(n - i) as f32 / secs(0.15) as f32).min(1.0);
        *s *= edge;
    }
    finish(b, 0.9)
}

// ── hearing ──────────────────────────────────────────────────────────────────

// ── garden (Mirror mode) ─────────────────────────────────────────────────────
//
// A walled English garden on a still afternoon, slightly through the looking-glass:
// a breeze in the hedges with birdsong far off (the bed), and now and then a blackbird
// or robin nearer by, a distant clock striking, and a
// pocket watch ticking past (somebody is late).

/// A whistled note gliding `f0 -> f1` Hz (`bow` bends the glide: > 0 arches it), with a
/// little vibrato, a soft 2nd harmonic and a short rise/fall. The unit of birdsong.
fn whistle(len: f32, f0: f32, f1: f32, bow: f32, vib: f32) -> Vec<f32> {
    let n = secs(len);
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let k = i as f32 / n as f32;
            let f = (f0 + (f1 - f0) * k + bow * f0 * 4.0 * k * (1.0 - k))
                * (1.0 + vib * (t * 38.0 * TAU).sin());
            ph += f / SR;
            let env = (t / 0.006).min(1.0) * ((len - t) / 0.012).clamp(0.0, 1.0);
            ((ph * TAU).sin() + 0.15 * (ph * 2.0 * TAU).sin()) * env
        })
        .collect()
}

/// Birds at some distance in a walled garden: highs softened, a faint wall echo.
fn birds_far(mut b: Vec<f32>, lp: f32) -> Vec<f32> {
    one_pole_lowpass(&mut b, lp, SAMPLE_RATE);
    echo(&mut b, &[(0.045, 0.12), (0.11, 0.05)], lp * 0.6);
    finish(b, 0.9)
}

/// Blackbird: a few low, mellow fluted notes (1.3-2.8 kHz, slurred and arched) and then
/// the squeaky high twitter it ends its phrase on.
fn blackbird(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    let mut at = 0.0;
    for _ in 0..rng.index(3) + 3 {
        let len = rng.range(0.1, 0.26);
        let f0 = rng.range(1300.0, 2400.0);
        let f1 = f0 * rng.range(0.8, 1.3);
        let note = whistle(len, f0, f1, rng.range(-0.08, 0.15), 0.006);
        add_at(&mut b, &note, at, rng.range(0.6, 1.0));
        at += len + rng.range(0.03, 0.09);
    }
    at += 0.04;
    for _ in 0..rng.index(5) + 3 {
        let len = rng.range(0.018, 0.035);
        let f0 = rng.range(4500.0, 7000.0);
        let note = whistle(len, f0, f0 * rng.range(0.7, 1.2), 0.0, 0.0);
        add_at(&mut b, &note, at, rng.range(0.25, 0.4));
        at += len + rng.range(0.008, 0.02);
    }
    birds_far(b, 6000.0)
}

/// Robin: thin, sweet and high (2.5-7 kHz) — quick glides and a little trill, notes
/// hopping up and down with no settled pitch.
fn robin(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    let mut at = 0.0;
    for _ in 0..rng.index(3) + 3 {
        if rng.next_f32() < 0.4 {
            let f = rng.range(3500.0, 6000.0);
            for _ in 0..rng.index(5) + 5 {
                add_at(&mut b, &whistle(0.018, f, f * 0.85, 0.0, 0.0), at, 0.6);
                at += 0.028;
            }
        } else {
            let len = rng.range(0.06, 0.18);
            let f0 = rng.range(2500.0, 7000.0);
            let f1 = rng.range(2500.0, 7000.0);
            let note = whistle(len, f0, f1, rng.range(-0.1, 0.1), 0.01);
            add_at(&mut b, &note, at, rng.range(0.6, 1.0));
            at += len;
        }
        at += rng.range(0.04, 0.12);
    }
    birds_far(b, 7000.0)
}

/// A clock somewhere beyond the hedges striking three: one bell, inharmonic partials,
/// dulled and echoed by distance.
fn clock_chime(rng: &mut Rng) -> Vec<f32> {
    let f = rng.range(480.0, 560.0);
    let mut strike = vec![0.0; secs(0.55)];
    for (r, tau, g) in [
        (0.5, 0.3, 0.5),
        (1.0, 0.22, 1.0),
        (1.19, 0.16, 0.5),
        (1.5, 0.14, 0.4),
        (2.0, 0.1, 0.35),
        (2.52, 0.07, 0.2),
    ] {
        add_at(&mut strike, &ring(0.55, f * r, tau), 0.0, g);
    }
    fade_out(&mut strike, 0.1);
    let mut b = Vec::new();
    for k in 0..3 {
        add_at(&mut b, &strike, k as f32 * 0.52, 1.0);
    }
    one_pole_lowpass(&mut b, 1800.0, SAMPLE_RATE);
    echo(&mut b, &[(0.12, 0.2), (0.26, 0.08)], 1200.0);
    finish(b, 0.9)
}

/// A pocket watch hurrying past: fast tick-tock (5 beats a second), tiny bright
/// metallic clicks — here, a pitched ring is the point.
fn pocket_watch(rng: &mut Rng) -> Vec<f32> {
    let mut b = Vec::new();
    for k in 0..8 {
        let tock = k % 2 == 1;
        let exc = burst(0.003, 0.0005, 2000.0, 12000.0, rng);
        let f = if tock { 3900.0 } else { 4600.0 } * rng.range(0.98, 1.02);
        let mut tick = mode(&exc, f, 0.004, 1.0);
        add_at(&mut tick, &exc, 0.0, 2.0);
        add_at(&mut b, &tick, k as f32 * 0.2, if tock { 0.75 } else { 1.0 });
    }
    // Walking past: in and out over the ticks.
    let n = b.len() as f32;
    for (i, s) in b.iter_mut().enumerate() {
        *s *= (std::f32::consts::PI * i as f32 / n).sin();
    }
    finish(b, 0.9)
}

/// Garden bed (12 s loop): a breeze in hedges and leaves swelling on irregular gusts,
/// a little low air, and far-off birdsong — sparse short chirps at many distances.
fn garden_bed(rng: &mut Rng) -> Vec<f32> {
    let len = 12.0;
    let n = secs(len);
    let total = n + secs(0.6);
    let g1 = gusts(total, len, &[1.0, 2.0, 3.0, 5.0, 8.0], rng);
    let mut rustle = noise(total, rng);
    band2(&mut rustle, 700.0, 4500.0);
    let rustle = unit_rms(rustle);
    let flutter = drift(total, 0.03, rng);
    let mut air = noise(total, rng);
    band(&mut air, 60.0, 400.0);
    let air = unit_rms(air);
    let mut b: Vec<f32> = (0..total)
        .map(|i| {
            let g = g1[i];
            // Leaves only rustle in the gusts: a steady floor of broadband hiss between
            // them read as a fountain nobody could find.
            rustle[i] * (0.01 + 0.35 * g * g) * (0.6 + 0.4 * flutter[i]) + air[i] * (0.2 + 0.15 * g)
        })
        .collect();
    // Distant birdsong: 20-30 little chirps and chirp-runs through the loop.
    let mut birds = vec![0.0f32; total];
    for _ in 0..rng.index(10) + 20 {
        let mut at = rng.range(0.0, len);
        let level = rng.range(0.15, 1.0);
        for _ in 0..rng.index(4) + 1 {
            let l = rng.range(0.03, 0.09);
            let f0 = rng.range(2500.0, 6000.0);
            let note = whistle(l, f0, f0 * rng.range(0.75, 1.3), 0.0, 0.01);
            add_at(&mut birds, &note, at, level);
            at += l + rng.range(0.02, 0.06);
        }
    }
    birds.truncate(total);
    one_pole_lowpass(&mut birds, 4000.0, SAMPLE_RATE);
    let birds = unit_rms(birds);
    for (s, bd) in b.iter_mut().zip(&birds) {
        *s += bd * 0.12;
    }
    finish(seamless(b, n), 0.9)
}

/// How far a sound has to travel to reach the bot: through the maze's open passages,
/// not through walls. A buzzing panel one wall away but twenty cells round by corridor
/// is twenty cells away. Walking distance from the bot's cell, recomputed each frame
/// (a BFS over at most a couple of thousand cells).
#[derive(Default)]
pub struct Ear {
    w: usize,
    dist: Vec<u32>,
}

impl Ear {
    pub fn update(&mut self, maze: &Maze, bot: Cell) {
        self.w = maze.w;
        self.dist.clear();
        self.dist.resize(maze.w * maze.h, u32::MAX);
        let mut q = std::collections::VecDeque::new();
        self.dist[maze.idx(bot)] = 0;
        q.push_back(bot);
        while let Some(c) = q.pop_front() {
            let d0 = self.dist[maze.idx(c)];
            for d in 0..4usize {
                // Sound passes open passages and open doors; a shut door stops it.
                let carries = match maze.edge(c, d) {
                    EdgeKind::Open => true,
                    EdgeKind::Door | EdgeKind::Locked(_) => maze.transparent(c, d),
                    _ => false,
                };
                let nb = c.step(d);
                if carries && self.dist[maze.idx(nb)] == u32::MAX {
                    self.dist[maze.idx(nb)] = d0 + 1;
                    q.push_back(nb);
                }
            }
        }
    }

    /// Audible distance, in grid units, from `cam` to a source at grid point `p`: the
    /// corridor route (2 grid units per cell), never less than the straight line.
    pub fn distance(&self, p: Vec2, cam: Vec2) -> f32 {
        let straight = p.distance(cam);
        let (x, y) = ((p.x / 2.0).floor() as i64, (p.y / 2.0).floor() as i64);
        let h = self.dist.len() / self.w.max(1);
        if x < 0 || y < 0 || x as usize >= self.w || y as usize >= h {
            return f32::INFINITY;
        }
        match self.dist[y as usize * self.w + x as usize] {
            u32::MAX => f32::INFINITY,
            cells => straight.max(cells as f32 * 2.0),
        }
    }
}

// ── player ───────────────────────────────────────────────────────────────────

async fn clips(bufs: Vec<Vec<f32>>) -> Vec<Clip> {
    let mut out = Vec::with_capacity(bufs.len());
    for b in bufs {
        out.push(Clip::from_samples(&b).await);
    }
    out
}

fn variants(n: usize, rng: &mut Rng, f: fn(&mut Rng) -> Vec<f32>) -> Vec<Vec<f32>> {
    (0..n).map(|_| f(rng)).collect()
}

/// Footstep play volumes: carpet, stone, leaves, gravel. Matched on RMS above 300 Hz
/// (what the ear weighs) rather than by peak — every clip leaves `finish` at one peak.
const STEP_VOL: [f32; 4] = [0.36, 0.40, 0.34, 0.29];
/// A faulty panel's buzz dies out this far along the route (grid units; 2 per cell).
const BUZZ_RANGE: f32 = 8.0;

/// Play volumes of the ambience, shared with the `dump_wavs` scenes so they stay
/// faithful to the game.
const HUM_VOL: f32 = 0.035;
const WIND_VOL: f32 = 0.07;
const TORCH_VOL: f32 = 0.055;
/// Garden: the breeze-and-far-birds bed, and its one-shots (ranges are picked per play).
const GARDEN_VOL: f32 = 0.03;
const SONGBIRD_VOL: (f32, f32) = (0.04, 0.09);
const CLOCK_VOL: f32 = 0.06;
const WATCH_VOL: f32 = 0.1;
const CRICKET_VOL: (f32, f32) = (0.02, 0.05);
const TWIG_VOL: (f32, f32) = (0.05, 0.12);
/// Far-off wolves: always distant.
const WOLF_VOL: (f32, f32) = (0.03, 0.06);
/// The garden fountain at its rim, fading to nothing by `FOUNTAIN_RANGE` (walking
/// distance — the hedges muffle it).
const FOUNTAIN_VOL: f32 = 0.09;
const FOUNTAIN_RANGE: f32 = 9.0;
/// Bird calls at the bot's side, fading to nothing by `BIRD_RANGE`.
const CAW_VOL: f32 = 0.22;
const WINGS_VOL: f32 = 0.16;
const FLAP_VOL: f32 = 0.1;
/// Rats: close by only.
const SQUEAK_VOL: f32 = 0.1;
const SCURRY_VOL: f32 = 0.08;
/// Dungeon: a distant voice every so often, quiet — under the torches.
const INMATE_VOL: (f32, f32) = (0.05, 0.12);
const INMATE_EVERY: (f32, f32) = (25.0, 60.0);
const CHAIN_VOL: f32 = 0.22;
const RAT_RANGE: f32 = 7.0;
const HOOT_VOL: f32 = 0.2;
const BIRD_RANGE: f32 = 16.0;

/// Event play volumes.
const CLOSET_VOL: f32 = 0.3;
/// The argument's voices — one line every ~0.8 s for ~8 s, so kept under the steps.
const BOSS_VOICE_VOL: f32 = 0.3;
const BOT_VOICE_VOL: f32 = 0.22;
const VICTORY_VOL: f32 = 0.3;
const ELEVATOR_VOL: f32 = 0.4;

pub struct Sound {
    carpet: Vec<Clip>,
    stone: Vec<Clip>,
    leaves: Vec<Clip>,
    gravel: Vec<Clip>,
    exit: Clip,
    key_pickup: Clip,
    key_turn: Clip,
    door: Clip,
    office_door: Clip,
    closet: Clip,
    boss_voice: Vec<Clip>,
    lady_voice: Vec<Clip>,
    bot_voice: Vec<Clip>,
    /// Occupied washrooms answering: men's, women's.
    busy: [Vec<Clip>; 2],
    /// Last line each side said (boss, bot), so nobody repeats himself back to back.
    last_line: (usize, usize),
    victory: Clip,
    elevator: Clip,
    hum: Clip,
    wind: Clip,
    buzz: Vec<Clip>,
    owl: Vec<Clip>,
    caw: Vec<Clip>,
    wings: Vec<Clip>,
    flaps: Vec<Clip>,
    squeak: Vec<Clip>,
    scurry: Vec<Clip>,
    inmates: Vec<Clip>,
    chains: Vec<Clip>,
    crickets: Vec<Clip>,
    wolves: Vec<Clip>,
    trickle: Vec<Clip>,
    twigs: Vec<Clip>,
    crackle: Vec<Clip>,
    garden: Clip,
    blackbird: Vec<Clip>,
    robin: Vec<Clip>,
    clock: Clip,
    watch: Clip,
    rng: Rng,
    /// Theme whose bed is looping right now (`None` = silent or muted).
    bed: Option<ThemeKind>,
    next_critter: f32,
    next_wolf: f32,
    next_inmate: f32,
    next_trickle: f32,
    next_crackle: f32,
}

impl Sound {
    pub async fn load(seed: u64) -> Sound {
        let mut r = Rng::new(seed ^ 0x50_0D);
        let rng = &mut r;
        Sound {
            carpet: clips(variants(4, rng, step_carpet)).await,
            stone: clips(variants(4, rng, step_stone)).await,
            leaves: clips(variants(4, rng, step_leaves)).await,
            gravel: clips(variants(4, rng, step_gravel)).await,
            exit: Clip::from_samples(&exit_chime(rng)).await,
            key_pickup: Clip::from_samples(&key_pickup()).await,
            key_turn: Clip::from_samples(&key_turn(rng)).await,
            door: Clip::from_samples(&door_open(rng)).await,
            office_door: Clip::from_samples(&office_door(rng)).await,
            closet: Clip::from_samples(&closet_sting(rng)).await,
            boss_voice: clips(variants(5, rng, boss_line)).await,
            lady_voice: clips(variants(5, rng, lady_line)).await,
            bot_voice: clips(variants(5, rng, bot_line)).await,
            busy: [
                clips(variants(3, rng, wc_busy_man)).await,
                clips(variants(3, rng, wc_busy_woman)).await,
            ],
            last_line: (usize::MAX, usize::MAX),
            victory: Clip::from_samples(&boss_defeated()).await,
            elevator: Clip::from_samples(&elevator(rng)).await,
            hum: Clip::from_samples(&office_hum(rng)).await,
            wind: Clip::from_samples(&forest_wind(rng)).await,
            buzz: clips(variants(3, rng, ballast_buzz)).await,
            owl: clips(variants(3, rng, owl)).await,
            caw: clips(variants(4, rng, caw)).await,
            wings: clips(variants(2, rng, wingbeats)).await,
            flaps: clips(variants(3, rng, flaps)).await,
            squeak: clips(variants(4, rng, squeak)).await,
            scurry: clips(variants(3, rng, scurry)).await,
            inmates: clips(variants(6, rng, inmate)).await,
            chains: clips(variants(3, rng, chain_rattle)).await,
            crickets: clips(variants(3, rng, cricket)).await,
            wolves: clips(variants(3, rng, wolf)).await,
            trickle: clips(variants(2, rng, trickle)).await,
            twigs: clips(variants(2, rng, twig_snap)).await,
            crackle: clips(variants(4, rng, torch_crackle)).await,
            garden: Clip::from_samples(&garden_bed(rng)).await,
            blackbird: clips(variants(3, rng, blackbird)).await,
            robin: clips(variants(3, rng, robin)).await,
            clock: Clip::from_samples(&clock_chime(rng)).await,
            watch: Clip::from_samples(&pocket_watch(rng)).await,
            bed: None,
            next_critter: rng.range(2.0, 5.0),
            next_wolf: rng.range(15.0, 35.0),
            next_inmate: rng.range(10.0, 25.0),
            next_trickle: 0.0,
            next_crackle: 0.0,
            rng: r,
        }
    }

    fn pick<'a>(rng: &mut Rng, set: &'a [Clip]) -> &'a Clip {
        &set[rng.index(set.len())]
    }

    pub fn footstep(&mut self, kind: ThemeKind) {
        let (set, vol) = match kind {
            ThemeKind::Backrooms => (&self.carpet, STEP_VOL[0]),
            ThemeKind::Dungeon => (&self.stone, STEP_VOL[1]),
            ThemeKind::Forest => (&self.leaves, STEP_VOL[2]),
            ThemeKind::Garden => (&self.gravel, STEP_VOL[3]),
        };
        let v = vol * self.rng.range(0.8, 1.1);
        Self::pick(&mut self.rng, set).play_once(v);
    }

    pub fn exit(&self) {
        self.exit.play_once(0.5);
    }

    pub fn key_pickup(&self) {
        self.key_pickup.play_once(0.45);
    }

    pub fn key_turn(&self) {
        self.key_turn.play_once(0.55);
    }

    pub fn door_open(&self) {
        self.door.play_once(0.6);
    }

    pub fn closet(&self) {
        self.closet.play_once(CLOSET_VOL);
    }

    /// One gibberish line of the Boss argument: the Boss's bluster or the bot's chip
    /// voice, never the same line twice running.
    /// `lady`: the boss is a woman (`Maze::lady_boss`).
    pub fn argue(&mut self, boss: bool, lady: bool) {
        let (set, last, vol) = if boss && lady {
            (&self.lady_voice, &mut self.last_line.0, BOSS_VOICE_VOL)
        } else if boss {
            (&self.boss_voice, &mut self.last_line.0, BOSS_VOICE_VOL)
        } else {
            (&self.bot_voice, &mut self.last_line.1, BOT_VOICE_VOL)
        };
        let mut i = self.rng.index(set.len());
        if i == *last {
            i = (i + 1) % set.len();
        }
        *last = i;
        set[i].play_once(vol);
    }

    pub fn boss_defeated(&self) {
        self.victory.play_once(VICTORY_VOL);
    }

    pub fn elevator(&self) {
        self.elevator.play_once(ELEVATOR_VOL);
    }

    /// The bot tried an occupied washroom's handle: it rattles, and someone inside
    /// (`mens`: behind the men's door) says so.
    pub fn busy(&mut self, mens: bool) {
        self.office_door.play_once(0.3);
        let set = &self.busy[usize::from(!mens)];
        Self::pick(&mut self.rng, set).play_once(BOSS_VOICE_VOL);
    }

    pub fn office_door(&self) {
        self.office_door.play_once(0.45);
    }

    /// Per frame: keep `kind`'s bed looping (stopped while muted — on native an already
    /// playing loop would otherwise outlive the mute) and fire the theme's random and
    /// lamp-driven one-shots. `dt` is wall-clock.
    pub fn ambience(&mut self, kind: ThemeKind, fx: &Fx, ear: &Ear, cam: Vec2, dt: f32) {
        let hear = |p: Vec2| ear.distance(p, cam);
        let muted = audio::playback::muted();
        let want = (!muted).then_some(kind);
        if want != self.bed {
            self.stop();
            match want {
                Some(ThemeKind::Backrooms) => self.hum.play_looped(HUM_VOL),
                Some(ThemeKind::Forest) => self.wind.play_looped(WIND_VOL),
                Some(ThemeKind::Garden) => self.garden.play_looped(GARDEN_VOL),
                _ => {}
            }
            self.bed = want;
        }
        if muted {
            return;
        }
        match kind {
            ThemeKind::Backrooms => {
                for d in fx.blinked_on(hear) {
                    let v = 0.18 * (1.0 - d / BUZZ_RANGE).max(0.0).powi(2);
                    if v > 0.01 {
                        Self::pick(&mut self.rng, &self.buzz).play_once(v);
                    }
                }
            }
            ThemeKind::Dungeon => {
                self.next_crackle -= dt;
                if self.next_crackle <= 0.0 {
                    self.next_crackle = self.rng.range(0.3, 0.45);
                    let v = TORCH_VOL * fx.torch_level(hear);
                    if v > 0.01 {
                        Self::pick(&mut self.rng, &self.crackle).play_once(v);
                    }
                }
                self.next_inmate -= dt;
                if self.next_inmate <= 0.0 {
                    self.next_inmate = self.rng.range(INMATE_EVERY.0, INMATE_EVERY.1);
                    let v = self.rng.range(INMATE_VOL.0, INMATE_VOL.1);
                    Self::pick(&mut self.rng, &self.inmates).play_once(v);
                }
            }
            ThemeKind::Forest => {
                self.next_wolf -= dt;
                if self.next_wolf <= 0.0 {
                    self.next_wolf = self.rng.range(35.0, 80.0);
                    let v = self.rng.range(WOLF_VOL.0, WOLF_VOL.1);
                    Self::pick(&mut self.rng, &self.wolves).play_once(v);
                }
                self.next_critter -= dt;
                if self.next_critter <= 0.0 {
                    self.next_critter = self.rng.range(2.0, 6.0);
                    if self.rng.next_f32() < 0.8 {
                        let v = self.rng.range(CRICKET_VOL.0, CRICKET_VOL.1);
                        Self::pick(&mut self.rng, &self.crickets).play_once(v);
                    } else {
                        let v = self.rng.range(TWIG_VOL.0, TWIG_VOL.1);
                        Self::pick(&mut self.rng, &self.twigs).play_once(v);
                    }
                }
            }
            ThemeKind::Garden => {
                self.next_critter -= dt;
                if self.next_critter <= 0.0 {
                    self.next_critter = self.rng.range(5.0, 12.0);
                    let r = self.rng.next_f32();
                    let song = self.rng.range(SONGBIRD_VOL.0, SONGBIRD_VOL.1);
                    if r < 0.45 {
                        Self::pick(&mut self.rng, &self.blackbird).play_once(song);
                    } else if r < 0.85 {
                        Self::pick(&mut self.rng, &self.robin).play_once(song);
                    } else if r < 0.93 {
                        self.watch.play_once(WATCH_VOL);
                    } else {
                        self.clock.play_once(CLOCK_VOL);
                    }
                }
            }
        }
    }

    /// Garden: the fountain's trickle, back to back while it's in earshot, at a volume
    /// from `ear`'s walking distance to it (no live volume on WASM, so each 1.8 s copy
    /// picks its level as it starts; copies overlap by their fades).
    pub fn fountain(&mut self, at: Option<Vec2>, ear: &Ear, cam: Vec2, dt: f32) {
        self.next_trickle -= dt;
        let Some(at) = at else {
            return;
        };
        if self.next_trickle > 0.0 || audio::playback::muted() {
            return;
        }
        self.next_trickle = 1.72;
        let v = FOUNTAIN_VOL
            * (1.0 - ear.distance(at, cam) / FOUNTAIN_RANGE)
                .max(0.0)
                .powi(2);
        if v > 0.004 {
            Self::pick(&mut self.rng, &self.trickle).play_once(v);
        }
    }

    /// A critter's call (`view::critters`), `dist` world units from the bot. Straight-line
    /// distance, not `Ear`'s walking route: birds call from the treetops, and a rat in
    /// the next corridor carries through the stone well enough.
    pub fn bird(&mut self, call: Call, dist: f32) {
        if audio::playback::muted() {
            return;
        }
        let (set, vol, range) = match call {
            Call::Caw => (&self.caw, CAW_VOL, BIRD_RANGE),
            Call::Takeoff => (&self.wings, WINGS_VOL, BIRD_RANGE),
            Call::Flap => (&self.flaps, FLAP_VOL, BIRD_RANGE),
            Call::Hoot => (&self.owl, HOOT_VOL, BIRD_RANGE),
            Call::Squeak => (&self.squeak, SQUEAK_VOL, RAT_RANGE),
            Call::Scurry => (&self.scurry, SCURRY_VOL, RAT_RANGE),
        };
        let v = vol * (1.0 - dist / range).max(0.0).powf(1.5);
        if v > 0.005 {
            Self::pick(&mut self.rng, set).play_once(v);
        }
    }

    /// Dungeon: the bot's footfall beside a chain (`Decor::chain_near`) — it brushes
    /// the links more often than not.
    pub fn chain(&mut self) {
        if audio::playback::muted() || self.rng.next_f32() > 0.7 {
            return;
        }
        let v = CHAIN_VOL * self.rng.range(0.7, 1.0);
        Self::pick(&mut self.rng, &self.chains).play_once(v);
    }

    /// Silence the looping bed (leaving the game, muting, switching theme).
    pub fn stop(&mut self) {
        self.hum.stop();
        self.wind.stop();
        self.garden.stop();
        self.bed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_one_shots() -> Vec<(&'static str, Vec<f32>)> {
        let rng = &mut Rng::new(1);
        vec![
            ("carpet", step_carpet(rng)),
            ("stone", step_stone(rng)),
            ("leaves", step_leaves(rng)),
            ("gravel", step_gravel(rng)),
            ("exit", exit_chime(rng)),
            ("key_pickup", key_pickup()),
            ("key_turn", key_turn(rng)),
            ("door", door_open(rng)),
            ("office_door", office_door(rng)),
            ("closet", closet_sting(rng)),
            ("boss_line_0", boss_line(rng)),
            ("lady_line_0", lady_line(rng)),
            ("boss_line_1", boss_line(rng)),
            ("boss_line_2", boss_line(rng)),
            ("boss_line_3", boss_line(rng)),
            ("bot_line_0", bot_line(rng)),
            ("bot_line_1", bot_line(rng)),
            ("bot_line_2", bot_line(rng)),
            ("bot_line_3", bot_line(rng)),
            ("boss_defeated", boss_defeated()),
            ("elevator", elevator(rng)),
            ("buzz", ballast_buzz(rng)),
            ("owl", owl(rng)),
            ("caw", caw(rng)),
            ("wingbeats", wingbeats(rng)),
            ("flaps", flaps(rng)),
            ("squeak", squeak(rng)),
            ("scurry", scurry(rng)),
            ("cricket", cricket(rng)),
            ("wolf", wolf(rng)),
            ("trickle", trickle(rng)),
            ("twig", twig_snap(rng)),
            ("crackle", torch_crackle(rng)),
            ("blackbird", blackbird(rng)),
            ("robin", robin(rng)),
            ("clock", clock_chime(rng)),
            ("pocket_watch", pocket_watch(rng)),
            ("wc_busy_man", wc_busy_man(rng)),
            ("wc_busy_woman", wc_busy_woman(rng)),
        ]
    }

    /// Audition aid, not a check: writes every clip to `tmp/labyrinth_sounds/` at the
    /// volume the game plays it, plus a per-theme scene (bed + walking steps + that
    /// theme's one-shots) to hear the balance. Run:
    /// `cargo test -p labyrinth --release dump_wavs -- --ignored`
    #[test]
    #[ignore]
    fn dump_wavs() {
        let dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/labyrinth_sounds");
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, b: &[f32]| {
            let bytes = audio::wav::encode_mono_wav(b, SAMPLE_RATE);
            std::fs::write(dir.join(format!("{name}.wav")), bytes).unwrap();
        };
        for (name, b) in all_one_shots() {
            write(name, &b);
        }
        let rng = &mut Rng::new(3);
        let hum = office_hum(rng);
        let wind = forest_wind(rng);
        let garden = garden_bed(rng);
        write("bed_hum", &hum.repeat(3));
        write("bed_wind", &wind.repeat(2));
        write("bed_garden", &garden.repeat(2));
        write("groan", &groan(rng));
        write("sob", &sob(rng));
        write("wail", &wail(rng));
        write("chain", &chain_rattle(rng));
        // Scenes: 8 s at in-game volumes, two footfalls per cell walked (0.28 s apart,
        // then the tick dwell).
        let scene = |bed: Option<(&[f32], f32)>,
                     step: fn(&mut Rng) -> Vec<f32>,
                     step_vol: f32,
                     extra: &mut dyn FnMut(&mut Vec<f32>, &mut Rng)| {
            let rng = &mut Rng::new(4);
            let mut out = vec![0.0; secs(8.0)];
            if let Some((b, v)) = bed {
                for (i, o) in out.iter_mut().enumerate() {
                    *o += b[i % b.len()] * v;
                }
            }
            let mut at = 0.3;
            for i in 0.. {
                if at >= 7.5 {
                    break;
                }
                add_at(&mut out, &step(rng), at, step_vol * rng.range(0.8, 1.1));
                at += if i % 2 == 0 { 0.28 } else { 0.33 };
            }
            extra(&mut out, rng);
            out
        };
        write(
            "scene_backrooms",
            &scene(
                Some((&hum, HUM_VOL)),
                step_carpet,
                STEP_VOL[0],
                &mut |o, r| {
                    add_at(o, &ballast_buzz(r), 3.1, 0.12);
                },
            ),
        );
        write(
            "scene_dungeon",
            &scene(None, step_stone, STEP_VOL[1], &mut |o, r| {
                let mut at = 0.0;
                while at < 7.5 {
                    add_at(o, &torch_crackle(r), at, TORCH_VOL * 0.6);
                    at += r.range(0.3, 0.45);
                }
            }),
        );
        write(
            "scene_forest",
            &scene(
                Some((&wind, WIND_VOL)),
                step_leaves,
                STEP_VOL[2],
                &mut |o, r| {
                    add_at(o, &owl(r), 1.5, 0.12);
                    add_at(o, &caw(r), 4.6, CAW_VOL * 0.5);
                    add_at(o, &wingbeats(r), 5.5, WINGS_VOL * 0.5);
                    add_at(o, &cricket(r), 3.6, CRICKET_VOL.1);
                    add_at(o, &wolf(r), 0.5, WOLF_VOL.1);
                    add_at(o, &twig_snap(r), 6.1, TWIG_VOL.1);
                },
            ),
        );
        // The argument: lines alternating every 0.8 s, then the fanfare and the lift.
        let mut argument = vec![0.0; secs(10.0)];
        for i in 0..10 {
            let (line, v) = if i % 2 == 0 {
                (boss_line(rng), BOSS_VOICE_VOL)
            } else {
                (bot_line(rng), BOT_VOICE_VOL)
            };
            add_at(&mut argument, &line, i as f32 * 0.8, v);
        }
        add_at(&mut argument, &boss_defeated(), 8.1, VICTORY_VOL);
        write("scene_argument", &argument);
        write(
            "scene_garden",
            &scene(
                Some((&garden, GARDEN_VOL)),
                step_gravel,
                STEP_VOL[3],
                &mut |o, r| {
                    add_at(o, &blackbird(r), 0.8, SONGBIRD_VOL.1);
                    add_at(o, &robin(r), 4.3, SONGBIRD_VOL.0);
                    add_at(o, &pocket_watch(r), 3.5, WATCH_VOL);
                    add_at(o, &clock_chime(r), 6.2, CLOCK_VOL);
                },
            ),
        );
    }

    /// A source on the far side of a wall is as far away as the corridor route round
    /// to it — never heard through the wall.
    #[test]
    fn ear_goes_round_walls_not_through() {
        let m = {
            let _g = crate::rng_guard();
            macroquad::rand::srand(5);
            crate::maze::generate(15, 11, 0.0, 0, 3)
        };
        let mut ear = Ear::default();
        let bot = m.start;
        ear.update(&m, bot);
        let centre = |c: Cell| Vec2::new(2.0 * c.x as f32 + 1.5, 2.0 * c.y as f32 + 1.5);
        let mut checked = 0;
        for y in 0..m.h as i32 {
            for x in 0..m.w as i32 {
                let c = Cell { x, y };
                let path = ear.dist[m.idx(c)];
                let straight = centre(c).distance(centre(bot));
                let heard = ear.distance(centre(c), centre(bot));
                if path == u32::MAX {
                    assert!(heard.is_infinite(), "{c:?} heard through sealed walls");
                } else {
                    assert!(heard >= path as f32 * 2.0 && heard >= straight - 1e-3);
                    if path as f32 * 2.0 > straight + 4.0 {
                        checked += 1;
                    }
                }
            }
        }
        assert!(
            checked > 0,
            "no cell whose route is longer than the straight line"
        );
    }

    #[test]
    fn clips_are_bounded_and_audible() {
        for (name, b) in all_one_shots() {
            assert!(!b.is_empty(), "{name} empty");
            assert!(
                b.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
                "{name} out of range"
            );
            assert!(b.iter().any(|s| s.abs() > 0.3), "{name} silent");
            // A wolf's howl is a long call by nature (3-5 s, plus the forest's echo) and
            // plays once a minute at most; a crow's run of 3-5 caws with its echo can pass
            // 2 s too. Everything else stays short.
            let max = match name {
                "wolf" => 8.0,
                "caw" => 3.5,
                _ => 2.0,
            };
            assert!(b.len() < secs(max), "{name} too long for a one-shot");
        }
    }

    /// A looping bed must not click at the seam: the jump from the last sample back to
    /// the first is no bigger than the sample-to-sample motion inside the loop.
    #[test]
    fn beds_loop_without_a_click() {
        let rng = &mut Rng::new(2);
        for (name, b) in [
            ("hum", office_hum(rng)),
            ("wind", forest_wind(rng)),
            ("garden", garden_bed(rng)),
        ] {
            let seam = (b[0] - b[b.len() - 1]).abs();
            let max_step = b
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max);
            assert!(
                seam <= max_step,
                "{name}: seam jump {seam} > max step {max_step}"
            );
        }
    }
}
