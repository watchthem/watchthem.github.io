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
/// `xtask::audio_bridge`), which *does* immediately silence everything already
/// playing, at the hardware level.
static MUTED: AtomicBool = AtomicBool::new(false);

/// Not meant to be called by game code — see `MUTED`'s doc comment. `control::Control`
/// is the sole intended caller.
pub fn set_muted(muted: bool) {
    MUTED.store(muted, Ordering::Relaxed);
}

/// Also true for the whole run of a screenshot/clip capture (`screenshot::is_capturing`):
/// those are automated headless runs — build steps, CI — that must never make a sound,
/// whatever game is being captured. Checked here, at the one gate every clip plays
/// through, so no game has to remember to mute itself.
pub fn muted() -> bool {
    static CAPTURING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    MUTED.load(Ordering::Relaxed) || *CAPTURING.get_or_init(screenshot::is_capturing)
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    /// `xtask::audio_bridge`: 1 when the browser's `AudioContext` is running, 0
    /// while autoplay policy still holds it suspended (and then shows the page's
    /// "sound is off" banner).
    fn hcg_audio_running() -> i32;
}

/// False on WASM until the page has had the user gesture browsers require before audio
/// may start. A one-shot played before that isn't dropped by the browser: its
/// `start(0)` is queued against a frozen clock, and the first tap would then fire every
/// footstep/line-clear since page load at once. Always true on native.
fn backend_running() -> bool {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        hcg_audio_running() != 0
    }
    #[cfg(not(target_arch = "wasm32"))]
    true
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

    /// Skipped while the browser still blocks audio (see `backend_running`), so sound
    /// picks up from the current game state once it's unblocked.
    pub fn play_once(&self, volume: f32) {
        if muted() || !backend_running() {
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

    /// Like `play_once`, but *not* skipped while the browser still blocks audio: the
    /// clip is queued and starts with the first gesture. For a one-off that should still
    /// be heard late rather than never — a game's opening jingle (tetris's intro) —
    /// never for per-event sounds, which would all pile up and fire at once.
    pub fn play_once_queued(&self, volume: f32) {
        if muted() {
            return;
        }
        backend_running();
        audio::play_sound(
            &self.0,
            PlaySoundParams {
                looped: false,
                volume,
            },
        );
    }

    /// Not gated on `backend_running`: a bed started while audio is blocked just stays
    /// queued and begins with the first gesture, which is what an ambience should do —
    /// games start their loops once, not every frame. The call still reveals the page's
    /// "sound is off" banner.
    pub fn play_looped(&self, volume: f32) {
        if muted() {
            return;
        }
        backend_running();
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
