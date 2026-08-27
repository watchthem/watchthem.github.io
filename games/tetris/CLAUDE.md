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
`MEASURE_SECS`. `CHORD_GUIDE_TONES` jazz-harmonizes one chord per measure as just the
3rd+7th (guide tones — the two notes that define a chord's color; a full triad reads as
muddy under one melody line). Comp envelope fractions must sum to `1.0 * MEASURE_SECS`
exactly or it drifts out of sync with the melody — regression-tested
(`comp_track_len_matches_melody_len_per_measure`). Square-wave lead gets a light
low-pass + reduced sustain so it doesn't read as harsh/clippy.

`korobeiniki_track`'s final `normalize_peak` target is `0.65`, not the more obvious
`0.85`-ish — real headroom, not style. Every note's ADSR ramps to full `1.0` gain at
the *end of attack* regardless of `sustain_level` (sustain only caps the plateau after
decay), so a busy melody has many moments near the true peak, not one.
`normalize_peak` only sees discrete sample values, not inter-sample ("true peak")
overshoot introduced when a decoder reconstructs the waveform between samples — square
waves (steep edges, strong high harmonics) are exactly the content most prone to that.
Too little margin here reads as distortion at the loudest moments, not literal clipping
you'd see in the sample data.

**Fixing a bad chord**: check every guide tone against every melody note in that
measure as a semitone class (0-11) — a clash is exactly `1` or `11` apart. Only OK if
the *next* chord resolves it; a clash that just sits there for a full measure is a bug.
Watch for the subtler version too: a chord can be clash-free yet still not resolve
where the *melody itself* actually lands (e.g. the closing note) — check what the tune
is really centered on, not just adjacent notes. Prefer octave-shifting a guide tone (12
semitones) for smoother voice-leading over adding a 3rd note. Iterate by listening
(`mise run run tetris`) — code-reading alone won't tell you if a harmony works.

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
`next_frame().await` after the two heaviest chunks (`intro`, the 4-explosion `clear`
batch) — cut it to 1 long task of 119ms. `next_frame()` needs `Sfx::load` to only run
inside the real windowed loop (`amain`), never `run_headless` — it has no event loop to
yield into there.

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

## Running

```bash
mise run run tetris                                    # native
mise run build-wasm tetris                              # WASM → dist/tetris/
HCG_SEED=1 target/release/tetris --no-ui --once --variant bag7 --debug
```
