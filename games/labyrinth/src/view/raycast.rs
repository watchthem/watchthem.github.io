//! Pseudo-3D first-person view: a DDA ray per screen column against the maze's dense
//! `WallGrid`, drawn as textured vertical strips sampled from the active `Theme`.
//! Floor and ceiling are two flat gradient rects (theme palette), not per-pixel cast.

use crate::maze::WallGrid;
use crate::view::theme::Theme;
use macroquad::prelude::*;

/// One ray per two horizontal pixels — ~450 columns at 900px.
const COL_STEP: f32 = 2.0;
/// Camera plane half-width -> horizontal FOV (~60deg at 0.66).
const FOV: f32 = 0.66;

/// `pos` is in grid units, `yaw` radians (+x east, +y south), `bob` a small vertical
/// head-bob offset in strip-height fraction.
pub fn draw(grid: &WallGrid, theme: &Theme, area: Rect, pos: Vec2, yaw: f32, bob: f32) {
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

        let (perp, side, wall_x) = cast(grid, pos, ray);
        let line_h = area.h / perp.max(0.0001);
        let full_top = horizon - line_h * 0.5;
        let y0 = full_top.max(area.y);
        let y1 = (horizon + line_h * 0.5).min(area.y + area.h);
        if y1 <= y0 {
            sx += COL_STEP;
            continue;
        }

        // Fisheye-corrected distance already; fog + face shading are multiplicative.
        let lit = (1.0 / (1.0 + perp * 0.14 + perp * perp * 0.02)).clamp(0.05, 1.0);
        let face = if side == 0 { pal.ew_tint } else { 1.0 };
        let k = lit * face;
        let tint = Color::new(k, k, k, 1.0);

        let src_x = (wall_x * tw).clamp(0.0, tw - 1.0);
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
        // Distance fog as a translucent wash — darkening alone can't tint toward the
        // theme's fog color.
        draw_rectangle(
            sx,
            y0,
            COL_STEP + 0.6,
            y1 - y0,
            Color::new(pal.fog.r, pal.fog.g, pal.fog.b, 1.0 - lit),
        );
        sx += COL_STEP;
    }
}

/// DDA to the first solid grid cell. Returns `(perpendicular distance, side, wall_x)`
/// where `side` 0 == an x-facing wall (E/W), 1 == a y-facing wall (N/S), and `wall_x`
/// in `0..1` is the hit position along the wall face (texture u).
fn cast(grid: &WallGrid, pos: Vec2, ray: Vec2) -> (f32, i32, f32) {
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
    let wall_x = if side == 0 {
        (pos.y + perp * ray.y).fract()
    } else {
        (pos.x + perp * ray.x).fract()
    };
    let wall_x = if wall_x < 0.0 { wall_x + 1.0 } else { wall_x };
    (perp, side, wall_x)
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
