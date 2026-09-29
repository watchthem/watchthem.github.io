//! Wildlife, purely for the viewer: bunnies hopping about the mirror garden (one of
//! them the White Rabbit — down the rabbit hole, through the
//! looking-glass), and birds in the fog forest — crows gliding in over the treetops to
//! perch in a tree, cawing now and then, and flapping off again after a while or when
//! the bot comes close; owls on low branches, blinking and hooting. Their calls come
//! out as `Call`s for the sound layer (`take_calls`), positioned where the bird is.
//!
//! They live in world space around the camera: each is (re)spawned somewhere open
//! within a few cells, preferably out of sight (behind the camera or far into the
//! haze), and respawned once it strays too far — an endless population from a handful.
//! Randomness is their own xorshift, never `macroquad::rand` (the maze stream) — they
//! don't touch the game at all. Drawn as small procedural billboards, depth-tested at
//! their centre column; among Forest trees the caller sorts them in via
//! `raycast::Props::amid`.

use crate::maze::WallGrid;
use crate::view::fx::Lighting;
use crate::view::raycast::{View, fade_at, tree_spots};
use crate::view::theme::{Palette, ThemeKind};
use macroquad::prelude::*;
use std::f32::consts::PI;

/// Seconds per hop.
const HOP_SECS: f32 = 0.32;
/// A rat's dash, world units per second.
const RAT_SPEED: f32 = 2.4;
/// Bunnies and owls farther than this are respawned; crows past `FLY_GONE`.
const GONE: f32 = 9.0;
const FLY_GONE: f32 = 14.0;
/// Crows fly at this height, above the tallest spruces (5.5).
const CRUISE: f32 = 6.5;
/// A perched crow takes off when the bot comes this close.
const STARTLE: f32 = 1.6;
/// Clearance from any solid square: a bunny never hops into a hedge or a trunk.
const CLEAR: f32 = 0.14;

#[derive(Clone, Copy, PartialEq)]
enum Kind {
    Bunny,
    /// Dungeon rats: dash along the floor, stop, sniff, squeak.
    Rat,
    Crow,
    Owl,
}

#[derive(Clone, Copy, PartialEq)]
enum Flight {
    /// Gliding down to its perch (`to`, at `perch_z`).
    Inbound,
    Perched,
    /// Off and away, climbing back to `CRUISE`.
    Leaving,
}

/// A bird call, for the sound layer.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Call {
    Caw,
    /// Wings clattering as a crow takes off.
    Takeoff,
    /// A few wingbeats in flight.
    Flap,
    Hoot,
    /// A rat's squeak.
    Squeak,
    /// Tiny claws scrabbling as a rat dashes off.
    Scurry,
}

#[derive(Clone, Copy)]
struct Critter {
    kind: Kind,
    pos: Vec2,
    from: Vec2,
    to: Vec2,
    /// Bunnies: hop progress 0..1, negative = seconds of rest left. Perched crows:
    /// seconds until they fly off.
    hop: f32,
    /// Current height (perch / flight).
    z: f32,
    perch_z: f32,
    flight: Flight,
    /// Seconds to the next call; seconds since the last one (beak open).
    call: f32,
    since_call: f32,
    vel: Vec2,
    /// Per-critter clock offset (flaps, blinks, pecks).
    phase: f32,
    /// Bunnies: 0 is the White Rabbit.
    variant: u8,
}

pub struct Critters {
    theme: Option<ThemeKind>,
    rng: u64,
    list: Vec<Critter>,
    t: f32,
    calls: Vec<(Call, Vec2)>,
}

/// Open floor at `p` — among thin walls, the strips either side of a slab too, so
/// rats run the whole width of a corridor, right up to the walls.
fn open_at(grid: &WallGrid, p: Vec2) -> bool {
    !grid.solid_at(p)
}

/// Nothing solid on the way from `a` to `b`: a dash never goes through a thin slab.
fn path_clear(grid: &WallGrid, a: Vec2, b: Vec2) -> bool {
    let n = (a.distance(b) / 0.05).ceil().max(1.0) as i32;
    (1..=n).all(|i| open_at(grid, a.lerp(b, i as f32 / n as f32)))
}

/// Standing room: `p` and everything within `CLEAR` of it is open ground.
fn clear_at(grid: &WallGrid, p: Vec2) -> bool {
    [
        (0.0, 0.0),
        (CLEAR, 0.0),
        (-CLEAR, 0.0),
        (0.0, CLEAR),
        (0.0, -CLEAR),
    ]
    .iter()
    .all(|&(dx, dy)| open_at(grid, p + vec2(dx, dy)))
}

impl Critters {
    pub fn new(seed: u64) -> Critters {
        Critters {
            theme: None,
            rng: seed | 1,
            list: Vec::new(),
            t: 0.0,
            calls: Vec::new(),
        }
    }

    fn rand(&mut self) -> f32 {
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        (self.rng >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.rand()
    }

    /// Advance everything by `dt`; `cam`/`dir` place respawns out of sight.
    pub fn update(&mut self, dt: f32, theme: ThemeKind, grid: &WallGrid, cam: Vec2, dir: Vec2) {
        if self.theme != Some(theme) {
            self.theme = Some(theme);
            self.list.clear();
            let kinds: &[(Kind, usize)] = match theme {
                // One bunny — the White Rabbit, forever late — so never more than one in view.
                ThemeKind::Garden => &[(Kind::Bunny, 1)],
                ThemeKind::Dungeon => &[(Kind::Rat, 3)],
                ThemeKind::Forest => &[(Kind::Crow, 5), (Kind::Owl, 4)],
                _ => &[],
            };
            for &(kind, n) in kinds {
                for i in 0..n {
                    self.list.push(Critter {
                        kind,
                        pos: Vec2::splat(-1.0e4),
                        from: Vec2::ZERO,
                        to: Vec2::ZERO,
                        hop: 0.0,
                        z: 0.0,
                        perch_z: 0.0,
                        flight: Flight::Inbound,
                        call: 0.0,
                        since_call: 9.0,
                        vel: Vec2::ZERO,
                        phase: 0.0,
                        variant: i as u8,
                    });
                }
            }
        }
        self.t += dt;
        for i in 0..self.list.len() {
            let mut c = self.list[i];
            let dist = c.pos.distance(cam);
            let lost = match c.kind {
                Kind::Bunny | Kind::Rat => dist > GONE || !open_at(grid, c.pos),
                // A fog regen can clear the ground under a perch.
                Kind::Owl => {
                    let (x, y) = (c.from.x.floor() as i32, c.from.y.floor() as i32);
                    dist > GONE || !grid.is_solid(x, y)
                }
                Kind::Crow => dist > FLY_GONE,
            };
            if lost {
                match self.spawn(c, grid, cam, dir) {
                    Some(n) => c = n,
                    None => continue,
                }
            }
            self.animate(&mut c, dt, grid, cam);
            self.list[i] = c;
        }
    }

    /// A branch on a tree beside a path within a few cells of the camera: the tree's
    /// trunk position and the perch, nudged to the tree's path side. `hidden`: only
    /// out of sight (behind the camera, or deep in the haze).
    fn find_perch(
        &mut self,
        grid: &WallGrid,
        cam: Vec2,
        dir: Vec2,
        hidden: bool,
    ) -> Option<(Vec2, Vec2)> {
        for _ in 0..40 {
            let p = cam.floor() + vec2(self.range(-7.0, 8.0), self.range(-7.0, 8.0)).floor();
            let rel = p + Vec2::splat(0.5) - cam;
            if rel.length() < 2.5 || (hidden && rel.dot(dir) > 0.0 && rel.length() < 6.0) {
                continue;
            }
            let (sx, sy) = (p.x as i32, p.y as i32);
            if !grid.is_solid(sx, sy) || grid.out_of_bounds(sx, sy) {
                continue;
            }
            let Some(open) = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                .into_iter()
                .find(|&(dx, dy)| open_at(grid, p + vec2(dx as f32 + 0.5, dy as f32 + 0.5)))
            else {
                continue;
            };
            if let Some((tree, _)) = tree_spots(sx, sy).next() {
                return Some((tree, tree + vec2(open.0 as f32, open.1 as f32) * 0.12));
            }
        }
        None
    }

    fn spawn(&mut self, mut c: Critter, grid: &WallGrid, cam: Vec2, dir: Vec2) -> Option<Critter> {
        c.phase = self.range(0.0, 100.0);
        c.since_call = 9.0;
        match c.kind {
            Kind::Crow => {
                // Flies in from well off, gliding down to a perch near the bot.
                let (tree, perch) = self.find_perch(grid, cam, dir, false)?;
                let a = self.range(0.0, 2.0 * PI);
                c.from = tree;
                c.to = perch;
                c.pos = perch + vec2(a.cos(), a.sin()) * 11.0;
                c.vel = (perch - c.pos).normalize_or_zero() * self.range(1.8, 2.6);
                c.z = CRUISE;
                c.perch_z = self.range(2.2, 3.6);
                c.flight = Flight::Inbound;
                Some(c)
            }
            Kind::Owl => {
                let (tree, perch) = self.find_perch(grid, cam, dir, true)?;
                c.from = tree;
                c.pos = perch;
                c.z = self.range(1.0, 1.7);
                c.call = self.range(4.0, 25.0);
                Some(c)
            }
            Kind::Bunny | Kind::Rat => {
                for _ in 0..40 {
                    let p =
                        cam.floor() + vec2(self.range(-7.0, 8.0), self.range(-7.0, 8.0)).floor();
                    let rel = p + Vec2::splat(0.5) - cam;
                    // Out of sight: behind the camera, or deep in the haze.
                    if rel.length() < 2.5 || (rel.dot(dir) > 0.0 && rel.length() < 6.0) {
                        continue;
                    }
                    // Anywhere in the square: among thin walls, the strip beside a slab.
                    let q = p + vec2(self.range(0.1, 0.9), self.range(0.1, 0.9));
                    if clear_at(grid, q) {
                        c.pos = q;
                        c.from = q;
                        c.to = q;
                        c.hop = -self.range(0.2, 2.0);
                        c.call = self.range(2.0, 10.0);
                        return Some(c);
                    }
                }
                None
            }
        }
    }

    fn animate(&mut self, c: &mut Critter, dt: f32, grid: &WallGrid, cam: Vec2) {
        c.since_call += dt;
        match c.kind {
            Kind::Crow => self.fly(c, dt, grid, cam),
            Kind::Owl => {
                c.call -= dt;
                if c.call <= 0.0 {
                    c.call = self.range(18.0, 40.0);
                    c.since_call = 0.0;
                    self.calls.push((Call::Hoot, c.pos));
                }
            }
            Kind::Bunny | Kind::Rat => {
                let rat = c.kind == Kind::Rat;
                if c.hop < 0.0 {
                    c.hop += dt;
                    // Rats sniff about and squeak now and then while they're still.
                    if rat {
                        c.call -= dt;
                        if c.call <= 0.0 {
                            c.call = self.range(5.0, 14.0);
                            c.since_call = 0.0;
                            self.calls.push((Call::Squeak, c.pos));
                        }
                    }
                    if c.hop < 0.0 {
                        return;
                    }
                    // Off again: somewhere near, away from the bot if it's close.
                    let (lo, hi) = if rat { (0.4, 1.0) } else { (0.25, 0.5) };
                    let away = c.pos - cam;
                    let shy = if rat { 2.0 } else { 1.3 };
                    for _ in 0..6 {
                        let a = if away.length() < shy {
                            away.to_angle() + self.range(-0.8, 0.8)
                        } else {
                            self.range(0.0, 2.0 * PI)
                        };
                        let to = c.pos + Vec2::from_angle(a) * self.range(lo, hi);
                        if clear_at(grid, to) && path_clear(grid, c.pos, to) {
                            c.from = c.pos;
                            c.to = to;
                            c.hop = 0.0;
                            if rat {
                                self.calls.push((Call::Scurry, c.pos));
                            }
                            return;
                        }
                    }
                    c.hop = -self.range(0.3, 1.0);
                    return;
                }
                // A bunny hops at a fixed rate; a rat runs at a fixed speed.
                c.hop += if rat {
                    dt * RAT_SPEED / c.from.distance(c.to).max(0.05)
                } else {
                    dt / HOP_SECS
                };
                if c.hop >= 1.0 {
                    c.pos = c.to;
                    c.hop = if rat {
                        -self.range(0.4, 3.0)
                    } else {
                        -self.range(0.1, 1.8)
                    };
                } else {
                    c.pos = c.from.lerp(c.to, c.hop);
                }
            }
        }
    }

    fn fly(&mut self, c: &mut Critter, dt: f32, grid: &WallGrid, cam: Vec2) {
        match c.flight {
            Flight::Inbound => {
                let step = c.vel.length() * dt;
                let rem = c.to - c.pos;
                if rem.length() <= step {
                    c.pos = c.to;
                    c.z = c.perch_z;
                    c.vel = Vec2::ZERO;
                    c.flight = Flight::Perched;
                    c.hop = self.range(10.0, 30.0);
                    c.call = self.range(0.5, 4.0);
                } else {
                    c.pos += rem.normalize() * step;
                    // Glides down over the last few units.
                    c.z = c.perch_z + (CRUISE - c.perch_z) * (rem.length() / 4.0).min(1.0);
                    // Flapping on the way in; the glide at the end is silent.
                    c.call -= dt;
                    if c.call <= 0.0 && rem.length() > 4.0 {
                        c.call = self.range(0.5, 0.9);
                        self.calls.push((Call::Flap, c.pos));
                    }
                }
            }
            Flight::Perched => {
                c.call -= dt;
                if c.call <= 0.0 {
                    c.call = self.range(4.0, 14.0);
                    c.since_call = 0.0;
                    self.calls.push((Call::Caw, c.pos));
                }
                c.hop -= dt;
                let (tx, ty) = (c.from.x.floor() as i32, c.from.y.floor() as i32);
                let startled = c.pos.distance(cam) < STARTLE;
                if c.hop <= 0.0 || startled || !grid.is_solid(tx, ty) {
                    self.calls.push((Call::Takeoff, c.pos));
                    if startled || self.rand() < 0.4 {
                        c.since_call = 0.0;
                        self.calls.push((Call::Caw, c.pos));
                    }
                    let away = c.pos - cam;
                    let a = if startled {
                        away.to_angle() + self.range(-0.6, 0.6)
                    } else {
                        self.range(0.0, 2.0 * PI)
                    };
                    c.vel = Vec2::from_angle(a) * self.range(2.2, 3.0);
                    c.flight = Flight::Leaving;
                    c.call = 0.6;
                }
            }
            Flight::Leaving => {
                c.pos += c.vel * dt;
                c.z = (c.z + 2.5 * dt).min(CRUISE);
                // Climbing away: hard, steady wingbeats.
                c.call -= dt;
                if c.call <= 0.0 {
                    c.call = self.range(0.35, 0.5);
                    self.calls.push((Call::Flap, c.pos));
                }
            }
        }
    }

    /// The calls made since the last time, each with where it came from.
    pub fn take_calls(&mut self) -> Vec<(Call, Vec2)> {
        std::mem::take(&mut self.calls)
    }

    /// Where each one is, for sorting among the trees (`Props::amid`).
    pub fn points(&self) -> Vec<Vec2> {
        self.list.iter().map(|c| c.pos).collect()
    }

    /// Every critter's index and view depth.
    pub fn depths(&self, view: &View) -> Vec<(f32, usize)> {
        self.list
            .iter()
            .enumerate()
            .map(|(i, c)| (view.depth(c.pos), i))
            .collect()
    }

    /// Draw all of them, far to near.
    pub fn draw_all(&self, view: &View, pal: &Palette, light: &Lighting) {
        let mut d = self.depths(view);
        crate::util::sort_by_key(&mut d, |t| -t.0);
        for (_, i) in d {
            self.draw(i, view, pal, light);
        }
    }

    /// Draw critter `i`, if it's in front of the camera and the wall there.
    pub fn draw(&self, i: usize, view: &View, pal: &Palette, light: &Lighting) {
        let c = &self.list[i];
        let Some((sx, depth)) = view.project(c.pos) else {
            return;
        };
        if !view.visible(sx, depth) || view.behind_portal(c.pos, sx) {
            return;
        }
        let px = view.px_per_unit(depth);
        if px * 0.25 < 2.0 {
            return;
        }
        // Drawn as screen-space shapes, not column-clipped: in a mirror, only a critter
        // wholly inside the glass shows (it pops at the glass edge rather than
        // spilling over the hedge beside it).
        let reach = 0.13 * px;
        if view.reflected() && !(view.visible(sx - reach, depth) && view.visible(sx + reach, depth))
        {
            return;
        }
        let (lit, fog) = fade_at(view, pal, c.pos, c.z, depth, light.base);
        let g = view.glass_tint();
        let col = |rgb: [f32; 3]| {
            Color::new(
                rgb[0] * g[0] * lit + fog.r * (1.0 - lit),
                rgb[1] * g[1] * lit + fog.g * (1.0 - lit),
                rgb[2] * g[2] * lit + fog.b * (1.0 - lit),
                1.0,
            )
        };
        // Which way it faces on screen: its motion's screen direction.
        let heading = match c.kind {
            Kind::Crow if c.flight == Flight::Perched => c.pos - c.from,
            Kind::Crow if c.flight == Flight::Inbound => c.to - c.pos,
            Kind::Crow => c.vel,
            _ => c.to - c.from,
        };
        let face = match view.project(c.pos + heading.normalize_or_zero() * 0.3) {
            Some((hx, _)) if hx < sx => -1.0,
            _ => 1.0,
        };
        let t = self.t + c.phase;
        // Screen point of `(u, z)`: `u` along its facing, `z` up.
        let at = |u: f32, z: f32| vec2(sx + u * px * face, view.screen_y(z, depth));
        let circle = |u: f32, z: f32, r: f32, color: Color| {
            let p = at(u, z);
            draw_circle(p.x, p.y, r * px, color);
        };
        let ellipse = |u: f32, z: f32, rx: f32, rz: f32, rot: f32, color: Color| {
            let p = at(u, z);
            draw_ellipse(p.x, p.y, rx * px, rz * px, rot * face, color);
        };
        let tri = |a: (f32, f32), b: (f32, f32), cc: (f32, f32), color: Color| {
            draw_triangle(at(a.0, a.1), at(b.0, b.1), at(cc.0, cc.1), color);
        };
        let shadow = |r: f32| {
            let g = at(0.0, 0.0);
            let flat = ((g.y - view.horizon).max(0.0) / px).min(1.0) * 0.3;
            draw_ellipse(
                g.x,
                g.y,
                r * px,
                r * px * flat,
                0.0,
                Color::new(0.0, 0.0, 0.0, 0.3 * lit),
            );
        };
        let h = if c.hop >= 0.0 {
            (c.hop * PI).sin()
        } else {
            0.0
        };

        match c.kind {
            Kind::Rat => {
                // Nose down, tail trailing; runs flat (no hop).
                shadow(0.06);
                let (fur, pink) = (col([0.28, 0.24, 0.22]), col([0.75, 0.52, 0.5]));
                let running = c.hop >= 0.0;
                let sniff = if running {
                    0.0
                } else {
                    0.006 * (t * 14.0).sin()
                };
                // The tail: a thin curve behind.
                let mut prev = at(-0.07, 0.02);
                for i in 1..=6 {
                    let k = i as f32 / 6.0;
                    let p = at(
                        -0.07 - 0.13 * k,
                        0.02 - 0.015 * k + 0.01 * (t * 6.0 + k * 3.0).sin() * k,
                    );
                    draw_line(prev.x, prev.y, p.x, p.y, (0.006 * px).max(1.0), pink);
                    prev = p;
                }
                ellipse(0.0, 0.035, 0.075, 0.032, 0.0, fur);
                ellipse(0.055 + sniff, 0.032, 0.035, 0.022, 0.25, fur);
                tri(
                    (0.07 + sniff, 0.04),
                    (0.115 + sniff, 0.024),
                    (0.07 + sniff, 0.018),
                    fur,
                );
                circle(0.117 + sniff, 0.024, 0.005, pink);
                circle(0.05, 0.058, 0.012, pink);
                circle(0.075 + sniff, 0.04, 0.004, col([0.02, 0.02, 0.02]));
                if running {
                    // Legs a blur under the body.
                    let (a, b) = (at(-0.04, 0.004), at(0.04, 0.004));
                    draw_line(a.x, a.y, b.x, b.y, (0.008 * px).max(1.0), fur);
                }
            }
            Kind::Bunny => {
                let z = h * 0.13;
                shadow(0.09);
                let (fur, ear_in) = match c.variant % 3 {
                    0 => ([0.96, 0.95, 0.93], [0.95, 0.7, 0.72]),
                    1 => ([0.55, 0.45, 0.36], [0.85, 0.62, 0.6]),
                    _ => ([0.72, 0.68, 0.64], [0.88, 0.66, 0.66]),
                };
                // Mid-hop the body stretches out; resting it hunches, ears twitch.
                let stretch = 1.0 + h * 0.25;
                let twitch = if c.hop < 0.0 {
                    (t * 3.0).sin().max(0.0) * 0.15
                } else {
                    -0.35 * h
                };
                ellipse(-0.04, z + 0.055, 0.05, 0.05, 0.0, col(fur));
                ellipse(0.0, z + 0.07, 0.085 * stretch, 0.055, 0.0, col(fur));
                circle(-0.095, z + 0.08, 0.022, col([1.0, 1.0, 1.0]));
                ellipse(0.055, z + 0.2, 0.013, 0.055, -0.25 + twitch, col(fur));
                ellipse(0.08, z + 0.2, 0.013, 0.055, 0.1 + twitch, col(fur));
                ellipse(0.08, z + 0.2, 0.006, 0.04, 0.1 + twitch, col(ear_in));
                circle(0.075, z + 0.12, 0.04, col(fur));
                circle(0.092, z + 0.128, 0.008, col([0.08, 0.05, 0.05]));
            }
            Kind::Crow if c.flight == Flight::Perched => {
                // On a branch: bobs its head now and then, beak open while it caws.
                let z = c.z;
                let black = col([0.07, 0.07, 0.09]);
                let (a, b) = (at(-0.13, z - 0.004), at(0.13, z + 0.008));
                draw_line(
                    a.x,
                    a.y,
                    b.x,
                    b.y,
                    (0.02 * px).max(1.0),
                    col([0.22, 0.15, 0.1]),
                );
                for u in [-0.01, 0.015] {
                    let (a, b) = (at(u, z + 0.03), at(u, z));
                    draw_line(
                        a.x,
                        a.y,
                        b.x,
                        b.y,
                        (0.006 * px).max(1.0),
                        col([0.2, 0.2, 0.2]),
                    );
                }
                tri((-0.04, z + 0.06), (-0.12, z + 0.0), (-0.1, z - 0.02), black);
                ellipse(0.0, z + 0.065, 0.065, 0.038, 0.35, black);
                let cawing = c.since_call < 0.6;
                let bob = if cawing {
                    0.012 * (c.since_call * 25.0).sin().abs()
                } else {
                    0.008 * ((t * 1.7).sin() > 0.9) as u8 as f32
                };
                let (hu, hz) = (0.055, z + 0.105 + bob);
                circle(hu, hz, 0.027, black);
                let gape = if cawing { 0.012 } else { 0.0 };
                let beak = col([0.25, 0.25, 0.27]);
                tri(
                    (hu + 0.02, hz + 0.008),
                    (hu + 0.06, hz + gape),
                    (hu + 0.02, hz - 0.002),
                    beak,
                );
                tri(
                    (hu + 0.02, hz - 0.002),
                    (hu + 0.055, hz - gape),
                    (hu + 0.02, hz - 0.012),
                    beak,
                );
                circle(hu + 0.01, hz + 0.008, 0.005, col([0.8, 0.8, 0.8]));
            }
            Kind::Crow => {
                // In flight: faster wingbeats climbing away, a glide coming in.
                let rate = if c.flight == Flight::Leaving {
                    13.0
                } else {
                    7.0
                };
                let flap = (t * rate).sin();
                let black = col([0.06, 0.06, 0.08]);
                let z = c.z + 0.03 * (t * 1.3).sin();
                tri(
                    (0.0, z + 0.01),
                    (0.1, z + 0.005),
                    (-0.03, z + 0.26 * flap),
                    col([0.04, 0.04, 0.05]),
                );
                tri((-0.12, z), (-0.22, z + 0.025), (-0.21, z - 0.03), black);
                ellipse(0.0, z, 0.12, 0.03, 0.0, black);
                circle(0.12, z + 0.01, 0.03, black);
                tri(
                    (0.14, z + 0.015),
                    (0.19, z + 0.005),
                    (0.14, z - 0.005),
                    col([0.25, 0.25, 0.27]),
                );
                tri(
                    (-0.05, z + 0.01),
                    (0.07, z + 0.01),
                    (0.0, z + 0.3 * flap),
                    black,
                );
            }
            Kind::Owl => {
                let z = c.z;
                let brown = col([0.42, 0.3, 0.2]);
                let bark = col([0.22, 0.15, 0.1]);
                // The branch it sits on.
                let (a, b) = (at(-0.14, z - 0.005), at(0.14, z + 0.01));
                draw_line(a.x, a.y, b.x, b.y, (0.022 * px).max(1.0), bark);
                ellipse(0.0, z + 0.095, 0.065, 0.095, 0.0, brown);
                ellipse(0.0, z + 0.07, 0.042, 0.06, 0.0, col([0.7, 0.6, 0.45]));
                // Head turns now and then; blinks every few seconds.
                let look = (t * 0.4).sin().clamp(-0.6, 0.6) * 0.015;
                tri(
                    (-0.058 + look, z + 0.16),
                    (-0.03 + look, z + 0.18),
                    (-0.05 + look, z + 0.215),
                    brown,
                );
                tri(
                    (0.058 + look, z + 0.16),
                    (0.03 + look, z + 0.18),
                    (0.05 + look, z + 0.215),
                    brown,
                );
                let face_col = col([0.78, 0.68, 0.52]);
                circle(-0.028 + look, z + 0.155, 0.032, face_col);
                circle(0.028 + look, z + 0.155, 0.032, face_col);
                let blink = (t * 0.7).fract() < 0.05;
                for u in [-0.028, 0.028] {
                    if blink {
                        let (a, b) = (
                            at(u + look - 0.016, z + 0.155),
                            at(u + look + 0.016, z + 0.155),
                        );
                        draw_line(a.x, a.y, b.x, b.y, (0.005 * px).max(1.0), bark);
                    } else {
                        // Eyes catch what light there is: they stay bright in the haze.
                        circle(
                            u + look,
                            z + 0.155,
                            0.018,
                            Color::new(1.0, 0.8, 0.15, 0.4 + 0.6 * lit),
                        );
                        circle(u + look * 1.4, z + 0.155, 0.009, col([0.02, 0.02, 0.02]));
                    }
                }
                tri(
                    (look - 0.008, z + 0.14),
                    (look + 0.008, z + 0.14),
                    (look, z + 0.118),
                    col([0.3, 0.25, 0.2]),
                );
            }
        }
    }
}
