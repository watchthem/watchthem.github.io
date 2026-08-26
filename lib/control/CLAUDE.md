# lib/control

Shared meta-control layer every game embeds — hotkeys for speed/pause/mute/fullscreen,
not gameplay input (no game in this workspace takes player moves; see root CLAUDE.md).

## Hotkeys

| Key | Effect | Platform |
|---|---|---|
| `=`/`-` | speed ×1.1 / ÷1.1, clamped [0.1, 10] | both |
| `0` | reset speed to 1x | both |
| `Space` | pause (`scale(dt)` returns 0) | both |
| `M` | mute/unmute audio | both |
| `F` / double-click | toggle fullscreen | native: `macroquad::window::set_fullscreen`; WASM: page-level JS (`xtask::fullscreen_bridge`) |
| `V` | cycle game variant (games that have one) | both |
| `R` | seed editor (view/type/replay a seed) | both |
| `S` | save screenshot | page-level JS (`xtask::screenshot_bridge`) |
| `?` | hotkey popup | page-level JS (`xtask::hotkey_popup`) |
| `Esc` | close popup / back to menu (bundle shell) or quit (standalone binary) | native |

Every game draws `control.label()` (`"x1.000"`/`"PAUSED"`, `"  MUTED"` suffix when muted)
in its own HUD — see any game's `lib.rs`.

**New hotkey checklist**: add to `Control::handle_keys()` (or a page-level bridge for a
WASM-only DOM concern), then update **both** `draw_popup`'s `LINES` (native popup) and
`xtask::hotkey_popup`'s `dl` (WASM popup) — the popup lies if only one is updated. Prefer
folding a new mode into an existing cycling hotkey (`V`) over adding a dedicated key — see
match-3/klondike/spider's variant cycles; only add a new key if the existing control
genuinely can't express the distinction.

## Esc (`Menu` vs `Quit`)

Only the native standalone shell (`bundle/src/shell.rs`) has an actual menu for Esc to
return to — a per-game standalone binary (`start()`) has none, Esc there just quits the
process. `Control` can't tell these apart from inside a game (both paths call the same
shared `amain()`), so the shell calls `control::mark_in_shell()` once, before running
any game — a process-wide flag (`IN_SHELL`), not a parameter threaded through every
game's `amain`/`start`/`play_until_exit`, since which binary a process *is* never
changes mid-run (`hcg --game <name>` still goes through the shell's `play_until_exit`,
same as landing on the menu first would). `Control::new()` snapshots the flag once at
construction. Drives three things together — don't fix one without the others:
`exit_requested()`'s actual `ExitReason` (Menu only in-shell, Quit otherwise — a
standalone binary treats either the same via `process::exit(0)`, but the reported
reason should still match reality), `draw_overlay()`'s corner-hint text, and
`draw_popup()`'s `Esc` line in the full hotkey panel.

## Mute (`M`)

Cross-platform toggle (unlike fullscreen — muting has no external DOM state that can
change behind our back, so no need to duplicate keydown handling in page JS).
`handle_keys()` toggles a `muted` field and mirrors it into `audio::playback::set_muted`
(a process-wide flag every `audio::Clip::play_once`/`play_looped` checks — games never
check `control.muted()` themselves before playing a sound). On WASM, muting
*additionally* calls `hcg_set_audio_muted` to suspend/resume the real browser
`AudioContext` (`xtask::audio_mute_bridge` + `xtask::audio_context_capture_script`, which
must run *before* `mq_js_bundle.js` — unlike every other bridge — to wrap
`window.AudioContext` and capture the instance quad-snd's bundled audio backend
constructs, since nothing else exposes it) — genuinely releasing the audio hardware while
muted (e.g. a connected Bluetooth headset isn't held routed to a silent tab), not just
silencing playback while still holding the device open.

## Fullscreen (`F` / double-click)

**Cannot** go through `macroquad::window::set_fullscreen` on WASM — that calls
`canvas.requestFullscreen()`, and browsers force a fullscreened element to
`width/height: 100%` via an unoverridable `!important` UA style, which stomps the pinned
native-resolution canvas box `xtask::native_size_style` depends on (see root CLAUDE.md's
"Canvas sizing is load-bearing"). `fullscreen_bridge` instead fullscreens `<html>`,
leaving the canvas element — and its own pinned size / `fitCanvas()` scale-to-fit
transform — untouched.

**Known upstream bug, not fixable from this repo: native fullscreen is broken on Linux
X11.** `miniquad`'s X11 backend (`native/linux_x11.rs::set_fullscreen`, confirmed
unfixed in current upstream `master`, not just the pinned `0.4.10`) interns an *empty* X
atom instead of `_NET_WM_STATE_FULLSCREEN` when asked to go back to windowed, and always
sends the EWMH `_NET_WM_STATE` ClientMessage with the "ADD" action (`data[0] = 1`)
instead of "REMOVE" (`0`) — the function's own TODO comment admits going back to
windowed doesn't really work. It also unconditionally unmaps/remaps the window on
*every* toggle (both directions), which is almost certainly why a toggle can also cost
the window input focus — symptoms reported: `F`/double-click sometimes fails to leave
fullscreen (or flickers back into it), and keys stop being delivered afterward until the
window is manually refocused or the same key is pressed twice. Wayland's backend
(`native/linux_wayland.rs`) uses the real `xdg_toplevel` `set_fullscreen`/
`unset_fullscreen` protocol requests and doesn't have this problem — this is X11
(including XWayland) specifically. Decided not to patch/vendor `miniquad` over this (real
fix would mean forking the crate); `FULLSCREEN_TOGGLE_COOLDOWN_SECS` debounce (mirrored
in `bundle/src/shell.rs`'s menu) softens accidental double-toggles but doesn't touch the
underlying bug. Revisit if `miniquad` ever fixes this upstream, or if the
vendor-a-patched-copy option becomes worth it later.

## Analytics

`control.episode_complete(game_name, score)` — call at the point each game resets for a
new round. Fires `gtag('event', 'episode_complete', {game, episode, score})` through
`xtask::analytics_bridge`'s `env.hcg_ga_event` miniquad plugin (not wasm-bindgen).

Other page-level analytics (`session_signals_bridge`, `wall_analytics_bridge`,
`share_result_bridge`) aren't `Control`'s concern — see root CLAUDE.md's "In-game
controls" section.
