//! Rendering. `minimap` is the knowledge renderer (phase 2). `raycast` (the
//! first-person view), `theme` and `fx` arrive in later phases.

pub mod boss;
pub mod critters;
pub mod decor;
pub mod fog;
pub mod fx;
pub mod hud;
pub mod keys;
pub mod label;
pub mod minimap;
pub mod raycast;
pub mod theme;

use macroquad::prelude::*;

pub const HEADER_H: f32 = 30.0;

/// Distinct colours per key id (Dungeon mode) — shared by the minimap and the 3D view
/// so a key and its door read as a pair in both.
pub const KEY_COLORS: [Color; 4] = [
    Color::new(1.0, 0.80, 0.20, 1.0),
    Color::new(0.35, 0.80, 1.0, 1.0),
    Color::new(0.55, 1.0, 0.45, 1.0),
    Color::new(1.0, 0.45, 0.75, 1.0),
];

/// The first-person view: the whole canvas — the header and the minimap float over it.
pub fn stage_rect() -> Rect {
    Rect::new(0.0, 0.0, screen_width(), screen_height())
}

/// Corner inset for the minimap, under the header.
pub fn minimap_inset() -> Rect {
    let s = 220.0;
    let m = 12.0;
    Rect::new(screen_width() - s - m, HEADER_H + m, s, s)
}
