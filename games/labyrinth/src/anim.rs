//! Movement animation state machine. A game step is *turn in place, then walk forward*
//! (project convention — animate the real motion, not a start->end lerp), with a slight
//! head-bob. The solver decides on the tick; the animation plays that decision out over
//! roughly the tick interval.
//!
//! `Idle -> Turn -> Walk -> Idle` for a normal step; `Idle -> Turn -> Idle` when the
//! bot finds a mirror where it believed there was a passage (Mirror mode) — it turns to
//! face the glass and its own reflection is the tell, no extra lunge. Stepping through a
//! locked door for the first time inserts `Unlock` between them: the bot stands at the
//! door while the key turns, then the door swings and it walks through. An office door
//! (Tower mode) is the same pause with no key: the bot pushes, the door swings, and it
//! walks in once the door is mostly clear.
//!
//! `update` reports the moments sound and the door state hang off (`AnimEvent`) — a
//! footfall lands when the walk finishes, not when the solver decided to move.

use crate::maze::{Cell, DIRS, STAIR_RISE, STAIR_SILL, STAIR_STEPS, STAIR_TREAD, WallGrid};
use macroquad::prelude::*;
use std::f32::consts::{PI, TAU};

/// A quarter turn; bigger turns take longer (`turn_secs`).
const TURN_SECS: f32 = 0.3;
/// How far a foot lifts while shuffling round in place (stride units, walk = 1).
const TURN_STEP: f32 = 0.6;
/// One cell's walk: two strides (left, right), a footfall at the middle and the end.
const WALK_SECS: f32 = 0.56;
const STRIDES_PER_CELL: f32 = 2.0;
/// Standing at a locked door while the key turns.
pub const UNLOCK_SECS: f32 = 0.5;
/// A junction glance: turning the head toward a side corridor, then holding there long
/// enough to take in what's down it.
const LOOK_TURN: f32 = 0.45;
pub const LOOK_HOLD: f32 = 0.5;
/// Held long enough to read what's written on a wall.
pub const READ_HOLD: f32 = 0.7;
/// Looking into a room from its doorway before moving on: a glance at each side wall,
/// this far either side of straight in (rad), held this long (shorter than a read).
pub const PEEK_TURN: f32 = 0.55;
pub const PEEK_HOLD: f32 = 0.25;
/// ...from half a cell into the doorway, stepping there and back this fast.
const LEAN_REACH: f32 = 1.0;
const LEAN_SECS: f32 = 0.35;
/// Standing at an office door while it swings — shorter than the swing itself
/// (`SWING_SECS`): the panel moves away from the bot's path, so it can start walking
/// while the door finishes opening.
const PUSH_SECS: f32 = 0.35;
/// An office door's swing from shut to fully open — the view animates it.
pub const SWING_SECS: f32 = 0.55;
/// Dungeon's last move (`descend`): from the exit cell's centre through the arch and
/// down the stairwell's flight (`maze::STAIR_*`) to stand on tread `DESCEND_TREAD`,
/// over this long, the eye dropping a riser as each tread is reached.
const DESCEND_TREAD: usize = 6;
const DESCEND_SECS: f32 = 2.2;
/// First a look down the flight from the top, standing.
const PEER_SECS: f32 = 0.7;
const DESCEND_STRIDES: f32 = 7.0;
/// ...looking down the flight (`pitch`, a fraction of the view height), then easing
/// toward a shallower look ahead on the way down, so the drop shows.
const DESCEND_PITCH: f32 = 0.35;
const DESCEND_PITCH_LOW: f32 = 0.12;

/// From the exit cell's centre, the arch face `arch` away: how far to walk (to the
/// middle of `DESCEND_TREAD`).
fn descend_reach(arch: f32) -> f32 {
    arch + STAIR_SILL + STAIR_TREAD * (DESCEND_TREAD as f32 + 0.5)
}

/// How far below the floor the eye is `s` into the stairwell (from its arch face):
/// a riser per tread, each dropped over the first half of its tread as the foot lands.
fn stair_sink(s: f32) -> f32 {
    let u = (s - STAIR_SILL) / STAIR_TREAD;
    if u < 0.0 {
        return 0.0;
    }
    let n = (u.floor() as usize).min(STAIR_STEPS - 1);
    let f = if u.floor() as usize >= STAIR_STEPS {
        1.0
    } else {
        ease((u.fract() * 2.0).min(1.0))
    };
    STAIR_RISE * (n as f32 + f)
}

/// One glance in a scan: a world yaw to turn the head to, how long to hold it there
/// (`LOOK_HOLD`, `READ_HOLD`), and how far to look down while there (`pitch`'s units,
/// a fraction of the view height — 0 for a corridor or a sign, more for a tea table),
/// and how far to stoop (eye drop, world units) for something lower still.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Look {
    pub yaw: f32,
    pub hold: f32,
    pub pitch: f32,
    pub stoop: f32,
}

impl Look {
    /// Straight down corridor/wall `d`, level.
    pub fn dir(d: usize, hold: f32) -> Look {
        Look {
            yaw: dir_yaw(d),
            hold,
            pitch: 0.0,
            stoop: 0.0,
        }
    }
}

/// What stops the bot at a doorway before it walks through.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gate {
    /// A locked door: the key turns, then the door opens.
    Key,
    /// An office door: pushed open, no key.
    Push,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Phase {
    Idle,
    /// Looking down side corridors at a junction (`scan`).
    Look,
    Turn,
    Unlock,
    Walk,
    /// Down the stairwell's first steps (`descend`), no footfall at the end.
    Descend,
    /// A half step into a doorway to look round the room, or back out (`open`).
    Lean,
}

/// A moment in the animation the view reacts to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AnimEvent {
    /// The first foot landed, halfway across the cell.
    Step,
    /// The second foot landed: the bot is on the new cell.
    Footfall,
    /// The bot, at a locked door, starts turning its key.
    KeyTurn,
    /// The key has turned: the door opens and the bot walks through.
    DoorOpen,
    /// The bot pushes an office door: it starts to swing.
    DoorPush,
}

/// Facing angle for a `DIRS` index, in the raycaster's world frame (+x east, +y south).
pub fn dir_yaw(d: usize) -> f32 {
    let (dx, dy) = DIRS[d];
    (dy as f32).atan2(dx as f32)
}

/// Shortest signed angular delta from `a` to `b`, in `(-PI, PI]`.
pub fn ang_diff(a: f32, b: f32) -> f32 {
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
    /// Head-bob: eye height above its resting 0.5, in world units.
    pub bob: f32,
    /// How far the bot has gone down the stairwell (`descend`), world units.
    sink: f32,
    descending: bool,
    /// The stairwell's arch face, from the exit cell's centre (`descend`).
    arch: f32,
    /// Looking down, as a fraction of the view height (the horizon rises by it).
    pub pitch: f32,
    /// Gait, -1..1: which leg is lifted and how far (the sign alternates every step).
    /// 0 at rest — drives the bot's reflection in Mirror mode.
    pub stride: f32,
    /// Which foot the next step leads with (±1).
    foot: f32,
    /// Set by `face`: the running `Turn` ends in `Idle` rather than a `Walk`.
    turn_only: bool,
    /// Set by `begin_step(.., Some(gate))`: the `Turn` leads into `Unlock`.
    gate: Option<Gate>,
    /// The gate being passed while in `Unlock`.
    at_gate: Gate,
    /// Set by `open`: after the door swings, stay put instead of walking through.
    stay: bool,
    /// `scan`: glances still to come (yaws unwrapped from the current one), next last.
    looks: Vec<Look>,
    /// The current glance's pitch and stoop, eased from `*_from` over the turn.
    pitch_from: f32,
    pitch_to: f32,
    stoop: f32,
    stoop_from: f32,
    stoop_to: f32,
    /// How long the current glance holds.
    hold: f32,
    /// After opening a door from the doorway: seconds to keep looking in.
    after_open: Vec<Look>,
    /// `open(.., lean)`: half a cell toward the door to look in, then back out.
    lean: Option<usize>,
    /// Leaning in: where to step back to once the glances are done.
    lean_home: Option<Vec2>,
    /// Stay leaned in after the glances, until `step_back` (the boss argument).
    lean_stay: bool,
    /// An event raised by `begin_step` itself (entering `Unlock` with no turn first),
    /// reported by the next `update`.
    pending: Option<AnimEvent>,
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
            sink: 0.0,
            descending: false,
            arch: 0.5,
            pitch: 0.0,
            stride: 0.0,
            foot: 1.0,
            turn_only: false,
            gate: None,
            at_gate: Gate::Key,
            stay: false,
            looks: Vec::new(),
            pitch_from: 0.0,
            pitch_to: 0.0,
            stoop: 0.0,
            stoop_from: 0.0,
            stoop_to: 0.0,
            hold: LOOK_HOLD,
            after_open: Vec::new(),
            lean: None,
            lean_home: None,
            lean_stay: false,
            pending: None,
        }
    }

    pub fn is_idle(&self) -> bool {
        self.phase == Phase::Idle
    }

    /// Arriving at a junction for the first time: look down each side corridor in
    /// `dirs` in turn — turn, hold long enough for a viewer to see what's there — then
    /// stay idle until the next step turns wherever it's going. (The old idle glance
    /// eased toward the next corridor in the few frames between steps: a twitch.)
    /// `looks`: each glance with how long to hold it and how far down to look. A scan
    /// that looked down anywhere ends by raising the head level again (no turn), so
    /// the next step never walks off looking at its feet.
    pub fn scan(&mut self, looks: &[Look]) {
        let mut yaw = self.yaw;
        self.looks.clear();
        for l in looks {
            yaw += ang_diff(yaw, l.yaw);
            self.looks.push(Look { yaw, ..*l });
        }
        if looks.iter().any(|l| l.pitch > 0.0 || l.stoop > 0.0) {
            self.looks.push(Look {
                yaw,
                hold: 0.0,
                pitch: 0.0,
                stoop: 0.0,
            });
        }
        self.looks.reverse();
        if let Some(l) = self.looks.pop() {
            self.yaw_from = self.yaw;
            self.pitch_from = self.pitch;
            self.stoop_from = self.stoop;
            self.yaw_to = l.yaw;
            self.pitch_to = l.pitch;
            self.stoop_to = l.stoop;
            self.hold = l.hold;
            self.t = 0.0;
            self.phase = Phase::Look;
        }
    }

    /// Turn to face `dir` and push the office door there open, without stepping
    /// through — the bot opens every door it passes to look inside.
    /// `after`: glances to take once the door has swung (a `scan` — looking round a
    /// room seen whole from the doorway, or holding on something to read); none, and
    /// the bot stands idle.
    /// `lean`: step half a cell into the doorway for the glances, and back out after.
    pub fn open(&mut self, dir: usize, after: &[Look], lean: bool) {
        self.after_open = after.to_vec();
        self.lean = lean.then_some(dir);
        self.lean_stay = false;
        self.yaw_from = self.yaw;
        self.yaw_to = self.yaw + ang_diff(self.yaw, dir_yaw(dir));
        self.t = 0.0;
        self.gate = Some(Gate::Push);
        self.stay = true;
        self.phase = if ang_diff(self.yaw, dir_yaw(dir)).abs() > 0.01 {
            Phase::Turn
        } else {
            self.after_turn()
        };
    }

    /// After `open(.., lean)`: stay leaned in once there, until `step_back`.
    pub fn stay_leaning(&mut self) {
        self.lean_stay = true;
    }

    /// Leaned in and staying: step back out to where the bot stood, facing the same
    /// way — backing out of the boss's office.
    pub fn step_back(&mut self) {
        if let Some(home) = self.lean_home.take() {
            self.lean_stay = false;
            self.pos_from = self.pos;
            self.pos_to = home;
            self.t = 0.0;
            self.phase = Phase::Lean;
        }
    }

    /// Snap to a cell/facing with no interpolation (new maze, reseed).
    pub fn reset(&mut self, at: Cell, facing: usize) {
        *self = Anim::new(at, facing);
    }

    /// Begin playing out a step: turn from the current facing to `dir`, then walk from
    /// `from` to the next cell. A pure-straight step skips the turn. `gate`: the step
    /// goes through a shut door — pause there while it opens.
    pub fn begin_step(&mut self, from: Cell, dir: usize, gate: Option<Gate>) {
        self.yaw_from = self.yaw;
        self.yaw_to = self.yaw + ang_diff(self.yaw, dir_yaw(dir));
        self.pos_from = cell_pos(from);
        self.pos_to = cell_pos(Cell {
            x: from.x + DIRS[dir].0,
            y: from.y + DIRS[dir].1,
        });
        self.t = 0.0;
        self.gate = gate;
        self.phase = if ang_diff(self.yaw, dir_yaw(dir)).abs() > 0.01 {
            Phase::Turn
        } else {
            self.after_turn()
        };
    }

    /// Eye height above its resting 0.5: the head-bob, less any way down the stairs
    /// or stoop.
    pub fn eye_lift(&self) -> f32 {
        self.bob - self.sink - self.stoop
    }

    /// Dungeon's exit reached: turn to the stairwell in wall `dir` (its arch face `arch`
    /// from the cell's centre, `Maze::wall_face`) and walk down the flight. Ends idle,
    /// standing on the stairs — no `Footfall`.
    pub fn descend(&mut self, dir: usize, arch: f32) {
        self.arch = arch;
        self.yaw_from = self.yaw;
        self.yaw_to = self.yaw + ang_diff(self.yaw, dir_yaw(dir));
        let (dx, dy) = DIRS[dir];
        self.pos_from = self.pos;
        self.pos_to = self.pos + vec2(dx as f32, dy as f32) * descend_reach(arch);
        self.looks.clear();
        self.t = 0.0;
        self.descending = true;
        self.phase = if ang_diff(self.yaw, dir_yaw(dir)).abs() > 0.01 {
            Phase::Turn
        } else {
            self.after_turn()
        };
    }

    /// What follows the turn of a step: the door, or straight into the walk.
    fn after_turn(&mut self) -> Phase {
        if std::mem::take(&mut self.descending) {
            return Phase::Descend;
        }
        match self.gate.take() {
            Some(g) => {
                self.at_gate = g;
                self.pending = Some(match g {
                    Gate::Key => AnimEvent::KeyTurn,
                    Gate::Push => AnimEvent::DoorPush,
                });
                Phase::Unlock
            }
            None => Phase::Walk,
        }
    }

    /// Turn in place to face `dir` without stepping — the bot has found a mirror where
    /// it expected a passage (Mirror mode).
    pub fn face(&mut self, dir: usize) {
        // Already facing it: nothing to do (a zero turn still shuffled a foot — a step
        // on the spot, plain to see in a mirror).
        if ang_diff(self.yaw, dir_yaw(dir)).abs() < 0.01 {
            return;
        }
        self.yaw_from = self.yaw;
        self.yaw_to = self.yaw + ang_diff(self.yaw, dir_yaw(dir));
        self.t = 0.0;
        self.phase = Phase::Turn;
        self.turn_only = true;
    }

    pub fn update(&mut self, dt: f32) -> Option<AnimEvent> {
        if let Some(e) = self.pending.take() {
            return Some(e);
        }
        match self.phase {
            Phase::Idle => {
                self.bob *= (1.0 - dt * 8.0).max(0.0);
                self.stride *= (1.0 - dt * 8.0).max(0.0);
            }
            Phase::Look => {
                self.t += dt;
                self.bob *= (1.0 - dt * 8.0).max(0.0);
                self.stride *= (1.0 - dt * 8.0).max(0.0);
                let k = (self.t / LOOK_TURN).clamp(0.0, 1.0);
                self.yaw = self.yaw_from + (self.yaw_to - self.yaw_from) * ease(k);
                self.pitch = self.pitch_from + (self.pitch_to - self.pitch_from) * ease(k);
                self.stoop = self.stoop_from + (self.stoop_to - self.stoop_from) * ease(k);
                if self.t >= LOOK_TURN + self.hold {
                    self.t = 0.0;
                    self.yaw_from = self.yaw_to;
                    self.pitch_from = self.pitch_to;
                    self.stoop_from = self.stoop_to;
                    match self.looks.pop() {
                        Some(l) => {
                            self.yaw_to = l.yaw;
                            self.pitch_to = l.pitch;
                            self.stoop_to = l.stoop;
                            self.hold = l.hold;
                        }
                        None => {
                            self.phase = Phase::Idle;
                            // Leaning in: step back out.
                            if let Some(home) = self.lean_home.take() {
                                self.pos_from = self.pos;
                                self.pos_to = home;
                                self.phase = Phase::Lean;
                            }
                        }
                    }
                }
            }
            Phase::Lean => {
                self.t += dt;
                let k = (self.t / LEAN_SECS).clamp(0.0, 1.0);
                self.pos = self.pos_from.lerp(self.pos_to, ease(k));
                self.bob = (k * std::f32::consts::PI).sin() * 0.012;
                if k >= 1.0 {
                    self.t = 0.0;
                    self.bob = 0.0;
                    self.phase = Phase::Idle;
                    if self.lean_home.is_some() {
                        // In: now the glances.
                        let after = std::mem::take(&mut self.after_open);
                        self.scan(&after);
                        if self.phase == Phase::Idle && !self.lean_stay {
                            self.pos_from = self.pos;
                            self.pos_to = self.lean_home.take().expect("leaning in");
                            self.phase = Phase::Lean;
                        }
                    }
                }
            }
            Phase::Turn => {
                self.t += dt;
                let delta = self.yaw_to - self.yaw_from;
                let k = (self.t / turn_secs(delta)).clamp(0.0, 1.0);
                self.yaw = self.yaw_from + delta * ease(k);
                // Turning in place is stepping round, not spinning on a turntable: the
                // feet shuffle (alternating, via the sign of the sine) and the head dips
                // a little with each — seen in the Mirror reflection and the bob.
                let shuffle = (k * turn_steps(delta) * PI).sin();
                self.stride = shuffle * TURN_STEP * self.foot;
                self.bob = shuffle.abs() * 0.012;
                if k >= 1.0 {
                    self.yaw = self.yaw_to;
                    self.phase = if self.turn_only {
                        self.turn_only = false;
                        Phase::Idle
                    } else {
                        self.after_turn()
                    };
                    self.t = 0.0;
                    return self.pending.take();
                }
            }
            Phase::Unlock => {
                self.t += dt;
                let (secs, done) = match self.at_gate {
                    Gate::Key => (UNLOCK_SECS, Some(AnimEvent::DoorOpen)),
                    Gate::Push => (PUSH_SECS, None),
                };
                // Opening from the doorway: hold while the door finishes its swing.
                let secs = if self.stay { SWING_SECS } else { secs };
                if self.t >= secs {
                    self.phase = if std::mem::take(&mut self.stay) {
                        // Then look in, if there's something worth a look — from half a
                        // step into the doorway, if leaning in.
                        if let Some(d) = self.lean.take() {
                            let (dx, dy) = DIRS[d];
                            self.lean_home = Some(self.pos);
                            self.pos_from = self.pos;
                            self.pos_to = self.pos + vec2(dx as f32, dy as f32) * LEAN_REACH;
                            Phase::Lean
                        } else {
                            let after = std::mem::take(&mut self.after_open);
                            self.phase = Phase::Idle;
                            self.scan(&after);
                            self.phase
                        }
                    } else {
                        Phase::Walk
                    };
                    self.t = 0.0;
                    return done;
                }
            }
            Phase::Walk => {
                self.t += dt;
                let k = (self.t / WALK_SECS).clamp(0.0, 1.0);
                // Constant pace, not eased per cell: easing made every cell a
                // slow-fast-slow surge, which read as one lunge per cell (and bunched the
                // two footfalls into a pair) however many strides the legs took.
                let ph = k * STRIDES_PER_CELL * PI;
                // Two strides: eye height rises through each and settles at its
                // footfall; the lifted leg swaps (the sine's sign) at the halfway one, and
                // the body sways a touch over the planted foot.
                self.stride = ph.sin() * self.foot;
                self.bob = ph.sin().abs() * 0.03;
                let travel = self.pos_to - self.pos_from;
                let sway = vec2(-travel.y, travel.x).normalize_or_zero() * (-0.012 * self.stride);
                self.pos = self.pos_from.lerp(self.pos_to, k) + sway;
                let mid = (self.t - dt) / WALK_SECS < 0.5 && k >= 0.5;
                if mid && k < 1.0 {
                    return Some(AnimEvent::Step);
                }
                if k >= 1.0 {
                    self.pos = self.pos_to;
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                    return Some(AnimEvent::Footfall);
                }
            }
            Phase::Descend => {
                self.t += dt;
                self.pitch = DESCEND_PITCH * ease((self.t / PEER_SECS).min(1.0));
                if self.t < PEER_SECS {
                    return None;
                }
                let k = ((self.t - PEER_SECS) / DESCEND_SECS).clamp(0.0, 1.0);
                let ph = k * DESCEND_STRIDES * PI;
                self.stride = ph.sin() * self.foot;
                self.bob = ph.sin().abs() * 0.02;
                self.pos = self.pos_from.lerp(self.pos_to, k);
                // The eye rides the treads under it, `s` past the arch.
                self.sink = stair_sink(self.pos.distance(self.pos_from) - self.arch);
                self.pitch = DESCEND_PITCH + (DESCEND_PITCH_LOW - DESCEND_PITCH) * ease(k);
                let strides = k * DESCEND_STRIDES;
                let before = ((self.t - PEER_SECS - dt) / DESCEND_SECS * DESCEND_STRIDES).floor();
                if k < 1.0 && strides.floor() > before {
                    return Some(AnimEvent::Step);
                }
                if k >= 1.0 {
                    self.pos = self.pos_to;
                    self.sink = stair_sink(descend_reach(self.arch) - self.arch);
                    self.phase = Phase::Idle;
                    self.t = 0.0;
                    return Some(AnimEvent::Step);
                }
            }
        }
        None
    }
}

/// Duration of a turn through `delta` radians: a quarter turn is `TURN_SECS`, an
/// about-face half again as long — not twice, a body swings faster through a big turn.
fn turn_secs(delta: f32) -> f32 {
    TURN_SECS * (delta.abs() / (PI / 2.0)).max(1.0).powf(0.6)
}

/// Foot shuffles for a turn through `delta` radians: one per ~45deg, at least one for
/// any real turn — none for a few degrees' correction (e.g. squaring up after a glance),
/// which otherwise read as a step taken on the spot.
fn turn_steps(delta: f32) -> f32 {
    if delta.abs() < 0.2 {
        return 0.0;
    }
    (delta.abs() / (PI / 4.0)).round().max(1.0)
}

fn ease(k: f32) -> f32 {
    // smoothstep
    k * k * (3.0 - 2.0 * k)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Down the stairwell the eye goes down the flight, not just forward: a riser per
    /// tread, never back up, ending well below the floor — under the stepped vault
    /// (`Decor::stairwell`: 0.76 less a riser per pair of treads) all the way, and
    /// looking less steeply down at the end than at the top.
    #[test]
    fn descent_goes_down_the_flight() {
        let mut a = Anim::new(Cell { x: 1, y: 1 }, 1);
        let from = a.pos;
        let arch = crate::maze::face_dist(true);
        a.descend(1, arch);
        let mut last = 0.0;
        let mut top_pitch: f32 = 0.0;
        for _ in 0..1000 {
            a.update(1.0 / 60.0);
            let s = a.pos.distance(from) - arch;
            assert!(a.sink >= last - 1e-6, "rose back up at {s}");
            last = a.sink;
            top_pitch = top_pitch.max(a.pitch);
            if s > STAIR_SILL {
                let k = (((s - STAIR_SILL) / STAIR_TREAD) as usize).min(STAIR_STEPS - 1);
                let roof = 0.76 - STAIR_RISE * (k - k % 2) as f32;
                assert!(0.5 + a.eye_lift() < roof - 0.15, "eye in the vault at {s}");
            }
            if a.is_idle() {
                break;
            }
        }
        assert!(a.is_idle());
        assert!((0.34..=0.45).contains(&a.sink), "sink {}", a.sink);
        assert!(a.pitch < top_pitch - 0.1, "{} {top_pitch}", a.pitch);
    }

    /// Peeking into a room: after the door swings the bot leans half a cell into the
    /// doorway, glances each way, and steps back to where it stood.
    #[test]
    fn peek_leans_in_and_back() {
        let home = Cell { x: 1, y: 1 };
        let mut a = Anim::new(home, 1);
        let start = a.pos;
        let side = |s: f32| Look {
            yaw: dir_yaw(1) + s * PEEK_TURN,
            hold: PEEK_HOLD,
            pitch: 0.0,
            stoop: 0.0,
        };
        a.open(1, &[side(-1.0), side(1.0)], true);
        let mut reach: f32 = 0.0;
        for _ in 0..2000 {
            a.update(1.0 / 60.0);
            reach = reach.max((a.pos - start).x);
            if a.is_idle() {
                break;
            }
        }
        assert!(a.is_idle());
        assert!((reach - LEAN_REACH).abs() < 1e-3, "reach {reach}");
        assert!(a.pos.distance(start) < 1e-4);
    }

    /// The boss: lean in and stay there (idle, so the argument can run), then back
    /// straight out on `step_back` without turning.
    #[test]
    fn boss_lean_stays_until_step_back() {
        let mut a = Anim::new(Cell { x: 1, y: 1 }, 1);
        let start = a.pos;
        a.open(1, &[], true);
        a.stay_leaning();
        for _ in 0..600 {
            a.update(1.0 / 60.0);
            if a.is_idle() {
                break;
            }
        }
        assert!(a.is_idle());
        assert!(((a.pos - start).x - LEAN_REACH).abs() < 1e-3, "{}", a.pos);
        let yaw = a.yaw;
        a.step_back();
        assert!(!a.is_idle());
        for _ in 0..600 {
            a.update(1.0 / 60.0);
            assert!((a.yaw - yaw).abs() < 1e-5, "turned while backing out");
            if a.is_idle() {
                break;
            }
        }
        assert!(a.pos.distance(start) < 1e-4);
    }

    /// A scan that looks down somewhere ends level: the head comes back up (and the
    /// stoop straightens) before the scan goes idle, so no step starts walking with its
    /// eyes on the ground.
    #[test]
    fn scan_pitch_returns_level() {
        let mut a = Anim::new(Cell { x: 1, y: 1 }, 1);
        a.scan(&[
            Look::dir(0, LOOK_HOLD),
            Look {
                yaw: dir_yaw(1),
                hold: READ_HOLD,
                pitch: 0.25,
                stoop: 0.15,
            },
        ]);
        let (mut peak, mut low): (f32, f32) = (0.0, 0.0);
        for _ in 0..1000 {
            a.update(1.0 / 60.0);
            peak = peak.max(a.pitch);
            low = low.min(a.eye_lift());
            if a.is_idle() {
                break;
            }
        }
        assert!(a.is_idle());
        assert!((peak - 0.25).abs() < 1e-3, "peak {peak}");
        assert_eq!(a.pitch, 0.0);
        assert!((low + 0.15).abs() < 1e-3, "low {low}");
        assert_eq!(a.eye_lift(), 0.0);
        assert!(ang_diff(a.yaw, dir_yaw(1)).abs() < 1e-4);
    }
}
