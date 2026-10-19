//! Rules and state. Pure fixed-timestep simulation: `step()` advances exactly one
//! `SIM_DT` tick and never reads wall-clock or frame time, so a seed replays exactly.
//! All randomness comes from the game's own `Rng` (seeded in `new`), never macroquad's
//! global one.

use std::f32::consts::{PI, TAU};

pub const W: f32 = 900.0;
pub const H: f32 = 720.0;
pub const SIM_DT: f32 = 1.0 / 60.0;

pub const SHIP_R: f32 = 10.0;
pub const TURN_RATE: f32 = 4.5 * PI / 180.0;
pub const THRUST: f32 = 0.11;
pub const FRICTION: f32 = 0.992;
pub const MAX_SPEED: f32 = 7.0;

pub const BULLET_SPEED: f32 = 8.0;
pub const BULLET_TTL: u32 = 60;
pub const MAX_BULLETS: usize = 4;
pub const FIRE_COOLDOWN: u32 = 10;

pub const START_LIVES: u32 = 3;
pub const INVULN_TICKS: u32 = 120;
pub const RESPAWN_DELAY: u32 = 90;
/// Respawn waits until no rock is within this of the centre (classic rule).
pub const RESPAWN_CLEAR: f32 = 130.0;
pub const WAVE_DELAY: u32 = 120;

pub const HYPER_TICKS: u32 = 40;
pub const HYPER_COOLDOWN: u32 = 60;
/// Chance (out of 256) that re-entry from hyperspace kills the ship outright.
pub const HYPER_DEATH_CHANCE: u32 = 16;

pub const SAUCER_BULLET_SPEED: f32 = 6.5;
pub const SAUCER_BULLET_TTL: u32 = 90;
pub const SAUCER_FIRE_EVERY: u32 = 36;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
    pub fn from_angle(a: f32) -> Self {
        Self::new(a.cos(), a.sin())
    }
    pub fn dot(self, o: Self) -> f32 {
        self.x * o.x + self.y * o.y
    }
    pub fn len(self) -> f32 {
        self.dot(self).sqrt()
    }
    pub fn angle(self) -> f32 {
        self.y.atan2(self.x)
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, o: Self) -> Self {
        Self::new(self.x + o.x, self.y + o.y)
    }
}
impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, o: Self) -> Self {
        Self::new(self.x - o.x, self.y - o.y)
    }
}
impl std::ops::Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, k: f32) -> Self {
        Self::new(self.x * k, self.y * k)
    }
}

fn wrap1(v: f32, size: f32) -> f32 {
    // floor, not rem_euclid: the latter is a slow soft-float fmod and this is hot.
    v - size * (v / size).floor()
}

pub fn wrap(p: Vec2) -> Vec2 {
    Vec2::new(wrap1(p.x, W), wrap1(p.y, H))
}

/// Shortest signed offset on a ring of `size`; inputs are wrapped coordinates, so
/// `d` is already in (-size, size) and no `rem_euclid` (a slow soft-float fmod) is needed.
fn delta1(d: f32, size: f32) -> f32 {
    if d > size * 0.5 {
        d - size
    } else if d < -size * 0.5 {
        d + size
    } else {
        d
    }
}

/// `to - from` on the torus: the offset to the nearest of `to`'s wrapped images.
pub fn wrap_delta(from: Vec2, to: Vec2) -> Vec2 {
    Vec2::new(delta1(to.x - from.x, W), delta1(to.y - from.y, H))
}

/// Signed shortest angle from `a` to `b`, in (-PI, PI].
pub fn angle_diff(a: f32, b: f32) -> f32 {
    let d = b - a;
    d - TAU * (d / TAU).round()
}

/// xorshift64* — the sim's only randomness source.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        // splitmix the seed so small seeds (1, 2, 3...) still start well-mixed.
        let mut z = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        Self((z ^ (z >> 31)) | 1)
    }
    pub fn next_u32(&mut self) -> u32 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        (self.0.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    }
    /// Uniform in [lo, hi).
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * (self.next_u32() as f32 / 4_294_967_296.0)
    }
    pub fn below(&mut self, n: u32) -> u32 {
        self.next_u32() % n
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Controls {
    pub turn: i8,
    pub thrust: bool,
    pub fire: bool,
    pub hyperspace: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RockSize {
    Large,
    Medium,
    Small,
}

impl RockSize {
    pub fn radius(self) -> f32 {
        match self {
            RockSize::Large => 40.0,
            RockSize::Medium => 20.0,
            RockSize::Small => 10.0,
        }
    }
    pub fn score(self) -> u64 {
        match self {
            RockSize::Large => 20,
            RockSize::Medium => 50,
            RockSize::Small => 100,
        }
    }
    fn child(self) -> Option<RockSize> {
        match self {
            RockSize::Large => Some(RockSize::Medium),
            RockSize::Medium => Some(RockSize::Small),
            RockSize::Small => None,
        }
    }
    /// Speed range in px/tick.
    fn speed(self) -> (f32, f32) {
        match self {
            RockSize::Large => (0.5, 1.2),
            RockSize::Medium => (0.9, 1.9),
            RockSize::Small => (1.3, 2.7),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Rock {
    pub pos: Vec2,
    pub vel: Vec2,
    pub size: RockSize,
    /// Outline variant — cosmetic only.
    pub shape: u8,
    pub angle: f32,
    pub spin: f32,
}

#[derive(Clone, Debug)]
pub struct Ship {
    pub pos: Vec2,
    pub vel: Vec2,
    pub angle: f32,
    pub invuln_ticks: u32,
    pub alive: bool,
    /// Ticks left inside hyperspace (ship is gone but not dead).
    pub hyper_ticks: u32,
    pub hyper_cooldown: u32,
    pub fire_cooldown: u32,
    /// Whether thrust was applied on the last step (drives the flame + sound).
    pub thrusting: bool,
}

impl Ship {
    fn spawn() -> Self {
        Self {
            pos: Vec2::new(W * 0.5, H * 0.5),
            vel: Vec2::default(),
            angle: -PI * 0.5,
            invuln_ticks: INVULN_TICKS,
            alive: true,
            hyper_ticks: 0,
            hyper_cooldown: 0,
            fire_cooldown: 0,
            thrusting: false,
        }
    }
    /// On the field and collidable-or-blinking (not dead, not in hyperspace).
    pub fn present(&self) -> bool {
        self.alive && self.hyper_ticks == 0
    }
    pub fn dir(&self) -> Vec2 {
        Vec2::from_angle(self.angle)
    }
}

#[derive(Clone, Debug)]
pub struct Bullet {
    pub pos: Vec2,
    pub vel: Vec2,
    pub ttl: u32,
    pub from_ship: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SaucerSize {
    Large,
    Small,
}

impl SaucerSize {
    pub fn radius(self) -> f32 {
        match self {
            SaucerSize::Large => 16.0,
            SaucerSize::Small => 9.0,
        }
    }
    pub fn score(self) -> u64 {
        match self {
            SaucerSize::Large => 200,
            SaucerSize::Small => 1000,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Saucer {
    pub pos: Vec2,
    pub vel: Vec2,
    pub size: SaucerSize,
    pub fire_timer: u32,
    pub turn_timer: u32,
    /// Horizontal distance left before it exits (it crosses once, wrapping only vertically).
    pub travel_left: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cause {
    Rock(RockSize),
    Saucer,
    SaucerBullet,
    Hyperspace,
}

#[derive(Clone, Copy, Debug)]
pub enum Event {
    Fire,
    RockDestroyed { size: RockSize, pos: Vec2 },
    SaucerDestroyed { size: SaucerSize, pos: Vec2 },
    SaucerFire,
    ShipDestroyed { cause: Cause, pos: Vec2 },
    HyperspaceOut { pos: Vec2 },
    HyperspaceIn { pos: Vec2 },
    WaveStart,
    Respawn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Running,
    GameOver,
}

#[derive(Clone)]
pub struct Game {
    pub ship: Ship,
    pub rocks: Vec<Rock>,
    pub bullets: Vec<Bullet>,
    pub saucer: Option<Saucer>,
    pub wave: u32,
    pub lives: u32,
    pub score: u64,
    pub tick: u64,
    pub deaths: u32,
    pub hyperspaces: u32,
    /// Ticks until the dead ship may respawn (then it still waits for a clear centre).
    pub respawn_timer: u32,
    /// Ticks until the next wave spawns once the field is clear.
    pub wave_timer: u32,
    pub saucer_timer: u32,
    /// Events produced by the last `step()` (cleared at the start of each step).
    pub events: Vec<Event>,
    rng: Rng,
}

impl Game {
    pub fn new(seed: u64) -> Self {
        let mut g = Self {
            ship: Ship::spawn(),
            rocks: Vec::new(),
            bullets: Vec::new(),
            saucer: None,
            wave: 0,
            lives: START_LIVES,
            score: 0,
            tick: 0,
            deaths: 0,
            hyperspaces: 0,
            respawn_timer: 0,
            wave_timer: 0,
            saucer_timer: 0,
            events: Vec::new(),
            rng: Rng::new(seed),
        };
        g.start_wave();
        g
    }

    pub fn outcome(&self) -> Outcome {
        if self.lives == 0 && !self.ship.alive {
            Outcome::GameOver
        } else {
            Outcome::Running
        }
    }

    fn start_wave(&mut self) {
        self.wave += 1;
        let n = (4 + 2 * (self.wave - 1)).min(11);
        for _ in 0..n {
            // Spawn along an edge, well away from the ship.
            let pos = loop {
                let p = if self.rng.below(2) == 0 {
                    Vec2::new(self.rng.range(0.0, W), 0.0)
                } else {
                    Vec2::new(0.0, self.rng.range(0.0, H))
                };
                if wrap_delta(self.ship.pos, p).len() > 220.0 {
                    break p;
                }
            };
            let rock = self.make_rock(pos, Vec2::default(), RockSize::Large);
            self.rocks.push(rock);
        }
        self.saucer_timer = self.saucer_interval();
        self.events.push(Event::WaveStart);
    }

    fn saucer_interval(&mut self) -> u32 {
        // Comes sooner in later waves.
        let base = 900u32.saturating_sub(self.wave * 50).max(300);
        base + self.rng.below(300)
    }

    fn make_rock(&mut self, pos: Vec2, base_vel: Vec2, size: RockSize) -> Rock {
        let (lo, hi) = size.speed();
        // Classic ramp: rocks keep getting faster, flattening out around wave 20.
        let boost = 1.0 + 0.08 * (self.wave.min(21) - 1) as f32;
        let speed = self.rng.range(lo, hi) * boost;
        let dir = self.rng.range(0.0, TAU);
        Rock {
            pos,
            vel: base_vel * 0.4 + Vec2::from_angle(dir) * speed,
            size,
            shape: self.rng.below(4) as u8,
            angle: self.rng.range(0.0, TAU),
            spin: self.rng.range(-0.03, 0.03),
        }
    }

    fn kill_ship(&mut self, cause: Cause) {
        self.events.push(Event::ShipDestroyed {
            cause,
            pos: self.ship.pos,
        });
        self.ship.alive = false;
        self.ship.thrusting = false;
        self.deaths += 1;
        self.lives = self.lives.saturating_sub(1);
        self.respawn_timer = RESPAWN_DELAY;
    }

    fn split_rock(&mut self, i: usize) {
        let rock = self.rocks.swap_remove(i);
        self.events.push(Event::RockDestroyed {
            size: rock.size,
            pos: rock.pos,
        });
        if let Some(child) = rock.size.child() {
            for _ in 0..2 {
                let r = self.make_rock(rock.pos, rock.vel, child);
                self.rocks.push(r);
            }
        }
    }

    pub fn step(&mut self, c: Controls) {
        self.events.clear();
        self.tick += 1;
        if self.outcome() == Outcome::GameOver {
            return;
        }
        self.step_ship(c);
        self.step_saucer();
        for r in &mut self.rocks {
            r.pos = wrap(r.pos + r.vel);
            r.angle += r.spin;
        }
        for b in &mut self.bullets {
            b.pos = wrap(b.pos + b.vel);
            b.ttl -= 1;
        }
        self.bullets.retain(|b| b.ttl > 0);
        self.collide();
        self.step_waves();
    }

    fn step_ship(&mut self, c: Controls) {
        let ship = &mut self.ship;
        if !ship.alive {
            if self.lives == 0 {
                return;
            }
            if self.respawn_timer > 0 {
                self.respawn_timer -= 1;
                return;
            }
            let centre = Vec2::new(W * 0.5, H * 0.5);
            let clear = self
                .rocks
                .iter()
                .all(|r| wrap_delta(centre, r.pos).len() > RESPAWN_CLEAR + r.size.radius())
                && self
                    .saucer
                    .as_ref()
                    .is_none_or(|s| wrap_delta(centre, s.pos).len() > RESPAWN_CLEAR);
            if clear {
                self.ship = Ship::spawn();
                self.events.push(Event::Respawn);
            }
            return;
        }
        ship.fire_cooldown = ship.fire_cooldown.saturating_sub(1);
        ship.hyper_cooldown = ship.hyper_cooldown.saturating_sub(1);
        ship.invuln_ticks = ship.invuln_ticks.saturating_sub(1);
        if ship.hyper_ticks > 0 {
            ship.hyper_ticks -= 1;
            if ship.hyper_ticks == 0 {
                let pos = ship.pos;
                self.events.push(Event::HyperspaceIn { pos });
                if self.rng.below(256) < HYPER_DEATH_CHANCE {
                    self.kill_ship(Cause::Hyperspace);
                }
            }
            return;
        }
        if c.hyperspace && ship.hyper_cooldown == 0 {
            self.events.push(Event::HyperspaceOut { pos: ship.pos });
            self.hyperspaces += 1;
            let ship = &mut self.ship;
            ship.hyper_ticks = HYPER_TICKS;
            ship.hyper_cooldown = HYPER_TICKS + HYPER_COOLDOWN;
            ship.vel = Vec2::default();
            ship.thrusting = false;
            ship.pos = Vec2::new(
                self.rng.range(40.0, W - 40.0),
                self.rng.range(40.0, H - 40.0),
            );
            return;
        }
        ship.angle = (ship.angle + c.turn.signum() as f32 * TURN_RATE).rem_euclid(TAU);
        ship.thrusting = c.thrust;
        if c.thrust {
            ship.vel = ship.vel + ship.dir() * THRUST;
            let s = ship.vel.len();
            if s > MAX_SPEED {
                ship.vel = ship.vel * (MAX_SPEED / s);
            }
        }
        ship.vel = ship.vel * FRICTION;
        ship.pos = wrap(ship.pos + ship.vel);
        let live = self.bullets.iter().filter(|b| b.from_ship).count();
        if c.fire && ship.fire_cooldown == 0 && live < MAX_BULLETS {
            ship.fire_cooldown = FIRE_COOLDOWN;
            let d = ship.dir();
            self.bullets.push(Bullet {
                pos: ship.pos + d * SHIP_R,
                vel: ship.vel + d * BULLET_SPEED,
                ttl: BULLET_TTL,
                from_ship: true,
            });
            self.events.push(Event::Fire);
        }
    }

    fn step_saucer(&mut self) {
        let Some(s) = self.saucer.as_mut() else {
            if self.saucer_timer > 0 && !self.rocks.is_empty() {
                self.saucer_timer -= 1;
                if self.saucer_timer == 0 {
                    self.spawn_saucer();
                }
            }
            return;
        };
        s.pos = Vec2::new(s.pos.x + s.vel.x, wrap1(s.pos.y + s.vel.y, H));
        s.travel_left -= s.vel.x.abs();
        if s.travel_left <= 0.0 {
            self.saucer = None;
            self.saucer_timer = self.saucer_interval();
            return;
        }
        s.turn_timer = s.turn_timer.saturating_sub(1);
        if s.turn_timer == 0 {
            let vy = [-1.0, 0.0, 1.0][self.rng.below(3) as usize] * s.vel.x.abs();
            let s = self.saucer.as_mut().unwrap();
            s.vel.y = vy;
            s.turn_timer = 60 + self.rng.below(60);
        }
        let s = self.saucer.as_mut().unwrap();
        s.fire_timer = s.fire_timer.saturating_sub(1);
        if s.fire_timer == 0 {
            s.fire_timer = SAUCER_FIRE_EVERY;
            let (size, pos) = (s.size, s.pos);
            let angle = if size == SaucerSize::Small && self.ship.present() {
                // Aimed, with an error cone that tightens every wave.
                let err = (0.3 - 0.02 * self.wave as f32).max(0.03);
                wrap_delta(pos, self.ship.pos).angle() + self.rng.range(-err, err)
            } else {
                self.rng.range(0.0, TAU)
            };
            self.bullets.push(Bullet {
                pos,
                vel: Vec2::from_angle(angle) * SAUCER_BULLET_SPEED,
                ttl: SAUCER_BULLET_TTL,
                from_ship: false,
            });
            self.events.push(Event::SaucerFire);
        }
    }

    fn spawn_saucer(&mut self) {
        // Small saucers appear from wave 3 and take over entirely past 30,000 points.
        let small_chance = if self.wave < 3 {
            0
        } else if self.score > 30_000 {
            256
        } else {
            64 + (self.score as u32 / 200).min(150)
        };
        let size = if self.rng.below(256) < small_chance {
            SaucerSize::Small
        } else {
            SaucerSize::Large
        };
        let speed = match size {
            SaucerSize::Large => 1.6,
            SaucerSize::Small => 2.4,
        };
        let left = self.rng.below(2) == 0;
        let r = size.radius();
        self.saucer = Some(Saucer {
            pos: Vec2::new(
                if left { -r } else { W + r },
                self.rng.range(60.0, H - 60.0),
            ),
            vel: Vec2::new(if left { speed } else { -speed }, 0.0),
            size,
            fire_timer: SAUCER_FIRE_EVERY,
            turn_timer: 60,
            travel_left: W + 2.0 * r,
        });
    }

    fn destroy_saucer(&mut self) {
        let s = self.saucer.take().unwrap();
        self.events.push(Event::SaucerDestroyed {
            size: s.size,
            pos: s.pos,
        });
        self.saucer_timer = self.saucer_interval();
    }

    fn collide(&mut self) {
        // Bullets vs rocks / saucer.
        let mut bi = 0;
        while bi < self.bullets.len() {
            let b = self.bullets[bi].clone();
            let hit_rock = self
                .rocks
                .iter()
                .position(|r| wrap_delta(b.pos, r.pos).len() < r.size.radius());
            if let Some(ri) = hit_rock {
                self.bullets.swap_remove(bi);
                let size = self.rocks[ri].size;
                self.split_rock(ri);
                if b.from_ship {
                    self.score += size.score();
                }
                continue;
            }
            if b.from_ship
                && let Some(s) = &self.saucer
                && wrap_delta(b.pos, s.pos).len() < s.size.radius()
            {
                let pts = s.size.score();
                self.bullets.swap_remove(bi);
                self.destroy_saucer();
                self.score += pts;
                continue;
            }
            bi += 1;
        }

        // Saucer vs rocks.
        if let Some(s) = &self.saucer {
            let (sp, sr) = (s.pos, s.size.radius());
            if let Some(ri) = self
                .rocks
                .iter()
                .position(|r| wrap_delta(sp, r.pos).len() < sr + r.size.radius())
            {
                self.split_rock(ri);
                self.destroy_saucer();
            }
        }

        // Ship vs everything.
        if !self.ship.present() {
            return;
        }
        let sp = self.ship.pos;
        if let Some(ri) = self
            .rocks
            .iter()
            .position(|r| wrap_delta(sp, r.pos).len() < SHIP_R + r.size.radius())
        {
            if self.ship.invuln_ticks > 0 {
                return;
            }
            let size = self.rocks[ri].size;
            self.split_rock(ri);
            self.score += size.score();
            self.kill_ship(Cause::Rock(size));
            return;
        }
        if self.ship.invuln_ticks > 0 {
            return;
        }
        if let Some(bi) = self
            .bullets
            .iter()
            .position(|b| !b.from_ship && wrap_delta(sp, b.pos).len() < SHIP_R)
        {
            self.bullets.swap_remove(bi);
            self.kill_ship(Cause::SaucerBullet);
            return;
        }
        if let Some(s) = &self.saucer
            && wrap_delta(sp, s.pos).len() < SHIP_R + s.size.radius()
        {
            let pts = s.size.score();
            self.destroy_saucer();
            self.score += pts;
            self.kill_ship(Cause::Saucer);
        }
    }

    fn step_waves(&mut self) {
        if !self.rocks.is_empty() || self.saucer.is_some() {
            return;
        }
        if self.wave_timer == 0 {
            self.wave_timer = WAVE_DELAY;
        }
        self.wave_timer -= 1;
        if self.wave_timer == 0 {
            self.start_wave();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_delta_takes_short_way() {
        let d = wrap_delta(Vec2::new(10.0, 10.0), Vec2::new(W - 10.0, H - 10.0));
        assert!((d.x + 20.0).abs() < 1e-3 && (d.y + 20.0).abs() < 1e-3);
    }

    #[test]
    fn same_seed_same_game() {
        let run = |seed| {
            let mut g = Game::new(seed);
            for i in 0..3000u32 {
                g.step(Controls {
                    turn: 1,
                    thrust: i % 7 == 0,
                    fire: true,
                    hyperspace: false,
                });
            }
            (g.score, g.tick, g.deaths, g.rocks.len())
        };
        assert_eq!(run(7), run(7));
    }
}
