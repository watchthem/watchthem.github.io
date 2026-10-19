use macroquad::prelude::*;
use render_cache::RenderCache;

mod game;
mod solver;
mod sound;

use game::{Cause, Event, Game, H, Outcome, RockSize, SaucerSize, Vec2 as GVec, W};

/// Max sim steps per rendered frame — at 10x on a slow frame this drops time rather than
/// spiralling into ever-longer frames.
const MAX_STEPS_PER_FRAME: u32 = 24;
/// Ticks the final frame of an episode stays on screen before the next one starts.
const GAME_OVER_HOLD: u32 = 180;
const HUD_H: f32 = 44.0;
const HUD_FONT: u16 = 26;

const LINE: Color = Color::new(0.92, 0.95, 1.0, 1.0);
const GLOW: Color = Color::new(0.55, 0.8, 1.0, 0.18);
const DIM: Color = Color::new(0.6, 0.66, 0.78, 1.0);
const FLAME: Color = Color::new(1.0, 0.75, 0.4, 1.0);
const FLAME_CORE: Color = Color::new(1.0, 0.95, 0.75, 1.0);
/// Solid body behind outlines, so ship and rocks read as objects rather than wireframes.
const HULL_FILL: Color = Color::new(0.07, 0.1, 0.17, 1.0);
const ROCK_FILL: Color = Color::new(0.05, 0.055, 0.075, 1.0);
const CRATER: Color = Color::new(0.6, 0.66, 0.78, 0.55);
const CRATER_INNER: Color = Color::new(0.6, 0.66, 0.78, 0.25);
const COCKPIT: Color = Color::new(0.45, 0.85, 1.0, 1.0);

pub fn conf() -> Conf {
    Conf {
        window_title: "Asteroids".to_owned(),
        window_width: W as i32,
        window_height: H as i32,
        high_dpi: true,
        ..Default::default()
    }
}

// ── CLI args ────────────────────────────────────────────────────────────────────

pub struct CliArgs {
    /// `--debug`: log deaths (cause, distance, speed), hyperspace jumps and waves.
    pub debug: bool,
    /// `--once`: play one episode to game over, print a result line, exit.
    pub once: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pub no_ui: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cli_args() -> CliArgs {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (base, rest) = game_common::parse_base_args(&args);
    if let Some(other) = rest.first() {
        eprintln!("unknown argument '{other}' (expected --debug, --once, --no-ui)");
        std::process::exit(2);
    }
    CliArgs {
        debug: base.debug,
        once: base.once,
        no_ui: base.no_ui,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn parse_cli_args() -> CliArgs {
    bundled_cli()
}

fn bundled_cli() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        #[cfg(not(target_arch = "wasm32"))]
        no_ui: false,
    }
}

fn log_events(debug: bool, game: &Game) {
    if !debug {
        return;
    }
    for e in &game.events {
        match *e {
            Event::ShipDestroyed { cause, pos } => {
                // Nearest threat of the kind that killed it, for the distance column.
                let dist = match cause {
                    Cause::Rock(_) | Cause::Hyperspace => game
                        .rocks
                        .iter()
                        .map(|r| game::wrap_delta(pos, r.pos).len() - r.size.radius())
                        .fold(f32::INFINITY, f32::min),
                    Cause::Saucer | Cause::SaucerBullet => 0.0,
                };
                eprintln!(
                    "death tick={} wave={} cause={:?} nearest_rock_gap={:.0} speed={:.2} rocks={} lives_left={}",
                    game.tick,
                    game.wave,
                    cause,
                    dist,
                    game.ship.vel.len(),
                    game.rocks.len(),
                    game.lives,
                );
            }
            Event::HyperspaceOut { .. } => {
                eprintln!("hyperspace tick={} wave={}", game.tick, game.wave)
            }
            Event::WaveStart => {
                eprintln!("wave {} tick={} score={}", game.wave, game.tick, game.score)
            }
            _ => {}
        }
    }
}

fn result_line(game: &Game) -> String {
    format!(
        "result=gameover wave={} score={} ticks={} deaths={} hyperspaces={}",
        game.wave, game.score, game.tick, game.deaths, game.hyperspaces
    )
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_headless(cli: CliArgs) -> ! {
    let mut seed = screenshot::seed();
    let mut game = Game::new(seed);
    #[allow(clippy::disallowed_methods)]
    let t0 = std::time::Instant::now();
    let mut solver_ticks = 0u64;
    loop {
        let c = solver::choose(&game);
        solver_ticks += 1;
        game.step(c);
        log_events(cli.debug, &game);
        if game.outcome() == Outcome::GameOver {
            if cli.debug {
                eprintln!(
                    "us_per_tick={:.2}",
                    t0.elapsed().as_secs_f64() * 1e6 / solver_ticks as f64
                );
            }
            println!("{}", result_line(&game));
            if cli.once {
                std::process::exit(0);
            }
            seed = seed.wrapping_add(1);
            game = Game::new(seed);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub fn start() {
    let cli = parse_cli_args();
    if cli.no_ui {
        run_headless(cli);
    } else {
        macroquad::Window::from_config(conf(), async move {
            amain(cli).await;
        });
    }
}

#[cfg(target_arch = "wasm32")]
pub fn start() {
    macroquad::Window::from_config(conf(), async move {
        amain(parse_cli_args()).await;
    });
}

pub async fn play() {
    amain(bundled_cli()).await;
}

#[cfg(not(target_arch = "wasm32"))]
pub async fn play_until_exit() -> control::ExitReason {
    amain(bundled_cli()).await
}

// ── Visual-only effects ─────────────────────────────────────────────────────────

/// Unit-radius jagged outlines, scaled by rock radius. Cosmetic: `Rock::shape` picks one.
const ROCK_SHAPES: [&[(f32, f32)]; 4] = [
    &[
        (0.0, -1.0),
        (0.45, -0.85),
        (0.95, -0.4),
        (0.75, 0.05),
        (1.0, 0.45),
        (0.5, 0.95),
        (0.05, 0.7),
        (-0.45, 0.95),
        (-0.95, 0.4),
        (-0.8, -0.1),
        (-0.95, -0.5),
        (-0.4, -0.9),
    ],
    &[
        (-0.2, -1.0),
        (0.5, -0.95),
        (1.0, -0.3),
        (0.85, 0.35),
        (0.4, 0.5),
        (0.6, 0.95),
        (-0.15, 0.95),
        (-0.7, 0.7),
        (-1.0, 0.1),
        (-0.6, -0.15),
        (-0.85, -0.65),
    ],
    &[
        (0.1, -0.95),
        (0.7, -0.75),
        (0.6, -0.35),
        (1.0, 0.0),
        (0.7, 0.75),
        (0.15, 1.0),
        (-0.35, 0.6),
        (-0.8, 0.85),
        (-1.0, 0.2),
        (-0.75, -0.55),
        (-0.35, -0.6),
    ],
    &[
        (-0.3, -0.95),
        (0.3, -1.0),
        (0.9, -0.6),
        (1.0, 0.1),
        (0.65, 0.4),
        (0.8, 0.8),
        (0.2, 0.95),
        (-0.5, 0.9),
        (-0.95, 0.45),
        (-0.7, 0.0),
        (-1.0, -0.4),
    ],
];

/// Per-shape craters `(x, y, r)` in the same unit space, drawn inside the outline.
const ROCK_CRATERS: [&[(f32, f32, f32)]; 4] = [
    &[(-0.35, -0.3, 0.22), (0.35, 0.25, 0.16), (-0.1, 0.45, 0.1)],
    &[(0.3, -0.45, 0.2), (-0.4, 0.3, 0.18), (0.1, 0.1, 0.09)],
    &[(0.2, 0.35, 0.24), (-0.45, -0.1, 0.14), (0.35, -0.4, 0.1)],
    &[(-0.25, 0.35, 0.2), (0.4, -0.3, 0.17), (-0.35, -0.45, 0.09)],
];

/// Ship hull in ship-local units (x forward, y starboard); star-shaped about the origin
/// so it fills as a triangle fan.
const SHIP_HULL: [(f32, f32); 13] = [
    (17.0, 0.0),
    (8.0, -3.0),
    (3.0, -4.0),
    (-4.0, -12.0),
    (-9.0, -12.0),
    (-7.0, -5.0),
    (-11.0, -4.0),
    (-11.0, 4.0),
    (-7.0, 5.0),
    (-9.0, 12.0),
    (-4.0, 12.0),
    (3.0, 4.0),
    (8.0, 3.0),
];
const SHIP_COCKPIT: [(f32, f32); 4] = [(9.0, 0.0), (4.0, -2.0), (1.5, 0.0), (4.0, 2.0)];
/// Wing-root panel lines.
const SHIP_PANELS: [((f32, f32), (f32, f32)); 2] =
    [((3.0, -4.0), (-7.0, -5.0)), ((3.0, 4.0), (-7.0, 5.0))];
/// Engine nozzle x (flame origin).
const SHIP_TAIL: f32 = -11.0;

enum FxKind {
    /// Rock/saucer dust.
    Dot,
    /// Ship hull fragment: a short spinning segment.
    Segment { half: f32, angle: f32, spin: f32 },
    /// Hyperspace flash: glowing rings + radial streaks; `inward` converges (re-entry).
    Warp { inward: bool, phase: f32 },
}

struct Fx {
    pos: GVec,
    vel: GVec,
    ttl: u32,
    max_ttl: u32,
    kind: FxKind,
}

/// Visual-only effects, advanced in sim steps so pause/speed apply. Own RNG so nothing
/// here can perturb the sim.
struct Effects {
    fx: Vec<Fx>,
    rng: game::Rng,
}

impl Effects {
    fn new() -> Self {
        Self {
            fx: Vec::new(),
            rng: game::Rng::new(0xF00D),
        }
    }

    fn burst(&mut self, pos: GVec, n: u32, speed: f32, ttl: u32) {
        for _ in 0..n {
            let a = self.rng.range(0.0, std::f32::consts::TAU);
            let s = self.rng.range(0.2, 1.0) * speed;
            let t = ttl / 2 + self.rng.below(ttl / 2 + 1);
            self.fx.push(Fx {
                pos,
                vel: GVec::from_angle(a) * s,
                ttl: t,
                max_ttl: t,
                kind: FxKind::Dot,
            });
        }
    }

    fn on_events(&mut self, events: &[Event], ship_angle: f32) {
        for e in events {
            match *e {
                Event::RockDestroyed { size, pos } => {
                    let n = match size {
                        RockSize::Large => 16,
                        RockSize::Medium => 11,
                        RockSize::Small => 7,
                    };
                    self.burst(pos, n, 1.8, 40);
                }
                Event::SaucerDestroyed { pos, .. } => self.burst(pos, 18, 2.2, 50),
                Event::ShipDestroyed { pos, .. } => {
                    for i in 0..5 {
                        let a = ship_angle + i as f32 * 1.26;
                        let t = 80 + self.rng.below(40);
                        self.fx.push(Fx {
                            pos: pos + GVec::from_angle(a) * 4.0,
                            vel: GVec::from_angle(a + self.rng.range(-0.4, 0.4))
                                * self.rng.range(0.4, 1.2),
                            ttl: t,
                            max_ttl: t,
                            kind: FxKind::Segment {
                                half: self.rng.range(4.0, 8.0),
                                angle: self.rng.range(0.0, std::f32::consts::TAU),
                                spin: self.rng.range(-0.12, 0.12),
                            },
                        });
                    }
                    self.burst(pos, 10, 1.4, 50);
                }
                Event::HyperspaceOut { pos } | Event::HyperspaceIn { pos } => {
                    let inward = matches!(e, Event::HyperspaceIn { .. });
                    self.fx.push(Fx {
                        pos,
                        vel: GVec::default(),
                        ttl: WARP_TICKS,
                        max_ttl: WARP_TICKS,
                        kind: FxKind::Warp {
                            inward,
                            phase: self.rng.range(0.0, std::f32::consts::TAU),
                        },
                    });
                }
                _ => {}
            }
        }
    }

    fn step(&mut self) {
        for f in &mut self.fx {
            f.pos = game::wrap(f.pos + f.vel);
            f.vel = f.vel * 0.985;
            f.ttl -= 1;
            if let FxKind::Segment { angle, spin, .. } = &mut f.kind {
                *angle += *spin;
            }
        }
        self.fx.retain(|f| f.ttl > 0);
    }

    fn draw(&self) {
        for f in &self.fx {
            let life = f.ttl as f32 / f.max_ttl as f32;
            let c = Color::new(LINE.r, LINE.g, LINE.b, life);
            match f.kind {
                FxKind::Dot => draw_rectangle(f.pos.x - 1.0, f.pos.y - 1.0, 2.0, 2.0, c),
                FxKind::Segment { half, angle, .. } => {
                    let d = GVec::from_angle(angle) * half;
                    glow_line(f.pos - d, f.pos + d, c);
                }
                FxKind::Warp { inward, phase } => draw_warp(f.pos, 1.0 - life, inward, phase),
            }
        }
    }
}

// ── Drawing ─────────────────────────────────────────────────────────────────────

const WARP_TICKS: u32 = 32;

/// Hyperspace in/out. `p` runs 0→1. Out: rings expand from a bright core that fades.
/// In: rings converge and the core flares as the ship arrives.
fn draw_warp(pos: GVec, p: f32, inward: bool, phase: f32) {
    let (r, flash) = if inward {
        (44.0 - p * 38.0, p * (1.0 - p) * 4.0)
    } else {
        (6.0 + p * 38.0, (1.0 - p) * (1.0 - p))
    };
    let fade = 1.0 - p * p;
    let glow = |a: f32| Color::new(GLOW.r, GLOW.g, GLOW.b, a);
    // Core flash: soft halo then a white-hot centre.
    draw_circle(pos.x, pos.y, 26.0 * flash, glow(0.25 * flash));
    draw_circle(pos.x, pos.y, 12.0 * flash, glow(0.5 * flash));
    draw_circle(
        pos.x,
        pos.y,
        4.0 * flash,
        Color::new(1.0, 1.0, 1.0, flash.min(1.0)),
    );
    // Outer ring with a layered glow, plus a lagging inner ring.
    for (rr, k) in [(r, 1.0), (r * 0.6, 0.6)] {
        draw_circle_lines(pos.x, pos.y, rr, 9.0, glow(0.12 * fade * k));
        draw_circle_lines(pos.x, pos.y, rr, 4.0, glow(0.3 * fade * k));
        draw_circle_lines(
            pos.x,
            pos.y,
            rr,
            1.5,
            Color::new(LINE.r, LINE.g, LINE.b, fade * k),
        );
    }
    // Radial streaks riding the ring.
    for i in 0..10 {
        let d = GVec::from_angle(phase + i as f32 * std::f32::consts::TAU / 10.0);
        let c = Color::new(LINE.r, LINE.g, LINE.b, 0.8 * fade);
        glow_line(pos + d * (r * 0.75), pos + d * (r * 1.1 + 4.0), c);
    }
}

/// Fills a polygon that is star-shaped about `centre` as a triangle fan.
fn fill_fan(centre: GVec, pts: &[GVec], c: Color) {
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        draw_triangle(vec2(centre.x, centre.y), vec2(a.x, a.y), vec2(b.x, b.y), c);
    }
}

/// Crisp line over a wide low-alpha copy of itself — a cheap vector-monitor glow.
fn glow_line(a: GVec, b: GVec, c: Color) {
    draw_line(
        a.x,
        a.y,
        b.x,
        b.y,
        5.0,
        Color::new(GLOW.r, GLOW.g, GLOW.b, GLOW.a * c.a),
    );
    draw_line(a.x, a.y, b.x, b.y, 1.6, c);
}

fn glow_poly(points: &[GVec], c: Color) {
    for i in 0..points.len() {
        glow_line(points[i], points[(i + 1) % points.len()], c);
    }
}

/// Calls `f` at `pos` and at each wrapped image needed for an object of radius `r`
/// straddling an edge.
fn for_images(pos: GVec, r: f32, mut f: impl FnMut(GVec)) {
    let xs = [
        0.0,
        if pos.x < r {
            W
        } else if pos.x > W - r {
            -W
        } else {
            0.0
        },
    ];
    let ys = [
        0.0,
        if pos.y < r {
            H
        } else if pos.y > H - r {
            -H
        } else {
            0.0
        },
    ];
    for (i, &dx) in xs.iter().enumerate() {
        for (j, &dy) in ys.iter().enumerate() {
            if (i == 1 && dx == 0.0) || (j == 1 && dy == 0.0) {
                continue;
            }
            f(GVec::new(pos.x + dx, pos.y + dy));
        }
    }
}

/// Draws the ship at `pos` facing `angle`, `scale`d; `flame` is the thrust flicker
/// length (0 = no flame).
fn draw_ship(pos: GVec, angle: f32, scale: f32, line: Color, flame: f32) {
    let f = GVec::from_angle(angle);
    let s = GVec::new(-f.y, f.x);
    let at = |(x, y): (f32, f32)| pos + f * (x * scale) + s * (y * scale);
    if flame > 0.0 {
        let tail = SHIP_TAIL * scale;
        let back = |len: f32, w: f32| {
            [
                pos + f * tail + s * (w * scale),
                pos + f * ((SHIP_TAIL - len) * scale),
                pos + f * tail - s * (w * scale),
            ]
        };
        glow_poly(&back(flame, 3.5), FLAME);
        glow_poly(&back(flame * 0.5, 1.8), FLAME_CORE);
    }
    let hull = SHIP_HULL.map(at);
    fill_fan(pos, &hull, HULL_FILL);
    for (a, b) in SHIP_PANELS {
        draw_line(at(a).x, at(a).y, at(b).x, at(b).y, 1.0, CRATER);
    }
    glow_poly(&SHIP_COCKPIT.map(at), COCKPIT);
    glow_poly(&hull, line);
}

fn draw_game(game: &Game, frame: u64) {
    for r in &game.rocks {
        let rad = r.size.radius();
        let shape = ROCK_SHAPES[r.shape as usize % ROCK_SHAPES.len()];
        let (sin, cos) = r.angle.sin_cos();
        for_images(r.pos, rad, |p| {
            let at = |x: f32, y: f32| p + GVec::new(x * cos - y * sin, x * sin + y * cos) * rad;
            let pts: Vec<GVec> = shape.iter().map(|&(x, y)| at(x, y)).collect();
            fill_fan(p, &pts, ROCK_FILL);
            // Craters: a dim rim plus a brighter lit edge on one side, so the texture turns
            // with the rock. Small rocks keep only the biggest one.
            let craters = ROCK_CRATERS[r.shape as usize % ROCK_CRATERS.len()];
            let n = if r.size == RockSize::Small {
                1
            } else {
                craters.len()
            };
            for &(cx, cy, cr) in &craters[..n] {
                let c = at(cx, cy);
                let rr = cr * rad;
                draw_poly_lines(c.x, c.y, 8, rr, r.angle.to_degrees(), 1.0, CRATER);
                let lit = GVec::from_angle(r.angle + 2.4) * (rr * 0.35);
                draw_poly_lines(
                    c.x + lit.x,
                    c.y + lit.y,
                    8,
                    rr * 0.55,
                    0.0,
                    1.0,
                    CRATER_INNER,
                );
            }
            glow_poly(&pts, LINE);
        });
    }

    if let Some(s) = &game.saucer {
        let k = match s.size {
            SaucerSize::Large => 1.0,
            SaucerSize::Small => 0.58,
        };
        for_images(s.pos, 20.0 * k, |p| {
            let v = |x: f32, y: f32| p + GVec::new(x * k, y * k);
            glow_poly(
                &[
                    v(-20.0, 2.0),
                    v(-9.0, -5.0),
                    v(9.0, -5.0),
                    v(20.0, 2.0),
                    v(9.0, 9.0),
                    v(-9.0, 9.0),
                ],
                LINE,
            );
            glow_line(v(-20.0, 2.0), v(20.0, 2.0), LINE);
            glow_poly(
                &[v(-9.0, -5.0), v(-5.0, -12.0), v(5.0, -12.0), v(9.0, -5.0)],
                LINE,
            );
        });
    }

    for b in &game.bullets {
        let c = if b.from_ship { LINE } else { FLAME };
        draw_circle(
            b.pos.x,
            b.pos.y,
            3.5,
            Color::new(GLOW.r, GLOW.g, GLOW.b, 0.35),
        );
        draw_circle(b.pos.x, b.pos.y, 1.8, c);
    }

    let ship = &game.ship;
    // Respawn blink: visible 6 frames of every 10 while invulnerable.
    let blink_off = ship.invuln_ticks > 0 && (ship.invuln_ticks / 5) % 2 == 1;
    if ship.present() && !blink_off {
        let flame = match (ship.thrusting, frame % 4) {
            (false, _) | (_, 3) => 0.0,
            (_, 0 | 2) => 13.0,
            _ => 9.0,
        };
        for_images(ship.pos, 20.0, |p| {
            draw_ship(p, ship.angle, 1.0, LINE, flame)
        });
    }
}

fn draw_hud(game: &Game, label: &str, stream: bool) {
    draw_text(
        format!("{:>6}", game.score),
        16.0,
        30.0,
        HUD_FONT as f32,
        LINE,
    );
    for i in 0..game.lives.min(8) {
        let p = GVec::new(132.0 + i as f32 * 22.0, 22.0);
        draw_ship(p, -std::f32::consts::FRAC_PI_2, 0.6, DIM, 0.0);
    }
    let wave = format!("WAVE {}", game.wave);
    let wd = measure_text(&wave, None, HUD_FONT, 1.0);
    draw_text(&wave, (W - wd.width) * 0.5, 30.0, HUD_FONT as f32, DIM);
    if !stream {
        let ld = measure_text(label, None, HUD_FONT, 1.0);
        draw_text(label, W - 16.0 - ld.width, 30.0, HUD_FONT as f32, DIM);
    }
}

pub async fn amain(cli: CliArgs) -> control::ExitReason {
    let mut control = control::Control::new();
    rand::srand(control.seed());
    let mut game = Game::new(rand::rand() as u64);
    let mut fx = Effects::new();
    let sfx = sound::Sfx::load().await;
    let mut shot = screenshot::Capture::from_env();
    let mut accum = 0.0f32;
    let mut over_hold = 0u32;
    let mut daily_done = false;
    let mut frame = 0u64;

    // The preview screenshot is taken a few seconds in; pre-roll so it lands mid-wave
    // with rocks already split and the ship busy, not on the opening layout.
    if screenshot::is_capturing() {
        for _ in 0..900 {
            game.step(solver::choose(&game));
        }
    }

    let mut hud_texts: Vec<String> = (0..10)
        .map(|d| d.to_string())
        .chain(["WAVE ", "GAME OVER", "PAUSED", "MUTED", "x.", " "].map(String::from))
        .collect();
    hud_texts.push(control.label());
    let hud_refs: Vec<&str> = hud_texts.iter().map(String::as_str).collect();
    render_cache::prewarm_glyphs(&hud_refs, &[HUD_FONT, 48]);
    let mut hud = RenderCache::new(Rect::new(0.0, 0.0, W, HUD_H));
    let mut hud_key = (u64::MAX, 0u32, 0u32, String::new());

    loop {
        control.handle_keys();
        if let Some(r) = control.exit_requested() {
            break r;
        }
        if let Some(seed) = control.take_reseed() {
            rand::srand(seed);
            game = Game::new(rand::rand() as u64);
            fx = Effects::new();
            accum = 0.0;
            over_hold = 0;
            daily_done = false;
        }

        accum += control.scale(get_frame_time());
        let mut steps = 0;
        while !daily_done && accum >= game::SIM_DT && steps < MAX_STEPS_PER_FRAME {
            accum -= game::SIM_DT;
            steps += 1;
            if game.outcome() == Outcome::GameOver {
                over_hold += 1;
                fx.step();
                if over_hold < GAME_OVER_HOLD {
                    continue;
                }
                control.episode_complete("asteroids", game.score as i64);
                if control.daily_mode() {
                    control::share_result(&control::daily_verdict_text(
                        "Asteroids",
                        control::daily_puzzle_number(),
                        &format!("score {}, wave {}", game.score, game.wave),
                    ));
                    daily_done = true;
                    break;
                }
                if cli.once {
                    println!("{}", result_line(&game));
                    std::process::exit(0);
                }
                game = Game::new(rand::rand() as u64);
                over_hold = 0;
                continue;
            }
            let c = solver::choose(&game);
            let ship_angle = game.ship.angle;
            game.step(c);
            log_events(cli.debug, &game);
            fx.on_events(&game.events, ship_angle);
            sfx.on_events(&game.events);
            fx.step();
        }
        if steps == MAX_STEPS_PER_FRAME {
            accum = 0.0;
        }
        sfx.update(&game, control.scale(1.0) > 0.0);

        clear_background(BLACK);
        draw_game(&game, frame);
        fx.draw();

        let label = control.label();
        let key = (game.score, game.lives, game.wave, label.clone());
        if key != hud_key {
            hud.mark_dirty();
            hud_key = key;
        }
        let stream = control.stream_mode();
        hud.draw(|| draw_hud(&game, &label, stream));

        if game.outcome() == Outcome::GameOver {
            let t = "GAME OVER";
            let d = measure_text(t, None, 48, 1.0);
            draw_text(t, (W - d.width) * 0.5, H * 0.5, 48.0, LINE);
        }

        frame += 1;
        shot.tick();
        screenshot::handle_hotkey();
        control.draw_overlay();
        next_frame().await;
    }
}
