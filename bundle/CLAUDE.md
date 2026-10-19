# bundle — merged wasm + native shell

`bin hcg`: every game in one binary. On the web it runs the one game the page names; natively
it is a desktop app with a game menu. Root `CLAUDE.md` has the rules a new game must follow
("Adding a new game"); this file is the why and the detail.

## Merged wasm bundle

Every game page loads the **same** wasm: `dist/hcg.wasm`, built from `bundle/` (bin `hcg`),
which contains every game and picks which one to run at startup. There is no per-game
`dist/<name>/<name>.wasm` any more.

Why: ~99% of a game's shipped wasm is framework (macroquad/miniquad/fontdue/ttf_parser/std,
plus the default font and GL shaders in rodata) and only ~22–25KB is the game's own logic, so
separate binaries paid for the same framework once each. Measured at 11 games: 613,234 bytes for all
merged, against 4,196,365 bytes for the 11 separate binaries they replace (each 366–410KB).
Marginal cost is linear in games (366KB for 1, 391KB for 2, 613KB for 11), which is why
there's **one** bundle rather than several smaller ones: every extra bundle re-pays ~366KB of
framework, and splitting could save at most the ~250KB of game logic it excluded.

Shape (keep it for new games):

- Each game crate is **lib + thin bin**: everything that used to be in `src/main.rs` is now
  `src/lib.rs` (same crate root, so `mod`/`crate::` paths inside `game.rs`/`solver.rs` are
  untouched), and `src/main.rs` is three lines — `fn main() { <crate>::start(); }`.
  game2048 is the one exception: its `lib.rs` was already the pure-logic half, so its loop
  lives in `src/play.rs` and is re-exported at the crate root to keep the API uniform.
- Each game lib exposes `conf()`, `start()`, and `play()`. `start()` is the old `main()`
  verbatim (native `--no-ui` branching included) and belongs to the standalone binary;
  `play()` is what the bundle calls — an `async fn` that runs the game with exactly the
  `CliArgs` the *browser* build produces and does **not** call `Window::from_config` itself.
  `CliArgs` fields can stay private; nothing outside the crate needs them.
- **Per-game standalone binaries stay** (`mise run run <name>`, screenshots, clips,
  `--debug`/`--once`/`--no-ui` benchmarking). The bundle is an additional target, not a
  replacement, and native CLI flags live on the per-game binary only — `mise run run-bundle
  <name>` just picks a game (`hcg --game <name>`).
- Dispatch is a hand-written `match` in `bundle/src/main.rs` with one
  `Window::from_config(<game>::conf(), <game>::play())` arm per game (one call per arm, since
  every `play()` future is its own type). `GAME_NAMES` there is a **positional** contract:
  index → game, plain-alphabetical by `games/<dir>`, baked into each page by
  `xtask::game_id_bridge` / `xtask::bundle_game_index`. Not `xtask::all_games()`'s order —
  that sorts by display `title()`, so a retitled game would silently renumber every page.
  `bundle`'s own `bundle_list_matches_games_dir` test fails if the list drifts from `games/`.
- **Every JS bridge must be registered on every game page.** The merged module carries every
  game's imports whatever game a page runs, and a wasm import the page never registered fails
  instantiation with a LinkError — that's why `variant_query_bridge` (only minesweeper reads
  it) is no longer gated behind `name == "minesweeper"`.
- One game per process/page load **on the web build**. `play()` never returns, seeds the
  global RNG itself, and prewarms only its own glyphs. The native standalone shell (below)
  does run multiple games in the same process one after another via `play_until_exit()`
  instead — each game's own existing prewarm-before-first-`RenderCache` call already
  covers this safely on every fresh entry, so no extra cross-game prewarm step was needed;
  see "Native standalone shell" for why.

`dist/hcg.wasm` is not content-hashed (same as the per-game wasm it replaces), so cross-page
reuse rests on the browser's HTTP cache plus `static/sw.js`'s network-first strategy. It now
changes whenever *any* game changes; content-hashing the filename is the open follow-up if
that reuse should be a guarantee rather than a heuristic.

## Native standalone shell

`bundle/src/shell.rs` (native-only, `#[cfg(not(target_arch = "wasm32"))]`) turns the same
`hcg` binary into a desktop app: one window, a game-selection menu, and the ability to
return to that menu from any game — the shape a Steam-style build needs instead of `hcg
--game snake` jumping into one game and never coming back. The web build (`main.rs`'s
`#[cfg(target_arch = "wasm32")]` half, `dist/hcg.wasm`) is untouched by any of this — it
still calls each game's `play()` exactly as before.

- **`hcg` with no arguments now lands on the menu**; `hcg --game <name>` still boots
  straight into that game (the path `mise run run-bundle <name>` drives) — Esc or the
  window's close button returns to the menu instead of exiting the process. Unknown
  `--game`/argument values still print usage and `exit(2)`, same as before the shell.
- **`control::ExitReason` / `Control::exit_requested()`** (`lib/control`) is what makes a
  game loop return instead of looping forever: `Some(ExitReason::Menu)` on Esc,
  `Some(ExitReason::Quit)` when the window is closed (only fires because
  `Control::new()` now calls `prevent_quit()` on native — without that a close click kills
  the process before Rust ever reads the event), `None` on WASM always (nothing to return
  to there; Esc there is `xtask::hotkey_popup`'s page-level JS, not a Rust binding). Every
  game's inner loop function (`amain`/`run_ui`, or `run` for minesweeper) now returns
  `control::ExitReason` and checks `exit_requested()` once per frame, right after
  `control.handle_keys()`, breaking with the reason. `play()` (browser) and `start()`
  (standalone per-game binary) both still discard that return value via `.await;` or an
  `async move { ... }` wrapper around the `Window::from_config` call — neither of them
  ever sees anything but the loop running forever in practice, since nothing native calls
  `prevent_quit()`/produces `Some(_)` on those paths outside the shell.
- **`play_until_exit()`** is each game's third native-only entry point alongside
  `conf()`/`play()`/`start()`: `bundle/src/shell.rs`'s `run_game` dispatches to it with one
  `match` arm per game (same reasoning as the wasm dispatch `match` — every game's future
  is its own distinct type, so no shared call after a lookup). A new game needs this arm
  too, or the shell's menu will `panic!` selecting it.
- **One shell window, one native size per game**: each game's `Conf::window_width`/`window_height`
  still differs (tetris 600×720, game2048 500×610, the rest 900×720 — same as
  `xtask::native_size`, just not read from there natively). `run_game` calls
  `request_new_screen_size` to the selected game's native size before awaiting
  `play_until_exit()`, and the shell resizes back to its own 900×720 on the way back to
  the menu. This was the simplest of the three options considered (the alternatives were
  letterboxing to one fixed size, or drawing each game into an offscreen `render_target`
  and blitting it scaled) — the tradeoff is a real, visible window resize on every
  menu↔game transition, and up to a frame or two of mismatched canvas size right after
  (`request_new_screen_size` only applies on the next `next_frame().await`, not
  immediately) since games draw at absolute pixel coordinates rather than scaling to
  `screen_width()`/`screen_height()` (see "Canvas sizing is load-bearing" below — same
  constraint, different consequence natively than on the web).
- **Menu itself** (`run_menu` in `shell.rs`) is a plain, uncached per-frame redraw (no
  `RenderCache`) — arrow-key/mouse/one-finger-tap navigation, Enter/Space/click/tap to
  play. `xtask` is a native-only (`[target.'cfg(not(...))'.dependencies]` in
  `bundle/Cargo.toml`) dependency of `bundle` for its `title()` helper (game-name display
  casing) — kept off the wasm dependency graph so its own deps (`image`, `maud`, …) never
  have a reason to build for `wasm32`.
- **Menu art** (`bundle/src/menu_art.rs`): each tile is that game's own site
  `preview.png` (decoded once at shell startup and drawn "cover"-fit) with a translucent
  label strip on top. `bundle/build.rs` stages each game's `dist/<game>/preview.png` into
  `OUT_DIR` (substituting a hardcoded 1x1 stub when the real file doesn't exist yet — a
  fresh clone or `mise run clean`'d tree), and `menu_art.rs` `include_bytes!`s from
  `OUT_DIR` rather than `dist/` directly — so `cargo check`/clippy/`mise run check` always
  compile, and only the *art itself* (not the build) depends on the site build having run
  (`mise run deploy`/`build-wasm <game>`) for real thumbnails instead of a black square.
  Don't reintroduce a task-level "write a placeholder into `dist/<game>/preview.png` if
  missing" fix instead — `build-wasm`'s screenshot step skips recapture whenever that path
  already exists, so a stub written there poisons every future real deploy permanently.
  The header
  reuses the homepage's hero *scene* (`static/hotel-scene.svg`, tracked in git — not a
  `dist/` artifact) without its JS dissolve/swivel/hue-cycle animation (web-only pixel
  canvas code, no macroquad port attempted): the SVG is ~100 flat `<rect>` fills on an
  80x60 grid, hand-parsed once and redrawn as plain rectangles rather than rasterized
  through an SVG library or `resvg`/`rsvg-convert` (which `mise run rasterize` already
  needs and isn't guaranteed present).
- **Deliberately not built yet** (see `.notes/steam-standalone-menu-handoff.md` for the
  full reasoning): persisted save data (bests/last-played — every game re-seeds fresh on
  each entry today, same as a brand-new process) and Steamworks/packaging — the latter is
  a product decision (Steam Direct's $100 fee is real money, which is this project's
  standing no-spend constraint) before it's an engineering one, not something to build
  toward speculatively. `.notes/cross-platform-distribution-scope.md` scopes free
  Windows/macOS/Android/Linux distribution instead.
