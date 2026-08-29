//! Procedural wall art. One `Texture2D` per theme, generated into an `Image` at
//! startup (no filesystem on WASM, and `include_bytes!` PNGs would eat the wasm
//! budget), `FilterMode::Nearest`. The grain is hash-noise seeded by `HCG_SEED`, not
//! `macroquad::rand` — so building the atlas never perturbs the maze RNG stream.
//!
//! Phase 4 ships one wall texture + a palette (ceiling/floor/fog) per theme. Per-face
//! decals and the fx hookup are later phases.

use macroquad::prelude::*;

pub const TEX: u16 = 128;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ThemeKind {
    Backrooms,
    Dungeon,
    Mine,
    Forest,
    Garden,
}

impl ThemeKind {
    pub const ALL: [ThemeKind; 5] = [
        ThemeKind::Backrooms,
        ThemeKind::Dungeon,
        ThemeKind::Mine,
        ThemeKind::Forest,
        ThemeKind::Garden,
    ];

    pub fn for_generation(g: u32) -> ThemeKind {
        // Dev override for screenshotting a specific theme without playing to that gen.
        #[cfg(not(target_arch = "wasm32"))]
        if let Ok(name) = std::env::var("HCG_LABYRINTH_THEME")
            && let Some(t) = Self::ALL
                .iter()
                .find(|t| t.label().eq_ignore_ascii_case(&name))
        {
            return *t;
        }
        Self::ALL[(g as usize) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            ThemeKind::Backrooms => "Backrooms",
            ThemeKind::Dungeon => "Dungeon",
            ThemeKind::Mine => "Mine",
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
}

pub struct Theme {
    pub kind: ThemeKind,
    pub wall: Texture2D,
    pub palette: Palette,
}

impl Theme {
    pub fn build(kind: ThemeKind, seed: u64) -> Theme {
        let mut img = Image::gen_image_color(TEX, TEX, BLACK);
        match kind {
            ThemeKind::Backrooms => paint_backrooms(&mut img, seed),
            ThemeKind::Dungeon => paint_dungeon(&mut img, seed),
            ThemeKind::Mine => paint_mine(&mut img, seed),
            ThemeKind::Forest => paint_forest(&mut img, seed),
            ThemeKind::Garden => paint_garden(&mut img, seed),
        }
        let wall = Texture2D::from_image(&img);
        wall.set_filter(FilterMode::Nearest);
        Theme {
            kind,
            wall,
            palette: palette_for(kind),
        }
    }
}

fn palette_for(kind: ThemeKind) -> Palette {
    match kind {
        ThemeKind::Backrooms => Palette {
            ceil_top: c(0.70, 0.67, 0.44),
            ceil_bot: c(0.82, 0.79, 0.55),
            floor_near: c(0.55, 0.50, 0.32),
            floor_far: c(0.30, 0.28, 0.18),
            fog: c(0.42, 0.40, 0.26),
            ew_tint: 0.82,
        },
        ThemeKind::Dungeon => Palette {
            ceil_top: c(0.04, 0.04, 0.06),
            ceil_bot: c(0.12, 0.11, 0.13),
            floor_near: c(0.22, 0.20, 0.19),
            floor_far: c(0.05, 0.05, 0.06),
            fog: c(0.04, 0.04, 0.05),
            ew_tint: 0.70,
        },
        ThemeKind::Mine => Palette {
            ceil_top: c(0.05, 0.04, 0.03),
            ceil_bot: c(0.14, 0.11, 0.08),
            floor_near: c(0.26, 0.20, 0.14),
            floor_far: c(0.06, 0.05, 0.03),
            fog: c(0.05, 0.04, 0.03),
            ew_tint: 0.68,
        },
        ThemeKind::Forest => Palette {
            ceil_top: c(0.10, 0.16, 0.10),
            ceil_bot: c(0.24, 0.34, 0.20),
            floor_near: c(0.20, 0.22, 0.12),
            floor_far: c(0.04, 0.06, 0.03),
            fog: c(0.08, 0.12, 0.08),
            ew_tint: 0.74,
        },
        ThemeKind::Garden => Palette {
            ceil_top: c(0.35, 0.50, 0.62),
            ceil_bot: c(0.62, 0.78, 0.88),
            floor_near: c(0.28, 0.34, 0.20),
            floor_far: c(0.06, 0.09, 0.05),
            fog: c(0.30, 0.42, 0.30),
            ew_tint: 0.80,
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

fn paint_mine(img: &mut Image, seed: u64) {
    // Rough hewn rock (fBm) with darker cracks and occasional ore glint.
    let rock = c(0.30, 0.24, 0.17);
    fill(img, |u, v| {
        let n = fbm(u, v, seed);
        let crack = if fbm(u * 1.7 + 3.0, v * 1.7, seed ^ 99) < 0.28 {
            0.35
        } else {
            1.0
        };
        let ore = if hash2((u * 40.0) as i32, (v * 40.0) as i32, seed ^ 5) > 0.985 {
            0.5
        } else {
            0.0
        };
        Color::new(
            rock.r * (0.6 + n * 0.7) * crack + ore * 0.3,
            rock.g * (0.6 + n * 0.7) * crack + ore * 0.4,
            rock.b * (0.6 + n * 0.7) * crack + ore * 0.7,
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
    // Dense hedge: green noise, darkened toward the tile edges, scattered blooms.
    let leaf = c(0.12, 0.30, 0.12);
    fill(img, |u, v| {
        let n = fbm(u, v, seed) * 0.6 + vnoise(u, v, 30.0, seed ^ 1) * 0.4;
        let edge = 1.0 - ((u - 0.5).abs() + (v - 0.5).abs()) * 0.5;
        let bloom = if hash2((u * 24.0) as i32, (v * 24.0) as i32, seed ^ 8) > 0.97 {
            1.0
        } else {
            0.0
        };
        Color::new(
            leaf.r + n * 0.10 + bloom * 0.6,
            (leaf.g + n * 0.35) * edge,
            leaf.b + n * 0.10 + bloom * 0.3,
            1.0,
        )
    });
}
