//! Asteroids' procedural sound, on `lib/audio`. Every clip is synthesized once at
//! startup (`Sfx::load`); nothing is loaded from disk.
//!
//! Two trigger paths:
//! - `on_events` — one-shots off the sim's `Event`s (fire, explosions,
//!   hyperspace). Called once per sim step, and a fast-forwarded frame runs many steps, so each
//!   sound plays at most once per *frame* (`played` bitmask, cleared by `update`): ten
//!   small rocks shattering in one frame at 10x is one crack, not a clipped pile-up.
//! - `update` — once per frame, owns everything derived from state rather than events:
//!   the thrust rumble and saucer warble loops, and the two-note heartbeat. The heartbeat
//!   is scheduled off `game.tick` (so it speeds up with game speed, like everything else
//!   on screen), and its interval shrinks as the wave's remaining rock "hits" drop.
//!
//! Frequent sounds (fire, rock explosions, heartbeat) are noise/thumps; nothing is
//! melodic. All randomness is `audio::Rng` — never `macroquad::rand`, which
//! would perturb the seeded gameplay stream.

use crate::game::{Event, Game, Outcome, RockSize, SaucerSize};
use audio::filter::{one_pole_highpass, one_pole_lowpass};
use audio::{Clip, Rng, SAMPLE_RATE};
use std::cell::Cell;
use std::f32::consts::TAU;

const SR: f32 = SAMPLE_RATE as f32;

/// Heartbeat interval bounds, in sim ticks (60/s at 1x): slowest with a full wave of
/// large rocks, fastest on the last few smalls.
const BEAT_SLOW: u64 = 60;
const BEAT_FAST: u64 = 14;
/// Remaining "hits" (large = 7: itself + 2 mediums + 4 smalls) at or above which the beat
/// is at its slowest — the opening four-rock wave.
const BEAT_FULL_HITS: u32 = 28;
/// Background pulse, not a lead: it plays continuously all wave, so it sits well under
/// the one-shots (0.5 was judged too present by ear).
const BEAT_VOLUME: f32 = 0.22;
/// Wall-clock floor between beats: at high game speed the tick schedule would otherwise
/// turn the beat into a buzz.
const BEAT_MIN_GAP_SECS: f64 = 0.16;

/// Frames the thrust loop keeps playing after thrust stops.
const THRUST_HOLD_FRAMES: u32 = 8;

fn secs(n: f32) -> usize {
    (n * SR) as usize
}

fn noise(n: usize, rng: &mut Rng) -> Vec<f32> {
    (0..n).map(|_| rng.range(-1.0, 1.0)).collect()
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

/// Noise with a fast attack and exponential decay (`tau` s), band-limited `lo..hi` Hz.
fn burst(len: f32, tau: f32, lo: f32, hi: f32, rng: &mut Rng) -> Vec<f32> {
    let mut b = noise(secs(len), rng);
    for (i, s) in b.iter_mut().enumerate() {
        let t = i as f32 / SR;
        *s *= (t / 0.002).min(1.0) * (-t / tau).exp();
    }
    one_pole_highpass(&mut b, lo, SAMPLE_RATE);
    one_pole_lowpass(&mut b, hi, SAMPLE_RATE);
    one_pole_lowpass(&mut b, hi, SAMPLE_RATE);
    b
}

/// Sine sweeping `f0 -> f1` Hz (exponentially), decay `tau`, soft-clipped by `drive` so
/// a low fundamental still has harmonics small speakers can reproduce.
fn thump(len: f32, f0: f32, f1: f32, tau: f32, drive: f32) -> Vec<f32> {
    let n = secs(len);
    let mut ph = 0.0f32;
    (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let f = f0 * (f1 / f0).powf(t / len);
            ph += f / SR;
            let s = ((ph * TAU).sin() * drive).tanh();
            s * (t / 0.003).min(1.0) * (-t / tau).exp()
        })
        .collect()
}

/// Square wave sweeping `f0 -> f1` Hz exponentially, decay `tau`, lowpassed at `lp`.
fn zap(len: f32, f0: f32, f1: f32, tau: f32, lp: f32) -> Vec<f32> {
    let n = secs(len);
    let mut ph = 0.0f32;
    let mut b: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let f = f0 * (f1 / f0).powf(t / len);
            ph = (ph + f / SR).fract();
            let s = if ph < 0.5 { 1.0 } else { -1.0 };
            s * (t / 0.002).min(1.0) * (-t / tau).exp()
        })
        .collect();
    one_pole_lowpass(&mut b, lp, SAMPLE_RATE);
    b
}

/// Scale to exactly `peak` and fade the last few ms to zero (no end click).
fn finish(mut b: Vec<f32>, peak: f32) -> Vec<f32> {
    let p = b.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if p > 0.0 {
        b.iter_mut().for_each(|s| *s *= peak / p);
    }
    let fade = secs(0.004).min(b.len());
    let n = b.len();
    for (k, s) in b[n - fade..].iter_mut().enumerate() {
        *s *= 1.0 - k as f32 / fade as f32;
    }
    b
}

/// Make `b` loop seamlessly: crossfade its last `fade` seconds into its head, then drop
/// the tail. Assumes stationary content (noise beds).
fn make_loop(mut b: Vec<f32>, fade: f32) -> Vec<f32> {
    let f = secs(fade);
    let n = b.len() - f;
    for k in 0..f {
        let w = k as f32 / f as f32;
        b[k] = b[k] * w + b[n + k] * (1.0 - w);
    }
    b.truncate(n);
    b
}

fn fire_clip(rng: &mut Rng) -> Vec<f32> {
    // Short falling "pew" with a breath of noise on the attack — soft, it's constant.
    let mut b = zap(0.09, 1500.0, 380.0, 0.035, 3500.0);
    add_at(&mut b, &burst(0.03, 0.008, 800.0, 6000.0, rng), 0.0, 0.5);
    finish(b, 0.8)
}

fn rock_clip(size: RockSize, rng: &mut Rng) -> Vec<f32> {
    // Larger = longer, darker, with more low-end body.
    let (len, tau, lp, body_f, body_g) = match size {
        RockSize::Large => (0.75, 0.22, 900.0, 85.0, 0.7),
        RockSize::Medium => (0.5, 0.13, 1600.0, 120.0, 0.45),
        RockSize::Small => (0.28, 0.06, 2800.0, 170.0, 0.25),
    };
    let mut out = finish(burst(len, tau, 40.0, lp, rng), 1.0);
    add_at(
        &mut out,
        &thump(len * 0.6, body_f, body_f * 0.5, tau * 0.6, 1.5),
        0.0,
        body_g,
    );
    finish(out, 0.9)
}

fn ship_clip(rng: &mut Rng) -> Vec<f32> {
    // Long rumble, a low drop underneath and a scatter of crackles in the tail.
    let mut b = burst(1.4, 0.45, 30.0, 700.0, rng);
    add_at(&mut b, &thump(0.8, 90.0, 28.0, 0.3, 2.0), 0.0, 0.25);
    for _ in 0..10 {
        let at = rng.range(0.05, 0.8);
        let c = burst(0.04, 0.01, 1500.0, 5000.0, rng);
        add_at(&mut b, &c, at, rng.range(0.03, 0.08));
    }
    finish(b, 0.95)
}

fn saucer_boom_clip(rng: &mut Rng) -> Vec<f32> {
    // Mid-weight burst plus a falling electronic whine — "the machine died".
    let mut b = burst(0.7, 0.18, 60.0, 1800.0, rng);
    add_at(&mut b, &zap(0.5, 900.0, 120.0, 0.2, 2200.0), 0.0, 0.08);
    finish(b, 0.9)
}

fn saucer_fire_clip(rng: &mut Rng) -> Vec<f32> {
    // Lower, squarer and blunter than the ship's pew, so they read as different guns.
    let mut b = zap(0.11, 700.0, 260.0, 0.045, 2200.0);
    add_at(&mut b, &burst(0.03, 0.01, 300.0, 3000.0, rng), 0.0, 0.3);
    finish(b, 0.8)
}

fn hyper_clip(rising: bool, rng: &mut Rng) -> Vec<f32> {
    // A filtered sine sweep with airy noise; out rises (vanishing), in falls (arriving).
    let len = 0.28;
    let (f0, f1) = if rising {
        (180.0f32, 1400.0f32)
    } else {
        (1400.0, 180.0)
    };
    let n = secs(len);
    let mut ph = 0.0f32;
    let mut b: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let x = t / len;
            let f = f0 * (f1 / f0).powf(x);
            ph += f / SR;
            let env = (t / 0.01).min(1.0) * (1.0 - x).powf(0.7);
            ((ph * TAU).sin() + 0.3 * (ph * TAU * 2.0).sin()) * env
        })
        .collect();
    let mut air = noise(n, rng);
    one_pole_highpass(&mut air, 2000.0, SAMPLE_RATE);
    for (i, s) in air.iter_mut().enumerate() {
        *s *= 0.15 * (1.0 - i as f32 / n as f32);
    }
    add_at(&mut b, &air, 0.0, 1.0);
    finish(b, 0.8)
}

fn beat_clip(f: f32) -> Vec<f32> {
    // Low, short, slightly driven thump.
    let b = thump(0.16, f * 1.15, f, 0.05, 2.5);
    finish(b, 0.9)
}

fn thrust_loop(rng: &mut Rng) -> Vec<f32> {
    // Low rumbling noise with a slow wobble, looped.
    let len = 0.8;
    let fade = 0.1;
    let mut b = noise(secs(len + fade), rng);
    // Kept up in the few-hundred-Hz-to-1kHz band small speakers actually reproduce; a
    // 260/400Hz lowpass made it a sub-bass rumble that was inaudible on laptops.
    one_pole_lowpass(&mut b, 1100.0, SAMPLE_RATE);
    one_pole_lowpass(&mut b, 1400.0, SAMPLE_RATE);
    one_pole_highpass(&mut b, 120.0, SAMPLE_RATE);
    let b = make_loop(b, fade);
    // Two full wobble cycles per loop so the modulation is seamless too.
    let n = b.len();
    let b: Vec<f32> = b
        .into_iter()
        .enumerate()
        .map(|(i, s)| s * (0.8 + 0.2 * (TAU * 2.0 * i as f32 / n as f32).sin()))
        .collect();
    let p = b.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    b.into_iter().map(|s| s * 0.9 / p).collect()
}

fn saucer_loop(size: SaucerSize) -> Vec<f32> {
    // Classic siren warble: a square-ish tone whose pitch swings on an LFO. The large
    // saucer is lower and slower, the small one higher and nervier. Loop length holds
    // whole LFO cycles and a whole number of carrier cycles (the sine modulation
    // integrates to zero over a full cycle), so the loop point is phase-continuous.
    let (base, depth, lfo_hz, cycles) = match size {
        SaucerSize::Large => (300.0f32, 0.18f32, 5.0f32, 4u32),
        SaucerSize::Small => (620.0, 0.14, 9.0, 8),
    };
    let len = cycles as f32 / lfo_hz;
    let n = secs(len);
    // Snap base so base * len is an integer number of carrier cycles.
    let base = (base * len).round() / len;
    let mut ph = 0.0f32;
    let mut b: Vec<f32> = (0..n)
        .map(|i| {
            let t = i as f32 / SR;
            let f = base * (1.0 + depth * (TAU * lfo_hz * t).sin());
            ph = (ph + f / SR).fract();
            ((ph * TAU).sin() * 3.0).tanh()
        })
        .collect();
    one_pole_lowpass(&mut b, 2500.0, SAMPLE_RATE);
    let p = b.iter().fold(0.0f32, |m, s| m.max(s.abs())).max(1e-6);
    b.iter_mut().for_each(|s| *s *= 0.9 / p);
    b
}

/// Bit positions in `Sfx::played`.
#[derive(Clone, Copy)]
enum Slot {
    Fire,
    RockLarge,
    RockMedium,
    RockSmall,
    Ship,
    SaucerBoom,
    SaucerFire,
    HyperOut,
    HyperIn,
}

pub struct Sfx {
    fire: Clip,
    rock_large: Clip,
    rock_medium: Clip,
    rock_small: Clip,
    ship: Clip,
    saucer_boom: Clip,
    saucer_fire: Clip,
    hyper_out: Clip,
    hyper_in: Clip,
    beat_hi: Clip,
    beat_lo: Clip,
    thrust: Clip,
    saucer_large: Clip,
    saucer_small: Clip,

    /// One-shot slots already played this frame (bit per `Slot`).
    played: Cell<u16>,
    thrust_on: Cell<bool>,
    thrust_hold: Cell<u32>,
    saucer_on: Cell<Option<SaucerSize>>,
    /// Tick at which the next heartbeat is due; `None` while silent (between waves).
    next_beat: Cell<Option<u64>>,
    beat_low: Cell<bool>,
    last_tick: Cell<u64>,
    last_beat_wall: Cell<f64>,
}

impl Sfx {
    pub async fn load() -> Self {
        let mut rng = Rng::new(0xA57E_201D);
        let r = &mut rng;
        Sfx {
            fire: Clip::from_samples(&fire_clip(r)).await,
            rock_large: Clip::from_samples(&rock_clip(RockSize::Large, r)).await,
            rock_medium: Clip::from_samples(&rock_clip(RockSize::Medium, r)).await,
            rock_small: Clip::from_samples(&rock_clip(RockSize::Small, r)).await,
            ship: Clip::from_samples(&ship_clip(r)).await,
            saucer_boom: Clip::from_samples(&saucer_boom_clip(r)).await,
            saucer_fire: Clip::from_samples(&saucer_fire_clip(r)).await,
            hyper_out: Clip::from_samples(&hyper_clip(true, r)).await,
            hyper_in: Clip::from_samples(&hyper_clip(false, r)).await,
            beat_hi: Clip::from_samples(&beat_clip(62.0)).await,
            beat_lo: Clip::from_samples(&beat_clip(55.0)).await,
            thrust: Clip::from_samples(&thrust_loop(r)).await,
            saucer_large: Clip::from_samples(&saucer_loop(SaucerSize::Large)).await,
            saucer_small: Clip::from_samples(&saucer_loop(SaucerSize::Small)).await,
            played: Cell::new(0),
            thrust_on: Cell::new(false),
            thrust_hold: Cell::new(0),
            saucer_on: Cell::new(None),
            next_beat: Cell::new(None),
            beat_low: Cell::new(false),
            last_tick: Cell::new(0),
            last_beat_wall: Cell::new(f64::NEG_INFINITY),
        }
    }

    /// Plays `clip` unless its slot already sounded this frame.
    fn once(&self, slot: Slot, clip: &Clip, volume: f32) {
        let bit = 1u16 << slot as u16;
        let played = self.played.get();
        if played & bit != 0 {
            return;
        }
        self.played.set(played | bit);
        clip.play_once(volume);
    }

    pub fn on_events(&self, events: &[Event]) {
        // A ship ramming a rock emits both; the ship explosion covers the rock's.
        let ship_died = events
            .iter()
            .any(|e| matches!(e, Event::ShipDestroyed { .. }));
        for e in events {
            match *e {
                Event::Fire => self.once(Slot::Fire, &self.fire, 0.18),
                Event::RockDestroyed { size, .. } if !ship_died => match size {
                    RockSize::Large => self.once(Slot::RockLarge, &self.rock_large, 0.55),
                    RockSize::Medium => self.once(Slot::RockMedium, &self.rock_medium, 0.45),
                    RockSize::Small => self.once(Slot::RockSmall, &self.rock_small, 0.38),
                },
                Event::RockDestroyed { .. } => {}
                Event::SaucerDestroyed { size, .. } => {
                    // The small saucer is the rarer, bigger prize: a touch louder.
                    let vol = match size {
                        SaucerSize::Large => 0.55,
                        SaucerSize::Small => 0.7,
                    };
                    self.once(Slot::SaucerBoom, &self.saucer_boom, vol)
                }
                Event::SaucerFire => self.once(Slot::SaucerFire, &self.saucer_fire, 0.2),
                Event::ShipDestroyed { .. } => self.once(Slot::Ship, &self.ship, 0.75),
                Event::HyperspaceOut { .. } => self.once(Slot::HyperOut, &self.hyper_out, 0.35),
                Event::HyperspaceIn { .. } if !ship_died => {
                    self.once(Slot::HyperIn, &self.hyper_in, 0.35)
                }
                Event::HyperspaceIn { .. } => {}
                Event::WaveStart | Event::Respawn => {}
            }
        }
    }

    pub fn update(&self, game: &Game, running: bool) {
        self.played.set(0);
        let live = running && game.outcome() == Outcome::Running;

        // Thrust rumble. The bot's thrust flickers as its heading drifts in and out of its
        // thrust cone; hold the loop through short gaps rather than restarting it.
        let held = if game.ship.thrusting {
            THRUST_HOLD_FRAMES
        } else {
            self.thrust_hold.get().saturating_sub(1)
        };
        self.thrust_hold.set(held);
        let thrust = live && held > 0 && game.ship.present();
        if thrust != self.thrust_on.get() {
            if thrust {
                self.thrust.play_looped(0.3);
            } else {
                self.thrust.stop();
            }
            self.thrust_on.set(thrust);
        }

        // Saucer warble.
        let saucer = if live {
            game.saucer.as_ref().map(|s| s.size)
        } else {
            None
        };
        if saucer != self.saucer_on.get() {
            if let Some(old) = self.saucer_on.get() {
                self.saucer_clip(old).stop();
            }
            if let Some(new) = saucer {
                self.saucer_clip(new).play_looped(0.16);
            }
            self.saucer_on.set(saucer);
        }

        // Heartbeat.
        if game.tick < self.last_tick.get() {
            // New game.
            self.next_beat.set(None);
        }
        self.last_tick.set(game.tick);
        if !running {
            return;
        }
        if !live || game.rocks.is_empty() {
            self.next_beat.set(None);
            return;
        }
        let hits: u32 = game
            .rocks
            .iter()
            .map(|r| match r.size {
                RockSize::Large => 7,
                RockSize::Medium => 3,
                RockSize::Small => 1,
            })
            .sum();
        let k = (hits.min(BEAT_FULL_HITS) as f32 / BEAT_FULL_HITS as f32).sqrt();
        let interval = BEAT_FAST + ((BEAT_SLOW - BEAT_FAST) as f32 * k) as u64;
        match self.next_beat.get() {
            None => self.next_beat.set(Some(game.tick + interval)),
            Some(due) if game.tick >= due => {
                let now = macroquad::miniquad::date::now();
                if now - self.last_beat_wall.get() >= BEAT_MIN_GAP_SECS {
                    let low = self.beat_low.get();
                    if low {
                        self.beat_lo.play_once(BEAT_VOLUME);
                    } else {
                        self.beat_hi.play_once(BEAT_VOLUME);
                    }
                    self.beat_low.set(!low);
                    self.last_beat_wall.set(now);
                }
                self.next_beat.set(Some(game.tick + interval));
            }
            Some(_) => {}
        }
    }

    fn saucer_clip(&self, size: SaucerSize) -> &Clip {
        match size {
            SaucerSize::Large => &self.saucer_large,
            SaucerSize::Small => &self.saucer_small,
        }
    }
}
