//! Thin wrapper over `macroquad::audio` for playing runtime-synthesized clips.

use crate::wav::{SAMPLE_RATE, encode_mono_wav};
use macroquad::audio::{self, PlaySoundParams, Sound};
use std::sync::atomic::{AtomicBool, Ordering};

/// Global mute gate, checked by every `Clip::play_once`/`play_looped` call — set by
/// `control::Control`'s `M` hotkey (the one place in this workspace that owns
/// cross-game meta-controls), not meant to be called directly by game code. A process-
/// wide flag rather than a per-`Clip` one: every game's entire sound output should mute
/// together, and nothing here tracks in-flight `Clip`s to reach into, so this can't
/// retroactively silence an already-playing looped clip on native — on WASM, muting
/// additionally suspends the browser's `AudioContext` (see `control::Control` /
/// `xtask::audio_mute_bridge`), which *does* immediately silence everything already
/// playing, at the hardware level.
static MUTED: AtomicBool = AtomicBool::new(false);

/// Not meant to be called by game code — see `MUTED`'s doc comment. `control::Control`
/// is the sole intended caller.
pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
}

pub fn muted() -> bool {
    MUTED.load(Ordering::Relaxed)
}

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
        if muted() {
            return;
        }
        audio::play_sound(
            &self.0,
            PlaySoundParams {
                looped: false,
                volume,
            },
        );
    }

    pub fn play_looped(&self, volume: f32) {
        if muted() {
            return;
        }
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
