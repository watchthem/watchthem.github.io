//! One-shot synthesized sound effects, sfxr/jsfxr-style: a single oscillator swept by a
//! handful of parameter curves (frequency slide, arpeggio, vibrato) and shaped by an
//! ADSR envelope — see <https://github.com/chr15m/jsfxr> for the reference model this
//! mirrors. Hand-rolled rather than depending on the `sfxr` crate: that crate pulls in
//! `rand`, which on wasm32-unknown-unknown needs the `js` feature (→ wasm-bindgen) to
//! avoid a `getrandom` build failure — incompatible with this workspace's
//! miniquad-only WASM setup. This module never needs `rand` at all: randomization (for a
//! per-game "family of variations" preset, added when a game actually wires one up) can
//! use `crate::rng::Rng` directly.

use crate::rng::Rng;
use crate::synth::{Envelope, Waveform};

#[derive(Clone, Copy, Debug)]
pub enum SfxWave {
    Square { duty: f32 },
    Triangle,
    Sine,
    Sawtooth,
    Noise,
}

impl From<SfxWave> for Waveform {
    fn from(w: SfxWave) -> Self {
        match w {
            SfxWave::Square { duty } => Waveform::Square { duty },
            SfxWave::Triangle => Waveform::Triangle,
            SfxWave::Sine => Waveform::Sine,
            SfxWave::Sawtooth => Waveform::Sawtooth,
            SfxWave::Noise => Waveform::Noise,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SfxParams {
    pub wave: SfxWave,
    pub envelope: Envelope,
    /// Starting frequency, Hz.
    pub start_freq: f32,
    /// Hz/sec added to frequency every second (can be negative, for a falling pitch).
    pub freq_slide: f32,
    /// Change in `freq_slide` itself per second — a second-order sweep, for a slide that
    /// itself accelerates/decelerates.
    pub freq_delta_slide: f32,
    /// Semitones jumped partway through the note; `0.0` disables the arpeggio.
    pub arp_semitones: f32,
    /// Fraction of the envelope's `total_secs()` at which the arpeggio jump happens.
    pub arp_time_frac: f32,
    /// Vibrato depth as a fraction of frequency; `0.0` disables vibrato.
    pub vibrato_depth: f32,
    /// Vibrato rate, Hz.
    pub vibrato_speed: f32,
}

impl SfxParams {
    /// Renders the full clip to mono `f32` samples at `sample_rate` Hz. `rng_seed` only
    /// matters for `SfxWave::Noise` — pass any fixed value for a deterministic preset.
    pub fn render(&self, sample_rate: u32, rng_seed: u64) -> Vec<f32> {
        let mut rng = Rng::new(rng_seed);
        let waveform: Waveform = self.wave.into();
        let total = self.envelope.total_secs();
        let n = (total * sample_rate as f32).ceil() as usize;
        let dt = 1.0 / sample_rate as f32;
        let mut out = Vec::with_capacity(n);
        let mut phase = 0.0f32;
        for i in 0..n {
            let t = i as f32 * dt;
            let mut freq =
                self.start_freq + self.freq_slide * t + 0.5 * self.freq_delta_slide * t * t;
            if self.arp_semitones != 0.0 && t >= total * self.arp_time_frac {
                freq *= 2f32.powf(self.arp_semitones / 12.0);
            }
            if self.vibrato_depth != 0.0 {
                freq *= 1.0
                    + self.vibrato_depth * (t * self.vibrato_speed * std::f32::consts::TAU).sin();
            }
            let freq = freq.max(1.0);
            phase += freq * dt;
            let sample = waveform.sample(phase, &mut rng) * self.envelope.gain_at(t);
            out.push(sample);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn basic_params(wave: SfxWave) -> SfxParams {
        SfxParams {
            wave,
            envelope: Envelope {
                attack: 0.01,
                decay: 0.05,
                sustain_level: 0.6,
                sustain: 0.1,
                release: 0.05,
            },
            start_freq: 440.0,
            freq_slide: 0.0,
            freq_delta_slide: 0.0,
            arp_semitones: 0.0,
            arp_time_frac: 0.5,
            vibrato_depth: 0.0,
            vibrato_speed: 0.0,
        }
    }

    #[test]
    fn render_length_matches_envelope_duration() {
        let p = basic_params(SfxWave::Square { duty: 0.5 });
        let samples = p.render(crate::wav::SAMPLE_RATE, 1);
        let expected = (p.envelope.total_secs() * crate::wav::SAMPLE_RATE as f32).ceil() as usize;
        assert_eq!(samples.len(), expected);
    }

    #[test]
    fn render_is_bounded_and_not_silent() {
        let p = basic_params(SfxWave::Sawtooth);
        let samples = p.render(crate::wav::SAMPLE_RATE, 7);
        assert!(samples.iter().all(|s| (-1.0..=1.0).contains(s)));
        assert!(samples.iter().any(|&s| s.abs() > 0.01));
    }

    #[test]
    fn freq_slide_shifts_pitch_over_time() {
        // A falling slide should end at a lower instantaneous frequency than it started —
        // approximate by counting zero-crossings in the first vs. last third.
        let mut p = basic_params(SfxWave::Square { duty: 0.5 });
        p.freq_slide = -2000.0;
        p.envelope.sustain = 0.3;
        let samples = p.render(crate::wav::SAMPLE_RATE, 1);
        let third = samples.len() / 3;
        let crossings = |s: &[f32]| s.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        let first = crossings(&samples[..third]);
        let last = crossings(&samples[samples.len() - third..]);
        assert!(
            last < first,
            "expected falling pitch: first={first} last={last}"
        );
    }

    #[test]
    fn deterministic_for_same_seed() {
        let p = basic_params(SfxWave::Noise);
        let a = p.render(crate::wav::SAMPLE_RATE, 42);
        let b = p.render(crate::wav::SAMPLE_RATE, 42);
        assert_eq!(a, b);
    }
}
