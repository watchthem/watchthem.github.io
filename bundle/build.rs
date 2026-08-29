//! Stages each game's preview.png into OUT_DIR for `menu_art.rs`'s `include_bytes!()`
//! calls, instead of embedding `dist/<game>/preview.png` directly. This lets `cargo
//! check`/clippy (and therefore `mise run check`) build `bundle` on a fresh clone or
//! after `mise run clean` — where `dist/` doesn't exist yet — by substituting a
//! placeholder when the real preview is missing. The placeholder is written to
//! `OUT_DIR`, never to `dist/` itself, so it can never be mistaken for a real screenshot
//! by `build-wasm`'s "does `dist/<name>/preview.png` already exist" skip check (see
//! `mise.toml`) — a stub written into `dist/` would silently poison every future real
//! deploy. See root `CLAUDE.md`'s "Native standalone shell" section.
//!
//! Content doesn't matter for the placeholder: these bytes are only ever decoded at
//! native shell runtime (see `menu_art.rs::load_previews`), never at compile time, so
//! `cargo check`/clippy never exercises them.

use std::env;
use std::fs;
use std::path::Path;

// Plain-alphabetical by games/ dir, same set `menu_art.rs::PREVIEW_BYTES` embeds.
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

// 1x1 black PNG.
const STUB_PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xB5, 0x1C, 0x0C,
    0x02, 0x00, 0x00, 0x00, 0x0B, 0x49, 0x44, 0x41, 0x54, 0x78, 0xDA, 0x63, 0x64, 0xF8, 0x0F, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xE3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44,
    0xAE, 0x42, 0x60, 0x82,
];

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    for name in GAME_NAMES {
        let src = Path::new("../dist").join(name).join("preview.png");
        let dest = Path::new(&out_dir).join(format!("preview_{name}.png"));
        println!("cargo:rerun-if-changed={}", src.display());
        match fs::read(&src) {
            Ok(bytes) => fs::write(&dest, bytes).unwrap(),
            Err(_) => fs::write(&dest, STUB_PNG).unwrap(),
        }
    }
}
