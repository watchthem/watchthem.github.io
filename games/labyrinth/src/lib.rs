//! Labyrinth — a bot explores a procedurally generated maze it can only see one
//! corridor at a time, and the minimap shows exactly what it has worked out so far.
//!
//! Phase 3 of `.notes/labyrinth_plan.md`: pseudo-3D first-person raycaster view with a
//! knowledge-driven minimap inset, plus turn-then-walk movement animation.

use macroquad::prelude::*;
use render_cache::RenderCache;

mod anim;
mod game;
mod know;
mod maze;
mod solver;
mod view;

use game::{Game, Outcome};

/// `macroquad::rand` is a process-global RNG, so any test that calls `srand` then reads
/// it races every other such test under cargo's parallel harness. RNG-sensitive tests
/// take this lock.
#[cfg(test)]
pub(crate) static RNG_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
pub(crate) fn rng_guard() -> std::sync::MutexGuard<'static, ()> {
    RNG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Idle dwell between steps — a short "thinking" beat once the previous step's
/// turn+walk animation has finished playing out. The visible cadence is this plus the
/// animation length (~0.25s); `control.scale` multiplies the whole thing.
pub const TICK: f32 = 0.05;

pub fn conf() -> Conf {
    Conf {
        window_title: "Labyrinth".to_owned(),
        window_width: 900,
        window_height: 720,
        high_dpi: true,
        ..Default::default()
    }
}

// ── CLI args (native only) ─────────────────────────────────────────────────────

pub struct CliArgs {
    pub debug: bool,
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
    CliArgs {
        debug: false,
        once: false,
    }
}

fn bundled_cli() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        #[cfg(not(target_arch = "wasm32"))]
        no_ui: false,
    }
}

fn log_tick(debug: bool, g: &Game) {
    if !debug {
        return;
    }
    match g.last_step {
        Some((from, dir)) => eprintln!(
            "step {} from ({},{}) dir={} pos=({},{}) coverage={:.2}",
            g.steps,
            from.x,
            from.y,
            know::DIR_NAMES[dir],
            g.know.pos.x,
            g.know.pos.y,
            g.know.coverage(),
        ),
        None => eprintln!("no move — {}", g.result_line()),
    }
}

/// Headless: no window, no GL, no miniquad — pure step loop.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_headless(cli: CliArgs) -> ! {
    rand::srand(screenshot::seed());
    let mut g = Game::new(1);
    loop {
        if g.tick() {
            log_tick(cli.debug, &g);
        } else {
            if cli.debug {
                eprintln!("episode end — {}", g.result_line());
            }
            if cli.once {
                println!("{}", g.result_line());
                std::process::exit(0);
            }
            g = Game::new(g.generation + 1);
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

const BG: Color = Color::new(0.04, 0.04, 0.06, 1.0);
const HUD: Color = Color::new(0.65, 0.65, 0.82, 1.0);

pub async fn amain(cli: CliArgs) -> control::ExitReason {
    let mut control = control::Control::new();
    rand::srand(control.seed());
    render_cache::prewarm_glyphs(
        &[
            "Labyrinth 0123456789 Won Stuck Explore steps ratio x",
            &control.label(),
        ],
        &[18, 20, 22],
    );

    let art_seed = screenshot::seed();
    let (mut game, mut grid, mut theme) = fresh(1, art_seed);
    let mut anim = anim::Anim::new(game.know.pos, game.know.facing);
    let mut dwell = 0.0f32;
    let mut hold = 0.0f32;
    let mut shot = screenshot::Capture::from_env();
    let mut daily_done = false;

    let mut cached_inset = view::minimap_inset();
    let mut map_cache = RenderCache::new(cached_inset);

    loop {
        control.handle_keys();
        if let Some(reason) = control.exit_requested() {
            break reason;
        }
        if let Some(seed) = control.take_reseed() {
            rand::srand(seed);
            dwell = 0.0;
            hold = 0.0;
            daily_done = false;
            (game, grid, theme) = fresh(1, art_seed);
            anim.reset(game.know.pos, game.know.facing);
            map_cache.mark_dirty();
        }

        let dt = control.scale(get_frame_time().min(0.1));
        anim.update(dt);

        // Step only once the previous step's animation has fully played out, plus a
        // short dwell — so turn/walk are always visible start to finish.
        if !daily_done && anim.is_idle() {
            if game.done() {
                hold += dt;
                if hold >= 0.9 {
                    hold = 0.0;
                    if control.daily_mode() {
                        daily_done = true;
                    } else if !cli.once {
                        let next = game.generation + 1;
                        (game, grid, theme) = fresh(next, art_seed);
                        anim.reset(game.know.pos, game.know.facing);
                        map_cache.mark_dirty();
                    }
                }
            } else {
                dwell += dt;
                if dwell >= TICK {
                    dwell = 0.0;
                    let advancing = game.tick();
                    log_tick(cli.debug, &game);
                    if let Some((from, dir)) = game.last_step {
                        anim.begin_step(from, dir);
                    }
                    if game.know.dirty {
                        map_cache.mark_dirty();
                        game.know.dirty = false;
                    }
                    if !advancing {
                        control.episode_complete("labyrinth", score(&game));
                        if control.daily_mode() {
                            control::share_result(&control::daily_verdict_text(
                                "Labyrinth",
                                control::daily_puzzle_number(),
                                &verdict_clause(&game),
                            ));
                        } else if cli.once {
                            println!("{}", game.result_line());
                            std::process::exit(0);
                        }
                    }
                }
            }
        }

        clear_background(BG);

        let stage = view::stage_rect();
        let inset = view::minimap_inset();
        if inset != cached_inset {
            map_cache = RenderCache::new(inset);
            cached_inset = inset;
        }

        // First-person view fills the stage.
        view::raycast::draw(&grid, &theme, stage, anim.pos, anim.yaw, anim.bob);

        // Minimap inset over a translucent backdrop.
        draw_rectangle(
            inset.x - 4.0,
            inset.y - 4.0,
            inset.w + 8.0,
            inset.h + 8.0,
            Color::new(0.02, 0.02, 0.03, 0.72),
        );
        let time = get_time();
        map_cache.draw(|| view::minimap::draw(&game.know, inset, time));
        let cellf = vec2((anim.pos.x - 1.5) * 0.5, (anim.pos.y - 1.5) * 0.5);
        view::minimap::draw_bot(&game.know, inset, cellf, anim.yaw);

        if !control.stream_mode() {
            draw_header(&game, theme.kind, &control);
        }

        shot.tick();
        screenshot::handle_hotkey();
        control.draw_overlay();
        next_frame().await;
    }
}

/// A fresh maze + its raycaster wall grid + its theme (rotates per generation, grain
/// pinned to `art_seed`).
fn fresh(generation: u32, art_seed: u64) -> (Game, maze::WallGrid, view::theme::Theme) {
    let game = Game::new(generation);
    let grid = game.maze.wall_grid();
    let kind = view::theme::ThemeKind::for_generation(generation);

    #[cfg(not(target_arch = "wasm32"))]
    let t0 = std::time::Instant::now();
    let theme = view::theme::Theme::build(kind, art_seed);
    #[cfg(not(target_arch = "wasm32"))]
    if generation == 1 {
        eprintln!(
            "labyrinth: {} wall texture ({}px) generated in {:?}",
            kind.label(),
            view::theme::TEX,
            t0.elapsed()
        );
    }

    (game, grid, theme)
}

fn score(g: &Game) -> i64 {
    // Lower is better in spirit, but the analytics score wants "more = better": reward
    // winning and penalise wasted walk. Optimal run ~= 1000.
    match g.outcome {
        Some(Outcome::Won) => (1000.0 / g.optimal_ratio()).round() as i64,
        _ => 0,
    }
}

fn verdict_clause(g: &Game) -> String {
    match g.outcome {
        Some(Outcome::Won) => format!(
            "out in {} steps ({:.1}x optimal)",
            g.steps,
            g.optimal_ratio()
        ),
        Some(Outcome::Stuck) => "lost in the maze".into(),
        _ => format!("still walking after {} steps", g.steps),
    }
}

fn draw_header(g: &Game, theme: view::theme::ThemeKind, control: &control::Control) {
    let fs = 20.0;
    let y = view::HEADER_H * 0.72;
    let status = match g.outcome {
        Some(Outcome::Won) => "OUT",
        Some(Outcome::Stuck) => "LOST",
        Some(Outcome::Survived) => "…",
        None => g.mode.label(),
    };
    let tail = if g.done() {
        format!("{:.1}x optimal", g.optimal_ratio())
    } else {
        format!("seen {:.0}%", g.know.coverage() * 100.0)
    };
    let line = format!(
        "{status}   {}   gen {}   steps {}   {tail}",
        theme.label(),
        g.generation,
        g.steps,
    );
    draw_text(&line, 10.0, y, fs, HUD);
    let label = control.label();
    let d = measure_text(&label, None, fs as u16, 1.0);
    draw_text(&label, screen_width() - d.width - 10.0, y, fs, HUD);
}
