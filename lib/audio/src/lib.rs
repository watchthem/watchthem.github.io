//! Procedural audio for hcg games — oscillator/envelope synthesis, sfxr-style one-shot
//! SFX, in-memory WAV encoding, and a `macroquad::audio` playback wrapper. Every clip is
//! synthesized at runtime, not shipped as an asset file.
//!
//! Sound triggers bind to game *status* (a line clear, a win, a death, ...) exactly as
//! they would for a human-played game — every game in this workspace is self-playing,
//! but the AI is only an input-emulation layer (it presses the same buttons a human
//! would), never a sound source in its own right.
//!
//! No streaming/live-evolving synthesis: quad-snd has no audio-callback/ring-buffer API
//! (see upstream issue <https://github.com/not-fl3/quad-snd/issues/37>, open/unfixed),
//! so every clip — a short SFX or a longer looping music bed — must be fully rendered to
//! a buffer up front, then played back as a fixed clip.
//!
//! Music composition (`music.rs`: chord/scale tables, a sequencer) isn't built yet — it
//! lands with the first game that actually needs a background track, rather than as an
//! empty module today.

pub mod playback;
pub mod rng;
pub mod sfx;
pub mod synth;
pub mod wav;

pub use playback::Clip;
pub use rng::Rng;
pub use wav::SAMPLE_RATE;
