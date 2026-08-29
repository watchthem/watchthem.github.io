//! The minimap: a pure render of `Knowledge`. Unknown space is blank, seen floor is
//! lit, seen walls are drawn, the frontier glows, spent dead ends are dimmed, and the
//! bot sits where `Knowledge` says it is. Nothing here reads `Maze`.
//!
//! North-up (the plan's starting choice — stable shape, easier to read the fill).

use crate::know::Knowledge;
use crate::maze::Cell;
use macroquad::prelude::*;

const UNSEEN: Color = Color::new(0.05, 0.05, 0.08, 1.0);
const FLOOR: Color = Color::new(0.16, 0.17, 0.24, 1.0);
const FLOOR_PRUNED: Color = Color::new(0.10, 0.10, 0.14, 1.0);
const WALL: Color = Color::new(0.55, 0.58, 0.72, 1.0);
const FRONTIER: Color = Color::new(0.30, 0.85, 0.55, 1.0);
const BOT: Color = Color::new(1.0, 0.82, 0.25, 1.0);
const EXIT: Color = Color::new(0.35, 0.70, 1.0, 1.0);
const START: Color = Color::new(0.5, 0.5, 0.6, 1.0);

/// Fog: a cell dims to its floor brightness over this many ticks of not being seen.
/// (The bot keeps the layout in memory — the maze may have mutated behind it, so the
/// faded cells are the ones its map could now be wrong about.)
const FOG_FADE_TICKS: f32 = 26.0;

/// Distinct colours per key id (Dungeon mode).
const KEY_COLORS: [Color; 4] = [
    Color::new(1.0, 0.80, 0.20, 1.0),
    Color::new(0.35, 0.80, 1.0, 1.0),
    Color::new(0.55, 1.0, 0.45, 1.0),
    Color::new(1.0, 0.45, 0.75, 1.0),
];

/// Cell pixel size + top-left origin so the `w x h` grid fits centered in `area`.
fn layout(k: &Knowledge, area: Rect) -> (f32, Vec2) {
    let cell = (area.w / k.w as f32)
        .min(area.h / k.h as f32)
        .floor()
        .max(1.0);
    let gw = cell * k.w as f32;
    let gh = cell * k.h as f32;
    let origin = vec2(area.x + (area.w - gw) * 0.5, area.y + (area.h - gh) * 0.5);
    (cell, origin)
}

/// Draws the static knowledge layer (everything that only changes on a reveal). The
/// live bot marker is drawn separately by `draw_bot` every frame on top.
pub fn draw(k: &Knowledge, area: Rect, time: f64, fog: bool) {
    let (cell, origin) = layout(k, area);
    let px = |c: Cell| vec2(origin.x + c.x as f32 * cell, origin.y + c.y as f32 * cell);
    // Fog mode: cells fade as they go stale (0 = fresh, 1 = about to be forgotten).
    let freshness = |c: Cell| -> f32 {
        if fog {
            (1.0 - k.age(c) as f32 / FOG_FADE_TICKS).clamp(0.15, 1.0)
        } else {
            1.0
        }
    };

    draw_rectangle(area.x, area.y, area.w, area.h, UNSEEN);

    // Floor tiles, shaded by Tremaux visit count.
    for y in 0..k.h as i32 {
        for x in 0..k.w as i32 {
            let c = Cell { x, y };
            if !k.cell_seen(c) {
                continue;
            }
            let p = px(c);
            let base = if k.pruned[k.idx(c)] {
                FLOOR_PRUNED
            } else {
                FLOOR
            };
            let v = (k.visits(c) as f32 * 0.14).min(0.5);
            let f = freshness(c);
            let col = Color::new(
                (base.r + v) * f,
                (base.g + v * 0.7) * f,
                (base.b + v * 0.3) * f,
                1.0,
            );
            draw_rectangle(p.x, p.y, cell, cell, col);
        }
    }

    // Frontier glow: seen cells that still border the unknown.
    let pulse = 0.55 + 0.45 * ((time * 3.0).sin() as f32 * 0.5 + 0.5);
    for y in 0..k.h as i32 {
        for x in 0..k.w as i32 {
            let c = Cell { x, y };
            if !k.is_frontier(c) {
                continue;
            }
            let p = px(c);
            let inset = cell * 0.28;
            draw_rectangle(
                p.x + inset,
                p.y + inset,
                cell - inset * 2.0,
                cell - inset * 2.0,
                Color::new(FRONTIER.r, FRONTIER.g, FRONTIER.b, 0.35 * pulse),
            );
        }
    }

    // Walls: any known-wall edge of a seen cell.
    let t = (cell * 0.12).clamp(1.0, 3.0);
    for y in 0..k.h as i32 {
        for x in 0..k.w as i32 {
            let c = Cell { x, y };
            if !k.cell_seen(c) {
                continue;
            }
            let p = px(c);
            if k.edge_open(c, 0) == Some(false) {
                draw_line(p.x, p.y, p.x + cell, p.y, t, WALL);
            }
            if k.edge_open(c, 2) == Some(false) {
                draw_line(p.x, p.y + cell, p.x + cell, p.y + cell, t, WALL);
            }
            if k.edge_open(c, 3) == Some(false) {
                draw_line(p.x, p.y, p.x, p.y + cell, t, WALL);
            }
            if k.edge_open(c, 1) == Some(false) {
                draw_line(p.x + cell, p.y, p.x + cell, p.y + cell, t, WALL);
            }
        }
    }

    // Locked doors: a bar across the edge, in the key's colour.
    for y in 0..k.h as i32 {
        for x in 0..k.w as i32 {
            let c = Cell { x, y };
            if !k.cell_seen(c) {
                continue;
            }
            let p = px(c);
            for d in 0..4usize {
                if let Some(key) = k.edge_locked(c, d) {
                    let col = KEY_COLORS[key as usize % KEY_COLORS.len()];
                    let bar = (cell * 0.22).max(2.0);
                    let (bx, by, bw, bh) = match d {
                        0 => (p.x, p.y - bar * 0.5, cell, bar),
                        2 => (p.x, p.y + cell - bar * 0.5, cell, bar),
                        3 => (p.x - bar * 0.5, p.y, bar, cell),
                        _ => (p.x + cell - bar * 0.5, p.y, bar, cell),
                    };
                    draw_rectangle(bx, by, bw, bh, col);
                }
            }
        }
    }

    // Phantom edges (mirror lies the bot currently believes): a faint cyan tick, so a
    // viewer can see where the bot is about to be fooled.
    for y in 0..k.h as i32 {
        for x in 0..k.w as i32 {
            let c = Cell { x, y };
            if !k.cell_seen(c) {
                continue;
            }
            let p = px(c);
            for d in 0..4usize {
                if !k.edge_phantom(c, d) {
                    continue;
                }
                let (mx, my) = match d {
                    0 => (p.x + cell * 0.5, p.y),
                    2 => (p.x + cell * 0.5, p.y + cell),
                    3 => (p.x, p.y + cell * 0.5),
                    _ => (p.x + cell, p.y + cell * 0.5),
                };
                draw_circle(
                    mx,
                    my,
                    (cell * 0.14).max(1.5),
                    Color::new(0.5, 0.9, 1.0, 0.5),
                );
            }
        }
    }

    // Keys spotted but not yet collected: a diamond in the key's colour.
    for &(kc, key) in &k.keys_seen {
        let p = px(kc);
        let col = KEY_COLORS[key as usize % KEY_COLORS.len()];
        let cx = p.x + cell * 0.5;
        let cy = p.y + cell * 0.5;
        let r = cell * 0.30;
        draw_triangle(vec2(cx, cy - r), vec2(cx + r, cy), vec2(cx, cy + r), col);
        draw_triangle(vec2(cx, cy - r), vec2(cx - r, cy), vec2(cx, cy + r), col);
    }

    // Start marker.
    let sp = px(Cell { x: 0, y: 0 });
    draw_rectangle_lines(sp.x + 2.0, sp.y + 2.0, cell - 4.0, cell - 4.0, 2.0, START);

    // Exit, once seen.
    if let Some(e) = k.exit_seen {
        let ep = px(e);
        let g = 0.6 + 0.4 * ((time * 4.0).sin() as f32 * 0.5 + 0.5);
        draw_rectangle(
            ep.x + 2.0,
            ep.y + 2.0,
            cell - 4.0,
            cell - 4.0,
            Color::new(EXIT.r, EXIT.g, EXIT.b, g),
        );
    }
}

/// The bot marker + facing wedge — drawn live every frame. `cell_pos` is fractional
/// cell coordinates (so the walk animation slides it between cells), `yaw` its facing
/// angle in the raycaster's world frame.
pub fn draw_bot(k: &Knowledge, area: Rect, cell_pos: Vec2, yaw: f32) {
    let (cell, origin) = layout(k, area);
    let c = vec2(
        origin.x + (cell_pos.x + 0.5) * cell,
        origin.y + (cell_pos.y + 0.5) * cell,
    );
    let r = (cell * 0.30).max(2.5);
    draw_circle(c.x, c.y, r, BOT);
    draw_line(
        c.x,
        c.y,
        c.x + yaw.cos() * cell * 0.6,
        c.y + yaw.sin() * cell * 0.6,
        2.0,
        Color::new(0.1, 0.1, 0.1, 1.0),
    );
}
