//! Oscillator + envelope primitives shared by SFX (`sfx.rs`) and, later, music synthesis.

use crate::rng::Rng;

/// Basic waveform shapes, mirroring the classic 8-bit sound-channel set (pulse/triangle/
/// noise) plus sine/saw for smoother voices.
#[derive(Clone, Copy, Debug)]
pub enum Waveform {
    /// `duty` is the fraction of the cycle spent high, in `(0.0, 1.0)` — 0.5 is a plain
    /// square wave; other values give the classic NES pulse-width timbres.
    Square {
        duty: f32,
    },
    Triangle,
    Sine,
    Sawtooth,
    /// White noise — needs its own RNG draw per sample, so unlike the other variants its
    /// output isn't a pure function of `phase`.
    Noise,
}

impl Waveform {
    /// Samples the waveform at `phase` (wrapped to `[0.0, 1.0)`), returning an amplitude
    /// in `[-1.0, 1.0]`. `rng` is only consumed by `Waveform::Noise`.
    pub fn sample(self, phase: f32, rng: &mut Rng) -> f32 {
        let phase = phase.rem_euclid(1.0);
        match self {
            Waveform::Square { duty } => {
                if phase < duty {
                    1.0
                } else {
                    -1.0
                }
            }
            Waveform::Triangle => 4.0 * (phase - 0.5).abs() - 1.0,
            Waveform::Sine => (phase * std::f32::consts::TAU).sin(),
            Waveform::Sawtooth => 2.0 * phase - 1.0,
            Waveform::Noise => rng.next_f32() * 2.0 - 1.0,
        }
    }
}

/// Standard attack/decay/sustain/release envelope, in absolute seconds (not fractions —
/// simpler to reason about when composing a specific SFX preset by ear/measurement).
#[derive(Clone, Copy, Debug)]
pub struct Envelope {
    pub attack: f32,
    pub decay: f32,
    pub sustain_level: f32,
    pub sustain: f32,
    pub release: f32,
}

impl Envelope {
    pub fn total_secs(&self) -> f32 {
        self.attack + self.decay + self.sustain + self.release
    }

    /// Gain in `[0.0, 1.0]` at `t` seconds since the note started; `0.0` outside
    /// `[0.0, total_secs()]`.
    pub fn gain_at(&self, t: f32) -> f32 {
        if t < 0.0 {
            return 0.0;
        }
        if t < self.attack {
            return if self.attack > 0.0 {
                t / self.attack
            } else {
                1.0
            };
        }
        let t = t - self.attack;
        if t < self.decay {
            let k = if self.decay > 0.0 {
                t / self.decay
            } else {
                1.0
            };
            return 1.0 + (self.sustain_level - 1.0) * k;
        }
        let t = t - self.decay;
        if t < self.sustain {
            return self.sustain_level;
        }
        let t = t - self.sustain;
        if t < self.release {
            let k = if self.release > 0.0 {
                t / self.release
            } else {
                1.0
            };
            return self.sustain_level * (1.0 - k);
        }
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_wave_switches_at_duty() {
        let mut rng = Rng::new(1);
        let w = Waveform::Square { duty: 0.5 };
        assert_eq!(w.sample(0.1, &mut rng), 1.0);
        assert_eq!(w.sample(0.6, &mut rng), -1.0);
    }

    #[test]
    fn triangle_peaks_at_edges_and_troughs_at_center() {
        let mut rng = Rng::new(1);
        let w = Waveform::Triangle;
        assert!((w.sample(0.0, &mut rng) - 1.0).abs() < 1e-6);
        assert!((w.sample(0.5, &mut rng) - -1.0).abs() < 1e-6);
    }

    #[test]
    fn sine_bounded() {
        let mut rng = Rng::new(1);
        let w = Waveform::Sine;
        for i in 0..100 {
            let s = w.sample(i as f32 / 100.0, &mut rng);
            assert!((-1.0..=1.0).contains(&s));
        }
    }

    #[test]
    fn noise_bounded_and_varies() {
        let mut rng = Rng::new(1);
        let w = Waveform::Noise;
        let a = w.sample(0.0, &mut rng);
        let b = w.sample(0.0, &mut rng);
        assert!((-1.0..=1.0).contains(&a));
        assert!((-1.0..=1.0).contains(&b));
        assert_ne!(a, b);
    }

    #[test]
    fn envelope_shape() {
        let env = Envelope {
            attack: 0.1,
            decay: 0.1,
            sustain_level: 0.5,
            sustain: 0.2,
            release: 0.1,
        };
        assert_eq!(env.gain_at(-1.0), 0.0);
        assert!((env.gain_at(0.0) - 0.0).abs() < 1e-6);
        assert!((env.gain_at(0.1) - 1.0).abs() < 1e-6);
        assert!((env.gain_at(0.2) - 0.5).abs() < 1e-6);
        assert!((env.gain_at(0.4) - 0.5).abs() < 1e-6);
        assert!((env.gain_at(0.5) - 0.0).abs() < 1e-6);
        assert_eq!(env.gain_at(1.0), 0.0);
    }
}
