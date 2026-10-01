//! Labyrinth — a bot explores a procedurally generated maze it can only see one
//! corridor at a time, and the minimap shows exactly what it has worked out so far.
//!
//! Phase 3 of `.notes/labyrinth_plan.md`: pseudo-3D first-person raycaster view with a
//! knowledge-driven minimap inset, plus turn-then-walk movement animation.

use macroquad::prelude::*;
use render_cache::RenderCache;

mod anim;
mod banter;
mod game;
mod know;
mod maze;
mod solver;
mod sound;
mod util;
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
/// animation length (~0.4–0.6s); `control.scale` multiplies the whole thing.
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
        variant: initial_wasm_variant(),
    }
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn hcg_initial_variant_ordinal() -> i32;
}

/// Reads the `?variant=tower|dungeon|fog|mirror` URL query param (see
/// `xtask::variant_query_bridge`) so a link can land directly in that mode instead of the
/// `V`-cycle's `TowerClimb` starting point — `V` still cycles from there either way.
#[cfg(target_arch = "wasm32")]
fn initial_wasm_variant() -> Option<Mode> {
    match unsafe { hcg_initial_variant_ordinal() } {
        0 => Some(Mode::TowerClimb),
        1 => Some(Mode::DungeonCrawl),
        2 => Some(Mode::Fog),
        3 => Some(Mode::Mirror),
        _ => None,
    }
}

fn bundled_cli() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        #[cfg(target_arch = "wasm32")]
        variant: initial_wasm_variant(),
        #[cfg(not(target_arch = "wasm32"))]
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

/// What the camera shows of Dungeon keys/doors and the exit, lagging `Game` until the
/// animation reaches that moment (and sounding it then): a key vanishes as the bot's
/// foot lands on its cell, a door opens once its key has turned, the exit chimes on
/// arrival — not a walk-length early, when the solver committed to the step.
#[derive(Default)]
struct Shown {
    /// Junction glances held back until the door here has been opened.
    pending_looks: Vec<anim::Look>,
    /// Walls whose writing the bot has already read — turned to, or walked up to
    /// facing it square on — so it doesn't stop to read them again.
    read: std::collections::HashSet<(maze::Cell, usize)>,
    /// Garden sights (`Decor::looks_at` keys) already looked at.
    seen: std::collections::HashSet<usize>,
    keys_taken: u32,
    doors_open: u32,
    /// The locked door being unlocked (key id) and seconds since the key turned.
    unlocking: Option<(u8, f32)>,
    /// Picked up in `Game`, not yet shown: taken at the next footfall.
    keys_due: u32,
    /// The door the current step unlocks.
    door_due: Option<u8>,
    exit_due: bool,
    /// Office doors (Tower mode), indexed like `Maze::doors`: how far each has swung
    /// open, 0..1.
    swing: Vec<f32>,
    /// Which way each is swinging: +1 pushed open (`DoorPush` seen), -1 pulled shut
    /// behind the bot, 0 at rest.
    swinging: Vec<f32>,
    /// The office door the current step pushes open.
    push_due: Option<usize>,
    /// The office door being pulled shut (`Game::closed_door`), until the hand's on it.
    close_due: Option<usize>,
    /// The janitor's-closet sting has played this maze.
    closet_cued: bool,
    /// The current door push opens the closet: play the sting as it swings.
    closet_due: bool,
    /// Tower's lift: how far its doors have slid open (0..1), sliding, and whether the
    /// current step waits at them.
    lift: f32,
    lift_sliding: bool,
    lift_due: bool,
    /// Seconds since the boss conceded (the banner's clock), if he has.
    since_defeat: Option<f32>,
    /// An occupied washroom being tried: its answer is due at the handle (`DoorPush`),
    /// men's or not; then seconds since it answered (the caption's clock).
    busy_due: Option<bool>,
    since_busy: Option<f32>,
}

impl Shown {
    fn new(game: &Game) -> Shown {
        let n = game.maze.doors.len();
        Shown {
            swing: vec![0.0; n],
            swinging: vec![0.0; n],
            ..Shown::default()
        }
    }
}

/// The minimap floats over the 3D view semi-transparent, so the corridor behind it
/// still reads; the live bot marker drawn on top stays opaque.
fn minimap_cache(inset: Rect) -> RenderCache {
    RenderCache::new(inset).with_opacity(0.5)
}

/// Seconds per game tick while the bot argues with the boss (vs `TICK`): the
/// argument runs `game::ARGUE_TICKS` ticks, ~8 s.
const ARGUE_PACE: f32 = 0.17;
/// Ticks per line of the argument (speakers alternate).
pub const ARGUE_LINE: u32 = 6;

const BG: Color = Color::new(0.04, 0.04, 0.06, 1.0);
const HEADER_BG: Color = Color::new(0.04, 0.04, 0.06, 0.45);
const HUD: Color = Color::new(0.65, 0.65, 0.82, 1.0);

pub async fn amain(cli: CliArgs) -> control::ExitReason {
    let mut control = control::Control::new();
    rand::srand(control.seed());
    render_cache::prewarm_glyphs(
        &[
            "Labyrinth 0123456789 UP OUT LOST Tower The Backrooms Dungeon Fog Lost in the Mirror Garden floor keys mirrors bumps steps x optimal",
            &control.label(),
        ],
        &[18, 20, 22],
    );
    // Door signs, posters, whiteboard marker: baked into textures at one size.
    render_cache::prewarm_glyphs(&view::decor::labels(), &[view::label::BAKE_PX]);
    render_cache::prewarm_glyphs(&view::boss::texts(), &view::boss::SIZES);

    // The ambient wall runs every game at once: stay silent there.
    if control.stream_mode() {
        audio::playback::set_muted(true);
    }

    let art_seed = screenshot::seed();
    let mut sound = sound::Sound::load(art_seed).await;
    let mut mode = cli.variant.unwrap_or(Mode::TowerClimb);
    let (mut game, mut grid, mut theme, mut fx, mut decor) = fresh(mode, 1, art_seed);
    let mut shown = Shown::new(&game);
    let mut ear = sound::Ear::default();
    let mut anim = anim::Anim::new(game.know.pos, game.know.facing);
    let mut dwell = 0.0f32;
    let mut hold = 0.0f32;
    let mut shot = screenshot::Capture::from_env();
    let mut daily_done = false;
    let mut critters = view::critters::Critters::new(art_seed);

    let mut cached_inset = view::minimap_inset();
    let mut map_cache = minimap_cache(cached_inset);

    loop {
        control.handle_keys();
        if let Some(reason) = control.exit_requested() {
            sound.stop();
            break reason;
        }
        if let Some(seed) = control.take_reseed() {
            rand::srand(seed);
            dwell = 0.0;
            hold = 0.0;
            daily_done = false;
            (game, grid, theme, fx, decor) = fresh(mode, 1, art_seed);
            anim.reset(game.know.pos, game.know.facing);
            critters.clear_crumbs();
            shown = Shown::new(&game);
            map_cache.mark_dirty();
        }

        // `V` (or a one-finger swipe) cycles the mode — project convention.
        if is_key_pressed(KeyCode::V) || control.variant_swipe() {
            mode = mode.next();
            dwell = 0.0;
            hold = 0.0;
            daily_done = false;
            (game, grid, theme, fx, decor) = fresh(mode, 1, art_seed);
            anim.reset(game.know.pos, game.know.facing);
            critters.clear_crumbs();
            shown = Shown::new(&game);
            map_cache.mark_dirty();
        }

        let dt = control.scale(get_frame_time().min(0.1));
        match anim.update(dt) {
            Some(anim::AnimEvent::Footfall) => {
                sound.footstep(theme.kind);
                if decor.chain_near(anim.pos, 0.8) {
                    sound.chain();
                }
                // First time at a junction: look down the side corridors before going on.
                let k = &game.know;
                if !game.done() && k.visits(k.pos) == 1 {
                    let (ahead, back) = (k.facing, maze::opposite(k.facing));
                    let mut looks: Vec<anim::Look> = (0..4)
                        .filter(|&d| d != ahead && d != back && k.edge_open(k.pos, d) == Some(true))
                        .map(|d| anim::Look::dir(d, anim::LOOK_HOLD))
                        .collect();
                    // Then what's written on the walls here and the Garden's set pieces
                    // and oddities, here or through a side opening, each once, aimed so
                    // it fits the view (looking down, stooping, for a low one). Not one
                    // behind (passed), nor one straight ahead at eye level (in view all
                    // the way up). One in the same direction as a corridor glance or
                    // another look folds into it — unless it needs the head lower (the
                    // DRINK ME bottle under its sign), when it's a look of its own.
                    let facing = anim::dir_yaw(ahead);
                    let reads = decor
                        .reads_at(k.pos)
                        .into_iter()
                        .filter(|g| g.key != back && shown.read.insert((k.pos, g.key)))
                        .filter(|g| g.key != ahead || g.pitch > 0.0);
                    let open = |d| k.edge_open(k.pos, d) == Some(true);
                    let sights = decor.looks_at(k.pos, open).into_iter().filter(|g| {
                        let off = anim::ang_diff(facing, g.yaw).abs();
                        off <= 2.3 && (off >= 0.6 || g.pitch > 0.0) && shown.seen.insert(g.key)
                    });
                    for g in reads.chain(sights) {
                        let same = |l: &anim::Look| {
                            anim::ang_diff(l.yaw, g.yaw).abs() < 0.35
                                && (l.pitch - g.pitch).abs() < 0.1
                                && (l.stoop - g.stoop).abs() < 0.05
                        };
                        match looks.iter_mut().find(|l| same(l)) {
                            Some(l) => l.hold = anim::READ_HOLD,
                            None => looks.push(anim::Look {
                                yaw: g.yaw,
                                hold: anim::READ_HOLD,
                                pitch: g.pitch,
                                stoop: g.stoop,
                            }),
                        }
                    }
                    // One sweep, side to side, not back and forth.
                    crate::util::sort_by_key(&mut looks, |l| anim::ang_diff(facing, l.yaw));
                    // A shut office door here: open it first (the next tick, as soon as
                    // the bot looks at it), and glance round the rest afterwards.
                    if let Some(door) = k.marked_door(k.pos) {
                        looks.retain(|l| anim::ang_diff(l.yaw, anim::dir_yaw(door)).abs() >= 0.35);
                        shown.pending_looks = looks;
                    } else if !looks.is_empty() {
                        anim.scan(&looks);
                    }
                }
                if shown.keys_due != 0 {
                    shown.keys_taken |= std::mem::take(&mut shown.keys_due);
                    sound.key_pickup();
                }
                if std::mem::take(&mut shown.exit_due) {
                    sound.exit();
                    // Dungeon: on down the stairwell cut into the exit cell's wall.
                    if let Some(d) = game.maze.stairwell {
                        anim.descend(d, game.maze.wall_face());
                    }
                }
            }
            Some(anim::AnimEvent::Step) => sound.footstep(theme.kind),
            Some(anim::AnimEvent::KeyTurn) => {
                shown.unlocking = shown.door_due.map(|k| (k, 0.0));
                sound.key_turn();
                // The gate's squeal and clang, timed to its swing.
                sound.door_open();
            }
            Some(anim::AnimEvent::DoorOpen) => {
                if let Some(k) = shown.door_due.take() {
                    shown.doors_open |= 1 << k;
                }
                shown.unlocking = None;
            }
            Some(anim::AnimEvent::DoorPush) => {
                if std::mem::take(&mut shown.lift_due) {
                    shown.lift_sliding = true;
                    sound.elevator();
                } else {
                    if let Some(i) = shown.push_due.take() {
                        shown.swinging[i] = 1.0;
                    }
                    if let Some(i) = shown.close_due.take() {
                        shown.swinging[i] = -1.0;
                    }
                    if std::mem::take(&mut shown.closet_due) {
                        sound.closet();
                    }
                    if let Some(mens) = shown.busy_due.take() {
                        sound.busy(mens);
                        shown.since_busy = Some(0.0);
                    }
                    sound.office_door();
                }
            }
            None => {}
        }
        for (s, &way) in shown.swing.iter_mut().zip(&shown.swinging) {
            *s = (*s + way * dt / anim::SWING_SECS).clamp(0.0, 1.0);
        }
        if shown.lift_sliding {
            shown.lift = (shown.lift + dt / anim::SWING_SECS).min(1.0);
        }
        if let Some((_, t)) = &mut shown.unlocking {
            *t += dt;
        }
        if let Some(t) = &mut shown.since_busy {
            *t += dt;
        }
        if let Some(t) = &mut shown.since_defeat {
            *t += dt;
        }
        let arguing = game.arguing().is_some();

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
                        (game, grid, theme, fx, decor) = fresh(mode, next, art_seed);
                        anim.reset(game.know.pos, game.know.facing);
                        critters.clear_crumbs();
                        shown = Shown::new(&game);
                        map_cache.mark_dirty();
                    }
                }
            } else if !shown.pending_looks.is_empty()
                && game.know.marked_door(game.know.pos).is_none()
                && game.arguing().is_none()
            {
                // The door's open — and any argument with the boss behind it is over (the
                // bot keeps its eyes on him till he concedes): now the glances round the
                // junction.
                anim.scan(&std::mem::take(&mut shown.pending_looks));
            } else {
                dwell += dt;
                // The argument plays at a readable pace; everything else at the tick rate.
                let pace = if arguing { ARGUE_PACE } else { TICK };
                if dwell >= pace {
                    dwell = 0.0;
                    let advancing = game.tick();
                    // The argument's voices, alternating; the boss's fall.
                    match game.arguing() {
                        Some(t) if t % ARGUE_LINE == 1 => {
                            sound.argue((t / ARGUE_LINE) % 2 == 1, game.maze.lady_boss);
                        }
                        _ => {}
                    }
                    if game.boss_defeated() && shown.since_defeat.is_none() {
                        shown.since_defeat = Some(0.0);
                        sound.boss_defeated();
                        // It's over: back out of his office, eyes still on him.
                        anim.step_back();
                    }
                    // Fog mutates walls out of sight; the camera must see the new maze
                    // once it looks there.
                    if game.walls_changed {
                        grid = game.maze.wall_grid();
                        game.walls_changed = false;
                    }
                    log_tick(cli.debug, &game);
                    if let Some((from, dir)) = game.last_step {
                        // Glances held for a door here are moot once the bot walks on.
                        shown.pending_looks.clear();
                        shown.keys_due |= game.know.keys_held & !shown.keys_taken;
                        shown.door_due = game
                            .maze
                            .lock_at(from, dir)
                            .filter(|&k| shown.doors_open & (1 << k) == 0);
                        shown.exit_due = game.outcome == Some(Outcome::Won);
                        shown.push_due = game.opened_door;
                        // Into the lift: wait at its doors while they slide open.
                        shown.lift_due = game.maze.lift && from.step(dir) == game.maze.exit;
                        let gate = if shown.door_due.is_some() {
                            Some(anim::Gate::Key)
                        } else if shown.push_due.is_some() || shown.lift_due {
                            Some(anim::Gate::Push)
                        } else {
                            None
                        };
                        anim.begin_step(from, dir, gate);
                    } else if let Some((_, dir)) = game.bump {
                        anim.face(dir);
                    } else if let Some((i, dir)) = game.knocked {
                        // Try the handle; the door stays put, someone answers.
                        shown.busy_due = Some(game.maze.wc_door(i) != Some(false));
                        anim.open(dir, &[anim::Look::dir(dir, anim::PEEK_HOLD)], false);
                    } else if let Some((i, dir)) = game.closed_door {
                        // Turn to the door and pull it to, standing in the hall.
                        shown.close_due = Some(i);
                        anim.open(dir, &[], false);
                    } else if let Some(dir) = game.opened_from_doorway {
                        shown.push_due = game.opened_door;
                        // The IT room and the janitor's closet have things to read on
                        // their walls: stand in the doorway long enough to read them.
                        // The closet's sting plays as its door swings open.
                        let kind = game.opened_door.and_then(|i| {
                            let cell = game.maze.doors[i].cell;
                            game.maze
                                .rooms
                                .iter()
                                .find(|r| r.contains(cell))
                                .map(|r| r.kind)
                        });
                        // Something to read: hold on it. A room seen whole from the doorway:
                        // a quick glance at each side wall before the door's pulled to.
                        // Otherwise the bot goes in.
                        let door_yaw = anim::dir_yaw(dir);
                        let side = |s: f32| anim::Look {
                            yaw: door_yaw + s * anim::PEEK_TURN,
                            hold: anim::PEEK_HOLD,
                            pitch: 0.0,
                            stoop: 0.0,
                        };
                        // Seen whole: lean half a cell in for the glances.
                        let (after, lean) = match kind {
                            // Eyes on the boss for the whole argument: no looking round.
                            // Step up to his desk; `step_back` once he concedes.
                            Some(maze::RoomKind::Boss) => (Vec::new(), true),
                            // The IT room and the closet: lean in, look round, read the poster.
                            Some(maze::RoomKind::It | maze::RoomKind::Janitor) => (
                                vec![side(-1.0), anim::Look::dir(dir, anim::READ_HOLD), side(1.0)],
                                true,
                            ),
                            _ if game.opened_door.is_some_and(|i| game.room_known(i)) => {
                                (vec![side(-1.0), side(1.0)], true)
                            }
                            _ => (Vec::new(), false),
                        };
                        if kind == Some(maze::RoomKind::Janitor) && !shown.closet_cued {
                            shown.closet_cued = true;
                            shown.closet_due = true;
                        }
                        anim.open(dir, &after, lean);
                        if kind == Some(maze::RoomKind::Boss) {
                            anim.stay_leaning();
                        }
                    }
                    // Fog fades cells by age, which changes every tick without any
                    // new reveal setting `dirty`.
                    if game.know.dirty || game.mode == Mode::Fog {
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
            map_cache = minimap_cache(inset);
            cached_inset = inset;
        }
        let time = get_time();

        // First-person view fills the stage, then the live fx pass over it.
        fx.update_lamps(time);
        ear.update(&game.maze, game.know.pos);
        sound.ambience(theme.kind, &fx, &ear, anim.pos, get_frame_time().min(0.1));
        sound.fountain(decor.fountain(), &ear, anim.pos, get_frame_time().min(0.1));
        let light = fx.lighting(time, anim.pos);
        let fire = campfire(&game, theme.kind);
        // Game-time `dt`: the wildlife freezes with the pause (and speeds up with it).
        critters.update(
            dt,
            theme.kind,
            &grid,
            anim.pos,
            vec2(anim.yaw.cos(), anim.yaw.sin()),
        );
        for (call, at) in critters.take_calls() {
            sound.bird(call, at.distance(anim.pos));
        }
        let forest = theme.kind == view::theme::ThemeKind::Forest;
        // Forest: the fire and the critters sort in among the trees.
        if forest {
            critters.drop_crumb(anim.pos);
            fx.update_motes(get_frame_time().min(0.1));
        }
        let amid: Vec<Vec2> = if forest {
            critters
                .points()
                .into_iter()
                .chain(fire)
                .chain(fx.mote_points(anim.pos, time))
                .collect()
        } else {
            Vec::new()
        };
        let dynamic = view::decor::Dynamic {
            t: time as f32,
            boss_hp: game
                .boss
                .as_ref()
                .map_or(1.0, |b| b.hp as f32 / game::BOSS_HP as f32),
            lift: shown.lift,
            fire,
        };
        // What the mirrors show besides hedges and the bot.
        let reflect = |mv: &view::raycast::View| {
            decor.draw_reflection(mv, &theme.palette, &light, dynamic);
            critters.draw_all(mv, &theme.palette, &light);
        };
        let cam = view::raycast::draw(
            &grid,
            &theme,
            stage,
            anim.pos,
            anim.yaw,
            anim.eye_lift(),
            &light,
            anim.stride,
            &view::raycast::Props {
                doors_open: shown.doors_open,
                // The key turns, then the door swings open over the rest of the pause.
                door_swing: shown.unlocking.map(|(k, t)| {
                    let lead = sound::GATE_LEAD;
                    (k, ((t - lead) / (anim::UNLOCK_SECS - lead)).clamp(0.0, 1.0))
                }),
                swing: &shown.swing,
                amid: &amid,
                reflect: Some(&reflect),
                look_down: anim.pitch,
            },
        );
        let mut draw_decor = || {
            decor.draw(
                &cam,
                &theme.palette,
                &light,
                &grid.panels,
                &shown.swing,
                dynamic,
            )
        };
        if forest {
            // The bot's trail lies under everything standing out there.
            critters.draw_crumbs(&cam, &theme.palette, &light);
            // Far to near: the trees between each thing and the one behind it, then it —
            // painter's order is the trees' only occlusion, so critters, the campfire
            // and the motes all sort in among them. The decor (the campfire) goes at the
            // fire's depth, or last if there's none.
            enum Item {
                Critter(usize),
                Decor,
                Mote(view::fx::MoteDot),
            }
            let mut items: Vec<(f32, Item)> = critters
                .depths(&cam)
                .into_iter()
                .map(|(d, i)| (d, Item::Critter(i)))
                .collect();
            items.push((
                fire.map_or(f32::NEG_INFINITY, |f| cam.depth(f)),
                Item::Decor,
            ));
            items.extend(
                fx.mote_dots(&cam, time)
                    .into_iter()
                    .map(|(d, m)| (d, Item::Mote(m))),
            );
            crate::util::sort_by_key(&mut items, |t| -t.0);
            let trees = view::raycast::forest_trees(&cam, &grid);
            let mut far = amid.iter().map(|&p| cam.depth(p)).fold(0.0, f32::max);
            for (d, item) in items {
                view::raycast::draw_trees_between(&cam, &theme, &light, &trees, d, far);
                far = far.min(d);
                match item {
                    Item::Critter(i) => critters.draw(i, &cam, &theme.palette, &light),
                    Item::Decor => draw_decor(),
                    Item::Mote(m) => view::fx::draw_mote(&m),
                }
            }
            view::raycast::draw_trees_between(&cam, &theme, &light, &trees, f32::NEG_INFINITY, far);
        } else {
            draw_decor();
            critters.draw_all(&cam, &theme.palette, &light);
        }
        // Keys over the dressing, hidden by whatever of it stands nearer.
        view::keys::draw(
            &cam,
            decor.front_mut(),
            &theme.palette,
            light.base,
            &game.maze.keys,
            shown.keys_taken,
        );
        fx.draw_lamps(
            &cam,
            decor.front(),
            theme.palette.haze,
            theme.palette.fog,
            time,
        );
        if !forest {
            fx.draw_motes(&cam, get_frame_time().min(0.1), time);
        }

        // Minimap inset over a translucent backdrop.
        draw_rectangle(
            inset.x - 4.0,
            inset.y - 4.0,
            inset.w + 8.0,
            inset.h + 8.0,
            Color::new(0.02, 0.02, 0.03, 0.3),
        );
        let fog = game.mode == Mode::Fog;
        map_cache.draw(|| view::minimap::draw(&game.know, inset, time, fog));
        // Clamped: the garden's last step leaves the grid through its gate.
        let cellf = vec2((anim.pos.x - 1.5) * 0.5, (anim.pos.y - 1.5) * 0.5).clamp(
            Vec2::ZERO,
            vec2(game.maze.w as f32 - 1.0, game.maze.h as f32 - 1.0),
        );
        view::minimap::draw_bot(&game.know, inset, cellf, anim.yaw);

        // Dungeon: the stairwell swallows the view while the finished maze is held.
        if game.maze.stairwell.is_some() && game.outcome == Some(Outcome::Won) && anim.is_idle() {
            let k = (hold / 0.9).clamp(0.0, 1.0);
            draw_rectangle(
                stage.x,
                stage.y,
                stage.w,
                stage.h,
                Color::new(0.0, 0.0, 0.0, k * k * (3.0 - 2.0 * k)),
            );
        }
        // Tower climb: wipe between floors while the finished maze is held.
        if game.mode == Mode::TowerClimb && game.outcome == Some(Outcome::Won) {
            fx.draw_floor_wipe(stage, (hold / 0.9).clamp(0.0, 1.0));
        }

        view::boss::draw(stage, &game, shown.since_defeat);
        if let Some(t) = shown.since_busy {
            view::boss::draw_busy(stage, t);
        }

        // The header is a translucent strip over the top of the view.
        if !control.stream_mode() {
            draw_rectangle(0.0, 0.0, screen_width(), view::HEADER_H, HEADER_BG);
            draw_header(&game, &control);
        }

        shot.tick();
        screenshot::handle_hotkey();
        control.draw_overlay();
        next_frame().await;
    }
}

/// A fresh maze + its raycaster wall grid + its theme (one per mode, grain
/// pinned to `art_seed`).
fn fresh(
    mode: Mode,
    generation: u32,
    art_seed: u64,
) -> (
    Game,
    maze::WallGrid,
    view::theme::Theme,
    view::fx::Fx,
    view::decor::Decor,
) {
    let game = Game::with_mode(mode, generation);
    let grid = game.maze.wall_grid();
    let kind = view::theme::ThemeKind::for_mode(mode);

    #[cfg(not(target_arch = "wasm32"))]
    #[allow(clippy::disallowed_methods)] // native-only, cfg-gated
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

    let fx = view::fx::Fx::build(kind, art_seed, &game.maze);
    // Dressing varies per maze, not only per seed.
    let dress = art_seed ^ (generation as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    let decor = view::decor::Decor::build(kind, &game.maze, |p| fx.lamp_near(p, 0.3), dress);
    (game, grid, theme, fx, decor)
}

/// Forest: where the exit's campfire burns — off-centre in the exit cell, toward one of
/// its walls, so the bot arriving at the cell stops beside it, not in it. `None` while
/// that ground doesn't exist yet (fog).
fn campfire(game: &Game, kind: view::theme::ThemeKind) -> Option<Vec2> {
    let exit = game.maze.exit;
    if kind != view::theme::ThemeKind::Forest || !game.maze.is_born(exit) {
        return None;
    }
    let (gx, gy) = maze::WallGrid::cell_center(exit);
    let centre = vec2(gx as f32 + 0.5, gy as f32 + 0.5);
    let wall = (0..4).find(|&d| !game.maze.is_open(exit, d));
    Some(wall.map_or(centre, |d| {
        centre + vec2(maze::DIRS[d].0 as f32, maze::DIRS[d].1 as f32) * 0.3
    }))
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

fn draw_header(g: &Game, control: &control::Control) {
    let fs = 20.0;
    let y = view::hud::baseline(view::HEADER_H * 0.5, fs as u16);
    let status = match g.outcome {
        Some(Outcome::Won) => g.mode.win_verb(),
        Some(Outcome::Stuck) => "LOST",
        Some(Outcome::Survived) => "…",
        None => g.mode.title(),
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
        extra = format!("   mirrors {}", g.bumps);
    }
    let line = format!(
        "{status}   {} {}   steps {}   seen {:.0}%{extra}",
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
