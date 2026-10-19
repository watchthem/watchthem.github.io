# hcg — Hypercasual Games

Rust workspace of small self-playing games targeting native and WASM via macroquad. Each game
lives in `games/<name>/` as a separate crate.

This file covers conventions shared across the whole workspace. Crate-specific design lives in
that crate's own `CLAUDE.md` — check there before re-deriving anything:

- `xtask/CLAUDE.md` — site generation: page structure, SEO/JSON-LD, theme, PWA, JS bridges,
  canvas-fit details, page-level analytics
- `bundle/CLAUDE.md` — merged wasm binary (why, size numbers) and the native desktop shell
- `lib/render_cache/`, `lib/audio/`, `lib/control/`, `lib/beam_solver/`, `lib/cards/` — API +
  gotchas
- `games/<name>/CLAUDE.md` — every game except arrow-blocks has one; `games/snake/` (terse)
  and `games/match-3/` (full) show the range

## Workspace layout

```
Cargo.toml          workspace root (resolver = "2", opt-level = "z" + lto for release)
mise.toml           task runner — primary interface for builds
games/<name>/       one crate per game
lib/                shared crates (cards, minesweeper, screenshot, control, beam_solver,
                    render_cache, game_common, audio)
bundle/             every game in one binary (`hcg`), game chosen at runtime — NOT under games/
xtask/              native-only site generator
static/             assets copied verbatim into dist/
dist/               build output (git-ignored), served via python HTTP
```

## Task runner (mise)

All tasks take the game name as `$1` unless noted.

| Command | What it does |
|---------|-------------|
| `mise run run snake` | Native debug build + launch (the game's own binary) |
| `mise run run-bundle snake` | Same game through the merged `hcg` binary — the dispatch path the browser takes |
| `mise run build-bundle` | Release WASM of all games → `dist/hcg.wasm` |
| `mise run build-wasm snake` | Stages one game's *page* → `dist/snake/` (index.html, manifest, preview). Builds no wasm |
| `mise run deploy` | Bundle once, then every game's page (incremental: skips unchanged screenshots / HTML) |
| `mise run clean` | `rm -rf dist` — force a full rebuild |
| `mise run serve` | `python3 -m http.server 8080 --directory dist` |
| `mise run check` | `cargo fmt --all -- --check` + `cargo clippy --workspace --all-targets -- -D warnings` |
| `mise run indexnow [urls]` | Ping IndexNow (Bing/Yandex/…) after a deploy that changed pages; no args = whole sitemap |

WASM target: `wasm32-unknown-unknown`. **Not** wasm-bindgen — macroquad uses miniquad's own JS
bundle. Pushing to `master` deploys via `.github/workflows/deploy.yml` (~10 min; the separate
`Check` workflow finishing first is not the deploy).

`.githooks/pre-commit` runs `mise run check`; activate once per clone/worktree with
`git config core.hooksPath .githooks`.

## Adding a new game

Registration checklist — every site a game must appear in (grep an existing game such as
`water-sort` outside `games/` to confirm nothing new was added):

1. `games/<name>/`: lib + three-line bin (`fn main() { <crate>::start(); }`). Deps:
   `macroquad = "0.4"`, `control`, `screenshot`, `render_cache`, `game_common` (+ `audio`,
   `beam_solver`, `cards` as needed). Copy `games/snake/src/lib.rs`'s skeleton.
2. Root `Cargo.toml` `members`.
3. `bundle/`: path dep in `Cargo.toml`; `GAME_NAMES` in `src/main.rs` (plain-alphabetical by
   directory — positional contract, `bundle_list_matches_games_dir` test enforces it); wasm
   dispatch arm in `main.rs`; `run_game` arm in `src/shell.rs`
   (`size_to(<g>::conf()); <g>::play_until_exit().await`); preview list in `build.rs`;
   `include_bytes!` entry in `src/menu_art.rs`. Without the dispatch arm the page runs game 0;
   without the shell arm the native menu panics.
4. `xtask/src/lib.rs`: `description(name)` and `game_flavor(name)` arms (copy rules below);
   `native_size` / `max_fit_scale` only if not 900×720; `hotkey_popup`'s
   `has_variant_switch` list only if it has a `V` variant cycle.
5. Verify: `mise run run <name>`, `mise run run-bundle <name>`, `hcg` menu tile + Esc back,
   `mise run build-bundle && mise run build-wasm <name>` and load the page (Playwright).
   Inserting into `GAME_NAMES` renumbers later games — load one of those pages too.
6. Ship: `games/<name>/CLAUDE.md` (layout table, constants, solver design + measured numbers,
   rendering, gotchas, Running); move the idea in `.notes/aiideas.md` to Shipped as a one-line
   pointer; after deploy, `mise run indexnow` the new page + homepage + wall.

Planning a new game: write `.notes/<name>_plan.md` as phases that each end runnable and
committable (phase 0 = scaffold + full registration). Reference this section and the
conventions below instead of restating them; the plan should hold only game-specific design.
`.notes/aiideas.md` → "More games" has the candidate list and the two filters every candidate
must pass (the bot needs a real failure mode; episodes must end on their own or at a cap).

## Game crate shape (every game)

- Each lib exposes `conf()`, `start()` (standalone binary: parses CLI, `--no-ui` branches
  *before* `Window::from_config`; never `#[macroquad::main]`, which opens the window first),
  `play()` (what the browser bundle calls — runs with browser `CliArgs`, never calls
  `Window::from_config`), and native-only `play_until_exit()` (what the desktop shell calls).
- The loop (`amain`/`run_ui`) returns `control::ExitReason`: call `control.handle_keys()`, then
  `if let Some(r) = control.exit_requested() { break r }` once per frame. `start()` exits the
  process after it returns. One game per page load on the web; the native shell runs several
  in sequence (see `bundle/CLAUDE.md`).
- `Conf`: `high_dpi: true`; 900×720 unless there's a reason (then register in `native_size`).
- Seed with `rand::srand(control.seed())` at startup (quad-rand's default seed is fixed);
  `control.episode_complete(game_name, score)` at each reset.
- Native CLI flags, parsed by `game_common::parse_base_args(&args)` plus the game's own extras
  from the leftover tokens:

| Flag | Behavior |
|------|----------|
| `--debug` | Print every move/tick/action to stderr |
| `--once` | Play one episode to its natural end, print a `result=...` line, exit |
| `--variant <...>` | Only games with selectable modes — pin the variant |
| `--no-ui` | No window/GL/audio at all |

  `--no-ui --once` with `HCG_SEED=<n>` is the standard deterministic benchmark — always
  `--release` (beam searches are ~100x slower in debug). A game driven by real `dt`
  (animation, timers) drives a fixed virtual `dt` in `run_headless` instead of reading
  `get_frame_time()`/`date::now()`, reusing the same step functions. A continuous-physics
  game (asteroids) goes further: `step()` is exactly one fixed sim tick, and the window loop
  accumulates `control.scale(get_frame_time())` into whole steps (capped per frame), so
  windowed and headless run the identical simulation.

## Self-playing solver games

Split into `game.rs` (rules/state — `Game`, `Move`, `legal_moves()`, `apply()`,
`state_hash()`), `solver.rs` (move selection) and `lib.rs` (rendering + loop, ticking the
solver at a watchable interval). Common solver shapes:

- **Many simultaneously legal moves, scored against each other** (klondike, spider, match-3, tetris,
  bubble-shooter, water-sort): use `lib/beam_solver` rather than a hand-rolled search.
- **Fixed technique escalation over a puzzle with one known answer** (sudoku): write the
  escalation directly in `solver.rs`.
- **Per-tick heuristic or small search in the game's own code** (snake's BFS, game2048's
  expectimax, minesweeper's constraint solving, labyrinth's explorer, asteroids' threat
  model + flyable escape rollouts).

Decide by measurement, not inspection: match-3 was built 1-ply on a branching-factor argument
and beam search then beat it by 6–14pp at ~1.5ms/move. Tune against measured outcomes over a
range of `HCG_SEED`s — and look at the *distribution* (when/why it loses), not just the rate.
The solver must never draw from quad-rand's global RNG (it desyncs same-seed A/B runs).

## Rendering perf

Games tick a few times a second but render at ~60fps. Split drawing into a static part (changes
once per tick) and an animated part. Cache the static part with `lib/render_cache::RenderCache`
(`mark_dirty()` when its inputs change, `.draw(|| ...)` every frame); draw the animated part
live on top. Rebuild a cache whose rect depends on screen size when that changes. Call
`render_cache::prewarm_glyphs` with every text/size before constructing any cache — see
`lib/render_cache/CLAUDE.md` for this and the other gotchas.

## In-game controls

Every game uses `lib/control` (`Control::new()`, `handle_keys()` once per frame, frame time
through `scale(dt)`) for speed/pause/mute/fullscreen/seed/variant hotkeys; draw
`control.label()` in the game's header. See `lib/control/CLAUDE.md` (hotkey table, adding a
hotkey — both popups need updating). Capture (`HCG_SCREENSHOT`, `HCG_CLIP_FIFO`) is always
silent automatically. Extend an existing control (e.g. the `V` cycle) for a new mode rather
than adding a hotkey.

## Site and page rules

Pages are generated by `xtask` (maud), never hand-built strings; `mise run build-wasm` is the
only way to get a browser-testable page. Rules that touch every game (details in
`xtask/CLAUDE.md`):

- `<script>`/`<style>` content goes through `maud::PreEscaped`; script JS also through
  `xtask::minify_js`. Share state between script blocks via `window.*`, not bare top-level names.
- A game page is one screen of game (`.stage`, `overflow: hidden`) then indexable content
  (h1, description, home link, related games). Don't regress its `<a>` links or in-flow text.
  Anything new below the fold must be hidden under `hcg-bare` (`?embed=1`/`?stream=1`).
- **Copy:** `game_flavor` names the human's bad habit in that game and has the AI be
  insufferable about it, disjoint from `description()`. **No implementation detail in any site
  copy** (no solver/algorithm terms). Present 2–3 options before finalizing copy.
- **Every JS bridge must be registered on every page** — the merged wasm imports all of them;
  a missing one is a LinkError on that page. New page-level analytics events must be
  suppressed under `?embed=1`/`?stream=1`.
- New page colours are CSS variables with a light-theme value; `.stage` stays black.

## Canvas sizing is load-bearing

Games draw at absolute pixel coordinates for their native `window_width`/`window_height`;
none scale by `screen_width()`/`screen_height()`. The page pins the canvas to native size and
fits it with CSS `transform: scale(...)`. Don't reintroduce `width: 100vw; height: 100vh` on
the canvas unless every game's drawing becomes scale-aware (labyrinth is the one exception,
via `xtask::fills_fullscreen`).

## WASM caveats

- `std::time::SystemTime::now()` / `Instant::now()` panic on WASM — use
  `macroquad::miniquad::date::now()`. `clippy.toml` bans both; native-only timing opts out with
  `#[allow(clippy::disallowed_methods)]`.
- No filesystem on WASM — no `std::fs` in game code.
- Calling JS from Rust (no wasm-bindgen): a page script calls `miniquad_add_plugin` to add a
  function to `importObject.env` before `load(...)`; Rust declares a matching
  `unsafe extern "C"` fn. Strings cross as `(ptr, len)` decoded with `UTF8ToString`
  (`control::ga_event` / `xtask::analytics_bridge`); byte buffers are copied out with
  `wasm_memory.buffer.slice(ptr, ptr + len)` before use (`xtask::screenshot_bridge`).
- A bare `cargo build --target wasm32-unknown-unknown` of a game fails to link; use
  `mise run build-bundle`.

## After a significant change

Once a piece of work lands (a new game/variant, an architectural change, a fixed bug whose
cause wasn't obvious, a measured before/after result) — before ending the turn:

1. Save what's durably worth remembering. A crate's architecture/design/gotchas go in that
   crate's `CLAUDE.md` (checked in, loaded when working there, can't go stale like a memory
   file); memory is for process lessons, user preferences and cross-cutting facts — not
   anything re-derivable from code or these docs.
2. If the change is a new convention, shared crate, or standard pattern, update this file (or
   ask). Avoid hard counts ("all N games") — they go stale with every new game.
