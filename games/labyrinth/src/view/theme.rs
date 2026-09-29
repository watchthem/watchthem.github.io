//! Procedural wall art. One `Texture2D` per theme, generated into an `Image` at
//! startup (no filesystem on WASM, and `include_bytes!` PNGs would eat the wasm
//! budget), `FilterMode::Nearest`. The grain is hash-noise seeded by `HCG_SEED`, not
//! `macroquad::rand` — so building the atlas never perturbs the maze RNG stream.
//!
//! Phase 4 ships one wall texture + a palette (ceiling/floor/fog) per theme. Per-face
//! decals and the fx hookup are later phases.

use crate::view::fog::Mist;
use macroquad::prelude::*;

pub const TEX: u16 = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Backrooms,
    Dungeon,
    Forest,
    Garden,
}

impl ThemeKind {
    // Native-only: the `HCG_LABYRINTH_THEME` override and the startup log.
    #[cfg(not(target_arch = "wasm32"))]
    pub const ALL: [ThemeKind; 4] = [
        ThemeKind::Backrooms,
        ThemeKind::Dungeon,
        ThemeKind::Forest,
        ThemeKind::Garden,
    ];

    /// Each mode owns one look — the theme is how a viewer tells the modes apart, so
    /// it never varies independently of the mode.
    pub fn for_mode(mode: crate::Mode) -> ThemeKind {
        // Dev override for screenshotting a specific theme without playing to that gen.
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(name) = std::env::var("HCG_LABYRINTH_THEME")
            && let Some(t) = Self::ALL
                .iter()
                .find(|t| t.label().eq_ignore_ascii_case(&name))
        {
            return *t;
        }
        match mode {
            crate::Mode::TowerClimb => ThemeKind::Backrooms,
            crate::Mode::DungeonCrawl => ThemeKind::Dungeon,
            crate::Mode::Fog => ThemeKind::Forest,
            crate::Mode::Mirror => ThemeKind::Garden,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Backrooms => "Backrooms",
            ThemeKind::Dungeon => "Dungeon",
            ThemeKind::Forest => "Forest",
            ThemeKind::Garden => "Garden",
        }
    }
}

#[derive(Clone, Copy)]
pub struct Palette {
    pub ceil_top: Color,
    pub ceil_bot: Color,
    pub floor_near: Color,
    pub floor_far: Color,
    pub fog: Color,
    /// Multiplied into E/W faces so corners read (N/S faces stay at 1.0).
    pub ew_tint: f32,
    /// Distance-falloff multiplier. 1.0 = plain darkness; Fog mode's forest is well
    /// above it so corridors dissolve into pale mist a few cells out.
    pub haze: f32,
    /// Open-air mist (the Forest): everything out there fades on `fog::Mist` instead of
    /// the `haze` curve — exponential, thicker near the ground, into a colour that
    /// depends on where the ray points. `haze` ≥ `fog::MIST_HAZE` marks it for the
    /// shared `raycast::lit_at` callers; `fog` is the mist's colour just below level.
    pub mist: Option<Mist>,
}

/// A theme's ceiling (`Theme::ceiling`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ceiling {
    /// Outdoors: the palette gradient is the sky.
    Sky,
    /// The dungeon's vault: flagstones — a bare dark gradient read as night sky.
    Stone,
    /// The office's drop ceiling: a grid of acoustic tiles with the light panels set
    /// in it — a flat gradient read as a painted backdrop.
    Tiles,
}

pub struct Theme {
    pub kind: ThemeKind,
    pub wall: Texture2D,
    pub palette: Palette,
    /// The forest: walls aren't drawn as faces at all — every solid square of the
    /// maze is planted with trees instead (`raycast::draw_trees`).
    pub trees: bool,
    /// The garden: hedges are clipped foliage volumes (a recessed core + leaf clumps).
    pub bushes: bool,
    /// What's overhead indoors, cast as perspective tiles over the palette gradient
    /// (`raycast::draw_ceiling`), which becomes the joints showing between them.
    pub ceiling: Ceiling,
    /// Outdoor billboard art: spruces (Forest) or hedge leaf clumps (Garden).
    pub sprites: Option<Sprites>,
    /// Outdoors: a textured ground plane (and, in the Garden, open country to the
    /// horizon) instead of the flat floor/ceiling gradients.
    pub ground: Option<Ground>,
}

impl ThemeKind {
    /// Under a roof: no drifting motes (they read as outdoor pollen/snow).
    pub fn indoors(self) -> bool {
        matches!(self, ThemeKind::Backrooms | ThemeKind::Dungeon)
    }
}

impl Theme {
    pub fn build(kind: ThemeKind, seed: u64) -> Theme {
        let mut img = Image::gen_image_color(TEX, TEX, BLACK);
        match kind {
            ThemeKind::Backrooms => paint_backrooms(&mut img, seed),
            ThemeKind::Dungeon => paint_dungeon(&mut img, seed),
            ThemeKind::Forest => paint_forest(&mut img, seed),
            ThemeKind::Garden => paint_garden(&mut img, seed),
        }
        let wall = Texture2D::from_image(&img);
        if kind == ThemeKind::Garden {
            // Fine leaves: filtered and mipmapped, or a far hedge shimmers.
            wall.set_filter(FilterMode::Linear);
            mipmap(&wall, true);
        } else {
            wall.set_filter(FilterMode::Nearest);
        }
        Theme {
            kind,
            wall,
            palette: palette_for(kind),
            trees: kind == ThemeKind::Forest,
            bushes: kind == ThemeKind::Garden,
            ceiling: match kind {
                ThemeKind::Dungeon => Ceiling::Stone,
                ThemeKind::Backrooms => Ceiling::Tiles,
                ThemeKind::Forest | ThemeKind::Garden => Ceiling::Sky,
            },
            sprites: sprites_for(kind, seed),
            ground: ground_for(kind, seed),
        }
    }
}

/// The theme's foliage sprites. Painting them takes tens of ms (more on WASM), and a
/// theme is rebuilt for every maze with the same art seed, so they're painted once per
/// (kind, seed) and the texture handle reused after that.
fn sprites_for(kind: ThemeKind, seed: u64) -> Option<Sprites> {
    thread_local! {
        static CACHE: std::cell::RefCell<Vec<(ThemeKind, u64, Sprites)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    if let Some(hit) = CACHE.with_borrow(|c| {
        c.iter()
            .find(|(k, s, _)| *k == kind && *s == seed)
            .map(|e| e.2.clone())
    }) {
        return Some(hit);
    }
    let sprites = match kind {
        ThemeKind::Forest => Sprites::new(
            (0..SPRUCES).map(|v| paint_spruce(seed ^ ((v as u64 + 1) * 0x51_7CC1))),
            [0.0; 3],
            true,
        ),
        ThemeKind::Garden => Sprites::new(
            (0..CLUMPS)
                .map(|v| paint_clump(seed ^ ((v as u64 + 1) * 0x2F_1A55), v == 3))
                .chain((0..BLOOM_KINDS * BLOOM_ALTS).map(|v| {
                    let s = seed ^ ((v as u64 + 1) * 0x7B_10C5);
                    let mut c = paint_clump(s, false);
                    paint_blossoms(&mut c, s, v / BLOOM_ALTS);
                    c
                })),
            [0.04, 0.11, 0.05],
            false,
        ),
        _ => return None,
    };
    CACHE.with_borrow_mut(|c| c.push((kind, seed, sprites.clone())));
    Some(sprites)
}

fn palette_for(kind: ThemeKind) -> Palette {
    match kind {
        ThemeKind::Backrooms => Palette {
            // The drop ceiling's T-bar joints (`raycast::draw_office_ceiling`), fading
            // to the fog at the horizon like the tiles do.
            ceil_top: c(0.60, 0.57, 0.40),
            ceil_bot: c(0.42, 0.40, 0.26),
            floor_near: c(0.55, 0.50, 0.32),
            floor_far: c(0.30, 0.28, 0.18),
            fog: c(0.42, 0.40, 0.26),
            ew_tint: 0.82,
            haze: 1.0,
            mist: None,
        },
        ThemeKind::Dungeon => Palette {
            ceil_top: c(0.05, 0.045, 0.04),
            ceil_bot: c(0.03, 0.028, 0.026),
            floor_near: c(0.22, 0.20, 0.19),
            floor_far: c(0.05, 0.05, 0.06),
            fog: c(0.04, 0.04, 0.05),
            ew_tint: 0.70,
            haze: 1.0,
            mist: None,
        },
        // The sky and fog colours follow `fog::FOREST` (zenith / horizon / just below).
        ThemeKind::Forest => Palette {
            ceil_top: c(0.80, 0.84, 0.85),
            ceil_bot: c(0.72, 0.76, 0.74),
            floor_near: c(0.40, 0.44, 0.34),
            floor_far: c(0.56, 0.61, 0.55),
            fog: c(0.70, 0.74, 0.71),
            ew_tint: 0.74,
            haze: 4.0,
            mist: Some(crate::view::fog::FOREST),
        },
        ThemeKind::Garden => Palette {
            ceil_top: c(0.52, 0.67, 0.79),
            ceil_bot: c(0.62, 0.75, 0.84),
            floor_near: c(0.30, 0.37, 0.22),
            floor_far: c(0.30, 0.40, 0.29),
            fog: c(0.30, 0.42, 0.30),
            ew_tint: 0.80,
            haze: 1.0,
            mist: None,
        },
    }
}

// ── hash noise (no global RNG) ─────────────────────────────────────────────────

fn hash2(x: i32, y: i32, seed: u64) -> f32 {
    let mut h = seed ^ 0x9E3779B97F4A7C15;
    h ^= (x as i64 as u64).wrapping_mul(0xD1B54A32D192ED03);
    h ^= (y as i64 as u64).wrapping_mul(0xABC98388FB8FAC03);
    h = h.wrapping_mul(0x2545F4914F6CDD1D);
    h ^= h >> 29;
    (h & 0xFFFFFF) as f32 / 0xFFFFFF as f32
}

/// Bilinear value noise at grid scale `cells` across the tile.
fn vnoise(u: f32, v: f32, cells: f32, seed: u64) -> f32 {
    let (fx, fy) = (u * cells, v * cells);
    let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
    let (tx, ty) = (fx - x0 as f32, fy - y0 as f32);
    let (sx, sy) = (smooth(tx), smooth(ty));
    let a = hash2(x0, y0, seed);
    let b = hash2(x0 + 1, y0, seed);
    let cc = hash2(x0, y0 + 1, seed);
    let d = hash2(x0 + 1, y0 + 1, seed);
    lerp(lerp(a, b, sx), lerp(cc, d, sx), sy)
}

fn fbm(u: f32, v: f32, seed: u64) -> f32 {
    let mut sum = 0.0;
    let mut amp = 0.5;
    let mut freq = 4.0;
    for o in 0..4 {
        sum += amp * vnoise(u, v, freq, seed ^ (o as u64 * 0x1234567));
        amp *= 0.5;
        freq *= 2.0;
    }
    sum.clamp(0.0, 1.0)
}

fn smooth(t: f32) -> f32 {
    t * t * (3.0 - 2.0 * t)
}
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}
fn c(r: f32, g: f32, b: f32) -> Color {
    Color::new(r, g, b, 1.0)
}
fn shade(base: Color, k: f32) -> Color {
    Color::new(base.r * k, base.g * k, base.b * k, 1.0)
}

fn fill(img: &mut Image, f: impl Fn(f32, f32) -> Color) {
    for y in 0..TEX {
        for x in 0..TEX {
            let u = x as f32 / TEX as f32;
            let v = y as f32 / TEX as f32;
            img.set_pixel(x as u32, y as u32, f(u, v));
        }
    }
}

// ── per-theme painters ────────────────────────────────────────────────────────

fn paint_backrooms(img: &mut Image, seed: u64) {
    // Over-lit flat yellow-beige, faint ceiling-tile grid, near-zero contrast.
    let base = c(0.79, 0.75, 0.50);
    fill(img, |u, v| {
        let n = (fbm(u, v, seed) - 0.5) * 0.06;
        let grid = if (u * 4.0).fract() < 0.03 || (v * 4.0).fract() < 0.03 {
            -0.05
        } else {
            0.0
        };
        shade(base, 1.0 + n + grid)
    });
}

fn paint_dungeon(img: &mut Image, seed: u64) {
    // Brick courses: offset rows, per-brick value jitter, dark mortar.
    let mortar = c(0.06, 0.055, 0.05);
    let brick = c(0.34, 0.30, 0.28);
    let rows = 6.0;
    let cols = 4.0;
    fill(img, |u, v| {
        let row = (v * rows).floor();
        let offset = if (row as i32) % 2 == 0 { 0.0 } else { 0.5 };
        let cu = (u * cols + offset).fract();
        let cv = (v * rows).fract();
        let m = 0.06;
        if cu < m || cu > 1.0 - m || cv < m || cv > 1.0 - m {
            return shade(mortar, 0.8 + 0.4 * fbm(u, v, seed));
        }
        let bx = (u * cols + offset).floor();
        let jitter = hash2(bx as i32, row as i32, seed) * 0.35 + 0.8;
        let moss = if v > 0.75 {
            vnoise(u, v, 20.0, seed ^ 7) * 0.10
        } else {
            0.0
        };
        Color::new(
            brick.r * jitter,
            brick.g * jitter + moss,
            brick.b * jitter,
            1.0,
        )
    });
}

fn paint_forest(img: &mut Image, seed: u64) {
    // Vertical trunk stripes with bark noise.
    let bark = c(0.20, 0.15, 0.10);
    let trunks = 5.0;
    fill(img, |u, v| {
        let tu = (u * trunks).fract();
        let gap = (tu - 0.5).abs() * 2.0; // 0 center .. 1 edge
        let bark_n = vnoise(u, v, 40.0, seed) * 0.4 + fbm(u * 0.5, v, seed ^ 3) * 0.3;
        let lit = 1.0 - gap * 0.5;
        Color::new(
            (bark.r + bark_n * 0.15) * lit,
            (bark.g + bark_n * 0.18) * lit,
            (bark.b + bark_n * 0.10) * lit,
            1.0,
        )
    });
}

fn paint_garden(img: &mut Image, seed: u64) {
    // A clipped hedge's face up close: a dark tangle under densely packed small leaves,
    // each lit a little differently (brighter ones on top), brighter toward the top
    // (skylight). Tiles both ways, like the ground.
    let n = TEX as usize;
    let mut c = Canvas::new(n, n);
    for y in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
            let k = 0.75 + 0.5 * tnoise(u, v, 16, seed ^ 0x6E);
            c.px[y * n + x] = [0.03 * k, 0.08 * k, 0.035 * k, 1.0];
        }
    }
    let mut rng = Hr(seed ^ 0x0004_ED6E);
    let mut leaves: Vec<(f32, f32, f32, f32, f32, f32)> = (0..1500)
        .map(|_| {
            let a = 2.0 + 1.3 * rng.f();
            (
                rng.f().powf(0.8),
                rng.f() * n as f32,
                rng.f() * n as f32,
                a,
                a * (0.5 + 0.15 * rng.f()),
                rng.f() * std::f32::consts::TAU,
            )
        })
        .collect();
    crate::util::sort_by_key(&mut leaves, |l| l.0);
    for (k, x, y, a, b, ang) in leaves {
        let col = mix3([0.05, 0.14, 0.05], [0.30, 0.50, 0.17], 0.25 + 0.75 * k);
        c.ellipse(true, x, y, a, b, ang, |lx, ly| {
            let rib = 1.0 + 0.12 * (1.0 - ly.abs()) - 0.1 * lx;
            col.map(|v| v * rib)
        });
    }
    for y in 0..n {
        for x in 0..n {
            let p = c.px[y * n + x];
            let sky = 0.82 + 0.3 * (1.0 - y as f32 / n as f32);
            img.set_pixel(
                x as u32,
                y as u32,
                Color::new(p[0] * sky, p[1] * sky, p[2] * sky, 1.0),
            );
        }
    }
}

// ── foliage sprites ───────────────────────────────────────────────────────────

/// Mip levels per sprite: full size, then three successive halvings.
pub const LEVELS: usize = 4;
/// Transparent gutter between atlas tiles, so linear filtering never samples a neighbour.
const GAP: usize = 2;
pub(crate) const SPRUCES: usize = 6;
/// Painted at a spruce's own narrow proportions (drawn about 4 units tall and 1 wide),
/// so `draw_spruce` scales it almost uniformly — squeezing a wider painting sideways
/// turned its branches into hanging strands.
pub(crate) const TREE_W: usize = 96;
pub(crate) const TREE_H: usize = 320;
/// Rows left empty above the spruce's tip and below its ground line.
const TREE_MARGIN: f32 = 4.0;
/// A spruce sprite's full height over the tree's own height (tip to ground).
pub const TREE_SPAN: f32 = TREE_H as f32 / (TREE_H as f32 - 2.0 * TREE_MARGIN);
/// How far the ground line sits above the sprite's bottom edge, as a fraction of it.
pub const TREE_FOOT: f32 = TREE_MARGIN / TREE_H as f32;
/// Plain leaf-clump variants: the Garden atlas's first `CLUMPS` sprites.
pub const CLUMPS: usize = 4;
/// Flowering clumps after them (`bloom_variant`): red roses, white roses, pink, yellow,
/// and roses half painted red — the Queen's gardeners' job left unfinished.
pub const BLOOM_KINDS: usize = 5;
/// Leaf bases painted per bloom kind, so neighbouring flowering clumps differ.
pub const BLOOM_ALTS: usize = 2;
const CLUMP_PX: usize = 64;

/// Atlas variant of a flowering clump: bloom `kind` (< `BLOOM_KINDS`), leaf base `alt`.
pub fn bloom_variant(kind: usize, alt: usize) -> usize {
    CLUMPS + kind % BLOOM_KINDS * BLOOM_ALTS + alt % BLOOM_ALTS
}
/// Radius of a clump's leafy silhouette, in sprite pixels: what its world radius maps to.
const CLUMP_BODY: f32 = 25.0;
/// A clump sprite's half-width over the clump's radius.
pub const CLUMP_SPAN: f32 = CLUMP_PX as f32 * 0.5 / CLUMP_BODY;

/// Billboard art for the outdoor themes: every variant of one foliage sprite in one
/// atlas (one texture bind for hundreds of draws). Each variant is stored twice — in
/// colour, baked at full light, and as a white silhouette with the same alpha. Fog is
/// paler than the leaves, so a multiplicative tint can't fade a sprite into it; the
/// raycaster instead draws the silhouette over the colour, tinted `fog` at `1 - lit`.
/// Every variant also carries a hand-rolled mip chain, so a distant sprite is a
/// downsample rather than a shimmering sparse sampling of the full-size one.
///
/// `premul` (the spruces, in the Forest's mist): the colour row is stored premultiplied
/// and drawn with `fog::premultiplied_material`, which adds the mist in the same pass
/// and fogs soft edges exactly (see there) — the silhouette row goes unused. It replaced
/// drawing a silhouette grown a texel past the colour, blurred, over the tree: that
/// fogged past every edge, a pale halo round each tree against anything darker.
#[derive(Clone)]
pub struct Sprites {
    pub tex: Texture2D,
    pub variants: usize,
    /// Premultiplied atlas: draw it with this material (`None` if the atlas is straight
    /// alpha, or the material failed to build — then default blending, edges a touch dark).
    pub premul: Option<Material>,
    w: usize,
    h: usize,
}

impl Sprites {
    fn new(
        canvases: impl Iterator<Item = Canvas>,
        bleed: [f32; 3],
        premultiplied: bool,
    ) -> Sprites {
        let canvases: Vec<Canvas> = canvases.collect();
        let (w, h) = (canvases[0].w, canvases[0].h);
        let tex = Texture2D::from_image(&atlas(&canvases, bleed, premultiplied));
        tex.set_filter(FilterMode::Linear);
        Sprites {
            tex,
            variants: canvases.len(),
            premul: if premultiplied {
                crate::view::fog::premultiplied_material()
            } else {
                None
            },
            w,
            h,
        }
    }

    /// Width over height of one sprite.
    pub fn aspect(&self) -> f32 {
        self.w as f32 / self.h as f32
    }

    /// The smallest mip level still holding at least one texel per screen pixel for a
    /// sprite drawn `px` tall.
    pub fn level_for(&self, px: f32) -> usize {
        let mut l = 0;
        while l + 1 < LEVELS && (self.h >> (l + 1)) as f32 >= px {
            l += 1;
        }
        l
    }

    /// A continuous `level_for`, for blending two levels (trilinear-style) instead of
    /// popping between them: the finer level and the weight of the next coarser one,
    /// for a sprite drawn `px` tall. Asking for fewer px than it's drawn blurs it.
    pub fn level_mix(&self, px: f32) -> (usize, f32) {
        let lf = (self.h as f32 / px.max(1.0))
            .log2()
            .clamp(0.0, (LEVELS - 1) as f32);
        let l = (lf as usize).min(LEVELS - 2);
        (l, lf - l as f32)
    }

    /// Atlas rect of `variant` at mip `level`; `fog` picks its white silhouette.
    pub fn src(&self, variant: usize, level: usize, fog: bool) -> Rect {
        let (bw, bh) = block(self.w, self.h);
        let (x, y, w, h) = level_rect(self.w, self.h, level);
        Rect::new(
            (variant % self.variants * bw + x) as f32,
            (if fog { bh } else { 0 } + y) as f32,
            w as f32,
            h as f32,
        )
    }
}

/// One variant's footprint in the atlas: (column stride, row height).
fn block(w: usize, h: usize) -> (usize, usize) {
    (w + GAP, h + GAP + h / 2 + GAP)
}

/// Where mip `level` sits inside a variant's block: full size on top, the three
/// halvings side by side under it.
fn level_rect(w: usize, h: usize, level: usize) -> (usize, usize, usize, usize) {
    let (lw, lh) = (w >> level, h >> level);
    match level {
        0 => (0, 0, w, h),
        1 => (0, h + GAP, lw, lh),
        2 => (w / 2 + GAP, h + GAP, lw, lh),
        _ => (w / 2 + w / 4 + 2 * GAP, h + GAP, lw, lh),
    }
}

/// Colour row over silhouette row, one block per variant. Straight alpha: fully
/// transparent texels take `bleed` (a dark foliage tone) so linear filtering fringes
/// edges with shadow rather than black. `premultiplied`: colour × alpha, transparent
/// texels all zero — filtering is exact, no bleed needed.
fn atlas(canvases: &[Canvas], bleed: [f32; 3], premultiplied: bool) -> Image {
    let (w, h) = (canvases[0].w, canvases[0].h);
    let (bw, bh) = block(w, h);
    let (aw, ah) = (bw * canvases.len(), bh * 2);
    let mut bytes = vec![0u8; aw * ah * 4];
    for (i, row) in bytes.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        // Transparent everywhere by default, colour row tinted with the bleed tone.
        let rgb = if premultiplied {
            [0.0; 3]
        } else if i / aw < bh {
            bleed
        } else {
            [1.0; 3]
        };
        for k in 0..3 {
            row[k] = (rgb[k] * 255.0) as u8;
        }
    }
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for (v, c0) in canvases.iter().enumerate() {
        let mut level = c0.clone();
        for l in 0..LEVELS {
            let (lx, ly, _, _) = level_rect(w, h, l);
            for y in 0..level.h {
                for x in 0..level.w {
                    let p = level.px[y * level.w + x];
                    if p[3] <= 0.0 {
                        continue;
                    }
                    let at = ((ly + y) * aw + v * bw + lx + x) * 4;
                    let k = if premultiplied { p[3] } else { 1.0 };
                    bytes[at..at + 4].copy_from_slice(&[
                        q(p[0] * k),
                        q(p[1] * k),
                        q(p[2] * k),
                        q(p[3]),
                    ]);
                }
            }
            for y in 0..level.h {
                for x in 0..level.w {
                    let at = ((ly + bh + y) * aw + v * bw + lx + x) * 4;
                    bytes[at + 3] = q(level.px[y * level.w + x][3]);
                }
            }
            level = level.half();
        }
    }
    Image {
        bytes,
        width: aw as u16,
        height: ah as u16,
    }
}

/// Straight-alpha RGBA working buffer for painting a sprite.
#[derive(Clone)]
pub(crate) struct Canvas {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) px: Vec<[f32; 4]>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Canvas {
        Canvas {
            w,
            h,
            px: vec![[0.0; 4]; w * h],
        }
    }

    /// Paint one opaque pixel (soft edges come from the mips and linear filtering).
    fn put(&mut self, x: i32, y: i32, c: [f32; 3]) {
        if x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h {
            self.px[y as usize * self.w + x as usize] = [c[0], c[1], c[2], 1.0];
        }
    }

    #[cfg(test)]
    fn alpha(&self, x: usize, y: usize) -> f32 {
        self.px[y * self.w + x][3]
    }

    /// 2x2 box downsample, averaged in premultiplied alpha.
    pub(crate) fn half(&self) -> Canvas {
        let mut out = Canvas::new((self.w / 2).max(1), (self.h / 2).max(1));
        for y in 0..out.h {
            for x in 0..out.w {
                let mut acc = [0.0f32; 4];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let (sx, sy) = ((2 * x + dx).min(self.w - 1), (2 * y + dy).min(self.h - 1));
                    let p = self.px[sy * self.w + sx];
                    for k in 0..3 {
                        acc[k] += p[k] * p[3];
                    }
                    acc[3] += p[3];
                }
                if acc[3] > 0.0 {
                    out.px[y * out.w + x] = [
                        acc[0] / acc[3],
                        acc[1] / acc[3],
                        acc[2] / acc[3],
                        acc[3] / 4.0,
                    ];
                }
            }
        }
        out
    }
}

/// Splitmix stream for laying out one sprite — private to it, never the game RNG.
struct Hr(u64);

impl Hr {
    fn f(&mut self) -> f32 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        ((z ^ (z >> 31)) >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        lerp(a[0], b[0], t),
        lerp(a[1], b[1], t),
        lerp(a[2], b[2], t),
    ]
}

/// One branch of a spruce, leaving the trunk at canvas point (`x`, `y`). The branch is a
/// 3D curve seen from the side: it runs `len` px out along azimuth `φ`, of which the
/// canvas shows `sx` = sin φ horizontally (±1 = in the picture plane, 0 = pointing
/// straight at or away from the viewer), while `cz` = cos φ is its depth (+ toward the
/// viewer). Its droop is never foreshortened, so a branch pointing at the viewer reads
/// as a short, steep dark mass crossing the trunk.
struct Branch {
    x: f32,
    y: f32,
    sx: f32,
    cz: f32,
    len: f32,
    /// Slope at the root against the horizontal run (+ = down), and the curvature
    /// pulling the tip back up.
    s0: f32,
    curv: f32,
    /// Depth of the needle spray at its fullest, px.
    thick: f32,
    tone: f32,
    /// A self-pruned lower branch: bare grey twigs, a few needle tufts left at most.
    dead: bool,
    seed: u64,
}

impl Branch {
    /// Painter's order: how far toward the viewer the branch's middle sits.
    fn depth(&self) -> f32 {
        self.cz * self.len * 0.5
    }
}

/// A Norway spruce in a dense stand, seen through mist: a narrow, ragged cone on a
/// single thin leader. Branches grow in whorls of 4–6 around the trunk at yearly
/// intervals — irregular ones (a short year, a long one) with smaller internodal
/// branches between — so the silhouette is no left/right comb: some branches lie in the
/// picture plane, most point partly toward or away from the viewer (foreshortened; the
/// ones behind dim, seen through gaps). Length follows the cone from tip down with
/// strong per-branch and per-whorl variation, some broken or missing. Upper branches
/// angle up; lower ones droop from the trunk and turn their tips back up. The lowest
/// crown is sparse and partly dead (self-pruned in the stand's shade), with bare grey
/// twigs below it on a visible trunk. Each live branch is a needle spray
/// (`paint_branch`), deepest near the trunk and thin at the tip, never hanging strands —
/// long thin strands read as tropical lianas. Light is overcast sky: lit upper
/// surfaces, dark undersides and interior, a darker lower crown. Per-tree variety: bare
/// foot, width, taper, lean, whorl rhythm, hue.
pub(crate) fn paint_spruce(seed: u64) -> Canvas {
    use std::f32::consts::TAU;
    let mut c = Canvas::new(TREE_W, TREE_H);
    let mut rng = Hr(seed);
    let base = TREE_H as f32 - TREE_MARGIN;
    let top = TREE_MARGIN;
    let th = base - top;
    let cx = TREE_W as f32 * 0.5;
    let bare = 0.05 + 0.08 * rng.f();
    let crown_base = base - th * bare;
    let crown_h = crown_base - top;
    let hw_max = (cx - 3.0) * (0.85 + 0.15 * rng.f());
    let taper = 0.9 + 0.25 * rng.f();
    let lean = (rng.f() - 0.5) * 3.0;
    let blue = rng.f() * 0.5;
    let warm = rng.f() * 0.4;
    let ramp = spruce_ramp(blue, warm);
    let bark = [0.15, 0.13, 0.11];
    let deadwood = [0.21, 0.19, 0.17];
    let t_at = |y: f32| ((crown_base - y) / crown_h).clamp(0.0, 1.0);
    let axis = |y: f32| cx + lean * ((base - y) / th);
    // The cone, a little pulled in at the very bottom where the crown thins out.
    let env = |t: f32| hw_max * (1.0 - t).powf(taper) * (0.8 + 0.2 * smooth((t / 0.2).min(1.0)));
    // Root slope: swept up at the top, level through the middle, drooping low down.
    let slope = |t: f32, r: f32| lerp(0.7, -0.65, t.powf(1.6)) + (r - 0.5) * 0.6;
    let curl = |s0: f32, r: f32| {
        if s0 > 0.0 {
            -(0.45 + 0.5 * r) * s0 - 0.12
        } else {
            0.12 * -s0
        }
    };
    let tone = |cz: f32| {
        if cz >= 0.0 {
            0.92 + 0.16 * cz
        } else {
            0.92 + 0.32 * cz
        }
    };

    let lead = th * (0.035 + 0.02 * rng.f());
    let mut branches: Vec<Branch> = Vec::new();
    let mut n = 0u64;
    let mut sd = || {
        n += 1;
        seed ^ n.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    };
    // This tree's growth rhythm: mean whorl spacing and how erratic it is.
    let rhythm = crown_h * (0.024 + 0.008 * rng.f());
    let erratic = 0.25 + 0.3 * rng.f();
    let mut y = crown_base;
    while y > top + lead {
        let t = t_at(y);
        let step = rhythm
            * (1.0 - 0.3 * t)
            * match rng.f() {
                r if r < 0.12 => 0.5,
                r if r < 0.22 => 1.6,
                _ => 1.0 + (rng.f() - 0.5) * 2.0 * erratic,
            };
        // Whorl-wide vigour: a good year's whorl reaches further all round.
        let w = env(t) * (0.82 + 0.3 * rng.f());
        let k = 4 + (rng.f() * 3.0) as usize;
        let phi0 = rng.f() * TAU;
        // Low in the crown: branches missing, or dead.
        let (keep, die) = if t < 0.07 {
            (0.6, 0.65)
        } else if t < 0.2 {
            (0.75, 0.2 * (0.2 - t) / 0.13)
        } else {
            (0.86, 0.0)
        };
        let mut push = |rng: &mut Hr, y: f32, len: f32, phi: f32, dead: bool, sd: u64| {
            let (sx, cz) = (phi.sin(), phi.cos());
            let s0 = slope(t, rng.f()) + if dead { 0.35 } else { 0.0 };
            branches.push(Branch {
                x: axis(y),
                y,
                sx,
                cz,
                len,
                s0,
                curv: if dead { -0.1 } else { curl(s0, rng.f()) },
                thick: len * (0.22 + 0.14 * rng.f()) + 2.0,
                tone: tone(cz),
                dead,
                seed: sd,
            });
        };
        for i in 0..k {
            if rng.f() > keep {
                continue;
            }
            let phi = phi0 + (i as f32 + (rng.f() - 0.5) * 0.6) * TAU / k as f32;
            let len = w * match rng.f() {
                r if r < 0.12 => 0.3 + 0.2 * rng.f(), // broken or suppressed
                r if r < 0.25 => 1.0 + 0.15 * rng.f(),
                _ => 0.5 + 0.55 * rng.f(),
            } + 2.0;
            let dead = rng.f() < die;
            let len = if dead {
                len * (0.35 + 0.25 * rng.f())
            } else {
                len
            };
            let yb = y + (rng.f() - 0.5) * 5.0;
            push(&mut rng, yb, len, phi, dead, sd());
        }
        // Internodal branches: short, anywhere between this whorl and the next.
        if t > 0.08 {
            for _ in 0..(rng.f() * 2.6) as usize {
                let yb = y - step * (0.2 + 0.6 * rng.f());
                let len = w * (0.25 + 0.3 * rng.f()) + 1.5;
                let phi = rng.f() * TAU;
                push(&mut rng, yb, len, phi, false, sd());
            }
        }
        y -= step;
    }
    // Top whorls: short, swept up around the leader.
    for i in 0..3 {
        let y = top + lead + i as f32 * 3.0;
        let phi0 = rng.f() * TAU;
        for j in 0..3 {
            let phi = phi0 + j as f32 * TAU / 3.0;
            let len = 3.0 + 2.0 * i as f32 + 2.0 * rng.f();
            branches.push(Branch {
                x: axis(y),
                y,
                sx: phi.sin(),
                cz: phi.cos(),
                len,
                s0: -0.7 + 0.2 * rng.f(),
                curv: 0.1,
                thick: 3.0,
                tone: 1.1,
                dead: false,
                seed: sd(),
            });
        }
    }
    // Dead branches on the bare stretch under the crown, drooping.
    for _ in 0..(3 + (rng.f() * 4.0) as usize) {
        let y = crown_base + (base - crown_base) * 0.7 * rng.f() - 2.0;
        let phi = rng.f() * TAU;
        branches.push(Branch {
            x: axis(y),
            y,
            sx: phi.sin(),
            cz: phi.cos(),
            len: 3.0 + hw_max * 0.25 * rng.f(),
            s0: 0.2 + 0.4 * rng.f(),
            curv: -0.1,
            thick: 3.0,
            tone: 1.0,
            dead: true,
            seed: sd(),
        });
    }
    crate::util::sort_by_key(&mut branches, |b| b.depth());
    let split = branches.partition_point(|b| b.depth() < 0.0);
    for b in &branches[..split] {
        paint_branch(&mut c, b, &ramp, t_at(b.y), deadwood);
    }

    // Dark interior around the trunk: shade and dead inner twigs, thin and broken.
    for y in (top + lead) as i32..=crown_base as i32 {
        let t = t_at(y as f32);
        let hw = env(t) * 0.18 * (0.5 + vnoise(0.5, y as f32 / 320.0, 40.0, seed ^ 5)) + 0.8;
        let ax = axis(y as f32);
        for x in (ax - hw) as i32..=(ax + hw) as i32 {
            if hash2(x, y, seed ^ 6) < 0.7 {
                c.put(x, y, mix3(ramp[0], bark, 0.3 * hash2(x, y, 9)));
            }
        }
    }
    // Trunk.
    for y in top as i32 + 8..=base as i32 {
        let t = (base - y as f32) / th;
        let hw = 0.6 + th * 0.009 * (1.0 - t) + ((0.03 - t) / 0.03).max(0.0) * 2.0;
        let ax = axis(y as f32);
        for x in (ax - hw - 1.0) as i32..=(ax + hw + 1.0) as i32 {
            let s = (x as f32 + 0.5 - ax) / hw;
            if s.abs() > 1.0 {
                continue;
            }
            let k = (0.6 + 0.35 * s) * (0.85 + 0.3 * hash2(x, y / 3, seed ^ 11));
            c.put(x, y, bark.map(|v| v * k));
        }
    }
    for b in &branches[split..] {
        paint_branch(&mut c, b, &ramp, t_at(b.y), deadwood);
    }

    // Leader: a thin straight spire.
    let mut y = top;
    while y < top + lead + 4.0 {
        let ax = axis(y);
        let k = (y - top) / lead;
        c.put(ax as i32, y as i32, ramp[2]);
        if k > 0.4 {
            c.put(
                ax as i32
                    + if hash2(0, y as i32, seed) < 0.5 {
                        1
                    } else {
                        -1
                    },
                y as i32,
                ramp[1],
            );
        }
        y += 1.0;
    }
    c
}

/// Five tones, darkest first: interior, shade, body, lit, highlight. `blue` pulls the
/// greens toward blue spruce, `warm` the lights toward yellow new growth.
fn spruce_ramp(blue: f32, warm: f32) -> [[f32; 3]; 5] {
    [
        [0.020, 0.045 + 0.01 * blue, 0.035 + 0.015 * blue],
        [0.045, 0.095 + 0.01 * blue, 0.065 + 0.03 * blue],
        [0.075 - 0.02 * blue, 0.160, 0.095 + 0.05 * blue],
        [
            0.135 - 0.04 * blue + 0.04 * warm,
            0.245,
            0.125 + 0.07 * blue - 0.03 * warm,
        ],
        [
            0.215 - 0.05 * blue + 0.08 * warm,
            0.330,
            0.160 + 0.08 * blue - 0.05 * warm,
        ],
    ]
}

/// A continuous pick from a tone ramp by a 0..1 light value.
fn ramp_at(ramp: &[[f32; 3]; 5], l: f32) -> [f32; 3] {
    let x = l.clamp(0.0, 0.999) * 4.0;
    let i = x as usize;
    mix3(ramp[i], ramp[(i + 1).min(4)], x - i as f32)
}

/// A 1px stroke, coloured by `col` along it (0 at the start, 1 at the end).
fn sprig_line(c: &mut Canvas, x0: f32, y0: f32, x1: f32, y1: f32, col: impl Fn(f32) -> [f32; 3]) {
    let n = ((x1 - x0).abs().max((y1 - y0).abs()) * 1.5).ceil().max(1.0) as i32;
    for i in 0..=n {
        let f = i as f32 / n as f32;
        c.put(
            lerp(x0, x1, f).floor() as i32,
            lerp(y0, y1, f).floor() as i32,
            col(f),
        );
    }
}

/// One branch as a needle spray along a twig that leaves the trunk, sags and turns its
/// tip back up (`Branch` has the geometry). The spray is deepest a fifth of the way out
/// and thins to the tip. Sprigs are 3D: shoots fanning forward and sideways off the
/// twig, the pendant ones below in shade, the (shorter) ones above lit toward their
/// ends, all projected like the twig — so a branch in the picture plane is a flat pad
/// with a ragged hanging fringe and one pointing at the viewer spreads into a fan. A
/// dense dark core keeps it from reading as loose threads. Dead branches are bare grey
/// twigs with a hanging twiglet or two and at most a few dark needle tufts.
fn paint_branch(c: &mut Canvas, b: &Branch, ramp: &[[f32; 3]; 5], t: f32, deadwood: [f32; 3]) {
    let mut r = Hr(b.seed);
    let pos = |u: f32| {
        (
            b.x + b.sx * b.len * u,
            b.y + b.len * (b.s0 * u + b.curv * u * u),
        )
    };
    let slope = |u: f32| b.s0 + 2.0 * b.curv * u;
    // A 3D direction (along the branch's run, sideways, down) to the canvas.
    let project = |along: f32, lat: f32, down: f32| (along * b.sx + lat * b.cz, down);
    if b.dead {
        let (x1, y1) = pos(1.0);
        let k = 0.8 + 0.3 * r.f();
        sprig_line(c, b.x, b.y, x1, y1, |_| deadwood.map(|v| v * k));
        for _ in 0..(b.len * 0.12) as usize {
            let u = 0.2 + 0.8 * r.f();
            let (px, py) = pos(u);
            let l = 1.5 + b.len * 0.12 * r.f();
            let (dx, dy) = project(0.5, r.f() - 0.5, 0.8);
            sprig_line(c, px, py, px + dx * l, py + dy * l, |_| {
                deadwood.map(|v| v * 0.8)
            });
            if r.f() < 0.35 {
                let tuft = ramp_at(ramp, 0.15);
                c.put(px as i32, py as i32 + 1, tuft);
                c.put(px as i32 + 1, py as i32 + 1, tuft);
            }
        }
        return;
    }
    let prof = |u: f32| (u / 0.2).min(1.0).sqrt() * (1.0 - u).max(0.0).powf(0.9) * 1.2 + 0.06;
    // Second-order branches first, under the main spray, each turned off the twig's
    // azimuth (so foreshortened on its own): they break the spray into clumps and its
    // outline into smaller sprays of the same shape — ragged at every scale.
    if b.len > 10.0 && b.cz > -0.5 {
        let phi = b.sx.atan2(b.cz);
        for _ in 0..2 + (r.f() * 2.0) as usize {
            let u = 0.2 + 0.6 * r.f();
            let (px, py) = pos(u);
            let turn = (0.5 + 0.5 * r.f()) * if r.f() < 0.5 { -1.0 } else { 1.0 };
            let (sx, cz) = ((phi + turn).sin(), (phi + turn).cos());
            let s0 = slope(u) * 0.6 + (r.f() - 0.4) * 0.5;
            paint_branch(
                c,
                &Branch {
                    x: px,
                    y: py,
                    sx,
                    cz,
                    len: b.len * (1.0 - u) * (0.4 + 0.35 * r.f()) + 2.0,
                    s0,
                    curv: -0.3 * s0.max(0.0) - 0.1,
                    thick: b.thick * 0.6,
                    tone: b.tone,
                    dead: false,
                    seed: b.seed ^ r.0,
                },
                ramp,
                t,
                deadwood,
            );
        }
    }
    // Upper crown catches more sky; everything under the crown is darker.
    let lit = b.tone * (0.8 + 0.35 * t);
    // Branches behind are mostly hidden: fewer sprigs.
    let density = if b.cz < -0.3 { 1.5 } else { 2.2 };
    let sprigs = (b.len * density) as usize + 5;
    // Pendant sprigs, then the core, then the lit upper shoots.
    for pass in 0..3 {
        if pass == 1 {
            // Twig and a dense core so the spray doesn't read as loose threads; a branch
            // pointing at the viewer shows its core's width too.
            let n = (b.len * 1.5) as i32;
            for i in 0..=n {
                let u = i as f32 / n as f32;
                let (px, py) = pos(u);
                let half = b.thick * prof(u) * 0.18;
                let wide = b.thick * prof(u) * 0.3 * b.cz.abs();
                let mut yy = py - half;
                while yy <= py + half * 1.4 {
                    let v = (yy - py + half) / (half * 2.4 + 0.01);
                    let col = ramp_at(ramp, (0.36 - 0.18 * v) * lit);
                    let mut xx = px - wide;
                    while xx <= px + wide {
                        c.put(xx as i32, yy as i32, col);
                        xx += 1.0;
                    }
                    yy += 1.0;
                }
            }
            continue;
        }
        for _ in 0..sprigs {
            let u = r.f().powf(0.8);
            let (px, py) = pos(u);
            // Twig tangent in its vertical plane (run, down), and the normal below it.
            let g = slope(u);
            let nrm = (1.0 + g * g).sqrt();
            let (ta, td) = (1.0 / nrm, g / nrm);
            let beta = (0.55 + 0.55 * r.f()) * if pass == 0 { 1.0 } else { -1.0 };
            let (cb, sb) = (beta.cos(), beta.sin());
            let (run, down) = (ta * cb - td * sb, td * cb + ta * sb);
            // Fan the shoot sideways, forward-angled.
            let alpha = (r.f() - 0.5) * 2.2;
            let (dx, dy) = project(run * alpha.cos(), run * alpha.sin(), down);
            let l = b.thick * prof(u) * (0.55 + 0.6 * r.f());
            let (l, shade0, shade1) = if pass == 0 {
                (l * (0.3 + 0.6 * r.f()), 0.18, 0.32)
            } else {
                (l * 0.5, 0.5, 1.0)
            };
            // Light comes in patches: only some shoots catch the sky.
            let catch = if pass == 2 {
                0.55 + 0.6 * vnoise(px / TREE_W as f32, py / TREE_W as f32, 9.0, b.seed ^ 3)
            } else {
                1.0
            };
            let j = 0.12 * (r.f() - 0.5);
            sprig_line(c, px, py, px + dx * l, py + dy * l, |f| {
                ramp_at(ramp, (lerp(shade0, shade1 * catch, f) + j) * lit)
            });
        }
    }
}

/// One leaf clump of a clipped hedge, a `CLUMP_PX` square sprite: a dark inner mass
/// under ~220 small leaves scattered over a disc of radius `CLUMP_BODY`-ish, painted
/// back to front. Clipped, not wild: the shading normal is a sphere's flattened toward
/// the viewer, so a clump reads as a gently domed patch of a sheared surface rather than
/// a ball; each leaf tilts a little on its own and leaves past the rim keep the outline
/// ragged. `blooms` scatters a few small pale flowers over the lit side.
fn paint_clump(seed: u64, blooms: bool) -> Canvas {
    use std::f32::consts::TAU;
    let mut c = Canvas::new(CLUMP_PX, CLUMP_PX);
    let mut rng = Hr(seed);
    let mid = CLUMP_PX as f32 * 0.5;
    let body = CLUMP_BODY - 4.0;
    let sun = vec3(0.45, 0.65, 0.62).normalize();
    let deep = [0.02, 0.07, 0.03];
    let dark = [0.04, 0.12, 0.05];
    let lit = [0.40, 0.64, 0.23];
    for py in 0..CLUMP_PX as i32 {
        for px in 0..CLUMP_PX as i32 {
            let (dx, dy) = (
                (px as f32 + 0.5 - mid) / body,
                (py as f32 + 0.5 - mid) / body,
            );
            let d = (dx * dx + dy * dy).sqrt();
            if d + (hash2(px, py, seed) - 0.5) * 0.3 < 0.95 {
                let nz = (1.0 - d * d).max(0.0).sqrt();
                let nrm = vec3(dx * 0.55, -dy * 0.55, nz + 0.6).normalize();
                let k = 0.3 + 0.5 * nrm.dot(sun).max(0.0);
                c.put(px, py, mix3(deep, dark, k));
            }
        }
    }
    struct Leaf {
        hz: f32,
        x: f32,
        y: f32,
        ang: f32,
        a: f32,
        b: f32,
        rgb: [f32; 3],
    }
    let mut leaves: Vec<Leaf> = (0..220)
        .map(|_| {
            let rho = rng.f().sqrt() * 1.12;
            let th = rng.f() * TAU;
            let (ex, ey) = (rho * th.cos(), rho * th.sin());
            let nz = (1.0 - rho * rho).max(0.0).sqrt();
            let hz = nz + 0.2 * rng.f();
            // Each leaf tilts a little off the sphere's normal.
            let n = vec3(
                ex * 0.5 + (rng.f() - 0.5) * 0.6,
                -ey * 0.5 + (rng.f() - 0.5) * 0.6,
                nz + 0.6,
            )
            .normalize();
            let k = (0.3 + 0.8 * n.dot(sun).max(0.0)) * (0.72 + 0.28 * hz.min(1.0));
            let hue = (rng.f() - 0.5) * 0.08;
            let rgb = mix3(dark, lit, k.min(1.0));
            Leaf {
                hz,
                x: mid + ex * body,
                y: mid + ey * body,
                ang: th + (rng.f() - 0.5) * 1.6,
                a: 1.9 + 1.4 * rng.f(),
                b: 0.0,
                rgb: [rgb[0] + hue, rgb[1], rgb[2] - hue * 0.5],
            }
        })
        .collect();
    for l in &mut leaves {
        l.b = l.a * (0.45 + 0.15 * rng.f());
    }
    crate::util::sort_by_key(&mut leaves, |l| l.hz);
    for l in &leaves {
        let (s, co) = l.ang.sin_cos();
        let r = l.a.ceil() as i32 + 1;
        for py in l.y as i32 - r..=l.y as i32 + r {
            for px in l.x as i32 - r..=l.x as i32 + r {
                let (dx, dy) = (px as f32 + 0.5 - l.x, py as f32 + 0.5 - l.y);
                let (lx, ly) = (dx * co + dy * s, -dx * s + dy * co);
                if (lx / l.a).powi(2) + (ly / l.b).powi(2) > 1.0 {
                    continue;
                }
                // Brighter along the midrib, a touch darker toward the leaf's base.
                let k = (1.0 + 0.12 * (1.0 - ly.abs() / l.b) + 0.08 * lx / l.a)
                    * (0.94 + 0.12 * hash2(px, py, seed ^ 3));
                c.put(px, py, l.rgb.map(|v| v * k));
            }
        }
    }
    if blooms {
        for _ in 0..4 {
            let th = rng.f() * TAU * 0.5 - 2.4; // upper-right half, facing the sun
            let rho = 0.3 + 0.6 * rng.f();
            let (x, y) = (
                (mid + rho * th.cos() * body) as i32,
                (mid + rho * th.sin() * body) as i32,
            );
            for (dx, dy) in [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)] {
                c.put(x + dx, y + dy, [0.95, 0.86, 0.90]);
            }
            c.put(x, y, [0.95, 0.85, 0.35]);
        }
    }
    c
}

/// Scatter 4–6 blossoms over a painted clump (`paint_clump`), a few sprite pixels each
/// (a rose ~3 cm across at the clump's scale), lit from the same upper-right sun. Kind
/// 0–1 and 4 are roses (a darker spiral of petals; 4 half white, half painted red), 2–3
/// five-petalled flowers round a darker heart. All inside the clump's disc, so the fog
/// silhouette (the canvas alpha) is unchanged.
fn paint_blossoms(c: &mut Canvas, seed: u64, kind: usize) {
    use std::f32::consts::TAU;
    const RED: [f32; 3] = [0.9, 0.08, 0.12];
    const WHITE: [f32; 3] = [0.97, 0.96, 0.92];
    let (petal, heart) = match kind {
        0 => (RED, [0.45, 0.02, 0.05]),
        1 => (WHITE, [0.85, 0.8, 0.62]),
        2 => ([0.98, 0.62, 0.74], [0.95, 0.85, 0.35]),
        3 => ([0.99, 0.86, 0.22], [0.62, 0.36, 0.08]),
        _ => (WHITE, [0.45, 0.02, 0.05]),
    };
    let rose = !matches!(kind, 2 | 3);
    let mut rng = Hr(seed ^ 0xB105_5011);
    let mid = CLUMP_PX as f32 * 0.5;
    let body = CLUMP_BODY - 4.0;
    let sun = vec2(0.58, -0.81);
    let n = 4 + (rng.f() * 3.0) as usize;
    for _ in 0..n {
        let rho = 0.15 + 0.55 * rng.f().sqrt();
        let th = rng.f() * TAU;
        let (bx, by) = (mid + rho * th.cos() * body, mid + rho * th.sin() * body);
        let r = 3.6 + 1.6 * rng.f();
        let spin = rng.f() * TAU;
        // Painted roses: which way the red runs across this bloom.
        let brush = rng.f() * TAU;
        let ri = r.ceil() as i32 + 1;
        for py in by as i32 - ri..=by as i32 + ri {
            for px in bx as i32 - ri..=bx as i32 + ri {
                let (dx, dy) = (px as f32 + 0.5 - bx, py as f32 + 0.5 - by);
                let d = (dx * dx + dy * dy).sqrt() / r;
                let ang = dy.atan2(dx) + spin;
                let edge = if rose {
                    0.92 + 0.08 * (ang * 6.0).cos()
                } else {
                    0.7 + 0.3 * (ang * 5.0).cos().abs()
                };
                if d > edge {
                    continue;
                }
                // Domed: lit toward the sun, shaded away and toward the rim.
                let lit = 1.0 + 0.2 * vec2(dx, dy).dot(sun) / r - 0.15 * d * d;
                let mut rgb = petal;
                if kind == 4
                    && (dx * brush.cos() + dy * brush.sin()) / r + 0.15 * (ang * 3.0).sin() > 0.0
                {
                    rgb = RED;
                }
                let k = if rose {
                    // A spiral of petal edges winding into a dark heart.
                    let curl = ((ang + d * 9.0).sin() * 0.5 + 0.5).powi(3);
                    lit * (1.0 - 0.22 * curl) * (0.72 + 0.28 * d.min(1.0) / edge).min(1.0)
                } else {
                    lit
                };
                let rgb = if !rose && d < 0.3 {
                    heart
                } else {
                    rgb.map(|v| (v * k).min(1.0))
                };
                let rgb = if rose && d < 0.18 {
                    mix3(rgb, heart, 0.6)
                } else {
                    rgb
                };
                c.put(px, py, rgb);
            }
        }
    }
}

// ── ground ───────────────────────────────────────────────────────────────────────

/// Side of the tiling ground texture, texels.
const GROUND_PX: usize = 512;
/// World units one repeat of the ground texture spans (so 128 texels per grid square).
pub const GROUND_PERIOD: f32 = 4.0;
/// Side of one ground decal in the scatter atlas (2x2 of them).
const DECAL_PX: usize = 128;

/// What lies on a ground square: walkway, the soil under a hedge/tree, or the open
/// meadow past the garden's gate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tile {
    Path,
    Under,
    Outside,
}

/// The outdoor themes' ground plane (`raycast::draw_ground`): a tiling texture laid in
/// world space (`period` units per repeat, GPU-mipmapped, wrap = repeat), plus a 2x2
/// atlas of decals (twigs, stones, clover, daisies …) scattered one per square with a
/// hashed rotation/flip, and the vertex tints that tell walkway from undergrowth.
#[derive(Clone)]
pub struct Ground {
    pub base: Texture2D,
    pub decals: Texture2D,
    /// Mean colour of `base` — what it averages to far off, where the texture fades out
    /// into the backdrop gradient.
    pub avg: [f32; 3],
    /// Vertex tints (≤ 1, they multiply) per `Tile`.
    pub path: [f32; 3],
    pub under: [f32; 3],
    pub outside: [f32; 3],
    /// How dark the ground gets right at the foot of the foliage (contact shade).
    pub ao: f32,
    /// Mowing-stripe contrast along the walkways, 0 = none.
    pub stripes: f32,
    /// Open country (Garden): the far ground and sky melt into this haze at the horizon,
    /// with a low tree line on it. `None`: everything fades into the palette fog.
    pub horizon: Option<[f32; 3]>,
    /// The Forest: both layers draw with this blend (`fog::premultiplied_material`, its
    /// decals uploaded premultiplied) so the mist goes on in the same pass, exactly —
    /// decals included.
    pub premul: Option<Material>,
    kind: ThemeKind,
}

impl Ground {
    pub fn tint(&self, tile: Tile) -> [f32; 3] {
        match tile {
            Tile::Path => self.path,
            Tile::Under => self.under,
            Tile::Outside => self.outside,
        }
    }

    /// Which decal (if any) a square carries, from a 0..1 hash of it.
    pub fn decal(&self, tile: Tile, r: f32) -> Option<usize> {
        let pick = |table: &[(f32, usize)]| {
            let mut acc = 0.0;
            table.iter().find_map(|&(p, v)| {
                acc += p;
                (r < acc).then_some(v)
            })
        };
        match (self.kind, tile) {
            // Forest: 0 twigs + cones, 1 roots, 2 stones, 3 moss + fallen leaves.
            (ThemeKind::Forest, Tile::Path) => pick(&[(0.22, 0), (0.12, 2), (0.14, 3)]),
            (ThemeKind::Forest, _) => pick(&[(0.35, 1), (0.25, 3), (0.2, 0)]),
            // Garden: 0 clover, 1 daisies, 2 buttercups, 3 hedge litter.
            (_, Tile::Path) => pick(&[(0.22, 0), (0.12, 1), (0.08, 2)]),
            (_, Tile::Under) => Some(3),
            (_, Tile::Outside) => pick(&[(0.3, 1), (0.3, 2), (0.2, 0)]),
        }
    }
}

fn ground_for(kind: ThemeKind, seed: u64) -> Option<Ground> {
    thread_local! {
        static CACHE: std::cell::RefCell<Vec<(ThemeKind, u64, Ground)>> =
            const { std::cell::RefCell::new(Vec::new()) };
    }
    if let Some(hit) = CACHE.with_borrow(|c| {
        c.iter()
            .find(|(k, s, _)| *k == kind && *s == seed)
            .map(|e| e.2.clone())
    }) {
        return Some(hit);
    }
    let (base, decals) = match kind {
        ThemeKind::Forest => (paint_forest_floor(seed), paint_decals(kind, seed)),
        ThemeKind::Garden => (paint_lawn(seed), paint_decals(kind, seed)),
        _ => return None,
    };
    let avg = mean_rgb(&base);
    let bleed = mean_rgb(&decals);
    let (path, under, outside, ao, stripes, horizon) = match kind {
        ThemeKind::Forest => (
            [1.0, 0.94, 0.82],
            [0.66, 0.78, 0.66],
            [0.66, 0.78, 0.66],
            0.3,
            0.0,
            None,
        ),
        _ => (
            [1.0, 1.0, 1.0],
            [0.42, 0.38, 0.30],
            [0.98, 1.0, 0.80],
            0.42,
            0.07,
            Some([0.74, 0.81, 0.82]),
        ),
    };
    let misty = kind == ThemeKind::Forest;
    let decals_tex = if misty {
        let mut pm = decals.clone();
        for p in &mut pm.px {
            for k in 0..3 {
                p[k] *= p[3];
            }
        }
        gpu_texture(&pm, [0.0; 3], false)
    } else {
        gpu_texture(&decals, bleed, false)
    };
    let ground = Ground {
        base: gpu_texture(&base, avg, true),
        decals: decals_tex,
        premul: if misty {
            crate::view::fog::premultiplied_material()
        } else {
            None
        },
        avg,
        path,
        under,
        outside,
        ao,
        stripes,
        horizon,
        kind,
    };
    CACHE.with_borrow_mut(|c| c.push((kind, seed, ground.clone())));
    Some(ground)
}

pub(crate) fn mean_rgb(c: &Canvas) -> [f32; 3] {
    let mut acc = [0.0f32; 4];
    for p in &c.px {
        for k in 0..3 {
            acc[k] += p[k] * p[3];
        }
        acc[3] += p[3];
    }
    let n = acc[3].max(1e-6);
    [acc[0] / n, acc[1] / n, acc[2] / n]
}

/// Upload a canvas as a linear-filtered, GPU-mipmapped texture (a floor seen at a
/// grazing angle is minified hard; without mips it shimmers), optionally wrapping so
/// world-space UVs repeat it. Transparent texels take `bleed` so filtering and the mip
/// chain fringe decals with ground colour, not black.
fn gpu_texture(c: &Canvas, bleed: [f32; 3], repeat: bool) -> Texture2D {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut bytes = Vec::with_capacity(c.px.len() * 4);
    for p in &c.px {
        let rgb = if p[3] > 0.0 {
            [p[0], p[1], p[2]]
        } else {
            bleed
        };
        bytes.extend_from_slice(&[q(rgb[0]), q(rgb[1]), q(rgb[2]), q(p[3])]);
    }
    let tex = Texture2D::from_rgba8(c.w as u16, c.h as u16, &bytes);
    tex.set_filter(FilterMode::Linear);
    mipmap(&tex, repeat);
    tex
}

/// Build the mip chain for a power-of-two texture and sample it trilinearly.
fn mipmap(tex: &Texture2D, repeat: bool) {
    use macroquad::miniquad::{FilterMode as F, MipmapFilterMode, TextureWrap};
    // SAFETY: plain GL state calls on a texture we own, outside any render pass.
    let ctx = unsafe { get_internal_gl() }.quad_context;
    let id = tex.raw_miniquad_id();
    if repeat {
        ctx.texture_set_wrap(id, TextureWrap::Repeat, TextureWrap::Repeat);
    }
    ctx.texture_generate_mipmaps(id);
    ctx.texture_set_min_filter(id, F::Linear, MipmapFilterMode::Linear);
}

/// Value noise that tiles: the lattice wraps every `cells` cells across the unit square.
fn tnoise(u: f32, v: f32, cells: i32, seed: u64) -> f32 {
    let (fx, fy) = (u * cells as f32, v * cells as f32);
    let (x0, y0) = (fx.floor() as i32, fy.floor() as i32);
    let (sx, sy) = (smooth(fx - x0 as f32), smooth(fy - y0 as f32));
    let h = |x: i32, y: i32| hash2(x.rem_euclid(cells), y.rem_euclid(cells), seed);
    lerp(
        lerp(h(x0, y0), h(x0 + 1, y0), sx),
        lerp(h(x0, y0 + 1), h(x0 + 1, y0 + 1), sx),
        sy,
    )
}

/// Tiling fbm from `cells` up, `octaves` doublings.
fn tfbm(u: f32, v: f32, cells: i32, octaves: u32, seed: u64) -> f32 {
    let (mut sum, mut amp, mut norm) = (0.0, 0.5, 0.0);
    for o in 0..octaves {
        sum += amp * tnoise(u, v, cells << o, seed ^ (o as u64 * 0x1234567));
        norm += amp;
        amp *= 0.5;
    }
    sum / norm
}

impl Canvas {
    /// Paint one opaque pixel, wrapping at the edges (a tiling texture).
    fn put_wrap(&mut self, x: i32, y: i32, c: [f32; 3]) {
        let (x, y) = (x.rem_euclid(self.w as i32), y.rem_euclid(self.h as i32));
        self.px[y as usize * self.w + x as usize] = [c[0], c[1], c[2], 1.0];
    }

    /// A filled, rotated ellipse; `shade(lx, ly)` (both -1..1 across it) colours it.
    #[allow(clippy::too_many_arguments)]
    fn ellipse(
        &mut self,
        wrap: bool,
        x: f32,
        y: f32,
        a: f32,
        b: f32,
        ang: f32,
        shade: impl Fn(f32, f32) -> [f32; 3],
    ) {
        let (s, co) = ang.sin_cos();
        let r = a.max(b).ceil() as i32 + 1;
        for py in y as i32 - r..=y as i32 + r {
            for px in x as i32 - r..=x as i32 + r {
                let (dx, dy) = (px as f32 + 0.5 - x, py as f32 + 0.5 - y);
                let (lx, ly) = ((dx * co + dy * s) / a, (-dx * s + dy * co) / b);
                if lx * lx + ly * ly > 1.0 {
                    continue;
                }
                let c = shade(lx, ly);
                if wrap {
                    self.put_wrap(px, py, c);
                } else {
                    self.put(px, py, c);
                }
            }
        }
    }

    /// A stroke `w` px wide from `a` to `b`, coloured per point by `shade(t, across)`.
    fn stroke(
        &mut self,
        wrap: bool,
        a: Vec2,
        b: Vec2,
        w: f32,
        shade: impl Fn(f32, f32) -> [f32; 3],
    ) {
        let len = a.distance(b).max(0.01);
        let d = (b - a) / len;
        let n = vec2(-d.y, d.x);
        let steps = (len * 2.0).ceil() as i32;
        for i in 0..=steps {
            let t = i as f32 / steps as f32;
            let p = a + (b - a) * t;
            let half = (w * 0.5).max(0.5);
            let mut k = -half;
            while k <= half {
                let q = p + n * k;
                let c = shade(t, k / half);
                if wrap {
                    self.put_wrap(q.x as i32, q.y as i32, c);
                } else {
                    self.put(q.x as i32, q.y as i32, c);
                }
                k += 0.5;
            }
        }
    }
}

/// Forest floor, tiling: dark humus under drifts of rusty needle litter and soft moss
/// patches, painted as big soft shapes plus a scatter of needle strokes. Kept low in
/// contrast — it's under a pale mist, and at 128 texels a unit it's magnified 3-5x at
/// the camera's feet.
pub(crate) fn paint_forest_floor(seed: u64) -> Canvas {
    let n = GROUND_PX;
    let mut c = Canvas::new(n, n);
    let mut rng = Hr(seed ^ 0xF0E5);
    let humus = [0.20, 0.15, 0.10];
    let litter = [0.40, 0.28, 0.17];
    let moss = [0.26, 0.34, 0.13];
    let moss_at =
        |u: f32, v: f32| smooth(((tfbm(u, v, 5, 3, seed ^ 1) - 0.52) / 0.22).clamp(0.0, 1.0));
    for y in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
            let big = tfbm(u, v, 8, 3, seed);
            let m = moss_at(u, v);
            let grain = tnoise(u, v, 128, seed ^ 2);
            let base = mix3(humus, litter, 0.2 + 0.6 * big);
            let green = moss.map(|k| k * (0.8 + 0.4 * tnoise(u, v, 64, seed ^ 3)));
            let col = mix3(base, green, m * 0.85);
            c.px[y * n + x] = [
                col[0] * (0.93 + 0.14 * grain),
                col[1] * (0.93 + 0.14 * grain),
                col[2] * (0.93 + 0.14 * grain),
                1.0,
            ];
        }
    }
    // Needles: short strokes in two tones, thinned out over the moss.
    // Each needle is only half its own colour, half the ground under it: a texture,
    // not a pattern.
    for i in 0..9_000 {
        let (x, y) = (rng.f() * n as f32, rng.f() * n as f32);
        if moss_at(x / n as f32, y / n as f32) > 0.1 + 0.5 * rng.f() {
            continue;
        }
        let ang = rng.f() * std::f32::consts::TAU;
        let len = 4.0 + 5.0 * rng.f();
        let k = 0.85 + 0.25 * rng.f();
        let under = c.px[(y as usize).min(n - 1) * n + (x as usize).min(n - 1)];
        let own = if i % 3 == 0 {
            [0.26, 0.18, 0.10]
        } else {
            mix3([0.46, 0.31, 0.17], [0.50, 0.40, 0.25], rng.f())
        };
        let tone = mix3(own, [under[0], under[1], under[2]], 0.5);
        let d = vec2(ang.cos(), ang.sin()) * len;
        let a = vec2(x, y);
        c.stroke(true, a, a + d, 1.0, |t, _| {
            tone.map(|v| v * k * (0.9 + 0.2 * t))
        });
    }
    c
}

/// Mown lawn, tiling: a green field with broad soft patches, a fine speckle of lighter
/// and darker blades, and a few low clover drifts. Stripes are laid per walkway by the
/// raycaster (vertex tone), not painted here.
fn paint_lawn(seed: u64) -> Canvas {
    let n = GROUND_PX;
    let mut c = Canvas::new(n, n);
    let mut rng = Hr(seed ^ 0x1A77);
    let grass = [0.30, 0.47, 0.17];
    let dry = [0.42, 0.50, 0.20];
    for y in 0..n {
        for x in 0..n {
            let (u, v) = (x as f32 / n as f32, y as f32 / n as f32);
            let patch = tfbm(u, v, 6, 3, seed);
            let yellow = smooth(((tfbm(u, v, 3, 2, seed ^ 4) - 0.55) / 0.2).clamp(0.0, 1.0));
            let grain = tnoise(u, v, 160, seed ^ 5);
            let col = mix3(grass, dry, yellow * 0.5);
            let k = (0.86 + 0.22 * patch) * (0.94 + 0.12 * grain);
            c.px[y * n + x] = [col[0] * k, col[1] * k, col[2] * k, 1.0];
        }
    }
    // Blades: tiny strokes, lighter tips and darker shadows between them.
    for i in 0..30_000 {
        let (x, y) = (rng.f() * n as f32, rng.f() * n as f32);
        let ang = rng.f() * std::f32::consts::TAU;
        let len = 1.5 + 2.5 * rng.f();
        let tone = if i % 2 == 0 {
            [0.36, 0.54, 0.20]
        } else {
            [0.22, 0.37, 0.13]
        };
        let k = 0.85 + 0.3 * rng.f();
        let a = vec2(x, y);
        c.stroke(
            true,
            a,
            a + vec2(ang.cos(), ang.sin()) * len,
            1.0,
            |_, _| tone.map(|v| v * k),
        );
    }
    // Low clover drifts: trefoils where a coarse noise runs high.
    for _ in 0..2_600 {
        let (x, y) = (rng.f() * n as f32, rng.f() * n as f32);
        if tfbm(x / n as f32, y / n as f32, 4, 2, seed ^ 6) < 0.58 {
            continue;
        }
        trefoil(
            &mut c,
            true,
            x,
            y,
            1.6 + 0.8 * rng.f(),
            rng.f() * std::f32::consts::TAU,
            0.8 + 0.3 * rng.f(),
        );
    }
    c
}

/// A three-leaf clover seen from above: three round leaflets round a point.
fn trefoil(c: &mut Canvas, wrap: bool, x: f32, y: f32, r: f32, ang: f32, k: f32) {
    for i in 0..3 {
        let a = ang + i as f32 * std::f32::consts::TAU / 3.0;
        let (lx, ly) = (x + a.cos() * r, y + a.sin() * r);
        c.ellipse(wrap, lx, ly, r * 1.05, r * 0.95, a, |px, _| {
            let base = [0.20, 0.42, 0.17];
            let lit = 1.0 + 0.15 * px;
            base.map(|v| v * k * lit)
        });
    }
}

/// The 2x2 decal atlas for one theme's ground; each decal keeps a clear margin so a
/// rotated or flipped copy never touches its square's edge.
fn paint_decals(kind: ThemeKind, seed: u64) -> Canvas {
    let n = DECAL_PX * 2;
    let mut atlas = Canvas::new(n, n);
    for v in 0..4 {
        let mut d = Canvas::new(DECAL_PX, DECAL_PX);
        let mut rng = Hr(seed ^ (v as u64 + 1).wrapping_mul(0x000D_ECA1));
        match (kind, v) {
            (ThemeKind::Forest, 0) => decal_twigs(&mut d, &mut rng),
            (ThemeKind::Forest, 1) => decal_roots(&mut d, &mut rng),
            (ThemeKind::Forest, 2) => decal_stones(&mut d, &mut rng),
            (ThemeKind::Forest, _) => decal_moss(&mut d, &mut rng, seed),
            (_, 0) => decal_clover(&mut d, &mut rng),
            (_, 1) => decal_flowers(&mut d, &mut rng, true),
            (_, 2) => decal_flowers(&mut d, &mut rng, false),
            _ => decal_litter(&mut d, &mut rng),
        }
        let (ox, oy) = ((v % 2) * DECAL_PX, (v / 2) * DECAL_PX);
        for y in 0..DECAL_PX {
            for x in 0..DECAL_PX {
                atlas.px[(oy + y) * n + ox + x] = d.px[y * DECAL_PX + x];
            }
        }
    }
    atlas
}

/// A point inside a decal, at least `m` px from its edge.
fn inside(rng: &mut Hr, m: f32) -> Vec2 {
    let s = DECAL_PX as f32 - 2.0 * m;
    vec2(m + rng.f() * s, m + rng.f() * s)
}

fn decal_twigs(d: &mut Canvas, rng: &mut Hr) {
    for _ in 0..2 {
        let a = inside(rng, 22.0);
        let ang = rng.f() * std::f32::consts::TAU;
        let len = 40.0 + 30.0 * rng.f();
        let b = (a + vec2(ang.cos(), ang.sin()) * len).clamp(Vec2::splat(12.0), Vec2::splat(116.0));
        let bark = [0.30, 0.22, 0.15];
        d.stroke(false, a, b, 2.2, |_, s| bark.map(|v| v * (1.0 - 0.3 * s)));
        for _ in 0..3 {
            let p = a + (b - a) * (0.2 + 0.6 * rng.f());
            let ba = ang + (rng.f() - 0.5) * 2.0;
            let q = (p + vec2(ba.cos(), ba.sin()) * (8.0 + 10.0 * rng.f()))
                .clamp(Vec2::splat(10.0), Vec2::splat(118.0));
            d.stroke(false, p, q, 1.0, |_, _| bark.map(|v| v * 0.85));
        }
    }
    for _ in 0..2 + (rng.f() * 3.0) as usize {
        let p = inside(rng, 16.0);
        let ang = rng.f() * std::f32::consts::TAU;
        d.ellipse(false, p.x, p.y, 7.0, 4.0, ang, |lx, ly| {
            // Scales: a checker of darker grooves along the cone.
            let scale = ((lx * 4.0).round() as i32 + (ly * 2.0).round() as i32) % 2 == 0;
            let k = if scale { 0.8 } else { 1.0 } * (1.0 - 0.3 * ly);
            [0.42 * k, 0.27 * k, 0.14 * k]
        });
    }
}

fn decal_roots(d: &mut Canvas, rng: &mut Hr) {
    for _ in 0..2 {
        let mut p = inside(rng, 14.0);
        let mut ang = rng.f() * std::f32::consts::TAU;
        let len = 70 + (rng.f() * 40.0) as i32;
        let w0 = 4.0 + 3.0 * rng.f();
        for i in 0..len {
            let t = i as f32 / len as f32;
            ang += (rng.f() - 0.5) * 0.25;
            let q = p + vec2(ang.cos(), ang.sin());
            if !(12.0..116.0).contains(&q.x) || !(12.0..116.0).contains(&q.y) {
                break;
            }
            let w = w0 * (1.0 - 0.75 * t);
            // Shadow on one side, a lit ridge along the other.
            d.stroke(false, p, q, w + 1.5, |_, _| [0.10, 0.08, 0.06]);
            d.stroke(false, p, q, w, |_, s| {
                let k = 0.8 + 0.45 * (-s).max(0.0);
                [0.44 * k, 0.37 * k, 0.29 * k]
            });
            p = q;
        }
    }
}

fn decal_stones(d: &mut Canvas, rng: &mut Hr) {
    for _ in 0..2 + (rng.f() * 2.0) as usize {
        let p = inside(rng, 22.0);
        let (a, b) = (6.0 + 8.0 * rng.f(), 5.0 + 6.0 * rng.f());
        let ang = rng.f() * std::f32::consts::TAU;
        let grey = 0.36 + 0.12 * rng.f();
        let mossy = rng.f() < 0.5;
        d.ellipse(
            false,
            p.x + 1.5,
            p.y + 1.5,
            a + 1.0,
            b + 1.0,
            ang,
            |_, _| [0.08, 0.07, 0.06],
        );
        d.ellipse(false, p.x, p.y, a, b, ang, |lx, ly| {
            let k = 1.05 - 0.35 * (lx + ly).max(-1.0) * 0.5 - 0.2 * (lx * lx + ly * ly);
            if mossy && lx + ly < -0.6 {
                [0.24 * k, 0.33 * k, 0.12 * k]
            } else {
                [grey * k, grey * 0.97 * k, grey * 0.9 * k]
            }
        });
    }
}

fn decal_moss(d: &mut Canvas, rng: &mut Hr, seed: u64) {
    let c = vec2(64.0, 64.0) + (vec2(rng.f(), rng.f()) - 0.5) * 20.0;
    let r = 26.0 + 10.0 * rng.f();
    for y in 0..DECAL_PX as i32 {
        for x in 0..DECAL_PX as i32 {
            let p = vec2(x as f32 + 0.5, y as f32 + 0.5);
            let edge = vnoise(p.x / 128.0, p.y / 128.0, 9.0, seed ^ 0x3055) * 0.45 + 0.75;
            let dd = p.distance(c) / (r * edge);
            if dd < 1.0 {
                let k = (1.1 - 0.35 * dd) * (0.85 + 0.3 * hash2(x / 2, y / 2, seed ^ 0x3056));
                d.put(x, y, [0.25 * k, 0.37 * k, 0.12 * k]);
            }
        }
    }
    for _ in 0..5 + (rng.f() * 4.0) as usize {
        let p = inside(rng, 14.0);
        let ang = rng.f() * std::f32::consts::TAU;
        let tone = mix3([0.62, 0.48, 0.16], [0.46, 0.28, 0.12], rng.f());
        d.ellipse(false, p.x, p.y, 4.5, 2.8, ang, |lx, ly| {
            let vein = if ly.abs() < 0.15 { 0.85 } else { 1.0 };
            tone.map(|v| v * vein * (1.0 - 0.15 * lx))
        });
    }
}

fn decal_clover(d: &mut Canvas, rng: &mut Hr) {
    let c = inside(rng, 40.0);
    for _ in 0..40 {
        let p = c + vec2(rng.f() - 0.5, rng.f() - 0.5) * 50.0;
        trefoil(
            d,
            false,
            p.x,
            p.y,
            2.2 + rng.f(),
            rng.f() * std::f32::consts::TAU,
            0.85 + 0.3 * rng.f(),
        );
    }
}

/// Daisies (white, yellow eye) or buttercups (small, all yellow), in a loose drift.
fn decal_flowers(d: &mut Canvas, rng: &mut Hr, daisies: bool) {
    let c = inside(rng, 36.0);
    let count = if daisies { 7 } else { 9 };
    for _ in 0..count {
        let p = c + vec2(rng.f() - 0.5, rng.f() - 0.5) * 56.0;
        if daisies {
            let petals = 8;
            let r = 2.6 + rng.f();
            for i in 0..petals {
                let a = i as f32 * std::f32::consts::TAU / petals as f32 + rng.f() * 0.3;
                let q = p + vec2(a.cos(), a.sin()) * r;
                d.ellipse(false, q.x, q.y, r * 0.7, r * 0.35, a, |lx, _| {
                    [0.93 - 0.08 * lx, 0.92 - 0.08 * lx, 0.88 - 0.05 * lx]
                });
            }
            d.ellipse(false, p.x, p.y, 1.6, 1.6, 0.0, |_, _| [0.92, 0.72, 0.18]);
        } else {
            let r = 1.8 + 0.8 * rng.f();
            for i in 0..5 {
                let a = i as f32 * std::f32::consts::TAU / 5.0;
                let q = p + vec2(a.cos(), a.sin()) * r * 0.7;
                d.ellipse(false, q.x, q.y, r * 0.6, r * 0.5, a, |_, _| {
                    [0.95, 0.80, 0.16]
                });
            }
            d.put(p.x as i32, p.y as i32, [0.70, 0.55, 0.12]);
        }
    }
}

fn decal_litter(d: &mut Canvas, rng: &mut Hr) {
    for _ in 0..22 {
        let p = inside(rng, 12.0);
        let ang = rng.f() * std::f32::consts::TAU;
        let tone = mix3([0.45, 0.36, 0.16], [0.24, 0.34, 0.12], rng.f());
        d.ellipse(false, p.x, p.y, 3.2, 1.7, ang, |lx, _| {
            tone.map(|v| v * (1.0 - 0.15 * lx))
        });
    }
    for _ in 0..2 {
        let a = inside(rng, 24.0);
        let ang = rng.f() * std::f32::consts::TAU;
        let b = a + vec2(ang.cos(), ang.sin()) * 22.0;
        d.stroke(false, a, b, 1.2, |_, _| [0.26, 0.20, 0.13]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn coverage(c: &Canvas) -> f32 {
        c.px.iter().map(|p| p[3]).sum::<f32>() / c.px.len() as f32
    }

    #[test]
    fn spruce_stands_on_its_trunk() {
        for v in 0..SPRUCES as u64 {
            let c = paint_spruce(v * 77 + 1);
            let foot = (TREE_H as f32 - TREE_MARGIN) as usize - 1;
            assert_eq!(c.alpha(TREE_W / 2, foot), 1.0, "trunk reaches the ground");
            for (x, y) in [
                (0, 0),
                (TREE_W - 1, 0),
                (0, TREE_H - 1),
                (TREE_W - 1, TREE_H - 1),
            ] {
                assert_eq!(c.alpha(x, y), 0.0, "corner ({x},{y}) is clear");
            }
            // The tip narrows to a leader; the lower crown spreads wide.
            let row = |y: usize| (0..TREE_W).filter(|&x| c.alpha(x, y) > 0.0).count();
            assert!(row(TREE_MARGIN as usize + 2) < 4);
            let wide = (TREE_H / 2..TREE_H * 9 / 10).map(row).max().unwrap();
            assert!(wide > TREE_W / 2, "crown width {wide}");
            let cov = coverage(&c);
            assert!((0.15..0.6).contains(&cov), "coverage {cov}");
        }
    }

    /// Startup cost of painting the Forest's spruces:
    /// `cargo test -p labyrinth --release spruce_paint_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    #[allow(clippy::disallowed_methods)] // native test timing
    fn spruce_paint_cost() {
        let runs = 20;
        let t0 = std::time::Instant::now();
        for r in 0..runs {
            for v in 0..SPRUCES as u64 {
                std::hint::black_box(paint_spruce(r * 1000 + v));
            }
        }
        eprintln!("{SPRUCES} spruces: {:?}", t0.elapsed() / runs as u32);
    }

    #[test]
    fn clump_is_a_ragged_ball() {
        for v in 0..CLUMPS as u64 {
            let c = paint_clump(v + 9, v == 3);
            let m = CLUMP_PX / 2;
            assert_eq!(c.alpha(m, m), 1.0);
            assert_eq!(c.alpha(0, 0), 0.0);
            assert_eq!(c.alpha(CLUMP_PX - 1, CLUMP_PX - 1), 0.0);
            // Area near a disc of radius CLUMP_BODY.
            let disc =
                std::f32::consts::PI * CLUMP_BODY * CLUMP_BODY / (CLUMP_PX * CLUMP_PX) as f32;
            let cov = coverage(&c);
            assert!(
                (cov / disc - 1.0).abs() < 0.3,
                "coverage {cov} vs disc {disc}"
            );
        }
    }

    /// Dump the painted art to `tmp/labyrinth_art/` for inspection (no window, no GL):
    /// `cargo test -p labyrinth --release dump_art -- --ignored`.
    #[test]
    #[ignore]
    fn dump_art() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/labyrinth_art");
        std::fs::create_dir_all(&dir).unwrap();
        let save = |c: &Canvas, name: &str, bg: [f32; 3]| {
            let mut bytes = Vec::with_capacity(c.px.len() * 4);
            // `export_png` flips rows (it expects GL's bottom-up order).
            for p in c.px.chunks(c.w).rev().flatten() {
                for k in 0..3 {
                    bytes
                        .push(((p[k] * p[3] + bg[k] * (1.0 - p[3])).clamp(0.0, 1.0) * 255.0) as u8);
                }
                bytes.push(255);
            }
            let img = Image {
                bytes,
                width: c.w as u16,
                height: c.h as u16,
            };
            img.export_png(dir.join(name).to_str().unwrap());
        };
        let fog = [0.60, 0.66, 0.60];
        let mut row = Canvas::new(TREE_W * 8, TREE_H);
        for v in 0..8 {
            let t = paint_spruce(0x5EED ^ ((v as u64 + 1) * 0x51_7CC1));
            for y in 0..TREE_H {
                for x in 0..TREE_W {
                    row.px[y * row.w + v * TREE_W + x] = t.px[y * TREE_W + x];
                }
            }
        }
        save(&row, "spruces.png", fog);
        let n = CLUMPS + BLOOM_KINDS * BLOOM_ALTS;
        let mut row = Canvas::new(CLUMP_PX * n, CLUMP_PX);
        for v in 0..n {
            let t = if v < CLUMPS {
                paint_clump(0x5EED ^ ((v as u64 + 1) * 0x2F_1A55), v == 3)
            } else {
                let s = 0x5EED ^ ((v as u64 - 3) * 0x7B_10C5);
                let mut c = paint_clump(s, false);
                paint_blossoms(&mut c, s, (v - CLUMPS) / BLOOM_ALTS);
                c
            };
            for y in 0..CLUMP_PX {
                for x in 0..CLUMP_PX {
                    row.px[y * row.w + v * CLUMP_PX + x] = t.px[y * CLUMP_PX + x];
                }
            }
        }
        save(&row, "clumps.png", [0.03, 0.08, 0.035]);
        save(&paint_forest_floor(0x5EED), "forest_floor.png", fog);
        save(&paint_lawn(0x5EED), "lawn.png", fog);
        save(
            &paint_decals(ThemeKind::Forest, 0x5EED),
            "forest_decals.png",
            [0.3, 0.22, 0.14],
        );
        save(
            &paint_decals(ThemeKind::Garden, 0x5EED),
            "garden_decals.png",
            [0.3, 0.45, 0.17],
        );
        let mut img = Image::gen_image_color(TEX, TEX, BLACK);
        paint_garden(&mut img, 0x5EED);
        let mut c = Canvas::new(TEX as usize, TEX as usize);
        for (i, p) in img.get_image_data().iter().enumerate() {
            c.px[i] = [p[0], p[1], p[2], p[3]].map(|v| v as f32 / 255.0);
        }
        save(&c, "hedge_wall.png", [0.0; 3]);
    }

    /// The ground textures repeat across squares: opposite edges continue each other
    /// about as smoothly as neighbouring columns inside do.
    #[test]
    fn ground_tiles_seamlessly() {
        for c in [paint_forest_floor(3), paint_lawn(3)] {
            let n = c.w;
            let diff = |x0: usize, x1: usize| {
                (0..n)
                    .map(|y| {
                        let (a, b) = (c.px[y * n + x0], c.px[y * n + x1]);
                        (0..3).map(|k| (a[k] - b[k]).abs()).sum::<f32>()
                    })
                    .sum::<f32>()
                    / n as f32
            };
            let seam = diff(n - 1, 0);
            let inner = diff(n / 2 - 1, n / 2);
            assert!(seam < inner * 1.5 + 0.01, "seam {seam} vs inner {inner}");
        }
    }

    #[test]
    fn mips_fit_their_blocks() {
        for (w, h) in [(TREE_W, TREE_H), (CLUMP_PX, CLUMP_PX)] {
            let (bw, bh) = block(w, h);
            for l in 0..LEVELS {
                let (x, y, lw, lh) = level_rect(w, h, l);
                assert!(
                    x + lw + GAP <= bw && y + lh + GAP <= bh,
                    "{w}x{h} level {l}"
                );
            }
        }
        let c = paint_clump(1, false);
        let half = c.half();
        let (a, b) = (coverage(&c), coverage(&half));
        assert!(
            (a - b).abs() < 0.02,
            "downsampling keeps coverage: {a} vs {b}"
        );
    }
}
