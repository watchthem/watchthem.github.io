//! Thin wrapper over `macroquad::audio` for playing runtime-synthesized clips. Mute/
//! master-volume UI (a `Control` hotkey, an `xtask::hotkey_popup` entry) is a later
//! phase — every `Clip` for now just plays at whatever volume the caller passes.

use crate::wav::{SAMPLE_RATE, encode_mono_wav};
use macroquad::audio::{self, PlaySoundParams, Sound};

/// A loaded, playable synthesized clip.
pub struct Clip(Sound);

impl Clip {
    /// Encodes `samples` (mono `f32`, `[-1.0, 1.0]`, `SAMPLE_RATE` Hz) as WAV and loads
    /// it through macroquad's audio backend. Must run inside the async game loop, same
    /// as any other macroquad asset load, and never under `--no-ui` (no audio device/GL
    /// context in headless mode).
    pub async fn from_samples(samples: &[f32]) -> Self {
        let bytes = encode_mono_wav(samples, SAMPLE_RATE);
        let sound = audio::load_sound_from_bytes(&bytes)
            .await
            .expect("a WAV this crate encoded should always decode");
        Clip(sound)
    }

    pub fn play_once(&self, volume: f32) {
        audio::play_sound(
            &self.0,
            PlaySoundParams {
                looped: false,
                volume,
            },
        );
    }

    pub fn play_looped(&self, volume: f32) {
        audio::play_sound(
            &self.0,
            PlaySoundParams {
                looped: true,
                volume,
            },
        );
    }

    pub fn stop(&self) {
        audio::stop_sound(&self.0);
    }
}
