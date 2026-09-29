//! Generates dist/index.html (the game list) and dist/sitemap.xml.
use maud::{DOCTYPE, Markup, PreEscaped, html};
use std::path::Path;
use xtask::{
    all_games, base_url, description, favicon_links, gtag_head, homepage_json_ld, manifest_json,
    minify_js, preview_srcset, pwa_head, social_image, sw_register_bridge, theme_init_script,
    title, wall_analytics_bridge, wall_json_ld, wall_live_bridge,
};

/// Feeds `meta description`, `og:description`, the PWA manifest's `description` and the
/// `WebSite` JSON-LD node — i.e. everything a *machine* reads, and nothing a visitor sees on
/// the page. A meta description is not a ranking input at all, only the SERP snippet, so this
/// one is written for click-through: a claim first, then the game names a person actually
/// scans a result for. The on-page pitch (`SITE_PITCH`) is the one that carries phrase-match
/// weight, and it's deliberately worded differently.
const SITE_DESCRIPTION: &str = "Just games. No player needed. Zero-player browser games where AI bots solve Snake, 2048, Klondike, Minesweeper and more, live.";

/// The visible pitch on the homepage, under the "games" heading. Unlike `SITE_DESCRIPTION`
/// this is real body text, which *is* a ranking input — so it carries both category
/// phrasings the site should own ("zero-player", "play themselves") while opening in the
/// site's own voice. It sat in the header until that grew to h1 + kicker + three lines +
/// link; keep it somewhere on the page, wherever it goes.
const SITE_PITCH: &str = "Just games. No player needed. Free zero-player browser games that play themselves — AI bots solve Snake, 2048, Klondike and more, live.";

// Archivo is only ever used at its default weight (400) — the STYLE block never sets
// font-weight on anything in the Archivo family, only on Fraunces — so 500/600 aren't
// requested here; trims one @font-face block (and its file) Google Fonts would otherwise
// serve unused.
const FONTS_HREF: &str = "https://fonts.googleapis.com/css2?family=Archivo:wght@400&family=Fraunces:ital,wght@0,600;1,500&display=swap";

const STYLE: &str = r#"
:root {
  color-scheme: dark;
  --bg: #171310;
  --cream: #f0ece2;
  --cream-dim: #ede6d4;
  --ink: #2c2015;
  --ink-soft: #5b4b36;
  --ink-faint: #786451;
  --text: #e7ddcd;
  --text-dim: #a89a86;
  --accent: #d4a373;
  --border: rgba(212, 163, 115, 0.18);
  --heading: #f0ece2;
  --card-bg: #1c1712;
  --card-title: #e7c98f;
  --shadow: rgba(0, 0, 0, 0.65);
  --postcard-shadow: rgba(0, 0, 0, 0.4);
}

/* Set by xtask::theme_init_script from the hcg_theme cookie / prefers-color-scheme. */
:root[data-theme="light"] {
  color-scheme: light;
  --bg: #f5efe4;
  --cream: #fffdf8;
  --text: #3a2d1f;
  --text-dim: #6e5d48;
  --accent: #9a5b22;
  --border: rgba(154, 91, 34, 0.22);
  --heading: #2c2015;
  --card-bg: #fffaf2;
  --card-title: #7a4a1c;
  --shadow: rgba(60, 40, 20, 0.28);
  --postcard-shadow: rgba(60, 40, 20, 0.18);
}

* { margin: 0; padding: 0; box-sizing: border-box; }

html { overflow-x: hidden; }

body {
  background: var(--bg);
  color: var(--text);
  font-family: 'Archivo', system-ui, sans-serif;
  min-height: 100vh;
  min-height: 100dvh;
  display: flex;
  flex-direction: column;
  align-items: center;
  overflow-x: hidden;
  padding-bottom: env(safe-area-inset-bottom);
}

@keyframes fadeUp {
  from { opacity: 0; transform: translateY(14px); }
  to { opacity: 1; transform: none; }
}

.fade-up { animation: fadeUp 0.6s ease both; }
header.fade-up { animation-delay: 0s; }
.scene-card.fade-up { animation-delay: 0.12s; }
.postcards.fade-up { animation-delay: 0.24s; }
.games.fade-up { animation-delay: 0.36s; }

header {
  padding: 3rem 1rem 1rem;
  text-align: center;
}

header h1 {
  font-family: 'Fraunces', serif;
  font-weight: 600;
  font-size: clamp(1.7rem, 7vw, 2.5rem);
  color: var(--heading);
}

header .kicker {
  margin-top: 0.6rem;
  font-size: 0.7rem;
  letter-spacing: 0.14em;
  text-transform: uppercase;
  color: var(--accent);
  opacity: 0.85;
}

/* Sits under the "games" heading rather than in the header. It used to follow the kicker,
   which stacked h1 + joke + three lines of prose + link before the page showed anything —
   too tall a wall of text to open on. Down here it introduces the grid it describes, and
   since it's still real body text on the page its ranking weight is unchanged (which is why
   it was moved rather than cut; see SITE_PITCH). Left-aligned to the grid, not centred like
   the header. No max-width: it was capped when it sat in the header stack, where a wide
   banner of prose competed with the h1 — down here it just runs the grid's own width. */
.games .pitch {
  margin: -0.4rem 0 1.2rem;
  font-size: 0.85rem;
  line-height: 1.65;
  color: var(--text-dim);
}

header .wall-link {
  display: inline-block;
  margin-top: 0.8rem;
  font-size: 0.75rem;
  color: var(--text-dim);
  text-decoration: none;
  border-bottom: 1px solid transparent;
}

header .wall-link:hover {
  color: var(--accent);
  border-bottom-color: var(--accent);
}

.main {
  display: flex;
  flex-wrap: nowrap;
  gap: 2.5rem;
  align-items: flex-end;
  justify-content: center;
  max-width: 900px;
  width: 95%;
  margin: 2rem 0 3rem;
}

.scene-card {
  position: relative;
  flex-shrink: 0;
  width: 100%;
  max-width: 504px;
  padding: 12px;
  border-radius: 14px;
  background: linear-gradient(160deg, rgba(212, 163, 115, 0.09), rgba(0, 0, 0, 0) 60%);
  box-shadow: 0 30px 60px -24px var(--shadow);
}

.scene-card::before {
  content: "";
  position: absolute;
  inset: -40px;
  z-index: -1;
  background: radial-gradient(circle at 50% 40%, rgba(212, 163, 115, 0.16), transparent 70%);
  filter: blur(20px);
  pointer-events: none;
}

#hotel {
  display: block;
  width: 100%;
  max-width: 480px;
  height: auto;
  aspect-ratio: 4 / 3;
  border-radius: 6px;
  image-rendering: pixelated;
  image-rendering: crisp-edges;
}

.postcards {
  flex: 0 1 260px;
  min-width: 200px;
  max-width: 280px;
  margin-bottom: 0.5rem;
  display: flex;
  flex-direction: column;
  align-items: center;
}

/* A fixed-height window onto the taller `.postcard-track` beneath it — 3 card-heights
   plus the 2 gaps between them, so exactly 3 cards show and the rest are clipped rather
   than just squeezed. Real up/down reel motion (see POSTCARD_SCRIPT_TEMPLATE) needs a
   clipped viewport around a longer strip of real cards; there's no way to get that look
   from 3 fixed elements whose content merely swaps in place. */
.postcard-viewport {
  position: relative;
  width: 100%;
  overflow: hidden;
  height: calc(9rem * 3 + 0.85rem * 2);
}

.postcard-track {
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
  width: 100%;
  /* Tells the browser not to hijack a vertical drag here for page-scroll/pinch-zoom —
     Pointer Events (see POSTCARD_SCRIPT_TEMPLATE) need uninterrupted pointermove while
     dragging, since the reel scrolls the same axis as the page itself. */
  touch-action: none;
  cursor: grab;
  transition: transform 0.4s cubic-bezier(0.22, 1, 0.36, 1);
}

.postcard-track.dragging {
  cursor: grabbing;
  transition: none;
}

.postcard-slot {
  height: 9rem;
  /* Flex items default to `min-height: auto`, which lets their content-based minimum
     size win over an explicit `height` — the fixed reel-step math below breaks if cards
     aren't all exactly 9rem regardless of quote length. `min-height: 0` opts out. */
  min-height: 0;
  flex: 0 0 auto;
  background: var(--cream);
  color: var(--ink);
  border-radius: 4px;
  padding: 1rem 1.1rem;
  box-shadow: 0 12px 24px var(--postcard-shadow);
  display: flex;
  flex-direction: column;
  justify-content: center;
  transform: rotate(var(--r, 0deg));
}

.postcard-slot blockquote {
  font-family: 'Fraunces', serif;
  font-style: italic;
  font-weight: 500;
  font-size: 0.85rem;
  line-height: 1.5;
  overflow-wrap: break-word;
}

.postcard-slot cite {
  display: block;
  margin-top: 0.55rem;
  font-style: normal;
  font-size: 0.6rem;
  letter-spacing: 0.01em;
  color: var(--ink-faint);
}

/* Rotated 90deg rather than new glyphs: the existing left/right chevrons (‹/›) read as
   up/down once turned, so prev/next keep the same characters and meaning, just re-oriented
   to match the stack's vertical axis. Hidden until hover/focus rather than always-on — the
   stack already conveys "there's more" via autoplay motion, so a permanently visible
   button pair above/below it would be redundant chrome most of the time. */
.postcard-arrow {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  border-radius: 50%;
  border: 1px solid rgba(212, 163, 115, 0.4);
  background: transparent;
  color: var(--accent);
  font-family: 'Fraunces', serif;
  font-size: 1.1rem;
  line-height: 1;
  /* ‹/› fall back out of Fraunces to the browser's default serif for this glyph, whose
     vertical metrics sit lower in the em box — this padding re-centers the visible glyph
     ink in the circle rather than the font's own line box. */
  padding-bottom: 0.28em;
  cursor: pointer;
  transform: rotate(90deg);
  opacity: 0;
  transition: opacity 0.2s ease, background 0.15s, border-color 0.15s, transform 0.15s;
}

.postcards:hover .postcard-arrow,
.postcards:focus-within .postcard-arrow {
  opacity: 1;
}

.postcard-arrow:hover, .postcard-arrow:focus-visible {
  background: rgba(212, 163, 115, 0.15);
  border-color: var(--accent);
  opacity: 1;
}

.postcard-arrow:active { transform: rotate(90deg) scale(0.92); }

.postcard-prev { margin-bottom: 0.4rem; }
.postcard-next { margin-top: 0.4rem; }

.games {
  width: 95%;
  max-width: 960px;
  margin: 0 0 3rem;
}

.games h2 {
  font-size: 0.75rem;
  letter-spacing: 0.15em;
  color: var(--text-dim);
  text-transform: uppercase;
  margin-bottom: 1rem;
  font-weight: normal;
}

.game-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
  gap: 1rem;
}

.game-card {
  display: flex;
  flex-direction: column;
  border-radius: 0.75rem;
  background: var(--card-bg);
  border: 1px solid var(--border);
  text-decoration: none;
  color: inherit;
  overflow: hidden;
  transition: border-color 0.15s, transform 0.15s;
  touch-action: manipulation;
  -webkit-tap-highlight-color: transparent;
}

.game-card:hover, .game-card:active {
  border-color: var(--accent);
  transform: translateY(-2px);
}

.game-card img {
  display: block;
  width: 100%;
  aspect-ratio: 4 / 3;
  object-fit: cover;
  background: var(--bg);
}

.game-card .card-body {
  padding: 0.75rem 1rem 1rem;
}

.game-card h3 {
  font-family: 'Fraunces', serif;
  font-size: 1.05rem;
  font-weight: 600;
  color: var(--card-title);
  margin-bottom: 0.35rem;
}

.game-card p {
  font-size: 0.8rem;
  line-height: 1.4;
  color: var(--text-dim);
}

@media (prefers-reduced-motion: reduce) {
  .fade-up { animation: none; opacity: 1; transform: none; }
  .postcard-track { transition: none; }
}

@media (max-width: 720px) {
  header { padding: 2rem 1rem 0.5rem; }

  .main {
    flex-direction: column;
    align-items: center;
    gap: 1.75rem;
    margin: 1rem 0 2rem;
  }

  .scene-card { max-width: 444px; margin: 0 auto; }

  .postcards { margin-bottom: 0; max-width: 420px; }

  .game-grid { grid-template-columns: repeat(auto-fill, minmax(150px, 1fr)); }
}
"#;

// Each tile's WASM instance still renders its game at full native resolution internally
// (canvas backing size follows clientWidth/height × devicePixelRatio, independent of the
// tile's on-screen CSS size — see xtask::native_size_style's doc comment on why the box
// can't just be shrunk directly), so this page's GPU/CPU cost scales with tile count same
// as opening that many game tabs at once. `wall_live_bridge` caps how many tiles are
// simultaneously live (mounted iframe vs. static preview `<img>`) rather than shrinking
// per-tile resolution — see its doc comment for the budget rule and why (iOS Safari's
// WebGL context cap).
const WALL_STYLE: &str = r#"
:root { color-scheme: dark; --bg: #171310; --text: #e7ddcd; --text-dim: #a89a86; --accent: #d4a373; }
:root[data-theme="light"] { color-scheme: light; --bg: #f5efe4; --text: #3a2d1f; --text-dim: #6e5d48; --accent: #9a5b22; }
* { margin: 0; padding: 0; box-sizing: border-box; }
html, body { background: var(--bg); color: var(--text); min-height: 100%; }
body { font-family: 'Archivo', system-ui, sans-serif; }
header { padding: 1.5rem 1rem 1rem; text-align: center; }
header a { color: var(--text-dim); font-size: 0.8rem; text-decoration: none; }
header a:hover { color: var(--accent); }
header h1 { font-family: 'Fraunces', serif; font-weight: 600; font-size: clamp(1.4rem, 5vw, 2rem); margin-top: 0.4rem; }
header p { margin-top: 0.4rem; font-size: 0.8rem; color: var(--text-dim); }
.wall-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(280px, 1fr));
  gap: 12px;
  padding: 1rem;
}
.wall-tile {
  position: relative;
  width: 100%;
  aspect-ratio: 5 / 4;
  border-radius: 6px;
  background: #000;
  overflow: hidden;
  cursor: pointer;
}
.wall-tile img.wall-preview {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}
.wall-tile iframe.wall-live {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  border: none;
  display: block;
}
/* Per-tile link to that game's own page. Present in the DOM on every load (an `<a href>` is
   followed regardless of how it's styled, so this is 11 real internal links from a page that
   previously had exactly one), but revealed only on hover/focus so the grid still reads as an
   uninterrupted video wall at rest. z-index puts it above the live iframe, which otherwise
   covers the whole tile once mounted. */
.wall-tile .wall-label {
  position: absolute;
  bottom: 0;
  left: 0;
  right: 0;
  z-index: 2;
  padding: 0.5rem 0.6rem;
  background: linear-gradient(to top, rgba(0, 0, 0, 0.85), rgba(0, 0, 0, 0));
  color: #e7ddcd;
  font-size: 0.8rem;
  text-decoration: none;
  opacity: 0;
  transition: opacity 0.18s;
}
.wall-tile:hover .wall-label,
.wall-tile:focus-within .wall-label { opacity: 1; }
.wall-tile .wall-label:hover { color: #d4a373; }
/* No hover on a touch screen, so the label would be permanently unreachable there — show it
   outright instead. Tapping the label navigates; tapping anywhere else in the tile still
   mounts the live game (see wall_live_bridge). */
@media (hover: none) {
  .wall-tile .wall-label { opacity: 1; }
}
"#;

/// The hero scene draws in four separate 80x60 layers rather than one, so the chair can be
/// swapped without touching the room: `back` (wall/floor/curtains/window) and `front` (the bed,
/// which occludes the chair) are painted once and never repainted; the two chair sprites —
/// hotel armchair and gaming chair — are painted once each into transparent layers, and every
/// frame of the transition picks each chair pixel from one sprite or the other by an ordered
/// threshold, then composites back -> chair -> front and blits the result upscaled.
///
/// The pick is a **dither dissolve**, not an alpha crossfade: cross-fading two pixel-art
/// sprites shows a half-transparent double image (both chairs visible at once, muddy colors
/// that exist in neither palette) and looks like a rendering bug. Flipping whole pixels one
/// at a time on a per-pixel threshold keeps every intermediate frame made only of real palette
/// colors. The threshold is mostly vertical (bottom rows flip first, so the gaming chair grows
/// up off the floor) with a Bayer 4x4 term mixed in to break the sweep line into pixel grain.
/// Pixels at the leading edge of the sweep flash cyan for a few frames — the chair powering on.
///
/// As the dissolve runs the room dims and the bed lights up neon (a faint pool of light on the
/// floor under the frame — never behind the bed, see the halo build for why — a `lighter` tint on
/// the frame/headboard, and an under-frame LED strip). Both ramp in over the dissolve's first third and then stay: the page's resting state
/// is the night room with a glowing bed and the gaming chair, not the daylit room it loads with.
/// The light's *hue* cycles RGB-peripheral style while the chair dissolves and eases onto a fixed
/// pink by the end; only its brightness keeps moving afterwards, on a slow breathe.
///
/// The dissolve runs once, ~1.5s after load. The breathe that follows it is throttled to ~15fps,
/// costs no per-pixel work (only the `t`-dependent chair layer does, and `t` stops moving), and
/// is suspended whenever the hero is scrolled out of view or the tab is hidden — this page's
/// canvas art must not sit on the main thread, same concern that motivated `RenderCache`. Under
/// `prefers-reduced-motion` the end state is painted once and no loop ever starts.
const HOTEL_SCENE_SCRIPT: &str = r#"
(function () {
  const canvas = document.getElementById('hotel');
  if (!canvas) return;
  const ctx = canvas.getContext('2d');
  ctx.imageSmoothingEnabled = false;

  const W = 80, H = 60;

  function layer() {
    const cv = document.createElement('canvas');
    cv.width = W; cv.height = H;
    const c = cv.getContext('2d');
    c.imageSmoothingEnabled = false;
    return { cv, c };
  }

  const back = layer(), front = layer();
  const armchair = layer(), gamer = layer(), chair = layer();
  const gamerBase = layer(), gamerTop = layer();
  const halo = layer(), bedNeon = layer(), tint = layer(), curtains = layer();
  const floorLit = layer();

  let c = back.c;

  function px(x, y, w, h, col) {
    c.fillStyle = col;
    c.fillRect(Math.round(x), Math.round(y), Math.round(w), Math.round(h));
  }

  // wall + floor
  px(0, 0, W, 42, '#b89468');
  px(0, 0, W,  2, '#a08258');
  px(0, 41, W,  1, '#8a7048');
  for (let y = 42; y < H; y++) {
    px(0, y, W, 1, y % 4 < 2 ? '#7a5e1e' : '#6e5418');
  }
  for (let x = 6; x < W; x += 10) { px(x, 42, 1, H - 42, '#5a4010'); }

  // curtain rod
  px(8, 3, 54, 1, '#c0a060');
  px(8, 4, 54, 1, '#907040');
  px( 7, 2, 3, 3, '#c8a858');
  px(60, 2, 3, 3, '#c8a858');

  // window
  px(22,  5, 26, 32, '#3a2818');
  px(24,  7, 22, 28, '#7ab0d4');
  px(25,  8,  9, 10, '#8ec0e0');
  px(37,  8,  9, 10, '#8ec0e0');
  px(25, 20,  9, 14, '#6898b8');
  px(37, 20,  9, 14, '#6898b8');
  px(34,  7,  2, 28, '#3a2818');
  px(24, 19, 22,  2, '#3a2818');
  px(21, 36, 28,  3, '#4a3820');
  px(22, 37, 26,  2, '#6a5030');
  px(46,  7,  2, 28, '#c8a87a');
  px(22, 35, 22,  2, '#c0a070');

  // ── curtains — own layer, repainted as they close over the transition ──────
  // The panels stay pinned to the rod's ends and grow *inward*, 12px gathered to 25px spread:
  // the fabric bunched at the sides is what fills the middle when a real curtain closes. They
  // live in their own layer rather than in `back` because once closed they cover the window —
  // which is also what darkens it, instead of tinting the glass.
  const CUR = '#1e3e5e', CUR_HI = '#2a5070', CUR_LO = '#162e48';
  let curW = -1;

  function curtainsAt(close) {
    const w = 12 + Math.round(13 * Math.min(1, Math.max(0, close)));
    if (w === curW) return;
    curW = w;
    const cc = curtains.c;
    cc.clearRect(0, 0, W, H);
    for (const x0 of [10, 60 - w]) {
      cc.fillStyle = CUR; cc.fillRect(x0, 5, w, 37);
      for (let i = 1; i < w; i += 4) { cc.fillStyle = CUR_HI; cc.fillRect(x0 + i, 5, 1, 37); }
      for (let i = 2; i < w; i += 4) { cc.fillStyle = CUR_LO; cc.fillRect(x0 + i, 5, 1, 37); }
    }
    // Each panel's inner edge hangs a little heavier than the rest of the fold pattern.
    cc.fillStyle = CUR_HI;
    cc.fillRect(7 + w, 36, 3, 6); cc.fillRect(60 - w, 36, 3, 6);
    cc.fillStyle = CUR_LO;
    cc.fillRect(8 + w, 38, 2, 4); cc.fillRect(61 - w, 38, 2, 4);
  }

  // ── armchair — side profile facing right (toward bed), own layer ──────────
  c = armchair.c;
  const cx = 21;
  // chair back: tall post on the left
  px(cx,    32,  4, 21, '#5c1e0e');
  px(cx+1,  33,  2, 19, '#782a18');
  // tufting buttons (side view)
  px(cx+1,  37,  1,  2, '#8a3020');
  px(cx+1,  42,  1,  2, '#8a3020');
  // armrest rail (horizontal, connects back to seat front)
  px(cx,    44, 14,  2, '#501808');
  px(cx+1,  44, 13,  1, '#6a2010');
  // seat cushion
  px(cx+3,  46, 11,  8, '#702818');
  px(cx+4,  47,  9,  6, '#8a3020');
  px(cx+4,  47,  9,  1, '#9a3828');
  px(cx+4,  51,  9,  1, '#602010');
  // seat front face
  px(cx+3,  54, 11,  2, '#5a2010');
  // legs
  px(cx+3,  55,  2,  3, '#3a1008');
  px(cx+11, 55,  2,  3, '#3a1008');
  // floor shadow (left portion only, right hidden by bed)
  px(cx, 57, 12, 3, 'rgba(0,0,0,0.15)');

  // ── gaming chair — same anchor, same facing, taller shell + caster base ───
  // Split across two layers at the gas cylinder: `gamerBase` is bolted to the floor, `gamerTop`
  // is everything the chair swivels on, so the resting loop can sway the top while the casters
  // stay put. `gamer` is the two flattened together, which is what the dissolve consumes.
  c = gamerBase.c;
  const INK = '#20202a', INK_HI = '#2e2e3c', INK_LO = '#15151d';
  const RED = '#d92b3a', RED_HI = '#f04a58';
  const STEEL = '#3a3a4a', STEEL_HI = '#50506a';
  // floor shadow (wider than the armchair's — five-star base)
  px(cx-1, 57, 17, 3, 'rgba(0,0,0,0.2)');
  // five-star base + casters
  px(cx+1,  55, 13, 1, '#2c2c38');
  px(cx+2,  56, 11, 1, INK_LO);
  px(cx,    55,  3, 3, '#16161e');
  px(cx+12, 55,  3, 3, '#16161e');
  px(cx+1,  55,  1, 1, STEEL);
  px(cx+13, 55,  1, 1, STEEL);
  // gas cylinder
  px(cx+6,  49,  3, 6, STEEL);
  px(cx+7,  49,  1, 6, STEEL_HI);
  px(cx+8,  52,  1, 1, '#c07aff');

  c = gamerTop.c;
  // seat pan, red piping along the side
  px(cx+3,  45, 12, 4, INK);
  px(cx+3,  45, 12, 1, INK_HI);
  px(cx+3,  48, 12, 1, RED);
  px(cx+14, 46,  1, 2, INK_LO);
  // backrest — reclined, each segment shifted left of the one below it
  px(cx+2,  40,  5, 6, INK);
  px(cx+6,  40,  1, 6, RED);
  px(cx+1,  34,  6, 6, INK);
  px(cx+6,  34,  1, 6, RED);
  px(cx,    30,  5, 5, INK);
  px(cx+4,  30,  1, 5, RED);
  px(cx+1,  35,  1, 10, INK_HI);
  // headrest — pillow sitting proud of the shell top
  px(cx-1,  26,  6, 3, INK_HI);
  px(cx+4,  26,  1, 3, RED_HI);
  px(cx-1,  29,  6, 1, INK_LO);
  // lumbar pillow
  px(cx+5,  37,  2, 4, '#b02030');
  px(cx+5,  37,  2, 1, RED_HI);
  // armrest
  px(cx+7,  41,  6, 2, INK);
  px(cx+7,  41,  6, 1, INK_HI);
  px(cx+9,  43,  2, 3, INK_LO);

  gamer.c.drawImage(gamerBase.cv, 0, 0);
  gamer.c.drawImage(gamerTop.cv, 0, 0);

  // ── bed — drawn on top of chair ──────────────────────────────────────────
  c = front.c;
  const bx = 44;
  // headboard (original proportions: 29px wide)
  px(bx,    29, 29, 15, '#3e2010');
  px(bx+1,  30, 27, 13, '#6a3818');
  px(bx+2,  31, 25, 11, '#5a3010');
  // headboard panels (symmetric)
  px(bx+3,  32, 10,  9, '#7a4020');
  px(bx+15, 32, 10,  9, '#7a4020');
  px(bx+4,  33,  8,  7, '#8a4c28');
  px(bx+16, 33,  8,  7, '#8a4c28');
  px(bx+3,  32,  1,  9, '#4a2810');
  px(bx+15, 32,  1,  9, '#4a2810');
  // frame side
  px(bx,    43, 29, 14, '#4a3018');
  px(bx+1,  44, 27, 12, '#5a3c20');
  // duvet
  px(bx+1,  39, 27, 18, '#ddd5c0');
  px(bx+2,  40, 25, 16, '#ede6d4');
  px(bx+2,  43, 25,  1, '#c8c0a8');
  px(bx+2,  47, 25,  1, '#c8c0a8');
  px(bx+2,  51, 25,  1, '#c8c0a8');
  px(bx+1,  40,  1, 17, '#c0b898');
  px(bx+27, 40,  1, 17, '#c0b898');
  // pillows
  px(bx+2,  38, 11,  7, '#f0ece2');
  px(bx+15, 38, 11,  7, '#f0ece2');
  px(bx+3,  39,  9,  5, '#ffffff');
  px(bx+16, 39,  9,  5, '#ffffff');
  // legs
  px(bx+2,  54,  2,  3, '#2e1808');
  px(bx+26, 54,  2,  3, '#2e1808');
  // floor shadow
  px(bx+1, 57, 27, 3, 'rgba(0,0,0,0.18)');

  // ── chair dissolve ────────────────────────────────────────────────────────
  const from = armchair.c.getImageData(0, 0, W, H);
  const to = gamer.c.getImageData(0, 0, W, H);
  const mix = chair.c.createImageData(W, H);

  // Per-pixel flip threshold: 0 = flips first. Vertical sweep (floor upward) carrying a
  // Bayer 4x4 dither so the boundary is grain, not a straight line.
  const BAYER = [0, 8, 2, 10, 12, 4, 14, 6, 3, 11, 1, 9, 15, 7, 13, 5];
  const TOP = 24, BOTTOM = 60;
  const thresh = new Float32Array(W * H);
  for (let y = 0; y < H; y++) {
    const v = Math.min(1, Math.max(0, (BOTTOM - y) / (BOTTOM - TOP)));
    for (let x = 0; x < W; x++) {
      const d = (BAYER[(y % 4) * 4 + (x % 4)] + 0.5) / 16;
      thresh[y * W + x] = 0.76 * v + 0.24 * d;
    }
  }

  // ── neon bed lighting ─────────────────────────────────────────────────────
  // `halo` is light pooling on the **floor** under the bed, and nowhere else. Stacking the same
  // silhouette at every offset in a kernel at low alpha builds the falloff for free — alpha
  // accumulates near the bed and thins at the rim. `bedNeon` is the flat silhouette, drawn back
  // over the bed with `lighter` so the dark frame/headboard picks up the glow while the white
  // duvet clips and stays white. Both are stored as **white** masks, not pre-colored: the light
  // cycles hue during the transition (see `ledHue`), so each frame re-tints a mask through
  // `source-in` (which replaces color but keeps the mask's alpha profile) into a scratch layer.
  //
  // Two things this deliberately does *not* do, both of which it used to:
  //
  // 1. Glow behind the bed. The whole silhouette used to be dilated by a round 2px, so the
  //    headboard — a vertical panel standing against the wall — threw the same ring of light as
  //    the mattress, and the two read as one flat lightbox panel hung on the wall rather than
  //    two perpendicular planes. The emitter is a strip under the frame; the only surface it
  //    can reach is the floor, so nothing above the frame is dilated at all.
  // 2. Spread the *whole* lower silhouette sideways — that just made a bed-shaped slab of flat
  //    magenta with a hard edge, a painted rectangle rather than a pool. Only the base band
  //    (frame bottom, legs, strip) spreads, so the falloff starts where the emitter is.
  //
  // The kernel is wide and flat rather than round: floor seen at this shallow an angle is
  // foreshortened, so a circle of light on it reaches much further sideways than up or down.
  // Alpha is kept low enough that the floor's plank texture still reads through the pool.
  const BASE_Y = 51;
  floorLit.c.drawImage(front.cv, 0, 0);
  floorLit.c.globalCompositeOperation = 'destination-out';
  floorLit.c.fillRect(0, 0, W, BASE_Y);

  halo.c.globalAlpha = 0.028;
  const RX = 13, UP = 4, DOWN = 3;
  for (let dx = -RX; dx <= RX; dx++) {
    for (let dy = -UP; dy <= DOWN; dy++) {
      const ry = dy < 0 ? UP : DOWN;
      if ((dx * dx) / (RX * RX) + (dy * dy) / (ry * ry) > 1) continue;
      halo.c.drawImage(floorLit.cv, dx, dy);
    }
  }
  halo.c.globalAlpha = 1;
  halo.c.globalCompositeOperation = 'source-in';
  halo.c.fillStyle = '#ffffff';
  halo.c.fillRect(0, 0, W, H);
  // The mask stays *solid* across the base's own footprint instead of having the bed punched back
  // out of it. The bed is drawn over the halo anyway, so the hole was never visible at scale 1 —
  // but the pulse scales the halo up, which scaled the hole up too and opened a dark ring between
  // the bed and its own glow. A solid mask can be enlarged with nothing to show through.

  bedNeon.c.drawImage(front.cv, 0, 0);
  bedNeon.c.globalCompositeOperation = 'source-in';
  bedNeon.c.fillStyle = '#ffffff';
  bedNeon.c.fillRect(0, 0, W, H);

  function tinted(mask, col) {
    const c2 = tint.c;
    c2.globalCompositeOperation = 'source-over';
    c2.clearRect(0, 0, W, H);
    c2.drawImage(mask.cv, 0, 0);
    c2.globalCompositeOperation = 'source-in';
    c2.fillStyle = col;
    c2.fillRect(0, 0, W, H);
    c2.globalCompositeOperation = 'source-over';
    return tint.cv;
  }

  // Room-dim / bed-glow strength: 0 before the dissolve starts, ramps in over its first third,
  // then *stays* — night room + lit bed is the resting state, not a passing effect.
  function envelope(t) {
    if (t <= 0) return 0;
    return Math.min(1, t / 0.28);
  }

  // RGB-peripheral hue cycle: spins fast for almost the whole dissolve, then snaps onto the
  // resting neon purple over the last fifth — the cycle owns the transition, the purple owns the
  // rest. (Settling from halfway through washed the RGB out under the resting hue long before the
  // transition was over.) Blending takes the shortest way round the wheel, or a settle from hue 20
  // to 283 would run the long way through green.
  const REST_HUE = 283;
  function ledHue(t) {
    if (t >= 1) return REST_HUE;
    const spin = (t * 860) % 360;
    const settle = Math.min(1, Math.max(0, (t - 0.78) / 0.22));
    const d = ((REST_HUE - spin + 540) % 360) - 180;
    return (spin + d * settle + 360) % 360;
  }

  // Same hue as an [r,g,b] triple, for the dissolve's leading-edge sparkle pixels — those are
  // written straight into the chair's ImageData, which can't take a CSS color string.
  function hueRgb(h) {
    const f = (n) => {
      const k = (n + h / 30) % 12;
      return Math.round(255 * (0.62 - 0.38 * Math.max(-1, Math.min(k - 3, 9 - k, 1))));
    };
    return [f(0), f(8), f(4)];
  }

  // The dither dissolve only depends on `t`, so once `t` stops moving (the resting breathe loop
  // holds it at 2) the chair layer is left alone and a rest frame costs a handful of drawImage
  // calls on an 80x60 surface, no per-pixel work at all.
  let mixT = NaN;

  // Everything composites at the canvas's own resolution (S = 6x the 80x60 art), not at 80x60
  // followed by one upscale. That only matters for the swiveling chair, and it matters a lot: a
  // shear applied in 80x60 space quantizes the lean to whole art pixels, so the chair jumps 6
  // screen pixels at a time and reads as juddering pixel noise rather than motion. Sheared at
  // output scale the same lean lands on 1/6-art-pixel steps — the blocks stay crisp (nearest
  // sampling, `imageSmoothingEnabled = false`) but their edges move smoothly.
  const S = canvas.width / W;
  const blit = (s, cv) => s.drawImage(cv, 0, 0, W * S, H * S);

  // A real yaw can't be drawn from one side-profile sprite, and at this scale the honest
  // foreshortening of a small turn (width * cos 12deg) is under half a pixel — invisible. So the
  // swivel is a shear about the top of the gas cylinder instead: the shell leans while the seat
  // barely moves and the casters not at all, which is what a chair being idly twisted looks like.
  // `sx` narrows the shell slightly at the extremes of the sway, borrowing the one part of the
  // real projection that does read at 80x60.
  const PIVOT_X = 28, PIVOT_Y = 51;

  function drawSwivel(s, sway) {
    blit(s, gamerBase.cv);
    const k = 0.115 * sway;
    const sx = 1 - 0.05 * Math.abs(sway);
    s.setTransform(sx, 0, k, 1, S * (PIVOT_X * (1 - sx) - k * PIVOT_Y), 0);
    blit(s, gamerTop.cv);
    s.setTransform(1, 0, 0, 1, 0, 0);
  }

  function paint(t, pulse, wob, sway, chase) {
    if (t !== mixT) {
      const a = from.data, b = to.data, m = mix.data;
      const [sr, sg, sb] = hueRgb(ledHue(t));
      for (let i = 0, p = 0; p < thresh.length; i += 4, p++) {
        const th = thresh[p];
        const src = t > th ? b : a;
        m[i] = src[i]; m[i+1] = src[i+1]; m[i+2] = src[i+2]; m[i+3] = src[i+3];
        if ((a[i+3] || b[i+3]) && Math.abs(t - th) < 0.045) {
          m[i] = sr; m[i+1] = sg; m[i+2] = sb; m[i+3] = 255;
        }
      }
      chair.c.putImageData(mix, 0, 0);
      mixT = t;
    }

    const env = envelope(t);
    // Flicker on the way in so the strip reads as an LED powering on rather than a linear
    // opacity ramp; after the dissolve, `pulse` (the slow breathe) takes over.
    const led = env * (t < 1 ? 0.9 + 0.1 * Math.sin(t * 37) : pulse);
    const col = 'hsl(' + (ledHue(t) + wob).toFixed(1) + ', 100%, 62%)';
    // Curtains shut over the first half of the dissolve, ahead of the chair finishing.
    curtainsAt(t / 0.5);
    const s = ctx;
    s.imageSmoothingEnabled = false;
    s.setTransform(1, 0, 0, 1, 0, 0);
    blit(s, back.cv);
    blit(s, curtains.cv);
    // Two dim passes: the room takes both, the chair only the second — it's lit by the bed.
    if (env) { s.fillStyle = 'rgba(4,6,22,' + 0.42 * env + ')'; s.fillRect(0, 0, W * S, H * S); }
    if (sway) drawSwivel(s, sway); else blit(s, chair.cv);
    if (env) { s.fillStyle = 'rgba(4,6,22,' + 0.2 * env + ')'; s.fillRect(0, 0, W * S, H * S); }
    if (led > 0) {
      // The pool also swells a few percent with the pulse, about the bed's base — a glow whose
      // *reach* moves reads as light far more than one that only changes opacity. Anchored at
      // the base, not the bed's center, so the pulse widens the pool instead of sliding it down
      // the floor.
      const grow = 1 + 0.1 * led;
      s.globalCompositeOperation = 'lighter';
      s.globalAlpha = 0.62 * led;
      s.setTransform(grow, 0, 0, grow, S * 58 * (1 - grow), S * 56 * (1 - grow));
      blit(s, tinted(halo, col));
      s.setTransform(1, 0, 0, 1, 0, 0);
      s.globalAlpha = 1;
      s.globalCompositeOperation = 'source-over';
      blit(s, front.cv);
      s.globalCompositeOperation = 'lighter';
      s.globalAlpha = 0.22 * led;
      blit(s, tinted(bedNeon, col));
      // Under-frame strip in segments on a travelling phase, so it reads as a real LED strip
      // chasing rather than one rectangle fading up and down together.
      s.fillStyle = col;
      for (let i = 0; i < 9; i++) {
        s.globalAlpha = Math.min(1, led * (0.62 + 0.5 * Math.sin(chase - i * 0.8)));
        s.fillRect((45 + i * 3) * S, 56 * S, 3 * S, S);
      }
      s.globalAlpha = 1;
      s.globalCompositeOperation = 'source-over';
    } else {
      blit(s, front.cv);
    }
  }

  // Rest-state fast path. Once the dissolve is over `t` is pinned at 2 and `envelope(t)` at 1, so
  // every layer below the glow is a fixed image: the room dimmed by both passes, the chair's
  // static base dimmed by the second, and — separately, because the sway shears it — the shell,
  // also dimmed by the second. Baking those at output resolution turns a rest frame from seven
  // full-canvas composites plus two full-canvas fills into four.
  //
  // The bed and its neon merge per frame into one 80x60 canvas rather than taking an upscale
  // each: `bedNeon` is built from `front` with 'source-in', so its alpha *is* front's alpha, which
  // means the only thing beneath the neon in a real frame is the bed itself — compositing the two
  // before the upscale is equivalent to compositing them after it.
  //
  // Measured (4x CPU throttle, 8s resting window, .notes/perf_probe.js): 847ms -> 636ms of
  // main-thread task time, 10.6% -> 7.9% of one thread. (A per-layer cost model predicted 5.9%;
  // it over-predicted because a 1:1 copy of a 480x360 opaque canvas is not free — measure, don't
  // extrapolate.)
  //
  // Not bit-identical to the per-layer path, and worth knowing which part isn't. Over a
  // deterministic 400-frame run (.notes/hero_frame.js, which fakes a virtual rAF clock so two
  // builds paint the same frame at the same timestamp): the baked base and pre-dimmed shell differ
  // on 3.6% of pixels by at most +-1/255, pure 8-bit rounding from compositing into an offscreen
  // canvas; the bed/neon merge differs on a further disjoint 1.7% by up to +-5/255, because the
  // neon's 0.22*led alpha quantizes once at 80x60 before the upscale instead of at output
  // resolution. Both are invisible against a glow that pulses far harder than that between
  // frames, but if exactness ever matters more than 1.4 percentage points of CPU, drawing `front`
  // and `bedNeon` as two output-res blits (the pre-merge code) brings the worst case back to +-1.
  const DIM1 = 0.42, DIM2 = 0.2;
  let rest = null;

  // A scratch canvas at the *output* resolution, so a baked layer costs a 1:1 copy per frame
  // instead of an upscale.
  function big() {
    const cv = document.createElement('canvas');
    cv.width = W * S; cv.height = H * S;
    const c2 = cv.getContext('2d');
    c2.imageSmoothingEnabled = false;
    return { cv, c: c2 };
  }

  // `src` upscaled and dimmed by `a`. 'source-atop' confines the wash to the sprite's own pixels,
  // which is what dimming the whole composite does wherever the sprite is opaque — true of these
  // sprites, every one of which is drawn with fillRect.
  function dimmedBig(src, a) {
    const l = big();
    l.c.drawImage(src.cv, 0, 0, W * S, H * S);
    l.c.globalCompositeOperation = 'source-atop';
    l.c.fillStyle = 'rgba(4,6,22,' + a + ')';
    l.c.fillRect(0, 0, W * S, H * S);
    l.c.globalCompositeOperation = 'source-over';
    return l;
  }

  // The region that actually changes between one rest frame and the next: the sheared shell, the
  // halo (which the pulse scales), the bed and its neon, and the LED strip. Wall, window,
  // curtains, floor and the chair's own base are identical in every rest frame, so recompositing
  // them is pure waste — measured, only 22-24% of pixels change per frame.
  //
  // Derived from the layers' own alpha bounds rather than hardcoded, so it tracks the art: move
  // the bed or widen the glow and the box follows instead of silently cropping it.
  //
  // Measured (4x CPU throttle, 8s resting window, .notes/perf_probe.js): 640ms -> 452ms of
  // main-thread task time, 8.0% -> 5.6% of one thread. Bit-identical to the unclipped path: 0
  // differing pixels of 172800, at five different points in the sway/pulse cycle
  // (.notes/hero_frame.js at 400/431/520/640/777 frames). That check is the one that matters here
  // — a dirty rect that is too small doesn't look broken, it leaves a stale smear only at certain
  // phases, so verify across phases rather than on one frame.
  function alphaBounds(l) {
    const d = l.c.getImageData(0, 0, W, H).data;
    let x0 = W, y0 = H, x1 = -1, y1 = -1;
    for (let y = 0, p = 3; y < H; y++) {
      for (let x = 0; x < W; x++, p += 4) {
        if (d[p]) {
          if (x < x0) x0 = x;
          if (x > x1) x1 = x;
          if (y < y0) y0 = y;
          if (y > y1) y1 = y;
        }
      }
    }
    return { x0: x0, y0: y0, x1: x1, y1: y1 };
  }

  // One rect, not several: two tighter boxes measured *slower* (440ms vs 425ms per 8s window) —
  // the clip setup costs more than the pixels it saves.
  function dirtyRect() {
    const shell = alphaBounds(gamerTop), glow = alphaBounds(halo), bed = alphaBounds(front);
    // Worst case of `drawSwivel`'s shear at |sway| = 1: the lean displaces a pixel by 0.115x its
    // distance from the pivot row, and `sx` pulls the silhouette in by up to 5%.
    const lean = 0.115 * Math.max(Math.abs(shell.y0 - PIVOT_Y), Math.abs(shell.y1 - PIVOT_Y));
    const narrow = 0.05 * Math.max(Math.abs(shell.x0 - PIVOT_X), Math.abs(shell.x1 - PIVOT_X));
    // Worst case of the halo's `grow` (1 + 0.1 * led, and the loudest `breathe` reaches ~1.35),
    // which scales about (58, 56).
    const g = 0.14;
    const grow = g * Math.max(
      Math.abs(glow.x0 - 58), Math.abs(glow.x1 - 58),
      Math.abs(glow.y0 - 56), Math.abs(glow.y1 - 56));
    // The LED strip is drawn from raw coordinates, not a sprite, so it's included literally.
    const x0 = Math.min(shell.x0 - lean - narrow, glow.x0 - grow, bed.x0, 45);
    const x1 = Math.max(shell.x1 + lean + narrow, glow.x1 + grow, bed.x1, 45 + 9 * 3);
    const y0 = Math.min(shell.y0, glow.y0 - grow, bed.y0, 56);
    const y1 = Math.max(shell.y1, glow.y1 + grow, bed.y1, 57);
    // A pixel of slack each way, then out to device pixels.
    const left = Math.max(0, Math.floor(x0) - 1), top = Math.max(0, Math.floor(y0) - 1);
    const right = Math.min(W, Math.ceil(x1) + 1), bottom = Math.min(H, Math.ceil(y1) + 1);
    return { x: left * S, y: top * S, w: (right - left) * S, h: (bottom - top) * S };
  }

  // Built lazily on the first rest frame, by which point the curtains are shut and the chair has
  // finished dissolving — so whatever is in those layers now is what rests there forever.
  function buildRest() {
    const base = big();
    base.c.drawImage(back.cv, 0, 0, W * S, H * S);
    base.c.drawImage(curtains.cv, 0, 0, W * S, H * S);
    base.c.fillStyle = 'rgba(4,6,22,' + DIM1 + ')';
    base.c.fillRect(0, 0, W * S, H * S);
    base.c.drawImage(gamerBase.cv, 0, 0, W * S, H * S);
    base.c.fillStyle = 'rgba(4,6,22,' + DIM2 + ')';
    base.c.fillRect(0, 0, W * S, H * S);
    rest = { base: base, top: dimmedBig(gamerTop, DIM2), bed: layer(), dirty: dirtyRect() };
  }

  // `full` paints the whole canvas; every frame after that is clipped to `rest.dirty` (see
  // `dirtyRect`). The first rest frame has to be unclipped — the frame underneath it came from the
  // dissolve, so pixels outside the box aren't yet in their resting state. From then on they never
  // change, which is exactly what makes clipping safe rather than merely cheap. The clip is set
  // under the identity transform and a canvas clip is independent of later `setTransform` calls,
  // so the shear and the halo's growth still land where they always did.
  let restFull = false;

  function paintRest(pulse, wob, sway, chase) {
    if (!rest) buildRest();
    const led = pulse;
    const col = 'hsl(' + (ledHue(2) + wob).toFixed(1) + ', 100%, 62%)';
    const s = ctx;
    s.imageSmoothingEnabled = false;
    s.setTransform(1, 0, 0, 1, 0, 0);
    const clipped = restFull;
    if (clipped) {
      s.save();
      s.beginPath();
      s.rect(rest.dirty.x, rest.dirty.y, rest.dirty.w, rest.dirty.h);
      s.clip();
    } else {
      restFull = true;
    }
    s.drawImage(rest.base.cv, 0, 0);
    const k = 0.115 * sway;
    const sx = 1 - 0.05 * Math.abs(sway);
    s.setTransform(sx, 0, k, 1, S * (PIVOT_X * (1 - sx) - k * PIVOT_Y), 0);
    s.drawImage(rest.top.cv, 0, 0);
    s.setTransform(1, 0, 0, 1, 0, 0);
    if (led > 0) {
      const grow = 1 + 0.1 * led;
      s.globalCompositeOperation = 'lighter';
      s.globalAlpha = 0.62 * led;
      s.setTransform(grow, 0, 0, grow, S * 58 * (1 - grow), S * 56 * (1 - grow));
      blit(s, tinted(halo, col));
      s.setTransform(1, 0, 0, 1, 0, 0);
      s.globalAlpha = 1;
      s.globalCompositeOperation = 'source-over';
      const b = rest.bed;
      b.c.globalCompositeOperation = 'source-over';
      b.c.clearRect(0, 0, W, H);
      b.c.drawImage(front.cv, 0, 0);
      b.c.globalCompositeOperation = 'lighter';
      b.c.globalAlpha = 0.22 * led;
      b.c.drawImage(tinted(bedNeon, col), 0, 0);
      b.c.globalAlpha = 1;
      b.c.globalCompositeOperation = 'source-over';
      blit(s, b.cv);
      s.globalCompositeOperation = 'lighter';
      s.fillStyle = col;
      for (let i = 0; i < 9; i++) {
        s.globalAlpha = Math.min(1, led * (0.62 + 0.5 * Math.sin(chase - i * 0.8)));
        s.fillRect((45 + i * 3) * S, 56 * S, 3 * S, S);
      }
      s.globalAlpha = 1;
      s.globalCompositeOperation = 'source-over';
    } else {
      blit(s, front.cv);
    }
    if (clipped) s.restore();
  }

  // Resting loop, sampled at ~15fps (REST_MS). Brightness is four sines at unrelated periods rather than
  // one, the fastest fast enough (~100ms) to read as flicker rather than breathing; hue only
  // wobbles a few degrees around the resting pink (the full RGB cycle belongs to the transition).
  // The chair's sway runs on its own two periods, so the two never lock into a shared beat.
  //
  // Throttled and suspended whenever the hero scrolls out of view; rAF already suspends it in a
  // hidden tab. A rest frame is drawImage-only (see `mixT`), far cheaper than the games' own
  // render loops, but still not free — hence the observer rather than an unconditional loop.
  // Resting paint interval. 64ms (~15fps) rather than the 32ms this used to run at: measured
  // 4x-CPU-throttled main-thread cost dropped from 19.3% of one thread to 10.6% for a loop whose
  // fastest term is a ~100ms flicker, so the sampling is still inside it. The transition itself is
  // unaffected — it stays on every rAF tick, since a dissolve is watched closely and only lasts
  // 1.6s, while the rest loop runs for as long as the page is open.
  const REST_MS = 64;

  const breathe = (now) =>
    0.66 + 0.2 * Math.sin(now / 940) + 0.12 * Math.sin(now / 430 + 1.7)
         + 0.06 * Math.sin(now / 210 + 0.5) + 0.05 * Math.sin(now / 97 + 2.4)
         // occasional bloom, so the strip surges every few seconds instead of only undulating
         + 0.22 * Math.pow(Math.max(0, Math.sin(now / 2600)), 8);
  const wobble = (now) => 9 * Math.sin(now / 1500) + 4 * Math.sin(now / 560 + 2.2);
  const swayAt = (now) => 0.74 * Math.sin(now / 1250) + 0.26 * Math.sin(now / 690 + 0.9);

  if (matchMedia('(prefers-reduced-motion: reduce)').matches) { paint(2, 1, 0, 0, 0); return; }

  paint(-1, 1, 0, 0, 0);
  const DELAY = 1500, DUR = 1600;
  let t0 = 0, live = true, last = 0, done = false;

  function frame(now) {
    if (!live) return;
    if (done) {
      if (now - last > REST_MS) {
        paintRest(breathe(now), wobble(now), swayAt(now), now / 190);
        last = now;
      }
      requestAnimationFrame(frame);
      return;
    }
    if (!t0) t0 = now;
    const e = now - t0 - DELAY;
    if (e >= 0) {
      const raw = Math.min(1, e / DUR);
      const eased = raw < 0.5 ? 2 * raw * raw : 1 - 2 * (1 - raw) * (1 - raw);
      // Overshoot the 0..1 range slightly so the glow band starts off-sprite and leaves it.
      paint(eased * 1.1 - 0.05, 1, 0, 0, 0);
      if (raw >= 1) { done = true; last = now; }
    }
    requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);

  if (window.IntersectionObserver) {
    new IntersectionObserver((entries) => {
      const on = entries[entries.length - 1].isIntersecting;
      if (on === live) return;
      live = on;
      if (on) requestAnimationFrame(frame);
    }, { threshold: 0 }).observe(canvas);
  }
})();
"#;

/// A real vertical reel, not 3 fixed cards whose text swaps in place: `.postcard-viewport`
/// clips a window onto `.postcard-track`, which JS populates with **3 concatenated copies**
/// of all 7 quotes (21 real `.postcard-slot` elements — the 3 SSR'd into the page are torn
/// down and rebuilt, so no-JS visitors still just see 3 static cards). `index` is the track
/// position (0..21) currently scrolled to the top of the viewport; moving means animating
/// `translateY(-index * step)` where `step` is one card's rendered height plus the track's
/// gap, measured from the real DOM once after building it (cards are a fixed CSS `height`,
/// so `step` itself never needs re-measuring on resize/reflow). Landing on the prev or next
/// quote is then just `index +/- 1` — the middle copy (indices 7..14) is "home"; drifting
/// into the first or third copy (a big drag flick, or enough autoplay ticks) triggers
/// `normalize()`, which jumps `index` by +/-7 with the transition disabled — invisible,
/// since the copy it jumps to shows byte-identical content. Autoplay ticks by 1 step every
/// 4s (one new quote revealed at a time, matching how a real physical reel would move);
/// arrows step by 1 too, for the same reason — everything moves in the same unit as the
/// drag.
///
/// How many cards are visible is not fixed at 3: `updateVisibleCount` sets
/// `.postcard-viewport`'s `height` (in units of `step`, so it's always a whole number of
/// cards, never a partial one peeking in) to match the hero scene's own rendered height on
/// the side-by-side desktop layout, and only falls back to a flat 3 once `.main` stacks
/// (mobile) — checked via `.main`'s own computed `flex-direction` rather than duplicating
/// the `@media (max-width: 720px)` breakpoint as a second magic number here. This is the
/// one thing that *does* need re-running on resize (the hero's rendered height changes
/// continuously with viewport width even before the mobile breakpoint), unlike `step`.
///
/// Dragging is Pointer Events, not separate mouse/touch handlers — one code path drags with
/// either input, `.postcard-track`'s `touch-action: none` hands the whole vertical gesture to
/// JS instead of letting the browser read it as a page scroll (needed since the drag axis and
/// the page's own scroll axis are the same). While held, `.dragging` kills the transition and
/// the track's `transform` tracks the pointer 1:1 (`-index * step + dy`) — a reel that only
/// ever jumps on release doesn't read as something you're physically pulling. On release,
/// `Math.round(-dy / step)` turns the raw pixel drag into a whole number of cards and lands
/// exactly on the nearest one — a small drag rounds to 0 steps and the transition alone
/// carries it back to where it started (a spring-back with no dedicated threshold constant:
/// "less than half a card" and "snaps back" are the same condition here), a big drag can
/// commit to more than one card at once, same as flicking a real reel harder.
///
/// Autoplay pauses on hover/focus/drag and never starts at all under reduced-motion (CSS
/// also drops `.postcard-track`'s transition in that case — dragging still works, it just
/// tracks and resettles with no eased motion) — only the automatic timer is motion-gated,
/// manual controls always work.
const POSTCARD_SCRIPT_TEMPLATE: &str = r#"
(function () {
  const QUOTES = [__QUOTES__];
  const TILTS = [__TILTS__];
  const wrap = document.querySelector('.postcards');
  const viewport = document.querySelector('.postcard-viewport');
  const track = document.querySelector('.postcard-track');
  const prevBtn = document.querySelector('.postcard-prev');
  const nextBtn = document.querySelector('.postcard-next');
  if (!wrap || !viewport || !track || QUOTES.length < 3) return;

  const reduced = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
  const COPIES = 3;
  const total = QUOTES.length * COPIES;

  track.innerHTML = '';
  const cards = [];
  for (let i = 0; i < total; i++) {
    const quoteIndex = i % QUOTES.length;
    const [quote, speaker] = QUOTES[quoteIndex];
    const card = document.createElement('div');
    card.className = 'postcard-slot';
    // Keyed by quote identity, not the raw track index: QUOTES.length and TILTS.length
    // are coprime, so indexing by `i` gave the same quote a different tilt each time a
    // different one of its 3 copies scrolled past — a visible "flip" every loop.
    card.style.setProperty('--r', TILTS[quoteIndex % TILTS.length] + 'deg');
    const bq = document.createElement('blockquote');
    bq.textContent = '"' + quote + '"';
    const cite = document.createElement('cite');
    cite.textContent = '— ' + speaker;
    card.appendChild(bq);
    card.appendChild(cite);
    track.appendChild(card);
    cards.push(card);
  }

  // getComputedStyle().height (not getBoundingClientRect()) on purpose — each card has a
  // small decorative `rotate()` tilt, and a rotated rect's axis-aligned bounding box is
  // taller than the box itself, which would throw the reel out of sync with
  // `.postcard-viewport`'s untransformed CSS height.
  const gap = parseFloat(getComputedStyle(track).rowGap || getComputedStyle(track).gap || '0');
  const cardHeight = parseFloat(getComputedStyle(cards[0]).height);
  const step = cardHeight + gap;

  let index = QUOTES.length; // top of the middle ("home") copy — matches the SSR'd 0,1,2
  let timer = null;
  let paused = false;

  // On the side-by-side desktop layout, show as many cards as the hero scene fits rather
  // than a fixed 3 — checked via `.main`'s own computed flex-direction (the same signal
  // the `@media (max-width: 720px)` rule flips) rather than duplicating that breakpoint
  // as a magic number here. Once `.main` stacks (mobile), the hero is no longer a height
  // budget to match, so it's back to a fixed 3.
  const mainEl = document.querySelector('.main');
  const heroEl = document.querySelector('.scene-card');

  function updateVisibleCount() {
    const stackedLayout = !mainEl || getComputedStyle(mainEl).flexDirection === 'column';
    let n = 3;
    if (!stackedLayout && heroEl) {
      const heroHeight = heroEl.getBoundingClientRect().height;
      n = Math.max(1, Math.floor((heroHeight + gap) / step));
    }
    viewport.style.height = (n * step - gap) + 'px';
  }

  updateVisibleCount();
  window.addEventListener('resize', updateVisibleCount);

  function apply(px, animate) {
    if (!animate) {
      track.style.transition = 'none';
      track.style.transform = 'translateY(' + px + 'px)';
      void track.offsetHeight;
      track.style.transition = '';
    } else {
      track.style.transform = 'translateY(' + px + 'px)';
    }
  }

  function normalize() {
    let wrapped = false;
    while (index >= QUOTES.length * 2) { index -= QUOTES.length; wrapped = true; }
    while (index < QUOTES.length) { index += QUOTES.length; wrapped = true; }
    return wrapped;
  }

  function settle(animate) {
    apply(-index * step, animate);
    const after = () => { if (normalize()) apply(-index * step, false); };
    if (animate && !reduced) setTimeout(after, 400); else after();
  }

  function moveBy(delta) {
    index += delta;
    settle(true);
  }

  function stop() { clearInterval(timer); timer = null; }
  function start() { if (!reduced && !paused) timer = setInterval(() => moveBy(1), 4000); }
  function restart() { stop(); start(); }

  prevBtn.addEventListener('click', () => { moveBy(-1); restart(); });
  nextBtn.addEventListener('click', () => { moveBy(1); restart(); });

  wrap.addEventListener('mouseenter', () => { paused = true; stop(); });
  wrap.addEventListener('mouseleave', () => { paused = false; start(); });
  wrap.addEventListener('focusin', () => { paused = true; stop(); });
  wrap.addEventListener('focusout', () => { paused = false; start(); });

  let dragging = false;
  let dragPointerId = null;
  let dragStartY = 0;
  let dragDy = 0;

  function endDrag() {
    dragging = false;
    dragPointerId = null;
    track.classList.remove('dragging');
  }

  track.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    dragging = true;
    dragPointerId = e.pointerId;
    dragStartY = e.clientY;
    dragDy = 0;
    track.setPointerCapture(e.pointerId);
    track.classList.add('dragging');
    stop();
  });

  track.addEventListener('pointermove', (e) => {
    if (!dragging || e.pointerId !== dragPointerId) return;
    dragDy = e.clientY - dragStartY;
    track.style.transform = 'translateY(' + (-index * step + dragDy) + 'px)';
  });

  track.addEventListener('pointerup', (e) => {
    if (!dragging || e.pointerId !== dragPointerId) return;
    endDrag();
    index += Math.round(-dragDy / step);
    settle(true);
    restart();
  });

  track.addEventListener('pointercancel', () => {
    if (!dragging) return;
    endDrag();
    settle(true);
    restart();
  });

  apply(-index * step, false);
  start();
})();
"#;

/// Escapes a Rust string into a single-quoted JS string literal (backslash, quote,
/// newline) for splicing into `POSTCARD_SCRIPT_TEMPLATE`'s `QUOTES` array — text only
/// ever lands via `textContent`, so HTML-escaping isn't needed here, just valid JS syntax.
fn js_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            _ => out.push(ch),
        }
    }
    out.push('\'');
    out
}

fn postcard_script(quotes: &[(&str, &str)], tilts: &[f64]) -> String {
    let quotes_js = quotes
        .iter()
        .map(|(q, s)| format!("[{}, {}]", js_str(q), js_str(s)))
        .collect::<Vec<_>>()
        .join(", ");
    let tilts_js = tilts
        .iter()
        .map(|t| t.to_string())
        .collect::<Vec<_>>()
        .join(", ");
    POSTCARD_SCRIPT_TEMPLATE
        .replace("__QUOTES__", &quotes_js)
        .replace("__TILTS__", &tilts_js)
}

const QUOTES: &[(&str, &str)] = &[
    (
        "Gaming is solved.",
        "a man in a hoodie who discovered Pong last Tuesday",
    ),
    (
        "I haven't pressed a button in weeks. I just describe my intended gameplay and the AI plays for me.",
        "a thought leader on the future of fun",
    ),
    (
        "Human players will be completely obsolete within 18 months. We'll only need play prompters.",
        "someone who has never finished a game in their life",
    ),
    (
        "Human input is now an unnecessary dependency.",
        "a corporate futurist with a gaming chair still wrapped in plastic",
    ),
    (
        "Beating games yourself is just stubbornness now. The AI has already seen the credits.",
        "posted from a hotel room at a gaming conference",
    ),
    (
        "The era of human gameplay is over. These are the last games played by hand.",
        "a VC who just funded an AI esports team",
    ),
    (
        "No one will need gamers in 6 months.",
        "a LinkedIn post with 40,000 reactions and zero comments",
    ),
    (
        "We've eliminated the gameplay bottleneck.",
        "a startup founder who delegates his coffee order to an assistant",
    ),
    (
        "There's a whole popup of controls behind the ? key. I've never opened it, but I appreciate that it's there.",
        "a man who fired his financial advisor for a chatbot",
    ),
    (
        "Touching the game is now a legacy workflow.",
        "a keynote speaker who never unpacked his controller",
    ),
    (
        "The player is now an optional layer.",
        "a management consultant between airport lounges",
    ),
    (
        "We're not watching someone play anymore. We're observing autonomous entertainment at scale.",
        "a founder who calls YouTube \"legacy media\"",
    ),
    (
        "The leaderboard is no longer a ranking. It's a preview of the companies that will acquire each other.",
        "a seed investor at a private gaming retreat",
    ),
    (
        "Skill is no longer a core competency.",
        "an esports analyst who has never played competitively",
    ),
    (
        "Once the AI learns what winning looks like, the rest of the game is mostly administrative.",
        "a consultant who has never read the rules",
    ),
    (
        "We've decoupled fun from participation.",
        "a venture partner who watches games at 3x speed",
    ),
    (
        "You are still playing. AI can fix that.",
        "an AI companion ad, glowing over a rain-soaked street",
    ),
    (
        "Is AI winning, son?",
        "a dad who left his wife for a chatbot",
    ),
    (
        "Congratulations on your promotion to spectator.",
        "an HR email nobody remembers approving",
    ),
    (
        "You watch. AI plays. Everyone wins. Mostly Big Tech.",
        "a shareholder with a last shred of consciousness",
    ),
    (
        "You didn't set the high score. Your subscription did.",
        "the terms of service, buried in section 12",
    ),
    (
        "It's not giving up. It's giving up the controller.",
        "a marriage counselor, billing by the session",
    ),
];

/// The site's colour-theme control: a three-position icon slider (auto / dark / light) pinned
/// to the homepage's top-right corner. Only the homepage carries it — the wall and game pages
/// just apply whatever it saved, via `xtask::theme_init_script` (the cookie is `path=/`).
///
/// Native radio inputs underneath, so arrow-key navigation, focus and the checked state come
/// from the browser; the sliding thumb follows `:has(:checked)`, no JS layout. The only JS is
/// syncing the initial checked radio from `data-theme-pref` and saving a change.
const THEME_SWITCH_CSS: &str = r#"
.theme-switch {
  position: absolute;
  top: 0.9rem;
  right: 0.9rem;
  z-index: 5;
  display: grid;
  grid-template-columns: repeat(3, 2rem);
  height: 2rem;
  padding: 0;
  border: 1px solid var(--border);
  border-radius: 999px;
  color: var(--text-dim);
}
.theme-switch input { position: absolute; opacity: 0; pointer-events: none; }
.theme-switch label {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  border-radius: 999px;
  transition: color 0.2s;
}
.theme-switch label:hover { color: var(--accent); }
.theme-switch svg { width: 15px; height: 15px; }
.theme-switch input:checked + label { color: var(--accent); }
.theme-switch input:focus-visible + label { outline: 2px solid var(--accent); outline-offset: -2px; }
.theme-switch .thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: calc(2rem - 4px);
  height: calc(2rem - 6px);
  border-radius: 999px;
  background: var(--border);
  transition: transform 0.25s cubic-bezier(0.22, 1, 0.36, 1);
}
.theme-switch:has(#theme-dark:checked) .thumb { transform: translateX(2rem); }
.theme-switch:has(#theme-light:checked) .thumb { transform: translateX(4rem); }
.theme-switch .sr-only {
  position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap;
}
@media (prefers-reduced-motion: reduce) { .theme-switch .thumb { transition: none; } }
"#;

const THEME_SWITCH_SCRIPT: &str = "
(function() {
  var pref = document.documentElement.getAttribute('data-theme-pref') || 'auto';
  var inputs = document.querySelectorAll('.theme-switch input');
  inputs.forEach(function(input) {
    input.checked = input.value === pref;
    input.addEventListener('change', function() {
      if (input.checked && window.__hcgSetTheme) window.__hcgSetTheme(input.value);
    });
  });
})();
";

fn theme_switch() -> Markup {
    // (value, accessible name, SVG body) — 24x24 viewBox, stroked in currentColor.
    let options = [
        (
            "auto",
            "Auto theme (follow system)",
            r#"<circle cx="12" cy="12" r="8.5"/><path d="M12 3.5a8.5 8.5 0 0 1 0 17z" fill="currentColor" stroke="none"/>"#,
        ),
        (
            "dark",
            "Dark theme",
            r#"<path d="M20 14.5A8.5 8.5 0 0 1 9.5 4a8.5 8.5 0 1 0 10.5 10.5z"/>"#,
        ),
        (
            "light",
            "Light theme",
            r#"<circle cx="12" cy="12" r="4"/><path d="M12 2.5v2.5M12 19v2.5M2.5 12H5M19 12h2.5M5.3 5.3l1.8 1.8M16.9 16.9l1.8 1.8M5.3 18.7l1.8-1.8M16.9 7.1l1.8-1.8"/>"#,
        ),
    ];
    html! {
        style { (PreEscaped(THEME_SWITCH_CSS)) }
        fieldset class="theme-switch" role="radiogroup" aria-label="Color theme" {
            span class="thumb" aria-hidden="true" {}
            @for (value, label, svg) in options {
                input type="radio" name="theme" id=(format!("theme-{value}")) value=(value)
                    checked[value == "auto"];
                label for=(format!("theme-{value}")) title=(label) {
                    svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8"
                        stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" {
                        (PreEscaped(svg))
                    }
                    span class="sr-only" { (label) }
                }
            }
        }
        script { (PreEscaped(minify_js(THEME_SWITCH_SCRIPT))) }
    }
}

fn main() {
    let dist = Path::new("dist");
    let base_url = base_url();

    // A dist/ directory only counts as a game if it has a `games/<name>` source crate
    // *and* a built page — that excludes static redirect stubs (e.g. static/2048/index.html
    // -> game2048/), which live alongside real games in dist/ but aren't ones. It used to
    // key off "the directory contains a .wasm", which stopped distinguishing anything once
    // every game page started loading the one shared `dist/hcg.wasm` instead of its own
    // per-game binary (see `xtask::BUNDLE_WASM`) — that check would have found zero games.
    let mut games: Vec<String> = all_games()
        .into_iter()
        .filter(|name| dist.join(name).join("index.html").is_file())
        .collect();
    games.sort_by_key(|name| title(name));

    let og = social_image(&base_url, dist, Some("og-image.png"));

    // Small fixed alternating tilt per postcard so the stack doesn't look perfectly
    // squared-off — deterministic (not `Math.random()`) so there's no first-paint jump.
    let tilts = [-1.2, 1.3, -0.7];

    let page = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                // Brand first, tagline second: a brand search has to land on the brand,
                // but "Hotel Chair Games" alone tells a SERP reader (and Google) nothing
                // about what the site is, and nobody searches the brand yet. "Watch them
                // play" is also the site's own domain (watchthem.github.io), so the title,
                // the URL and the premise all say the same thing. Game pages deliberately
                // keep their terse "<Game> — Hotel Chair Games" form; their keyword weight
                // comes from `game_page_info`'s on-page text instead.
                title { "Hotel Chair Games — Watch Them Play" }
                (favicon_links(&base_url, dist))
                meta name="description" content=(SITE_DESCRIPTION);
                link rel="canonical" href=(base_url);
                meta property="og:type" content="website";
                meta property="og:site_name" content="Hotel Chair Games";
                meta property="og:locale" content="en_US";
                meta property="og:title" content="Hotel Chair Games — Watch Them Play";
                meta property="og:description" content=(SITE_DESCRIPTION);
                meta property="og:url" content=(base_url);
                meta property="og:image" content=(og.url);
                // Describes `static/hotel-scene.svg` (rasterized to dist/og-image.png),
                // which is the hotel room itself — not a grid of games.
                meta property="og:image:alt" content="A dim hotel room with an armchair pulled up to a glowing screen";
                meta name="twitter:card" content=(og.twitter_card);
                meta name="twitter:image" content=(og.url);
                (homepage_json_ld(&base_url, dist, SITE_DESCRIPTION))
                (gtag_head())
                (pwa_head("#171310"))
                (theme_init_script())
                link rel="preconnect" href="https://fonts.googleapis.com";
                link rel="preconnect" href="https://fonts.gstatic.com" crossorigin;
                // Loaded async (classic loadCSS pattern): a plain `<link rel=stylesheet>`
                // here blocks first paint on an extra cross-origin round trip (measured
                // ~850ms in Lighthouse). `media="print"` makes the browser fetch it without
                // treating it as render-blocking for the screen; the `onload` swap applies
                // it the moment it arrives. `display=swap` (already in FONTS_HREF) then
                // handles the brief system-font-to-webfont swap once it does.
                link rel="preload" href=(FONTS_HREF) as="style";
                link rel="stylesheet" href=(FONTS_HREF) media="print" onload="this.media='all'";
                noscript {
                    link rel="stylesheet" href=(FONTS_HREF);
                }
                style { (PreEscaped(STYLE)) }
            }
            body {
                (theme_switch())
                header class="fade-up" {
                    h1 { "Hotel Chair Games" }
                    p class="kicker" { "The bed is taken. Sit anyway." }
                    a class="wall-link" href="wall/" { "→ leave the whole wall running" }
                }
                main class="main" {
                    div class="scene-card fade-up" {
                        canvas id="hotel" width="480" height="360" {}
                    }
                    div class="postcards fade-up" role="region" aria-roledescription="carousel"
                        aria-label="Overheard AI-hype quotes" {
                        button type="button" class="postcard-arrow postcard-prev" aria-label="Show previous quotes" {
                            "‹"
                        }
                        div class="postcard-viewport" {
                            div class="postcard-track" {
                                @for i in 0..3 {
                                    @let (quote, speaker) = QUOTES[i];
                                    div class="postcard-slot" style=(format!("--r: {}deg", tilts[i % tilts.len()])) {
                                        blockquote {
                                            "\"" (quote) "\""
                                        }
                                        cite { "— " (speaker) }
                                    }
                                }
                            }
                        }
                        button type="button" class="postcard-arrow postcard-next" aria-label="Show next quotes" {
                            "›"
                        }
                    }
                }
                section class="games fade-up" {
                    h2 { "games" }
                    p class="pitch" { (SITE_PITCH) }
                    div class="game-grid" {
                        @for game in &games {
                            a class="game-card" href=(format!("{game}/")) {
                                // 224px, though a card actually lays out at 226 — a browser
                                // picks the first candidate at least as wide as the
                                // density-adjusted need, so declaring the true 226 would put
                                // a 2x display at 452 and skip straight past the 450w tier to
                                // the full-size preview. Under-declaring by 2px keeps a 900px
                                // game on its 225w tier at 1x and its 450w tier at 2x; the ~1%
                                // shortfall against the real box is not visible.
                                @let srcset = preview_srcset(dist, game, "");
                                img src=(format!("{game}/preview.png"))
                                    srcset=[srcset.clone()]
                                    sizes=[srcset.is_some().then_some("(max-width: 1010px) 48vw, 224px")]
                                    alt=(title(game)) loading="lazy";
                                div class="card-body" {
                                    h3 { (title(game)) }
                                    p { (description(game)) }
                                }
                            }
                        }
                    }
                }
                script { (PreEscaped(minify_js(HOTEL_SCENE_SCRIPT))) }
                script { (PreEscaped(minify_js(&postcard_script(QUOTES, &tilts)))) }
                (sw_register_bridge("./sw.js"))
            }
        }
    };

    std::fs::write(dist.join("index.html"), page.into_string()).unwrap();
    std::fs::write(
        dist.join("manifest.webmanifest"),
        manifest_json(dist, "Hotel Chair Games", SITE_DESCRIPTION, "#171310", ""),
    )
    .unwrap();

    // (directory name, display title) pairs — shared by the wall's tiles and its `ItemList`
    // JSON-LD so the visible grid and the machine-readable list can't drift apart.
    let wall_items: Vec<(String, String)> = games
        .iter()
        .map(|game| (game.clone(), title(game)))
        .collect();

    let wall_page = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { "Ambient Wall — Hotel Chair Games" }
                (favicon_links(&base_url, dist))
                meta name="description" content="Every self-playing game on this site at once, one AI per screen — leave it running in the background, nothing to do here either.";
                link rel="canonical" href=(format!("{base_url}wall/"));
                // The wall had no OG tags at all, which mattered more here than anywhere
                // else: it's the most shareable page on the site, so every link to it
                // rendered as a bare URL with no title, text or image.
                meta property="og:type" content="website";
                meta property="og:site_name" content="Hotel Chair Games";
                meta property="og:locale" content="en_US";
                meta property="og:title" content="Ambient Wall — Hotel Chair Games";
                meta property="og:description" content=(format!("{} self-playing games running at once, one AI per tile. Leave it on in the background.", games.len()));
                meta property="og:url" content=(format!("{base_url}wall/"));
                meta property="og:image" content=(og.url);
                meta property="og:image:alt" content="A dim hotel room with an armchair pulled up to a glowing screen";
                meta name="twitter:card" content=(og.twitter_card);
                meta name="twitter:image" content=(og.url);
                (wall_json_ld(&base_url, &wall_items))
                (gtag_head())
                (theme_init_script())
                style { (PreEscaped(WALL_STYLE)) }
            }
            body {
                header {
                    a href="../" { "← Hotel Chair Games" }
                    h1 { "Ambient Wall" }
                    p { (format!("{} AIs. Zero players. Maximum efficiency.", games.len())) }
                }
                main class="wall-grid" {
                    @for (game, game_title) in &wall_items {
                        div class="wall-tile" title=(game_title) data-game=(game) {
                            @let srcset = preview_srcset(dist, game, "../");
                            img class="wall-preview" src=(format!("../{game}/preview.png"))
                                srcset=[srcset.clone()]
                                sizes=[srcset.is_some().then_some("(max-width: 620px) 92vw, (max-width: 950px) 46vw, (max-width: 1300px) 24vw, 19vw")]
                                alt=(format!("{game_title} being played by an AI")) loading="lazy";
                            a class="wall-label" href=(format!("../{game}/")) { (game_title) " ↗" }
                        }
                    }
                }
                (wall_analytics_bridge())
                (wall_live_bridge())
                (sw_register_bridge("../sw.js"))
            }
        }
    };
    std::fs::create_dir_all(dist.join("wall")).unwrap();
    std::fs::write(
        dist.join("wall").join("index.html"),
        wall_page.into_string(),
    )
    .unwrap();

    let today = time::OffsetDateTime::now_utc().date();
    let mut urls = vec![base_url.clone(), format!("{base_url}wall/")];
    urls.extend(games.iter().map(|g| format!("{base_url}{g}/")));
    let mut sitemap = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for url in &urls {
        sitemap.push_str(&format!(
            "  <url><loc>{url}</loc><lastmod>{today}</lastmod></url>\n"
        ));
    }
    sitemap.push_str("</urlset>\n");
    std::fs::write(dist.join("sitemap.xml"), sitemap).unwrap();

    println!(
        "wrote dist/index.html and dist/sitemap.xml ({} game(s))",
        games.len()
    );
}
