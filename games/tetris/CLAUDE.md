# Tetris

Self-playing classic 10x20 Tetris, `beam_solver`-driven (depth 2). Own narrower 600x720
canvas (not the shared 900x720 default) since the board is inherently tall and narrow —
see `xtask::native_size`/`max_fit_scale`'s `"tetris"` arms.

## Source layout

| File | Contents |
|------|----------|
| `src/game.rs` | `Piece`, `Board`, rotation-shape derivation, `Game` (rules/state), scoring |
| `src/generator.rs` | `PieceGenerator`/`GenMode` — the 4 piece-randomizer algorithms |
| `src/solver.rs` | `beam_solver::SearchState` impl + the board-evaluation heuristic |
| `src/lib.rs` | `Session`/`View`/`FallAnim`, `VariantMode` (piece-gen mode cycle), CLI, rendering |
| `src/main.rs` | Thin standalone binary — `tetris::start()` and nothing else |

## Piece generation (`generator.rs`)

`GenMode` matches the RNG shape of real Tetris implementations, not one made-up
distribution — `V`-cycles like other games' variant switches (`VariantMode::Auto`
rotates all 4 by generation, same pattern as Sudoku's difficulty cycle):

| Mode | Behavior |
|---|---|
| `Bag7` | Modern Guideline: shuffle all 7 into a bag, deal one-by-one, reshuffle — every piece exactly once per 7 |
| `Classic` | 1989 NES-style: uniform, single reroll on immediate repeat — can still drought (esp. `I`) |
| `Tgm` | Arika Grand Master: reroll up to 4x against the last-4-piece history; never opens on `S`/`Z` |
| `Memoryless` | Pure uniform, no history — can drought/flood, kept specifically to be behaviorally distinct from the other three |

## Rotation shapes (`game.rs`)

Derived generically, not hand-transcribed: one hardcoded spawn-orientation shape per
piece (`Piece::base_shape`) plus a standard `(x,y) -> (size-1-y, x)` rotate-in-box
transform (`rotate_cw`) applied up to 4x, deduplicated by sorted-cell-set comparison
(`rotation_states`). No wall-kick tables — the solver enumerates every (rotation,
column) pair and hard-drops fresh from the top every time, never rotating a piece
already resting against neighbors. 3 unit tests cover rotation-count-matches-symmetry,
4-connectivity, and 4-rotations-return-to-start.

## Solver (`solver.rs`)

`BEAM_WIDTH = 12`, `BEAM_DEPTH = 2` (current piece + one known-next lookahead —
`Game::queue`/`LOOKAHEAD` keeps real pieces pre-generated so `SearchState::apply` stays
deterministic during search, same "pre-shuffled deck" trick as Klondike/Spider),
`NODE_BUDGET = 8_000`. Heuristic is Yiyuan Lee's widely-reused GA-tuned one-piece weight
set, scored on the *resulting* board per placement (not a delta):

```
W_AGGREGATE_HEIGHT = -0.510066
W_LINES_CLEARED    = +0.760666
W_HOLES            = -0.35663
W_BUMPINESS        = -0.184483
```

## Animation (`lib.rs`)

`Game::apply` places pieces and clears lines instantly (pure/discrete — headless mode
is a plain tight loop, no virtual-dt stepping). `FallAnim::pose()` fakes a real
multi-beat play-through cosmetically: hold at spawn (`SPAWN_ROW = -2`, 2 rows above the
visible board) → instant-snap rotate while sliding to the target column and partially
descending (`ROTATE_FRAC`/`SLIDE_FRAC`) → accelerating hard-drop the rest of the way.
`board_cache` (`RenderCache`) covers the locked board only (no text, so no font-atlas
prewarm needed for it) — the falling piece and line-clear flash stay live per-frame
draws on top, same split as klondike/spider's card table.

## Sound (`sound.rs`)

Procedural audio via `lib/audio` (oscillator/envelope engine), synthesized at startup,
no asset files. Every clip triggers off game *status*, not the solver directly — same
points a human-played game would hit.

| Clip | Trigger |
|---|---|
| `intro` | Once per load, before gameplay starts |
| `rotate` | Piece snaps to rotated shape (`fall.t` crosses `ROTATE_FRAC`) |
| `drop` | Piece committed to fall (inside `do_advance`, not mid-animation) |
| `lock` | Piece settles, no clear |
| `clear[0..3]` | Line clear, indexed by `lines_cleared - 1` |
| `game_over` | Episode ends |

Low-register, percussion-first — a tonal/melodic sound repeats on every routine event
and reads as grating; save melody for rare moments only. `rotate`/`lock` are filtered
noise/kick thumps, not tones. `clear` is `sound::explosion`: noise rumble + sub-bass
boom, pinned to exactly `FLASH_DUR` (the visual flash's own length) so sound and blink
match regardless of clear size, with a swept low-pass (bright crack -> dull rumble, one
continuous IIR pass — chunking would click at boundaries) so it reads as an explosion,
not another `lock` thump. `do_advance` skips `drop` right after a `lock` — both are bass
thumps, playing them in the same frame doubled up as an extra click.

`drop`'s envelope is kept under ~0.083s (`ROTATE_FRAC * (1 / ANIM_SPEED)`, `lib.rs`) on
purpose: `rotate` fires at that real-time offset on *every* piece (`FallAnim::pose`'s
snap happens whether or not the piece actually rotates), so a longer `drop` envelope had
its release tail still sounding when `rotate`'s own onset (a real click, by design)
landed on top of it — heard as one "hit" that's too long with a stray click stapled to
the end. `lib.rs` also gates `rotate` on `fall.spawn_shape != fall.target_shape` so a
piece that spawns already in its landing orientation stays silent there instead of
firing a "rotate" cue for a rotation that never happened.

`intro` = "Korobeiniki" (Коробе́йники, ~1861 Russian folk tune, public domain — the real
Tetris theme), transcribed fresh in `korobeiniki()`, 8 measures each exactly
`MEASURE_SECS`. The melody is **A natural minor** (F♮/G♮ in m5, never F#/G#, closes on a
held A4) — `CHORD_GUIDE_TONES` harmonizes it Am11-Am7-Em7-Am7-Dm7-C6-E7-**A6** (i-i-v-i-
iv-♭III-V-**I**), one chord per measure as just the 3rd+7th (guide tones — the two notes
that define a chord's color; a full triad reads as muddy under one melody line). The
final chord (m8) is a Picardy third — the tonic raised to major (`6` voicing, matching
`C6`'s own precedent, not `maj7`) instead of closing on the tonic minor. This does mean
the measure's opening C5 (the tune's own minor 3rd) briefly sits a half-step against the
comp's now-major C# before the melody moves on to its held A4 root — expected for a
Picardy third landing under a melody that hasn't also switched to major, not a bug; see
`CHORD_GUIDE_TONES`'s own doc comment before treating it as a clash to fix. Comp envelope
fractions must sum to `1.0 * MEASURE_SECS` exactly or it drifts out of sync with the
melody — regression-tested (`comp_track_len_matches_melody_len_per_measure`). Square-wave
lead gets a light low-pass + reduced sustain so it doesn't read as harsh/clippy.

`korobeiniki_track`'s final `normalize_peak` target is `0.65`, not the more obvious
`0.85`-ish — real headroom, not style. Every note's ADSR ramps to full `1.0` gain at
the *end of attack* regardless of `sustain_level` (sustain only caps the plateau after
decay), so a busy melody has many moments near the true peak, not one.
`normalize_peak` only sees discrete sample values, not inter-sample ("true peak")
overshoot introduced when a decoder reconstructs the waveform between samples — square
waves (steep edges, strong high harmonics) are exactly the content most prone to that.
Too little margin here reads as distortion at the loudest moments, not literal clipping
you'd see in the sample data.

`bass_track` adds a funky "3+1" figure under the comp: `BASS_ROOTS` (one root per
measure, derived from `CHORD_GUIDE_TONES`'s 3rd minus a minor/major 3rd, dropped another
octave) plays through `funky_bass_measure` — three short staccato hits (`da-da-da`) then
one longer held note filling the rest of the measure (`DAAA`), each hit/rest sized in
exact `EIGHTH` multiples so the figure sums to `MEASURE_SECS` like everything else here.
The 1st and 4th (held) hits are the root; the 2nd and 3rd are two different color
notes, not more root repeats — a perfect 5th (root + 7, correct for every chord quality
here) then the chord's own 7th/6th (`BASS_COLOR_TONE`: +10 for the `m7` chords, +9 for
the one `C6` measure, since a b7 there would clash with its actual 6th). `bass_hit` is a
sawtooth low-passed at 900Hz (`bass_track`, once over the whole line) rather than a bare
`Triangle` — a triangle at this register is close to a pure fundamental and read as
thin/flute-y rather than bassy; the filtered saw keeps real harmonic body.
`drum_track` layers a light jazz-funk kit on top: a soft closed-hihat pulse on every
eighth plus a soft kick on beat 1 and snare on beats 2/4 (`drum_measure`), each voice
scaled well under its own natural loudness before mixing — a full-volume kit read as a
different, busier song competing with the melody, not a light pulse under it. Hits land
at arbitrary offsets mid-track, so they're mixed with `place_at` (adds a clip into a
buffer at a given sample offset, growing it with silence as needed) rather than
`mix::sum` (which only aligns buffers starting at index 0). `hat`/`kick`/`snare` are
each rendered exactly once (`drum_track`) and placed repeatedly, not resynthesized per
hit — `chiptune::hihat`/`snare`'s noise components use a fixed internal seed regardless,
so every hit was already bit-identical audio, just wastefully recomputed 64/16 times.

Only the melody gets `melody_only`'s low-pass — filtering the whole mix would dull the
drum kit's noise-based hihat/snare, which needs its own high-frequency content to read
as percussion rather than a thump.

**Mix balance**: every bass/melody note's envelope peaks at `1.0` (`Envelope::gain_at`
ramps to full gain at the end of attack regardless of `sustain_level`), but `comp_chord`
— the only layer actually carrying the harmony — is normalized to `0.8`. Left alone, the
louder layers' transients set the peak `korobeiniki_track`'s final `normalize_peak`
scales *everything* against, burying the comp under thumping roots ("harmony sounds like
a mess" can be a balance bug, not a wrong-note bug). `bass_track` attenuates its own
output by `0.55` after the lowpass, before that shared scaling, to buy the comp real
headroom.

**Fixing a bad chord — two different bug classes, check both**: (1) *literal clash*:
check every guide tone against every melody note in that measure as a semitone class
(0-11) — a clash is exactly `1` or `11` apart, OK only if the *next* chord resolves it.
(2) *functional/key-center*: a chord can be clash-free against its own measure and still
be the *wrong chord* for the piece's actual key — this doesn't show up in a per-measure
pairwise check at all. Work out what key the melody is actually in (which notes never
appear — e.g. this melody never uses F#/G#, ruling out any chord that implies them) and
verify the whole progression makes sense in that one key, not just bar-by-bar. The
E-minor-vs-A-minor bug here (`Bm7` isn't diatonic to A minor and needs an F# the melody
contradicts four bars later; `Em7` "i" chords were actually the v) passed every pairwise
clash check and still sounded like "a mess" — only a full 8-measure functional analysis
found it. Watch for the clash check's subtler failure mode too: a chord can be
clash-free yet still not resolve where the melody itself actually *lands* (e.g. the
closing note) — check what the tune is really centered on, not just adjacent notes.
Prefer octave-shifting a guide tone (12 semitones) for smoother voice-leading over
adding a 3rd note. Iterate by listening (`mise run run tetris`) — code-reading alone
won't tell you if a harmony works, and for the functional check, if you're not confident
re-deriving the key/progression by ear, get a second pass from a stronger model.

**Gotcha**: macroquad's `audio` feature is off by default. Missing it silently no-ops
every `play_sound` call (stderr warning, no crash) instead of failing loudly — grep for
`"feature disabled"` if sound seems to do nothing.

**"Buffer underrun"-sounding artifacts at page load**: `Sfx::load` used to synthesize all
9 clips back-to-back, only `.await`ing (yielding to the browser) once all 9 were already
rendered. Measured via Playwright + a `longtask` `PerformanceObserver`: 3 main-thread
long tasks (230/82/116ms) in the first ~650ms of a real page load — not a glitch in the
rendered samples themselves (a `scan_for_clicks`-style sample-diff scan found zero
discontinuities). Fixed by loading each clip (`Clip::from_samples`, already a yield
point) right after synthesizing it instead of synthesizing all 9 first, plus an extra
`next_frame().await` after the heaviest chunks — cut it to 1 long task of 119ms.
`next_frame()` needs `Sfx::load` to only run inside the real windowed loop (`amain`),
never `run_headless` — it has no event loop to yield into there.

Adding `bass_track`/`drum_track` regressed this: one combined
melody+comp+bass+drums synthesis step was heavy enough on its own to bring it back to 3
tasks up to 202ms. `Sfx::load` synthesizes `intro`'s 4 layers (melody, comp, bass,
drums) as 4 separate steps, each with its own `next_frame().await`, rather than calling
`korobeiniki_track` (which still exists, `#[cfg(test)]`-only, as the single-call
reference the diagnostic tests use) — down to 2 tasks around 170ms/76ms. Splitting
melody from comp specifically didn't move the needle much further (some of the cost here
is fixed per-call interpreter/JIT overhead on wasm, not purely proportional to the
native synthesis time) — good enough given real new audio content had to go somewhere,
not chased further.

## Opening screen

`amain` waits `sound::INTRO_SECS` before spawning the first piece, drawing
`draw_opening_title` + `draw_demo_board` instead. Skipped (gameplay starts right away):
screenshot/clip capture (`screenshot::is_capturing()`, also skips `intro` — nobody's
listening), the ambient wall (`stream_mode()`, `?embed=1`/`?stream=1` — already muted,
see Sound above), and daily challenge (`daily_mode()`, `?daily=1` — `intro` still
plays, just doesn't block gameplay).

`DemoBoard`: cosmetic only, never touches `Session`/`Game`, own `audio::Rng` (not
`macroquad::rand`'s global one — that's seeded for deterministic replay; drawing from
it here would shift every real piece). Three beats, looping: board is **pre-filled**
(random, every row missing >=1 cell so nothing reads as a real line) and visible right
away; a **flood** sweeps bottom-to-top turning rows solid-color; a **drain** sweeps
top-to-bottom emptying rows; repeats with a fresh pattern. `ROWS_PER_SEC = 3.0` keeps
one full cycle (~13.3s) *longer* than the intro (~11.5s) on purpose — a visitor sees one
continuous flood into a partial drain, never the reset back to a fresh pattern.

`DemoRow::Solid` draws each flooded row the same way `Pattern` rows do — `draw_cell` per
column, not a flat `draw_rectangle` — for visual consistency with the rest of the board
(real pieces, `Pattern` rows all use the same per-cell grid look). What makes the flood
read as one connected mass isn't the geometry, it's `flood_color`: a *single* color for
the whole flooded region (picked once per cycle, in `random_cycle`), not a different
color per row — tried a flat rectangle-per-row first (no per-cell insets, no seams
between rows) specifically to avoid a "row of separated squares" look, but that reads as
inconsistent with the board's own block aesthetic; a uniform color across all the
per-cell squares turned out to fix the "separated squares" problem on its own, without
needing to depart from `draw_cell`.

`draw_opening_panel` is the opening screen's counterpart to `draw_hud`'s side panel
(shared `draw_panel_frame` for the bordered container, same `NEXT_BOX_H`) — without it
the whole panel column reads as missing/empty during loading (same "extra empty space"
`draw_panel_frame`'s own doc comment describes) and pops into existence the instant the
intro ends. Shows `DemoBoard::next_preview` (a cosmetic piece, regenerated alongside
`pattern`/`flood_color` each cycle — never touches the real solver's queue) and
zeroed-out stat lines — `GEN 0`, not `GEN 1`: the real HUD's own display is
`generation + 1`, so a fresh game's actual first frame reads `GEN 1`, but the opening
screen is *before* that first generation exists at all, and `0` reads as "nothing
started yet" rather than implying a generation already happened.

## Gotchas

- `gen` is a reserved keyword since the 2024 edition (future generator-block syntax) —
  `let gen = ...` fails to compile. Named `piece_gen` throughout instead.
- `BOARD_Y = 128` (not a smaller value) is deliberate clearance so the falling piece's
  cosmetic spawn point (`SPAWN_ROW = -2`) doesn't draw through the "TETRIS" title text.
- Interior-only grid lines (`1..W`/`1..H`, not `0..=W`/`0..=H`) in `draw_board_static` —
  drawing a grid line directly on the board's own outer edge partially overwrites the
  border rect there, asymmetrically enough between edges to read as a visibly thinner
  border on one side.
- The side panel (next-piece box + stat lines) gets the same bordered
  container/full-height treatment as the board itself — without it, the panel's actual
  content only fills part of its height, leaving bare background below that reads as
  "more empty space" next to the board.
- The "NEXT" panel shows only `queue.front()` — 1 piece, the classic (NES-era) Tetris
  convention, not modern guideline Tetris's 3-6-deep queue. `game::LOOKAHEAD` (3) is
  unrelated and unchanged: it's retained depth for the solver's `BEAM_DEPTH = 2`
  lookahead, not display count — don't conflate the two if either changes again.
- Native `start()` calls `std::process::exit(0)` after `amain(cli).await` — without it,
  Esc/close-window left the process hanging (a completed future, unresponsive window)
  once `Control::new()` started unconditionally calling `prevent_quit()` for the shell.
  Same bug likely exists in every other game's `start()` — not yet fixed there.

## Running

```bash
mise run run tetris                                    # native
mise run build-wasm tetris                              # WASM → dist/tetris/
HCG_SEED=1 target/release/tetris --no-ui --once --variant bag7 --debug
```
