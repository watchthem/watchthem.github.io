//! Move selection: `choose(&Game) -> Controls`, the bot's only input path. Per tick,
//! deterministic, no RNG:
//!
//! 1. **Threat model** — straight-line time-to-contact against every rock, the saucer
//!    and saucer bullets (wrap-aware, same circles the sim collides with).
//! 2. **Shoot or evade** the soonest threat: shoot it if turning + bullet flight beats the
//!    contact time; otherwise `plan_escape` rolls out 18 flyable options (turn-then-thrust
//!    per heading, or coast) against everything nearby and takes the roomiest. Hyperspace
//!    only when none stays clear for `ESCAPE_SOON` ticks.
//! 3. **Attack** — otherwise turn to the cheapest lead-angle intercept among targets not
//!    already covered by a bullet in flight and safe to hit (`safe_to_hit`).
//! 4. **Brake** when drifting too fast with nothing urgent.
//!
//! Fire is independent of all of that: whenever a bullet fired *now* would hit something
//! not already covered and safe to hit, fire.

use crate::game::{
    BULLET_SPEED, BULLET_TTL, Controls, FIRE_COOLDOWN, FRICTION, Game, H, MAX_BULLETS, MAX_SPEED,
    RockSize, SHIP_R, THRUST, TURN_RATE, Vec2, W, angle_diff, wrap_delta,
};

/// How far ahead (ticks) threats are considered.
const HORIZON: f32 = 120.0;
/// Clearance added to every contact radius.
const MARGIN: f32 = 10.0;
/// A threat closer than this (ticks) is dodged rather than ignored.
const REACT: f32 = 50.0;
/// Above this speed (px/tick) the bot brakes when nothing is urgent.
const CRUISE: f32 = 2.0;

#[derive(Clone, Copy)]
struct Body {
    pos: Vec2,
    vel: Vec2,
    r: f32,
    /// Shootable (rocks, saucer) vs only avoidable (saucer bullets).
    target: bool,
    /// A rock that breaks into fragments when hit (medium/large).
    splits: bool,
}

fn bodies(g: &Game) -> Vec<Body> {
    let mut v: Vec<Body> = g
        .rocks
        .iter()
        .map(|r| Body {
            pos: r.pos,
            vel: r.vel,
            r: r.size.radius(),
            target: true,
            splits: r.size != RockSize::Small,
        })
        .collect();
    if let Some(s) = &g.saucer {
        v.push(Body {
            pos: s.pos,
            vel: s.vel,
            r: s.size.radius(),
            target: true,
            splits: false,
        });
    }
    v.extend(g.bullets.iter().filter(|b| !b.from_ship).map(|b| Body {
        pos: b.pos,
        vel: b.vel,
        r: 2.0,
        target: false,
        splits: false,
    }));
    v
}

/// Earliest `t >= 0` with `|d + u t| <= r`, if within `max_t`.
fn contact_time(d: Vec2, u: Vec2, r: f32, max_t: f32) -> Option<f32> {
    let c = d.dot(d) - r * r;
    if c <= 0.0 {
        return Some(0.0);
    }
    let a = u.dot(u);
    let b = 2.0 * d.dot(u);
    if a < 1e-6 || b >= 0.0 {
        return None;
    }
    let disc = b * b - 4.0 * a * c;
    if disc < 0.0 {
        return None;
    }
    let t = (-b - disc.sqrt()) / (2.0 * a);
    (t <= max_t).then_some(t)
}

/// Lead-angle intercept for a bullet fired from the ship: `(aim angle, flight ticks)`.
fn intercept(g: &Game, b: &Body) -> Option<(f32, f32)> {
    let d = wrap_delta(g.ship.pos, b.pos);
    // Bullets inherit ship velocity, so solve in the ship's frame.
    let u = b.vel - g.ship.vel;
    let a = u.dot(u) - BULLET_SPEED * BULLET_SPEED;
    let bq = 2.0 * d.dot(u);
    let c = d.dot(d);
    let t = if a.abs() < 1e-6 {
        -c / bq
    } else {
        let disc = bq * bq - 4.0 * a * c;
        if disc < 0.0 {
            return None;
        }
        let s = disc.sqrt();
        let (t1, t2) = ((-bq - s) / (2.0 * a), (-bq + s) / (2.0 * a));
        let (lo, hi) = (t1.min(t2), t1.max(t2));
        if lo > 0.0 { lo } else { hi }
    };
    if !(t > 0.0 && t < BULLET_TTL as f32 - 2.0) {
        return None;
    }
    Some(((d + u * t).angle(), t))
}

/// Would a bullet with this start/velocity/ttl pass within `b`'s radius (any wrapped image)?
fn bullet_hits(pos: Vec2, vel: Vec2, ttl: f32, b: &Body, shrink: f32) -> bool {
    let u = b.vel - vel;
    let reach = u.len() * ttl + b.r;
    let d0 = wrap_delta(pos, b.pos);
    for kx in [-W, 0.0, W] {
        for ky in [-H, 0.0, H] {
            let d = d0 + Vec2::new(kx, ky);
            // Cheap reject before the quadratic: out of range for this image.
            if d.x.abs() > reach || d.y.abs() > reach {
                continue;
            }
            if contact_time(d, u, b.r * shrink, ttl).is_some() {
                return true;
            }
        }
    }
    false
}

/// Targets some ship bullet already in flight will hit.
fn covered(g: &Game, targets: &[Body]) -> Vec<bool> {
    targets
        .iter()
        .map(|t| {
            g.bullets
                .iter()
                .filter(|b| b.from_ship)
                .any(|b| bullet_hits(b.pos, b.vel, b.ttl as f32, t, 0.9))
        })
        .collect()
}

/// Evasion candidates: 16 compass headings plus the current facing.
const ESCAPE_HEADINGS: usize = 16;
/// How far (ticks) each escape is simulated.
const ESCAPE_TICKS: usize = 60;
/// Thrust only while facing within this of the wanted heading (radians).
const THRUST_CONE: f32 = 0.5;
/// Ticks of an escape that must stay clear; hyperspace only when no option manages it.
const ESCAPE_SOON: usize = 25;
/// Leniency (px) for conflicts after `ESCAPE_SOON`.
const LATE_SLACK: f32 = 15.0;
/// Bodies farther than this can't matter within `ESCAPE_TICKS`.
const ESCAPE_RANGE: f32 = 420.0;

/// Simulates turning toward each candidate heading (at the real turn rate) and thrusting
/// once inside `THRUST_CONE`, plus coasting (`None`), against every nearby body moving in
/// a straight line. Returns the option with the best clearance (edge to edge, px) and its
/// clearance over the first `ESCAPE_SOON` ticks. Exactly the controls `choose` will then emit, so the
/// plan is physically flyable.
fn plan_escape(g: &Game, all: &[Body]) -> (Option<f32>, f32) {
    let ship = &g.ship;
    let near: Vec<(Vec2, Vec2, f32)> = all
        .iter()
        .filter_map(|b| {
            let d = wrap_delta(ship.pos, b.pos);
            (d.len() < ESCAPE_RANGE).then_some((d, b.vel, b.r + SHIP_R))
        })
        .collect();
    let candidates = (0..ESCAPE_HEADINGS)
        .map(|k| Some(k as f32 * std::f32::consts::TAU / ESCAPE_HEADINGS as f32))
        .chain([Some(ship.angle), None]);
    let mut best = (None, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for h in candidates {
        let (mut p, mut v, mut a) = (Vec2::default(), ship.vel, ship.angle);
        let (mut soon, mut late) = (f32::INFINITY, f32::INFINITY);
        for t in 1..=ESCAPE_TICKS {
            if let Some(h) = h {
                let e = angle_diff(a, h);
                a += e.clamp(-TURN_RATE, TURN_RATE);
                if e.abs() < THRUST_CONE {
                    v = v + Vec2::from_angle(a) * THRUST;
                    let s = v.len();
                    if s > MAX_SPEED {
                        v = v * (MAX_SPEED / s);
                    }
                }
            }
            v = v * FRICTION;
            p = p + v;
            let tf = t as f32;
            let worst = near
                .iter()
                .map(|&(d, bv, r)| (d + bv * tf - p).len() - r)
                .fold(f32::INFINITY, f32::min);
            if t <= ESCAPE_SOON {
                soon = soon.min(worst);
            } else {
                late = late.min(worst);
            }
        }
        // A conflict past `ESCAPE_SOON` can still be re-planned around, so it only
        // breaks ties between options that are clear for now.
        let score = soon.min(late + LATE_SLACK);
        if score > best.1 {
            best = (h, score, soon);
        }
    }
    (best.0, best.2)
}

/// Don't break a medium/large rock closer than this unless it is already coming at the
/// ship: its fragments spray outward faster than the ship can turn or thrust away.
const SPLIT_SAFE: f32 = 150.0;

/// Whether hitting `b` now is safe: it doesn't split, is far enough away, or is already on
/// a collision course (where splitting it can only help).
fn safe_to_hit(g: &Game, b: &Body) -> bool {
    let d = wrap_delta(g.ship.pos, b.pos);
    !b.splits
        || d.len() - b.r > SPLIT_SAFE
        || contact_time(d, b.vel - g.ship.vel, b.r + SHIP_R + MARGIN, HORIZON).is_some()
}

/// Turn toward `aim`, returning the turn input.
fn steer(angle: f32, aim: f32) -> i8 {
    let e = angle_diff(angle, aim);
    if e.abs() < TURN_RATE * 0.5 {
        0
    } else if e > 0.0 {
        1
    } else {
        -1
    }
}

pub fn choose(g: &Game) -> Controls {
    let ship = &g.ship;
    if !ship.present() {
        return Controls::default();
    }
    let all = bodies(g);
    let targets: Vec<Body> = all.iter().copied().filter(|b| b.target).collect();
    let cov = covered(g, &targets);

    // Fire whenever a bullet launched now would hit something not already covered.
    let dir = ship.dir();
    let live = g.bullets.iter().filter(|b| b.from_ship).count();
    let can_fire = ship.fire_cooldown == 0 && live < MAX_BULLETS;
    let fire = can_fire
        && targets.iter().zip(&cov).any(|(t, &c)| {
            !c && safe_to_hit(g, t)
                && bullet_hits(
                    ship.pos + dir * SHIP_R,
                    ship.vel + dir * BULLET_SPEED,
                    BULLET_TTL as f32,
                    t,
                    0.85,
                )
        });
    let mut out = Controls {
        fire,
        ..Controls::default()
    };

    // 1. Soonest threat.
    let threat = all
        .iter()
        .filter_map(|b| {
            let d = wrap_delta(ship.pos, b.pos);
            let u = b.vel - ship.vel;
            let t = contact_time(d, u, b.r + SHIP_R + MARGIN, HORIZON)?;
            (t >= ship.invuln_ticks as f32).then_some((t, *b))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));

    if let Some((th, b)) = threat {
        // 2a. Shoot it, if aiming + flight beats contact.
        if b.target
            && let Some((aim, tb)) = intercept(g, &b)
        {
            let turn_t = angle_diff(ship.angle, aim).abs() / TURN_RATE;
            let wait = if live < MAX_BULLETS {
                ship.fire_cooldown as f32
            } else {
                FIRE_COOLDOWN as f32
            };
            if turn_t.max(wait) + tb + 4.0 < th {
                out.turn = steer(ship.angle, aim);
                return out;
            }
        }
        // 2b. Evade: fly the escape that keeps the most clearance from everything.
        if th < REACT {
            let (heading, clearance) = plan_escape(g, &all);
            if clearance < 0.0 && th < 15.0 && ship.hyper_cooldown == 0 && !fire {
                out.hyperspace = true;
                return out;
            }
            if let Some(h) = heading {
                out.turn = steer(ship.angle, h);
                out.thrust = angle_diff(ship.angle, h).abs() < THRUST_CONE;
            }
            return out;
        }
    }

    // 4. Brake if drifting fast.
    let speed = ship.vel.len();
    if speed > CRUISE {
        let back = (ship.vel * -1.0).angle();
        out.turn = steer(ship.angle, back);
        out.thrust = angle_diff(ship.angle, back).abs() < 0.4;
        return out;
    }

    // 3. Attack the cheapest uncovered intercept.
    let best = targets
        .iter()
        .zip(&cov)
        .filter(|&(t, &c)| !c && safe_to_hit(g, t))
        .filter_map(|(t, _)| {
            let (aim, tb) = intercept(g, t)?;
            let turn_t = angle_diff(ship.angle, aim).abs() / TURN_RATE;
            let mut cost = turn_t + tb * 0.5;
            if g.saucer.as_ref().is_some_and(|s| s.pos == t.pos) {
                cost -= 30.0;
            }
            Some((cost, aim))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    if let Some((_, aim)) = best {
        out.turn = steer(ship.angle, aim);
    } else if let Some(t) = targets.first() {
        // Nothing in range: face the nearest target while it drifts in.
        out.turn = steer(ship.angle, wrap_delta(ship.pos, t.pos).angle());
    }
    out
}
