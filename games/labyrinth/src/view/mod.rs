//! Rendering. `minimap` is the knowledge renderer (phase 2). `raycast` (the
//! first-person view), `theme` and `fx` arrive in later phases.

pub mod minimap;
pub mod raycast;

use macroquad::prelude::*;

pub const HEADER_H: f32 = 30.0;

/// Full-canvas rect below the header — where the first-person view will live. Until the
/// raycaster exists (phase 3) the minimap fills it.
pub fn stage_rect() -> Rect {
    Rect::new(0.0, HEADER_H, screen_width(), screen_height() - HEADER_H)
}

/// Corner inset for the minimap once the 3D view is in (phase 3). Unused in phase 2.
pub fn minimap_inset() -> Rect {
    let s = 220.0;
    let m = 12.0;
    Rect::new(screen_width() - s - m, HEADER_H + m, s, s)
}
