//! Every game in one binary, with the game to run chosen at *runtime* rather than at
//! compile time.
//!
//! Why: ~99% of a game's shipped wasm is framework (macroquad/miniquad/fontdue/
//! ttf_parser/std + the default font and GL shaders in rodata), and each game's own logic
//! is well under 1% of it — so 11 separate per-game binaries pay for the same framework 11
//! times. Merging pays for it once, and every game page then fetches the *same* wasm URL,
//! so a second page visit (or the ambient wall's 11 iframes) is an HTTP cache hit instead
//! of another ~370KB download.
//!
//! Each game crate keeps its own standalone binary (`cargo run --bin snake`, and mise's
//! screenshot/clip capture) — this is an additional target, not a replacement. Native CLI
//! flags (`--debug`/`--once`/`--no-ui`) live there, not here: `play()` deliberately feeds
//! every game the same `CliArgs` its browser build gets.
//!
//! Native-only, this binary is also the standalone desktop shell: `shell` (see
//! `shell.rs`) owns one window, a game-selection menu, and dispatches into each game's
//! `play_until_exit()` instead of `play()` so a game can hand control back instead of
//! running forever — see `.notes/steam-standalone-menu-handoff.md`. The web build (the
//! `#[cfg(target_arch = "wasm32")]` half of this file) is untouched by any of that.

#[cfg(not(target_arch = "wasm32"))]
mod menu_art;
#[cfg(not(target_arch = "wasm32"))]
mod shell;

/// Games in the order their `hcg_game_id` index is assigned: plain alphabetical by
/// `games/<dir>`, which is what `xtask::bundle_game_index` indexes into when it bakes each
/// page's id. Note that's *not* `xtask::all_games()`'s order — that one sorts by display
/// `title()` ("2048" before "Arrow Blocks"), which would renumber everything the day a title
/// changed. `bundle_list_matches_games_dir` below fails the test pass if this array ever
/// drifts from the directory listing.
const GAME_NAMES: [&str; 12] = [
    "arrow-blocks",
    "bubble-shooter",
    "game2048",
    "klondike",
    "labyrinth",
    "match-3",
    "minesweeper",
    "snake",
    "spider",
    "sudoku",
    "tetris",
    "water-sort",
];

// Set by the page before the wasm module loads (`xtask::game_id_bridge`), so the selection
// is resolved *before* `Window::from_config` and each game keeps its own window title and
// native canvas size. Same miniquad-plugin mechanism as `control`'s `hcg_is_stream_mode`
// and `xtask::variant_query_bridge`.
#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn hcg_game_id() -> i32;
}

#[cfg(target_arch = "wasm32")]
fn selected_game() -> &'static str {
    let id = unsafe { hcg_game_id() };
    // A page that failed to bake an id (or baked a stale one after a game was removed)
    // shows *a* game rather than a blank canvas — the alternative is an index panic with
    // nothing on screen to explain it.
    GAME_NAMES
        .get(id as usize)
        .copied()
        .unwrap_or(GAME_NAMES[0])
}

/// Native entry point: the standalone desktop shell (see `shell.rs`) — one window, a
/// game-selection menu, `--game <name>` still boots straight into a game (the path
/// `mise run run-bundle <name>` drives).
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    shell::run();
}

/// One `Window::from_config` call per arm, not one shared call after a lookup: every
/// game's `play()` returns its own distinct `Future` type, so they can't be selected into
/// a single value without boxing (which would need an allocation and a `Pin<Box<dyn
/// Future>>` that `from_config` doesn't take).
#[cfg(target_arch = "wasm32")]
fn main() {
    match selected_game() {
        "arrow-blocks" => {
            macroquad::Window::from_config(arrow_blocks::conf(), arrow_blocks::play())
        }
        "bubble-shooter" => {
            macroquad::Window::from_config(bubble_shooter::conf(), bubble_shooter::play())
        }
        "game2048" => macroquad::Window::from_config(game2048::conf(), game2048::play()),
        "klondike" => macroquad::Window::from_config(klondike::conf(), klondike::play()),
        "labyrinth" => macroquad::Window::from_config(labyrinth::conf(), labyrinth::play()),
        "match-3" => macroquad::Window::from_config(match_3::conf(), match_3::play()),
        "minesweeper" => macroquad::Window::from_config(minesweeper::conf(), minesweeper::play()),
        "snake" => macroquad::Window::from_config(snake::conf(), snake::play()),
        "spider" => macroquad::Window::from_config(spider::conf(), spider::play()),
        "sudoku" => macroquad::Window::from_config(sudoku::conf(), sudoku::play()),
        "tetris" => macroquad::Window::from_config(tetris::conf(), tetris::play()),
        "water-sort" => macroquad::Window::from_config(water_sort::conf(), water_sort::play()),
        // Unreachable: `selected_game` only ever returns a `GAME_NAMES` entry. Left as a
        // loud failure rather than a silent fallback so adding a game to the array without
        // an arm here shows up immediately instead of quietly running arrow-blocks.
        other => panic!("no dispatch arm for game '{other}'"),
    }
}

#[cfg(test)]
mod tests {
    use super::GAME_NAMES;

    /// `GAME_NAMES` is a *positional* contract with `xtask` (index -> game, baked into each
    /// page's `hcg_game_id`), and xtask derives its side from a directory listing. Adding a
    /// game to `games/` without adding it here would silently shift every later index by
    /// one, pointing pages at the wrong game — so assert the two lists are identical.
    #[test]
    fn bundle_list_matches_games_dir() {
        let games_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../games");
        let mut found: Vec<String> = std::fs::read_dir(games_dir)
            .expect("games/ readable")
            .map(|e| {
                e.expect("dir entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        found.sort();
        assert_eq!(found, GAME_NAMES);
    }
}
