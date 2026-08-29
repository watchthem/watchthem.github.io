# Labyrinth

A bot explores a procedurally generated maze it can only see one corridor at a time.
The minimap renders **only what the bot has worked out** — never the true maze. Pseudo-3D
DDA raycaster first-person view with a knowledge-driven minimap inset.

Build plan / history: `.notes/labyrinth_plan.md`. Phases 0–6 shipped (scaffold → maze gen
→ knowledge+solver → raycaster+anim → themes → effects → 4 modes). Phase 7 (site copy,
`aiideas` pointer done, wall-tile cost check, WASM browser verify) tracked in
`.notes/labyrinth_todo.md`.

## Source layout

| File | Contents |
|------|----------|
| `src/maze.rs` | `Maze`, `EdgeKind` {Wall,Open,Mirror,Locked(u8)}, `generate` (randomized-DFS + braid), `add_dungeon`/`add_mirrors`, `toggle_wall` (fog mutation), `path_to`, `WallGrid` (dense `(2w+1)×(2h+1)` solid+mirror grid), `bfs_from_start` (metric only) |
| `src/know.rs` | `Knowledge` — per-edge state + `phantom`/`busted`/`locked`+`key` bits, Tremaux `visits`, per-cell `stale` tick, `pruned`, `keys_held`/`keys_seen`, `exit_seen`/`exit_sight`, `reveal` (visibility feed), `hit_mirror`, `stale_memory_wall`, `look_hint` |
| `src/solver.rs` | `Solver::choose_move(&Knowledge, wander) -> Option<usize>` — exit→frontier→key→(fog)wander. **Never takes `&Maze`.** |
| `src/game.rs` | `Game`, `Mode` (TowerClimb/DungeonCrawl/Fog/Mirror), `Outcome`, `tick`, mode-aware `dims_for`/`step_cap`, `tick_fog` (mutate + roam exit), `optimal_ratio`, `result_line` |
| `src/anim.rs` | `Anim` state machine `Idle→Turn→Walk→Idle` / `Idle→Turn→Bump→Idle`, `look_toward` idle glance, `dir_yaw` |
| `src/view/mod.rs` | layout rects (`stage_rect`, `minimap_inset`, `HEADER_H`) |
| `src/view/raycast.rs` | DDA column caster → textured strips + fog wash; one-bounce mirror reflection + sheen |
| `src/view/minimap.rs` | `Knowledge` renderer (cached) — floor/walls/frontier/pruned/keys/locks/phantom/fog-fade; `draw_bot` live marker |
| `src/view/theme.rs` | `ThemeKind` ×5, `Theme::build` (128px hash-noise wall texture + palette), `HCG_LABYRINTH_THEME` dev override |
| `src/view/fx.rs` | `Fx` — light flicker (folded into raycaster shading), depth-parallax motes, exit glow, `draw_floor_wipe` |
| `src/lib.rs` | `conf`/`start`/`play`/`play_until_exit`/`amain`, `fresh` (maze+grid+theme+fx), render loop, V-cycle, `--debug`/`--once`/`--no-ui`/`--variant` |
| `src/main.rs` | `fn main() { labyrinth::start() }` |

## The load-bearing invariant

**The minimap renders `Knowledge` and nothing else; the solver reads `Knowledge` and
nothing else.** Enforced by construction: `solver::choose_move` takes `&Knowledge`, never
`&Maze`. The single place `Maze` and `Knowledge` meet is `Game::tick` calling
`Knowledge::reveal(&maze, LOS_RADIUS, tick)` — the per-tick visibility feed. The raycaster
*does* read `Maze` (via `WallGrid`) — it's a spectator camera on ground truth, and it
agrees with the minimap because both are fed by the same corridor line-of-sight.

## Maze model

- `generate`: randomized-DFS perfect maze, then **braid** (~0.15–0.25 of dead ends get one
  extra wall knocked out → loops). Loops make the frontier choice a real decision.
  Deterministic from `macroquad::rand` — `rand::srand(seed)` before `generate`.
- `EdgeKind` = **Wall | Open | Mirror | Locked(key)**. Three predicates:
  `is_open` (geometry — a locked door counts, a mirror doesn't), `transparent`
  (line of sight — only `Open`), `passable(keys)` (movement).
- `WallGrid`: cell centers on odd coords (always floor), pillars even/even, edge squares
  floor iff `is_open` (so a locked doorway reads as a passage); mirror edges flagged
  separately for the raycaster to reflect off.

## Knowledge & reveal

- `reveal`: from the bot cell, march straight down each `transparent` corridor up to
  `LOS_RADIUS` (6), `reveal_cell` each; stop at a wall/bend/door/mirror. Then
  `update_pruning`.
- **`is_frontier(c)` needs a *believed-open way in*** plus an unexplored edge. A cell
  glimpsed only through a wall edge (which happens when four neighbours' reveals fully
  surround a dead-end pocket) is **not** a frontier — routing to it would just fail.
- **`spot_keys` looks one cell deep through open edges** — same dead-end-pocket problem:
  a key in a pocket the bot never directly `reveal_cell`s would otherwise never be seen.
- **Pruning is reverse-reachability** from frontiers + exit + seen keys + locked-door
  cells + the bot itself + start, over believed-open edges. Anything unreached and seen is
  pruned. The bot is always a keep-source, so pruning can't strand it. (An earlier
  "don't transit start" rule stranded the bot whenever its only route to a frontier passed
  back through start — removed.)

## Solver

`choose_move(&Knowledge, wander)`, in order: **exit** (seen + reachable) → nearest
**frontier** → **key** we've spotted but don't hold (Dungeon) → **wander** to the stalest
neighbour (Fog only; the decaying map can leave no frontier) → `None` = `Stuck`.
Equal-length BFS paths break on lower cumulative Tremaux visits.

Base-explore measured (200 seeds, gen 1): 100% win, optimal ratio median 1.49, coverage
78%. The wasted walk is the content — ~1.0× optimal means LOS is too generous.

## Modes (`V`-cycle, `--variant tower|dungeon|fog|mirror`)

| Mode | Mechanic | Notes |
|------|----------|-------|
| **TowerClimb** | Endless stacked floors, exit = stairs up, `dims_for` ramps hardest | `fx::draw_floor_wipe` during the between-floor hold. Win rate ~100% at low gen — the wandering is the point. |
| **DungeonCrawl** | `add_dungeon` places locks on the solution path, each key in a region reachable *before* its lock → solvable by construction. Solver collects keys in forced order. | Minimap: lock bars + key diamonds, `keys n/N` HUD. |
| **Fog** | Maze mutates out of sight (`toggle_wall`, connectivity-checked/reverted); exit roams (`roam_exit`, skipped when the bot is within 4). The bot keeps its layout memory but it fades on the minimap and is sometimes now **wrong** → `stale_memory_wall` soft-bump. | The only "can lose" mode: ~59% win. Memorising the map doesn't help. |
| **Mirror** | `add_mirrors` turns corridor-facing walls into mirrors. LOS reads a mirror as `Open` + sets a `phantom` bit; the bot routes at it, `Bump`s, `hit_mirror` corrects to wall and sets `busted` so re-reveal can't re-fool. | Raycaster reflects one bounce + draws a sheen. `bumps` HUD. |

## Rendering

- Raycaster: `COL_STEP` 2px (~450 rays at 900px), FOV plane 0.66, DDA on `WallGrid`,
  `draw_texture_ex` 1px-wide slices, per-strip translucent fog wash (multiplicative
  darkening alone can't tint toward the theme fog colour). Floor/ceiling are two
  `v_gradient` rects. Mirror: `cast` marches, reflects the ray component on the hit axis,
  marches again, sums the distance; a cool sheen strip is sized to the *glass* distance.
- Minimap: `RenderCache` (inset ≈220px), `mark_dirty` on `know.dirty` + resize. `draw_bot`
  live every frame with the anim's interpolated pos/yaw. North-up. Fog mode fades cells by
  `age`.
- Themes: one 128px wall texture per `ThemeKind`, hash-noise grain seeded by
  `screenshot::seed()` (**not** `macroquad::rand` — must not perturb the maze stream),
  `FilterMode::Nearest`, ~2ms. Rebuilt per generation.
- `prewarm_glyphs` before any `RenderCache` — the font-atlas-corruption gotcha (root CLAUDE.md).
- Idle glance: `anim.look_toward(dir_yaw(know.look_hint()))` eases the view down the next
  corridor between steps so it isn't frozen on a wall.

## Gotchas

- `gen` is a reserved keyword in edition 2024 — don't name a variable `gen`.
- RNG tests take `crate::rng_guard()` — `macroquad::rand` is process-global and cargo runs
  tests in parallel.
- Fog's `optimal`/`optimal_ratio` go stale once the exit roams (reported but meaningless there).

## Running

```bash
mise run run labyrinth                                        # native
mise run run-bundle labyrinth                                 # via the merged hcg binary
cargo run --release -p labyrinth -- --no-ui --once --variant dungeon
HCG_SEED=42 cargo run --release -p labyrinth -- --no-ui --once --variant fog --debug
HCG_LABYRINTH_THEME=backrooms HCG_SEED=3 HCG_SCREENSHOT=x.png ./target/release/labyrinth --variant mirror
```
