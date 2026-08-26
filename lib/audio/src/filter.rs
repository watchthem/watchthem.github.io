//! Minimal one-pole (6dB/octave) low-pass and high-pass filters — enough to turn raw
//! white noise into a hi-hat/snare-shaped sound (band-limited) rather than static, no
//! DSP library needed. Both operate in place, in one pass, causal (no lookahead).

use std::f32::consts::PI;

/// Exponential-moving-average low-pass. Attenuates content above `cutoff_hz`.
pub fn one_pole_lowpass(samples: &mut [f32], cutoff_hz: f32, sample_rate: u32) {
    let dt = 1.0 / sample_rate as f32;
    let rc = 1.0 / (2.0 * PI * cutoff_hz);
    let alpha = dt / (rc + dt);
    let mut prev = 0.0f32;
    for s in samples.iter_mut() {
        prev += alpha * (*s - prev);
        *s = prev;
    }
}

/// Complementary one-pole high-pass (standard discrete RC form). Attenuates content
/// below `cutoff_hz`.
pub fn one_pole_highpass(samples: &mut [f32], cutoff_hz: f32, sample_rate: u32) {
    let dt = 1.0 / sample_rate as f32;
    let rc = 1.0 / (2.0 * PI * cutoff_hz);
    let alpha = rc / (rc + dt);
    let mut prev_in = 0.0f32;
    let mut prev_out = 0.0f32;
    for s in samples.iter_mut() {
        let x = *s;
        let y = alpha * (prev_out + x - prev_in);
        prev_in = x;
        prev_out = y;
        *s = y;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rng::Rng;
    use std::f32::consts::TAU;

    const SR: u32 = 44_100;

    fn sine(freq: f32, secs: f32) -> Vec<f32> {
        let n = (secs * SR as f32) as usize;
        (0..n)
            .map(|i| (i as f32 / SR as f32 * freq * TAU).sin())
            .collect()
    }

    fn rms(samples: &[f32]) -> f32 {
        // Skip the filter's initial settling transient.
        let skip = samples.len() / 4;
        let tail = &samples[skip..];
        (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt()
    }

    #[test]
    fn lowpass_attenuates_content_above_cutoff() {
        let mut high = sine(5000.0, 0.02);
        let original_rms = rms(&high);
        one_pole_lowpass(&mut high, 200.0, SR);
        assert!(rms(&high) < original_rms * 0.3);
    }

    #[test]
    fn lowpass_mostly_passes_content_below_cutoff() {
        let mut low = sine(100.0, 0.05);
        let original_rms = rms(&low);
        one_pole_lowpass(&mut low, 5000.0, SR);
        assert!(rms(&low) > original_rms * 0.8);
    }

    #[test]
    fn highpass_attenuates_content_below_cutoff() {
        let mut low = sine(100.0, 0.05);
        let original_rms = rms(&low);
        one_pole_highpass(&mut low, 2000.0, SR);
        assert!(rms(&low) < original_rms * 0.3);
    }

    #[test]
    fn highpass_mostly_passes_content_above_cutoff() {
        let mut high = sine(8000.0, 0.02);
        let original_rms = rms(&high);
        one_pole_highpass(&mut high, 1000.0, SR);
        assert!(rms(&high) > original_rms * 0.8);
    }

    #[test]
    fn filters_stay_bounded_on_noise() {
        let mut rng = Rng::new(1);
        let mut noise: Vec<f32> = (0..1000).map(|_| rng.next_f32() * 2.0 - 1.0).collect();
        one_pole_lowpass(&mut noise, 1000.0, SR);
        assert!(noise.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    }
}
