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

pub use game::Mode;
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
    /// `--variant tower|dungeon|fog|mirror` — pin a mode instead of the default
    /// (`TowerClimb`). `None` = the game's V-cycle starting point.
    pub variant: Option<game::Mode>,
    #[cfg(not(target_arch = "wasm32"))]
    pub no_ui: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cli_args() -> CliArgs {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (base, rest) = game_common::parse_base_args(&args);
    let mut variant = None;
    let mut it = rest.iter();
    while let Some(tok) = it.next() {
        match tok.as_str() {
            "--variant" => {
                let v = it.next().map(|s| s.as_str()).unwrap_or("");
                match game::Mode::parse(v) {
                    Some(m) => variant = Some(m),
                    None => {
                        eprintln!("--variant expects tower|dungeon|fog|mirror");
                        std::process::exit(2);
                    }
                }
            }
            other => {
                eprintln!(
                    "unknown argument '{other}' (expected --debug, --once, --no-ui, --variant <tower|dungeon|fog|mirror>)"
                );
                std::process::exit(2);
            }
        }
    }
    CliArgs {
        debug: base.debug,
        once: base.once,
        variant,
        no_ui: base.no_ui,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn parse_cli_args() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        variant: None,
    }
}

fn bundled_cli() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        variant: None,
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
    let mode = cli.variant.unwrap_or(Mode::TowerClimb);
    let mut g = Game::with_mode(mode, 1);
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
            g = Game::with_mode(mode, g.generation + 1);
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
            "Labyrinth 0123456789 UP OUT LOST Tower Dungeon Fog Mirror floor keys bumps steps x optimal",
            &control.label(),
        ],
        &[18, 20, 22],
    );

    let art_seed = screenshot::seed();
    let mut mode = cli.variant.unwrap_or(Mode::TowerClimb);
    let (mut game, mut grid, mut theme, mut fx) = fresh(mode, 1, art_seed);
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
            (game, grid, theme, fx) = fresh(mode, 1, art_seed);
            anim.reset(game.know.pos, game.know.facing);
            map_cache.mark_dirty();
        }

        // `V` (or a one-finger swipe) cycles the mode — project convention.
        if is_key_pressed(KeyCode::V) || control.variant_swipe() {
            mode = mode.next();
            dwell = 0.0;
            hold = 0.0;
            daily_done = false;
            (game, grid, theme, fx) = fresh(mode, 1, art_seed);
            anim.reset(game.know.pos, game.know.facing);
            map_cache.mark_dirty();
        }

        let dt = control.scale(get_frame_time().min(0.1));
        anim.update(dt);
        if !game.done() {
            anim.look_toward(anim::dir_yaw(game.know.look_hint()), dt);
        }

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
                        // Won → next floor/maze harder; Stuck/Survived → restart the
                        // ladder from floor 1.
                        let next = if game.outcome == Some(Outcome::Won) {
                            game.generation + 1
                        } else {
                            1
                        };
                        (game, grid, theme, fx) = fresh(mode, next, art_seed);
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
                    } else if let Some((from, dir)) = game.bump {
                        anim.begin_bump(from, dir);
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
        let time = get_time();

        // First-person view fills the stage, then the live fx pass over it.
        let light = fx.light_mul(time);
        view::raycast::draw(&grid, &theme, stage, anim.pos, anim.yaw, anim.bob, light);
        fx.draw_motes(stage, get_frame_time().min(0.1), time);
        let exit_near = game
            .know
            .exit_sight
            .map(|d| (1.0 - d as f32 / game::LOS_RADIUS as f32).clamp(0.0, 1.0))
            .unwrap_or(0.0);
        fx.draw_exit_glow(stage, time, exit_near);

        // Minimap inset over a translucent backdrop.
        draw_rectangle(
            inset.x - 4.0,
            inset.y - 4.0,
            inset.w + 8.0,
            inset.h + 8.0,
            Color::new(0.02, 0.02, 0.03, 0.72),
        );
        let fog = game.mode == Mode::Fog;
        map_cache.draw(|| view::minimap::draw(&game.know, inset, time, fog));
        let cellf = vec2((anim.pos.x - 1.5) * 0.5, (anim.pos.y - 1.5) * 0.5);
        view::minimap::draw_bot(&game.know, inset, cellf, anim.yaw);

        // Tower climb: wipe between floors while the finished maze is held.
        if game.mode == Mode::TowerClimb && game.outcome == Some(Outcome::Won) {
            fx.draw_floor_wipe(stage, (hold / 0.9).clamp(0.0, 1.0));
        }

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
fn fresh(
    mode: Mode,
    generation: u32,
    art_seed: u64,
) -> (Game, maze::WallGrid, view::theme::Theme, view::fx::Fx) {
    let game = Game::with_mode(mode, generation);
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

    let fx = view::fx::Fx::build(kind, art_seed);
    (game, grid, theme, fx)
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
        Some(Outcome::Won) => g.mode.win_verb(),
        Some(Outcome::Stuck) => "LOST",
        Some(Outcome::Survived) => "…",
        None => g.mode.label(),
    };
    let floor_word = if g.mode == Mode::TowerClimb {
        "floor"
    } else {
        "gen"
    };
    let mut extra = String::new();
    if g.mode == Mode::DungeonCrawl {
        extra = format!(
            "   keys {}/{}",
            g.know.keys_held.count_ones(),
            g.maze.keys.len()
        );
    } else if g.mode == Mode::Mirror {
        extra = format!("   bumps {}", g.bumps);
    }
    // Theme name only when it isn't already echoed by the mode word.
    let theme_tag = if theme.label() == status {
        String::new()
    } else {
        format!("   {}", theme.label())
    };
    let line = format!(
        "{status}{theme_tag}   {} {}   steps {}   seen {:.0}%{extra}",
        floor_word,
        g.generation,
        g.steps,
        g.know.coverage() * 100.0,
    );
    draw_text(&line, 10.0, y, fs, HUD);
    let label = control.label();
    let d = measure_text(&label, None, fs as u16, 1.0);
    draw_text(&label, screen_width() - d.width - 10.0, y, fs, HUD);
}
