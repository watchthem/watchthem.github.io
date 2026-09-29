use maud::{Markup, PreEscaped, html};
use std::path::Path;

/// Minifies an embedded JS `<script>` body via the `minifier` crate — comments and
/// whitespace stripped through a real tokenizer, not a hand-rolled regex, so it's safe
/// around the `//` inside `gtag_head`'s `https://...` string literal, comments like
/// `orientation_hint`'s pointer-detection rationale, etc. Source stays
/// hand-formatted/commented in this file for readability; every `script` block's JS
/// content should be wrapped in this before `PreEscaped`.
///
/// Deliberately *not* `minify-js` (a real AST-based minifier, tried first): 0.6.0 panics
/// building the page for every single game, from two separate internal bugs — `Global`
/// mode's `Option::unwrap()` on `None` on any top-level `if` statement at all (reproduced
/// down to `minify(&session, TopLevelMode::Global, b"if (x) { y(); }", ...)`), and
/// `Module` mode's `assertion failed: cons_expr.returns && alt_expr.returns` on an
/// `if`/`else` where a branch doesn't end in `return`. `minifier` only strips
/// comments/whitespace — no control-flow rewriting, no identifier mangling — so it can't
/// hit either class of bug, and (unlike `Module` mode's mangling) never risks breaking a
/// bare top-level name one `<script>` block relies on another to declare.
pub fn minify_js(src: &str) -> String {
    minifier::js::minify(src)
        .unwrap_or_else(|e| panic!("failed to minify JS: {e}\n---\n{src}"))
        .to_string()
}

/// `GITHUB_REPOSITORY` ("owner/repo") is auto-set by GitHub Actions; matches the default
/// Pages URL when no custom domain (CNAME) is set. `BASE_URL` always overrides.
///
/// Two Pages layouts, and the difference is load-bearing for `robots.txt`: a repo named
/// exactly `<owner>.github.io` is an *org/user page*, served at the root of that
/// subdomain, while any other name is a *project page* served under `/<repo>/`. Only the
/// root form can serve a `robots.txt` that crawlers actually read — they fetch it from
/// the origin root and nowhere else, so a project page's `/<repo>/robots.txt` is dead
/// weight (see `.notes/seo_ideas.md`).
pub fn base_url() -> String {
    if let Ok(url) = std::env::var("BASE_URL") {
        return url;
    }
    if let Ok(repo) = std::env::var("GITHUB_REPOSITORY")
        && let Some((owner, name)) = repo.split_once('/')
    {
        if name.eq_ignore_ascii_case(&format!("{owner}.github.io")) {
            return format!("https://{owner}.github.io/");
        }
        return format!("https://{owner}.github.io/{name}/");
    }
    "http://localhost:8080/".to_owned() // matches `mise run serve`
}

/// A game's on-disk directory name ("game2048", "arrow-blocks") to a display title
/// ("2048", "Arrow Blocks"), matching the old Python `removeprefix("game").title()`.
pub fn title(name: &str) -> String {
    name.strip_prefix("game")
        .unwrap_or(name)
        .split('-')
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Each game's fixed logical canvas resolution (its `window_width`/`window_height` in
/// `Conf`). Games draw at absolute pixel coordinates rather than scaling to
/// `screen_width()`/`screen_height()`, so the canvas must stay at exactly this size —
/// see `native_size_style`.
pub fn native_size(name: &str) -> (u32, u32) {
    match name {
        "game2048" => (500, 610),
        // Tetris's 10x20 board is inherently tall and narrow — a 900px-wide canvas left a
        // small next-piece panel stranded in a wide leftover gap. Narrower canvas sized to
        // its own content instead of the shared default.
        "tetris" => (600, 720),
        // Bubble Shooter's hex-packed board is inherently tall and narrow, same
        // reasoning as Tetris's own override just above.
        "bubble-shooter" => (600, 720),
        _ => (900, 720),
    }
}

/// Upper bound on the viewport-fit scale factor (see `native_size_style`). Without a
/// cap, a portrait-ish canvas like game2048's (500x610) gets magnified more than a
/// landscape one on a widescreen monitor, since the fit is height-limited there — and
/// because portrait height (610) is close to a landscape game's already-capped on-screen
/// height, *any* single cap that isn't quite low still leaves it filling ~85-90% of the
/// viewport height on common desktop resolutions (verified: 1.5 barely changed anything
/// at 1600x1000 or 1920x1080). game2048 gets its own tighter 1.0 cap — never upscaled
/// past its native resolution at all, so it's always pixel-perfect regardless of
/// `high_dpi`/CSS-transform interaction, and reads as a modest fixed-size window rather
/// than something that grows to dominate the screen. The 900x720 games keep the looser
/// 1.5, which matched their existing on-screen size on a typical 1080p display, since
/// there's no report of them looking oversized. Tetris's 600x720 canvas is portrait-ish
/// for the same reason game2048's is — same 1.0 treatment.
fn max_fit_scale(name: &str) -> f64 {
    match name {
        "game2048" | "tetris" | "bubble-shooter" => 1.0,
        _ => 1.5,
    }
}

/// Games that lay themselves out from `screen_width()`/`screen_height()` rather than fixed
/// coordinates (labyrinth's raycaster widens its view, its HUD and minimap anchor to the
/// edges), so in browser fullscreen `native_size_style` can grow the canvas to the
/// screen's aspect instead of letterboxing it.
fn fills_fullscreen(name: &str) -> bool {
    name == "labyrinth"
}

/// CSS + JS that pins the canvas to its native design resolution (so games drawing at
/// absolute pixel coordinates render correctly) and scales it uniformly to fit the
/// viewport via `transform: scale`, letterboxed and centered. A CSS transform doesn't
/// change `clientWidth`/`clientHeight`, so `mq_js_bundle.js`'s resize handling (which
/// syncs the canvas's backing resolution to its CSS box) never sees a mismatch.
///
/// `?stream=1` (see `mode_class_script`) swaps the opaque black `html`/`body`
/// background for transparent instead — for dropping the page into OBS/Twitch as a
/// browser-source layer over other scene content. The letterboxed area around the
/// (still fixed-size, never stretched — see CLAUDE.md's "Canvas sizing is load-bearing")
/// canvas just becomes see-through padding rather than black bars.
///
/// `.stage` (the canvas's wrapper) is exactly one viewport tall with `overflow: hidden`,
/// which is load-bearing in two ways now that the page below it scrolls (see
/// `game_page_info`). First, a `transform: scale()` doesn't shrink the *layout* box: a
/// 720px-tall canvas scaled to 0.7 still occupies 720px of layout, so without clipping it
/// would add a few hundred px of dead scroll region between the visually-centered canvas
/// and the text below. Second, it keeps "one screen of game, then content" exact — the
/// game still owns the whole first screen the way it did when `body` itself was the
/// centering flex container. `100dvh` (with a `100vh` fallback for older browsers) so a
/// mobile address bar appearing/collapsing doesn't leave the stage taller than the visible
/// viewport.
pub fn native_size_style(name: &str) -> Markup {
    let (w, h) = native_size(name);
    let max_scale = max_fit_scale(name);
    let fill = fills_fullscreen(name);
    html! {
        style {
            (PreEscaped(format!(
                "* {{ margin: 0; padding: 0; box-sizing: border-box; }}\n\
                 html {{ background: var(--page-bg); overflow-x: hidden; }}\n\
                 body {{ background: var(--page-bg); }}\n\
                 html.stream-mode, html.stream-mode body, html.stream-mode .stage {{ background: transparent; }}\n\
                 .stage {{ position: relative; height: 100vh; height: 100dvh; overflow: hidden; background: #000; \
                 display: flex; align-items: center; justify-content: center; }}\n\
                 main {{ display: grid; }}\n\
                 canvas, .loading {{ grid-area: 1 / 1; width: {w}px; height: {h}px; transform-origin: center; }}\n\
                 canvas {{ display: block; outline: none; visibility: hidden; \
                 animation: reveal-canvas 0s 250ms forwards; }}\n\
                 @keyframes reveal-canvas {{ to {{ visibility: visible; }} }}\n\
                 .loading {{ display: flex; align-items: center; justify-content: center; text-align: center; \
                 padding: 0 2rem; color: rgba(255, 255, 255, 0.35); font: italic 15px system-ui, sans-serif; \
                 pointer-events: none; }}\n\
                 html.hcg-bare, html.hcg-bare body {{ height: 100%; overflow: hidden; }}\n\
                 html.hcg-bare .stage {{ height: 100%; }}\n\
                 {PAGE_INFO_CSS}\n\
                 {POPUP_CSS}"
            )))
        }
        (mode_class_script())
        script {
            (PreEscaped(minify_js(&format!(
                "window.fitCanvas = function() {{\n\
                 \x20 const vw = window.innerWidth, vh = window.innerHeight;\n\
                 \x20 const k = Math.min(vw / {w}, vh / {h}, {max_scale});\n\
                 \x20 document.querySelectorAll('canvas, .loading').forEach(function(el) {{\n\
                 \x20   el.style.transform = `scale(${{k}})`;\n\
                 \x20 }});\n\
                 \x20 // Layout-aware games fill the screen in fullscreen: the canvas box grows\n\
                 \x20 // to the viewport's aspect at the same scale, so the HUD keeps its size.\n\
                 \x20 if ({fill}) {{\n\
                 \x20   const c = document.getElementById('glcanvas');\n\
                 \x20   const full = !!(document.fullscreenElement || document.webkitFullscreenElement);\n\
                 \x20   const cw = full ? Math.round(vw / k) : {w}, ch = full ? Math.round(vh / k) : {h};\n\
                 \x20   if (c.clientWidth !== cw || c.clientHeight !== ch) {{\n\
                 \x20     c.style.width = cw + 'px';\n\
                 \x20     c.style.height = ch + 'px';\n\
                 \x20     // mq_js_bundle's own resize handler may already have run for this\n\
                 \x20     // event with the old box; let it sync the backing size again.\n\
                 \x20     if (typeof window.onresize === 'function') window.onresize();\n\
                 \x20   }}\n\
                 \x20 }}\n\
                 }};\n\
                 window.addEventListener('resize', window.fitCanvas);\n\
                 document.addEventListener('DOMContentLoaded', window.fitCanvas);"
            ))))
        }
    }
}

/// Two classes on `<html>`, both set before first paint, plus `window.__hcgHide` — `true`
/// under either query param that asks a per-game page to hide chrome meant for a human
/// visitor (`?embed=1`, the ambient wall's tiles, too small for a 48px popup button to make
/// sense on; or `?stream=1`, an OBS/Twitch browser-source layer, where the same button
/// would show up on stream for no reason) — every later script that needs that check
/// (`hotkey_popup`, `orientation_hint`, `scroll_cue`, `session_signals_bridge`,
/// `daily_challenge_button`) reads this instead of recomputing it, since `native_size_style`
/// (and this script with it) always runs before any of them:
///
/// - `stream-mode` under `?stream=1`, for `native_size_style`'s transparent-background
///   rule to key off.
/// - `hcg-bare` under `window.__hcgHide`, which restores the old non-scrolling
///   `height: 100%; overflow: hidden` page and hides `game_page_info`'s below-the-fold
///   content entirely. An ambient-wall tile is an iframe a few hundred px tall — letting it
///   scroll to a text section would be actively wrong there (and on an OBS browser source
///   there's nobody to scroll it), so both those modes keep exactly the single-screen
///   canvas page they had before that section existed.
///
/// A synchronous script (not deferred to `DOMContentLoaded`) so the classes land before
/// first paint — `document.documentElement` already exists as soon as the parser reaches
/// the `<html>` start tag, well before `<body>`/the canvas/the WASM fetch. Deferring
/// `hcg-bare` in particular would let a wall tile paint one scrollable frame first.
///
/// `window.__hcgHide` is computed once here and read as a plain variable everywhere else,
/// rather than each site re-running its own `(function() {{...}}())` IIFE inline — one
/// `URLSearchParams` parse instead of six, and one place to get the `?embed=1`/`?stream=1`
/// logic right instead of six copies of it.
fn mode_class_script() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "var hcgQs = new URLSearchParams(location.search);\n\
                 if (hcgQs.get('stream') === '1') {\n\
                 \x20 document.documentElement.classList.add('stream-mode');\n\
                 }\n\
                 window.__hcgHide = hcgQs.get('embed') === '1' || hcgQs.get('stream') === '1';\n\
                 if (window.__hcgHide) {\n\
                 \x20 document.documentElement.classList.add('hcg-bare');\n\
                 }"
            )))
        }
    }
}

/// Site-wide colour theme: `auto` (follow `prefers-color-scheme`, the default), `dark` or
/// `light`, chosen with the homepage's switch (`generate_index`) and remembered in the `hcg_theme` cookie (`path=/`,
/// so one choice covers the homepage, the wall and every game; `auto` deletes it).
///
/// Resolves the preference to a concrete `data-theme="light"|"dark"` on `<html>`, so each
/// page's CSS needs only one `:root[data-theme="light"]` override block rather than a
/// second copy under a media query. Synchronous in `<head>` so the right theme is there
/// before first paint; without JS the attribute never appears and pages stay dark, as
/// before. Under `auto` it also follows a live OS theme change. `window.__hcgSetTheme` is
/// the cross-script entry point that switch calls (see CLAUDE.md on minified blocks).
pub fn theme_init_script() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 var m = document.cookie.match(/(?:^|; )hcg_theme=(light|dark)(?:;|$)/);\n\
                 \x20 var pref = m ? m[1] : 'auto';\n\
                 \x20 var mq = window.matchMedia('(prefers-color-scheme: light)');\n\
                 \x20 var root = document.documentElement;\n\
                 \x20 function apply() {\n\
                 \x20   root.setAttribute('data-theme', pref === 'auto' ? (mq.matches ? 'light' : 'dark') : pref);\n\
                 \x20   root.setAttribute('data-theme-pref', pref);\n\
                 \x20 }\n\
                 \x20 apply();\n\
                 \x20 var onChange = function() { if (pref === 'auto') apply(); };\n\
                 \x20 if (mq.addEventListener) mq.addEventListener('change', onChange);\n\
                 \x20 else if (mq.addListener) mq.addListener(onChange);\n\
                 \x20 window.__hcgSetTheme = function(p) {\n\
                 \x20   pref = p;\n\
                 \x20   document.cookie = 'hcg_theme=' + p + '; path=/; SameSite=Lax; max-age=' +\n\
                 \x20     (p === 'auto' ? 0 : 31536000);\n\
                 \x20   apply();\n\
                 \x20 };\n\
                 })();"
            )))
        }
    }
}

/// Sarcastic one-liner shown behind the canvas while the WASM module fetches/inits.
/// Same "watch, don't judge" tone as the homepage quotes. Sits in the same CSS grid
/// cell as the canvas (see `native_size_style`) so once the game starts clearing the
/// canvas each frame, it's painted over automatically — no JS needed to hide it. The
/// canvas is also `visibility: hidden` for its first 250ms (`native_size_style`'s
/// `reveal-canvas` animation-delay) so a fast-loading/cached WASM module can't paint
/// over the line before it's had a chance to be read. This also gives the page a real
/// LCP-eligible text node: `<canvas>` itself isn't a valid Largest Contentful Paint
/// candidate per spec, so a canvas-only body reports NO_LCP.
const LOADING_LINES: &[&str] = &[
    "Make yourself comfortable. We'll start soon.",
    "The AI is warming up. You are not required to do anything.",
    "Loading. Try not to backseat drive.",
    "Spinning up the AI. It already knows how this ends.",
    "Nothing to do here. That's the whole point.",
    "The AI's already won. Now watch the AI beat the game.",
    "The AI's just better at this. Relax and watch.",
    "Sit down. The AI's got this handled.",
    "Nice game you have here. Sure you're okay watching the AI play?",
    "Your controller's decorative today.",
    "This one's on the AI. You just enjoy the show.",
    "The AI will take over from here.",
    "Hands off. Your help won't be needed.",
    "This game's just better when the AI plays it.",
];

pub fn loading_screen() -> Markup {
    // Build-time page generation, native only: wall-clock is fine here.
    #[allow(clippy::disallowed_methods)]
    let idx = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0)
        % LOADING_LINES.len();
    html! {
        div class="loading" { (LOADING_LINES[idx]) }
    }
}

/// Hides `loading_screen`'s line once the wasm is running. Covering it with the canvas
/// isn't enough: a game that draws translucent shapes (fog, glows) lowers the canvas's own
/// alpha there, since the default blend applies to the alpha channel too, and the line
/// shows through the running game. The page is black under it anyway, so nothing else
/// changes. Waits 250ms after `on_init` (which runs just before the wasm's `main`), so the
/// line stays up until the canvas's `reveal-canvas` delay has passed and the game has
/// drawn its first frames. Must run after `mq_js_bundle.js` but before `load(...)`.
pub fn loading_screen_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 on_init: function() {\n\
                 \x20   setTimeout(function() {\n\
                 \x20     var el = document.querySelector('.loading');\n\
                 \x20     if (el) el.style.visibility = 'hidden';\n\
                 \x20   }, 250);\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_loading_screen\"\n\
                 });"
            )))
        }
    }
}

const POPUP_CSS: &str = "\
#hotkeys-btn { display: block; position: fixed; bottom: 14px; right: 14px; z-index: 10; \
width: 48px; height: 48px; border-radius: 50%; border: none; \
background: rgba(255,255,255,0.15); color: #fff; font: 20px system-ui, sans-serif; \
line-height: 48px; text-align: center; padding: 0; cursor: pointer; }\n\
#hotkeys-btn:hover { background: rgba(255,255,255,0.28); }\n\
#hotkeys { display: none; position: fixed; inset: 0; z-index: 11; \
background: rgba(0,0,0,0.75); align-items: center; justify-content: center; \
font-family: system-ui, sans-serif; }\n\
#hotkeys.open { display: flex; }\n\
#hotkeys .panel { position: relative; background: #1a1a1f; color: #eee; \
border-radius: 8px; padding: 20px 28px; min-width: 220px; }\n\
#hotkeys h2 { font-size: 16px; margin-bottom: 12px; }\n\
#hotkeys dl { display: grid; grid-template-columns: auto 1fr; gap: 4px 16px; \
font-size: 14px; margin: 0; }\n\
#hotkeys dt { font-family: monospace; color: #8cf; }\n\
#hotkeys dd { margin: 0; color: #ccc; }\n\
#hotkeys .stream-hint { margin: 14px 0 0; padding-top: 12px; \
border-top: 1px solid rgba(255,255,255,0.12); font-size: 12px; color: #999; \
line-height: 1.5; }\n\
#hotkeys .stream-hint a { color: #8cf; }\n\
#hotkeys-close { position: absolute; top: 8px; right: 8px; width: 32px; height: 32px; \
border-radius: 50%; border: none; background: rgba(255,255,255,0.1); color: #eee; \
font-size: 18px; line-height: 32px; text-align: center; padding: 0; cursor: pointer; }\n\
#hotkeys-close:hover { background: rgba(255,255,255,0.2); }";

/// The `?`-toggled, Esc-closed hotkey reference overlay, plus two touch-reachable
/// controls for it — an always-visible on-canvas `#hotkeys-btn` that opens it, and an
/// `#hotkeys-close` `×` in the panel's corner that closes it, since neither `?` nor Esc
/// is reachable without a physical keyboard. Both 32-48px (min. touch-target size) so
/// they're tappable without zooming. Pure HTML/CSS/JS — sits on top of the canvas rather
/// than being drawn by the game itself. Hotkeys listed here must match what
/// `control::Control` actually reads (`=`/`-`/`0`/`Space`/`F`), plus any per-game hotkey
/// the game's own `main.rs` reads directly (e.g. `V`). Hidden instead under `?embed=1`/
/// `?stream=1` (see `mode_class_script`'s `window.__hcgHide`) — a 48px popup button is
/// visual clutter on an ambient-wall tile or an OBS/Twitch browser-source layer. The panel
/// also carries a `?stream=1` link — otherwise stream mode is an undocumented URL param
/// nobody would ever find — so a streamer can turn it on from the same place they'd
/// already look for controls, without needing to know the query param exists ahead of
/// time; clicking it reloads into stream mode immediately, which also doubles as a live
/// preview before they copy the URL into OBS.
pub fn hotkey_popup(name: &str) -> Markup {
    let has_variant_switch = matches!(
        name,
        "klondike" | "spider" | "sudoku" | "minesweeper" | "tetris" | "match-3"
    );
    html! {
        button id="hotkeys-btn" aria-label="Show hotkeys" { "?" }
        div id="hotkeys" {
            div class="panel" {
                button id="hotkeys-close" aria-label="Close hotkeys" { "×" }
                h2 { "Hotkeys" }
                dl {
                    dt { "=" } dd { "speed up" }
                    dt { "-" } dd { "slow down" }
                    dt { "0" } dd { "reset speed" }
                    dt { "Space" } dd { "pause / resume" }
                    dt { "M" } dd { "mute / unmute" }
                    dt { "F" } dd { "toggle fullscreen (or double-click)" }
                    @if has_variant_switch {
                        dt { "V" } dd { "switch game variant" }
                    }
                    dt { "R" } dd { "show / type a seed to replay" }
                    dt { "S" } dd { "save screenshot" }
                    dt { "2-finger slide" } dd { "adjust speed (touch)" }
                    @if has_variant_switch {
                        dt { "swipe" } dd { "switch game variant (touch)" }
                    }
                    dt { "?" } dd { "toggle this help (or tap the button)" }
                    dt { "Esc" } dd { "close (or tap ×)" }
                }
                p class="stream-hint" {
                    "🎥 Streaming? " a href="?stream=1" { "Open in stream mode" }
                    " for a clean, transparent OBS/Twitch layer — no HUD, no popup, see-through background."
                }
            }
        }
        script {
            (PreEscaped(minify_js(
                "if (window.__hcgHide) {\n\
                 \x20 document.getElementById('hotkeys-btn').style.display = 'none';\n\
                 } else {\n\
                 \x20 document.addEventListener('keydown', function(e) {\n\
                 \x20   if (e.key === '?') document.getElementById('hotkeys').classList.toggle('open');\n\
                 \x20   else if (e.key === 'Escape') document.getElementById('hotkeys').classList.remove('open');\n\
                 \x20 });\n\
                 \x20 document.getElementById('hotkeys-btn').addEventListener('click', function() {\n\
                 \x20   document.getElementById('hotkeys').classList.toggle('open');\n\
                 \x20 });\n\
                 \x20 document.getElementById('hotkeys-close').addEventListener('click', function() {\n\
                 \x20   document.getElementById('hotkeys').classList.remove('open');\n\
                 \x20 });\n\
                 }"
            )))
        }
    }
}

/// A dismissible banner nudging a visitor to rotate their device when the viewport's
/// orientation doesn't match this game's native one (e.g. a 900x720 landscape game
/// opened on a portrait phone) — the `window.fitCanvas` scale-to-fit in `native_size_style`
/// already handles this case technically (it just shrinks the canvas further to fit),
/// but on a badly-mismatched orientation that can leave the game a small fraction of the
/// screen. Pure page-level HTML/CSS/JS, same pattern as `hotkey_popup`/`screenshot_bridge`.
/// Dismissal is per-`sessionStorage` (not persisted across visits) so it can nudge again
/// next session rather than being silenced forever after one tap. Never shown at all
/// under `?embed=1`/`?stream=1` (see `mode_class_script`'s `window.__hcgHide`) — a tiny
/// ambient-wall iframe tile's own viewport dimensions are a meaningless orientation
/// signal, and an OBS/Twitch
/// browser-source layer has no visitor around to rotate anything for either way.
///
/// An orientation *mismatch* alone is not enough to show it: that condition is satisfied
/// by an ordinary landscape desktop window opening a portrait game (tetris,
/// bubble-shooter, game2048), which told every desktop visitor to rotate a device they
/// can't. The banner is now gated on the viewport belonging to something rotatable at all.
pub fn orientation_hint(name: &str) -> Markup {
    let (w, h) = native_size(name);
    let game_is_landscape = w > h;
    let dismiss_key = format!("hcg-rotate-dismissed-{name}");
    html! {
        style {
            (PreEscaped(
                "#rotate-hint { display: none; background: rgba(20,20,24,0.92); color: #fff; \
                 font: 14px system-ui, sans-serif; padding: 10px 16px; \
                 align-items: center; justify-content: center; gap: 12px; text-align: center; }\n\
                 #rotate-hint.show { display: flex; }\n\
                 #rotate-hint button { background: rgba(255,255,255,0.15); color: #fff; \
                 border: none; border-radius: 50%; width: 28px; height: 28px; \
                 font-size: 16px; line-height: 28px; padding: 0; cursor: pointer; flex: none; }"
            ))
        }
        div id="rotate-hint" {
            span { "🔄 Rotate your device for a bigger screen" }
            button id="rotate-hint-close" aria-label="Dismiss" { "×" }
        }
        script {
            (PreEscaped(minify_js(&format!(
                "(function() {{\n\
                 \x20 if (window.__hcgHide) return;\n\
                 \x20 var key = '{dismiss_key}';\n\
                 \x20 var gameIsLandscape = {game_is_landscape};\n\
                 \x20 var el = document.getElementById('rotate-hint');\n\
                 \x20 // Only a device someone can physically rotate. `pointer: coarse` is the\n\
                 \x20 // primary-input test, so a touchscreen laptop driven by a trackpad is\n\
                 \x20 // excluded; maxTouchPoints then rules out a desktop browser that reports\n\
                 \x20 // a coarse pointer anyway (headless Chrome does).\n\
                 \x20 var canRotate = window.matchMedia('(pointer: coarse)').matches\n\
                 \x20   && navigator.maxTouchPoints > 0;\n\
                 \x20 function check() {{\n\
                 \x20   if (!canRotate) return;\n\
                 \x20   if (sessionStorage.getItem(key)) {{ el.classList.remove('show'); return; }}\n\
                 \x20   var viewportIsPortrait = window.innerHeight > window.innerWidth;\n\
                 \x20   el.classList.toggle('show', viewportIsPortrait === gameIsLandscape);\n\
                 \x20 }}\n\
                 \x20 window.addEventListener('resize', check);\n\
                 \x20 window.addEventListener('orientationchange', check);\n\
                 \x20 document.getElementById('rotate-hint-close').addEventListener('click', function() {{\n\
                 \x20   sessionStorage.setItem(key, '1');\n\
                 \x20   el.classList.remove('show');\n\
                 \x20 }});\n\
                 \x20 check();\n\
                 }})();"
            ))))
        }
    }
}

/// Fixed strip across the top of a game page holding its dismissible notices — the
/// rotate-device hint and the "sound is off" hint (shown by `audio_bridge`'s
/// `hcg_audio_running`). A flex column, so when both are showing they stack instead of
/// one covering the other. Placed after `load(...)`: `audio_bridge` looks `#sound-hint`
/// up at call time, well after the wasm fetch lets parsing reach it.
pub fn top_banners(name: &str) -> Markup {
    html! {
        style {
            (PreEscaped(
                "#top-banners { position: fixed; top: 0; left: 0; right: 0; z-index: 12; \
                 display: flex; flex-direction: column; }\n\
                 #sound-hint { display: none; background: rgba(20,20,24,0.92); color: #fff; \
                 font: 14px system-ui, sans-serif; padding: 10px 16px; text-align: center; \
                 cursor: pointer; }\n\
                 #sound-hint.show { display: block; }\n\
                 #sound-hint small { display: block; opacity: 0.7; font-size: 12px; margin-top: 2px; }"
            ))
        }
        div id="top-banners" {
            div id="sound-hint" {
                "🔇 Sound is off until you tap or press a key"
                small { "Allow autoplay for this site in your browser's settings to skip this." }
            }
            (orientation_hint(name))
        }
    }
}

/// Game-page palette. Only the page *around* the game follows `theme_init_script`'s light
/// theme: `.stage` stays black in both (every game draws a dark canvas, and the scroll cue
/// and loading line inside it are tuned for that), while the below-fold `.page-info`
/// section and the page background switch. The fixed overlay buttons float over both the
/// black stage and, once scrolled, the light section — so under light they turn dark
/// translucent instead of white translucent, which reads on either background.
const PAGE_INFO_CSS: &str = "\
:root { --page-bg: #000; --pi-text: #a89a86; --pi-accent: #d4a373; --pi-heading: #f0ece2; \
--pi-card-text: #e7ddcd; --pi-border: rgba(212,163,115,0.18); --pi-border-hi: rgba(212,163,115,0.5); \
--pi-card-bg: rgba(255,255,255,0.02); }\n\
:root[data-theme=\"light\"] { color-scheme: light; --page-bg: #f5efe4; --pi-text: #5e4e3b; \
--pi-accent: #9a5b22; --pi-heading: #2c2015; --pi-card-text: #3a2d1f; \
--pi-border: rgba(154,91,34,0.22); --pi-border-hi: rgba(154,91,34,0.6); --pi-card-bg: #fffaf2; }\n\
:root[data-theme=\"light\"] #hotkeys-btn, :root[data-theme=\"light\"] #daily-btn, \
:root[data-theme=\"light\"] #share-btn { background: rgba(20,16,12,0.55); }\n\
:root[data-theme=\"light\"] #hotkeys-btn:hover, :root[data-theme=\"light\"] #daily-btn:hover, \
:root[data-theme=\"light\"] #share-btn:hover { background: rgba(20,16,12,0.72); }\n\
.scroll-cue { position: absolute; bottom: 10px; left: 0; right: 0; z-index: 9; \
text-align: center; color: rgba(255,255,255,0.28); font: 20px system-ui, sans-serif; \
line-height: 1; pointer-events: none; transition: opacity 0.3s; }\n\
.scroll-cue.gone { opacity: 0; }\n\
.page-info { max-width: 760px; margin: 0 auto; padding: 3.5rem 1.5rem 4.5rem; \
font-family: system-ui, sans-serif; color: var(--pi-text); }\n\
.page-info .home-link { display: inline-block; margin-bottom: 1.2rem; font-size: 0.8rem; \
color: var(--pi-accent); text-decoration: none; }\n\
.page-info .home-link:hover { text-decoration: underline; }\n\
.page-info h1 { font-size: clamp(1.3rem, 5vw, 1.7rem); font-weight: 600; color: var(--pi-heading); \
margin-bottom: 0.9rem; }\n\
.page-info p { font-size: 0.95rem; line-height: 1.7; margin-bottom: 0.9rem; }\n\
.page-info p a { color: var(--pi-accent); }\n\
.page-info h2 { font-size: 0.7rem; letter-spacing: 0.14em; text-transform: uppercase; \
color: var(--pi-accent); margin: 2.75rem 0 1.1rem; }\n\
.related { display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); \
gap: 1rem; }\n\
.related a { display: block; text-decoration: none; border: 1px solid var(--pi-border); \
border-radius: 10px; overflow: hidden; background: var(--pi-card-bg); }\n\
.related a:hover { border-color: var(--pi-border-hi); }\n\
.related img { display: block; width: 100%; aspect-ratio: 4 / 3; object-fit: cover; }\n\
.related span { display: block; padding: 0.6rem 0.7rem; font-size: 0.85rem; color: var(--pi-card-text); }";

/// Every game in the workspace, by directory name, sorted the same way the homepage's
/// grid sorts (by display `title`). Read from `games/` in the source tree rather than from
/// `dist/` (which is how `generate_index` discovers them): `generate_game_html` runs once
/// per game *while* `mise run deploy` is still building the others, so on a fresh clone
/// `dist/` holds only the games built so far and a `dist/`-derived list would give the
/// first game zero related links and the last one all of them. `games/` is complete from
/// the start and needs no build to be accurate. Empty (so the section is simply skipped)
/// if the directory can't be read — e.g. running the generator from somewhere other than
/// the repo root.
pub fn all_games() -> Vec<String> {
    let mut games: Vec<String> = std::fs::read_dir("games")
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    games.sort_by_key(|name| title(name));
    games
}

/// A `<img>`'s responsive `srcset`: every `dist/<game>/preview-<w>.png` tier that exists
/// (`resize_preview` writes these, per-game exact fractions of that game's native preview
/// — see `TIER_FRACTIONS` there), plus the full-size `preview.png` as the largest
/// candidate. Read back off disk rather than hardcoded, since the tiers are per-game. A
/// game with no tiers at all (none generated yet, or a preview narrower than the smallest
/// tier) gets a plain `src` and no `srcset` — `None`.
///
/// Shared by the homepage/wall grids (`generate_index.rs`) and each game page's
/// related-games cards (`game_page_info`) — before this was wired into the latter, a
/// game page shipped the *full* ~900px-wide preview.png for each of its 3 related-game
/// cards despite displaying them at ~300px, the single largest image-delivery waste
/// Lighthouse flagged on a game page.
///
/// `prefix` is prepended to each candidate URL so the same tiers can be referenced from
/// pages at different depths (homepage at `dist/index.html` passes `""`, a page one
/// level deep — the wall, or any game page's related cards — passes `"../"`).
pub fn preview_srcset(dist: &Path, game: &str, prefix: &str) -> Option<String> {
    let dir = dist.join(game);
    let mut widths: Vec<u32> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(|e| {
            let name = e.ok()?.file_name().into_string().ok()?;
            name.strip_prefix("preview-")?
                .strip_suffix(".png")?
                .parse::<u32>()
                .ok()
        })
        .collect();
    if widths.is_empty() {
        return None;
    }
    widths.sort_unstable();
    let mut entries: Vec<String> = widths
        .iter()
        .map(|w| format!("{prefix}{game}/preview-{w}.png {w}w"))
        .collect();
    let (full_w, _) = image::image_dimensions(dir.join("preview.png")).ok()?;
    entries.push(format!("{prefix}{game}/preview.png {full_w}w"));
    Some(entries.join(", "))
}

/// Filename of the one wasm binary every game page loads — `bundle/`'s merged build, which
/// picks its game at runtime from `game_id_bridge`'s baked index. Site-root-relative
/// (`dist/hcg.wasm`), not per-game, so a second page visit and the wall's 11 iframes hit the
/// browser cache instead of downloading another copy of the framework.
pub const BUNDLE_WASM: &str = "hcg.wasm";

/// This game's index in the merged bundle's dispatch table (`bundle/src/main.rs`'s
/// `GAME_NAMES`) — what `game_id_bridge` bakes into the page.
///
/// Deliberately its own directory listing rather than `all_games()`: that one sorts by
/// display `title()` ("2048" before "Arrow Blocks"), so renaming a game's *title* would
/// silently renumber every index and point pages at the wrong game. Directory names are
/// stable, and `bundle`'s own `bundle_list_matches_games_dir` test asserts its array matches
/// this same plain-sorted listing.
pub fn bundle_game_index(name: &str) -> Option<usize> {
    let mut games: Vec<String> = std::fs::read_dir("games")
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    games.sort();
    games.iter().position(|g| g == name)
}

/// Registers a miniquad plugin exposing `env.hcg_game_id`, which tells the merged wasm
/// binary (`bundle/`) which of the 11 games *this* page is. Must run before `load(...)`,
/// same ordering constraint as `analytics_bridge` — and the id has to be readable before
/// `Window::from_config`, since it selects the game's window title and native canvas size,
/// not just its draw loop.
///
/// The index is baked in at generation time rather than read from a URL query or the path:
/// the id is a property of the page, not of how a visitor arrived at it, and a query param
/// would leak into canonical URLs and be trivially spoofable into a mismatch with the page's
/// own title/OG tags.
pub fn game_id_bridge(name: &str) -> Markup {
    let id = bundle_game_index(name).unwrap_or(0);
    html! {
        script {
            (PreEscaped(minify_js(&format!(
                "miniquad_add_plugin({{\n\
                 \x20 register_plugin: function(importObject) {{\n\
                 \x20   importObject.env.hcg_game_id = function() {{ return {id}; }};\n\
                 \x20 }},\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_game_id\"\n\
                 }});"
            ))))
        }
    }
}

/// The one line on a game page that is unique to that game. Two jobs, in this order: name the
/// specific bad habit a human has in *this* game, then have the AI be insufferable about it.
///
/// It used to explain the solver — beam widths, node budgets, technique names — on the theory that
/// the vocabulary bought keyword coverage. It didn't: those queries are either tool-intent
/// ("sudoku solver") or video-dominated ("ai plays tetris"), so the words earned nothing, and 11
/// explainers built to the same rhythm read as generated. Implementation detail doesn't belong in
/// site copy unless it earns its place with a laugh or a search term; anyone curious can read the
/// source. **Don't reintroduce solver internals here.**
fn game_flavor(name: &str) -> &'static str {
    match name {
        "snake" => {
            "You died to your own tail and called it lag. It has never met an obstacle it \
            didn't put there on purpose."
        }
        "game2048" => {
            "You swipe down when you panic. It picked the corner this ends in some forty \
            moves ago and will not be taking questions."
        }
        "tetris" => {
            "You were saving that I-piece for something. It doesn't need one — it builds \
            cathedrals out of whatever falls."
        }
        "klondike" => {
            "You restarted this deal four times. It has never restarted anything, and \
            regards your reshuffling as a form of prayer."
        }
        "spider" => {
            "You gave up on four-suit years ago. It plays four-suit like it's child's play."
        }
        "sudoku" => "You wrote it in pen. It was a 4. It knew that before you picked up the pen.",
        "minesweeper" => {
            "You called it intuition. It was a coin flip and you knew it. This one \
            doesn't flip coins; it simply declines to be wrong."
        }
        "match-3" => "You tapped the same gem twice and hoped. It doesn't hope. It arranges.",
        "arrow-blocks" => "You'd have saved the heart for last. It doesn't see a heart.",
        "bubble-shooter" => {
            "You aimed straight up. It banks off the wall like the wall was its \
            own idea."
        }
        "water-sort" => {
            "You poured red into blue and called it a plan. It has never once reached \
            for undo, and finds yours touching."
        }
        "labyrinth" => {
            "You keep one hand on the wall and call it a strategy. It let go of the \
            wall a long time ago and it's nearly at the stairs."
        }
        _ => "You would have done it differently. It would not have listened.",
    }
}

/// The `n` games following `name` in `all_games()` order, wrapping around. A rotation
/// rather than any notion of similarity: it needs no hand-maintained relatedness table,
/// and it guarantees every game is linked from exactly `n` others, so the internal link
/// graph is uniform instead of pointing everything at a few favourites.
fn related_games(name: &str, n: usize) -> Vec<String> {
    let games = all_games();
    let Some(pos) = games.iter().position(|game| game == name) else {
        return Vec::new();
    };
    (1..=n)
        .map(|offset| games[(pos + offset) % games.len()].clone())
        .filter(|game| game != name)
        .collect()
}

/// A dim chevron at the bottom of `.stage` hinting that the page continues below the game
/// (see `game_page_info`) — with the stage exactly one viewport tall, nothing peeks over
/// the fold to suggest it on its own. Fades out for good on the first scroll; `once: true`
/// so it isn't re-hidden on every subsequent scroll event. Must be emitted *inside*
/// `.stage` (it's positioned against it), which is why this is separate from
/// `game_page_info` rather than part of it. Hidden with the rest of the below-fold chrome
/// under `?embed=1`/`?stream=1` — `mode_class_script`'s `hcg-bare` makes those pages
/// unscrollable, so a "scroll down" hint would be pointing at nothing.
pub fn scroll_cue() -> Markup {
    html! {
        div class="scroll-cue" aria-hidden="true" { "⌄" }
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 var cue = document.querySelector('.scroll-cue');\n\
                 \x20 if (window.__hcgHide) { cue.style.display = 'none'; return; }\n\
                 \x20 window.addEventListener('scroll', function() {\n\
                 \x20   cue.classList.add('gone');\n\
                 \x20 }, { once: true, passive: true });\n\
                 })();"
            )))
        }
    }
}

/// The below-the-fold content section on a game page: an `h1`, the game's description, a
/// link back to the homepage, and a small grid of related games.
///
/// This exists for indexing, not decoration. Before it, a game page's `<body>` was a
/// `<canvas>`, a hidden hotkey popup and some scripts — no `<a>` at all (so Search
/// Console reported "Referring page: None detected", i.e. Google knew these URLs only
/// from the sitemap, never from the link graph) and no text in page flow at all, since
/// `loading_screen`'s line is painted over the moment the game starts drawing. Every
/// indexable word lived in `<title>`/`<meta>`/OG tags, which reads as a thin page.
///
/// It sits *after* `.stage` (one full viewport of game — see `native_size_style`), so the
/// game still owns the entire first screen and the page looks exactly as clean as it did;
/// this is only reachable by scrolling. `?embed=1`/`?stream=1` hide it outright via
/// `mode_class_script`'s `hcg-bare` class.
///
/// The Space-key `preventDefault` is a consequence of the page becoming scrollable at
/// all: Space is the pause hotkey (`control::Control`), and it's also the browser's
/// scroll-down key, so pausing used to be free but would now jump the reader a screen
/// down. `preventDefault` (not `stopPropagation`) leaves the event reaching miniquad's
/// own listener, so pause still works. Skipped when a button/link has focus, where Space
/// means "activate this" (e.g. the hotkey popup's own buttons).
pub fn game_page_info(dist: &Path, name: &str) -> Markup {
    let game_title = title(name);
    let related = related_games(name, 3);
    let all_count = all_games().len();
    html! {
        section id="about" class="page-info" {
            a class="home-link" href="../" { "← Hotel Chair Games" }
            h1 { (game_title) ", played by an AI" }
            p { (description(name)) }
            p { (game_flavor(name)) }
            p {
                "Nothing to install, nothing to sign up for, nothing to click. "
                a href="../wall/" { "The ambient wall" }
                " runs every game at once, if one of them isn't enough."
            }
            @if !related.is_empty() {
                h2 { "More self-playing games" }
                div class="related" {
                    @for game in &related {
                        @let srcset = preview_srcset(dist, game, "../");
                        a href=(format!("../{game}/")) {
                            img src=(format!("../{game}/preview.png"))
                                srcset=[srcset.clone()]
                                sizes=[srcset.is_some().then_some("(max-width: 480px) 90vw, (max-width: 900px) 45vw, 220px")]
                                alt=(format!("{} being played by an AI", title(game)))
                                loading="lazy";
                            span { (title(game)) }
                        }
                    }
                }
                p style="margin-top: 1.1rem; font-size: 0.85rem;" {
                    a href="../" { (format!("← All {all_count} games")) }
                }
            }
        }
        script {
            (PreEscaped(minify_js(
                "document.addEventListener('keydown', function(e) {\n\
                 \x20 if (e.key !== ' ') return;\n\
                 \x20 var tag = (e.target && e.target.tagName) || '';\n\
                 \x20 if (tag === 'BUTTON' || tag === 'A' || tag === 'INPUT') return;\n\
                 \x20 e.preventDefault();\n\
                 });"
            )))
        }
    }
}

/// Registers a miniquad plugin exposing `env.hcg_ga_event` to the wasm module, so
/// `control::Control::episode_complete` can fire `gtag('event', ...)` calls from Rust.
/// Must run after `mq_js_bundle.js` (needs its global `miniquad_add_plugin`/`UTF8ToString`)
/// but before `load(...)` (plugins register into the import object at instantiation time).
/// A no-op when `window.gtag` isn't defined (GTAG_ID unset locally).
///
/// Also the cheapest place to count `episode_complete`s toward `session_end`'s
/// `episodes_seen`: every one already passes through this one function on its way from
/// Rust, so `session_signals_bridge` just reads a counter incremented here instead of
/// adding a second wasm→JS call. `window.__hcgSessionStats` is only defined when that
/// bridge runs (i.e. not under `?embed=1`/`?stream=1`), so the check also doubles as that
/// suppression without this function needing to know about embed mode itself.
pub fn analytics_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 register_plugin: function(importObject) {\n\
                 \x20   importObject.env.hcg_ga_event = function(namePtr, nameLen, paramsPtr, paramsLen) {\n\
                 \x20     var name = UTF8ToString(namePtr, nameLen);\n\
                 \x20     var params = paramsLen > 0 ? JSON.parse(UTF8ToString(paramsPtr, paramsLen)) : {};\n\
                 \x20     if (name === 'episode_complete' && window.__hcgSessionStats) {\n\
                 \x20       window.__hcgSessionStats.episodesSeen++;\n\
                 \x20     }\n\
                 \x20     if (window.gtag) window.gtag('event', name, params);\n\
                 \x20   };\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_analytics\"\n\
                 });"
            )))
        }
    }
}

/// `game_switch {from, to}` and `session_end {game, seconds_watched, episodes_seen}` —
/// the two page-navigation/session-length signals from the `session-signals` idea in
/// `.notes/aiideas.md`. Both are pure page-level JS, no wasm round-trip: `game_switch`
/// only needs to compare this page's game name against whatever the last game page in
/// this tab was, and `session_end` only needs a page-visibility hook — neither needs
/// anything from the wasm module itself (`episodes_seen` aside, see below).
///
/// `from` comes from `sessionStorage`, not `document.referrer`: referrer is stripped or
/// absent under common referrer policies/extensions, and a plain reload would also read
/// as a same-game "switch" unless specially excluded. `sessionStorage` is scoped to this
/// tab and only changes when a *different* game page in this tab actually finishes
/// loading, which is exactly the transition this event wants.
///
/// `episodes_seen` piggybacks on `analytics_bridge`'s existing `hcg_ga_event` plugin
/// (see its doc comment) rather than adding a second wasm export — every
/// `episode_complete` already passes through that one function.
///
/// `session_end` fires on `visibilitychange` (backgrounded/switched tab) and `pagehide`
/// (actual navigation/close), not `unload` — mobile Safari does not reliably fire
/// `unload` at all. A `sent` flag stops the pair from double-firing when both occur in
/// quick succession. No `transport_type: 'beacon'` parameter: that's a Universal Analytics
/// field, and GA4's gtag.js already uses `sendBeacon`/`fetch(keepalive)` by itself for hits
/// sent while a page is going away. Passing it here would just ride along as a junk custom
/// event parameter and show up as one in reports.
///
/// Suppressed entirely under `?embed=1`/`?stream=1` (see `mode_class_script`'s
/// `window.__hcgHide`): the wall (`generate_index`'s `wall_page`) runs up to 11 of these
/// pages at once in iframes, and a `game_switch`/`session_end` from every tile on every
/// wall view would flood the
/// property with events nobody could use — the wall has its own, separate signal
/// (`wall_analytics_bridge`). Dropped instead of sent when `window.gtag` is undefined
/// (GTAG_ID unset locally), same convention as every other bridge here.
pub fn session_signals_bridge(name: &str) -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(&format!(
                "(function() {{\n\
                 \x20 if (window.__hcgHide) return;\n\
                 \x20 var GAME = \"{name}\";\n\
                 \x20 var prev = sessionStorage.getItem('hcg_last_game');\n\
                 \x20 if (prev && prev !== GAME && window.gtag) {{\n\
                 \x20   window.gtag('event', 'game_switch', {{ from: prev, to: GAME }});\n\
                 \x20 }}\n\
                 \x20 sessionStorage.setItem('hcg_last_game', GAME);\n\
                 \x20\n\
                 \x20 window.__hcgSessionStats = {{ episodesSeen: 0, start: performance.now() }};\n\
                 \x20 var sent = false;\n\
                 \x20 function sendSessionEnd() {{\n\
                 \x20   if (sent || !window.gtag) return;\n\
                 \x20   sent = true;\n\
                 \x20   var seconds = Math.round((performance.now() - window.__hcgSessionStats.start) / 1000);\n\
                 \x20   window.gtag('event', 'session_end', {{\n\
                 \x20     game: GAME,\n\
                 \x20     seconds_watched: seconds,\n\
                 \x20     episodes_seen: window.__hcgSessionStats.episodesSeen\n\
                 \x20   }});\n\
                 \x20 }}\n\
                 \x20 document.addEventListener('visibilitychange', function() {{\n\
                 \x20   if (document.visibilityState === 'hidden') sendSessionEnd();\n\
                 \x20 }});\n\
                 \x20 window.addEventListener('pagehide', sendSessionEnd);\n\
                 }})();"
            ))))
        }
    }
}

/// `wall_view` (fired once per load of `generate_index`'s `wall_page`) and
/// `wall_tile_click` (`{game}`, fired when a visitor focuses one of the embedded game
/// iframes) — the wall was previously the most shareable page on the site and emitted no
/// analytics at all.
///
/// Click detection can't use a plain `click` listener: a click landing inside an
/// `<iframe>` fires and stays inside *that* iframe's own document, it never bubbles out
/// to the parent page. The standard workaround is used instead — clicking into an iframe
/// moves keyboard focus into it, which fires `blur` on the parent `window`; checking
/// `document.activeElement` right after (`setTimeout(…, 0)` lets focus actually land
/// first) identifies which iframe just got clicked via its `data-game` attribute
/// (`wall_page` sets one per tile). Each embedded game page already suppresses its own
/// `game_switch`/`session_end` under `?embed=1` (see `session_signals_bridge`) — this is
/// the wall's own, separate signal: one event per interaction with the grid, not one
/// `session_end` per tile per wall view.
pub fn wall_analytics_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 if (window.gtag) window.gtag('event', 'wall_view', {});\n\
                 \x20 window.addEventListener('blur', function() {\n\
                 \x20   setTimeout(function() {\n\
                 \x20     var el = document.activeElement;\n\
                 \x20     if (el && el.tagName === 'IFRAME' && el.dataset.game && window.gtag) {\n\
                 \x20       window.gtag('event', 'wall_tile_click', { game: el.dataset.game });\n\
                 \x20     }\n\
                 \x20   }, 0);\n\
                 \x20 });\n\
                 })();"
            )))
        }
    }
}

/// Mount/unmount logic for `wall_page`'s tiles. Each tile starts as a `<div
/// class="wall-tile" data-game="...">` holding only a static `<img class="wall-preview">`
/// (that game's existing `preview.png`, same file the OG-image fallback chain uses) — no
/// iframe, no WASM instance, until this script decides one is warranted. `loading="lazy"`
/// on a bare `<iframe>` (the previous design) only defers the *first* load; it never
/// unloads, so scrolling the page once was enough to mount all 11 WASM instances — each
/// holding its own WebGL context and wasm heap — and leave every one of them running
/// forever. iOS Safari caps simultaneous WebGL contexts and silently evicts the oldest
/// once that cap is hit; on a page whose entire premise is "leave 11 of these running",
/// that reliably went badly. This replaces the static `<iframe>` with an
/// `IntersectionObserver` that mounts a live iframe only for tiles actually near the
/// viewport and unmounts (fully removes, not `src="about:blank"` — that can still pin the
/// context alive) any tile that scrolls back out, under a hard cap on how many can be
/// live at once.
///
/// Budget is computed once at load, not re-checked on resize/rotate — the wall is a
/// leave-it-running background page, not a page a visitor is expected to resize mid-view:
///   - `navigator.connection.saveData` set → 0. A data-saver visitor gets zero live tiles,
///     full stop — the cap is enforced for *every* mount path below, including a tap, not
///     just the automatic one. A tap is still an explicit request to spend a WASM
///     download's worth of data, but "save data" is a stated, standing preference this
///     page has no way to ask about per-click, so it's honored outright rather than
///     partially.
///   - `navigator.deviceMemory` present and < 4 → 3 (both signals are Chromium-only and
///     absent elsewhere, incl. all of iOS Safari — the `undefined` case falls through to
///     the viewport-width rule below, which is the one that actually matters on phones).
///   - viewport width < 700 → 3, < 1100 → 6, else 11 (every game on the site, no cap).
///
/// `prefers-reduced-motion: reduce` disables the `IntersectionObserver` entirely (nothing
/// auto-mounts while scrolling) but leaves `budget` computed as above and tap-to-mount
/// live — reduced motion means "don't animate/change things around me without asking",
/// not "never let me start one on purpose".
///
/// Eviction picks whichever live tile's vertical center is currently furthest from the
/// viewport's vertical center — a simple proxy for "least likely to be looked at right
/// now" that's cheap to compute and needs no separate LRU bookkeeping; the grid only
/// scrolls vertically (see `WALL_STYLE`'s `auto-fit` columns), so vertical distance is
/// the only axis that matters.
///
/// The tile's own click handler bails out on any click that lands inside an `<a>` — each
/// tile carries a hover-revealed `.wall-label` link to that game's own page (see
/// `WALL_STYLE`), and a tap there means "take me to this game", not "mount it here".
///
/// Tap-to-mount fires `wall_tile_click` itself rather than relying on
/// `wall_analytics_bridge`'s blur/`document.activeElement` detection, because that
/// detection only works once an iframe already exists to receive focus — the whole point
/// of the preview state is that it doesn't. No double-fire risk between the two paths:
/// once a tile is live its iframe fills the tile and swallows the click in its own
/// document (same cross-document reason `wall_analytics_bridge`'s doc comment gives for
/// needing the blur trick at all), so this tile's own `click` listener structurally can't
/// fire again until the tile goes back to preview state — `wall_analytics_bridge`'s blur
/// path is what picks up interaction with an already-live tile instead.
pub fn wall_live_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 var reduceMotion = window.matchMedia &&\n\
                 \x20   window.matchMedia('(prefers-reduced-motion: reduce)').matches;\n\
                 \x20 var conn = navigator.connection;\n\
                 \x20 var budget;\n\
                 \x20 if (conn && conn.saveData) {\n\
                 \x20   budget = 0;\n\
                 \x20 } else if (navigator.deviceMemory && navigator.deviceMemory < 4) {\n\
                 \x20   budget = 3;\n\
                 \x20 } else if (window.innerWidth < 700) {\n\
                 \x20   budget = 3;\n\
                 \x20 } else if (window.innerWidth < 1100) {\n\
                 \x20   budget = 6;\n\
                 \x20 } else {\n\
                 \x20   budget = 11;\n\
                 \x20 }\n\
                 \x20\n\
                 \x20 var live = new Map();\n\
                 \x20\n\
                 \x20 function distanceFromViewportCenter(tile) {\n\
                 \x20   var r = tile.getBoundingClientRect();\n\
                 \x20   return Math.abs((r.top + r.height / 2) - window.innerHeight / 2);\n\
                 \x20 }\n\
                 \x20\n\
                 \x20 function evictFarthest(except) {\n\
                 \x20   var worst = null, worstDist = -1;\n\
                 \x20   live.forEach(function(_, tile) {\n\
                 \x20     if (tile === except) return;\n\
                 \x20     var d = distanceFromViewportCenter(tile);\n\
                 \x20     if (d > worstDist) { worstDist = d; worst = tile; }\n\
                 \x20   });\n\
                 \x20   if (worst) unmount(worst);\n\
                 \x20 }\n\
                 \x20\n\
                 \x20 function mount(tile) {\n\
                 \x20   if (budget <= 0 || live.has(tile)) return;\n\
                 \x20   if (live.size >= budget) evictFarthest(tile);\n\
                 \x20   if (live.size >= budget) return;\n\
                 \x20   var game = tile.dataset.game;\n\
                 \x20   var iframe = document.createElement('iframe');\n\
                 \x20   iframe.className = 'wall-live';\n\
                 \x20   iframe.dataset.game = game;\n\
                 \x20   iframe.title = tile.getAttribute('title') || game;\n\
                 \x20   iframe.setAttribute('allow', 'fullscreen');\n\
                 \x20   iframe.src = '../' + game + '/index.html?embed=1';\n\
                 \x20   tile.appendChild(iframe);\n\
                 \x20   var img = tile.querySelector('img');\n\
                 \x20   if (img) img.style.visibility = 'hidden';\n\
                 \x20   live.set(tile, iframe);\n\
                 \x20 }\n\
                 \x20\n\
                 \x20 function unmount(tile) {\n\
                 \x20   var iframe = live.get(tile);\n\
                 \x20   if (!iframe) return;\n\
                 \x20   iframe.remove();\n\
                 \x20   live.delete(tile);\n\
                 \x20   var img = tile.querySelector('img');\n\
                 \x20   if (img) img.style.visibility = '';\n\
                 \x20 }\n\
                 \x20\n\
                 \x20 var tiles = document.querySelectorAll('.wall-tile');\n\
                 \x20 tiles.forEach(function(tile) {\n\
                 \x20   tile.addEventListener('click', function(e) {\n\
                 \x20     if (e.target.closest('a')) return;\n\
                 \x20     if (live.has(tile) || budget <= 0) return;\n\
                 \x20     mount(tile);\n\
                 \x20     if (live.has(tile) && window.gtag) {\n\
                 \x20       window.gtag('event', 'wall_tile_click', { game: tile.dataset.game });\n\
                 \x20     }\n\
                 \x20   });\n\
                 \x20 });\n\
                 \x20\n\
                 \x20 if (budget > 0 && !reduceMotion && 'IntersectionObserver' in window) {\n\
                 \x20   var io = new IntersectionObserver(function(entries) {\n\
                 \x20     entries.forEach(function(entry) {\n\
                 \x20       if (entry.isIntersecting) mount(entry.target);\n\
                 \x20       else unmount(entry.target);\n\
                 \x20     });\n\
                 \x20   }, { rootMargin: '200px' });\n\
                 \x20   tiles.forEach(function(tile) { io.observe(tile); });\n\
                 \x20 }\n\
                 })();"
            )))
        }
    }
}

/// Registers a miniquad plugin exposing `env.hcg_initial_variant_is_hex`, letting the
/// wasm module read the page's `?variant=hex` query param at startup — used by the
/// `/minesweeper-hex` redirect stub (`static/minesweeper-hex/index.html`) so it lands
/// directly in Hex mode instead of the Square default. Must run before `load(...)`, same
/// ordering constraint as `analytics_bridge`.
///
/// Registered on *every* game page even though only minesweeper reads it: since all pages
/// load the one merged binary (see `BUNDLE_WASM`), `lib/minesweeper`'s import of this
/// function is present in the module whatever game the page runs, and a wasm import the page
/// never registered fails instantiation with a LinkError.
pub fn variant_query_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 register_plugin: function(importObject) {\n\
                 \x20   importObject.env.hcg_initial_variant_is_hex = function() {\n\
                 \x20     return new URLSearchParams(location.search).get('variant') === 'hex' ? 1 : 0;\n\
                 \x20   };\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_variant_query\"\n\
                 });"
            )))
        }
    }
}

/// Registers a miniquad plugin exposing `env.hcg_is_stream_mode`, letting
/// `control::Control::stream_mode()` read the page's `?stream=1` query param at startup
/// so a game can skip drawing its own in-canvas HUD (score, speed label) for an OBS/Twitch
/// browser-source layer. Must run before `load(...)`, same ordering constraint as
/// `analytics_bridge`/`variant_query_bridge`. Registered unconditionally for every game,
/// since every game has a HUD worth hiding.
pub fn stream_mode_query_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 register_plugin: function(importObject) {\n\
                 \x20   importObject.env.hcg_is_stream_mode = function() {\n\
                 \x20     return new URLSearchParams(location.search).get('stream') === '1' ? 1 : 0;\n\
                 \x20   };\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_stream_mode\"\n\
                 });"
            )))
        }
    }
}

/// Registers a miniquad plugin exposing `env.hcg_is_daily_mode`, letting
/// `control::Control::daily_mode()` read the page's `?daily=1` query param at startup —
/// set by clicking `daily_challenge_button`. Same shape and ordering constraint as
/// `stream_mode_query_bridge`. Registered unconditionally, like that bridge.
pub fn daily_mode_query_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 register_plugin: function(importObject) {\n\
                 \x20   importObject.env.hcg_is_daily_mode = function() {\n\
                 \x20     return new URLSearchParams(location.search).get('daily') === '1' ? 1 : 0;\n\
                 \x20   };\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_daily_mode\"\n\
                 });"
            )))
        }
    }
}

/// Registers a miniquad plugin exposing `env.hcg_is_popup_open`, letting
/// `control::Control::scale()` poll — live, every frame, unlike the read-once
/// `hcg_is_stream_mode`/`hcg_is_daily_mode` above — whether the page-level `?` hotkey
/// popup (`hotkey_popup`'s `#hotkeys` div) is currently open, so a game pauses while a
/// popup covers it instead of continuing to simulate underneath. The element doesn't
/// exist in the DOM yet when this registers (it renders later in `<body>`), but doesn't
/// need to: the closure only runs later, once the wasm module is calling it every
/// frame, by which point the whole page has rendered. Registered unconditionally, same
/// as `stream_mode_query_bridge`/`daily_mode_query_bridge` — every page has this popup.
pub fn popup_pause_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 register_plugin: function(importObject) {\n\
                 \x20   importObject.env.hcg_is_popup_open = function() {\n\
                 \x20     var el = document.getElementById('hotkeys');\n\
                 \x20     return el && el.classList.contains('open') ? 1 : 0;\n\
                 \x20   };\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_popup_pause\"\n\
                 });"
            )))
        }
    }
}

/// `S` hotkey: registers a miniquad plugin exposing `env.hcg_save_screenshot`, called
/// from `screenshot::handle_hotkey` (Rust detects the keypress and reads pixels via
/// `get_screen_data()`, synchronously inside its own frame — see that function's doc
/// comment for why, replacing an earlier page-level `canvas.toBlob()` design that
/// reliably captured blank/transparent frames). Rust hands over raw RGBA8 bytes plus
/// `width`/`height`, already flipped to top-row-first, and a timestamp-based base name;
/// this prepends `name` (the game, baked in at generation time — matches the old
/// filename shape, `{name}-screenshot-<ts>.png`, since multiple games' downloads can
/// land in the same folder) and builds a 2D canvas from the pixels (a `<canvas>` 2D
/// context has no `preserveDrawingBuffer` pitfall — it isn't WebGL), doing the PNG
/// encoding + download entirely with browser APIs, no image-decoding dependency needed
/// on the Rust side. Must run before `load(...)`, same ordering constraint as
/// `analytics_bridge`.
pub fn screenshot_bridge(name: &str) -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(&format!(
                "miniquad_add_plugin({{\n\
                 \x20 register_plugin: function(importObject) {{\n\
                 \x20   importObject.env.hcg_save_screenshot = function(rgbaPtr, rgbaLen, width, height, namePtr, nameLen) {{\n\
                 \x20     var base = UTF8ToString(namePtr, nameLen);\n\
                 \x20     var rgba = new Uint8ClampedArray(wasm_memory.buffer.slice(rgbaPtr, rgbaPtr + rgbaLen));\n\
                 \x20     var c = document.createElement('canvas');\n\
                 \x20     c.width = width;\n\
                 \x20     c.height = height;\n\
                 \x20     c.getContext('2d').putImageData(new ImageData(rgba, width, height), 0, 0);\n\
                 \x20     c.toBlob(function(blob) {{\n\
                 \x20       var url = URL.createObjectURL(blob);\n\
                 \x20       var a = document.createElement('a');\n\
                 \x20       a.href = url;\n\
                 \x20       a.download = '{name}-' + base + '.png';\n\
                 \x20       a.click();\n\
                 \x20       URL.revokeObjectURL(url);\n\
                 \x20     }});\n\
                 \x20   }};\n\
                 \x20 }},\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_screenshot\"\n\
                 }});"
            ))))
        }
    }
}

const DAILY_BTN_CSS: &str = "\
#daily-btn { display: block; position: fixed; top: 14px; left: 14px; z-index: 10; \
padding: 0 14px; height: 40px; border-radius: 20px; border: none; \
background: rgba(255,255,255,0.15); color: #fff; font: 14px system-ui, sans-serif; \
line-height: 40px; text-align: center; text-decoration: none; cursor: pointer; }\n\
#daily-btn:hover { background: rgba(255,255,255,0.28); }\n\
html.hcg-fullscreen #daily-btn { display: none; }";

/// A fixed top-left button toggling `?daily=1` on/off by reloading the page — the entry
/// point for `control::Control::daily_mode()` (read from that query param at wasm
/// startup, see `daily_mode_query_bridge`). Deliberately a page-level button rather than
/// a hotkey: unlike the existing hotkeys, "start a different, deterministic run" isn't
/// something that makes sense mid-episode, so there's nothing to bind a keypress to.
/// Label/href are set by script at load time (from the *current* URL) rather than baked
/// in statically, since the same generated HTML serves both states. Hidden under
/// `?embed=1`/`?stream=1` (see `mode_class_script`'s `window.__hcgHide`), same as
/// `hotkeys-btn`; also hidden under the `hcg-fullscreen` class `fullscreen_bridge` toggles
/// on `<html>` — clicking it
/// reloads the page, which would silently kick a fullscreened visitor back out, so it's
/// pointless clutter to show there. The click itself fires a `daily_challenge_click` GA
/// event (`entering`/leaving flag, since the same button does both) straight from this
/// script — no wasm round-trip needed, unlike `episode_complete`'s `daily` flag (see
/// `control::Control::episode_complete`), since the click always precedes the reload
/// that would otherwise destroy any in-flight wasm call anyway.
pub fn daily_challenge_button() -> Markup {
    html! {
        style { (PreEscaped(DAILY_BTN_CSS)) }
        a id="daily-btn" href="?daily=1"
            title="You may watch closely. That won't help either."
            { "📅 Daily Challenge" }
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 var btn = document.getElementById('daily-btn');\n\
                 \x20 if (window.__hcgHide) { btn.style.display = 'none'; return; }\n\
                 \x20 var isDaily = new URLSearchParams(location.search).get('daily') === '1';\n\
                 \x20 if (isDaily) {\n\
                 \x20   btn.textContent = '🎲 Random Run';\n\
                 \x20   btn.href = location.pathname;\n\
                 \x20 }\n\
                 \x20 btn.addEventListener('click', function() {\n\
                 \x20   if (window.gtag) window.gtag('event', 'daily_challenge_click', { entering: !isDaily });\n\
                 \x20 });\n\
                 })();"
            )))
        }
    }
}

const SHARE_BTN_CSS: &str = "\
#share-btn { display: none; position: fixed; bottom: 14px; left: 14px; z-index: 10; \
padding: 0 14px; height: 40px; border-radius: 20px; border: none; \
background: rgba(255,255,255,0.15); color: #fff; font: 14px system-ui, sans-serif; \
line-height: 40px; text-align: center; cursor: pointer; }\n\
#share-btn.show { display: block; }\n\
#share-btn:hover { background: rgba(255,255,255,0.28); }";

/// A hidden-until-shown "Share Result" button plus the `env.hcg_offer_share` miniquad
/// plugin that reveals it (called from `control::share_result` when a daily-challenge
/// run ends). Doesn't call `navigator.share()` itself from the plugin function — that
/// runs from inside the wasm frame loop, not from a user gesture, and the Web Share API
/// requires a real click to fire — so the plugin only stashes the text and un-hides the
/// button; the click listener (registered here too, same combined button+script shape as
/// `hotkey_popup`) does the actual share, falling back to a clipboard copy (with a brief
/// "Copied!" label swap) where `navigator.share` isn't available, and fires a
/// `daily_share_click` GA event (`method: "web_share"|"clipboard"`) either way — the
/// interesting funnel signal is "did they actually click share", not just "was the
/// button shown". Must run before `load(...)`, same ordering constraint as the other
/// bridges.
pub fn share_result_bridge() -> Markup {
    html! {
        style { (PreEscaped(SHARE_BTN_CSS)) }
        button id="share-btn" { "📤 Share Result" }
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 var shareText = '';\n\
                 \x20 var btn = document.getElementById('share-btn');\n\
                 \x20 miniquad_add_plugin({\n\
                 \x20   register_plugin: function(importObject) {\n\
                 \x20     importObject.env.hcg_offer_share = function(textPtr, textLen) {\n\
                 \x20       shareText = UTF8ToString(textPtr, textLen);\n\
                 \x20       btn.classList.add('show');\n\
                 \x20     };\n\
                 \x20   },\n\
                 \x20   version: 1,\n\
                 \x20   name: \"hcg_share\"\n\
                 \x20 });\n\
                 \x20 btn.addEventListener('click', function() {\n\
                 \x20   var method = navigator.share ? 'web_share' : 'clipboard';\n\
                 \x20   if (window.gtag) window.gtag('event', 'daily_share_click', { method: method });\n\
                 \x20   if (navigator.share) {\n\
                 \x20     navigator.share({ text: shareText }).catch(function() {});\n\
                 \x20   } else if (navigator.clipboard) {\n\
                 \x20     navigator.clipboard.writeText(shareText).then(function() {\n\
                 \x20       var original = btn.textContent;\n\
                 \x20       btn.textContent = 'Copied!';\n\
                 \x20       setTimeout(function() { btn.textContent = original; }, 1500);\n\
                 \x20     });\n\
                 \x20   }\n\
                 \x20 });\n\
                 })();"
            )))
        }
    }
}

/// `F` or double-click/double-tap toggles fullscreen. Pure page-level JS rather than
/// `macroquad::window::set_fullscreen` (which on WASM calls `canvas.requestFullscreen()`
/// via `mq_js_bundle.js`) — browsers apply `:fullscreen { width: 100%; height: 100% }` as
/// a `!important` UA style, which cannot be overridden from author CSS at any specificity,
/// so fullscreening the canvas directly stomps the pinned native-resolution box
/// `native_size_style` relies on and leaves the game drawing at fixed pixel coordinates
/// into a canvas that's now some unrelated size — the visible symptom was drawing that
/// read as "shrunk"/"downscaled" once fullscreen, and on some pages looked like nothing
/// had happened at all. Fullscreening `<html>` instead leaves the canvas element itself
/// completely unconstrained by the UA style, so its own pinned size and `window.fitCanvas()`
/// scale-to-fit transform keep working unchanged — just re-evaluated against a bigger
/// viewport, which is also why `fullscreenchange` re-runs `window.fitCanvas` (the transition
/// doesn't reliably fire its own `resize` event). Native builds toggle fullscreen from
/// `control::Control` instead, straight through `macroquad::window::set_fullscreen` — no
/// DOM/canvas-target issue there since it's a real OS window, not a styled element.
///
/// The touch path is its own listener rather than relying on `dblclick` firing from a
/// double-tap: iOS Safari treats a double-tap on a plain (non-form, non-link) element as
/// its own double-tap-to-zoom gesture and consumes it before a `dblclick` event ever
/// reaches JS, so double-tap silently did nothing there. Detecting two `touchend`s within
/// 350ms ourselves and calling `preventDefault()` on the second suppresses that native
/// zoom and fires the toggle instead — scoped to the canvas only, so pinch-zoom/
/// double-tap-zoom elsewhere on the page (e.g. the hotkey popup text) is untouched, which
/// matters since disabling zoom via the viewport meta tag site-wide would be an
/// accessibility regression. `webkitRequestFullscreen`/`webkitExitFullscreen`/
/// `webkitFullscreenElement` fall back for older WebKit that predates the unprefixed API.
///
/// The `fullscreenchange`/`webkitfullscreenchange` listener also toggles an
/// `hcg-fullscreen` class on `<html>` (same convention as `stream_mode_query_bridge`'s
/// `stream-mode` class) — `daily_challenge_button` hides itself under it. Deliberately a
/// JS-driven class rather than a plain CSS `:fullscreen`/`:-webkit-full-screen` selector:
/// a plain (non-forgiving) comma-separated selector list is invalid as a whole if any one
/// selector in it is unrecognized, so `:-webkit-full-screen` alone being unsupported in a
/// given browser would have silently dropped the *entire* rule, including the plain
/// `:fullscreen` half — reusing `hcgIsFullscreen()`'s own already-correct cross-browser
/// detection sidesteps that class of bug entirely.
///
/// Also grabs a Screen Wake Lock while fullscreen (feature-detected — silently
/// no-ops on browsers without `navigator.wakeLock`), released on exit. A wake
/// lock is auto-released by the browser whenever the tab goes hidden, with no
/// event fired on re-fullscreen, so `visibilitychange` re-requests it on
/// return rather than relying on the `fullscreenchange` listener alone.
pub fn fullscreen_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "function hcgIsFullscreen() {\n\
                 \x20 return !!(document.fullscreenElement || document.webkitFullscreenElement);\n\
                 }\n\
                 function hcgToggleFullscreen() {\n\
                 \x20 if (hcgIsFullscreen()) {\n\
                 \x20   (document.exitFullscreen || document.webkitExitFullscreen).call(document);\n\
                 \x20 } else {\n\
                 \x20   var el = document.documentElement;\n\
                 \x20   (el.requestFullscreen || el.webkitRequestFullscreen).call(el);\n\
                 \x20 }\n\
                 }\n\
                 var hcgWakeLock = null;\n\
                 function hcgUpdateWakeLock() {\n\
                 \x20 if (!navigator.wakeLock) return;\n\
                 \x20 if (hcgIsFullscreen()) {\n\
                 \x20   navigator.wakeLock.request('screen').then(function(lock) { hcgWakeLock = lock; }).catch(function() {});\n\
                 \x20 } else if (hcgWakeLock) {\n\
                 \x20   hcgWakeLock.release().catch(function() {});\n\
                 \x20   hcgWakeLock = null;\n\
                 \x20 }\n\
                 }\n\
                 document.addEventListener('visibilitychange', function() {\n\
                 \x20 if (document.visibilityState === 'visible') hcgUpdateWakeLock();\n\
                 });\n\
                 document.addEventListener('keydown', function(e) {\n\
                 \x20 if (e.key === 'f' || e.key === 'F') hcgToggleFullscreen();\n\
                 });\n\
                 var hcgCanvas = document.querySelector('canvas');\n\
                 hcgCanvas.addEventListener('dblclick', hcgToggleFullscreen);\n\
                 var hcgLastTouchEnd = 0;\n\
                 hcgCanvas.addEventListener('touchend', function(e) {\n\
                 \x20 var now = Date.now();\n\
                 \x20 if (now - hcgLastTouchEnd <= 350) {\n\
                 \x20   e.preventDefault();\n\
                 \x20   hcgToggleFullscreen();\n\
                 \x20 }\n\
                 \x20 hcgLastTouchEnd = now;\n\
                 }, { passive: false });\n\
                 ['fullscreenchange', 'webkitfullscreenchange'].forEach(function(ev) {\n\
                 \x20 document.addEventListener(ev, function() {\n\
                 \x20   if (typeof window.fitCanvas === 'function') window.fitCanvas();\n\
                 \x20   document.documentElement.classList.toggle('hcg-fullscreen', hcgIsFullscreen());\n\
                 \x20   hcgUpdateWakeLock();\n\
                 \x20 });\n\
                 });"
            )))
        }
    }
}

/// Wraps `window.AudioContext`/`webkitAudioContext` so whatever instance quad-snd's own
/// (bundled-into-`mq_js_bundle.js`) audio backend constructs gets captured into
/// `window.__hcgAudioCtx` — nothing on the page has any other way to reach it, since
/// quad-snd never hands the context back out. Must run **before**
/// `script src="../mq_js_bundle.js"`, not after like every other bridge here: it has to
/// already be in place by the time that script (or anything the wasm module later
/// triggers) does `new AudioContext()`. Reading `window.AudioContext` fresh at call time
/// (rather than caching a reference before this runs) is what every real caller does, so
/// wrapping the global is enough regardless of exactly when quad-snd constructs its
/// context. Harmless if no `AudioContext` constructor exists at all (very old browsers) —
/// the wrapper just never gets installed, and `audio_bridge` below already
/// no-ops when `window.__hcgAudioCtx` never got set.
pub fn audio_context_capture_script() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "(function() {\n\
                 \x20 var Orig = window.AudioContext || window.webkitAudioContext;\n\
                 \x20 if (!Orig) return;\n\
                 \x20 function Wrapped() {\n\
                 \x20   var ctx = new Orig(...arguments);\n\
                 \x20   window.__hcgAudioCtx = ctx;\n\
                 \x20   return ctx;\n\
                 \x20 }\n\
                 \x20 Wrapped.prototype = Orig.prototype;\n\
                 \x20 window.AudioContext = Wrapped;\n\
                 \x20 window.webkitAudioContext = Wrapped;\n\
                 })();"
            )))
        }
    }
}

/// Registers a miniquad plugin with the page's two audio hooks, both reading the context
/// `audio_context_capture_script` captured:
///
/// - `env.hcg_set_audio_muted` — the `M` hotkey, called from `control::Control::handle_keys`
///   on every mute toggle. Suspends/resumes (not closes — `AudioContext.close()` is
///   one-way, and quad-snd has no way to be told to build a fresh one) the context, the
///   standard Web Audio mechanism for releasing the audio hardware while a page isn't
///   using it — e.g. so a connected Bluetooth headset isn't held routed to a muted,
///   silent tab. A no-op (both directions) when no context was ever captured.
/// - `env.hcg_audio_running` — polled by `audio::Clip` before each play. Browsers hold a
///   new `AudioContext` suspended until the page gets a click/tap/key press, and no API
///   can ask the visitor for permission instead. While it's suspended this returns 0 and
///   shows `top_banners`' `#sound-hint` (only on pages whose game actually tries to make a sound); the
///   banner hides itself when the context starts running. quad-snd's own bundle already
///   resumes the context on any `mousedown`/`keydown`/`touch*` anywhere on the document,
///   so the banner needs no click handler of its own.
///
/// Hidden under `?embed=1`/`?stream=1` like the rest of the chrome (those modes mute
/// audio anyway, so the hook is never reached). Must run after `mq_js_bundle.js` but
/// before `load(...)`, same ordering constraint as `analytics_bridge`.
pub fn audio_bridge() -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(
                "miniquad_add_plugin({\n\
                 \x20 register_plugin: function(importObject) {\n\
                 \x20   importObject.env.hcg_set_audio_muted = function(muted) {\n\
                 \x20     var ctx = window.__hcgAudioCtx;\n\
                 \x20     if (!ctx) return;\n\
                 \x20     if (muted) ctx.suspend().catch(function() {});\n\
                 \x20     else ctx.resume().catch(function() {});\n\
                 \x20   };\n\
                 \x20   var hintWired = false;\n\
                 \x20   importObject.env.hcg_audio_running = function() {\n\
                 \x20     var ctx = window.__hcgAudioCtx;\n\
                 \x20     // No Web Audio at all: nothing will ever play, so nothing to nag about.\n\
                 \x20     if (!ctx || ctx.state === 'running') return 1;\n\
                 \x20     var hint = document.getElementById('sound-hint');\n\
                 \x20     if (!window.__hcgHide && hint && !hintWired) {\n\
                 \x20       hintWired = true;\n\
                 \x20       hint.classList.add('show');\n\
                 \x20       ctx.addEventListener('statechange', function() {\n\
                 \x20         if (ctx.state === 'running') hint.classList.remove('show');\n\
                 \x20       });\n\
                 \x20     }\n\
                 \x20     return 0;\n\
                 \x20   };\n\
                 \x20 },\n\
                 \x20 version: 1,\n\
                 \x20 name: \"hcg_audio\"\n\
                 });"
            )))
        }
    }
}

pub fn description(name: &str) -> String {
    let title = title(name);
    match name {
        "snake" => "Snake, played by an AI that refuses to corner itself. Watch it clear level after level while your controller gathers dust.".into(),
        "game2048" => "2048, played to the end by an AI. It hoards every tile into one corner with total confidence and no input from you.".into(),
        "klondike" => "Klondike solitaire that deals, plays and wins itself. An AI works through the hand you would have restarted twice.".into(),
        "spider" => "Spider solitaire, played by an AI through one, two and four suits. Ten columns, no undo button, nothing for you to do.".into(),
        "sudoku" => "Sudoku solved by an AI, one certain cell at a time. It fills in the grid you would have penciled in wrong.".into(),
        "arrow-blocks" => "A block puzzle that solves itself. An AI takes apart a heart, a crown and a castle, one arrow at a time.".into(),
        "minesweeper" => "Minesweeper played by an AI that almost never has to guess. Square grids, hex grids, and no flag you placed by mistake.".into(),
        "tetris" => "Tetris, played by an AI that never needs the I-piece. It keeps the stack flat and the lines coming while you sit and watch.".into(),
        "match-3" => "A match-3 puzzle that plays itself. An AI lines up combos you would not have spotted and clears the board without asking.".into(),
        "bubble-shooter" => "Bubble Shooter played by an AI with better aim than yours. It banks shots into gaps you would never have taken.".into(),
        "water-sort" => "Water Sort played by an AI that has never needed an undo. It pours a mess of colors back into order, endlessly.".into(),
        "labyrinth" => "Labyrinth, explored by an AI that can only see one corridor at a time. The map beside it fills in as it goes — watch it work out the whole floor and still take the long way to the exit.".into(),
        _ => format!("Watch an AI play {title} automatically in your browser."),
    }
}

/// Sets up the `dataLayer`/`gtag()` stub eagerly (so early `episode_complete` calls still
/// queue), but defers actually fetching `gtag.js` itself until the first user interaction
/// — it's ~67 KiB of mostly-unused-at-load JS that Lighthouse flags, and a self-playing
/// game doesn't need analytics wired before first paint. Queued `dataLayer` entries are
/// processed by `gtag.js` once it does load. A no-op when `GTAG_ID` is unset locally.
pub fn gtag_head() -> Markup {
    let Ok(gtag_id) = std::env::var("GTAG_ID") else {
        return html! {};
    };
    if gtag_id.is_empty() {
        return html! {};
    }
    html! {
        script {
            (PreEscaped(minify_js(&format!(
                "window.dataLayer = window.dataLayer || [];\n\
                 function gtag(){{dataLayer.push(arguments);}}\n\
                 gtag('js', new Date());\n\
                 gtag('config', '{gtag_id}');\n\
                 function hcgLoadGtag() {{\n\
                 \x20 var s = document.createElement('script');\n\
                 \x20 s.async = true;\n\
                 \x20 s.src = 'https://www.googletagmanager.com/gtag/js?id={gtag_id}';\n\
                 \x20 document.head.appendChild(s);\n\
                 \x20 ['pointerdown', 'keydown', 'touchstart', 'scroll'].forEach(function(e) {{\n\
                 \x20   document.removeEventListener(e, hcgLoadGtag);\n\
                 \x20 }});\n\
                 }}\n\
                 ['pointerdown', 'keydown', 'touchstart', 'scroll'].forEach(function(e) {{\n\
                 \x20 document.addEventListener(e, hcgLoadGtag, {{ once: true, passive: true }});\n\
                 }});",
            ))))
        }
    }
}

/// `<link rel="icon">` tags: the SVG always, plus the rasterized PNG and the
/// apple-touch-icon (see `mise run rasterize`) when present. Falls back to SVG-only
/// locally, where rasterization is skipped without resvg.
///
/// `dist/favicon.ico` deliberately has no tag here — nothing links to it; it exists
/// purely because browsers and Googlebot-Image request that root path unconditionally,
/// and used to get a 404 for it on every crawl.
pub fn favicon_links(base_url: &str, dist: &Path) -> Markup {
    let svg_url = format!("{base_url}favicon.svg");
    let has_png = dist.join("favicon.png").exists();
    let has_apple_touch = dist.join("apple-touch-icon.png").exists();
    html! {
        link rel="icon" href=(svg_url) type="image/svg+xml";
        @if has_png {
            link rel="icon" href=(format!("{base_url}favicon.png")) type="image/png" sizes="192x192";
        }
        @if has_apple_touch {
            link rel="apple-touch-icon" href=(format!("{base_url}apple-touch-icon.png")) sizes="180x180";
        }
    }
}

/// `<link rel="manifest">` + `<meta name="theme-color">`, shared by every generated page
/// (homepage and each game). Each page writes its own `manifest.webmanifest` alongside
/// its `index.html` (see `manifest_json`) — a game installs as its own home-screen app,
/// separate from the homepage's, matching aiideas.md's "PWA / installable" idea.
pub fn pwa_head(theme_color: &str) -> Markup {
    html! {
        link rel="manifest" href="manifest.webmanifest";
        meta name="theme-color" content=(theme_color);
    }
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Builds a page's `manifest.webmanifest` contents. `start_url`/`scope` are both "./" —
/// each generated page (homepage, or a game's own `dist/<name>/`) is installed as an
/// independent app scoped to just that directory. `icon_dir` is the relative path back to
/// wherever `favicon.svg`/`favicon.png`/`icon-512.png`/`icon-512-maskable.png` live
/// (`dist/`'s root) — "" from the homepage itself, "../" from a game one level down. The
/// raster sizes are included only when present, same fallback pattern as
/// `favicon_links`/`social_image` (skipped locally without resvg). The maskable icon
/// (`static/favicon-maskable.svg` — safe-zone-padded + opaque background, unlike the
/// edge-to-edge `favicon.svg`) declares `"purpose":"maskable"` so Android/adaptive-icon
/// installs don't letterbox-crop the artwork — Lighthouse's `maskable-icon` PWA audit
/// checks for exactly this.
pub fn manifest_json(
    dist: &Path,
    title: &str,
    description: &str,
    theme_color: &str,
    icon_dir: &str,
) -> String {
    let mut icons = vec![format!(
        r#"{{"src":"{icon_dir}favicon.svg","sizes":"any","type":"image/svg+xml"}}"#
    )];
    if dist.join("favicon.png").exists() {
        icons.push(format!(
            r#"{{"src":"{icon_dir}favicon.png","sizes":"192x192","type":"image/png"}}"#
        ));
    }
    if dist.join("icon-512.png").exists() {
        icons.push(format!(
            r#"{{"src":"{icon_dir}icon-512.png","sizes":"512x512","type":"image/png"}}"#
        ));
    }
    if dist.join("icon-512-maskable.png").exists() {
        icons.push(format!(
            r#"{{"src":"{icon_dir}icon-512-maskable.png","sizes":"512x512","type":"image/png","purpose":"maskable"}}"#
        ));
    }
    format!(
        r##"{{"name":"{}","short_name":"{}","description":"{}","start_url":"./","scope":"./","display":"standalone","background_color":"#000000","theme_color":"{theme_color}","orientation":"any","icons":[{}]}}"##,
        json_escape(title),
        json_escape(title),
        json_escape(description),
        icons.join(",")
    )
}

/// Registers the shared `dist/sw.js` (see `static/sw.js`) for offline static-asset
/// caching. Deferred to `load` so it doesn't compete with the game's own WASM fetch for
/// bandwidth/priority on first visit. `sw_path` is relative to the page doing the
/// registering — "./sw.js" from the homepage, "../sw.js" from a game page. A service
/// worker's default scope (when not passed explicitly) is resolved from *its own* script
/// location, not the registering page's — since `sw.js` always lives at `dist/`'s root,
/// every page ends up registering the same single worker at the same site-wide scope,
/// regardless of which relative path reached it. That's fine here: caching is per-URL
/// inside the worker's fetch handler (see `static/sw.js`), so each app's own assets still
/// get cached and served offline independently even though one worker controls the whole
/// site.
pub fn sw_register_bridge(sw_path: &str) -> Markup {
    html! {
        script {
            (PreEscaped(minify_js(&format!(
                "if ('serviceWorker' in navigator) {{\n\
                 \x20 window.addEventListener('load', function() {{\n\
                 \x20   navigator.serviceWorker.register('{sw_path}');\n\
                 \x20 }});\n\
                 }}"
            ))))
        }
    }
}

pub struct SocialImage {
    pub url: String,
    pub twitter_card: &'static str,
}

/// Absolute URL to this game's gameplay clip, if `mise run clip` has produced one for it
/// — clips are opt-in per game (see the explicit game list in `mise.toml`'s `deploy`
/// task), so most games don't have one yet and this returns `None` for them.
pub fn social_video(base_url: &str, dist: &Path, name: &str) -> Option<String> {
    let rel = format!("{name}/clip.mp4");
    dist.join(&rel).exists().then(|| format!("{base_url}{rel}"))
}

/// Picks the best available image for `og:image`/`twitter:image`: a real in-game
/// screenshot (see `mise run screenshot`) beats the rasterized favicon, which beats the
/// bare favicon SVG that most crawlers won't render. All fall back locally, where those
/// build steps are skipped without xvfb-run/resvg.
pub fn social_image(base_url: &str, dist: &Path, preview: Option<&str>) -> SocialImage {
    if let Some(preview) = preview
        && dist.join(preview).exists()
    {
        return SocialImage {
            url: format!("{base_url}{preview}"),
            twitter_card: "summary_large_image",
        };
    }
    if dist.join("favicon.png").exists() {
        return SocialImage {
            url: format!("{base_url}favicon.png"),
            twitter_card: "summary",
        };
    }
    SocialImage {
        url: format!("{base_url}favicon.svg"),
        twitter_card: "summary",
    }
}

/// `application/ld+json` structured data for a game page: a `VideoGame` node plus a
/// `BreadcrumbList` (Home → this game), as one `@graph` script — Google's preferred shape
/// over multiple `ld+json` tags on one page. Deliberately omits `genre`: several games on
/// this site aren't really "Puzzle" (Snake, Tetris, Bubble Shooter) and guessing a genre
/// per game isn't worth the risk of a wrong one; `applicationCategory: "Game"` is accurate
/// for all of them instead. `image_url` is the caller's own already-computed
/// `social_image(...).url` — passed in rather than recomputed, so the favicon/screenshot
/// fallback chain only runs once per page. Every URL passed in must already be absolute.
pub fn game_json_ld(
    base_url: &str,
    title: &str,
    description: &str,
    page_url: &str,
    image_url: &str,
) -> Markup {
    let title = json_escape(title);
    let description = json_escape(description);
    let page_url = json_escape(page_url);
    let image_url = json_escape(image_url);
    let base_url = json_escape(base_url);
    let json = format!(
        r#"{{"@context":"https://schema.org","@graph":[{{"@type":"VideoGame","name":"{title}","description":"{description}","url":"{page_url}","image":"{image_url}","applicationCategory":"Game","operatingSystem":"Web Browser","browserRequirements":"Requires WebAssembly and WebGL","isAccessibleForFree":true,"playMode":"SinglePlayer","publisher":{{"@type":"Organization","name":"Hotel Chair Games","url":"{base_url}"}}}},{{"@type":"BreadcrumbList","itemListElement":[{{"@type":"ListItem","position":1,"name":"Hotel Chair Games","item":"{base_url}"}},{{"@type":"ListItem","position":2,"name":"{title}","item":"{page_url}"}}]}}]}}"#
    );
    html! {
        script type="application/ld+json" {
            (PreEscaped(json))
        }
    }
}

/// `application/ld+json` for the ambient wall: `CollectionPage` + an `ItemList` naming every
/// game in grid order. That pairing is the honest description of what the page is — a list
/// of the site's games, not a game itself — and it's the one page where an `ItemList` is
/// accurate, which is why neither the homepage nor a game page emits one. `games` is
/// `(directory name, display title)` pairs; each item's `url` is built from the base URL, so
/// the list doubles as a machine-readable version of the tile links.
pub fn wall_json_ld(base_url: &str, games: &[(String, String)]) -> Markup {
    let items = games
        .iter()
        .enumerate()
        .map(|(i, (name, title))| {
            format!(
                r#"{{"@type":"ListItem","position":{},"name":"{}","url":"{}{name}/"}}"#,
                i + 1,
                json_escape(title),
                json_escape(base_url),
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let base = json_escape(base_url);
    let json = format!(
        r#"{{"@context":"https://schema.org","@graph":[{{"@type":"CollectionPage","name":"Ambient Wall — Hotel Chair Games","url":"{base}wall/","description":"Every self-playing game on this site running at once, one AI per tile.","isPartOf":{{"@type":"WebSite","name":"Hotel Chair Games","url":"{base}"}}}},{{"@type":"ItemList","name":"Self-playing games on Hotel Chair Games","numberOfItems":{},"itemListElement":[{items}]}}]}}"#,
        games.len()
    );
    html! {
        script type="application/ld+json" {
            (PreEscaped(json))
        }
    }
}

/// `application/ld+json` for the homepage: a `WebSite` node plus the standalone
/// `Organization` node — `name`/`url`/`logo` on the latter are what Google actually uses to
/// build a Knowledge Graph brand entity, so those three fields are the point of this
/// function. `logo` follows the same exists-or-skip PNG-over-SVG fallback
/// `favicon_links`/`social_image` already use, since a bare SVG isn't reliably rendered by
/// every consumer of this data the way a raster PNG is.
pub fn homepage_json_ld(base_url: &str, dist: &Path, description: &str) -> Markup {
    let logo_url = if dist.join("favicon.png").exists() {
        format!("{base_url}favicon.png")
    } else {
        format!("{base_url}favicon.svg")
    };
    let base_url = json_escape(base_url);
    let description = json_escape(description);
    let logo_url = json_escape(&logo_url);
    let json = format!(
        r#"{{"@context":"https://schema.org","@graph":[{{"@type":"WebSite","name":"Hotel Chair Games","url":"{base_url}","description":"{description}","publisher":{{"@type":"Organization","name":"Hotel Chair Games","url":"{base_url}","logo":"{logo_url}"}}}},{{"@type":"Organization","name":"Hotel Chair Games","url":"{base_url}","logo":"{logo_url}"}}]}}"#
    );
    html! {
        script type="application/ld+json" {
            (PreEscaped(json))
        }
    }
}
