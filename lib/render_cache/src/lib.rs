use macroquad::prelude::*;

/// Caches a region of the canvas into an offscreen texture, redrawn only when the
/// caller marks it dirty instead of on every render frame.
///
/// Every self-playing game in this workspace ticks its game state on a fixed interval
/// (`TICK`, a few times a second) but was redrawing that state's *entire* visual
/// footprint every render frame (~60/sec) regardless — a 900x720 board with a few
/// hundred `draw_text`/`draw_rectangle`/bezier-curve calls costs real main-thread time
/// even under macroquad's immediate-mode API, and browsers never get to skip it since
/// nothing marks the canvas "unchanged". Measured impact fixing sudoku this way: mobile
/// Lighthouse Performance 70→95, Total Blocking Time 5.9s→130ms, Time to Interactive
/// 12.2s→3.0s (see the sudoku game's git history). Klondike/Spider's card rendering
/// (bezier-heavy suit symbols) is expensive enough that this pattern is not optional —
/// their previous unbounded per-frame redraw pushed Lighthouse's mobile trace to
/// 100+ seconds of blocking time.
///
/// Only cache content that's actually static between ticks. Genuinely continuous
/// animation (a card flying between piles, a tile sliding, a fading highlight) must
/// stay a live per-frame draw on top of the cached texture — see each game's `main.rs`
/// for the static/animated split.
pub struct RenderCache {
    target: RenderTarget,
    camera: Camera2D,
    rect: Rect,
    dirty: bool,
    backdrop: Color,
    supersample: u32,
    opacity: f32,
}

impl RenderCache {
    // `render_target()` defaults to `sample_count: 1`, but macroquad's
    // `render_target_ex` branches on `sample_count != 0` (not `> 1`) to decide whether
    // to build an MSAA resolve pass — so the "no multisampling" default still takes
    // that path and asserts `glCheckFramebufferStatus(...) != 0` on an extra resolve
    // framebuffer we don't need. On browsers/GPUs whose WebGL glue only wires that GL
    // function up for WebGL2 contexts, the call returns 0, the assert fails, and
    // `panic = "abort"` turns it into a wasm `unreachable` trap (`Uncaught
    // RuntimeError: unreachable executed` in the browser console). `sample_count: 0`
    // skips the resolve path entirely — this cache only ever blits a static texture,
    // so there's nothing to resolve anyway. See `with_supersample` for the tradeoff
    // this creates (no hardware antialiasing on cached content) and how it's worked
    // around without touching `sample_count`.
    fn build(rect: Rect, factor: u32) -> (RenderTarget, Camera2D) {
        let target = render_target_ex(
            (rect.w * factor as f32).round().max(1.0) as u32,
            (rect.h * factor as f32).round().max(1.0) as u32,
            RenderTargetParams {
                sample_count: 0,
                ..Default::default()
            },
        );
        // `Nearest` for an exact 1:1 blit (the `factor == 1` default) — no filtering
        // wanted when source and destination pixel grids match exactly. `Linear` for
        // a supersampled target instead, so shrinking it back down during `blit`
        // actually averages the extra pixels instead of just picking one — see
        // `with_supersample`.
        target.texture.set_filter(if factor > 1 {
            FilterMode::Linear
        } else {
            FilterMode::Nearest
        });
        let mut camera = Camera2D::from_display_rect(rect);
        camera.render_target = Some(target.clone());
        (target, camera)
    }

    /// `rect` is the screen-space region this cache covers, in the same absolute pixel
    /// coordinates the game already draws in — the texture is sized 1:1 to it, and the
    /// `draw` closure passed to `draw()` should keep using those same absolute
    /// coordinates unchanged (no need to offset by `rect`'s origin).
    pub fn new(rect: Rect) -> Self {
        let (target, camera) = Self::build(rect, 1);
        Self {
            target,
            camera,
            rect,
            dirty: true,
            backdrop: Color::new(0.0, 0.0, 0.0, 0.0),
            supersample: 1,
            opacity: 1.0,
        }
    }

    /// Opts this cache into clearing to an **opaque** `color` before each dirty redraw,
    /// instead of the default fully-transparent clear — use when the `draw` closure
    /// passed to `draw()` always paints its *entire* `rect` opaquely as its first draw
    /// call anyway (a solid board/panel background, say), so `color` should just be
    /// whatever that first draw call already fills with.
    ///
    /// For that kind of closure, this doesn't change how the fully-covered pixels turn
    /// out — they get overwritten by that first opaque draw regardless. What it fixes is
    /// any *translucent* draw layered on top within the same closure (a highlight, an
    /// underlay tint, a bonus-tile marker): alpha-blending onto a starting-transparent
    /// render target — which is what this cache's offscreen texture is, unlike the
    /// opaque screen framebuffer every live (uncached) draw call blends onto — composites
    /// visibly grayer than the identical draw made live. Reported independently three
    /// times in match-3 (a gem's gloss streak, its bonus-tile stripes/ring, its jelly
    /// underlay) before this got generalized instead of hand-fixed a fourth time; see
    /// `games/match-3/CLAUDE.md` for the specific measurements. Clearing to the same
    /// solid color the closure was about to paint over anyway sidesteps the whole
    /// class of bug in one place, with no more per-draw-call opaque-precompute tricks
    /// needed for *this* cache.
    ///
    /// Don't reach for this on a closure whose content doesn't fully cover `rect` (a
    /// shape that leaves gaps or corners — e.g. minesweeper's hex grid) — those still
    /// need the default transparent clear so the untouched pixels show through to
    /// whatever's really behind them on screen, not a solid `color`-filled patch.
    pub fn with_backdrop(mut self, color: Color) -> Self {
        self.backdrop = color;
        self
    }

    /// Renders into a texture `factor`x the size of `rect` in each dimension and
    /// shrinks it back down on every `blit` — gives cached content its own antialiasing
    /// (the downscale averages the extra samples) without touching `sample_count` (real
    /// MSAA needs the resolve pass `new`'s doc comment explains this cache deliberately
    /// avoids).
    ///
    /// Without this, cached content rasterizes with hard, single-sample edges while
    /// live (uncached) draws land on the default framebuffer, which typically *does*
    /// get real antialiasing from the browser's WebGL context — so a straight edge
    /// (jelly's rim, say) looks crisp when cached and ~1-2px softer when live,
    /// alternating as the game switches between the two. Reads as the edge visibly
    /// shifting, not just changing sharpness.
    ///
    /// **Important caveat on what `factor` alone does and does not fix.** Supersampling
    /// makes the cached path *antialiased*, but it does **not** make it *match the live
    /// path*, because the live path's AA comes from the browser's default framebuffer —
    /// an external, undocumented, platform-dependent source (whether the WebGL context
    /// antialiases at all, and how, varies across browsers/GPUs and their defaults). No
    /// value of `factor` reliably reproduces that; tuning `factor` until cached "looks
    /// like" live in one test browser is chasing a moving target. Measured directly on
    /// match-3: `factor: 2` overshoots a diagonal/pointed edge (a star tip) — the cached
    /// path spreads the transition over ~2 px where the live browser-AA path uses ~1 px —
    /// even though it was a good match for the shallow axis-aligned edge (jelly's rim) it
    /// was originally tuned against. The real fix for cached-vs-live *identity* is to run
    /// the live path through the same offscreen pipeline via `draw_fresh` (see its doc
    /// comment): then both paths supersample-and-downscale identically and the `factor`
    /// value stops mattering for identity — it's purely an aesthetic AA-quality knob,
    /// applied equally to both. `factor: 2` is retained on that basis (cheap, visibly
    /// smooth); it is no longer load-bearing for matching live.
    ///
    /// `factor: 2` quadruples the pixels rendered per redraw. For content only redrawn on
    /// a dirty `draw()` (once per solver tick at most) that's a rare event; for content
    /// driven every frame through `draw_fresh` it's a per-animating-frame cost — still
    /// only on frames already doing a full redraw (see `draw_fresh`).
    pub fn with_supersample(mut self, factor: u32) -> Self {
        self.supersample = factor.max(1);
        let (target, camera) = Self::build(self.rect, self.supersample);
        self.target = target;
        self.camera = camera;
        self
    }

    /// Blits the cached texture at `opacity` (0..=1) instead of fully opaque, so what's
    /// drawn underneath shows through the whole region — e.g. an overlay panel over a
    /// live scene. Applied only at blit time; the closure draws as usual.
    pub fn with_opacity(mut self, opacity: f32) -> Self {
        self.opacity = opacity.clamp(0.0, 1.0);
        self
    }

    /// Forces the next `draw()` call to actually re-run its closure instead of reusing
    /// the cached texture. Call this whenever the state `draw`'s closure reads has
    /// changed — typically once per solver tick, not once per frame.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    /// If dirty, re-renders `draw` into the cached texture (switching to this cache's
    /// render-target camera and back — `draw` should use ordinary absolute screen
    /// coordinates, same as drawing directly). Every call, dirty or not, then blits the
    /// texture back at `rect`'s position — a single draw call standing in for whatever
    /// `draw` costs.
    ///
    /// Handles the render-target-vs-screen Y-flip macroquad's `Camera2D` needs
    /// internally (`flip_y: true` on the blit) — get this wrong and cached content
    /// renders upside down; callers never need to think about it.
    ///
    /// Clears the texture to `backdrop` (fully transparent by default, or an opaque
    /// color via `with_backdrop`) before each re-render, so `draw`'s output is exactly
    /// this frame's content rather than accumulating on top of whatever the last dirty
    /// pass left behind. Without this, content that doesn't draw over the exact same
    /// pixels every time it's marked dirty (e.g. a board whose cell layout/footprint
    /// changes shape) leaves stale pixels ghosting behind the new content — see
    /// minesweeper's Square/Hex variant switch.
    pub fn draw(&mut self, mut draw: impl FnMut()) {
        if self.dirty {
            set_camera(&self.camera);
            clear_background(self.backdrop);
            draw();
            set_default_camera();
            self.dirty = false;
        }
        self.blit();
    }

    /// Renders `draw` through the exact same offscreen pipeline `draw()` uses (the
    /// `sample_count: 0` render target, `with_backdrop` clear, `with_supersample`
    /// downscale-on-blit, Y-flip) but **never caches** — it re-runs `draw` every single
    /// call. Use it for content that genuinely animates every frame, in the *same*
    /// `RenderCache` whose `draw()` you call for that content's static/idle state.
    ///
    /// Why this exists rather than "just draw the animated part live on top": cached
    /// content lives in an offscreen `sample_count: 0` target, which — see `new`'s doc
    /// comment — physically *cannot* receive the browser's default-framebuffer
    /// antialiasing (and real MSAA on the target is off-limits, it trips a WASM trap).
    /// `with_supersample` gives that cached content its own supersample-downscale AA
    /// instead. But a game that draws its animated frames *directly* to the screen still
    /// gets the browser's framebuffer AA on those — an external, undocumented,
    /// platform-dependent source (on/off by default varies across browsers/GPUs) — so the
    /// same unmoving edge is antialiased one way while animating (live, browser AA) and
    /// another way while settled (cached, supersample AA), and switching between them
    /// reads as the edge shifting. There is no way to make the *cached* path use the
    /// browser's AA (it's an offscreen target), so the only way to make the two match
    /// *by construction* — independent of whatever any given browser/GPU does — is to run
    /// the live path through this same offscreen supersample pipeline too. Routed through
    /// `draw_fresh`, an animating frame and the settled cached frame of the same content
    /// rasterize through byte-for-byte identical machinery, so a static element (a cell a
    /// swap didn't touch, say) is pixel-identical whether it's currently drawn live or
    /// cached. See `games/match-3`'s render loop for the reference use.
    ///
    /// Cost: this re-renders into the (possibly supersampled) target every frame it's
    /// called, so only use it for frames that were *already* doing a full redraw anyway
    /// (match-3's board is genuinely mid-animation — nothing to cache — for those frames).
    /// It is not a substitute for `draw()` on static content: that's the whole point of
    /// the cache. The extra cost over drawing live-to-screen is the supersample fill
    /// factor plus one downscale blit; the CPU-side draw-call count is unchanged, so it
    /// doesn't reintroduce the per-frame *draw-call* overhead `RenderCache` exists to
    /// avoid — it only pays more GPU fill on frames that redraw regardless.
    ///
    /// Leaves `dirty` set afterward, so the next `draw()` (e.g. the first settled frame
    /// after animation stops) re-renders the cache rather than blitting this last
    /// animated frame.
    pub fn draw_fresh(&mut self, draw: impl FnMut()) {
        self.dirty = true;
        self.draw(draw);
        self.dirty = true;
    }

    fn blit(&self) {
        draw_texture_ex(
            &self.target.texture,
            self.rect.x,
            self.rect.y,
            Color::new(1.0, 1.0, 1.0, self.opacity),
            DrawTextureParams {
                dest_size: Some(vec2(self.rect.w, self.rect.h)),
                flip_y: true,
                ..Default::default()
            },
        );
    }
}

/// Rasterizes every combination of `texts` x `font_sizes` into macroquad's shared font
/// atlas up front. Call this once, on the default camera, before constructing any
/// `RenderCache` that will draw text — growing the atlas (rasterizing a glyph/size
/// combination for the first time) while a `RenderCache`'s render-target camera is
/// active corrupts subsequently-drawn screen text (garbled, near-black) for the rest
/// of the run. Confirmed by bisection on Klondike: neither the card table nor the HUD
/// alone triggered it at any content size, only the combination, once enough distinct
/// rank glyphs were needed to grow the atlas past its initial size — pre-warming here
/// forces that growth to happen safely while still on the default camera.
pub fn prewarm_glyphs(texts: &[&str], font_sizes: &[u16]) {
    for &size in font_sizes {
        for &text in texts {
            measure_text(text, None, size, 1.0);
        }
    }
}
