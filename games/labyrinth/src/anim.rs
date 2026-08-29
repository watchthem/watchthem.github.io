//! Movement animation state machine. A game step is *turn in place, then walk forward*
//! (project convention — animate the real motion, not a start->end lerp), with a slight
//! head-bob. The solver decides on the tick; the animation plays that decision out over
//! roughly the tick interval.
//!
//! `Idle -> Turn -> Walk -> (Bump) -> Idle`. `Bump` is unused until phase 6's mirror
//! maze (the bot walks into a phantom passage and recoils) but the shape is built now
//! so that lands as a state, not a rewrite.

use crate::maze::{Cell, DIRS, WallGrid};
use macroquad::prelude::*;
use std::f32::consts::{PI, TAU};

const TURN_SECS: f32 = 0.09;
const WALK_SECS: f32 = 0.16;
#[allow(dead_code)] // phase 6 (mirror maze)
const BUMP_SECS: f32 = 0.14;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    Turn,
    Walk,
    #[allow(dead_code)] // phase 6 (mirror maze)
    Bump,
}

/// Facing angle for a `DIRS` index, in the raycaster's world frame (+x east, +y south).
pub fn dir_yaw(d: usize) -> f32 {
    let (dx, dy) = DIRS[d];
    (dy as f32).atan2(dx as f32)
}

/// Shortest signed angular delta from `a` to `b`, in `(-PI, PI]`.
fn ang_diff(a: f32, b: f32) -> f32 {
    let mut d = (b - a) % TAU;
    if d > PI {
        d -= TAU;
    } else if d < -PI {
        d += TAU;
    }
    d
}

pub struct Anim {
    phase: Phase,
    t: f32,
    yaw_from: f32,
    yaw_to: f32,
    pos_from: Vec2,
    pos_to: Vec2,
    /// Grid coords, continuously interpolated — the raycaster camera reads these.
    pub pos: Vec2,
    pub yaw: f32,
    pub bob: f32,
}

fn cell_pos(c: Cell) -> Vec2 {
    let (gx, gy) = WallGrid::cell_center(c);
    vec2(gx as f32 + 0.5, gy as f32 + 0.5)
}

impl Anim {
    pub fn new(start: Cell, facing: usize) -> Self {
        let p = cell_pos(start);
        let y = dir_yaw(facing);
        Anim {
            phase: Phase::Idle,
            t: 0.0,
            yaw_from: y,
            yaw_to: y,
            pos_from: p,
            pos_to: p,
            pos: p,
            yaw: y,
            bob: 0.0,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// Snap to a cell/facing with no interpolation (new maze, reseed).
    pub fn reset(&mut self, at: Cell, facing: usize) {
        *self = Anim::new(at, facing);
    }

    /// Begin playing out a step: turn from the current facing to `dir`, then walk from
    /// `from` to the next cell. A pure-straight step skips the turn.
    pub fn begin_step(&mut self, from: Cell, dir: usize) {
        self.yaw_from = self.yaw;
        self.yaw_to = self.yaw + ang_diff(self.yaw, dir_yaw(dir));
        self.pos_from = cell_pos(from);
        self.pos_to = cell_pos(Cell {
            x: from.x + DIRS[dir].0,
            y: from.y + DIRS[dir].1,
        });
        self.t = 0.0;
        self.phase = if ang_diff(self.yaw, dir_yaw(dir)).abs() > 0.01 {
            Phase::Turn
        } else {
            Phase::Walk
        };
    }

    pub fn update(&mut self, dt: f32) {
        match self.phase {
            Phase::Idle => {
                self.bob *= (1.0 - dt * 8.0).max(0.0);
            }
            Phase::Turn => {
                self.t += dt;
                let k = (self.t / TURN_SECS).clamp(0.0, 1.0);
                self.yaw = self.yaw_from + (self.yaw_to - self.yaw_from) * ease(k);
                if k >= 1.0 {
                    self.yaw = self.yaw_to;
                    self.phase = Phase::Walk;
                    self.t = 0.0;
                }
            }
            Phase::Walk => {
                self.t += dt;
                let k = (self.t / WALK_SECS).clamp(0.0, 1.0);
                self.pos = self.pos_from.lerp(self.pos_to, ease(k));
                self.bob = (k * PI * 2.0).sin() * 0.06;
                if k >= 1.0 {
                    self.pos = self.pos_to;
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                }
            }
            Phase::Bump => {
                self.t += dt;
                let k = (self.t / BUMP_SECS).clamp(0.0, 1.0);
                // Nudge a third of the way and snap back.
                let push = (k * PI).sin() * 0.33;
                self.pos = self.pos_from.lerp(self.pos_to, push);
                if k >= 1.0 {
                    self.pos = self.pos_from;
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                }
            }
        }
    }
}

fn ease(k: f32) -> f32 {
    // smoothstep
    k * k * (3.0 - 2.0 * k)
}
