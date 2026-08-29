//! Native-only standalone shell: one window, a game-selection menu, and the ability to
//! return to that menu from any game — see `.notes/steam-standalone-menu-handoff.md`.
//! The web build never touches this module (see `main.rs`'s `#[cfg]` split at the crate
//! root) and keeps loading `dist/hcg.wasm` exactly as it did before this file existed.

use crate::GAME_NAMES;
use crate::menu_art::{self, HeroScene};
use macroquad::prelude::*;

const SHELL_W: f32 = 900.0;
const SHELL_H: f32 = 720.0;
const MENU_COLS: usize = 4;
const TILE_MARGIN: f32 = 16.0;
/// Header band height — the hero scene plus title live in here, above the tile grid.
const TILE_TOP: f32 = 190.0;
const TILE_BOTTOM: f32 = 60.0;
const HERO_X: f32 = 24.0;
const HERO_Y: f32 = 24.0;
const HERO_SCALE: f32 = 2.0;
/// Height a menu tile's bottom label strip takes up, drawn over the preview thumbnail.
const TILE_LABEL_H: f32 = 34.0;
/// A one-finger touch that moves less than this and releases within `TAP_MAX_SECS`
/// counts as a tap-select — the same start/end-phase distance+time check
/// `control::Control::handle_touch` uses to detect a swipe, inverted: there it's a
/// *minimum* distance/a *maximum* duration for a gesture to count as a swipe; here it's
/// a *maximum* distance for a gesture to still count as a tap rather than a drag/scroll.
const TAP_MAX_DIST: f32 = 24.0;
const TAP_MAX_SECS: f64 = 0.5;
/// Mirrors `control::Control`'s own (private) double-click-to-fullscreen window — the
/// menu runs before any game's `Control` exists, so it needs its own copy of this and
/// the fullscreen-toggle state below, same reason it tracks its own `tap_start`.
const DOUBLE_CLICK_SECS: f64 = 0.4;
/// See `control::Control`'s own (private) constant of the same name/purpose.
const FULLSCREEN_TOGGLE_COOLDOWN_SECS: f64 = 0.3;

/// Entry point called from `main()` on every native target. Parses `--game <name>` (if
/// given, boots straight into that game — this is what `mise run run-bundle <name>` and
/// `hcg --game <name>` drive) and otherwise starts on the menu.
pub fn run() {
    // Marks the whole process, not just this call — every game's `Control` (constructed
    // inside its own `amain`, whether reached via `--game` or the interactive menu)
    // reads this once at construction to decide whether Esc has an actual menu to
    // return to. A per-game standalone binary never calls this, so it correctly gets
    // `in_shell: false` — see `control::mark_in_shell`'s own doc comment.
    control::mark_in_shell();
    let boot = parse_boot_game();
    macroquad::Window::from_config(shell_conf(), shell_main(boot));
}

fn shell_conf() -> Conf {
    Conf {
        window_title: "Hotel Chair Games".to_owned(),
        window_width: SHELL_W as i32,
        window_height: SHELL_H as i32,
        high_dpi: true,
        ..Default::default()
    }
}

/// `Some(index into GAME_NAMES)` for `--game <name>`, `None` for no arguments (land on
/// the menu). Unlike the pre-shell `--game`-is-mandatory parsing this replaces, a bare
/// `hcg` with no arguments is now a normal, supported way to start the app.
fn parse_boot_game() -> Option<usize> {
    let mut args = std::env::args().skip(1);
    let mut wanted: Option<String> = None;
    while let Some(arg) = args.next() {
        match arg.strip_prefix("--game=") {
            Some(v) => wanted = Some(v.to_owned()),
            None if arg == "--game" => wanted = args.next(),
            _ => {
                eprintln!(
                    "unknown argument '{arg}' (expected --game <name>, or no arguments to land on the menu)\n  names: {}",
                    GAME_NAMES.join(", ")
                );
                std::process::exit(2);
            }
        }
    }
    let wanted = wanted?;
    match GAME_NAMES.iter().position(|g| *g == wanted) {
        Some(i) => Some(i),
        None => {
            eprintln!(
                "unknown game '{wanted}' (expected one of: {})",
                GAME_NAMES.join(", ")
            );
            std::process::exit(2);
        }
    }
}

/// `prevent_quit()` here (not just inside each game's own `control::Control::new()`)
/// covers the window-close click while the menu itself is showing, before any game's
/// `Control` has been constructed for this process.
async fn shell_main(boot: Option<usize>) {
    prevent_quit();
    // Loaded once for the whole process, not per menu visit: decoding 11 PNGs and
    // parsing the hero SVG on every return-to-menu would be wasted work for assets that
    // never change at runtime.
    let previews = menu_art::load_previews();
    let hero = HeroScene::load();
    let mut selected = boot.unwrap_or(0);
    let mut in_game = boot.is_some();
    loop {
        if in_game {
            match run_game(selected).await {
                control::ExitReason::Menu => {
                    in_game = false;
                    // A game's own `Control` tracks fullscreen per-instance and never
                    // un-fullscreens on the way out (Esc exits to the menu, it doesn't
                    // toggle F first) — without this, leaving a fullscreened game left
                    // the OS window fullscreened while the menu tried to resize itself
                    // down to `SHELL_W`x`SHELL_H` underneath that, and a fullscreened
                    // window ignores/fights a resize request on most window managers,
                    // leaving the menu's tile hit-boxes (computed from the *requested*
                    // size, not the actual on-screen one) misaligned with where
                    // anything actually rendered — clicks landed nowhere real. Always
                    // force windowed on the way back, whether or not the game that just
                    // exited was actually fullscreened; a harmless no-op otherwise.
                    set_fullscreen(false);
                    request_new_screen_size(SHELL_W, SHELL_H);
                    // `run_game`'s loop breaks on Esc *before* its own
                    // `next_frame().await`, so the key-pressed flag is still set this
                    // frame. Without this, `run_menu`'s own Esc-quits-menu check (top of
                    // its loop) would immediately consume that same press and close the
                    // whole app instead of showing the menu.
                    next_frame().await;
                }
                control::ExitReason::Quit => return,
            }
        } else {
            match run_menu(&mut selected, &previews, &hero).await {
                MenuAction::Play(i) => {
                    selected = i;
                    in_game = true;
                }
                MenuAction::Quit => return,
            }
        }
    }
}

enum MenuAction {
    Play(usize),
    Quit,
}

/// Tile layout for the menu grid, recomputed every frame from the live canvas size
/// (cheap — 11 rects) rather than cached: the canvas can be mid-resize right after
/// returning from a game (`request_new_screen_size` only applies on the next
/// `next_frame().await`, see its doc comment), and there is no correctness cost to
/// recomputing here the way there would be for a game's expensive per-frame redraw.
fn tile_rects(sw: f32, sh: f32) -> Vec<Rect> {
    let cols = MENU_COLS;
    let rows = GAME_NAMES.len().div_ceil(cols);
    let avail_w = sw - TILE_MARGIN * (cols as f32 + 1.0);
    let avail_h = sh - TILE_TOP - TILE_BOTTOM - TILE_MARGIN * (rows as f32 - 1.0);
    let tw = avail_w / cols as f32;
    let th = avail_h / rows as f32;
    (0..GAME_NAMES.len())
        .map(|i| {
            let col = i % cols;
            let row = i / cols;
            Rect::new(
                TILE_MARGIN + col as f32 * (tw + TILE_MARGIN),
                TILE_TOP + row as f32 * (th + TILE_MARGIN),
                tw,
                th,
            )
        })
        .collect()
}

async fn run_menu(selected: &mut usize, previews: &[Texture2D], hero: &HeroScene) -> MenuAction {
    let cols = MENU_COLS;
    let len = GAME_NAMES.len();
    // (x, y, start time) of an in-progress single-finger touch — mirrors
    // `control::Control`'s own `one_finger_start` field, but the menu runs before any
    // game (and its `Control`) exists, so it tracks this itself.
    let mut tap_start: Option<(f32, f32, f64)> = None;
    let mut fullscreen = false;
    let mut last_click = f64::NEG_INFINITY;
    // See `control::Control`'s own (private) `FULLSCREEN_TOGGLE_COOLDOWN_SECS` doc
    // comment: some window managers/backends refocus the window as part of a
    // fullscreen transition and replay whatever key was still physically down as a
    // fresh `KeyDown`, which `is_key_pressed` sees as a second toggle a frame or two
    // later — flickering straight back to the state just left. Ignore any toggle
    // (`F`, double-click, or Esc's own forced un-fullscreen) within this window of the
    // last one, regardless of trigger.
    let mut last_fullscreen_toggle = f64::NEG_INFINITY;
    loop {
        if is_quit_requested() {
            return MenuAction::Quit;
        }
        if is_key_pressed(KeyCode::Escape) {
            // Esc backs out of fullscreen first, same as a browser/game convention —
            // only quits outright once the menu is already windowed. Without this, Esc
            // on a fullscreened menu quit the whole app in one press, with no way to
            // just drop back to windowed and keep browsing.
            if fullscreen {
                let now = get_time();
                if now - last_fullscreen_toggle > FULLSCREEN_TOGGLE_COOLDOWN_SECS {
                    last_fullscreen_toggle = now;
                    fullscreen = false;
                    set_fullscreen(false);
                }
            } else {
                return MenuAction::Quit;
            }
        }

        // `F`/double-click toggles fullscreen here too, same trigger as every game's
        // own `control::Control` — the menu previously had no fullscreen handling at
        // all, so `F` only started working once a game was already running.
        let mut toggle_fullscreen = is_key_pressed(KeyCode::F);
        if is_mouse_button_pressed(MouseButton::Left) {
            let now = get_time();
            if now - last_click < DOUBLE_CLICK_SECS {
                toggle_fullscreen = true;
                last_click = f64::NEG_INFINITY;
            } else {
                last_click = now;
            }
        }
        let now = get_time();
        if toggle_fullscreen && now - last_fullscreen_toggle > FULLSCREEN_TOGGLE_COOLDOWN_SECS {
            last_fullscreen_toggle = now;
            fullscreen = !fullscreen;
            set_fullscreen(fullscreen);
        }
        if is_key_pressed(KeyCode::Right)
            && !(*selected + 1).is_multiple_of(cols)
            && *selected + 1 < len
        {
            *selected += 1;
        }
        if is_key_pressed(KeyCode::Left) && !(*selected).is_multiple_of(cols) {
            *selected -= 1;
        }
        if is_key_pressed(KeyCode::Down) && *selected + cols < len {
            *selected += cols;
        }
        if is_key_pressed(KeyCode::Up) && *selected >= cols {
            *selected -= cols;
        }

        let rects = tile_rects(screen_width(), screen_height());
        let (mx, my) = mouse_position();

        // Hovering a tile moves the keyboard/gamepad-style selection onto it, so the
        // highlighted tile always tracks whichever input the player used most recently
        // instead of mouse-move and arrow keys fighting over two separate cursors.
        if let Some(i) = rects.iter().position(|r| r.contains(vec2(mx, my))) {
            *selected = i;
        }

        if is_mouse_button_pressed(MouseButton::Left)
            && let Some(i) = rects.iter().position(|r| r.contains(vec2(mx, my)))
        {
            return MenuAction::Play(i);
        }
        for touch in touches() {
            match touch.phase {
                TouchPhase::Started => {
                    tap_start = Some((touch.position.x, touch.position.y, get_time()));
                }
                TouchPhase::Ended => {
                    if let Some((sx, sy, start_time)) = tap_start.take() {
                        let dx = touch.position.x - sx;
                        let dy = touch.position.y - sy;
                        let dist = (dx * dx + dy * dy).sqrt();
                        if dist <= TAP_MAX_DIST
                            && get_time() - start_time <= TAP_MAX_SECS
                            && let Some(i) = rects.iter().position(|r| r.contains(touch.position))
                        {
                            return MenuAction::Play(i);
                        }
                    }
                }
                TouchPhase::Cancelled => tap_start = None,
                _ => {}
            }
        }
        if is_key_pressed(KeyCode::Enter)
            || is_key_pressed(KeyCode::KpEnter)
            || is_key_pressed(KeyCode::Space)
        {
            return MenuAction::Play(*selected);
        }

        clear_background(Color::new(0.05, 0.05, 0.09, 1.0));

        hero.draw(HERO_X, HERO_Y, HERO_SCALE);
        let (hero_w, _) = hero.size_at(HERO_SCALE);
        let title_x = HERO_X + hero_w + 24.0;
        draw_text("Hotel Chair Games", title_x, 64.0, 40.0, WHITE);
        draw_text(
            "Games, played by an AI, that watch themselves finish",
            title_x,
            96.0,
            20.0,
            Color::new(0.6, 0.6, 0.7, 1.0),
        );
        draw_text(
            "Enter: Play    Esc: Quit    Arrows/Mouse/Touch: Navigate",
            title_x,
            126.0,
            18.0,
            Color::new(0.5, 0.5, 0.6, 1.0),
        );

        for (i, r) in rects.iter().enumerate() {
            let picked = i == *selected;
            menu_art::draw_cover(&previews[i], *r);
            // Dim the whole tile a touch so the label strip (and the selection border)
            // read clearly against a bright screenshot; the label strip itself is darker
            // still, same "translucent overlay for text legibility" idea a normal game
            // launcher's tile art uses.
            draw_rectangle(r.x, r.y, r.w, r.h, Color::new(0.0, 0.0, 0.0, 0.15));
            let label_y = r.y + r.h - TILE_LABEL_H;
            draw_rectangle(
                r.x,
                label_y,
                r.w,
                TILE_LABEL_H,
                Color::new(0.0, 0.0, 0.0, 0.55),
            );
            if picked {
                draw_rectangle_lines(r.x, r.y, r.w, r.h, 3.0, Color::new(0.4, 0.75, 1.0, 1.0));
            }
            let title = xtask::title(GAME_NAMES[i]);
            let fs = 20.0f32;
            let td = measure_text(&title, None, fs as u16, 1.0);
            draw_text(
                &title,
                r.x + (r.w - td.width) * 0.5,
                label_y + TILE_LABEL_H * 0.5 + td.height * 0.35,
                fs,
                WHITE,
            );
        }

        next_frame().await;
    }
}

/// Resizes the shell window to game's native `conf()` size before running it — the
/// simplest of the options `.notes/steam-standalone-menu-handoff.md` lists for reconciling
/// one shared shell window with 11 games' differing `Conf::window_width`/`window_height`
/// (tetris 600x720, game2048 500x610, the other nine 900x720). Games draw at absolute
/// pixel coordinates and don't scale to `screen_width()`/`screen_height()` (root
/// CLAUDE.md, "Canvas sizing is load-bearing"), so a mismatched window for a frame or two
/// right after the resize request lands (`request_new_screen_size` applies on the next
/// `next_frame().await`, not immediately) is a brief visual wobble, not a correctness bug.
fn size_to(conf: Conf) {
    // Same reasoning as the Menu-return path above, mirrored for the opposite
    // direction: if the menu itself was fullscreened (its own `F` toggle, added
    // alongside this fix), a game starting up would otherwise inherit a fullscreened
    // OS window while its fresh `control::Control` thinks it's windowed — the same
    // requested-size-vs-actual-size mismatch, just entering a game instead of leaving
    // one. Always force windowed before resizing into a game.
    set_fullscreen(false);
    request_new_screen_size(conf.window_width as f32, conf.window_height as f32);
}

/// One `play_until_exit()` call per arm, not a shared call after a lookup — same reason
/// `bundle`'s wasm dispatch (`main.rs`) is a `match`: every game's `play_until_exit()`
/// future is its own distinct type.
async fn run_game(idx: usize) -> control::ExitReason {
    match GAME_NAMES[idx] {
        "arrow-blocks" => {
            size_to(arrow_blocks::conf());
            arrow_blocks::play_until_exit().await
        }
        "bubble-shooter" => {
            size_to(bubble_shooter::conf());
            bubble_shooter::play_until_exit().await
        }
        "game2048" => {
            size_to(game2048::conf());
            game2048::play_until_exit().await
        }
        "klondike" => {
            size_to(klondike::conf());
            klondike::play_until_exit().await
        }
        "labyrinth" => {
            size_to(labyrinth::conf());
            labyrinth::play_until_exit().await
        }
        "match-3" => {
            size_to(match_3::conf());
            match_3::play_until_exit().await
        }
        "minesweeper" => {
            size_to(minesweeper::conf());
            minesweeper::play_until_exit().await
        }
        "snake" => {
            size_to(snake::conf());
            snake::play_until_exit().await
        }
        "spider" => {
            size_to(spider::conf());
            spider::play_until_exit().await
        }
        "sudoku" => {
            size_to(sudoku::conf());
            sudoku::play_until_exit().await
        }
        "tetris" => {
            size_to(tetris::conf());
            tetris::play_until_exit().await
        }
        "water-sort" => {
            size_to(water_sort::conf());
            water_sort::play_until_exit().await
        }
        other => panic!("no dispatch arm for game '{other}'"),
    }
}
