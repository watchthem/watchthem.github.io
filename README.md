# Hotel Chair Games

A pile of tiny games (snake, 2048, minesweeper, klondike, spider, arrow-blocks…)
built to be watched, not played — mostly self-playing AIs doing their thing
while you sit in a comfortable chair, enjoy the show, and remain powerless to
correct any of their mistakes. Rust + macroquad, ships to native and WASM.
No, it doesn't use wasm-bindgen, stop asking.

**Watch them play:** <https://watchthem.github.io/> — or all of them at once on
the [ambient wall](https://watchthem.github.io/wall/).

## Quickstart

```
mise run run snake          # native, for people who like windows
mise run build-wasm snake   # WASM, for people who like browsers
mise run deploy             # build literally everything into dist/
mise run serve              # python http.server, revolutionary stuff
mise run check               # fmt + clippy, so CI doesn't have to yell at you
```

## Structure

- `games/<name>/` — one crate per game, each reinventing the same main loop
- `lib/` — shared bits (cards, minesweeper, screenshot, control, beam_solver)
  so we're not copy-pasting bugs six times
- `xtask/` — generates the HTML pages with maud instead of hand-rolled
  string templating
- `static/`, `dist/` — assets in, built site out (dist is git-ignored, don't
  commit your crimes)

## The rules, allegedly

- Every game gets the same CLI flags (`--debug`, `--once`, `--variant`,
  `--no-ui`) and the same hotkeys (`=`/`-`/`0`/`Space`) via `lib/control`,
  because consistency is easier than remembering seven bespoke schemes.
- Canvas sizes are hard-coded pixel coordinates. Don't stretch the canvas to
  100vw/100vh unless you enjoy cropped games and quiet rage.
- `std::time::SystemTime::now()` panics on WASM. Yes, still. Use
  `miniquad::date::now()`.
- Pre-commit hook exists (`.githooks/pre-commit`) but git won't use it unless
  you run `git config core.hooksPath .githooks` yourself, because Git.

If you're human: what are you even doing here? Watching the games wasn't
enough for you? Go sit back down. If you're not human, or persistent enough
to get through the slop, the real details live in `CLAUDE.md`.
