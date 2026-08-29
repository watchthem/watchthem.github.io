//! Menu-only visuals for the native standalone shell (see `shell.rs`): the per-game
//! preview thumbnails and the hero scene reused from the homepage. Kept out of `shell.rs`
//! itself so that file stays about the menu's state machine/input, not asset loading.

use macroquad::prelude::*;

/// Each game's already-generated site preview screenshot — the same PNG a game's own
/// page uses for its social-share image (see `xtask::social_image`), reused here rather
/// than capturing a separate native-only screenshot. **This means `bundle`'s native build
/// depends on the site build having run first** to get the *real* art: `build.rs` stages
/// `dist/<game>/preview.png` into `OUT_DIR` for each game, substituting a placeholder
/// when the real file doesn't exist yet (fresh clone, `mise run clean`) — so `cargo
/// check`/clippy/`mise run run-bundle` never fail to compile over this, they just show
/// placeholder art natively until `mise run deploy` (or `build-wasm <game>`) has run. See
/// `build.rs` and root `CLAUDE.md`'s "Native standalone shell" section.
const PREVIEW_BYTES: [(&str, &[u8]); 12] = [
    (
        "arrow-blocks",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_arrow-blocks.png")),
    ),
    (
        "bubble-shooter",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_bubble-shooter.png")),
    ),
    (
        "game2048",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_game2048.png")),
    ),
    (
        "klondike",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_klondike.png")),
    ),
    (
        "labyrinth",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_labyrinth.png")),
    ),
    (
        "match-3",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_match-3.png")),
    ),
    (
        "minesweeper",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_minesweeper.png")),
    ),
    (
        "snake",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_snake.png")),
    ),
    (
        "spider",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_spider.png")),
    ),
    (
        "sudoku",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_sudoku.png")),
    ),
    (
        "tetris",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_tetris.png")),
    ),
    (
        "water-sort",
        include_bytes!(concat!(env!("OUT_DIR"), "/preview_water-sort.png")),
    ),
];

/// One decoded texture per `crate::GAME_NAMES` entry, same order — looked up by name
/// rather than assumed positional, so a drift between `PREVIEW_BYTES`' own order and
/// `GAME_NAMES` can't silently show the wrong thumbnail on a tile.
pub fn load_previews() -> Vec<Texture2D> {
    crate::GAME_NAMES
        .iter()
        .map(|name| {
            let bytes = PREVIEW_BYTES
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, b)| *b)
                .unwrap_or_else(|| panic!("no preview.png embedded for game '{name}'"));
            Texture2D::from_file_with_format(bytes, Some(ImageFormat::Png))
        })
        .collect()
}

/// Draws `tex` scaled+cropped to fill `dest` without distortion (CSS `object-fit: cover`
/// — crop the longer axis rather than letterbox or stretch), since the 11 games' preview
/// screenshots span three different aspect ratios (900x720, 600x720, 500x610) but every
/// menu tile is the same shape.
pub fn draw_cover(tex: &Texture2D, dest: Rect) {
    let (tw, th) = (tex.width(), tex.height());
    let tex_aspect = tw / th;
    let dest_aspect = dest.w / dest.h;
    let source = if tex_aspect > dest_aspect {
        let sw = th * dest_aspect;
        Rect::new((tw - sw) * 0.5, 0.0, sw, th)
    } else {
        let sh = tw / dest_aspect;
        Rect::new(0.0, (th - sh) * 0.5, tw, sh)
    };
    draw_texture_ex(
        tex,
        dest.x,
        dest.y,
        WHITE,
        DrawTextureParams {
            dest_size: Some(vec2(dest.w, dest.h)),
            source: Some(source),
            ..Default::default()
        },
    );
}

/// The homepage hero scene (`static/hotel-scene.svg`) — the *pre-dissolve* hotel-room
/// frame of the JS canvas animation that runs there (dither-dissolve armchair -> gaming
/// chair morph, hue-cycling neon, sheared swivel; see `xtask/src/bin/generate_index.rs`'s
/// `HOTEL_SCENE_SCRIPT` doc comment for the full animation, which is web-only pixel
/// canvas code with no macroquad equivalent worth building — this menu reuses the *scene*,
/// not the animation). The SVG is nothing but ~100 flat `<rect>` fills on an 80x60 grid
/// (`shape-rendering="crispEdges"`, no curves, no gradients, no groups/transforms) — small
/// and regular enough to hand-parse here rather than pull in an SVG-rendering dependency
/// (or a rasterizer, which would also reintroduce the "needs `resvg`" build dependency
/// `mise run rasterize` already has to guard against for the web build's favicon/OG image).
/// Parsed once at shell startup and redrawn as plain rectangles every menu frame — same
/// "no `RenderCache`" choice as the rest of the menu, and for the same reason: cheap
/// enough (~100 draw calls) that caching would add complexity without a measurable win.
pub struct HeroScene {
    rects: Vec<(Rect, Color)>,
}

const HERO_SVG: &str = include_str!("../../static/hotel-scene.svg");
/// The SVG's own `viewBox` — the 80x60 grid every rect's `x`/`y`/`width`/`height` is in.
const HERO_W: f32 = 80.0;
const HERO_H: f32 = 60.0;

impl HeroScene {
    pub fn load() -> Self {
        let rects = HERO_SVG
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                if !line.starts_with("<rect") {
                    return None;
                }
                let x: f32 = attr(line, "x")?.parse().ok()?;
                let y: f32 = attr(line, "y")?.parse().ok()?;
                let w: f32 = attr(line, "width")?.parse().ok()?;
                let h: f32 = attr(line, "height")?.parse().ok()?;
                let color = parse_color(attr(line, "fill")?)?;
                Some((Rect::new(x, y, w, h), color))
            })
            .collect();
        Self { rects }
    }

    /// Draws the scene scaled uniformly by `scale` (source 80x60 units -> `scale` px each)
    /// with its top-left corner at `(x, y)`.
    pub fn draw(&self, x: f32, y: f32, scale: f32) {
        for (r, color) in &self.rects {
            draw_rectangle(
                x + r.x * scale,
                y + r.y * scale,
                r.w * scale,
                r.h * scale,
                *color,
            );
        }
    }

    /// `(width, height)` at the given `scale` (source 80x60 units -> `scale` px each) —
    /// what `draw`'s footprint will be, for callers laying out content next to it.
    pub fn size_at(&self, scale: f32) -> (f32, f32) {
        (HERO_W * scale, HERO_H * scale)
    }
}

/// `name="value"` attribute lookup within one SVG element's source line — good enough for
/// `HERO_SVG`'s fixed, single-line-per-element, always-double-quoted shape; not a general
/// SVG/XML attribute parser.
fn attr<'a>(line: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("{name}=\"");
    let start = line.find(&needle)? + needle.len();
    let end = start + line[start..].find('"')?;
    Some(&line[start..end])
}

/// `#rrggbb` or `rgba(r, g, b, a)` — the two fill formats `HERO_SVG` uses.
fn parse_color(s: &str) -> Option<Color> {
    if let Some(hex) = s.strip_prefix('#') {
        let r = u8::from_str_radix(hex.get(0..2)?, 16).ok()?;
        let g = u8::from_str_radix(hex.get(2..4)?, 16).ok()?;
        let b = u8::from_str_radix(hex.get(4..6)?, 16).ok()?;
        return Some(Color::new(
            r as f32 / 255.0,
            g as f32 / 255.0,
            b as f32 / 255.0,
            1.0,
        ));
    }
    let inner = s.strip_prefix("rgba(")?.strip_suffix(')')?;
    let mut parts = inner.split(',').map(|p| p.trim());
    let r: f32 = parts.next()?.parse().ok()?;
    let g: f32 = parts.next()?.parse().ok()?;
    let b: f32 = parts.next()?.parse().ok()?;
    let a: f32 = parts.next()?.parse().ok()?;
    Some(Color::new(r / 255.0, g / 255.0, b / 255.0, a))
}
