//! Animated effects: a live per-frame pass over the cached scene. Cheap, procedural,
//! per-theme, no per-frame allocation (the mote field is built once).
//!
//! Phase 5 ships light flicker, drifting motes, and the exit glow. The tower-climb
//! floor-transition wipe and the mirror-face sheen are coupled to phase 6's modes and
//! land with them.

use crate::view::theme::ThemeKind;
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Light {
    /// Warm, irregular — a torch.
    Torch,
    /// Near-white, mostly steady with an occasional hard dropout + buzz stutter.
    Fluorescent,
    /// Gentle canopy-dapple sway.
    Dapple,
    /// Steady daylight — no flicker.
    Steady,
}

#[derive(Clone, Copy)]
struct Mote {
    /// Screen-space fractions, 0..1.
    x: f32,
    y: f32,
    /// 0.2 (far, slow, small) .. 1.0 (near, fast, big).
    depth: f32,
    phase: f32,
}

const MOTE_CAP: usize = 48;

pub struct Fx {
    light: Light,
    tint: Color,
    mote_count: usize,
    motes: [Mote; MOTE_CAP],
}

impl Fx {
    pub fn build(kind: ThemeKind, seed: u64) -> Fx {
        let (light, tint, count) = match kind {
            ThemeKind::Backrooms => (Light::Fluorescent, Color::new(0.95, 0.93, 0.7, 1.0), 14),
            ThemeKind::Dungeon => (Light::Torch, Color::new(0.8, 0.7, 0.5, 1.0), 30),
            ThemeKind::Mine => (Light::Torch, Color::new(0.7, 0.65, 0.55, 1.0), 44),
            ThemeKind::Forest => (Light::Dapple, Color::new(0.85, 0.9, 0.55, 1.0), 26),
            ThemeKind::Garden => (Light::Steady, Color::new(1.0, 0.85, 0.9, 1.0), 22),
        };
        // Deterministic scatter (hash on `seed`), no global RNG.
        let mut motes = [Mote {
            x: 0.0,
            y: 0.0,
            depth: 0.5,
            phase: 0.0,
        }; MOTE_CAP];
        for (i, m) in motes.iter_mut().enumerate() {
            let h = |k: u64| {
                let mut v = seed ^ (i as u64).wrapping_mul(0x9E3779B97F4A7C15) ^ k;
                v = v.wrapping_mul(0xD1B54A32D192ED03);
                v ^= v >> 31;
                (v & 0xFFFF) as f32 / 0xFFFF as f32
            };
            *m = Mote {
                x: h(1),
                y: h(2),
                depth: 0.2 + 0.8 * h(3),
                phase: h(4) * std::f32::consts::TAU,
            };
        }
        Fx {
            light,
            tint,
            mote_count: count.min(MOTE_CAP),
            motes,
        }
    }

    /// Multiplier folded into the raycaster's distance shading — never a separate pass.
    /// `t` is wall-clock seconds (unaffected by `control.scale`, so 3x sim speed never
    /// turns this into a strobe).
    pub fn light_mul(&self, t: f64) -> f32 {
        let t = t as f32;
        match self.light {
            Light::Torch => {
                let f = (t * 11.0).sin() * 0.03 + (t * 23.7).sin() * 0.02 + (t * 3.1).sin() * 0.04;
                1.0 + f
            }
            Light::Fluorescent => {
                let buzz = (t * 120.0).sin() * 0.015;
                // Rare hard dropout: a brief dip roughly every ~7s.
                let phase = (t * 0.14).fract();
                let drop = if phase < 0.02 { -0.4 } else { 0.0 };
                (1.0 + buzz + drop).max(0.4)
            }
            Light::Dapple => 1.0 + (t * 1.7).sin() * 0.05 + (t * 0.9).cos() * 0.04,
            Light::Steady => 1.0,
        }
    }

    /// Drifting motes in screen space; nearer ones (higher `depth`) are bigger, faster
    /// and brighter — the depth spread is the parallax. `dt` is wall-clock.
    pub fn draw_motes(&mut self, area: Rect, dt: f32, t: f64) {
        let t = t as f32;
        for m in self.motes.iter_mut().take(self.mote_count) {
            m.y += dt * (0.01 + 0.05 * m.depth);
            if m.y > 1.05 {
                m.y -= 1.1;
            }
            let sway = ((t * 0.6 + m.phase).sin()) * 0.015 * m.depth;
            let px = area.x + (m.x + sway).rem_euclid(1.0) * area.w;
            let py = area.y + m.y * area.h;
            let r = (0.6 + 2.4 * m.depth) * (area.w / 900.0).max(0.6);
            let a = 0.10 + 0.22 * m.depth;
            let shimmer = 0.7 + 0.3 * (t * 4.0 + m.phase).sin();
            draw_circle(
                px,
                py,
                r,
                Color::new(self.tint.r, self.tint.g, self.tint.b, a * shimmer),
            );
        }
    }

    /// Tower climb: a rising light wipe between floors (`p` 0..1 over the hold). Peaks
    /// at the midpoint so the old floor washes out and the new one fades back in.
    pub fn draw_floor_wipe(&self, area: Rect, p: f32) {
        let a = (1.0 - (p * 2.0 - 1.0).abs()).clamp(0.0, 1.0);
        // A band sweeping up the screen plus an overall brighten.
        let band_y = area.y + area.h * (1.0 - p);
        draw_rectangle(
            area.x,
            area.y,
            area.w,
            area.h,
            Color::new(0.95, 0.93, 0.8, 0.55 * a),
        );
        draw_rectangle(
            area.x,
            band_y - 40.0,
            area.w,
            80.0,
            Color::new(1.0, 0.98, 0.9, 0.5 * a),
        );
    }

    /// Warm pulsing rim when the exit is currently in sight — the one unambiguous
    /// "payoff imminent" cue for a viewer. `near` in 0..1 scales it (closer == stronger).
    pub fn draw_exit_glow(&self, area: Rect, t: f64, near: f32) {
        if near <= 0.0 {
            return;
        }
        let pulse = 0.55 + 0.45 * ((t * 3.5).sin() as f32 * 0.5 + 0.5);
        let strength = (near * pulse).clamp(0.0, 1.0);
        let bands = 5;
        for i in 0..bands {
            let f = (i + 1) as f32 / bands as f32;
            let inset = f * 0.5 * area.h.min(area.w);
            let a = 0.12 * strength * (1.0 - f);
            draw_rectangle_lines(
                area.x + inset,
                area.y + inset,
                area.w - inset * 2.0,
                area.h - inset * 2.0,
                6.0,
                Color::new(1.0, 0.78, 0.35, a),
            );
        }
    }
}
