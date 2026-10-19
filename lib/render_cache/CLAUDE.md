# lib/render_cache

Caches a screen region's static, once-per-tick content in an offscreen texture so a 60fps
render loop blits one quad instead of redrawing the board every frame. Why it exists and the
usage rule live in root `CLAUDE.md` ("Rendering perf"); this file is the API + gotchas.

## API

| Call | Use |
|------|-----|
| `RenderCache::new(rect)` | one per screen region; rebuild when `rect` depends on screen size and that changes |
| `.mark_dirty()` | at every point the state the closure reads changes (tick, variant switch, new episode) — not every frame |
| `.draw(\|\| ...)` | every frame; re-runs the closure only when dirty, then blits |
| `.draw_fresh(\|\| ...)` | redraw unconditionally this frame |
| `.with_backdrop(color)` | opt-in: clear to an opaque `color` before a redraw (see translucency below) |
| `.with_supersample(factor)` | opt-in: render at `factor`x and downsample on blit — approximates antialiasing |
| `.with_opacity(a)` | blit at opacity `a` |
| `prewarm_glyphs(texts, sizes)` | call once, on the default camera, **before constructing any cache** |

## Gotchas (each cost real debugging time)

- **Font atlas corruption.** Rasterizing a glyph for the first time while a cache's
  render-target camera is active corrupts all later screen text (garbled/near-black,
  positions right). Prewarm every text/size combination the game will ever draw.
- **WASM `unreachable` trap.** macroquad's `render_target()` default (`sample_count: 1`)
  still takes the MSAA-resolve path and asserts on `glCheckFramebufferStatus`, which some
  WebGL glue doesn't wire → abort. `new` uses `render_target_ex(.., sample_count: 0)` —
  never switch back to `render_target()` or to real MSAA.
- **Ghosting.** `draw()` clears to transparent before re-running the closure. Without it,
  content whose footprint changes (minesweeper square↔hex) leaves stale pixels behind.
- **Translucency renders washed out inside a cache** (blending against a transparent
  target). Fix order: (1) if the closure's first draw covers the whole `rect` opaquely,
  `with_backdrop(that_color)`; (2) otherwise, or for near-opaque draws that must match
  live rendering pixel-for-pixel, precompute the blend in Rust and draw fully opaque.
  Don't make `with_backdrop` the default — content with gaps (hex grids) needs the
  transparent clear.
- **Hard edges vs. live antialiasing.** With `sample_count: 0` cached content is
  unantialiased, so it appears to "shift a pixel" as an object alternates between live and
  cached drawing. `with_supersample(2)` made match-3's board byte-identical; cost is paid
  only on dirty redraws.
- Y-flip between render target and screen is handled internally on blit.

## Verifying a visual cache bug

No unit-test surface. Diff screenshots: the in-game `S` hotkey, or Playwright against
`mise run serve` (click the canvas first — it needs focus before key events arrive), with
the fix stashed vs. applied. To compare cached vs. live, capture the same object
mid-animation (live) and settled (cached).
