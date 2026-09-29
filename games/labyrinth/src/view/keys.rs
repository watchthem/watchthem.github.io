//! Dungeon keys: uncollected keys floating in their corridors, drawn after the walls
//! *and* the dressing, far to near, and clipped column by column — by the walls
//! (`View::visible`) and by any nearer dressing (`Occluders`). Drawn before the dressing
//! (the old way, inside `raycast::draw`), a barrel or rack *behind* a key painted over
//! it; and clipped by its centre column only, a key half round a corner showed whole
//! over the wall, then vanished all at once.

use crate::maze::{Cell, WallGrid};
use crate::view::KEY_COLORS;
use crate::view::label::Occluders;
use crate::view::raycast::{COL_STEP, View, lit_at};
use crate::view::theme::Palette;
use macroquad::prelude::*;

/// Keys not yet `taken` (bit per key id), as spinning, bobbing iron-age keys in their
/// colours, each in a faint halo.
/// Each key adds its span to `front`, so the lamps drawn after (a torch on the wall
/// behind it) hide behind the key rather than painting over it.
pub fn draw(
    view: &View,
    front: &mut Occluders,
    pal: &Palette,
    light_mul: f32,
    keys: &[(Cell, u8)],
    taken: u32,
) {
    let mut sprites: Vec<(f32, f32, u8)> = keys
        .iter()
        .filter(|&&(_, k)| taken & (1 << k) == 0)
        .filter_map(|&(c, k)| {
            let (gx, gy) = WallGrid::cell_center(c);
            let (cx, depth) = view.project(vec2(gx as f32 + 0.5, gy as f32 + 0.5))?;
            Some((depth, cx, k))
        })
        .collect();
    crate::util::sort_by_key(&mut sprites, |t| -t.0);

    let area = view.area;
    let t = get_time() as f32;
    let mut spans: Vec<(usize, f32, f32, f32)> = Vec::new();
    for (depth, cx, key) in sprites {
        let h = view.px_per_unit(depth) * 0.26;
        if cx + h < area.x || cx - h > area.x + area.w {
            continue;
        }
        let cy = view.screen_y(0.4 + 0.03 * (t * 2.5 + key as f32).sin(), depth);
        // Column `ci` shows this key if no wall or nearer dressing stands there.
        let col_of = |x: f32| ((x - area.x) / COL_STEP).floor();
        let shows = |ci: f32| {
            let sx = area.x + (ci + 0.5) * COL_STEP;
            ci >= 0.0 && view.visible(sx, depth) && !front.hides(ci as usize, depth, cy)
        };
        // A rectangle, cut to the columns that show.
        let rect = |x: f32, y: f32, w: f32, hgt: f32, c: Color| {
            let (c0, c1) = (col_of(x), col_of(x + w));
            let mut ci = c0;
            while ci <= c1 {
                if shows(ci) {
                    let lo = x.max(area.x + ci * COL_STEP);
                    let hi = (x + w).min(area.x + (ci + 1.0) * COL_STEP);
                    if hi > lo {
                        draw_rectangle(lo, y, hi - lo, hgt, c);
                    }
                }
                ci += 1.0;
            }
        };

        let c = KEY_COLORS[key as usize % KEY_COLORS.len()];
        let lit = lit_at(depth, pal.haze, light_mul);
        let mix = |k: f32| {
            Color::new(
                c.r * k * lit + pal.fog.r * (1.0 - lit),
                c.g * k * lit + pal.fog.g * (1.0 - lit),
                c.b * k * lit + pal.fog.b * (1.0 - lit),
                1.0,
            )
        };
        // Halo: a disc, as one chord per column.
        let hr = h * 0.55;
        let halo = Color::new(c.r, c.g, c.b, 0.18 * lit);
        let mut ci = col_of(cx - hr);
        while ci <= col_of(cx + hr) {
            let (lo, hi) = (area.x + ci * COL_STEP, area.x + (ci + 1.0) * COL_STEP);
            let dx = (lo.max(cx - hr) + hi.min(cx + hr)) * 0.5 - cx;
            let half = (hr * hr - dx * dx).max(0.0).sqrt();
            if half > 0.0 {
                rect(lo, cy - half, COL_STEP, 2.0 * half, halo);
            }
            ci += 1.0;
        }
        // An old iron-age key, upright, turning slowly about its shaft: ring bow on
        // top, shaft, two bit teeth at the bottom. `s` is its turn — x offsets scale by
        // it, so the teeth swing round to the other side.
        let s = (t * 1.6 + key as f32).cos();
        let (bow_y, r) = (cy - h * 0.3, h * 0.17);
        let th = (h * 0.07).max(1.0);
        let rx = (r * s.abs()).max(th * 0.5);
        const SEGS: usize = 24;
        for i in 0..SEGS {
            let a0 = i as f32 / SEGS as f32 * std::f32::consts::TAU;
            let a1 = (i + 1) as f32 / SEGS as f32 * std::f32::consts::TAU;
            let (x0, y0) = (cx + rx * a0.cos(), bow_y + r * a0.sin());
            let (x1, y1) = (cx + rx * a1.cos(), bow_y + r * a1.sin());
            if shows(col_of((x0 + x1) * 0.5)) {
                draw_line(x0, y0, x1, y1, th, mix(1.0));
            }
        }
        let shaft = (h * 0.07 * s.abs()).max(1.0);
        rect(cx - shaft * 0.5, bow_y + r, shaft, h * 0.62, mix(0.85));
        let tooth = h * 0.16 * s;
        let (tx, tw) = if tooth >= 0.0 {
            (cx, tooth)
        } else {
            (cx + tooth, -tooth)
        };
        rect(tx, cy + h * 0.2, tw, h * 0.07, mix(0.8));
        rect(tx, cy + h * 0.33, tw * 0.7, h * 0.07, mix(0.8));
        // Its span for what's drawn later: the key itself, halo aside.
        let (top, bot) = (bow_y - r, cy + h * 0.4);
        let (x0, x1) = (
            col_of(cx - r.max(tooth.abs())),
            col_of(cx + r.max(tooth.abs())),
        );
        let mut ci = x0.max(0.0);
        while ci <= x1 {
            if shows(ci) {
                spans.push((ci as usize, depth, top, bot));
            }
            ci += 1.0;
        }
    }
    for (ci, depth, top, bot) in spans {
        if let Some(col) = front.0.get_mut(ci) {
            col.push((depth, top, bot));
        }
    }
}
