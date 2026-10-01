//! Water Sort's procedural sound, on `lib/audio`: the pour's bubbling glug, the glass
//! set back down, a bottle sorting, a lock releasing, and the level's end.
//!
//! Every clip is synthesized once at startup (`Sound::load`) and triggered off what the
//! viewer sees happen — the stream starting to fall, the bottle landing back in its
//! slot — not off `Game::apply`, which runs a whole animation earlier.
//!
//! All randomness is `audio::Rng`, never `macroquad::rand` — drawing from the gameplay
//! stream would break `HCG_SEED` replay.

use crate::game::{CAPACITY, Color};
use audio::filter::{one_pole_highpass, one_pole_lowpass};
use audio::mix::normalize_peak;
use audio::pitch::note_freq;
use audio::{Clip, Rng, SAMPLE_RATE};
use std::f32::consts::{PI, TAU};

const SR: f32 = SAMPLE_RATE as f32;

fn secs(n: f32) -> usize {
    (n * SR) as usize
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

/// A decaying sine partial.
fn ring(len: f32, f: f32, tau: f32) -> Vec<f32> {
    (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            (t * f * TAU).sin() * (t / 0.002).min(1.0) * (-t / tau).exp()
        })
        .collect()
}

/// Struck glass: a fundamental plus the inharmonic upper modes that make it read as
/// glass rather than a sine beep. Upper modes die faster, as they do on a real glass.
fn glass(f: f32, tau: f32) -> Vec<f32> {
    let len = tau * 6.0;
    let mut out = ring(len, f, tau);
    add_at(&mut out, &ring(len, f * 2.76, tau * 0.5), 0.0, 0.35);
    add_at(&mut out, &ring(len, f * 5.40, tau * 0.25), 0.0, 0.15);
    out
}

/// Noise shaped by a fast-attack exponential decay (`tau` seconds), band-limited.
fn burst(len: f32, tau: f32, lo: f32, hi: f32, rng: &mut Rng) -> Vec<f32> {
    let mut b: Vec<f32> = (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            rng.range(-1.0, 1.0) * (t / 0.001).min(1.0) * (-t / tau).exp()
        })
        .collect();
    one_pole_highpass(&mut b, lo, SAMPLE_RATE);
    one_pole_lowpass(&mut b, hi, SAMPLE_RATE);
    b
}

/// Pitch of the air column above `fill` units: shortens as the bottle fills, so the
/// pour's resonance rises — the sound that tells you a bottle is filling up without
/// looking. Range matches a real mug filling (~500 Hz -> ~1 kHz).
fn column_freq(fill: f32) -> f32 {
    450.0 * 7.0 / (7.0 - fill)
}

/// A pour is a stochastic *population* of bubbles (van den Doel, "Physically-based
/// models for liquid sounds", 2005): Poisson timing modulated by a slow stream wobble,
/// a power-law size spread (most bubbles tiny, quiet and high; a few larger and lower),
/// each a short up-chirping sine with a smooth onset — never an instant one, which is
/// what made a recording-matched spray read as static. The bottle's rising air-column
/// pitch shifts the population's centre and colours it through a quiet resonator,
/// rather than every bubble being locked to it (which read as cartoon).
///
/// Auditioned variants that lost to this one: slower clustered swells (multi-bubble
/// impacts, closer to a real pour's cross-band co-modulation) and sparse 110-260 Hz
/// "body" bubbles (closer low-end level) — both measured nearer a reference recording,
/// both sounded worse.
mod pour {
    /// Mean bubbles per second.
    pub const RATE: f32 = 160.0;
    /// Bubble pitch range at an empty bottle, Hz.
    pub const FREQ: (f32, f32) = (250.0, 4000.0);
    /// Skew toward the high (small-bubble) end: `u^skew` on the log-pitch axis, <1 = more small.
    pub const SKEW: f32 = 0.7;
    /// Multiplier on van den Doel's physical damping time — >1 rings longer (more tonal).
    pub const RING: f32 = 2.0;
    /// Pitch chirp per decay time constant.
    pub const RISE: f32 = 0.1;
    /// Depth of the slow stream-thickness wobble (0..1).
    pub const WOBBLE: f32 = 0.8;
    /// Wobble knot spacing, seconds — the length of one swell.
    pub const KNOT: f32 = 0.025;
    /// Level of the air-column resonance driven by the bubbles.
    pub const COLUMN: f32 = 0.25;
    /// Level of a soft, heavily lowpassed water wash under the bubbles.
    pub const WASH: f32 = 0.12;
}

/// One Minnaert-style bubble: a sine at `f` whose pitch climbs by `rise` per `tau` as
/// the bubble rises, dying away exponentially. A raised-cosine onset (0.8-3 ms,
/// scaled to the bubble) and a fade over the last `tau` keep both ends click-free.
fn bubble(f: f32, tau: f32, rise: f32) -> Vec<f32> {
    let attack = (tau * 0.6).clamp(0.0008, 0.003);
    let len = attack + tau * 6.0;
    let mut ph = 0.0f32;
    (0..secs(len))
        .map(|i| {
            let t = i as f32 / SR;
            ph += f * (1.0 + rise * t / tau) / SR;
            let a = if t < attack {
                0.5 - 0.5 * (t / attack * PI).cos()
            } else {
                (-(t - attack) / tau).exp()
            };
            (ph * TAU).sin() * a * ((len - t) / tau).min(1.0)
        })
        .collect()
}

/// Liquid falling into a bottle holding `from` units until it holds `to`, for `len`
/// seconds (plus a short ring-out).
fn glug(from: usize, to: usize, len: f32, rng: &mut Rng) -> Vec<f32> {
    let total = len + 0.12;
    let fill = |t: f32| from as f32 + (to - from) as f32 * (t / len).min(1.0);
    let env = |t: f32| (t / 0.02).min(1.0) * ((total - t) / 0.10).clamp(0.0, 1.0);
    // Slow shared wobble: the stream thickening and thinning.
    let knots: Vec<f32> = (0..=(total / pour::KNOT) as usize + 2)
        .map(|_| rng.next_f32())
        .collect();
    let wob = |t: f32| {
        let x = t / pour::KNOT;
        let (i, fr) = ((x as usize).min(knots.len() - 2), x.fract());
        let v = knots[i] + (knots[i + 1] - knots[i]) * (3.0 - 2.0 * fr) * fr * fr;
        1.0 - pour::WOBBLE + pour::WOBBLE * 2.0 * v
    };
    let mut out = vec![0.0f32; secs(total)];
    let (lo, hi) = pour::FREQ;
    let mut t = 0.0;
    loop {
        t += -rng.range(1e-4, 1.0).ln() / (pour::RATE * 1.8);
        if t >= len {
            break;
        }
        // Thinning against the wobble -> clustered, not uniform, arrivals.
        if rng.next_f32() > wob(t) / 1.8 {
            continue;
        }
        // Filling shifts the population up with the air column, by half its ratio.
        let shift = (column_freq(fill(t)) / column_freq(0.0)).sqrt();
        let f = lo * shift * (hi / lo).powf(rng.next_f32().powf(pour::SKEW));
        // van den Doel damping (d = 0.13 f + 0.0072 f^1.5), stretched by `RING`.
        let tau = pour::RING / (0.13 * f + 0.0072 * f.powf(1.5));
        // Bigger (lower) bubbles are louder, but only mildly — the crowd carries it.
        let amp = (lo * shift / f).powf(0.4) * rng.range(0.4, 1.0) * env(t);
        add_at(&mut out, &bubble(f, tau, pour::RISE), t, amp);
    }
    // A soft wash under the bubbles: noise lowpassed to almost nothing above 700 Hz.
    let mut w: Vec<f32> = (0..out.len())
        .map(|i| {
            let t = i as f32 / SR;
            rng.range(-1.0, 1.0) * env(t) * wob(t)
        })
        .collect();
    for _ in 0..3 {
        one_pole_lowpass(&mut w, 700.0, SAMPLE_RATE);
    }
    normalize_peak(&mut w, 1.0);
    let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    add_at(&mut out, &w, 0.0, pour::WASH * peak);
    // Air column: a moderate-Q resonator at its rising pitch, driven by the bubbles.
    let r = (-1.0 / (0.012 * SR)).exp();
    let (mut y1, mut y2) = (0.0f32, 0.0f32);
    let res: Vec<f32> = out
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            let wc = TAU * column_freq(fill(i as f32 / SR)) / SR;
            let y = x * (1.0 - r) + 2.0 * r * wc.cos() * y1 - r * r * y2;
            y2 = y1;
            y1 = y;
            y
        })
        .collect();
    let (pk_o, pk_r) = [&out, &res]
        .map(|v| v.iter().fold(1e-9f32, |m, x| m.max(x.abs())))
        .into();
    add_at(&mut out, &res, 0.0, pour::COLUMN * pk_o / pk_r);
    one_pole_lowpass(&mut out, 7000.0, SAMPLE_RATE);
    one_pole_highpass(&mut out, 120.0, SAMPLE_RATE);
    normalize_peak(&mut out, 0.9);
    out
}

/// A glass bottle set down on the shelf: a dull knock plus a short, quiet ring.
fn clink(rng: &mut Rng) -> Vec<f32> {
    let mut out = burst(0.05, 0.006, 300.0, 2500.0, rng);
    add_at(&mut out, &glass(2300.0, 0.035), 0.0, 0.25);
    out
}

/// One bell-like glass note per color, C major pentatonic from C5 up, so each sorted
/// color has its own voice.
fn sorted_chime(color: Color) -> Vec<f32> {
    const PENTA: [f32; 5] = [0.0, 2.0, 4.0, 7.0, 9.0];
    let c = color as usize;
    // `note_freq` is semitones from A4; C5 is +3.
    let st = 3.0 + 12.0 * (c / 5) as f32 + PENTA[c % 5];
    glass(note_freq(st), 0.25)
}

/// A lock's latch letting go: a metallic tick, a low thunk, a short spring ring.
fn unlock(rng: &mut Rng) -> Vec<f32> {
    let mut out = burst(0.04, 0.004, 2000.0, 8000.0, rng);
    let thunk: Vec<f32> = ring(0.12, 140.0, 0.025);
    add_at(&mut out, &thunk, 0.03, 0.6);
    add_at(&mut out, &ring(0.2, 1850.0, 0.04), 0.03, 0.15);
    out
}

/// Level solved: a rising glass arpeggio, C E G C.
fn won() -> Vec<f32> {
    let mut out = Vec::new();
    for (k, st) in [3.0, 7.0, 10.0, 15.0].into_iter().enumerate() {
        add_at(&mut out, &glass(note_freq(st), 0.3), k as f32 * 0.09, 0.5);
    }
    normalize_peak(&mut out, 0.9);
    out
}

/// Out of moves: two soft, low, falling notes.
fn stuck() -> Vec<f32> {
    let mut out = Vec::new();
    for (k, st) in [-14.0f32, -19.0].into_iter().enumerate() {
        let f = note_freq(st);
        let mut note = ring(0.7, f, 0.2);
        add_at(&mut note, &ring(0.7, f * 2.0, 0.1), 0.0, 0.3);
        add_at(&mut out, &note, k as f32 * 0.22, 0.6);
    }
    one_pole_lowpass(&mut out, 900.0, SAMPLE_RATE);
    normalize_peak(&mut out, 0.9);
    out
}

pub struct Sound {
    /// Indexed `[from][to]`, `from < to <= CAPACITY` (other slots unused).
    glugs: Vec<Vec<Option<Clip>>>,
    clink: Clip,
    chimes: Vec<Clip>,
    unlock: Clip,
    won: Clip,
    stuck: Clip,
}

impl Sound {
    pub async fn load(seed: u64) -> Sound {
        let mut rng = Rng::new(seed);
        let mut glugs = Vec::new();
        for from in 0..=CAPACITY {
            let mut row = Vec::new();
            for to in 0..=CAPACITY {
                row.push(if from < to {
                    Some(Clip::from_samples(&glug(from, to, 0.30, &mut rng)).await)
                } else {
                    None
                });
            }
            glugs.push(row);
        }
        let mut chimes = Vec::new();
        for c in 0..crate::PALETTE.len() {
            chimes.push(Clip::from_samples(&sorted_chime(c as Color)).await);
        }
        Sound {
            glugs,
            clink: Clip::from_samples(&clink(&mut rng)).await,
            chimes,
            unlock: Clip::from_samples(&unlock(&mut rng)).await,
            won: Clip::from_samples(&won()).await,
            stuck: Clip::from_samples(&stuck()).await,
        }
    }

    pub fn pour(&self, from: usize, to: usize) {
        if let Some(Some(c)) = self.glugs.get(from).and_then(|r| r.get(to)) {
            c.play_once(0.45);
        }
    }

    pub fn set_down(&self) {
        self.clink.play_once(0.3);
    }

    pub fn sorted(&self, color: Color) {
        self.chimes[color as usize % self.chimes.len()].play_once(0.4);
    }

    pub fn unlock(&self) {
        self.unlock.play_once(0.5);
    }

    pub fn won(&self) {
        self.won.play_once(0.5);
    }

    pub fn stuck(&self) {
        self.stuck.play_once(0.6);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_are_finite_and_in_range() {
        let mut rng = Rng::new(1);
        let mut all = vec![clink(&mut rng), unlock(&mut rng), won(), stuck()];
        for from in 0..CAPACITY {
            for to in from + 1..=CAPACITY {
                all.push(glug(from, to, 0.30, &mut rng));
            }
        }
        for c in 0..12 {
            all.push(sorted_chime(c));
        }
        for clip in &all {
            assert!(!clip.is_empty());
            assert!(clip.iter().all(|s| s.is_finite() && s.abs() <= 1.5));
        }
    }
}
