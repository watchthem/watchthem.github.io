use macroquad::prelude::*;
use render_cache::RenderCache;

mod game;
mod generator;
mod solver;
mod sound;

use game::{Board, Game, H, Phase, Piece, W, full_rows, place_cells, rotation_states};
use generator::{GenMode, PieceGenerator};
use solver::Solver;

// Tetris's board is inherently tall and narrow (10x20) — unlike every other game here,
// which uses the shared 900x720 default (see `xtask::native_size`), it gets its own
// narrower canvas (matched in `conf()` and `xtask::native_size`/`max_fit_scale`, same
// precedent as `game2048`'s 500x610) sized to the board+panel content instead of
// stretching a small next-piece panel across a wide leftover gap.
const WIN_W: f32 = 600.0;
const WIN_H: f32 = 720.0;

const CELL: f32 = 28.0;
const BOARD_W: f32 = W as f32 * CELL;
const BOARD_H: f32 = H as f32 * CELL;
const PANEL_GAP: f32 = 40.0;
/// Inner padding between the panel's bordered container and its content (next-piece
/// boxes, stat lines).
const PANEL_PAD: f32 = 10.0;
/// Width of the NEXT-piece boxes and the stat lines below them.
const PANEL_W: f32 = 120.0;
const PANEL_OUTER_W: f32 = PANEL_W + PANEL_PAD * 2.0;
const BOARD_X: f32 = (WIN_W - (BOARD_W + PANEL_GAP + PANEL_OUTER_W)) / 2.0;
// Tall enough that a piece spawning at `SPAWN_ROW` (2 rows above the board, like real
// Tetris) clears the title text above it instead of drawing through it.
const BOARD_Y: f32 = 128.0;
const PANEL_OUTER_X: f32 = BOARD_X + BOARD_W + PANEL_GAP;
const PANEL_X: f32 = PANEL_OUTER_X + PANEL_PAD;

/// Row the falling piece visually spawns at — purely cosmetic (`Game::apply` places
/// pieces instantly; this is the renderer's own "drop-in" effect, matching how real
/// Tetris spawns a piece a row or two above the visible playfield before it falls in).
const SPAWN_ROW: f32 = -2.0;
/// Animation timeline, as fractions of `FallAnim::t`'s [0, 1] range: hold at the spawn
/// orientation/column, then rotate-and-slide into the target column, then hard-drop the
/// rest of the way — the same three beats a real player (or bot) visibly goes through,
/// not an instant teleport to the final placement. See `FallAnim::pose`.
const ROTATE_FRAC: f32 = 0.15;
const SLIDE_FRAC: f32 = 0.60;
const ANIM_SPEED: f32 = 1.8;
const FLASH_DUR: f32 = 0.28;
const OVER_PAUSE: f32 = 2.5;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::new(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0)
}

fn piece_color(piece: Piece) -> Color {
    match piece {
        Piece::I => rgb(45, 226, 230),
        Piece::O => rgb(234, 219, 65),
        Piece::T => rgb(178, 90, 235),
        Piece::S => rgb(90, 216, 105),
        Piece::Z => rgb(232, 82, 82),
        Piece::J => rgb(80, 120, 235),
        Piece::L => rgb(235, 150, 60),
    }
}

fn smoothstep(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// ── Piece-generation mode (V-cycle) ─────────────────────────────────────────────
// See `generator::GenMode` for what each concrete mode approximates. `Auto` rotates the
// four by generation, same pattern as Sudoku's difficulty cycle.

#[derive(Clone, Copy, PartialEq)]
enum VariantMode {
    Bag7,
    Classic,
    Tgm,
    Memoryless,
    Auto,
}

impl VariantMode {
    fn next(self) -> Self {
        match self {
            VariantMode::Bag7 => VariantMode::Classic,
            VariantMode::Classic => VariantMode::Tgm,
            VariantMode::Tgm => VariantMode::Memoryless,
            VariantMode::Memoryless => VariantMode::Auto,
            VariantMode::Auto => VariantMode::Bag7,
        }
    }

    fn gen_mode(self, generation: u32) -> GenMode {
        match self {
            VariantMode::Bag7 => GenMode::Bag7,
            VariantMode::Classic => GenMode::Classic,
            VariantMode::Tgm => GenMode::Tgm,
            VariantMode::Memoryless => GenMode::Memoryless,
            VariantMode::Auto => match generation % 4 {
                0 => GenMode::Bag7,
                1 => GenMode::Classic,
                2 => GenMode::Tgm,
                _ => GenMode::Memoryless,
            },
        }
    }

    fn name(self, generation: u32) -> &'static str {
        match self.gen_mode(generation) {
            GenMode::Bag7 => "7-bag",
            GenMode::Classic => "classic",
            GenMode::Tgm => "TGM",
            GenMode::Memoryless => "memoryless",
        }
    }

    fn label(self) -> &'static str {
        match self {
            VariantMode::Auto => " (auto)",
            _ => "",
        }
    }
}

/// Groups the piece-generation state (`piece_gen`), the authoritative game state (`game`), and
/// the solver driving it — everything a new episode or a variant switch replaces
/// together. Kept separate from `View`, which only tracks how the *current* piece's
/// placement is being animated.
struct Session {
    mode: VariantMode,
    piece_gen: PieceGenerator,
    game: Game,
    solver: Solver,
}

impl Session {
    fn new(mode: VariantMode, generation: u32) -> Self {
        let mut piece_gen = PieceGenerator::new(mode.gen_mode(generation));
        let game = Game::new(generation, &mut piece_gen);
        Self {
            mode,
            piece_gen,
            game,
            solver: Solver::new(),
        }
    }

    fn next_generation(&self) -> Self {
        Self::new(self.mode, self.game.generation + 1)
    }

    fn switch_variant(&self) -> Self {
        Self::new(self.mode.next(), self.game.generation + 1)
    }
}

/// A piece's cosmetic journey from spawn to its already-decided landing placement — see
/// `pose` for the three visible beats this plays out (hold at spawn, rotate + slide,
/// drop). `Game::apply` has already placed the piece by the time this exists; nothing
/// here feeds back into game state.
struct FallAnim {
    piece: Piece,
    spawn_shape: [(i32, i32); 4],
    target_shape: [(i32, i32); 4],
    spawn_col: i32,
    target_col: i32,
    target_row: f32,
    t: f32,
}

impl FallAnim {
    /// The shape/column/row to draw for the current `t`. Three beats, matching how a
    /// real game (or a bot playing one) actually looks: sit at the spawn column in the
    /// spawn orientation, then rotate (an instant snap — real Tetris doesn't tween
    /// rotation either) while sliding to the target column and dropping partway, then
    /// accelerate straight down the rest of the way (a hard drop).
    fn pose(&self) -> ([(i32, i32); 4], f32, f32) {
        let mid_row = SPAWN_ROW + (self.target_row - SPAWN_ROW) * 0.3;
        if self.t < ROTATE_FRAC {
            (self.spawn_shape, self.spawn_col as f32, SPAWN_ROW)
        } else if self.t < SLIDE_FRAC {
            let local = smoothstep((self.t - ROTATE_FRAC) / (SLIDE_FRAC - ROTATE_FRAC));
            let col = self.spawn_col as f32 + (self.target_col - self.spawn_col) as f32 * local;
            let row = SPAWN_ROW + (mid_row - SPAWN_ROW) * local;
            (self.target_shape, col, row)
        } else {
            let local = (self.t - SLIDE_FRAC) / (1.0 - SLIDE_FRAC);
            let ease_in = local * local;
            let row = mid_row + (self.target_row - mid_row) * ease_in;
            (self.target_shape, self.target_col as f32, row)
        }
    }
}

#[derive(PartialEq)]
enum ViewPhase {
    Falling,
    Flash,
    GameOver,
}

/// Purely cosmetic state layered on top of `Session`: `Game::apply` places a piece and
/// clears lines instantly, so everything here just interpolates from "before" to
/// "already-happened" for the player's benefit. `settled` is what the (cached) board
/// texture actually draws — it only catches up to `session.game.board` once the current
/// piece's fall/flash animation finishes, so the piece never appears twice.
struct View {
    phase: ViewPhase,
    settled: Board,
    fall: Option<FallAnim>,
    locked_board: Board,
    cleared_rows: Vec<usize>,
    flash_t: f32,
    over_t: f32,
}

impl View {
    fn new(session: &Session) -> Self {
        Self {
            phase: ViewPhase::Falling,
            settled: session.game.board,
            fall: None,
            locked_board: session.game.board,
            cleared_rows: Vec::new(),
            flash_t: 0.0,
            over_t: 0.0,
        }
    }

    /// Picks and starts animating the next placement, or switches to the `GameOver`
    /// overlay (reporting the finished episode) if the board's topped out.
    fn advance(&mut self, session: &mut Session, control: &mut control::Control, debug: bool) {
        session.game.refill(&mut session.piece_gen);
        if session.game.phase != Phase::Playing {
            control.episode_complete("tetris", session.game.score as i64);
            if control.daily_mode() {
                control::share_result(&control::daily_verdict_text(
                    "Tetris",
                    control::daily_puzzle_number(),
                    &format!("score {}", session.game.score),
                ));
            }
            if debug {
                eprintln!(
                    "game_over score={} lines={} level={} generation={}",
                    session.game.score,
                    session.game.lines,
                    session.game.level,
                    session.game.generation + 1
                );
            }
            self.phase = ViewPhase::GameOver;
            self.over_t = OVER_PAUSE;
            return;
        }

        let mv = session
            .solver
            .choose_move(&session.game)
            .expect("Phase::Playing guarantees at least one legal placement");
        let piece = session.game.current;
        let target_shape = rotation_states(piece)[mv.rot as usize];
        let mut locked = session.game.board;
        place_cells(&mut locked, piece, &target_shape, mv.col, mv.row);
        let cleared = full_rows(&locked);

        session.game.apply(mv);
        session.game.refill(&mut session.piece_gen);
        if debug {
            eprintln!(
                "drop piece={:?} rot={} col={} lines={} score={} gen={}",
                piece,
                mv.rot,
                mv.col,
                session.game.lines,
                session.game.score,
                session.game.generation + 1
            );
        }

        let spawn_shape = rotation_states(piece)[0];
        let spawn_width = spawn_shape.iter().map(|c| c.0).max().unwrap() + 1;
        let spawn_col = ((W as i32 - spawn_width) / 2).max(0);

        self.fall = Some(FallAnim {
            piece,
            spawn_shape,
            target_shape,
            spawn_col,
            target_col: mv.col,
            target_row: mv.row as f32,
            t: 0.0,
        });
        self.locked_board = locked;
        self.cleared_rows = cleared;
        self.phase = ViewPhase::Falling;
    }
}

/// Purely cosmetic board animation for the opening screen (see `amain`'s intro wait
/// loop) — the classic attract-mode look, in three beats that repeat for as long as the
/// intro plays: the board starts **pre-filled** (a random full pattern, every row
/// guaranteed at least one empty cell so nothing ever reads as a complete line, visible
/// immediately, no reveal-from-nothing animation); a **flood** then sweeps bottom-to-top,
/// converting each row it passes from that scattered piece-color pattern into one flat
/// **solid-color line**; once the whole board is solid, a **drain** sweeps top-to-bottom,
/// emptying each row it passes; once the board is fully empty, a fresh random pattern
/// appears and the cycle repeats. Never touches `Session`/`Game`; real gameplay only
/// starts once the intro finishes.
///
/// Uses its own `audio::Rng` rather than `macroquad::rand`'s global generator
/// deliberately — that one is seeded from `control.seed()` for deterministic gameplay
/// replay, and drawing from it here (at an unpredictable, frame-rate-dependent rate)
/// would shift every subsequent real piece the solver sees, breaking `HCG_SEED`
/// reproducibility for a purely decorative effect (same reasoning as `lib/audio`'s own
/// `rng.rs`).
struct DemoBoard {
    /// This cycle's scattered piece-color content, guaranteed no complete rows.
    pattern: Board,
    /// This cycle's flood color, one per row (picked once per cycle, not per frame, so
    /// a given row stays one solid color for the whole flood/drain sweep).
    row_colors: [Color; H],
    rng: audio::Rng,
    draining: bool,
    /// While flooding: rows `[line, H)` are solid (already swept), `[0, line)` still
    /// show `pattern`. Moves `H -> 0` (sweeping bottom-to-top, growing the solid region
    /// upward). While draining: rows `[0, line)` are empty (already swept), `[line, H)`
    /// are still solid. Moves `0 -> H` (sweeping top-to-bottom, growing the empty
    /// region downward) before a fresh pattern is generated and flooding restarts.
    line: f32,
}

/// What one row currently looks like — see `DemoBoard::row_state`.
enum DemoRow {
    Pattern,
    Solid,
    Empty,
}

impl DemoBoard {
    /// Full sweep (20 rows) takes 20 / 3 ≈ 6.7s, one full flood+drain cycle ≈ 13.3s —
    /// deliberately *longer* than `sound::INTRO_SECS` (~8.3s) so the cycle never
    /// resets to a fresh pattern mid-intro: a visitor sees one continuous flood (and
    /// the start of the drain), never a visible loop-back. An earlier draft (6
    /// rows/sec, ~6.7s/cycle) was tuned the opposite way — to guarantee at least one
    /// full loop played out — which read as a repeat/reset instead of one smooth rise.
    const ROWS_PER_SEC: f32 = 3.0;
    /// Chance any given cell is filled, before the guaranteed-empty-column exclusion
    /// below — tuned to look like a dense but clearly gappy board, not a solid wall.
    const FILL_CHANCE: f32 = 0.55;

    fn new(seed: u64) -> Self {
        let mut rng = audio::Rng::new(seed);
        let (pattern, row_colors) = Self::random_cycle(&mut rng);
        Self {
            pattern,
            row_colors,
            rng,
            draining: false,
            line: H as f32,
        }
    }

    /// A full board with every row guaranteed at least one empty cell (picked before
    /// the random fill, so it always stays empty) — the "no complete lines" constraint
    /// this animation depends on to never accidentally look like a real clear — plus
    /// this cycle's per-row flood colors.
    fn random_cycle(rng: &mut audio::Rng) -> (Board, [Color; H]) {
        let mut board: Board = [[None; W]; H];
        for row in board.iter_mut() {
            let guaranteed_empty = rng.index(W);
            for (c, cell) in row.iter_mut().enumerate() {
                if c != guaranteed_empty && rng.next_f32() < Self::FILL_CHANCE {
                    *cell = Some(Piece::ALL[rng.index(Piece::ALL.len())]);
                }
            }
        }
        let mut row_colors = [WHITE; H];
        for color in row_colors.iter_mut() {
            *color = piece_color(Piece::ALL[rng.index(Piece::ALL.len())]);
        }
        (board, row_colors)
    }

    fn update(&mut self, dt: f32) {
        if self.draining {
            self.line += dt * Self::ROWS_PER_SEC;
            if self.line >= H as f32 {
                let (pattern, row_colors) = Self::random_cycle(&mut self.rng);
                self.pattern = pattern;
                self.row_colors = row_colors;
                self.line = H as f32;
                self.draining = false;
            }
        } else {
            self.line -= dt * Self::ROWS_PER_SEC;
            if self.line <= 0.0 {
                self.line = 0.0;
                self.draining = true;
            }
        }
    }

    fn row_state(&self, r: usize) -> DemoRow {
        let above_line = (r as f32) < self.line;
        match (self.draining, above_line) {
            (false, true) => DemoRow::Pattern, // not yet flooded
            (false, false) => DemoRow::Solid,  // flooded
            (true, true) => DemoRow::Empty,    // already drained
            (true, false) => DemoRow::Solid,   // not yet drained
        }
    }
}

// ── CLI args (native only — meaningless in a browser tab) ───────────────────────

pub struct CliArgs {
    debug: bool,
    once: bool,
    variant: Option<VariantMode>,
    #[cfg(not(target_arch = "wasm32"))]
    no_ui: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cli_args() -> CliArgs {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (base, rest) = game_common::parse_base_args(&args);

    let mut variant = None;
    let mut i = 0;
    while i < rest.len() {
        match rest[i].as_str() {
            "--variant" => {
                i += 1;
                let v = rest.get(i).unwrap_or_else(|| {
                    eprintln!(
                        "--variant requires a value: bag7, classic, tgm, memoryless, or auto"
                    );
                    std::process::exit(2);
                });
                variant = Some(match v.as_str() {
                    "bag7" => VariantMode::Bag7,
                    "classic" => VariantMode::Classic,
                    "tgm" => VariantMode::Tgm,
                    "memoryless" => VariantMode::Memoryless,
                    "auto" => VariantMode::Auto,
                    other => {
                        eprintln!(
                            "unknown --variant value '{other}': expected bag7, classic, tgm, memoryless, or auto"
                        );
                        std::process::exit(2);
                    }
                });
            }
            other => {
                eprintln!(
                    "unknown argument '{other}' (expected --debug, --once, --no-ui, --variant <bag7|classic|tgm|memoryless|auto>)"
                );
                std::process::exit(2);
            }
        }
        i += 1;
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
    bundled_cli()
}

/// Exactly the `CliArgs` a browser build gets — no argv, no `--variant` pin (so the game
/// keeps its `VariantMode::Auto` rotation). Shared by the wasm `parse_cli_args` above and
/// `play()`, and compiles on native too (`no_ui: false`).
fn bundled_cli() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        variant: None,
        #[cfg(not(target_arch = "wasm32"))]
        no_ui: false,
    }
}

fn print_result(session: &Session) {
    println!(
        "result=game_over score={} lines={} level={} generation={}",
        session.game.score,
        session.game.lines,
        session.game.level,
        session.game.generation + 1
    );
}

/// Same solver loop as the windowed game, but with no window/GL context and no per-tick
/// pacing — `Game::apply` is an instant, discrete move (unlike a dt-driven game), so
/// there's no virtual-time stepping to do here, just run flat-out.
#[cfg(not(target_arch = "wasm32"))]
pub fn run_headless(cli: CliArgs) {
    macroquad::rand::srand(screenshot::seed());
    let mode = cli.variant.unwrap_or(VariantMode::Auto);
    let mut session = Session::new(mode, 0);

    loop {
        session.game.refill(&mut session.piece_gen);
        match session.game.phase {
            Phase::Playing => {
                let mv = session
                    .solver
                    .choose_move(&session.game)
                    .expect("Phase::Playing guarantees at least one legal placement");
                let piece = session.game.current;
                session.game.apply(mv);
                if cli.debug {
                    eprintln!(
                        "drop piece={:?} rot={} col={} lines={} score={} gen={}",
                        piece,
                        mv.rot,
                        mv.col,
                        session.game.lines,
                        session.game.score,
                        session.game.generation + 1
                    );
                }
            }
            Phase::GameOver => {
                print_result(&session);
                if cli.once {
                    return;
                }
                session = session.next_generation();
            }
        }
    }
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn conf() -> Conf {
    Conf {
        window_title: "Tetris".to_owned(),
        window_width: WIN_W as i32,
        window_height: WIN_H as i32,
        high_dpi: true,
        ..Default::default()
    }
}

/// Entry point for the standalone per-game binary — same window/`--no-ui` branching
/// `main()` used to do.
///
/// `Control::new()` unconditionally calls `prevent_quit()` (needed so the native
/// standalone *shell*'s `play_until_exit()` can read a window-close as
/// `ExitReason::Quit` instead of the process dying immediately) — which means a plain
/// `.await;` here that drops `amain`'s returned `ExitReason` leaves the process running
/// with a completed future and an unresponsive window after Esc or the close button:
/// there's no menu for a bare standalone binary to return to, and nothing was left to
/// actually terminate the process either. `std::process::exit(0)` after the await is
/// what the shell's own `run_game`/`play_until_exit()` caller does with the same
/// `ExitReason` (there, by returning to the menu loop instead of exiting — a standalone
/// binary has no menu, so exiting is the only sensible response to either reason here).
#[cfg(not(target_arch = "wasm32"))]
pub fn start() {
    let cli = parse_cli_args();
    if cli.no_ui {
        run_headless(cli);
        return;
    }
    macroquad::Window::from_config(conf(), async move {
        amain(cli).await;
        std::process::exit(0);
    });
}

/// Entry point for the standalone per-game binary — same window/`--no-ui` branching
/// `main()` used to do.
#[cfg(target_arch = "wasm32")]
pub fn start() {
    macroquad::Window::from_config(conf(), async move {
        amain(parse_cli_args()).await;
    });
}

/// Entry point for the merged multi-game binary (see `bundle/`): no argv parsing —
/// the bundled build gets the same `CliArgs` the browser build does.
pub async fn play() {
    amain(bundled_cli()).await;
}

/// Entry point for the standalone shell (see
/// `.notes/steam-standalone-menu-handoff.md`): runs until the player asks to leave
/// (Esc) or closes the window, then returns instead of looping forever. `play()` above
/// stays as-is for the browser, where there is nothing to return to.
#[cfg(not(target_arch = "wasm32"))]
pub async fn play_until_exit() -> control::ExitReason {
    amain(bundled_cli()).await
}

pub async fn amain(cli: CliArgs) -> control::ExitReason {
    let mut control = control::Control::new();
    rand::srand(control.seed());
    let mode = cli.variant.unwrap_or(VariantMode::Auto);
    let mut session = Session::new(mode, 0);
    let mut view = View::new(&session);
    let mut shot = screenshot::Capture::from_env();

    // The ambient wall (`?embed=1`/`?stream=1`) runs up to 11 of these simultaneously as
    // iframes — one tile's jingle/SFX would be noise on its own, let alone eleven at
    // once layered together. Muting here (rather than at each individual play call)
    // reuses `audio::playback`'s existing global gate, the same one `M` toggles — every
    // `Clip::play_once`/`play_looped` for the rest of this session becomes a no-op with
    // zero changes needed at any of those call sites. `stream_mode()` is read once at
    // startup and never changes mid-session, so setting this once here is enough.
    if control.stream_mode() {
        audio::playback::set_muted(true);
    }

    // A screenshot/clip capture wants actual gameplay in its fixed time window, not a
    // title screen — and playing a jingle nobody's listening to just wastes the render
    // budget those captures often run under. Skip both the sound and the wait entirely.
    let capturing = screenshot::is_capturing();

    // Loaded once per page/process load, not per episode restart — a title-screen jingle
    // that replayed every ~seconds (the AI restarts a finished episode automatically)
    // would be exhausting rather than "typical". See games/tetris/src/sound.rs.
    let sfx = sound::Sfx::load(FLASH_DUR).await;
    if !capturing {
        sfx.intro.play_once(0.6);
    }

    // Every `View::advance` call commits a piece to fall — playing `drop` right here,
    // once, immediately, rather than syncing it to a mid-animation `fall.t` threshold
    // (the first draft's approach) guarantees it always fires before any outcome sound
    // for *that* piece (`lock`/`clear[..]`/`game_over`, all gated on `fall.t` reaching
    // 1.0 on some later frame): those can only ever be checked after this closure has
    // already returned. A real Tetris's hard-drop sound is a crisp, immediate cue tied
    // to the drop being committed, not something smeared across a slow cosmetic
    // descent — this also reads closer to that.
    // `play_drop` is `false` only for the call right after a non-clearing `lock`: that
    // branch already just played `lock` (also a low thump — `chiptune::kick`) the
    // instant before calling this, so `drop` for the newly-spawned piece would fire
    // within the same frame, layering two bass thumps into what reads as one thump plus
    // an unwanted extra click. Every other call site (initial load, reseed, variant
    // switch, post-clear-flash, game-over restart) has no `lock` immediately before it,
    // so `drop` alone is the right cue there.
    let do_advance = |view: &mut View,
                      session: &mut Session,
                      control: &mut control::Control,
                      play_drop: bool| {
        view.advance(session, control, cli.debug);
        if play_drop && view.phase == ViewPhase::Falling {
            sfx.drop.play_once(0.4);
        }
    };

    // The locked board (up to 200 flat-colored cells, no text) is redrawn only when a
    // piece finishes falling/flashing, not every render frame — see `render_cache` and
    // `games/snake/src/main.rs` for the same pattern on an equally text-free board. The
    // falling piece and the line-clear flash both change continuously, so they stay live
    // per-frame draws on top, same split as every other game here.
    let mut board_cache = RenderCache::new(Rect::new(
        BOARD_X - 1.0,
        BOARD_Y - 1.0,
        BOARD_W + 2.0,
        BOARD_H + 2.0,
    ));

    // Hold on an opening screen for as long as `intro` plays instead of starting the
    // game underneath it — skipped entirely (0-length wait) during a capture (per
    // `capturing` above), on the wall (already muted above; a visitor there gets a live
    // tile, not a title card), and for a daily-challenge visitor (`?daily=1` — "today's
    // puzzle" should start immediately, not queue behind a jingle; the intro sound
    // itself still plays for daily mode, just without blocking gameplay on it).
    // `control.handle_keys()`/`exit_requested()` still run so Esc (native shell) works
    // even before the first piece has spawned. The board area shows `DemoBoard`'s
    // purely cosmetic flood-and-drain animation instead of a progress bar — the real
    // board's position/size, but no `Session`/`Game` involved.
    let mut intro_elapsed = 0.0f32;
    let skip_opening = capturing || control.stream_mode() || control.daily_mode();
    let intro_wait = if skip_opening { 0.0 } else { sound::INTRO_SECS };
    let mut demo = DemoBoard::new(macroquad::miniquad::date::now() as u64);
    while intro_elapsed < intro_wait {
        control.handle_keys();
        if let Some(reason) = control.exit_requested() {
            return reason;
        }
        let dt = get_frame_time();
        intro_elapsed += dt;
        demo.update(dt);
        clear_background(rgb(15, 15, 20));
        draw_opening_title();
        draw_demo_board(&demo);
        control.draw_overlay();
        next_frame().await;
    }

    do_advance(&mut view, &mut session, &mut control, true);
    board_cache.mark_dirty();

    loop {
        control.handle_keys();
        if let Some(reason) = control.exit_requested() {
            break reason;
        }
        if let Some(seed) = control.take_reseed() {
            rand::srand(seed);
            session = Session::new(session.mode, 0);
            view = View::new(&session);
            do_advance(&mut view, &mut session, &mut control, true);
            board_cache.mark_dirty();
        }
        let dt = control.scale(get_frame_time());

        if is_key_pressed(KeyCode::V) || control.variant_swipe() {
            session = session.switch_variant();
            view = View::new(&session);
            do_advance(&mut view, &mut session, &mut control, true);
            board_cache.mark_dirty();
        }

        match view.phase {
            ViewPhase::Falling => {
                if let Some(fall) = &mut view.fall {
                    let prev_t = fall.t;
                    fall.t = (fall.t + dt * ANIM_SPEED).min(1.0);
                    // Sonifies `FallAnim::pose`'s rotate beat; the drop sound itself
                    // fires earlier, at commit time — see `do_advance`. Only when the
                    // piece's shape actually changes — a piece that spawns already in
                    // its landing orientation still hits this beat visually (the pose
                    // snap is a no-op), but playing a "rotate" click for a piece that
                    // never rotated is both semantically wrong and, since it happens on
                    // most drops, the main source of an unwanted extra click piggybacking
                    // on the drop sound's tail — see `sound.rs`'s `drop` comment.
                    if prev_t < ROTATE_FRAC
                        && fall.t >= ROTATE_FRAC
                        && fall.spawn_shape != fall.target_shape
                    {
                        sfx.rotate.play_once(0.5);
                    }
                    if fall.t >= 1.0 {
                        if view.cleared_rows.is_empty() {
                            sfx.lock.play_once(0.5);
                            view.settled = session.game.board;
                            board_cache.mark_dirty();
                            do_advance(&mut view, &mut session, &mut control, false);
                            board_cache.mark_dirty();
                            if view.phase == ViewPhase::GameOver {
                                sfx.game_over.play_once(0.7);
                            }
                        } else {
                            let idx = (view.cleared_rows.len() - 1).min(3);
                            sfx.clear[idx].play_once(0.7);
                            view.settled = view.locked_board;
                            view.flash_t = 0.0;
                            view.phase = ViewPhase::Flash;
                            board_cache.mark_dirty();
                        }
                    }
                }
            }
            ViewPhase::Flash => {
                view.flash_t += dt;
                if view.flash_t >= FLASH_DUR {
                    view.settled = session.game.board;
                    board_cache.mark_dirty();
                    do_advance(&mut view, &mut session, &mut control, true);
                    board_cache.mark_dirty();
                    if view.phase == ViewPhase::GameOver {
                        sfx.game_over.play_once(0.7);
                    }
                }
            }
            ViewPhase::GameOver => {
                if cli.once {
                    print_result(&session);
                    std::process::exit(0);
                }
                // Daily-challenge run: freeze here rather than counting down to a
                // restart (see `control::Control::daily_mode`) — `View::advance` already
                // fired `share_result` the moment this phase was entered, and nothing
                // else moves `view.phase` off `GameOver`, so skipping the countdown is
                // enough to hold the board on its final frame indefinitely.
                if !control.daily_mode() {
                    view.over_t -= dt;
                    if view.over_t <= 0.0 {
                        session = session.next_generation();
                        view = View::new(&session);
                        do_advance(&mut view, &mut session, &mut control, true);
                        board_cache.mark_dirty();
                    }
                }
            }
        }

        clear_background(rgb(15, 15, 20));

        draw_hud(&session, &control);

        board_cache.draw(|| draw_board_static(&view.settled));

        if view.phase == ViewPhase::Falling
            && let Some(fall) = &view.fall
        {
            draw_falling(fall);
        }
        if view.phase == ViewPhase::Flash {
            draw_flash(&view.cleared_rows, view.flash_t);
        }
        if view.phase == ViewPhase::GameOver && !control.stream_mode() {
            draw_game_over(view.over_t, control.daily_mode());
        }

        shot.tick();
        screenshot::handle_hotkey();
        control.draw_overlay();
        next_frame().await;
    }
}

// ── Drawing ───────────────────────────────────────────────────────────────────

fn draw_cell(x: f32, y: f32, color: Color) {
    draw_rectangle(x + 1.0, y + 1.0, CELL - 2.0, CELL - 2.0, color);
}

/// Bordered background + interior grid lines shared by `draw_board_static` (real
/// gameplay) and `draw_demo_board` (the opening screen). Interior lines only (1..W /
/// 1..H, not 0..=W / 0..=H) — the outer edges are already marked by the border rect;
/// drawing a grid line directly on top of it there partially overwrote the border, and
/// asymmetrically enough between the left/right edges (line rasterization doesn't split
/// a 1px line evenly across a coordinate) that the left border ended up visibly thinner
/// than the right.
fn draw_board_frame() {
    draw_rectangle(
        BOARD_X - 1.0,
        BOARD_Y - 1.0,
        BOARD_W + 2.0,
        BOARD_H + 2.0,
        rgb(60, 60, 75),
    );
    draw_rectangle(BOARD_X, BOARD_Y, BOARD_W, BOARD_H, rgb(18, 18, 26));
    let grid = rgb(35, 35, 46);
    for c in 1..W {
        let x = BOARD_X + c as f32 * CELL;
        draw_line(x, BOARD_Y, x, BOARD_Y + BOARD_H, 1.0, grid);
    }
    for r in 1..H {
        let y = BOARD_Y + r as f32 * CELL;
        draw_line(BOARD_X, y, BOARD_X + BOARD_W, y, 1.0, grid);
    }
}

/// The locked board: background, grid lines, and every settled cell. No text at all —
/// see the `board_cache` comment in `amain` for why that matters.
fn draw_board_static(board: &Board) {
    draw_board_frame();
    for (r, row) in board.iter().enumerate() {
        for (c, cell) in row.iter().enumerate() {
            if let Some(piece) = cell {
                draw_cell(
                    BOARD_X + c as f32 * CELL,
                    BOARD_Y + r as f32 * CELL,
                    piece_color(*piece),
                );
            }
        }
    }
}

/// `DemoBoard`'s per-row `Pattern`/`Solid`/`Empty` state — see that type's own doc
/// comment for the three-beat animation this draws one frame of.
fn draw_demo_board(demo: &DemoBoard) {
    draw_board_frame();
    for r in 0..H {
        match demo.row_state(r) {
            DemoRow::Pattern => {
                for (c, cell) in demo.pattern[r].iter().enumerate() {
                    if let Some(piece) = cell {
                        draw_cell(
                            BOARD_X + c as f32 * CELL,
                            BOARD_Y + r as f32 * CELL,
                            piece_color(*piece),
                        );
                    }
                }
            }
            DemoRow::Solid => {
                draw_rectangle(
                    BOARD_X + 1.0,
                    BOARD_Y + r as f32 * CELL + 1.0,
                    BOARD_W - 2.0,
                    CELL - 2.0,
                    demo.row_colors[r],
                );
            }
            DemoRow::Empty => {}
        }
    }
}

/// The currently-falling piece, interpolated between its cosmetic spawn point and its
/// (already locked-in, per `Game::apply`) landing placement. Live every frame — unlike
/// `draw_board_static`, its position changes continuously.
fn draw_falling(fall: &FallAnim) {
    let (shape, col, row) = fall.pose();
    let color = piece_color(fall.piece);
    for &(dx, dy) in &shape {
        draw_cell(
            BOARD_X + (col + dx as f32) * CELL,
            BOARD_Y + (row + dy as f32) * CELL,
            color,
        );
    }
}

/// Blinks the rows about to be cleared a few times over `FLASH_DUR`, drawn on top of the
/// (already-locked, pre-clear) cached board texture. Live every frame — the blink state
/// changes continuously.
fn draw_flash(cleared_rows: &[usize], flash_t: f32) {
    let half_cycles = (flash_t / FLASH_DUR * 6.0) as i32;
    if half_cycles % 2 != 0 {
        return;
    }
    for &r in cleared_rows {
        draw_rectangle(
            BOARD_X,
            BOARD_Y + r as f32 * CELL,
            BOARD_W,
            CELL,
            Color::new(1.0, 1.0, 1.0, 0.85),
        );
    }
}

fn draw_piece_preview(x: f32, y: f32, w: f32, h: f32, piece: Piece) {
    const PREVIEW_CELL: f32 = 14.0;
    let shape = rotation_states(piece)[0];
    let sw = (shape.iter().map(|c| c.0).max().unwrap() + 1) as f32 * PREVIEW_CELL;
    let sh = (shape.iter().map(|c| c.1).max().unwrap() + 1) as f32 * PREVIEW_CELL;
    let ox = x + (w - sw) * 0.5;
    let oy = y + (h - sh) * 0.5;
    let color = piece_color(piece);
    for &(dx, dy) in &shape {
        draw_rectangle(
            ox + dx as f32 * PREVIEW_CELL + 1.0,
            oy + dy as f32 * PREVIEW_CELL + 1.0,
            PREVIEW_CELL - 2.0,
            PREVIEW_CELL - 2.0,
            color,
        );
    }
}

/// Title, mode/speed labels, and the side panel (next-piece previews + score/lines/
/// level). Cheap enough (a handful of `draw_text`/`draw_rectangle` calls, no per-cell
/// text) to redraw every frame directly rather than caching, same as every other game's
/// top HUD strip. Drawn unconditionally, even in stream mode — this is the game's own
/// visual identity (title, live score), not a HUD overlay for a spectator to hide. Only
/// the speed multiplier (meaningless with no visitor around to have changed it) is
/// stream-mode-gated, same split as `game2048`'s title/score-box HUD.
fn draw_hud(session: &Session, control: &control::Control) {
    let text = rgb(210, 210, 225);
    let dim = rgb(140, 140, 160);

    draw_text("TETRIS", BOARD_X, 46.0, 34.0, text);
    let mode_label = format!(
        "{}{}",
        session.mode.name(session.game.generation),
        session.mode.label()
    );
    draw_text(&mode_label, BOARD_X, 72.0, 18.0, dim);

    if !control.stream_mode() {
        let speed = control.label();
        let sd = measure_text(&speed, None, 20, 1.0);
        draw_text(&speed, WIN_W - 20.0 - sd.width, 46.0, 20.0, dim);
    }

    // The panel container: same border/fill treatment as the board, spanning its full
    // height. Without this, the panel's actual content (next-piece boxes + a handful of
    // stat lines) only fills the top third or so of the board's height, leaving a tall
    // stretch of bare background beneath it that reads as extra empty space on the right
    // — even though the board and panel are already horizontally centered as a pair.
    draw_rectangle(
        PANEL_OUTER_X - 1.0,
        BOARD_Y - 1.0,
        PANEL_OUTER_W + 2.0,
        BOARD_H + 2.0,
        rgb(60, 60, 75),
    );
    draw_rectangle(
        PANEL_OUTER_X,
        BOARD_Y,
        PANEL_OUTER_W,
        BOARD_H,
        rgb(18, 18, 26),
    );

    draw_text("NEXT", PANEL_X, BOARD_Y + 16.0, 20.0, dim);
    let mut y = BOARD_Y + 26.0;
    // Classic (NES-era) Tetris convention: one next piece, not a modern-guideline
    // multi-piece queue — see games/tetris/CLAUDE.md. Box is taller than the old
    // per-slot 68px so the single preview doesn't read as shrunken inside the panel.
    const NEXT_BOX_H: f32 = 96.0;
    if let Some(&piece) = session.game.queue.front() {
        draw_rectangle(PANEL_X, y, PANEL_W, NEXT_BOX_H, rgb(24, 24, 32));
        draw_piece_preview(PANEL_X, y, PANEL_W, NEXT_BOX_H, piece);
        y += NEXT_BOX_H;
    }

    y += 16.0;
    for (label, value) in [
        ("SCORE", session.game.score),
        ("LINES", session.game.lines),
        ("LEVEL", session.game.level),
        ("GEN", session.game.generation + 1),
    ] {
        let line = format!("{label}  {value}");
        draw_text(&line, PANEL_X, y, 20.0, text);
        y += 28.0;
    }
}

fn draw_game_over(over_t: f32, daily_mode: bool) {
    draw_rectangle(
        BOARD_X,
        BOARD_Y,
        BOARD_W,
        BOARD_H,
        Color::new(0.0, 0.0, 0.0, 0.72),
    );
    let cx = BOARD_X + BOARD_W * 0.5;
    let cy = BOARD_Y + BOARD_H * 0.5;

    let title = "GAME OVER";
    let d = measure_text(title, None, 30, 1.0);
    draw_text(title, cx - d.width * 0.5, cy - 10.0, 30.0, rgb(240, 90, 90));

    // Daily-challenge runs freeze here rather than restarting (see
    // `control::Control::daily_mode`) — a countdown to a restart that never happens
    // would lie, and `over_t` itself has stopped counting down anyway.
    let sub = if daily_mode {
        "Today's run is over.".to_owned()
    } else {
        format!("Restarting in {:.0}...", over_t.max(0.0))
    };
    let sd = measure_text(&sub, None, 18, 1.0);
    draw_text(
        &sub,
        cx - sd.width * 0.5,
        cy + 22.0,
        18.0,
        rgb(210, 210, 225),
    );
}

/// Opening-screen title, positioned exactly where `draw_hud` draws the real in-game
/// title (`BOARD_X`, 46.0, size 34) so the transition into gameplay doesn't jump — the
/// board area below it is `DemoBoard`'s flood-and-clear animation (`draw_board_static`
/// + `draw_flash`, drawn directly by `amain`'s wait loop), not drawn here.
fn draw_opening_title() {
    let text = rgb(210, 210, 225);
    let dim = rgb(140, 140, 160);
    draw_text("TETRIS", BOARD_X, 46.0, 34.0, text);
    draw_text("a Russian folk tune, jazz-comped", BOARD_X, 72.0, 18.0, dim);
}
