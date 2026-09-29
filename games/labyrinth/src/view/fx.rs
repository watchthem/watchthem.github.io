//! Animated effects: a live per-frame pass over the cached scene. Cheap, procedural,
//! per-theme, no per-frame allocation (the mote field is built once and lives in world
//! space, drawn through the raycaster's `View`).
//!
//! Light flicker, drifting motes, the tower-climb floor wipe — and the
//! maze's own light sources (`Lamp`): Backrooms ceiling panels (a few of them blinking),
//! Dungeon wall torches (some burnt out). A lamp lights the walls near it (`Lighting`,
//! folded into the raycaster's shading per strip) and is what the ambient sound listens
//! to (`blinked_on`, `torch_level`).

use crate::maze::{Cell, DIRS, Maze, WallGrid};
use crate::view::label::Occluders;
use crate::view::raycast::{self, COL_STEP, View};
use crate::view::theme::ThemeKind;
use macroquad::prelude::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Light {
    /// Warm, irregular — a torch.
    Torch,
    /// Near-white and steady; the faulty `Blinky` panels stutter on their own.
    Fluorescent,
    /// Gentle canopy-dapple sway.
    Dapple,
    /// Steady daylight — no flicker.
    Steady,
}

/// A particle hanging in the air of the maze, in world space: it stays put as the camera
/// walks past it (drifting and settling on its own), rather than riding along on the
/// screen.
#[derive(Clone, Copy)]
struct Mote {
    /// World position within a `MOTE_BOX`-sized tile, wrapped around the camera — the
    /// field is effectively infinite without ever respawning a particle.
    x: f32,
    y: f32,
    /// Height, 0 (floor) .. 1 (ceiling).
    z: f32,
    phase: f32,
}

/// Side of the world-space tile the mote field repeats over, in grid units (a cell is
/// 2 grid units, so ~5 cells each way — past that the haze has eaten them anyway).
const MOTE_BOX: f32 = 10.0;

const MOTE_CAP: usize = 160;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LampKind {
    /// Backrooms ceiling panel, steady.
    Panel,
    /// Backrooms ceiling panel with a failing ballast: steady, then a stutter burst.
    Blinky,
    /// Dungeon wall torch.
    Torch,
    /// A torch that has run out of fuel: cold, a thread of smoke, no light.
    Spent,
}

#[derive(Clone, Copy)]
struct Lamp {
    kind: LampKind,
    /// World position (grid units); z is implied by the kind.
    pos: Vec2,
    /// Per-lamp 0..1 hash: desynchronises flicker/blink.
    s: f32,
    on: bool,
    /// Blinky only: went dark -> lit this frame (the ballast's buzz).
    switched_on: bool,
}

/// Lamps farther than this (grid units) neither light, draw nor sound.
const LAMP_RANGE: f32 = 14.0;
/// A torch's crackle dies out this far along the route (grid units; 2 per cell).
const TORCH_HEARD: f32 = 7.0;

/// This frame's lighting: a scene-wide multiplier (`base`, the old single flicker) and
/// the lamps near the camera, sampled per wall strip by `at`.
pub struct Lighting {
    pub base: f32,
    ambient: f32,
    /// `(pos, gain, radius)`: gain > 0 brightens, < 0 (a blinky panel gone dark) dims.
    points: Vec<(Vec2, f32, f32)>,
    /// Backrooms ceiling panels near the camera, `(centre, lit)` — drawn as part of the
    /// drop ceiling (`raycast::draw_office_ceiling`).
    pub panels: Vec<(Vec2, bool)>,
}

impl Lighting {
    /// Light multiplier on a wall at world point `p`.
    pub fn at(&self, p: Vec2) -> f32 {
        let mut l = self.ambient;
        for &(q, gain, r) in &self.points {
            let k = (1.0 - p.distance(q) / r).max(0.0);
            l += gain * k * k;
        }
        (self.base * l).max(0.05)
    }
}

/// Cheap deterministic 0..1 hash of a float pair.
fn h01(a: f32, b: f32) -> f32 {
    ((a * 12.9898 + b * 78.233).sin() * 43_758.547)
        .fract()
        .abs()
}

/// A blinky panel's state at `t`: lit, except for a ~0.7s stutter once per 3.5-7.5s.
fn blink_on(t: f32, s: f32) -> bool {
    let period = 3.5 + 4.0 * s;
    let local = (t + s * 17.0).rem_euclid(period);
    if local > 0.7 {
        return true;
    }
    let slot = (local / 0.05).floor();
    h01(slot + (t / period).floor() * 31.0, s) > 0.5
}

/// Torch flame strength around 1.0.
fn torch_flick(t: f32, s: f32) -> f32 {
    let p = s * 40.0;
    1.0 + (t * 11.0 + p).sin() * 0.10
        + (t * 23.7 + p * 1.3).sin() * 0.06
        + (t * 3.1 + p).sin() * 0.08
}

fn lamps_for(kind: ThemeKind, maze: &Maze, seed: u64) -> Vec<Lamp> {
    let hash = |c: Cell, k: u64| {
        let mut v =
            seed ^ ((c.x as u64) << 32 | c.y as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ k;
        v = v.wrapping_mul(0xD1B5_4A32_D192_ED03);
        v ^= v >> 29;
        (v & 0xFFFF) as f32 / 65_535.0
    };
    let center = |c: Cell| {
        let (gx, gy) = WallGrid::cell_center(c);
        vec2(gx as f32 + 0.5, gy as f32 + 0.5)
    };
    let mut lamps = Vec::new();
    for y in 0..maze.h as i32 {
        for x in 0..maze.w as i32 {
            let c = Cell { x, y };
            match kind {
                // The lift car has its own ceiling light (decor), behind its doors.
                ThemeKind::Backrooms if c == maze.exit && maze.lift_door().is_some() => {}
                // One panel per cell, like a real drop ceiling; ~1 in 9 blinks.
                ThemeKind::Backrooms => lamps.push(Lamp {
                    kind: if hash(c, 1) < 0.11 {
                        LampKind::Blinky
                    } else {
                        LampKind::Panel
                    },
                    pos: center(c),
                    s: hash(c, 2),
                    on: true,
                    switched_on: false,
                }),
                // Torches on ~1 in 3.5 wall faces; a fifth of them burnt out.
                ThemeKind::Dungeon => {
                    for (d, &(dx, dy)) in DIRS.iter().enumerate() {
                        if maze.is_open(c, d) || maze.is_mirror(c, d) {
                            continue;
                        }
                        // The stairwell's portal: a torch on each jamb instead.
                        if c == maze.exit && maze.stairwell == Some(d) {
                            let across = vec2(-dy as f32, dx as f32);
                            for (i, side) in [-0.36f32, 0.36].into_iter().enumerate() {
                                lamps.push(Lamp {
                                    kind: LampKind::Torch,
                                    pos: center(c)
                                        + vec2(dx as f32, dy as f32) * (maze.wall_face() - 0.08)
                                        + across * side,
                                    s: hash(c, 40 + i as u64),
                                    on: true,
                                    switched_on: false,
                                });
                            }
                            continue;
                        }
                        if hash(c, 10 + d as u64) >= 0.28 {
                            continue;
                        }
                        let spent = hash(c, 20 + d as u64) < 0.2;
                        lamps.push(Lamp {
                            kind: if spent {
                                LampKind::Spent
                            } else {
                                LampKind::Torch
                            },
                            // Just proud of the wall's face.
                            pos: center(c) + vec2(dx as f32, dy as f32) * (maze.wall_face() - 0.05),
                            s: hash(c, 30 + d as u64),
                            on: !spent,
                            switched_on: false,
                        });
                    }
                }
                ThemeKind::Forest | ThemeKind::Garden => {}
            }
        }
    }
    lamps
}

pub struct Fx {
    light: Light,
    lamps: Vec<Lamp>,
    tint: Color,
    mote_count: usize,
    /// Settling speed, world units (floor->ceiling = 1) per second.
    fall: f32,
    motes: [Mote; MOTE_CAP],
}

impl Fx {
    pub fn build(kind: ThemeKind, seed: u64, maze: &Maze) -> Fx {
        let (light, tint, count, fall) = match kind {
            ThemeKind::Backrooms => (
                Light::Fluorescent,
                Color::new(0.95, 0.93, 0.7, 1.0),
                50,
                0.015,
            ),
            ThemeKind::Dungeon => (Light::Torch, Color::new(0.8, 0.7, 0.5, 1.0), 110, 0.03),
            ThemeKind::Forest => (Light::Dapple, Color::new(0.85, 0.9, 0.55, 1.0), 90, 0.02),
            ThemeKind::Garden => (Light::Steady, Color::new(1.0, 0.85, 0.9, 1.0), 70, 0.05),
        };
        // Deterministic scatter (hash on `seed`), no global RNG.
        let mut motes = [Mote {
            x: 0.0,
            y: 0.0,
            z: 0.5,
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
                x: h(1) * MOTE_BOX,
                y: h(2) * MOTE_BOX,
                z: h(3),
                phase: h(4) * std::f32::consts::TAU,
            };
        }
        Fx {
            light,
            lamps: lamps_for(kind, maze, seed),
            tint,
            mote_count: if kind.indoors() {
                0
            } else {
                count.min(MOTE_CAP)
            },
            fall,
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
            // Healthy panels hold dead steady; only the `Blinky` ones misbehave, and only
            // locally (`lighting`). A global mains hum here (19Hz) aliased against the
            // frame rate into a visible whole-scene pulse.
            Light::Fluorescent | Light::Steady => 1.0,
            Light::Dapple => 1.0 + (t * 1.7).sin() * 0.05 + (t * 0.9).cos() * 0.04,
        }
    }

    /// Advance the lamps to wall-clock `t`: blinky panels switch, `switched_on` flags
    /// the frame one comes back on.
    pub fn update_lamps(&mut self, t: f64) {
        let t = t as f32;
        for l in self.lamps.iter_mut().filter(|l| l.kind == LampKind::Blinky) {
            let on = blink_on(t, l.s);
            l.switched_on = on && !l.on;
            l.on = on;
        }
    }

    /// The frame's lighting as seen from `cam`. Dungeon walls sit in near-dark between
    /// torches; a spent torch leaves its stretch of wall dark.
    pub fn lighting(&self, t: f64, cam: Vec2) -> Lighting {
        let tf = t as f32;
        let ambient = if self.light == Light::Torch { 0.5 } else { 1.0 };
        let points = self
            .lamps
            .iter()
            .filter(|l| l.pos.distance(cam) < LAMP_RANGE)
            .filter_map(|l| match l.kind {
                LampKind::Torch => Some((l.pos, 0.95 * torch_flick(tf, l.s), 4.5)),
                LampKind::Blinky if !l.on => Some((l.pos, -0.6, 3.0)),
                _ => None,
            })
            .collect();
        let panels = self
            .lamps
            .iter()
            .filter(|l| matches!(l.kind, LampKind::Panel | LampKind::Blinky))
            .filter(|l| l.pos.distance(cam) < LAMP_RANGE)
            .map(|l| (l.pos, l.on))
            .collect();
        Lighting {
            base: self.light_mul(t),
            ambient,
            points,
            panels,
        }
    }

    /// Audible distances (`hear`: grid point -> distance, e.g. `sound::Ear`) of blinky
    /// panels that flickered back on this frame.
    pub fn blinked_on<'a>(
        &'a self,
        hear: impl Fn(Vec2) -> f32 + 'a,
    ) -> impl Iterator<Item = f32> + 'a {
        self.lamps
            .iter()
            .filter(|l| l.switched_on)
            .map(move |l| hear(l.pos))
            .filter(|&d| d < LAMP_RANGE)
    }

    /// 0..1: how loud burning torches should be at audible distances `hear` — nearest
    /// ones dominate, spent ones are silent.
    /// Is any lamp (lit or spent) within `r` of `p`? Keeps set dressing off a wall a
    /// torch already hangs on.
    pub fn lamp_near(&self, p: Vec2, r: f32) -> bool {
        self.lamps.iter().any(|l| l.pos.distance(p) < r)
    }

    pub fn torch_level(&self, hear: impl Fn(Vec2) -> f32) -> f32 {
        self.lamps
            .iter()
            .filter(|l| l.kind == LampKind::Torch)
            .map(|l| {
                let k = (1.0 - hear(l.pos) / TORCH_HEARD).max(0.0);
                k * k
            })
            .sum::<f32>()
            .min(1.0)
    }

    /// Wall torches, depth-tested against the walls and against the set dressing drawn
    /// before them (`front`: `Decor::front`). (Ceiling panels are part of the ceiling,
    /// drawn before everything else: `Lighting::panels`.)
    pub fn draw_lamps(&self, view: &View, front: &Occluders, haze: f32, fog: Color, t: f64) {
        let t = t as f32;
        let mut near: Vec<(f32, f32, &Lamp)> = self
            .lamps
            .iter()
            .filter(|l| matches!(l.kind, LampKind::Torch | LampKind::Spent))
            .filter(|l| l.pos.distance(view.pos) < LAMP_RANGE)
            .filter_map(|l| view.project(l.pos).map(|(sx, d)| (d, sx, l)))
            .collect();
        crate::util::sort_by_key(&mut near, |t| -t.0);
        for (depth, sx, l) in near {
            let ci = ((sx - view.area.x) / COL_STEP).floor().max(0.0) as usize;
            if view.visible(sx, depth)
                && !view.behind_portal(l.pos, sx)
                && !front.hides(ci, depth, view.screen_y(0.6, depth))
            {
                draw_torch(view, sx, depth, l, haze, fog, t);
            }
        }
    }

    /// Motes hanging in the maze air: each settles slowly and sways, projected through
    /// the frame's camera and hidden behind walls, so walking past them parallaxes
    /// correctly. `dt` is wall-clock.
    /// Let the motes settle by `dt`.
    pub fn update_motes(&mut self, dt: f32) {
        for m in self.motes.iter_mut().take(self.mote_count) {
            m.z -= dt * self.fall * (0.6 + 0.4 * m.phase.sin().abs());
            if m.z < 0.0 {
                m.z += 1.0;
            }
        }
    }

    /// Each mote's floor-plane position at time `t`: the copy of its tile position
    /// nearest the camera at `cam`, swaying.
    fn mote_at(&self, m: &Mote, cam: Vec2, t: f32) -> Vec2 {
        let sway = (t * 0.5 + m.phase).sin() * 0.15;
        let wrap =
            |v: f32, c: f32| c + (v - c + MOTE_BOX * 0.5).rem_euclid(MOTE_BOX) - MOTE_BOX * 0.5;
        vec2(wrap(m.x + sway, cam.x), wrap(m.y, cam.y))
    }

    /// Where every mote is (for sorting them among the forest's trees).
    pub fn mote_points(&self, cam: Vec2, t: f64) -> Vec<Vec2> {
        let t = t as f32;
        self.motes
            .iter()
            .take(self.mote_count)
            .map(|m| self.mote_at(m, cam, t))
            .collect()
    }

    /// The motes in view this frame as `(depth, dot)`, for drawing with `draw_mote` —
    /// in any order, or interleaved with the trees by depth.
    pub fn mote_dots(&self, view: &View, t: f64) -> Vec<(f32, MoteDot)> {
        let t = t as f32;
        let area = view.area;
        let mut out = Vec::new();
        for m in self.motes.iter().take(self.mote_count) {
            let wp = self.mote_at(m, view.pos, t);
            let Some((sx, depth)) = view.project(wp) else {
                continue;
            };
            if !(area.x..area.x + area.w).contains(&sx) || !view.visible(sx, depth) {
                continue;
            }
            let sy = view.screen_y(m.z, depth);
            if !(area.y..area.y + area.h).contains(&sy) {
                continue;
            }
            let near = (1.0 - depth / (MOTE_BOX * 0.5)).clamp(0.0, 1.0);
            let r = (view.px_per_unit(depth) * 0.008).clamp(0.6, 3.5);
            let shimmer = 0.7 + 0.3 * (t * 4.0 + m.phase).sin();
            let a = (0.08 + 0.35 * near) * shimmer;
            let color = Color::new(self.tint.r, self.tint.g, self.tint.b, a);
            out.push((
                depth,
                MoteDot {
                    at: vec2(sx, sy),
                    r,
                    color,
                },
            ));
        }
        out
    }

    /// Settle and draw every mote in view (no trees to sort them among).
    pub fn draw_motes(&mut self, view: &View, dt: f32, t: f64) {
        self.update_motes(dt);
        for (_, d) in self.mote_dots(view, t) {
            draw_mote(&d);
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
}

/// One mote, projected: where on screen, how big, how bright.
pub struct MoteDot {
    at: Vec2,
    r: f32,
    color: Color,
}

pub fn draw_mote(d: &MoteDot) {
    draw_circle(d.at.x, d.at.y, d.r, d.color);
}

fn fogged(c: Color, lit: f32, fog: Color) -> Color {
    Color::new(
        c.r * lit + fog.r * (1.0 - lit),
        c.g * lit + fog.g * (1.0 - lit),
        c.b * lit + fog.b * (1.0 - lit),
        c.a,
    )
}

/// A wall torch: iron bracket, and either a live flame with its glow or, spent, a
/// charred head trailing a thread of smoke.
fn draw_torch(view: &View, cx: f32, depth: f32, l: &Lamp, haze: f32, fog: Color, t: f32) {
    let px = view.px_per_unit(depth);
    let lit = raycast::lit_at(depth, haze, 1.0);
    let y = |z: f32| view.screen_y(z, depth);
    let stick_w = (0.035 * px).max(1.0);
    draw_rectangle(
        cx - stick_w * 0.5,
        y(0.66),
        stick_w,
        y(0.50) - y(0.66),
        fogged(Color::new(0.22, 0.14, 0.08, 1.0), lit, fog),
    );
    let cup_w = (0.08 * px).max(2.0);
    let head = if l.kind == LampKind::Spent {
        Color::new(0.05, 0.04, 0.04, 1.0)
    } else {
        Color::new(0.16, 0.14, 0.13, 1.0)
    };
    draw_rectangle(
        cx - cup_w * 0.5,
        y(0.69),
        cup_w,
        y(0.65) - y(0.69),
        fogged(head, lit, fog),
    );

    if l.kind == LampKind::Spent {
        for i in 0..4 {
            let k = (t * 0.35 + i as f32 / 4.0 + l.s).fract();
            let sx = cx + (k * 6.0 + l.s * 9.0).sin() * 0.03 * px;
            let r = (0.012 + 0.03 * k) * px;
            draw_circle(
                sx,
                y(0.70 + k * 0.28),
                r,
                Color::new(0.45, 0.45, 0.45, 0.22 * (1.0 - k) * lit),
            );
        }
        return;
    }

    // Emissive: the haze barely dims a flame.
    let glow = raycast::lit_at(depth, haze * 0.3, 1.0);
    let f = torch_flick(t, l.s);
    // Soft halo: stacked faint discs, densest at the flame.
    for (r, a) in [(0.3, 0.035), (0.2, 0.05), (0.11, 0.08)] {
        draw_circle(cx, y(0.75), r * px, Color::new(1.0, 0.6, 0.2, a * f * glow));
    }
    let h = 0.17 * (0.8 + 0.2 * (t * 13.0 + l.s * 30.0).sin()) * f;
    let sway = (t * 9.0 + l.s * 20.0).sin() * 0.018 * px;
    let base = y(0.685);
    for (w, hk, c) in [
        (0.06, 1.0, Color::new(1.0, 0.42, 0.08, 0.9 * glow)),
        (0.04, 0.7, Color::new(1.0, 0.72, 0.2, 0.95 * glow)),
        (0.02, 0.42, Color::new(1.0, 0.95, 0.7, glow)),
    ] {
        let hw = w * 0.5 * px;
        draw_triangle(
            vec2(cx - hw, base),
            vec2(cx + hw, base),
            vec2(cx + sway * hk, y(0.685 + h * hk)),
            c,
        );
    }
}
