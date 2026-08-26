//! Reusable 8-bit-style instrument voices and percussion — the building blocks a game's
//! own sound design (see e.g. `games/tetris/src/sound.rs`, which currently hand-rolls
//! its own one-off oscillator params) can lean on instead of re-deriving envelope/wave
//! parameters from scratch for every new sound. Not tied to any one game.
//!
//! Melodic voices (`pulse_lead`, `triangle_bass`) take a frequency and duration and
//! render one note. Percussion (`kick`, `snare`, `hihat`) is fixed-timbre, one-shot, no
//! pitch parameter — real drum hits.

use crate::filter::one_pole_highpass;
use crate::mix;
use crate::sfx::{SfxParams, SfxWave};
use crate::synth::Envelope;

fn note(freq_hz: f32, secs: f32, wave: SfxWave, sample_rate: u32) -> Vec<f32> {
    let attack = (secs * 0.05).min(0.01);
    let decay = secs * 0.1;
    let release = secs * 0.25;
    SfxParams {
        wave,
        envelope: Envelope {
            attack,
            decay,
            sustain_level: 0.8,
            sustain: (secs - attack - decay - release).max(0.0),
            release,
        },
        start_freq: freq_hz,
        freq_slide: 0.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(sample_rate, 1)
}

/// A bright pulse/square lead — the NES/Game Boy melodic voice. `duty` in `(0.0, 1.0)`
/// picks the timbre: 0.5 is a plain square, thinner values (e.g. 0.125/0.25) read as a
/// reedier pulse.
pub fn pulse_lead(freq_hz: f32, secs: f32, duty: f32, sample_rate: u32) -> Vec<f32> {
    note(freq_hz, secs, SfxWave::Square { duty }, sample_rate)
}

/// The NES's dedicated bass-channel timbre — softer than a pulse, no duty control.
pub fn triangle_bass(freq_hz: f32, secs: f32, sample_rate: u32) -> Vec<f32> {
    note(freq_hz, secs, SfxWave::Triangle, sample_rate)
}

/// A punchy low-end thump: a triangle wave sliding sharply down in pitch — the classic
/// cheap-drum-machine kick approximation. No noise component; the pitch drop alone
/// reads as "kick" at this level of synthesis.
pub fn kick(sample_rate: u32) -> Vec<f32> {
    SfxParams {
        wave: SfxWave::Triangle,
        envelope: Envelope {
            attack: 0.001,
            decay: 0.09,
            sustain_level: 0.0,
            sustain: 0.0,
            release: 0.02,
        },
        start_freq: 150.0,
        freq_slide: -1400.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(sample_rate, 1)
}

/// Noise "snap" plus a short low tonal body — the two components a real snare's
/// membrane-thump-plus-wire-buzz reduces to at this level of synthesis.
pub fn snare(sample_rate: u32) -> Vec<f32> {
    let tone = note(190.0, 0.08, SfxWave::Square { duty: 0.5 }, sample_rate);
    let mut noise = SfxParams {
        wave: SfxWave::Noise,
        envelope: Envelope {
            attack: 0.001,
            decay: 0.06,
            sustain_level: 0.0,
            sustain: 0.0,
            release: 0.03,
        },
        // `start_freq`/`freq_slide` are irrelevant for `SfxWave::Noise` (see
        // `Waveform::sample`) — kept at a harmless constant, present only because
        // `SfxParams` doesn't special-case noise's unused fields.
        start_freq: 1.0,
        freq_slide: 0.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(sample_rate, 2);
    one_pole_highpass(&mut noise, 1200.0, sample_rate);
    let mut out = mix::sum(&[&tone, &noise]);
    mix::normalize_peak(&mut out, 0.9);
    out
}

/// Filtered noise burst. `open` picks a closed (short, tight "tss") vs. open (longer
/// ring) hi-hat.
pub fn hihat(open: bool, sample_rate: u32) -> Vec<f32> {
    let release = if open { 0.18 } else { 0.02 };
    let mut noise = SfxParams {
        wave: SfxWave::Noise,
        envelope: Envelope {
            attack: 0.001,
            decay: 0.01,
            sustain_level: 0.0,
            sustain: 0.0,
            release,
        },
        start_freq: 1.0,
        freq_slide: 0.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(sample_rate, 3);
    one_pole_highpass(&mut noise, 6000.0, sample_rate);
    noise
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pitch::note_freq;
    use crate::wav::SAMPLE_RATE;

    fn bounded(samples: &[f32]) -> bool {
        samples
            .iter()
            .all(|s| s.is_finite() && (-1.5..=1.5).contains(s))
    }

    #[test]
    fn pulse_lead_renders_bounded_nonempty() {
        let s = pulse_lead(note_freq(0.0), 0.2, 0.5, SAMPLE_RATE);
        assert!(!s.is_empty());
        assert!(bounded(&s));
        assert!(s.iter().any(|&x| x.abs() > 0.01));
    }

    #[test]
    fn triangle_bass_renders_bounded_nonempty() {
        let s = triangle_bass(note_freq(-12.0), 0.3, SAMPLE_RATE);
        assert!(!s.is_empty());
        assert!(bounded(&s));
    }

    #[test]
    fn kick_is_short_and_bounded() {
        let s = kick(SAMPLE_RATE);
        assert!(!s.is_empty());
        assert!(bounded(&s));
        assert!((s.len() as f32 / SAMPLE_RATE as f32) < 0.2);
    }

    #[test]
    fn snare_mixes_tone_and_noise_without_clipping() {
        let s = snare(SAMPLE_RATE);
        assert!(!s.is_empty());
        assert!(s.iter().all(|&x| x.abs() <= 0.9 + 1e-3));
    }

    #[test]
    fn hihat_open_outlasts_closed() {
        let closed = hihat(false, SAMPLE_RATE);
        let open = hihat(true, SAMPLE_RATE);
        assert!(open.len() > closed.len());
        assert!(bounded(&closed));
        assert!(bounded(&open));
    }
}
