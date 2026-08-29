//! Pseudo-3D first-person view: a DDA ray per screen column against the maze's dense
//! `WallGrid`, drawn as vertical strips. Floor and ceiling are two flat gradient rects,
//! not per-pixel cast. Flat per-face colors here; procedural textures are phase 4.

use crate::maze::WallGrid;
use macroquad::prelude::*;

/// One ray per two horizontal pixels — ~450 columns at 900px.
const COL_STEP: f32 = 2.0;
/// Camera plane half-width -> horizontal FOV (~60deg at 0.66).
const FOV: f32 = 0.66;

const CEIL_TOP: Color = Color::new(0.05, 0.05, 0.09, 1.0);
const CEIL_BOT: Color = Color::new(0.15, 0.15, 0.20, 1.0);
const FLOOR_TOP: Color = Color::new(0.20, 0.18, 0.16, 1.0);
const FLOOR_BOT: Color = Color::new(0.06, 0.06, 0.06, 1.0);
const WALL_NS: Color = Color::new(0.66, 0.64, 0.70, 1.0);
const WALL_EW: Color = Color::new(0.46, 0.45, 0.52, 1.0);
const FOG: Color = Color::new(0.05, 0.05, 0.07, 1.0);

/// `pos` is in grid units, `yaw` radians (+x east, +y south), `bob` a small vertical
/// head-bob offset in strip-height fraction.
pub fn draw(grid: &WallGrid, area: Rect, pos: Vec2, yaw: f32, bob: f32) {
    let dir = vec2(yaw.cos(), yaw.sin());
    let plane = vec2(-dir.y, dir.x) * FOV;

    let horizon = area.y + area.h * (0.5 + bob);

    // Ceiling + floor as vertical gradients, split at the (bobbing) horizon.
    v_gradient(area.x, area.y, area.w, horizon - area.y, CEIL_TOP, CEIL_BOT);
    v_gradient(
        area.x,
        horizon,
        area.w,
        area.y + area.h - horizon,
        FLOOR_TOP,
        FLOOR_BOT,
    );

    let mut sx = area.x;
    while sx < area.x + area.w {
        let camx = 2.0 * (sx - area.x) / area.w - 1.0;
        let ray = dir + plane * camx;

        let (perp, side) = cast(grid, pos, ray);
        // Wall strip height from perpendicular distance (fisheye-corrected already).
        let line_h = area.h / perp.max(0.0001);
        let mid = horizon;
        let y0 = (mid - line_h * 0.5).max(area.y);
        let y1 = (mid + line_h * 0.5).min(area.y + area.h);

        let base = if side == 0 { WALL_NS } else { WALL_EW };
        let shade = (1.0 / (1.0 + perp * 0.15 + perp * perp * 0.02)).clamp(0.06, 1.0);
        let col = Color::new(
            base.r * shade + FOG.r * (1.0 - shade),
            base.g * shade + FOG.g * (1.0 - shade),
            base.b * shade + FOG.b * (1.0 - shade),
            1.0,
        );
        draw_rectangle(sx, y0, COL_STEP + 0.5, y1 - y0, col);
        sx += COL_STEP;
    }
}

/// DDA to the first solid grid cell. Returns `(perpendicular distance, side)` where
/// `side` 0 == an x-facing wall (E/W), 1 == a y-facing wall (N/S).
fn cast(grid: &WallGrid, pos: Vec2, ray: Vec2) -> (f32, i32) {
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
        sdist.x - delta.x
    } else {
        sdist.y - delta.y
    };
    (perp.max(0.0001), side)
}

fn v_gradient(x: f32, y: f32, w: f32, h: f32, top: Color, bot: Color) {
    if h <= 0.0 {
        return;
    }
    let bands = 24;
    let bh = h / bands as f32;
    for i in 0..bands {
        let t = i as f32 / (bands - 1) as f32;
        let c = Color::new(
            top.r + (bot.r - top.r) * t,
            top.g + (bot.g - top.g) * t,
            top.b + (bot.b - top.b) * t,
            1.0,
        );
        draw_rectangle(x, y + bh * i as f32, w, bh + 1.0, c);
    }
}
