//! Open-air mist (the Forest): a physically-shaped fog model shared by every pass that
//! draws something out there — sky, ground, trees, critters, the campfire.
//!
//! Two separate things, never mixed up: *lighting* only scales an object's own colour;
//! *mist* blends it toward the light the mist itself scatters at the eye. For a point
//! `dist` away at height `z`, seen from eye height `z0`:
//!
//! ```text
//! seen = own · T + colour(elevation) · (1 − T),   T = exp(−∫ density ds)
//! ```
//!
//! - `density` falls off with height (a uniform part plus a ground-hugging layer with
//!   e-folding height `scale`), so a tree's foot sinks into the mist sooner than its
//!   crown and the floor far out melts away before the trees standing on it.
//! - The scattered colour depends on the ray's elevation only: luminous at the horizon,
//!   paler and cooler overhead, dimmer looking down. The sky *is* this colour at
//!   infinite distance, so anything fading out converges on exactly what's behind it —
//!   no pale halo round a crown against a darker sky, no seam where the floor meets it.
//!
//! The old model (`raycast::lit_at`, `1 / (1 + haze·(0.14d + 0.02d²))` clamped at 0.05,
//! one flat fog colour) is kept for the indoor themes and the Garden.

/// The bot's resting eye height (the walking bob moves it a little).
pub const EYE: f32 = 0.5;

/// `Palette::haze` at or above this marks a palette whose `mist` is used — so
/// `raycast::lit_at`'s shared callers (the campfire, doors, keys …) follow the mist's
/// eye-level transmittance without each needing the palette's `Mist`.
pub const MIST_HAZE: f32 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mist {
    /// Extinction per world unit at eye height.
    pub density: f32,
    /// Share of the eye-level density that belongs to the ground-hugging layer.
    pub ground: f32,
    /// E-folding height of that layer, world units.
    pub scale: f32,
    /// Scattered light looking level, straight up, and down at the ground.
    pub horizon: [f32; 3],
    pub zenith: [f32; 3],
    pub floor: [f32; 3],
}

/// The Forest's morning mist.
pub const FOREST: Mist = Mist {
    density: 0.30,
    ground: 0.45,
    scale: 1.2,
    horizon: [0.72, 0.76, 0.74],
    zenith: [0.80, 0.84, 0.85],
    floor: [0.60, 0.64, 0.60],
};

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

impl Mist {
    /// Mean extinction per unit length along a straight path between heights `z0` and
    /// `z1` (the ground layer's exponential integrated exactly). `transmittance` is
    /// `exp(-mean_density · dist)`; callers fading many points at one height (the
    /// ground) hoist it out of the loop.
    pub fn mean_density(&self, z0: f32, z1: f32) -> f32 {
        let h = self.scale;
        let f = |z: f32| (-(z - EYE) / h).exp();
        let dz = z1 - z0;
        let layer = if dz.abs() < 1e-3 {
            f(0.5 * (z0 + z1))
        } else {
            h * (f(z0) - f(z1)) / dz
        };
        self.density * ((1.0 - self.ground) + self.ground * layer)
    }

    /// How much of a point's own light survives: `dist` along the ray (3D), from eye
    /// height `z0` to the point's height `z`.
    pub fn transmittance(&self, dist: f32, z0: f32, z: f32) -> f32 {
        (-self.mean_density(z0, z) * dist).exp()
    }

    /// Level transmittance at eye height — what `raycast::lit_at` returns for a misty
    /// palette.
    pub fn level(&self, dist: f32) -> f32 {
        (-self.density * dist).exp()
    }

    /// The light the mist scatters toward the eye along a ray at `elev` (rise over run:
    /// screen offset from the horizon over the view height, since the camera never
    /// pitches).
    pub fn colour(&self, elev: f32) -> [f32; 3] {
        let up = smoothstep(0.0, 0.5, elev);
        let down = smoothstep(0.0, 0.35, -elev);
        mix(mix(self.horizon, self.zenith, up), self.floor, down)
    }

    /// An object's colour `own` seen through the mist: `t` its transmittance, `elev`
    /// its ray's elevation.
    pub fn apply(&self, own: [f32; 3], t: f32, elev: f32) -> [f32; 3] {
        mix(self.colour(elev), own, t)
    }
}

/// macroquad's default vertex shader, plus the `normal` attribute (in every macroquad
/// vertex layout) passed through as the mist's in-scattered light.
const PREMUL_VS: &str = r#"#version 100
attribute vec3 position;
attribute vec2 texcoord;
attribute vec4 color0;
attribute vec4 normal;

varying lowp vec2 uv;
varying lowp vec4 color;
varying lowp vec3 inscatter;

uniform mat4 Model;
uniform mat4 Projection;

void main() {
    gl_Position = Projection * Model * vec4(position, 1);
    color = color0 / 255.0;
    inscatter = normal.rgb;
    uv = texcoord;
}"#;

/// Premultiplied texel × vertex colour, plus the in-scattered light over its coverage.
const PREMUL_FS: &str = r#"#version 100
varying lowp vec4 color;
varying lowp vec2 uv;
varying lowp vec3 inscatter;

uniform sampler2D Texture;

void main() {
    lowp vec4 t = texture2D(Texture, uv);
    gl_FragColor = vec4(t.rgb * color.rgb + t.a * inscatter, t.a * color.a);
}"#;

/// Premultiplied-alpha blending (`src + dst·(1 − src.a)`) for a premultiplied atlas,
/// with the mist added in the same pass. A vertex carries colour `(T·tint·w, w)` and, in
/// its `normal` slot, `fog·(1 − T)·w`: the fragment is `texel·colour + coverage·fog`,
/// which fogs a sprite *exactly*, soft edges included, in one pass per layer. Straight
/// alpha has no exact order: fog-over-colour leaves soft edges under-fogged (a dark
/// rim), a grown silhouette fogs past them (a pale halo) — the old spruces had one or
/// the other round every tree. Plain GLSL 100 + a standard blend state: WebGL1-safe.
pub fn premultiplied_material() -> Option<macroquad::material::Material> {
    use macroquad::material::{MaterialParams, load_material};
    use macroquad::miniquad::{
        BlendFactor, BlendState, BlendValue, Equation, PipelineParams, ShaderSource,
    };
    load_material(
        ShaderSource::Glsl {
            vertex: PREMUL_VS,
            fragment: PREMUL_FS,
        },
        MaterialParams {
            pipeline_params: PipelineParams {
                color_blend: Some(BlendState::new(
                    Equation::Add,
                    BlendFactor::One,
                    BlendFactor::OneMinusValue(BlendValue::SourceAlpha),
                )),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mist_is_thicker_low_and_fades_with_distance() {
        let m = FOREST;
        assert!((m.transmittance(3.0, EYE, EYE) - m.level(3.0)).abs() < 1e-5);
        assert!(m.transmittance(4.0, EYE, 0.0) < m.transmittance(4.0, EYE, 4.0));
        assert!(m.transmittance(8.0, EYE, 2.0) < m.transmittance(4.0, EYE, 2.0));
        // The sky at the horizon is exactly what the far floor fades into.
        let far = m.apply(
            [0.2, 0.15, 0.1],
            m.transmittance(200.0, EYE, 0.0),
            -EYE / 200.0,
        );
        let sky = m.colour(0.0);
        for k in 0..3 {
            assert!((far[k] - sky[k]).abs() < 0.01);
        }
    }
}

/// Offline scene preview: a CPU re-render of a Forest view (sky, textured ground,
/// spruces with the game's exact placement/sizing/mip choice) under the old and the new
/// fog model, so fog changes can be judged without running the game.
/// `cargo test -p labyrinth --release fog_preview -- --ignored` → `tmp/labyrinth_art/`.
#[cfg(test)]
mod preview {
    use super::*;
    use crate::maze;
    use crate::view::raycast::tree_spots;
    use crate::view::theme::{
        Canvas, LEVELS, SPRUCES, TREE_FOOT, TREE_SPAN, paint_forest_floor, paint_spruce,
    };
    use macroquad::prelude::*;

    const W: usize = 900;
    const H: usize = 720;

    #[derive(Clone, Copy, PartialEq)]
    enum Model {
        Old,
        New,
    }

    struct Tex {
        levels: Vec<Canvas>,
        sil: Vec<Vec<f32>>,
    }

    /// GPU-style bilinear sample of a straight-alpha level whose transparent texels
    /// carry `bleed`; returns rgb, alpha (and silhouette alpha if `sil`).
    fn bilinear(
        c: &Canvas,
        sil: Option<&[f32]>,
        u: f32,
        v: f32,
        bleed: [f32; 3],
        wrap: bool,
    ) -> ([f32; 3], f32) {
        let x = u * c.w as f32 - 0.5;
        let y = v * c.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let mut rgb = [0.0; 3];
        let mut a = 0.0;
        for (dx, dy, w) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let (mut xi, mut yi) = (x0 as i32 + dx, y0 as i32 + dy);
            if wrap {
                xi = xi.rem_euclid(c.w as i32);
                yi = yi.rem_euclid(c.h as i32);
            } else if xi < 0 || yi < 0 || xi >= c.w as i32 || yi >= c.h as i32 {
                for k in 0..3 {
                    rgb[k] += bleed[k] * w;
                }
                continue;
            }
            let i = yi as usize * c.w + xi as usize;
            let p = c.px[i];
            let (col, al) = match sil {
                Some(s) => ([1.0; 3], s[i]),
                None if p[3] <= 0.0 => (bleed, 0.0),
                None => ([p[0], p[1], p[2]], p[3]),
            };
            for k in 0..3 {
                rgb[k] += col[k] * w;
            }
            a += al * w;
        }
        (rgb, a)
    }

    /// Bilinear sample in premultiplied alpha (clamped to the level's edge, zero outside).
    fn bilinear_pm(c: &Canvas, u: f32, v: f32) -> ([f32; 3], f32) {
        let x = u * c.w as f32 - 0.5;
        let y = v * c.h as f32 - 0.5;
        let (x0, y0) = (x.floor(), y.floor());
        let (fx, fy) = (x - x0, y - y0);
        let mut rgb = [0.0; 3];
        let mut a = 0.0;
        for (dx, dy, w) in [
            (0, 0, (1.0 - fx) * (1.0 - fy)),
            (1, 0, fx * (1.0 - fy)),
            (0, 1, (1.0 - fx) * fy),
            (1, 1, fx * fy),
        ] {
            let (xi, yi) = (x0 as i32 + dx, y0 as i32 + dy);
            if xi < 0 || yi < 0 || xi >= c.w as i32 || yi >= c.h as i32 {
                continue;
            }
            let p = c.px[yi as usize * c.w + xi as usize];
            for k in 0..3 {
                rgb[k] += p[k] * p[3] * w;
            }
            a += p[3] * w;
        }
        (rgb, a)
    }

    /// A canvas's alpha grown by one texel (4-neighbour max), then 3x3 tent-blurred: the
    /// soft fog silhouette (see `Sprites`).
    fn soft_alpha(c: &Canvas) -> Vec<f32> {
        let (w, h) = (c.w as i32, c.h as i32);
        let at = |v: &[f32], x: i32, y: i32| {
            if x < 0 || y < 0 || x >= w || y >= h {
                0.0
            } else {
                v[(y * w + x) as usize]
            }
        };
        let alpha: Vec<f32> = c.px.iter().map(|p| p[3]).collect();
        let mut grown = vec![0.0f32; alpha.len()];
        for y in 0..h {
            for x in 0..w {
                grown[(y * w + x) as usize] = [(0, 0), (1, 0), (-1, 0), (0, 1), (0, -1)]
                    .iter()
                    .map(|&(dx, dy)| at(&alpha, x + dx, y + dy))
                    .fold(0.0, f32::max);
            }
        }
        let mut out = vec![0.0f32; alpha.len()];
        for y in 0..h {
            for x in 0..w {
                let mut s = 0.0;
                for dy in -1..=1 {
                    for dx in -1..=1 {
                        let k = (2 - dx * dx) * (2 - dy * dy);
                        s += at(&grown, x + dx, y + dy) * k as f32;
                    }
                }
                out[(y * w + x) as usize] = s / 16.0;
            }
        }
        out
    }

    fn old_lit(d: f32) -> f32 {
        (1.0 / (1.0 + 4.0 * (d * 0.14 + d * d * 0.02))).clamp(0.05, 1.0)
    }

    fn mix64(mut v: u64) -> u64 {
        v ^= v >> 30;
        v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v ^= v >> 27;
        v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
        v ^ (v >> 31)
    }

    struct Cam {
        pos: Vec2,
        dir: Vec2,
        plane: Vec2,
    }

    impl Cam {
        fn ray(&self, sx: f32) -> Vec2 {
            self.dir + self.plane * (2.0 * sx / W as f32 - 1.0)
        }
        fn project(&self, p: Vec2) -> Option<(f32, f32)> {
            let rel = p - self.pos;
            let inv = 1.0 / (self.plane.x * self.dir.y - self.dir.x * self.plane.y);
            let tx = inv * (self.dir.y * rel.x - self.dir.x * rel.y);
            let depth = inv * (-self.plane.y * rel.x + self.plane.x * rel.y);
            (depth > 0.15).then(|| (W as f32 * 0.5 * (1.0 + tx / depth), depth))
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render(
        model: Model,
        grid: &maze::WallGrid,
        cam: &Cam,
        trees: &[Tex],
        floor: &[Canvas],
        avg: [f32; 3],
        name: &str,
    ) {
        let m = FOREST;
        let hz = H as f32 * 0.5;
        let hf = H as f32;
        let mut img = vec![[0.0f32; 3]; W * H];
        let (ceil_top, ceil_bot) = ([0.52, 0.58, 0.53], [0.60, 0.66, 0.60]);
        let old_fog = [0.60, 0.66, 0.60];
        let path = [1.0, 0.94, 0.82];
        let under = [0.66, 0.78, 0.66];
        // Sky and ground.
        for y in 0..H {
            let yc = y as f32 + 0.5;
            for x in 0..W {
                let px = &mut img[y * W + x];
                if yc < hz {
                    *px = match model {
                        Model::Old => mix(ceil_top, ceil_bot, yc / hz),
                        Model::New => m.colour((hz - yc) / hf),
                    };
                    continue;
                }
                let d = EYE * hf / (yc - hz);
                let p = cam.pos + cam.ray(x as f32 + 0.5) * d;
                let sq = p.floor().as_ivec2();
                let tint = if grid.is_solid(sq.x, sq.y) {
                    under
                } else {
                    path
                };
                let fp = (128.0 * d * d / (EYE * hf)).max(1.0).log2().round() as usize;
                let lv = &floor[fp.min(floor.len() - 1)];
                let (t, _) = bilinear(lv, None, p.x / 4.0, p.y / 4.0, avg, true);
                let c = [0, 1, 2].map(|k| t[k] * tint[k]);
                *px = match model {
                    Model::Old => {
                        let lit = old_lit(d);
                        let a = if d > 18.0 {
                            0.0
                        } else {
                            lit * (1.0 - smoothstep(12.6, 18.0, d))
                        };
                        let back = mix(old_fog, [0, 1, 2].map(|k| avg[k] * path[k]), lit);
                        mix(back, c, a)
                    }
                    Model::New => {
                        let dist = (p - cam.pos).length();
                        let tr = m.transmittance((dist * dist + EYE * EYE).sqrt(), EYE, 0.0);
                        m.apply(c, tr, -EYE / dist)
                    }
                };
            }
        }
        // Trees, far to near.
        let range = match model {
            Model::Old => 10.0,
            Model::New => super::super::raycast::TREE_RANGE,
        };
        let r = range as i32 + 1;
        let (px0, py0) = (cam.pos.x as i32, cam.pos.y as i32);
        let mut list = Vec::new();
        for gy in (py0 - r).max(-3)..=(py0 + r).min(grid.gh as i32 + 2) {
            for gx in (px0 - r).max(-3)..=(px0 + r).min(grid.gw as i32 + 2) {
                if !grid.is_solid(gx, gy) {
                    continue;
                }
                for (wp, t) in tree_spots(gx, gy) {
                    if let Some((cx, depth)) = cam.project(wp)
                        && depth < range
                    {
                        list.push((depth, cx, t, wp));
                    }
                }
            }
        }
        crate::util::sort_by_key(&mut list, |t| -t.0);
        let blend = |img: &mut Vec<[f32; 3]>, x: usize, y: usize, c: [f32; 3], a: f32| {
            let p = &mut img[y * W + x];
            *p = mix(*p, c, a.clamp(0.0, 1.0));
        };
        // Shadows first.
        for &(depth, cx, _t, wp) in &list {
            let lat = hf / depth;
            let height = 3.0 + 2.5 * rf(_t, 1);
            let widen = 0.85 + 0.3 * rf(_t, 3);
            let qw = (height * 96.0 / 320.0).min(1.35) * TREE_SPAN * lat * widen;
            let ground = hz + EYE * lat;
            let eh = (ground - hz).max(0.0) * 0.45 / depth;
            let (col, a) = match model {
                Model::Old => {
                    let lit = old_lit(depth);
                    if lit <= 0.15 {
                        continue;
                    }
                    (mix(old_fog, [0.02, 0.04, 0.02], lit), 0.4 * lit)
                }
                Model::New => {
                    let dist = (wp - cam.pos).length();
                    let tr = m.transmittance((dist * dist + EYE * EYE).sqrt(), EYE, 0.0)
                        * range_fade(dist);
                    if tr <= 0.08 {
                        continue;
                    }
                    (m.apply([0.02, 0.04, 0.02], tr, -EYE / dist), 0.3 * tr)
                }
            };
            let rings: &[f32] = if model == Model::Old {
                &[1.0]
            } else {
                &[1.0, 0.72, 0.45]
            };
            for &k in rings {
                let rx = qw * 0.45 * k;
                let eh = eh * k;
                for y in (ground - eh).max(0.0) as usize..((ground + eh) as usize).min(H) {
                    for x in (cx - rx).max(0.0) as usize..((cx + rx) as usize).min(W) {
                        let (dx, dy) = ((x as f32 + 0.5 - cx) / rx, (y as f32 + 0.5 - ground) / eh);
                        if dx * dx + dy * dy <= 1.0 {
                            blend(&mut img, x, y, col, a / rings.len() as f32);
                        }
                    }
                }
            }
        }
        for &(depth, cx, t, wp) in &list {
            let lat = hf / depth;
            let height = 3.0 + 2.5 * rf(t, 1);
            let lean = (rf(t, 2) - 0.5) * 0.05;
            let widen = 0.85 + 0.3 * rf(t, 3);
            let qh = height * TREE_SPAN * lat;
            let qw = (height * 96.0 / 320.0).min(1.35) * TREE_SPAN * lat * widen;
            let ground = hz + EYE * lat;
            let y0 = ground + qh * TREE_FOOT - qh;
            let variant = (rf(t, 6) * SPRUCES as f32) as usize;
            let flip = rf(t, 7) < 0.5;
            let b = 0.8 + 0.2 * rf(t, 4);
            let warm = (rf(t, 5) - 0.5) * 0.16;
            let tint = [(b * (1.0 + warm)).min(1.0), b, (b * (1.0 - warm)).min(1.0)];
            let bias = match model {
                Model::Old => 0.06,
                Model::New => super::super::raycast::MIST_BLUR,
            };
            let lf = ((320.0 / (qh / (1.0 + bias * depth * depth)).max(1.0)).log2())
                .clamp(0.0, (LEVELS - 1) as f32);
            let level = (lf as usize).min(LEVELS - 2);
            let f = lf - level as f32;
            let fog_level = if f < 0.5 { level } else { level + 1 };
            let tex = &trees[variant];
            let dist = (wp - cam.pos).length();
            let (sn, cs) = lean.sin_cos();
            let ya = (y0 - qh * 0.1).max(0.0) as usize;
            let yb = ((y0 + qh * 1.1) as usize).min(H);
            let xa = (cx - qw).max(0.0) as usize;
            let xb = ((cx + qw) as usize).min(W);
            for y in ya..yb {
                for x in xa..xb {
                    // Undo the lean (rotation about the foot).
                    let (rx, ry) = (x as f32 + 0.5 - cx, y as f32 + 0.5 - ground);
                    let (lx, ly) = (cs * rx + sn * ry, -sn * rx + cs * ry);
                    let u = (lx + qw * 0.5) / qw;
                    let v = (ly + ground - y0) / qh;
                    if !(0.0..1.0).contains(&u) || !(0.0..1.0).contains(&v) {
                        continue;
                    }
                    let u = if flip { 1.0 - u } else { u };
                    let z = -ly / lat;
                    let (tr, fogc) = match model {
                        Model::Old => (old_lit(depth), old_fog),
                        Model::New => {
                            let d3 = (dist * dist + (z - EYE) * (z - EYE)).sqrt();
                            (
                                m.transmittance(d3, EYE, z.max(0.0)) * range_fade(dist),
                                m.colour((z - EYE) / dist),
                            )
                        }
                    };
                    if model == Model::New {
                        // Premultiplied layers, each fogged on its own: exact at soft edges.
                        let mut layer = |lv: usize, w: f32| {
                            let (pm, al) = bilinear_pm(&tex.levels[lv], u, v);
                            let p = &mut img[y * W + x];
                            for k in 0..3 {
                                p[k] = p[k] * (1.0 - al * w)
                                    + (pm[k] * tint[k] * tr + fogc[k] * (1.0 - tr) * al) * w;
                            }
                        };
                        if f > 0.02 {
                            layer(level + 1, 1.0);
                        }
                        if f < 0.98 {
                            layer(level, 1.0 - f);
                        }
                        continue;
                    }
                    let mut layer = |lv: usize, a: f32| {
                        let (c, al) =
                            bilinear(&tex.levels[lv], None, u, v, [0.03, 0.07, 0.05], false);
                        blend(&mut img, x, y, [0, 1, 2].map(|k| c[k] * tint[k]), al * a);
                    };
                    if f > 0.02 {
                        layer(level + 1, 1.0);
                    }
                    if f < 0.98 {
                        layer(level, 1.0 - f);
                    }
                    let (_, sa) = bilinear(
                        &tex.levels[fog_level],
                        Some(&tex.sil[fog_level]),
                        u,
                        v,
                        [1.0; 3],
                        false,
                    );
                    blend(&mut img, x, y, fogc, sa * (1.0 - tr));
                }
            }
        }
        let bytes: Vec<u8> = img
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 1.0].map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8))
            .collect();
        // `export_png` flips rows.
        let mut flipped = Vec::with_capacity(bytes.len());
        for row in bytes.chunks(W * 4).rev() {
            flipped.extend_from_slice(row);
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/labyrinth_art");
        std::fs::create_dir_all(&dir).unwrap();
        Image {
            bytes: flipped,
            width: W as u16,
            height: H as u16,
        }
        .export_png(dir.join(name).to_str().unwrap());
    }

    fn range_fade(dist: f32) -> f32 {
        let r = super::super::raycast::TREE_RANGE;
        1.0 - smoothstep(r * 0.65, r, dist)
    }

    fn rf(t: u64, i: u64) -> f32 {
        (mix64(t ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 40) as f32 / 16_777_216.0
    }

    #[test]
    #[ignore]
    fn fog_preview() {
        macroquad::rand::srand(7);
        let mz = maze::generate(14, 10, 0.3, 0, 0);
        let grid = mz.wall_grid();
        let seed = 0x5EED;
        let trees: Vec<Tex> = (0..SPRUCES)
            .map(|v| {
                let mut levels = vec![paint_spruce(seed ^ ((v as u64 + 1) * 0x51_7CC1))];
                for _ in 1..LEVELS {
                    let h = levels.last().unwrap().half();
                    levels.push(h);
                }
                let sil = levels.iter().map(soft_alpha).collect();
                Tex { levels, sil }
            })
            .collect();
        let mut floor = vec![paint_forest_floor(seed)];
        while floor.last().unwrap().w > 1 {
            let h = floor.last().unwrap().half();
            floor.push(h);
        }
        let avg = crate::view::theme::mean_rgb(&floor[0]);
        // The longest straight view down a walkway, and one looking half across it.
        let mut best = (0, vec2(1.5, 1.5), vec2(1.0, 0.0));
        for y in 0..mz.h as i32 {
            for x in 0..mz.w as i32 {
                let p = vec2(2.0 * x as f32 + 1.5, 2.0 * y as f32 + 1.5);
                for d in [
                    vec2(1.0, 0.0),
                    vec2(-1.0, 0.0),
                    vec2(0.0, 1.0),
                    vec2(0.0, -1.0),
                ] {
                    let mut n = 0;
                    while !grid.is_solid(
                        (p.x + d.x * (n + 1) as f32) as i32,
                        (p.y + d.y * (n + 1) as f32) as i32,
                    ) {
                        n += 1;
                    }
                    if n > best.0 {
                        best = (n, p, d);
                    }
                }
            }
        }
        let (_, pos, dir) = best;
        for (k, yaw) in [0.0f32, 0.45].into_iter().enumerate() {
            let dir = Vec2::from_angle(yaw).rotate(dir);
            let cam = Cam {
                pos,
                dir,
                plane: vec2(-dir.y, dir.x) * (W as f32 * 0.5 / H as f32),
            };
            render(
                Model::Old,
                &grid,
                &cam,
                &trees,
                &floor,
                avg,
                &format!("fog_old_{k}.png"),
            );
            render(
                Model::New,
                &grid,
                &cam,
                &trees,
                &floor,
                avg,
                &format!("fog_new_{k}.png"),
            );
        }
    }
}
