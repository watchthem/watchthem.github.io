# Water Sort

Self-playing water-sort puzzle. A beam-search AI pours colored liquid between bottles
until every color occupies one full bottle by itself. Difficulty (color count, bottle
count, locked bottles, hidden bottoms) scales with an unbounded level counter — see
"Level progression" below for why this is effectively never-ending rather than a fixed
level table.

## Source layout

| File | Contents |
|------|----------|
| `src/game.rs` | `Bottle`, `Game`, `Move`, `LevelParams` (difficulty scaling), generation + solvability probe |
| `src/solver.rs` | `Solver::choose_move` — `beam_solver`-backed pour selection |
| `src/lib.rs` | Rendering (bottle segments, fog, lock badge), pour animation, CLI |
| `src/sound.rs` | Procedural SFX (`lib/audio`), synthesized once at startup |
| `src/main.rs` | Thin standalone binary — `water_sort::start()` and nothing else |

## Bottle model (`game.rs`)

`Bottle.liquid: Vec<Color>` is bottom-to-top, length `0..=CAPACITY` (4, fixed across
every level — only bottle *count* and color *count* scale with difficulty, not bottle
height). A pour moves `min(top_run, room)` units, where `top_run` is how many units
share the source's top color. `Bottle::is_solved` (full **and** monochrome) is a sealed
state: `legal_moves` never offers a solved bottle as a pour source, so once a color is
sorted it stays sorted for the rest of the episode — this is also what makes lock
unlocking (below) monotonic without needing a separate "don't undo a solve" rule.

## Level progression (`LevelParams::for_level`)

Colors climb `3 → 12` (`MAX_COLORS`, the render palette's limit) roughly one every 3
levels, then hold at 12 forever. Empty (buffer) bottles climb `2 → 4`. Locked bottles
(`0 → 3`) and hidden-bottom fog depth (`0 → 1 → 2`) only start once colors have climbed
past their own gating thresholds (`LOCK_MIN_COLORS`, hardcoded level thresholds for fog)
— **difficulty keeps climbing past the color cap** via bottle count / locks / fog depth,
which is what makes "a few dozen levels, ideally never-ending" actually never-ending
rather than flat past level ~27: measured a real seed reaching level 190+ in a few
minutes of `--no-ui` soak with win rate still ~99%+ (see "Solver" below) — bottle count
alone (up to 16 at the color cap) keeps the search space growing indefinitely even
though the palette doesn't.

## Locked bottles: unlock condition can't be circular

A locked bottle (`Bottle.lock_target: Some(color)`) can't be poured into or out of until
some *other* bottle fully sorts `color` (`Bottle::unlocked` is sticky — flips false→true
once and never back, checked in `Game::refresh_locks` after every pour). The generation
code (`Game::apply_locks`) picks `lock_target` only from colors present in **no** locked
bottle's own starting contents (`trapped`, unioned across every locked bottle before any
target is assigned, not decided one lock at a time) — otherwise two locked bottles could
form an unlock cycle (A needs B's color solved, B needs A's), or a lock could need its
own trapped liquid moved to unlock itself. This is only reachable once
`colors > CAPACITY` (`LOCK_MIN_COLORS`, 6): a bottle holds at most `CAPACITY` (4)
distinct colors, so a free (untrapped) target color is guaranteed to exist once the
palette is bigger than a single bottle can span.

## Hidden-bottom fog is cosmetic only, and reveal is sticky

`Bottle.fog` (bottom N slots obscured until the bottle drains down to `fog` units or
fewer — see its doc comment) is never read by any *legality* rule in `game.rs`, only
by `lib.rs`'s rendering — same "AI knows the true state, the *drawing* hides it"
split every self-playing game in this workspace uses for anything visually withheld
(there's no real hidden-information search here, unlike a game where an opponent's
hand is genuinely unknown).

It is, however, mutated by `game.rs`: `apply_pour` zeroes `fog` for good the instant
draining crosses the threshold. **Fixed bug**: this used to be a live comparison
(`b.fog > 0 && b.liquid.len() > b.fog`, checked fresh every render) rather than a
one-way reveal — so a bottle that had drained low enough to reveal its fogged bottom,
then was later poured back into (e.g. becoming a merge destination) and refilled past
the old threshold, would show the fog panel again over units the viewer had already
seen. `lib.rs`'s render check didn't need to change at all once `fog` itself becomes
permanently `0` on reveal — the same live comparison is now correct by construction.

## Generation: shuffle-and-deal, biased toward solvable (not guaranteed)

`Game::deal_unlocked` shuffles a flat multiset (`colors` colors x `CAPACITY` units each)
and deals `CAPACITY`-sized chunks into `colors` filled bottles, appends `empty` empties.
No reverse-pour-from-solved-state construction (unlike klondike's deal or sudoku's
carve) — locks/fog are layered on afterward by `apply_locks`, decided independently of
the deal itself. `Game::new` retries up to `GENERATION_ATTEMPTS` (10) times, keeping the
first deal `bfs_solvable` (a budget-capped BFS, `SOLVABILITY_BUDGET` = 12,000 states)
confirms winnable *ignoring locks* (checking with locks live would need reasoning about
unlock ordering inside the BFS itself, for a marginal accuracy gain not worth the extra
cost — locks are already constructed so their unlock is reachable without the locked
bottle's own participation, per above), falling back to the last attempt if none confirm
within budget. **This is a bias, not a proof** — same acceptance every solver-family game
in this workspace already has for an occasionally-unsolvable deal (`Phase::Stuck`
exists for exactly this, same as klondike/spider). Measured: ~1 stuck episode per ~200
across a 3-seed, ~600-level soak (`--no-ui`, no `--once`) — see "Solver" below.

Lock-candidate selection (`apply_locks`) deliberately does **not** draw from the ambient
`macroquad::rand` stream `deal_unlocked` uses — see `shuffle_indices`'s doc comment:
`Game::new`'s retry count varies deal-to-deal (however many attempts solvability took),
so if lock selection shared that stream, retry count would perturb every later level's
colors/fog too. It's a tiny local PRNG seeded straight from `generation` instead,
independent of how many retries happened.

## Solver (`solver.rs`)

`beam_solver::SearchState` on `Game`, same shape as klondike/spider/match-3/
bubble-shooter. `BEAM_WIDTH=10, BEAM_DEPTH=6, NODE_BUDGET=4000` — reasoned from board
scale (small `legal_moves()` per state, up to ~16 bottles at the highest tiers), not
measured/tuned yet, same caveat as bubble-shooter's initial pass (root CLAUDE.md's
"win rate must be checked empirically" note applies if these ever get revisited).

`is_pointless` filters exactly one move shape: pouring an already-fully-monochrome
source into an empty bottle. Provably never helps — a monochrome partial stack accepts
the same future pours (more of its own color) wherever it currently sits, so moving it
to an empty bottle only relabels which bottle holds it. Without this the beam spent
width on pure bottle-shuffling that never changed the board's real shape.

Scoring (`score`, used for both root and every later ply — same function, unlike
klondike's separate `score_root`/`score_core`, since nothing here needs a pricier
once-per-real-move lookahead) is a **potential difference**, not a bag of per-move
rewards: `+100_000` for an outright win, otherwise `Φ(after) - Φ(before)` where `Φ` sums
`bottle_potential` over the board plus `UNLOCK` (300) per unlocked locked bottle. Because
`beam_solver` accumulates step scores along a line, a potential difference telescopes to
`Φ(final) - Φ(root)` — two lines reaching the same board score identically however many
moves they took, and any round trip sums to exactly zero. `score` computes the delta over
just the two bottles the pour touched (everything else cancels), so it stays O(1) despite
being defined as a whole-board evaluation.

`bottle_potential`: solved `500`, empty `150`, otherwise `15` per unit of top run, `+55`
if monochrome, `-20` per extra distinct color, `-10` per extra run (fragmentation —
`abab` untangles worse than `aabb`).

**Fixed bug — empty-bottle score farming.** Reported as "the solver systematically pours
the same color into two *different* empty bottles instead of consolidating." The previous
scoring paid `+150` whenever a pour emptied its source but only `-30` when a pour consumed
an empty bottle, and those two numbers were independent, so the beam could arbitrage the
gap: split a color into a fresh empty (`-30`), then merge it straight back (`+150` for
emptying the bottle it had just filled, `+74` for the pure-color merge) = `+194` over two
plies, versus `+74` for pouring that same unit directly onto the same-color bottle in one
ply — *both lines ending on an identical board*. `BEAM_DEPTH=6` sees the two-move detour
and always takes it. Reproduced exactly at `HCG_SEED=7`, level 7 generation 6, moves 3-4
(`3->5` scored 74 and lost to `3->6` at -30, and the pair `6->7`/`7->6` at 224 apiece
shows the payoff being collected). This was *not* a search width/depth/budget limit and
not an order-of-play artifact — the better move was legal, enumerated, and scored higher
at the root; the cumulative-sum lookahead overrode it. The earlier `destination_bonus`
three-way ranking (mixed < empty < pure) was addressing the right symptom with the wrong
mechanism — no value of the `-30` fixes it without over-penalizing the empty pours that
genuinely are necessary. Folding both bonuses into one *state* term (an empty bottle is
simply worth 150 in `Φ`) removes the arbitrage by construction; the pure-vs-mixed
destination preference then falls out of `Φ` on its own, so `destination_bonus` is gone.

**Measured**, paired A/B (both binaries run head-to-head on the same seeds simultaneously,
so machine load hits them equally — 20 seeds x 90s `--no-ui` soak, ~2250 episodes each):

| | levels won | moves/episode | `Stuck` |
|---|---|---|---|
| before | 2235 | 44.4 | 13 (0.58%) |
| after | 2271 (+1.6%) | 36.4 (-18%) | 22 (0.96%) |

Per-seed level progression was equal or better in 19 of 20 seeds. The headline result is
moves/episode: the same puzzles now solve in ~18% fewer pours, which is the wasted
split-and-remerge detours disappearing. The `Stuck` difference is not significant
(z ≈ 1.5) and is confounded by `after` reaching deeper, harder levels within the same time
budget; a `Stuck` episode costs no progress anyway (it re-deals the same level number).

Frequency of the reported pattern itself, counted off `HCG_DIAG` soak logs (4 seeds x 25s,
~2600 moves each) as "poured into an empty bottle while a non-full, unlocked, same-top-color
bottle was available":

| | pours into an empty | avoidable | ...where the alternative was a *pure* same-color bottle |
|---|---|---|---|
| before | 32% of moves | 17.6-20.0% | 13.1-14.7% |
| after | 18% of moves | 1.5-1.9% | 0.5-0.8% |

A ~10x drop overall and ~20x on the exact "a pure bottle of that color was sitting right
there" case. The 1.5-1.9% residual is almost entirely the *mixed*-alternative case, where
declining to contaminate a mixed bottle and using an empty is often genuinely correct.

**Rejected**: also collapsing interchangeable empty destinations in `is_pointless` (offer
only the lowest-indexed empty bottle, since no rule in `game.rs` tells two empty unlocked
bottles apart). It looks like free branching-factor savings — and the fix above
deliberately keeps *more* empties free, so that branching grew — but measured it cut
levels reached in a fixed 60s budget from 669 to 654 across 8 seeds: the candidate
enumeration it trims isn't where the per-move cost is, while the distinct board hashes it
removes cost the beam real line diversity.

### Diagnosing solver behavior from a soak log

`--debug` logs the chosen pour but not the position, which isn't enough to separate a bad
*choice* from a bad *position* after the fact. `HCG_DIAG=1` (alongside `--debug`) makes
`log_move` also dump the whole board before every pour — one line, `idx:letters` per
bottle bottom-to-top, `*` marking a locked one — so a long soak can be audited offline.
That's what caught the bug above; the "avoidable pour" numbers in the table are counted
straight off those logs.

## Rendering (`lib.rs`)

Bottles laid out in a grid (`Layout::compute`, up to 8 columns, wrapping to more rows as
bottle count grows) inside the default 900x720 canvas — no `xtask::native_size`
override needed, unlike bubble-shooter/tetris's portrait boards.

### Palette: validated, not eyeballed

`PALETTE`'s 12 colors were checked with this workspace's `dataviz` skill's
`validate_palette.js` (categorical, `--pairs all` — any two of the 12 colors can end
up stacked in the same bottle, so *every* pair matters, not just adjacent slots in
the array) — see `PALETTE`'s own doc comment for the two colors that failed
(`lime`/`yellow` too similar to a normal-vision eye, `indigo`/`purple` nearly
identical under colorblindness simulation) and what replaced them. If this palette
is ever revisited (a color added past 12, say, if `MAX_COLORS` ever grows), re-run
the validator rather than eyeballing a new hue in — it caught a real, reported bug
("yellows look too much alike") plus a second one nobody had noticed yet.

### Bottle shape: neck + shoulder + rounded-bottom body, all hand-rotated

A bottle isn't one rectangle — `draw_bottle` builds it from a neck whose top two
corners are rounded (`rounded_top_rect`, mirroring the bottom version below — a mouth
reads as an open rounded rim, not a flat rectangular cap; the outline's two side lines
start at `y + mouth_r` rather than `y` so they don't stroke a sharp corner back on top
of the rounded fill), a trapezoid shoulder flaring out to the body's full width (two
`draw_triangle` calls, drawn before the body so its square top edge hides the seam),
and a body whose bottom two corners are rounded (`rounded_bottom_rect`: a full-width
rect down to `h - r`, a narrower strip for the last `r`, plus two corner circles —
cheaper than any kind of signed-distance/stencil rounding and good enough at this
size). `mouth_r` (the neck's own rounding radius) is deliberately much smaller than
the body's `corner_r` — `(neck_w * 0.12).min(2.5)` vs. `(bw * 0.16).min(10.0)` — a
narrow neck read as a cartoonish blob at anywhere near the body's rounding scale;
a bottle mouth wants a subtle rim, not a pronounced round-over.

Liquid segments (further down in `draw_bottle`) are flush with the container on every
side — full `bw` width, no vertical inset either. **Fixed bug**: an earlier version
inset the liquid `3.0`px horizontally (`x + 3.0`, `bw - 6.0`) but not at all
vertically, which read as an inconsistent margin (a visible gap on the sides, none
top/bottom) rather than the liquid actually filling the glass. Flush fill relies on
the outline stroke (drawn afterward, further down in `draw_bottle`) landing right on
top of the liquid's own edge to read as the glass wall, instead of a manually-drawn
gap trying to approximate one. The bottom-most drawn segment (`idx == 0`) rounds at
the container's own `corner_r` — an exact match, since it really does trace the same
curve as the glass — while the *topmost visible* one (`top_idx`: the liquid's actual
surface, not just whichever slot happens to be full; on a single-segment bottle these
are the same slot, rounded on both ends into a pill shape) rounds at a smaller,
deliberately-not-matching `liquid_r = corner_r * 0.55` for a soft meniscus curve
instead of a flat top edge — a free liquid surface has no glass edge to trace, so
there's no inconsistency in its radius differing from the bottom's. Segments also
butt directly against each other with no inset between color bands — an even earlier
version shaved `1.0` off each segment's height there too, which read as a hairline
seam cut between every color rather than a natural boundary within one continuous
liquid column.

Every one of these shapes is drawn through `rect_rotated`/`line_rotated`, which rotate
their own corner points around a caller-supplied pivot via `rotate_pt`, rather than
through a native rectangle-rotation call — macroquad only rotates a plain rectangle
about its own center, which can't express "tip this whole multi-part glyph around its
base" the way a pour needs (see the next section). `rotate_pt` is the identity at
`angle == 0.0`, so a resting bottle's draw calls aren't a different code path, just the
same calls with `angle: 0.0`.

**Fixed bug**: the fill silhouette rounded (via `rounded_bottom_rect`/
`rounded_top_rect`) but the *outline* stroke didn't — it was still two straight
`line_rotated` segments stopping short of each rounded corner, with nothing drawn in
between, so the border had a visible gap at every rounded corner (and, without a
stroke curve to actually sell it, the rounded mouth read as "not rounded" despite the
fill change). `arc_rotated` closes this: a quarter-circle stroke, sampled in
*local* (pre-tilt) angle space same as any other vertex and rotated through the same
`rotate_pt` pivot, for all four rounded corners (two on the body, two on the neck).
The straight segments and the arcs meet exactly at the fill's own rounding radius on
each side, so the two together read as one continuous rounded border.

### Pour animation: lift, tip, pour, right-and-return

The whole animation is built around one invariant: **the pour lip is parked directly
above the destination's mouth for the entire time any liquid is drawn, so the stream is
a straight vertical fall.** Everything below follows from that.

A pour is five beats over `ANIM_DURATION` (0.9s), timed by `LIFT_FRAC`/`POUR_FRAC`/
`POUR_RISE_FRAC`/`POUR_HOLD_FRAC` fractions of `anim_t` in `run_ui`'s `pour` block:

1. **Lift** (`0..LIFT_FRAC`): the source's *position* — not just its rotation —
   interpolates (`lerp_pt`, eased) from its grid slot to the `hover` point (see
   "Where the bottle hovers" below). Upright throughout; nothing pours yet.
2. **Rise** (start of the `POUR_FRAC` window): position holds at `hover`; tilt eases
   `0 -> TILT_MAX * sign`, swinging the mouth out over the destination. Still nothing
   pouring — the bottle is aiming.
3. **Hold**: tilt *pinned* at `TILT_MAX * sign`. Liquid transfers during this beat only
   (`pour_progress` eases 0→1, mapping `from_len`/`to_len` from before → after amounts);
   stream drawn as a continuous ribbon.
4. **Drip** (rest of `POUR_FRAC`): tilt *still pinned*; nothing transfers, the stream
   switches to trailing droplets that taper away.
5. **Return** (after `POUR_FRAC`): position eases `hover -> grid slot` **and** the tilt
   eases back to 0 over the same beat — the bottle straightens out as it travels home.

Beat 5 doing the righting is deliberate: the tilt never changes while a stream is on
screen, so the stream's origin never has to be reconciled with a rotating bottle. Two
earlier versions got this wrong from opposite directions — one eased the tilt back down
*during* the transfer (the stream's geometry swept back toward, and through, the source:
"it collapses into the bottle, not flows out"), the other froze the stream's origin at
peak tilt while the bottle visibly rotated out from under it ("wrong position of upper
bottle"). Neither choice is available once nothing rotates until the stream is gone.

#### Where the bottle hovers, and why it isn't over the destination

`hover` puts the source `lip_reach` to the *side* of the destination — on its own side
of it — not above it:

```
lip_reach = bh * sin(TILT_MAX) + neck_w / 2 * cos(TILT_MAX)
hover.x   = tx - sign * lip_reach
```

`lip_reach` is exactly how far the pour lip (the rim corner on the leaning side, which
the tilt puts *lower* than the other one) swings sideways when the bottle tips to
`TILT_MAX` about its own bottom-center, so at peak tilt the lip lands on the
destination's centerline. `sign` leans toward the destination, which puts `hover` back
toward the source's own side (and keeps it on screen; it flips if the preferred side
would run off the near edge). `hover.y` is likewise derived from where the *lip* ends
up (`lip_drop`) rather than from the bottle's top edge, and is clamped by the tilted
silhouette's own topmost point (`top_gap`) rather than `src_pos.1`, which by then is
above anything actually drawn.

**This was the root cause of the long-running "trajectory looks wrong" complaint.**
The original code hovered the source's *body* in the destination's column, so tipping it
swung the lip a full `lip_reach` (~70-150px) past the destination, pointing away from it
— and the stream then had to hook sideways and back to reach the target, with a control
point that (to keep the curve clear of the source's own body) bowed it even further out
first. On screen that read as a thin thread flying out sideways and swooping back in.
Both earlier fixes only adjusted *which mouth position the stream hung off*; neither
touched the fact that the two endpoints were in the wrong places relative to each other.

`TILT_MAX` (70°) is also load-bearing, not decorative. Below ~25° the body's shoulder
corner still reaches further sideways than the mouth does, so the falling stream grazes
the glass it is supposedly leaving (liquid dribbling down the *outside* of the bottle);
and a shallow tip leaves the mouth as the highest point of the bottle, so the stream
reads as flowing uphill. 70° puts the mouth roughly level with the body's midpoint,
where the liquid — drawn bottom-stacked in the bottle's own rotating frame, there being
no level-surface physics here — can plausibly reach it.

#### Stream rendering

`draw_bottle`'s `angle`/`pivot` parameters (pivoting around the body's own
bottom-center) carry the tilt; its `shown_len: f32` (distinct from the bottle's real
`liquid.len()`) carries the fill-level animation the same way independent of position —
the source's `shown_len` briefly exceeds its already-truncated `liquid.len()` mid-pour,
with `extra_color` (the pour's color, captured before `apply`) filling in for the
segments `liquid` no longer has, safe because the transferred run is always a single
uniform color.

The stream itself (`StreamShape`) hangs off the lip, recomputed each frame through the
same `rotate_pt`/`src_pivot` the bottle is drawn with, so it can't drift from the drawn
glass. Its far end is the destination's *actual current liquid surface* (from `to_len`,
not the bottle's top). The quadratic Bezier (`bezier`) between them has its control
point at `(dst_surface.0, midpoint y)` — a gravity shape (lateral first, vertical into
the surface) that degenerates to the straight vertical drop it should be in the normal
case, and only bends when the hover height had to be clamped.

- `Ribbon` (Hold): `RIBBON_SEGMENTS` straight `draw_line` segments along the curve,
  narrowing slightly on the way down. Drawn together with a rotated rect filling the
  neck's interior with the pour color, shrinking back toward the rim as `pour_progress`
  advances — without it the fall appears to start out of empty glass, since liquid
  segments only ever fill the *body* and the neck stays dark exactly where the stream
  begins.
- `Drops` (Drip): `STREAM_DROPLETS` circles flowing along the same curve (phased by
  wall-clock time), radius shrinking as `taper` (0..1 across Drip) approaches 1 — a
  trailing-off drip rather than the stream vanishing outright.

### Removing solved bottles from the board

`draw_game` skips any `Bottle::is_solved()` bottle outright — its grid slot is simply
left empty, no reflow of the remaining bottles' positions (deliberately: reflowing on
every solve would make already-tracked bottles jump to new slots, which is a worse
spectator experience than a fixed grid with some empty gaps). The live pour overlay
(`run_ui`, drawn on top of the cached board every frame) isn't subject to this skip, so
a pour that completes a bottle still visibly finishes filling it before it vanishes —
it only disappears once the animation settles and `display_game` (what `draw_game`
actually reads) catches up to the solved state.

### Locked bottles

Render behind a dark scrim with a small padlock badge above them showing the target
color as a dot — cleared automatically (badge and scrim both) the frame after
`Bottle.unlocked` flips true, no separate animation needed since the pour that triggers
an unlock already has its own.

## Sound (`sound.rs`)

Triggers hang off the animation, not `Game::apply` (which runs a whole pour earlier):
the glug starts when `anim_t` crosses `STREAM_START` (stream begins falling); set-down
clink, sorted chime, unlock latch and won/stuck stingers all fire when the pour
animation completes, diffing `game` against the still-pre-pour `display_game`. A pour
that wins plays the won arpeggio *instead of* its sorted chime. Glug (a stochastic population
of smooth-onset sine bubbles — see `glug` / `mod pour`, incl. which variants lost) is pre-rendered per
`(to_before, to_after)` pair — pitch follows the air column above the liquid, so it
rises as the destination fills. Chimes are one pentatonic glass note per color. Muted
under `?embed`/`?stream` (wall).

## Running

```bash
mise run run water-sort                              # native
mise run build-wasm water-sort                        # WASM → dist/water-sort/
target/release/water-sort --no-ui --once --debug
HCG_SEED=42 target/release/water-sort --no-ui --debug  # soak, no --once: climbs levels forever
HCG_SEED=42 HCG_DIAG=1 target/release/water-sort --no-ui --debug  # + full board per pour
```
