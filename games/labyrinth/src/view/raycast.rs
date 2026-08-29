//! Pseudo-3D first-person view: a DDA ray per screen column against the maze's dense
//! `WallGrid`, drawn as textured vertical strips sampled from the active `Theme`.
//! Floor and ceiling are two flat gradient rects (theme palette), not per-pixel cast.
//!
//! Mirror mode: a ray hitting a mirror face reflects once and keeps marching, so the
//! reflected corridor renders as if it continued — plus a specular sheen so a viewer
//! can spot the lie a moment before the bot walks into it.

use crate::maze::WallGrid;
use crate::view::theme::Theme;
use macroquad::prelude::*;

/// One ray per two horizontal pixels — ~450 columns at 900px.
const COL_STEP: f32 = 2.0;
/// Camera plane half-width -> horizontal FOV (~60deg at 0.66).
const FOV: f32 = 0.66;

struct RayHit {
    perp: f32,
    side: i32,
    wall_x: f32,
    /// `Some(distance to the glass)` if the ray bounced off a mirror on the way.
    mirror_at: Option<f32>,
}

/// `pos` is in grid units, `yaw` radians (+x east, +y south), `bob` a small vertical
/// head-bob offset in strip-height fraction.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    grid: &WallGrid,
    theme: &Theme,
    area: Rect,
    pos: Vec2,
    yaw: f32,
    bob: f32,
    light_mul: f32,
) {
    let dir = vec2(yaw.cos(), yaw.sin());
    let plane = vec2(-dir.y, dir.x) * FOV;
    let pal = theme.palette;

    let horizon = area.y + area.h * (0.5 + bob);

    v_gradient(
        area.x,
        area.y,
        area.w,
        horizon - area.y,
        pal.ceil_top,
        pal.ceil_bot,
    );
    v_gradient(
        area.x,
        horizon,
        area.w,
        area.y + area.h - horizon,
        pal.floor_far,
        pal.floor_near,
    );

    let tex = &theme.wall;
    let tw = tex.width();
    let th = tex.height();

    let mut sx = area.x;
    while sx < area.x + area.w {
        let camx = 2.0 * (sx - area.x) / area.w - 1.0;
        let ray = dir + plane * camx;

        let hit = cast(grid, pos, ray);
        let line_h = area.h / hit.perp.max(0.0001);
        let full_top = horizon - line_h * 0.5;
        let y0 = full_top.max(area.y);
        let y1 = (horizon + line_h * 0.5).min(area.y + area.h);
        if y1 <= y0 {
            sx += COL_STEP;
            continue;
        }

        let lit =
            (light_mul / (1.0 + hit.perp * 0.14 + hit.perp * hit.perp * 0.02)).clamp(0.05, 1.0);
        let face = if hit.side == 0 { pal.ew_tint } else { 1.0 };
        let k = lit * face;
        let tint = Color::new(k, k, k, 1.0);

        let src_x = (hit.wall_x * tw).clamp(0.0, tw - 1.0);
        let src_y0 = (y0 - full_top) / line_h * th;
        let src_y1 = (y1 - full_top) / line_h * th;
        draw_texture_ex(
            tex,
            sx,
            y0,
            tint,
            DrawTextureParams {
                dest_size: Some(vec2(COL_STEP + 0.6, y1 - y0)),
                source: Some(Rect::new(src_x, src_y0, 1.0, (src_y1 - src_y0).max(0.5))),
                ..Default::default()
            },
        );
        draw_rectangle(
            sx,
            y0,
            COL_STEP + 0.6,
            y1 - y0,
            Color::new(pal.fog.r, pal.fog.g, pal.fog.b, 1.0 - lit),
        );

        // Mirror sheen: a cool specular wash over the strips that pass through glass,
        // sized to the *mirror's* distance, not the reflected wall's.
        if let Some(md) = hit.mirror_at {
            let mh = area.h / md.max(0.0001);
            let my0 = (horizon - mh * 0.5).max(area.y);
            let my1 = (horizon + mh * 0.5).min(area.y + area.h);
            if my1 > my0 {
                let shimmer = 0.10 + 0.06 * ((sx * 0.05 + md * 2.0).sin());
                draw_rectangle(
                    sx,
                    my0,
                    COL_STEP + 0.6,
                    my1 - my0,
                    Color::new(0.65, 0.80, 0.95, shimmer.clamp(0.03, 0.20)),
                );
            }
        }
        sx += COL_STEP;
    }
}

/// DDA to the first solid cell; if that cell's hit face is a mirror, reflect once and
/// keep marching. Returns the (summed) perpendicular distance, the final wall's side
/// and texture-u, and how far the glass was if a bounce happened.
fn cast(grid: &WallGrid, pos: Vec2, ray: Vec2) -> RayHit {
    let (d1, side1, map1) = march(grid, pos, ray);
    if !grid.is_mirror(map1.x, map1.y) {
        return RayHit {
            perp: d1,
            side: side1,
            wall_x: wall_u(pos, ray, d1, side1),
            mirror_at: None,
        };
    }
    // Reflect the ray across the face we struck and march on from just past the glass.
    let mut r2 = ray;
    if side1 == 0 {
        r2.x = -r2.x;
    } else {
        r2.y = -r2.y;
    }
    let hitpos = pos + ray * d1 + r2 * 1e-3;
    let (d2, side2, _map2) = march(grid, hitpos, r2);
    RayHit {
        perp: d1 + d2,
        side: side2,
        wall_x: wall_u(hitpos, r2, d2, side2),
        mirror_at: Some(d1),
    }
}

fn march(grid: &WallGrid, pos: Vec2, ray: Vec2) -> (f32, i32, IVec2) {
    let mut map = ivec2(pos.x.floor() as i32, pos.y.floor() as i32);
    let delta = vec2(
        if ray.x == 0.0 {
            1e30
        } else {
            (1.0 / ray.x).abs()
        },
        if ray.y == 0.0 {
            1e30
        } else {
            (1.0 / ray.y).abs()
        },
    );
    let step_x = if ray.x < 0.0 { -1 } else { 1 };
    let step_y = if ray.y < 0.0 { -1 } else { 1 };
    let mut sdist = vec2(
        if ray.x < 0.0 {
            (pos.x - map.x as f32) * delta.x
        } else {
            (map.x as f32 + 1.0 - pos.x) * delta.x
        },
        if ray.y < 0.0 {
            (pos.y - map.y as f32) * delta.y
        } else {
            (map.y as f32 + 1.0 - pos.y) * delta.y
        },
    );
    let mut side = 0;
    for _ in 0..512 {
        if sdist.x < sdist.y {
            sdist.x += delta.x;
            map.x += step_x;
            side = 0;
        } else {
            sdist.y += delta.y;
            map.y += step_y;
            side = 1;
        }
        if grid.is_solid(map.x, map.y) {
            break;
        }
    }
    let perp = if side == 0 {
        (sdist.x - delta.x).max(0.0001)
    } else {
        (sdist.y - delta.y).max(0.0001)
    };
    (perp, side, map)
}

fn wall_u(pos: Vec2, ray: Vec2, perp: f32, side: i32) -> f32 {
    let u = if side == 0 {
        pos.y + perp * ray.y
    } else {
        pos.x + perp * ray.x
    }
    .fract();
    if u < 0.0 { u + 1.0 } else { u }
}

fn v_gradient(x: f32, y: f32, w: f32, h: f32, top: Color, bot: Color) {
    if h <= 0.0 {
        return;
    }
    let bands = 24;
    let bh = h / bands as f32;
    for i in 0..bands {
        let t = i as f32 / (bands - 1) as f32;
        let col = Color::new(
            top.r + (bot.r - top.r) * t,
            top.g + (bot.g - top.g) * t,
            top.b + (bot.b - top.b) * t,
            1.0,
        );
        draw_rectangle(x, y + bh * i as f32, w, bh + 1.0, col);
    }
}
