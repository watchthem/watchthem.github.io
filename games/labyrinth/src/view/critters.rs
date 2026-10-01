//! Wildlife, purely for the viewer: bunnies hopping about the mirror garden (one of
//! them the White Rabbit — down the rabbit hole, through the
//! looking-glass), and birds in the fog forest — crows gliding in over the treetops to
//! perch in a tree, cawing now and then, and flapping off again after a while or when
//! the bot comes close; owls on low branches, blinking and hooting. Their calls come
//! out as `Call`s for the sound layer (`take_calls`), positioned where the bird is.
//!
//! In the fog the bot leaves a trail of breadcrumbs (`drop_crumb`), and the crows come
//! down to eat it — Hansel and Gretel's forest, where the way back doesn't last. A crumb
//! also goes when the ground under it dissolves (its square turns solid). Pure
//! dressing: the forgetting itself is `Game`'s, by age.
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
/// A crow only comes down for a crumb at least this far from the bot, and within
/// `CRUMB_REACH` of it (so the feeding happens where the bot may look back and see it).
const CRUMB_SHY: f32 = 2.5;
const CRUMB_REACH: f32 = 8.0;
/// The trail's spacing: a crumb every this far along the bot's path.
const CRUMB_STEP: f32 = 0.22;
/// Most crumbs on the ground at once (a few dozen squares of trail); the oldest go first.
const MAX_CRUMBS: usize = 200;

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
    /// On the ground pecking at a crumb (`Critter::crumb`); `hop` counts down to the
    /// last peck.
    Feeding,
}

/// A breadcrumb on the forest floor.
#[derive(Clone, Copy)]
struct Crumb {
    id: u32,
    pos: Vec2,
    /// A crow has it (on its way down, or pecking).
    claimed: bool,
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
    /// Crows: the crumb (`Crumb::id`) it's coming down for or eating.
    crumb: Option<u32>,
}

pub struct Critters {
    theme: Option<ThemeKind>,
    rng: u64,
    list: Vec<Critter>,
    t: f32,
    calls: Vec<(Call, Vec2)>,
    crumbs: Vec<Crumb>,
    next_crumb: u32,
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
            crumbs: Vec::new(),
            next_crumb: 0,
        }
    }

    /// The bot drops a crumb where it stands (grid coordinates), if the trail has none
    /// within `CRUMB_STEP` — called every frame, so it follows the path the bot walks.
    pub fn drop_crumb(&mut self, at: Vec2) {
        if self.crumbs.iter().any(|c| c.pos.distance(at) < CRUMB_STEP) {
            return;
        }
        if self.crumbs.len() >= MAX_CRUMBS
            && let Some(i) = self.crumbs.iter().position(|c| !c.claimed)
        {
            self.crumbs.remove(i);
        }
        let jitter = vec2(self.range(-0.05, 0.05), self.range(-0.05, 0.05));
        self.crumbs.push(Crumb {
            id: self.next_crumb,
            pos: at + jitter,
            claimed: false,
        });
        self.next_crumb += 1;
    }

    /// A new maze: the old trail is gone.
    pub fn clear_crumbs(&mut self) {
        self.crumbs.clear();
    }

    /// The nearest unclaimed crumb to `from` a crow would come down for, if any.
    fn free_crumb(&self, from: Vec2, cam: Vec2) -> Option<usize> {
        self.crumbs
            .iter()
            .enumerate()
            .filter(|(_, k)| {
                let d = k.pos.distance(cam);
                !k.claimed && (CRUMB_SHY..=CRUMB_REACH).contains(&d)
            })
            .min_by(|a, b| a.1.pos.distance(from).total_cmp(&b.1.pos.distance(from)))
            .map(|(i, _)| i)
    }

    /// Send crow `c` down to crumb `i`.
    fn go_for_crumb(&mut self, c: &mut Critter, i: usize, speed: f32) {
        let k = &mut self.crumbs[i];
        k.claimed = true;
        c.crumb = Some(k.id);
        c.to = k.pos;
        c.vel = (k.pos - c.pos).normalize_or_zero() * speed;
        c.perch_z = 0.0;
        c.flight = Flight::Inbound;
    }

    fn crumb_index(&self, id: Option<u32>) -> Option<usize> {
        let id = id?;
        self.crumbs.iter().position(|k| k.id == id)
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
            self.crumbs.clear();
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
                        crumb: None,
                    });
                }
            }
        }
        self.t += dt;
        // Ground dissolved into forest takes its crumbs with it.
        self.crumbs.retain(|k| !grid.solid_at(k.pos));
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
                if let Some(i) = self.crumb_index(c.crumb.take()) {
                    self.crumbs[i].claimed = false;
                }
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
                c.crumb = None;
                if self.rand() < 0.6
                    && let Some(i) = self.free_crumb(perch, cam)
                {
                    let speed = c.vel.length();
                    self.go_for_crumb(&mut c, i, speed);
                }
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
                // Its crumb gone (eaten by another, or the ground dissolved): off again.
                if c.crumb.is_some() && self.crumb_index(c.crumb).is_none() {
                    c.crumb = None;
                    c.vel = c.vel.normalize_or_zero() * 2.4;
                    c.flight = Flight::Leaving;
                    return;
                }
                let step = c.vel.length() * dt;
                let rem = c.to - c.pos;
                if rem.length() <= step {
                    c.pos = c.to;
                    c.z = c.perch_z;
                    if c.crumb.is_some() {
                        // Faces the way it came in.
                        c.from = c.pos - c.vel.normalize_or_zero() * 0.3;
                        c.flight = Flight::Feeding;
                        c.hop = self.range(2.5, 4.5);
                    } else {
                        c.flight = Flight::Perched;
                        c.hop = self.range(10.0, 30.0);
                    }
                    c.vel = Vec2::ZERO;
                    c.call = self.range(0.5, 4.0);
                } else {
                    c.pos += rem.normalize() * step;
                    // Glides down over the last few units (never climbing: a hop from
                    // one crumb to the next stays low).
                    c.z =
                        c.z.min(c.perch_z + (CRUISE - c.perch_z) * (rem.length() / 4.0).min(1.0));
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
                    // Bored of the branch: down to the bot's trail, if there's any.
                    if !startled
                        && self.rand() < 0.7
                        && let Some(i) = self.free_crumb(c.pos, cam)
                    {
                        self.go_for_crumb(c, i, 2.0);
                        return;
                    }
                    self.take_off(c, cam, startled);
                }
            }
            Flight::Feeding => {
                let Some(i) = self.crumb_index(c.crumb) else {
                    c.crumb = None;
                    self.take_off(c, cam, false);
                    return;
                };
                if c.pos.distance(cam) < STARTLE * 1.3 {
                    // Scattered off the trail: the crumb's still there.
                    self.crumbs[i].claimed = false;
                    c.crumb = None;
                    self.calls.push((Call::Takeoff, c.pos));
                    c.since_call = 0.0;
                    self.calls.push((Call::Caw, c.pos));
                    self.take_off(c, cam, true);
                    return;
                }
                c.hop -= dt;
                if c.hop > 0.0 {
                    return;
                }
                // Eaten. The next crumb along, if it's close; else away.
                self.crumbs.remove(i);
                c.crumb = None;
                match self.free_crumb(c.pos, cam) {
                    Some(j) if self.crumbs[j].pos.distance(c.pos) < 2.5 => {
                        self.go_for_crumb(c, j, 1.2);
                    }
                    _ => {
                        self.calls.push((Call::Takeoff, c.pos));
                        self.take_off(c, cam, false);
                    }
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

    /// Crow `c` flies off — away from the bot if `startled`, else anywhere.
    fn take_off(&mut self, c: &mut Critter, cam: Vec2, startled: bool) {
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

    /// The breadcrumbs on the forest floor, drawn before the trees and critters (which
    /// stand over them). Each is a few pale flecks, faded by the mist.
    pub fn draw_crumbs(&self, view: &View, pal: &Palette, light: &Lighting) {
        for k in &self.crumbs {
            let Some((sx, depth)) = view.project(k.pos) else {
                continue;
            };
            if !view.visible(sx, depth) {
                continue;
            }
            let px = view.px_per_unit(depth);
            if px * 0.012 < 0.5 {
                continue;
            }
            let (lit, fog) = fade_at(view, pal, k.pos, 0.0, depth, light.base);
            let col = Color::new(
                0.86 * lit + fog.r * (1.0 - lit),
                0.74 * lit + fog.g * (1.0 - lit),
                0.52 * lit + fog.b * (1.0 - lit),
                1.0,
            );
            let y = view.screen_y(0.0, depth);
            let flat = ((y - view.horizon).max(0.0) / px).min(1.0) * 0.5;
            let h = (k.id as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            for f in 0..2u64 {
                let r = |s: u64| ((h >> (s * 8)) & 0xFF) as f32 / 255.0 - 0.5;
                let (u, v) = (r(f * 2) * 0.07, r(f * 2 + 1) * 0.07);
                // `v` runs along the view: deeper flecks sit higher and closer together.
                let (fx, fy) = (sx + u * px, y - v * px * flat);
                let size = (0.012 + 0.005 * r(f + 6)) * px;
                draw_ellipse(fx, fy, size, size * flat.max(0.2), 0.0, col);
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
            Kind::Crow if matches!(c.flight, Flight::Perched | Flight::Feeding) => c.pos - c.from,
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
            Kind::Crow if matches!(c.flight, Flight::Perched | Flight::Feeding) => {
                // On a branch: bobs its head now and then, beak open while it caws. On
                // the ground: pecks.
                let feeding = c.flight == Flight::Feeding;
                let z = c.z;
                let black = col([0.07, 0.07, 0.09]);
                if feeding {
                    shadow(0.07);
                } else {
                    let (a, b) = (at(-0.13, z - 0.004), at(0.13, z + 0.008));
                    draw_line(
                        a.x,
                        a.y,
                        b.x,
                        b.y,
                        (0.02 * px).max(1.0),
                        col([0.22, 0.15, 0.1]),
                    );
                }
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
                // A peck: the head drops to the ground and back, a few times a second
                // with a pause now and then to look round.
                let peck = if feeding && (t * 0.5).fract() < 0.7 {
                    (t * 5.0).sin().max(0.0).powi(2)
                } else {
                    0.0
                };
                let (hu, hz) = (0.055 + 0.03 * peck, z + 0.105 + bob - 0.085 * peck);
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
