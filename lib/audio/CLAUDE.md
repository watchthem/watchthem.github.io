# lib/audio

Procedural sound: every clip is synthesized at startup into an in-memory 44.1kHz WAV and
played through `macroquad::audio`. No asset files, no `include_bytes!`.

| File | Contents |
|------|----------|
| `synth.rs` | oscillators (`Waveform`) + ADSR `Envelope` |
| `sfx.rs` | sfxr-style one-shots: `SfxParams { wave: SfxWave, .. }.render(sample_rate, seed)` |
| `chiptune.rs` | reusable voices: `pulse_lead`, `triangle_bass`, `kick`, `snare`, `hihat` |
| `filter.rs` / `mix.rs` / `pitch.rs` | one-pole LP/HP, buffer sum + peak normalize, `note_freq` |
| `rng.rs` | own splitmix64 — never draw sound randomness from `macroquad::rand` (it would perturb the seeded gameplay stream) |
| `wav.rs` | `hound` in-memory encode |
| `playback.rs` | `Clip::play_once` / `play_once_queued` / `play_looped` / `stop`; global mute gate |

Manual listening test: `cargo run -p audio --example chiptune_demo --release`.

## Adding sound to a game

- Put it in the game's `src/sound.rs` (see `games/tetris`, `games/water-sort`,
  `games/labyrinth`). Add `audio = { path = "../../lib/audio" }`.
- **Trigger from game-state transitions** (line clear, lock, win, death) — the AI is only
  an input source standing in for a human, never a sound trigger of its own.
- A discrete committed action sounds at its decision point, not at an animation
  threshold (a slow frame can cross several thresholds at once). Two sounds sharing one
  frame read as a glitch — check for that.
- Frequent sounds: noise bursts/thumps. Tonal/melodic writing only for rare moments — a
  bright arpeggio on every clear gets annoying fast.
- Tie durations to the animation constants they accompany (e.g. tetris's explosion =
  `FLASH_DUR`), never hardcode an absolute next to proportional values.
- Games never check mute themselves: `M`, `?embed=1`/`?stream=1` (wall — set muted at
  startup) and screenshot/clip capture all go through `playback`'s gate.
- If a sound misses by ear twice, analyze a reference recording rather than guessing again.
  For harmony, see `games/tetris/CLAUDE.md` "Fixing bad voicings".

## Platform constraints

- macroquad's `audio` feature is **off by default**; `lib/audio`'s Cargo.toml enables it
  and feature unification spreads it workspace-wide. Without it every call silently no-ops
  with a stderr warning — run the real binary and grep stderr, "compiles" proves nothing.
- Because it's workspace-wide, every game's native build links `-lasound` and opens the
  audio device at window init. CI uses a null ALSA device (`.github/workflows/deploy.yml`).
- No streaming/callback API in quad-snd: everything, including loops, is pre-rendered.
- Render at 44100Hz (ALSA panics otherwise). `set_sound_volume` is broken on wasm — set
  volume per play call.
- Never touch `macroquad::audio` under `--no-ui`.
- WASM autoplay: `play_once` skips while the AudioContext is suspended (no user gesture
  yet) instead of queueing a burst; `play_once_queued` is for a one-off that should still
  play late (an intro jingle). Muting also suspends the real AudioContext
  (`xtask::audio_bridge`) so a muted tab releases the device.
- Cost: enabling the feature was a one-time ~5KB gzip for the whole bundle; each game's
  own `sound.rs` is small on top.
