# Labyrinth

A bot explores a procedurally generated maze it can only see one corridor at a time.
First-person DDA raycaster view plus a minimap that renders **only what the bot has
worked out** — never the true maze. Four modes, each with its own theme. Leftovers:
`.notes/labyrinth_todo.md`.

**Lessons are recorded as "X read as Y"** — most of this game's tuning was by eye/ear
from user reports. Don't re-try a rejected approach without a new reason.

## Source layout

| File | Contents |
|------|----------|
| `src/maze.rs` | `Maze`, `EdgeKind`, `generate` (DFS + braid; `rooms > 0` → `maze/office.rs`), `Door`/`Room`/`name_rooms`, `add_dungeon`/`add_mirrors`, Fog's `unborn`/`materialize`/`dissolve`, `WallGrid`, `place_stairwell` |
| `src/maze/office.rs` | Tower floor plan: `halls` (corridor lattice), lift lobby + nooks (`stubs`), `fit_rooms`, `ascii` (test dump) |
| `src/know.rs` | `Knowledge` — per-edge bits (open/door/shut/locked/phantom/busted/sealed), Tremaux `visits`, `stale` tick, `pruned`, keys, `reveal` (visibility feed), `glimpse` (Fog), `forget`, `hit_mirror` |
| `src/solver.rs` | `Solver::choose_move(&Knowledge, wander)`, `door_to_open`. **Never takes `&Maze`.** |
| `src/game.rs` | `Game`, `Mode`, `Outcome`, `tick`, `dims_for`/`step_cap`, Fog `grow_fog`/`tick_fog`, the `Boss`, `door_to_close`, `result_line` |
| `src/anim.rs` | `Anim` state machine + `AnimEvent`s; `scan` (`Look`s), `open`, `face`, `descend` |
| `src/lib.rs` | entry points, `fresh`, `Shown` (presentation state lagging `Game`), junction-scan assembly, render loop, V-cycle, CLI |
| `src/banter.rs` | Boss fight script: `BOT_JABS`, `BOSS_RETORTS`, `LADY_RETORTS`, `CONCESSIONS` (≤34 chars) |
| `src/sound.rs` | `Sound` — every clip synthesized at startup on `lib/audio`; `Ear` (walking-distance hearing) |
| `src/view/raycast.rs` | column caster → sloped strip quads; mirrors; ground/clumps/trees/sky; returns `View` (camera, `zbuf`, `roof`) |
| `src/view/raycast/doorway.rs` | office doors + dungeon gates as framed openings with thick leaves (**Doorways**) |
| `src/view/decor.rs` | `Decor` — set dressing as `Block`s, per-column painter, overlays, `reads_at`/`looks_at`/`aim`, `chain_near` |
| `src/view/label.rs` | in-world text: `Spec` baked to a texture, `draw_on_wall` per column; `Occluders` |
| `src/view/keys.rs` | Dungeon keys, drawn after decor, clipped per column |
| `src/view/critters.rs` | White Rabbit, crows (+ Fog breadcrumbs they eat), owls, rats — own RNG, billboards, `take_calls` → `Sound::bird` |
| `src/view/fx.rs` | `Fx`: lamps (panels, torches), `Lighting`, motes, `draw_floor_wipe` |
| `src/view/fog.rs` | Forest `Mist`, `premultiplied_material`, `fog_preview` (ignored test) |
| `src/view/theme.rs` | `ThemeKind` (one per mode), wall textures, palettes, spruce/clump/blossom painting |
| `src/view/minimap.rs` | `Knowledge` renderer (cached), `draw_bot` |
| `src/view/boss.rs`, `hud.rs` | boss HUD (the only health bar), HUD text placement |
| `src/util.rs` | `sort_by_key` — the crate's only sort (see Gotchas) |

## The load-bearing invariant

**The minimap renders `Knowledge` and nothing else; the solver reads `Knowledge` and
nothing else.** `choose_move` takes `&Knowledge`, never `&Maze`. The one bridge is
`Game::look` → `Knowledge::reveal(&maze, LOS_RADIUS, tick)`. The raycaster reads `Maze`
(via `WallGrid`) — a spectator camera on ground truth; it agrees with the minimap
because both follow the same corridor line of sight.

## Maze model

- `generate(w, h, braid, locks, rooms)`: randomized-DFS perfect maze → `place_exit`
  (random cell in the far half by walking distance) → `add_dungeon(locks)` → braid
  (~0.15–0.25 of dead ends opened → loops). Doors go in while the maze is still perfect,
  and the braid reverts any knock where `door_bypassed` (a door shut *on its own* no
  longer cuts off the exit — the first version checked doors together and the bot won
  with 62/80 keys). Deterministic from `macroquad::rand`.
- `EdgeKind` = Wall | Open | Mirror | Locked(key) | Door | Sealed. `is_open` =
  geometry (locked/office door count, mirror/sealed don't), `transparent` = line of
  sight (`Open`, or an office `Door` while `open`), `passable(keys)` = movement.
- `WallGrid`: dense `(2w+1)×(2h+1)`; cells on odd coords, pillars even/even, edge
  squares floor iff `is_open`; mirror flags; rooms clear their interior pillars. The
  raycaster's world; `Knowledge::reveal` also uses it for sight through doorways.
- **Thin walls** (Tower + Dungeon, `Maze::thin_walls`; Garden hedges and Forest trees
  keep full squares): a solid square is a `WALL_T` 0.2 slab — a post plus an arm toward
  each `mass` neighbour (solid or doorway), `WallGrid::slabs` — the rest of it floor. A
  cell's centre is `face_dist` 0.9 from a wall face (0.5 for full squares;
  `Maze::wall_face`, `Decor::face`). `WallGrid::blocks(p)` is the point test matching
  what's drawn (slabs, doorway frames minus the opening). `march` tests the starting
  square's slab too (a camera can stand on the floor beside one — the descent does).

### Office floors (Tower, `maze/office.rs`)

A floor plan, not a maze — DFS offices read as mazes (dead ends everywhere, corridors
side by side).
- **Hall = corridor lattice**: main lines 2–4 cells apart (1–2 rooms back to back
  between), cross lines 3–6 apart, axis random; outer margins 0/2/3 (never 1). Segments
  are dropped (cross 40%, main 12%) only while the hall stays one piece with no dead
  end (`sound`) and every cell stays within `REACH` 3 of a corridor (`orphans`).
- **Dead ends are purposeful**: entrance stub (start top-left, reception), the **lift
  lobby** (one-cell stub ≥ ¾ of max hall distance from start = the exit; ½ gave short
  optimal routes), 0–2 **nooks** (decor: kitchenette, or 30% a lone desk with a red
  stapler). Doors never open onto the lobby or a nook.
- **Rooms tile the rest** (`fit_rooms`): boss (1×1), the WC pair (two 1×1 side by side,
  doors on one wall), then a reading-order sweep in `SIZES` shapes (1×1; 1×2 peek-only,
  door on the short end; 2×2, 3×2, 2×3, 3×3). Unreachable leftovers are **core**
  (`Maze::is_core`, ~2%; `bfs_from_start` allows them; decor skips them). ~40 rooms at
  gen 1. One door per room (8%: a second on the same wall — never a shortcut); none
  `Sealed` but the out-of-order WC on half the floors, an **occupied** WC
  on 70% of the rest (`Door::occupied`: an ordinary door the bot tries once — `Game::knocked`,
  a muffled man's/woman's "occupied" by the door's sign (`Maze::wc_door`, `Sound::busy`,
  captioned `boss::draw_busy`) — then `Maze::seal`ed, so `Knowledge` learns it `Sealed` and
  never retries; `occupied_washroom_is_tried_once`). WC doors always render as ordinary
  wooden doors; out-of-order keeps its sheet. **Liminal**: 55% of conference rooms and
  open spaces are `Room::vacant` (decided in `office.rs`, with `Room::style` — the
  decor layout pick — so two in three would-be zen rooms go vacant too; it was
  everywhere): bare floor, nothing on the
  walls, **no name plate** (`Maze::door_marked`). `Knowledge` learns a shut unmarked door
  as `plain` and treats it as a wall until opened; only small rooms' doors
  (`Maze::door_peeked`: peek-only or unmarked) are opened from the hall to look in — an
  open space's is opened only by walking in through it; the solver goes for frontiers and
  marked doors first, unmarked ones only when nothing else is left (`door_to_open`,
  `unmarked_doors_are_a_last_resort`). Empty rooms alone didn't read as liminal — the
  bot kept stopping to open each one. Kinds by size, no RNG
  (`name_rooms`): 1×1 Boss, Janitor, then Conference; 2-cell Kitchen, It, then
  Conference; bigger OpenSpace; the pair `Wc`. `Maze::lady_boss`: 1 floor in 5.
- **Peek-only rooms** (≤2 cells) are opened from the doorway, never entered — their
  furniture may fill the floor.
- Tests: `office_is_a_floor_plan` (adjacent hall cells always joined, no all-hall 2×2,
  connected, ≤4 dead ends, lift degree 1), `rooms_are_detours`,
  `washrooms_back_to_back`. Dumps: `cargo test -p labyrinth --release -- --ignored
  office_floors --nocapture`; `office_stats` also prints a fingerprint proving other
  modes' mazes unchanged (`generate` keeps the old office RNG draws when `rooms == 0`).

## Knowledge & reveal

- `reveal`: `reveal_cell` the bot's cell, then march each `transparent` corridor up to
  `LOS_RADIUS` 6. **Office rooms**: standing inside reveals the whole room (open plan —
  walking every cell was wasted: Tower gen 1 median 62 → 48.5 steps). From the hall at
  an open door, only cells whose centre is in sight through the `DOOR_OPENING` gap and
  past the walls as drawn (`sees_through_door`, `WallGrid::blocks`) — still only the
  line straight in (the opening allows ±0.27 per unit out; the next column over is 2
  units across); revealing the whole room from there mapped cells hidden behind walls, so the bot
  never stepped in (`doorway_shows_only_what_is_in_line`).
- Fog: `glimpse` also reveals every born cell within `FOG_GLIMPSE` 2.3 (the spruces are
  sparse; the view shows the next corridors).
- `is_frontier` needs a *believed-open way in* plus an unknown edge (cells glimpsed only
  through walls aren't goals). `spot_keys` looks one cell deep through open edges (keys
  in never-entered pockets).
- **Pruning** = reverse reachability from frontiers, exit, seen keys, locked doors, the
  bot and start over believed-open edges. The bot is always a keep-source.
- Office doors: `shut` bit = a closed, openable door not yet `done` (see **Doors**).

## Solver

`choose_move`: exit (seen, reachable, unless `exit_locked`) → spotted key (take it as
soon as seen) → nearest frontier or shut office door → Fog `wander` to the stalest
remembered reachable cell by BFS (never "stalest neighbour" — ping-pongs,
`fog_never_ping_pongs`) → `None` = Stuck. Equal-length paths break on lower Tremaux
visits. `door_to_open` fires first when the bot stands by a shut door. Base maze, 200
seeds gen 1: 100% win, ratio 1.49 — the wasted walk is the content.

## Modes (`V`-cycle, `--variant tower|dungeon|fog|mirror`)

Theme is fixed per mode (Tower=Backrooms, Dungeon, Fog=Forest, Mirror=Garden); the
header shows only `Mode::title()` (two names side by side read as nonsense). **The 3D
view must show each mode's mechanic**, not just the minimap.

| Mode | Mechanic |
|------|----------|
| **Tower** | Endless floors, office plans, boss before the lift, `draw_floor_wipe` between floors. |
| **Dungeon** | `add_dungeon` (2 + gen/2 locks on cut edges of the solution path, each key reachable before its lock). Iron-bar gates, floating keys, `keys n/N` HUD, stairwell exit. One key per cell. |
| **Fog** | **The maze only exists near the bot**: `grow_fog` `materialize`s cells within `FOG_SIGHT` before every reveal; cells unseen for `FOG_FORGET` 30 ticks are forgotten **and** `dissolve`d, so revisited ground comes back different (the exit may move). Invariant: the grid is connected when undecided edges count as open → never walled in (`fog_grows_lazily_and_never_encloses`). `forget` keeps edges shared with remembered cells (clearing them made fake frontiers → oscillation). Memory is never wrong (`fog_memory_is_never_wrong`). `Game::walls_changed` → rebuild `WallGrid`. `optimal` is a Manhattan bound. Thick mist (`fog::FOREST.density` 0.45, `TREE_RANGE` 8) with sight matched to it (`FOG_LOS` 4, `FOG_SIGHT` 5 — the other modes keep `LOS_RADIUS` 6), so ground changes closer by. Breadcrumbs: a fine trail, one every `CRUMB_STEP` along the path walked (`Critters::drop_crumb`, per frame); crows glide down and peck them up, scatter when the bot comes back, and a crumb goes with dissolved ground — dressing only. Not a losing mode: the bot always gets out, sometimes after a long walk (200 seeds gen 1: 100% win, worst 992 steps ≈ 3·w·h); No `step_cap` (`u32::MAX`); the other modes keep 8·w·h as a runaway guard. |
| **Mirror** | `add_mirrors` turns corridor-facing walls into glass. LOS reads a mirror as open + `phantom`; the bot knows one by its reflection: `see_reflection` corrects glass straight down its facing or chosen direction (plays as `bump`/`Anim::face`); only side glimpses fool it; `hit_mirror` sets `busted`. `bumps` HUD. Exit = gate in the outer hedge; reaching it takes one uncounted step out (`Game::leaving`). |

## Bot behaviour (presentation)

`Game::tick` decides; lib.rs's `Shown` presents it on animation events, so the view
lags the game: a key vanishes on the `Footfall` onto it, a gate opens after the key
turns, the exit chimes on arrival. Ticks run only when `Anim` is idle.

- **Anim**: a cell = two strides at constant pace (0.56 s; `Step` at halfway = sound,
  `Footfall` on arrival = dues) — per-cell easing bunched footfalls into pairs. Turns
  0.3 s/quarter, feet shuffle (`stride`) so the Mirror reflection steps round; a zero
  turn does nothing, <11° shuffles no feet. `eye_lift` = bob − sink − stoop (head-bob is
  a real eye-height change, so near geometry bobs more).
- **Junction scans**: on the first-visit `Footfall`, `Anim::scan` turns to each side
  corridor (`LOOK_TURN` 0.45 s, `LOOK_HOLD` 0.5 s), plus reads and sights, sorted into
  one side-to-side sweep. A scan that pitched or stooped ends with a level look
  (`scan_pitch_returns_level`). Idle glances between steps read as a twitch — removed.
- **Reads** (`Decor::reads_at` → `Glance`s keyed by wall): posters, notices, Pepe/Cake
  whiteboards, worded garden signs, the OUT OF ORDER sheet — `READ_HOLD` 0.7 s, aimed at
  the text itself. Skipped straight ahead, already read (`Shown::read`), or TPS sheets
  (too common).
- **Garden sights** (`Decor::looks_at`): boxes from the fitting just placed
  (`Decor::sight`) — tea table, fountain, checkerboard, chess figures, flamingo, the
  DRINK ME bottle (read the sign, then stoop and look down) — plus grins (`face`d: never
  looked at edge-on). **Roses and clocks are not sights** (too many). Offered from the
  cell or an open neighbour wherever `aim` fits it; skipped behind (>2.3 rad), seen
  (`Shown::seen`), or straight ahead at eye level.
- **`decor::aim`**: the view shows one world unit a view-height tall at depth 1, so
  from a cell centre anything >~0.45 units is cropped. `aim` centres the box, pitching
  up to `MAX_PITCH` 0.3 (y-shear smears past that) then stooping up to `MAX_STOOP` 0.2,
  and returns `None` unless the whole box fits within `FILL` at its near edge.
- **Doors**: a shut office door on the bot's cell is opened from the doorway first
  (`door_to_open` → `opened_from_doorway`, `Anim::open`: turn, push, hold, stay); the
  junction scan waits (`Shown::pending_looks`) until it's open — and, for the boss,
  until the argument is over. `Anim::open(dir, after, lean)` then plays the glances: a room seen whole
  from the doorway (`Game::room_known`) — and the IT room and janitor's closet, plus a
  read of their poster — leans half a cell in (`LEAN_REACH`, `Phase::Lean`), glances at each side wall
  (`PEEK_TURN` 0.55 rad, `PEEK_HOLD` 0.25 s), steps back, then the door is pulled to;
  at the boss's door the bot leans in and
  stays there (`stay_leaning`, no glances — eyes on him; the tick waits for an idle anim,
  so it must be idle while leaned in), then backs straight out without turning
  (`step_back`) on the tick he concedes (`boss_lean_stays_until_step_back`);
  the closet sting plays as its door swings. **Closing**: back in the hall by an open
  door whose room is fully known, `Game::door_to_close` fires first in the tick (door
  `open = false, done = true`, `closed_door` → `Anim::open` + `Shown::swinging` −1).
  `done` doors aren't `shut` to `Knowledge`, so they're never reopened to look
  (`bot_closes_doors_behind_it`).
- **The boss** (Tower): opening the boss door starts an argument — `ARGUE_TICKS` 48 at
  `ARGUE_PACE` 0.17 s, HP `boss_hp_at(t)` from 9001, alternating voices, then a
  `CONCESSIONS` banner. The HUD shows from tick 1 (tick 0 lasts the door swing and lean
  in — the bar sat there silent before the first line); THE BOSS title on a dark plate
  (bare gold text vanished into the ceiling panels). `Knowledge::exit_locked` keeps the solver off the lift until
  then (`boss_before_lift`). The lift waits (`Gate::Push`) while its doors slide open.
- **Dungeon exit**: `Anim::descend` turns to the stairwell, looks down (`pitch` 0.35),
  then walks through the arch (`Maze::wall_face` away) and down the flight to tread
  `DESCEND_TREAD` 6, the eye dropping a
  `STAIR_RISE` per tread under it (`stair_sink`, `maze::STAIR_*` shared with
  `Decor::stairwell`; ends ~0.33 below the floor, under the vault) while the pitch eases
  to 0.12; the win hold fades to black (`descent_goes_down_the_flight`). The old 0.2
  sink over a fixed pace barely read as going down.

## Rendering

### Pipeline

Order per frame: `raycast::draw` (sky/floor, ground, walls + doorways, mirrors, trees
or clumps) → decor → keys → lamps → motes (non-forest) → minimap/HUD. Forest instead
sorts critters, campfire and motes among the trees far→near (`Props::amid`,
`draw_trees_between`) — painter's order is the trees' only occlusion.

- **Wall pass = batched sloped quads**: one `Strip` per `COL_STEP` 2 px column; its
  right edge takes the next column's depth when it strikes the same `Surf` (grid line +
  side + mirror image, or a doorway plane), else extrapolates in 1/depth. Among thin
  walls `Surf::line` is `slab_line` (`4·square + 1|3` for a slab face, `4·boundary` for
  a full block's face; `WELL_LINE..+3` for the stairwell flight) — `raycast::march`
  ray-tests a thin square's two bars (`slab_hit`) and marches on past a miss. Flat-topped
  rects made every edge a 2 px staircase. Four `MeshBuf`s — walls, door furniture, fog
  wash, front — must reach the screen in that order; one about to overflow flushes the
  ones beneath first (`room_for`).
- FOV plane half-width = `area.w / 2·area.h` (px-per-unit equal on both axes).
- Per-strip translucent fog wash (darkening alone can't tint toward the fog colour).
- `Lighting::at(hit)` per strip: Backrooms ceiling panels (1 in 9 `Blinky`, stutters
  hashed from `t`), Dungeon torches (ambient 0.5 + falloff 4.5; a fifth `Spent`). Lamps
  hide behind decor via `Decor::front` (`Occluders`). Light scales a surface's colour
  (`lit_at`); the fog weight is distance only (`fog_at`) — folded together, a dark
  Blinky washed its stretch of office in the yellow-grey fog instead of darkening it.
- Theme textures seed from `screenshot::seed()`, **never `macroquad::rand`** (would
  perturb the maze stream). Rebuilt per maze.
- Ceilings (`Theme::ceiling`, `raycast::CeilingCast`: world rects on z = 1, near-clipped,
  drawn before the walls): Dungeon = inset flagstone polygons (a gradient read as night
  sky) — per square, or among thin walls three per cell from wall middle to wall middle
  (a joint along the corridor's middle read as a black wedge). Tower = drop ceiling
  (`draw_office_ceiling`): 1/3-unit acoustic tiles in a T-bar grid (the palette gradient,
  joint-coloured, fading to fog), lit per tile (pooling round each panel, `Lighting::at`,
  fog by distance), 3% stained; each cell's light panel fills the 2×2 tiles round its
  centre (`Lighting::panels`, lit/dead per frame). The flat gradient read as a painted
  backdrop, and the old screen-facing trapezoid panels (fx) didn't turn with the view.
- Motes are world-space (`MOTE_BOX` tiles around the camera), outdoor themes only.
- HUD is translucent over a full-canvas stage (header 0.45 alpha, minimap 0.5).
- Minimap: `RenderCache`, dirty on `know.dirty` (every tick in Fog — `age` fades
  floor **and walls**). `prewarm_glyphs` before any `RenderCache`.
- Exits are dressing, no light column: lift (Tower), stairwell (Dungeon), campfire
  (Fog, re-placed each frame — the exit moves), hedge gate (Mirror,
  `WallGrid::open_outside` → horizon).

### Doorways (`raycast/doorway.rs`)

Office doors and dungeon gates alike. The doorway square stays open in `WallGrid`; each
is drawn as a `WALL_T`-deep frame across it with an `OPENING` 0.62 × `DOOR_H` 0.82 hole
— wall beside and above, casing (office), lined jambs, soffit — and a leaf of real
thickness (office 0.045, gate bars 0.034) hinged on a jamb at the room-side face
(`GAP` proud). An office door swings right round (`OPEN_ANGLE` π, `Props::swing`) to lie
flat along the room-side wall face — at a quarter turn it stood out across half the view
of a doorway peek. Gates swing on `door_swing`. A shut office leaf laps `LAP` past both
edges of the opening (hidden behind the wall face) — cut to the opening, the `GAP`
either side showed the room through a sliver a few pixels wide.
- `door_column` walks the view-culled doorways a ray crosses, nearest first, into
  `Piece`s (bands between `Bound`s, sloped like strips). A ray through the opening
  leaves a `clip` (soffit edge) that everything behind is cut to, so bands never
  overlap. `View::roof`: decor and `Occluders` clip to the head.
- A leaf outside its frame (open, mid-swing) is its own event (`free_leaf`); where it
  stops the ray it leaves `under` (its top edge): strips and pieces behind are cut to
  above it, and `View::floor_clip`/`unders` do the same for decor, lamps and labels.
  **Never lower `zbuf` to the leaf** — that hid the room's ceiling panel above the leaf.
  The in-frame and free cases must partition `l0` exactly (a 1e-4 gap at the room-face
  plane left a see-through column mid-leaf while it swung).
- Office leaf: wood grain, two raised panels with lit/shadowed bevels, a **3D lever
  handle** on both faces (`HANDLE` boxes, ray-tested, `Look::Front`) with its shadow —
  a painted handle read flat. Steel frame (`TRIM`; wood-on-wood read as one brown mass).
  No grey "sealed" look any more: every office door opens (a locked one was a dead end
  dressed as a way in) bar the out-of-order WC's, which looks like any other door. Name
  plate rides the leaf (`sign_mount`). Door plates and all wall labels fade with distance
  like the walls (`label::Fade`: plate darkened + fog wash, bare text faded out) and
  aren't drawn once lost in the murk (a bright plate floated where its door had gone
  dark).
- Gate: 7 round bars (the ray stops on one, shaded by its offset from the axis), straps
  and a lock box in the key's colour as `Look::Front` pieces — the corridor shows
  between the bars. Hinge side hashed. Leftover: decor/keys behind a shut gate can paint
  over its straps (zbuf there is the far wall).
- `leaf_shuts_the_opening_and_opens_flat_against_the_jamb` pins the geometry.

### Mirrors

Rays reflect off up to `MAX_BOUNCES` 5 glasses; `MirrorHit::image`/`flip` is the camera
reflected across that glass and every one before (glass lines are exact grid lines →
bit-identical per chain), and `View::mirrored` renders the real world from it.
`glass_runs` groups columns per (level, glass, image); ground passes draw shallowest
first, then per level deepest first: clumps, reflected dressing (`Props::reflect`, ≤
`DECOR_BOUNCES` 2, windowed by `glass_window`, labels read backwards on odd levels),
bot, sheen. Past 2 bounces a level gets only flat floor and plain hedges (budget).
Tint the surface per level, never the fog (tunnels converge on the horizon, not black).
Frameless glass with faint streaks. The bot's reflection is `ROBOT` (21 rounded tapered
parts ray-tested per column; boxes read blocky, egg-chested balls "grotesque").
`mirror_matches_brute_force`, `reflected_window_keeps_to_the_glass`.

### Set dressing (`decor.rs`)

- Boxes (`Block`: footprint, z-span, `Shape` Box/Cyl/Ball/Dome/Bowl, `Finish`) are
  bucketed per column and painted far→near, except `stacked` blocks (inside a nearer
  block's span on the eye's side of its top/underside, or sunk into it) follow it. The
  older whole-box order and `rests_on` support keys both mis-ordered neighbours
  (flipping cooler bottle; janitor shelf "depth perception" bug). Short features on a
  round block's face (`on_face`: in front of its cross-section at their mid-height)
  follow it too — a ball's entry is its widest girth, so brows/eyes/nose proud of the
  face above or below it entered behind it and flickered under it as the boss's head
  shook. `paint_order` moves followers (and what's stacked on them) up to just after
  the nearer block; inserting that block before its first follower instead dragged it
  under everything between (a proud brow "over" the chin put the face over the brow).
  `boss_face_features_stay_on_top` checks against a 3D ray test (`painter_errors`);
  the ignored `painter_scene_errors` totals the painter's wrong pixels across every
  office room — use it to weigh any ordering change (intersecting round parts —
  plants, beanbags, hair round a head — and conference rooms are most of what's left).
  `stacked` also needs `x` to reach a line of sight to `a`'s top/underside (`reach`):
  without it a nose "stood on" the boss's shirt (eye at 0.5 = shirt top) and the throne
  crest "on" his chest, chaining his suit and the throne back over him in dark
  stripes. A follower drags along what was placed over it in front of it
  (`in_sight_together`; boxes only — a round block's entry is its girth), or the move
  leapfrogs it (arcade cabinet body over its own screen).
- Round shapes cut the column's plane as an ellipse, up to 12 lit bands. `Glass` alpha
  0.35→0.9 at the silhouette; `Lava`; `Leafy` mottles in world space (screen noise
  swims). `Motion` blocks (boss head/fist, lava blobs) are re-placed each frame.
- **Culling**: footprints clipped at the near cut; whole fittings skipped behind the
  wall zbuf (0.8–2.5 → ~0.2 ms/frame).
- Flat boxes (`z1 < FLAT`, `z0 > -FLAT`) paint first and never occlude. Sunk blocks
  (`z0 < 0`) clip at the stairwell's arch plane (`Decor::well`).
- Overlays (whiteboard marker, door plates, Pepe Silvia board) draw after, occluded by
  zbuf + `Occluders`. All text goes through `label.rs` (screen-space text didn't
  foreshorten); `labels()` must list every string (font-atlas gotcha).
- **Thin walls**: everything was laid out against full squares. `fit`, whiteboards,
  notices, TPS sheets, torches (`fx`), the ceiling chains and `reads_at` go to the real
  face (`Decor::face`); `settle_room` moves each room fitting touching a wall back with
  it (whole, unscaled — `place` boxes scale with the floor, so a wider floor would grow
  furniture), plus wall posters, `moves` and blob axes (`lava_stays_in_its_lamp`); in a
  peek-only room everything moves back from the door, clear of the open leaf.
  **Janitor and Kitchen skip it too**: each fitting is laid out against the real walls
  with `shifted` (the full-square frame translated by multiples of `recess`, unscaled) —
  settled, the side fittings stayed half a unit off their walls and the table sat in the
  back half. **Conference rooms skip `settle_room`**: laid out in world units on the real floor
  (`Floor`: `a` from the far wall's face toward the door, `b` across, `slab`/`round_at`).
  Authored as fractions of the full-square floor they huddled centred in a room 1.8
  wide, with spindly box chairs; settled fitting by fitting, the chairs left the table.
  Chairs are the desk's (`swivel_chair`, shared with `desk`). Four conceits: 1×1 nine
  chairs round a table for two + wall-size chart + credenza/plant, or zen (CALM,
  gong, plant, sand garden, `beanbag`s); 1×2 boardroom (12 chairs, gilt throne,
  chart) or "culture" (ping-pong, arcade cabinet, GOOD VIBES ONLY neon). The lift's
  front is a steel wall of slab thickness with the doors sliding inside it.
  `chain_near` adds the recess to its radius.
- **Placement**: hashed per maze, never `macroquad::rand`, **never on a walked line**
  (`decor_keeps_the_paths_clear`): wall fittings ≤ `MAX_DEPTH` 0.22 deep, free-standing
  furniture only in room pillar squares, nothing on an office-door cell, nothing
  in a wall a torch hangs on. Rooms are sized for the ~2.5-unit doorway peek (nothing
  below the desk top reads; text faces the door, big). Nothing in a peek room's door
  sweep — the leaf's floor inside the door (`peek_rooms_keep_inside_and_clear_of_the_door`).
- **Office**: hall fittings (few copiers — they lined every wall), whiteboards facing
  15% of doors, corkboards (half with a TPS sheet), reception, nook kitchenettes. Open spaces:
  desks in the inner pillar squares plus one against a free outer wall in most cells
  (`open_space_desks`, in the thin wall's floor strip — pillar desks alone read empty);
  3 in 4 inner squares hold a back-to-back pod split by a fabric divider (`desk_pod`), a
  third of those walled in as cubicles. `open_space_fittings_do_not_overlap` checks no
  two parts share space.
  `settle_room` moves what stands within 0.3 of a moved fitting with it (a chair left
  behind its desk opened a recess-wide gap). Wall-hung fittings (off the floor, resting on nothing)
  within 0.25 of a wall snap flush to its new face — mounted a few cm off the old face
  they hung in mid-air (washroom mirror/dryer, kitchen TV);
  `room_fittings_stand_or_hang_on_a_wall` checks every office room. Rooms by
  `RoomKind`: Boss (throne, trophies, poster; the boss — ~40 rounded blocks, bald,
  scowling, cigar; the **lady boss** swaps figure + mug: blue power suit, pearls,
  blonde bouffant — gold earrings vanished against the hair, half-moon readers read as a
  grin), Janitor (shelves, mop bucket, WET FLOOR, boombox, lava lamp), Kitchen,
  Conference (ironic `MEETING_ROOMS` names), OpenSpace desk pods, IT (racks, "the
  Internet", big poster; `it_storage` rack of laptops on the desks' side — on the racks'
  side its posts hid THE INTERNET's label from the door — and a low `aquarium` with a
  turtle under that label; the turtle swims door-side of the weed or it can't be seen),
  Kitchen (fridge, counter + sink, snack machine, bin, TV — its screen used to face the
  wall for two of four door directions), WC. Nods: Swingline staplers, Dundies, MACRODATA REFINEMENT,
  427, THE CAKE IS A LIE (rare — 3% of whiteboards; more read as spam). Lift car has no `fx` panel (it painted over the doors).
- **Dungeon**: shackles, ceiling chains, barrels, crates, bones, banners, rack, iron
  maiden, stocks, stains, whip; ≤5 `DUNGEON_NOTICES` (Gachimuchi nods) — PG-16, nothing
  graphic. **Stairwell exit** (`Maze::stairwell`, the exit cell's wall ahead of the last
  step, no RNG): a plain slab, flush with the other walls, with the portal cut into it
  (jambs and voussoirs as deep as the wall, half-raised portcullis, jamb torches). The
  flight is a **box behind the portal** (`maze::Well`, `STAIR_FLIGHT` 1.0 deep from the
  arch face) that overlaps the floor behind the wall and exists only through the arch:
  `march` sends a ray that strikes the arch face from the exit side into it
  (`thin_hit` → `well_exit`), a camera inside sees its walls or out back through the
  arch, and from the cell behind it's a plain wall (`stairwell_opens_only_from_the_exit`).
  `View::through_portal` marks the columns looking in: the flight's blocks
  (`Decor::flight`) paint only there, and nothing else past the arch does
  (`View::behind_portal` — decor, torches, critters; a barrel behind the wall once
  stood in the opening). The flight's dressing is excluded from
  `decor_keeps_the_paths_clear` (it's not really on that floor).
- **Garden** (Alice): ≤3 set pieces in mirror-free cells (checkerboard + queens, tea
  table, fountain — `Decor::fountain` feeds its sound), roses half-painted white, clocks
  at six, EAT ME / DRINK ME / THIS WAY / mirror-written Jabberwocky, corner chess
  figures and flamingo+hedgehog (`CORNER` 0.13), ≤2 Cheshire grins. Hedge fittings
  stand ≥0.07 proud of the clumps; never on mirrors (dressing near glass reads doubled).

### Keys (`view/keys.rs`)

Drawn after decor, every piece cut per column by `View::visible` + `decor.front()`.
Each key then adds its span to the `Occluders` (`Decor::front_mut`), so the lamps drawn
after hide behind it — a torch on the wall behind a key painted over it.
Inside `raycast::draw` a prop *behind* a key painted over it, and centre-column
clipping made a key pop whole over a wall corner.

### Outdoor themes

- **Ground** (`draw_ground`, Forest + Garden): world-space tile mesh, near-clipped and
  subdivided near the camera, mipmapped `theme::Ground` texture + decals, vertex colour
  for tile kind/tone/stripes/fade, over a depth-banded backdrop. Plain gradients read as
  painted backdrops. Inspect textures: `cargo test -p labyrinth --release dump_art --
  --ignored` → `tmp/labyrinth_art/`.
- **Garden hedges** are foliage volumes: recessed dark core + hashed leaf-clump
  billboards per exposed face (`draw_clumps`, per-column depth test); ~half the faces
  flower, one kind per face (`face_bloom`, same draw, different atlas rect). Flat slabs
  read as walls.
- **Forest spruces** (`paint_spruce`): 6 sprites painted as 3D-projected Norway spruce
  branches (irregular whorls, drooping lower branches, sparse dead base), one
  mip-chained atlas, 3–5.5 units tall, drawn at the tree's own aspect. Even shelves read
  as a Christmas-tree icon; squeezed sprites read as lianas.
- **Forest mist** (`fog.rs`): `own·T + colour(elevation)·(1−T)`, `T` integrated along
  the real 3D ray (uniform + ground-hugging layer); the sky is the mist colour at
  infinity, so everything converges on what's behind it. Trees fade per vertex row, ease
  out past `TREE_RANGE` 11; far trees blur via mip cross-fade (`level_mix`). Premultiplied
  alpha throughout (`premultiplied_material`) — straight alpha gave pale halos or dark
  rims. Iterate offline: `cargo test -p labyrinth --release fog_preview -- --ignored`.
  (User: the old curve read as "late-2000s 3D fog".)
- **Critters**: one White Rabbit (plain white — a waistcoat read as blood), crows
  (perch, caw, fly off when startled), owls (hoot), 3 Dungeon rats (dash, sniff,
  squeak) — they use the floor strips beside thin walls (`WallGrid::solid_at` tests the
  slabs; each dash is checked along its path so it never crosses one). Depth-tested at
  the centre column only.

## Audio (`sound.rs`)

- **Triggers are what the viewer sees**: `Footfall` → footstep (+ key pickup / exit
  chime / chain rattle near `Decor::chain_near`), `KeyTurn` (key + the gate's squeal and
  clang, timed to its swing via `sound::GATE_LEAD`),
  `DoorPush` (office latch clunk, also on closing), closet sting on its door opening.
- All randomness is `audio::Rng`, never `macroquad::rand`. Volumes are consts;
  **audition offline**: `cargo test -p labyrinth --release dump_wavs -- --ignored` →
  `tmp/labyrinth_sounds/` (every clip + 8 s scenes at in-game volume).
- **No live volume on WASM** (quad-snd #30): anything distance-dependent is one-shots
  with volume picked at play time; only beds loop, made seamless by cross-faded tails
  (`beds_loop_without_a_click`), stopped while muted / on leaving / on theme change.
  Captures (`HCG_SCREENSHOT`, clips, `?stream=1`) are silent.
- Lamp sounds use **walking distance** (`Ear`: BFS over passages, max(straight, 2 ×
  route cells)) — a panel behind a wall was heard at full level.
- **Footsteps** are unpitched: heel + toe contacts of similar level, short strikes
  band-passed at 12 dB/oct, floor modes ≤6 ms and mostly >300 Hz. Stone is noise only
  (`sole_contact`: 200–800 Hz body + slap, ~460 Hz centroid, corridor reflections
  low-passed ~850 Hz). Lessons: click grains = Minecraft; a pitched sweep = kick drum;
  80–400 Hz rings = a drum; **any discrete mode sounds like struck metal** (a hammer on
  a pan); 1.1 kHz centroid was too high.
- **Ambience**: Backrooms hum (120 + 120.25 Hz beating, no overdrive — the tanh stack
  was one piercing resonance) + Blinky buzz; Dungeon torch crackle (`torch_level`) +
  distant `inmate` voices every 25–60 s (groan / sob / wail through `far_off`: muffled,
  2.2 s stone room — deliberately ambiguous) + `chain_rattle` — modelled on a user-supplied reference rattle
  (analysed, not shipped): 8–12 hits in ~0.5 s, each a ~150 Hz thud (≈45% of the
  energy), a 1.1–2 kHz clank, a broadband tick, and 1–3 of 5 links re-struck, every link
  a slightly detuned set of the reference's partial lines (`CHAIN_LINES`, 3.8–15.4 kHz,
  τ 30–110 ms, top lines faint) — struck together they beat like a real chain. Band
  split matched to the reference (<300 / 6–12 kHz ≈ 45 / 38%). **The metal lives at
  6–15 kHz**: every earlier version rang its tones at 0.3–3 kHz and read as wind chime,
  plastic (too short), dreamcatcher, sheet metal (random modes), xylophone (PhISEM
  resonators retuned per hit, 330–2500 Hz). Gate opening
  (`door_open`): rusty hinge squeal (stick-slip saw through hinge modes), bars
  rattling, an iron-bar clang (free-bar ratios, ~0.7 s) — it was a wooden creak + thud;
  Forest wind with gusts + crickets, twig snaps, a rare wolf (a slow swell read as
  surf); Garden bed (gust-gated rustle — a steady hiss read as a fountain), blackbird,
  robin, clock, pocket watch, the fountain's trickle by walking distance. Crow flaps are
  soft air pushes (fast attacks read as gunshots).
- Voices (`babble`): glottal pulses through 3 formant resonators; boss / lady / bot
  (stepped pitch + ring mod + 6-bit crush) lines, never the same twice running.
  `closet_sting` wah guitar, `boss_defeated` chiptune, `elevator` ding-dong.

## Gotchas

- `gen` is a reserved keyword in edition 2024.
- Sort with `crate::util::sort_by_key`, not `slice::sort_by`: every std sort call site
  monomorphizes its own ~3KB copy of the sort into the shared wasm (22 sites were 58KB
  raw / 17KB gzip of the bundle). Negate the key for far-to-near order.
- RNG tests take `crate::rng_guard()` (`macroquad::rand` is process-global).
- `std::time` is banned (clippy.toml) — it panics on WASM; time with
  `miniquad::date::now()` and remove the timing after.
- Measure render cost natively `--release` in the relevant mode; verify visuals in the
  WASM build with headless Chrome (`tmp/doorwork/shots.cjs`, own port) and look at the
  screenshots — most bugs here were only visible on screen.

## Running

```bash
mise run run labyrinth                                        # native
mise run run-bundle labyrinth                                 # via the merged hcg binary
cargo run --release -p labyrinth -- --no-ui --once --variant dungeon
HCG_SEED=42 cargo run --release -p labyrinth -- --no-ui --once --variant fog --debug
HCG_LABYRINTH_THEME=backrooms HCG_SEED=3 HCG_SCREENSHOT=x.png ./target/release/labyrinth --variant mirror
mise run clip labyrinth --variant fog                         # dist/labyrinth/clip.mp4 (og:video), silent
```
