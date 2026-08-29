# Labyrinth

A bot explores a procedurally generated maze it can only see one corridor at a time.
The minimap renders **only what the bot has worked out** — never the true maze. Pseudo-3D
raycaster first-person view with a knowledge-driven minimap inset.

Build plan / history: `.notes/labyrinth_plan.md`, remaining work: `.notes/labyrinth_todo.md`.
Phases 0–5 shipped (scaffold → maze gen → knowledge+solver → raycaster+anim → themes →
effects). Phase 6 (modes) and phase 7 (site copy, wall tile) are still open.

## Source layout

| File | Contents |
|------|----------|
| `src/maze.rs` | `Maze` (per-cell 4-bit open mask), `generate` (randomized-DFS + braid), `WallGrid` (dense `(2w+1)×(2h+1)` bool for the raycaster), `bfs_from_start` (metric only) |
| `src/know.rs` | `Knowledge` — learned edges (unknown/open/wall), Tremaux `visits`, `pruned` (reverse-reachability dead-end fill), `exit_seen`/`exit_sight`, `reveal` (the visibility feed), `look_hint` |
| `src/solver.rs` | `Solver::choose_move(&Knowledge) -> Option<usize>` — exit-first then nearest-frontier BFS over known-open non-pruned edges, Tremaux tie-break. **Never takes `&Maze`.** |
| `src/game.rs` | `Game`, `Mode`, `Outcome`, `tick` (reveal → solve → step → reveal), `dims_for` generation ramp, `optimal_ratio`, `result_line` |
| `src/anim.rs` | `Anim` state machine `Idle→Turn→Walk→(Bump)→Idle` (Bump reserved for phase 6 mirror), `look_toward` idle glance, `dir_yaw` |
| `src/view/mod.rs` | layout rects (`stage_rect`, `minimap_inset`, `HEADER_H`) |
| `src/view/raycast.rs` | DDA column caster → textured vertical strips + fog wash |
| `src/view/minimap.rs` | `Knowledge` renderer (cached), `draw_bot` live marker |
| `src/view/theme.rs` | `ThemeKind` ×5, `Theme::build` (128px hash-noise wall texture + palette), `HCG_LABYRINTH_THEME` dev override |
| `src/view/fx.rs` | `Fx` — light flicker (folded into raycaster shading), depth-parallax motes, exit glow |
| `src/lib.rs` | `conf`/`start`/`play`/`play_until_exit`/`amain`, `fresh` (maze+grid+theme+fx), render loop, `--debug`/`--once`/`--no-ui` |
| `src/main.rs` | `fn main() { labyrinth::start() }` |

## The load-bearing invariant

**The minimap renders `Knowledge` and nothing else; the solver reads `Knowledge` and
nothing else.** Enforced by construction: `solver::choose_move` takes `&Knowledge`, never
`&Maze`. The single place `Maze` and `Knowledge` meet is `Game::tick` calling
`Knowledge::reveal(&maze, LOS_RADIUS)` — the per-tick visibility feed. The raycaster
*does* read `Maze` (via `WallGrid`) — it's a spectator camera on ground truth, and it
agrees with the minimap because both are fed by the same corridor line-of-sight.

## Maze + knowledge

- `generate`: randomized-DFS perfect maze, then **braid** (`braid` rate ≈ 0.15–0.25 of
  dead ends get one extra wall knocked out → loops). Loops are what make the frontier
  choice a real decision. Deterministic from `macroquad::rand` — `rand::srand(seed)`
  before `generate`.
- `WallGrid`: cell centers on odd coords (always floor), pillars on even/even, edge
  squares floor iff the passage is open. The raycaster casts against this.
- `Knowledge::reveal`: from the bot cell, march straight down each open corridor up to
  `LOS_RADIUS` (6), revealing every cell fully; stop at a wall or bend. Then
  `update_pruning`.
- **Pruning is reverse-reachability**, not a local "≤1 live neighbor" rule: multi-source
  BFS from every frontier + the exit over known-open edges, *not transiting `start`*;
  anything unreached is pruned. The local rule wrongly severs a frontier reachable only
  through a chain of dead-end-looking cells.

## Solver

1. Exit seen and reachable over known-open non-pruned edges → BFS straight to it.
2. Else → BFS to the nearest frontier cell. Equal-length paths broken by lower cumulative
   Tremaux visit count (spread out, don't grind one branch).
3. Nothing reachable → `None` → `Outcome::Stuck` (effectively unreachable with full LOS
   in a connected maze).

Measured (200 seeds, `--no-ui --once`, generation 1): **100% win, optimal ratio mean
1.88 / median 1.49, coverage 78%.** The wasted walk is the content — a bot at ~1.0×
optimal means LOS is too generous.

## Rendering

- Raycaster: `COL_STEP` 2px (~450 rays at 900px), FOV plane 0.66, DDA on `WallGrid`,
  `draw_texture_ex` 1px-wide texture slices, distance fog as a translucent per-strip
  wash (multiplicative darkening alone can't tint toward the theme fog color). Floor +
  ceiling are two `v_gradient` rects, not per-pixel cast.
- Minimap: `RenderCache` (inset ≈220px), `mark_dirty` on `know.dirty` (set by `reveal`
  when it learns something) + resize. `draw_bot` is live every frame with the anim's
  interpolated position/yaw. North-up.
- `prewarm_glyphs` before the cache — the font-atlas-corruption gotcha (root CLAUDE.md).
- Themes: one 128px wall texture per `ThemeKind`, hash-noise grain seeded by
  `screenshot::seed()` (not `macroquad::rand` — mustn't perturb the maze stream),
  `FilterMode::Nearest`, ~2ms to build. Rebuilt per generation (`fresh`).
- Idle glance: between steps `anim.look_toward(dir_yaw(know.look_hint()))` eases the view
  down the corridor the bot will likely take, so screenshots/spectators don't catch it
  frozen against a wall.

## Animation

`Idle → Turn → Walk → (Bump) → Idle`. A step is turn-in-place then walk-forward, each
smoothstepped, with a sine head-bob. The solver decides on the tick; the anim plays it
out. The loop only `tick`s again once `anim.is_idle()` + a short `TICK` dwell — so every
turn/walk is visible start to finish. `Bump` is built but unused until phase 6's mirror
maze.

## Running

```bash
mise run run labyrinth                                   # native
mise run run-bundle labyrinth                            # via the merged hcg binary
cargo run --release -p labyrinth -- --no-ui --once       # headless, prints result= line
HCG_SEED=42 cargo run --release -p labyrinth -- --no-ui --once --debug
HCG_LABYRINTH_THEME=backrooms HCG_SEED=3 HCG_SCREENSHOT=x.png ./target/release/labyrinth
```
