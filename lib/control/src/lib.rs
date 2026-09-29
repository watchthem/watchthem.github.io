use macroquad::prelude::*;

const STEP: f32 = 1.1;
const MIN_MULT: f32 = 0.1;
const MAX_MULT: f32 = 10.0;
#[cfg(not(target_arch = "wasm32"))]
const DOUBLE_CLICK_SECS: f64 = 0.4;
/// Some window managers/backends refocus the window as part of a fullscreen
/// transition and replay whatever key was still physically down at that moment as a
/// fresh `KeyDown` — which reads to `is_key_pressed` as a second, unintended toggle
/// landing a frame or two after the first, flickering back to the state the player
/// just left. Any toggle within this window of the last one is ignored rather than
/// applied, regardless of which trigger (`F` or double-click) fired it.
#[cfg(not(target_arch = "wasm32"))]
const FULLSCREEN_TOGGLE_COOLDOWN_SECS: f64 = 0.3;
/// Horizontal drag distance (px) per `STEP` multiplier change during a two-finger slide.
const TWO_FINGER_PX_PER_STEP: f32 = 80.0;
/// Minimum straight-line distance (px) for a one-finger touch to count as a swipe.
const SWIPE_MIN_DIST: f32 = 60.0;
/// Swipes slower than this (start to release) are treated as a drag/tap, not a swipe.
const SWIPE_MAX_SECS: f64 = 0.6;

/// Set once by `bundle/src/shell.rs` before any game's `play_until_exit()` runs — this
/// is the *only* place Esc genuinely returns to a menu, so it's the only place that
/// should say/do "Menu". A per-game standalone binary's `start()` never calls
/// `mark_in_shell`, so its `Control` correctly reports no shell: Esc there just quits
/// the process (see `start()`'s own doc comment in any game's `lib.rs`), and saying
/// "Esc Menu" in that hint would be actively misleading. A single process-wide flag
/// rather than threading a parameter through every game's `amain`/`start`/
/// `play_until_exit` (all ~11 of them, identical signatures) works because which
/// binary a process *is* never changes mid-run — `hcg --game <name>` still goes
/// through the shell's `play_until_exit`, same as landing on the menu first would.
#[cfg(not(target_arch = "wasm32"))]
static IN_SHELL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Call once, before running any game, from the native standalone shell only (see
/// `IN_SHELL`). Not `pub(crate)` — `bundle/src/shell.rs` is the one external caller.
#[cfg(not(target_arch = "wasm32"))]
pub fn mark_in_shell() {
    IN_SHELL.store(true, std::sync::atomic::Ordering::Relaxed);
}

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    fn hcg_ga_event(name_ptr: *const u8, name_len: u32, params_ptr: *const u8, params_len: u32);
    fn hcg_is_stream_mode() -> i32;
    fn hcg_is_daily_mode() -> i32;
    fn hcg_offer_share(text_ptr: *const u8, text_len: u32);
    /// Live (not read-once) — polled every `scale()` call. Backs the page-level `?`
    /// hotkey popup (`xtask::popup_pause_bridge`), the WASM half of "popups pause the
    /// game" — the native half is `Control::popup_open`, read directly since it's a
    /// Rust-side bool there instead of a DOM class to query.
    fn hcg_is_popup_open() -> i32;
    /// Suspends/resumes the browser's `AudioContext` (see `xtask::audio_bridge`
    /// and the capture script it depends on) so muting genuinely releases the audio
    /// hardware — e.g. a connected Bluetooth headset — rather than just silencing
    /// playback while still holding the device open. `muted` is `0`/`1`.
    fn hcg_set_audio_muted(muted: i32);
}

/// Fires a Google Analytics event (`gtag('event', name, params)`) via the small JS plugin
/// the page registers before the wasm module loads (see `xtask::analytics_bridge`).
/// `params_json` is a JSON object literal body, e.g. `{"episode":3,"score":140}`.
/// No-op on native, and harmless (silently dropped by the bridge) when `GTAG_ID` is unset.
fn ga_event(name: &str, params_json: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        hcg_ga_event(
            name.as_ptr(),
            name.len() as u32,
            params_json.as_ptr(),
            params_json.len() as u32,
        );
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (name, params_json);
    }
}

/// Reveals the page's hidden "Share Result" button with `text` pre-filled (see
/// `xtask::share_result_bridge`) — call once, right when a daily-challenge run ends.
/// Doesn't share directly: `navigator.share()` requires a real user gesture (a click) to
/// fire, which a call from inside the wasm frame loop isn't, so the JS side stores `text`
/// and waits for the button click instead. No-op on native (no page to show a button on).
pub fn share_result(text: &str) {
    #[cfg(target_arch = "wasm32")]
    unsafe {
        hcg_offer_share(text.as_ptr(), text.len() as u32);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = text;
    }
}

/// Whole UTC days since the Unix epoch, per `macroquad::miniquad::date::now()` (real
/// wall-clock time on both native and WASM — `std::time::SystemTime` panics on WASM, same
/// reason `Control::seed`'s own wall-clock fallback goes through the miniquad clock too).
fn today_day_index() -> u64 {
    macroquad::miniquad::date::now() as u64 / 86_400
}

/// Stable, ever-increasing puzzle number for a daily-challenge share (see
/// `daily_verdict_text`) — independent of `Control::seed`'s hash, just a human-readable
/// day count since a fixed epoch (2024-01-01T00:00:00Z).
pub fn daily_puzzle_number() -> u64 {
    const EPOCH_DAY: u64 = 19_723; // 2024-01-01 in days-since-epoch
    today_day_index().saturating_sub(EPOCH_DAY)
}

/// Wording for a daily-challenge share (see `share_result`) — one shared template so
/// every game's daily result reads in the same voice instead of being hand-written per
/// game. Deliberately not framed as "your score": there's no player input anywhere in
/// this workspace, so every visitor watching today's run sees the identical outcome —
/// there's nothing personal to have achieved. First-person "I watched" keeps it readable
/// as something a visitor is actually saying when they share it. `result_clause` is the
/// game-specific bit, completing "I watched {title} ___ today" (e.g. `"score 40"`,
/// `"solve it in 87 ticks"`).
pub fn daily_verdict_text(title: &str, puzzle_number: u64, result_clause: &str) -> String {
    format!("Case #{puzzle_number}: I watched {title} {result_clause} today.")
}

/// Simulation-speed multiplier, adjustable via hotkeys (`=`/`-` step by 10%, `0` resets
/// to 1x, `Space` pauses; on native, `F`/double-click also toggles fullscreen — on WASM
/// that's `xtask::fullscreen_bridge` instead, see its doc comment for why), plus
/// episode-progress tracking that reports a `episode_complete` GA event each time a game
/// round ends. Touch equivalents for the two hotkeys that matter on a spectator page with
/// no keyboard: a two-finger slide scrubs speed the same way `=`/`-` do, and a one-finger
/// swipe (`variant_swipe()`) stands in for `V`'s variant cycle on the games that have one.
pub struct Control {
    mult: f32,
    paused: bool,
    episode: u64,
    #[cfg(not(target_arch = "wasm32"))]
    fullscreen: bool,
    /// Held only while `fullscreen` is true — mirrors the WASM build's Screen Wake Lock
    /// (`xtask::fullscreen_bridge`), same trigger. Released by drop when set back to
    /// `None`, so no explicit "release" call is needed on the way out of fullscreen.
    #[cfg(not(target_arch = "wasm32"))]
    wake_lock: Option<keepawake::KeepAwake>,
    #[cfg(not(target_arch = "wasm32"))]
    last_click: f64,
    /// See `FULLSCREEN_TOGGLE_COOLDOWN_SECS`.
    #[cfg(not(target_arch = "wasm32"))]
    last_fullscreen_toggle: f64,
    #[cfg(not(target_arch = "wasm32"))]
    popup_open: bool,
    /// Set by `handle_keys()`, read (not recomputed) by `exit_requested()` — see that
    /// method's doc comment for why the two are split like this instead of
    /// `exit_requested()` reading raw input itself.
    #[cfg(not(target_arch = "wasm32"))]
    pending_exit: Option<ExitReason>,
    /// Snapshotted from `IN_SHELL` at construction — whether Esc has an actual menu to
    /// return to. See `IN_SHELL`'s own doc comment.
    #[cfg(not(target_arch = "wasm32"))]
    in_shell: bool,
    /// (avg x of the two touches, `mult` at gesture start) while a two-finger drag is live.
    two_finger_anchor: Option<(f32, f32)>,
    /// (x, y, start time) of an in-progress single-finger touch.
    one_finger_start: Option<(f32, f32, f64)>,
    variant_swipe: bool,
    stream_mode: bool,
    daily_mode: bool,
    /// Seed actually driving the RNG right now — set once by `seed()` at startup, then
    /// again on every manual reseed. Shown (read-only, live) in the `R` editor panel.
    current_seed: u64,
    /// True while the `R` panel is open. Cross-platform (not `#[cfg]`-gated, unlike
    /// `popup_open`/`pending_exit`) since a manual reseed is a legitimate thing to want
    /// on a browser tab too, not just the native standalone shell.
    seed_editing: bool,
    /// Digits typed so far in the open editor — prefilled from `current_seed` when `R`
    /// opens it, so Enter alone with no edits is a no-op reseed rather than a blank field.
    seed_input: String,
    /// Set for one frame by `Enter` in the editor; a game's loop takes it, reseeds the
    /// global RNG, and restarts its current episode — see `take_reseed()`.
    pending_reseed: Option<u64>,
    /// `M` toggles this — see `handle_keys`. Mirrored into `audio::playback`'s global
    /// mute gate (every `Clip` checks it) on every toggle, plus, on WASM, into
    /// `hcg_set_audio_muted` to suspend/resume the real audio hardware.
    muted: bool,
}

/// Why a game's `play_until_exit()` loop returned control to the standalone shell (see
/// `.notes/steam-standalone-menu-handoff.md`) — `Menu` lands back on the game grid,
/// `Quit` ends the process. Not `#[cfg]`-gated to WASM even though only the native shell
/// produces it: a per-game `play_until_exit()` needs one return type usable from both
/// `#[cfg(...)]` arms of its own signature without the type itself vanishing on WASM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitReason {
    Menu,
    Quit,
}

impl Control {
    pub fn new() -> Self {
        #[cfg(not(target_arch = "wasm32"))]
        prevent_quit();
        Self {
            mult: 1.0,
            paused: false,
            episode: 0,
            #[cfg(not(target_arch = "wasm32"))]
            fullscreen: false,
            #[cfg(not(target_arch = "wasm32"))]
            wake_lock: None,
            #[cfg(not(target_arch = "wasm32"))]
            last_click: f64::NEG_INFINITY,
            #[cfg(not(target_arch = "wasm32"))]
            last_fullscreen_toggle: f64::NEG_INFINITY,
            #[cfg(not(target_arch = "wasm32"))]
            popup_open: false,
            #[cfg(not(target_arch = "wasm32"))]
            pending_exit: None,
            #[cfg(not(target_arch = "wasm32"))]
            in_shell: IN_SHELL.load(std::sync::atomic::Ordering::Relaxed),
            two_finger_anchor: None,
            one_finger_start: None,
            variant_swipe: false,
            #[cfg(target_arch = "wasm32")]
            stream_mode: unsafe { hcg_is_stream_mode() != 0 },
            #[cfg(not(target_arch = "wasm32"))]
            stream_mode: false,
            #[cfg(target_arch = "wasm32")]
            daily_mode: unsafe { hcg_is_daily_mode() != 0 },
            #[cfg(not(target_arch = "wasm32"))]
            daily_mode: false,
            current_seed: 0,
            seed_editing: false,
            seed_input: String::new(),
            pending_reseed: None,
            muted: false,
        }
    }

    /// True under the page's `?stream=1` query param (see
    /// `xtask::stream_mode_query_bridge`) — an OBS/Twitch browser-source layer has no
    /// visitor around to show score/speed HUD text to. Read once at startup (the query
    /// string doesn't change mid-session); always `false` natively, since there's no
    /// browser URL to read. Games should skip their own in-canvas HUD draw calls when
    /// this is true — `Control` only carries the flag, it doesn't touch rendering itself.
    pub fn stream_mode(&self) -> bool {
        self.stream_mode
    }

    /// True under the page's `?daily=1` query param (see `xtask::daily_mode_query_bridge`
    /// / `xtask::daily_challenge_button`) — a visitor asked for today's shared-seed board
    /// instead of a random one. Read once at startup, same as `stream_mode`. Games should
    /// seed their RNG with `control.seed()` instead of `screenshot::seed()`, and freeze on
    /// episode end (calling `share_result`) instead of starting a new episode, when this
    /// is true.
    pub fn daily_mode(&self) -> bool {
        self.daily_mode
    }

    /// RNG seed for this run. `HCG_SEED` (native testing override) wins if set — same
    /// precedence as `screenshot::seed()`. Otherwise, when `daily_mode()` is on, every
    /// visitor loading the page on the same UTC day gets the identical hash of
    /// `today_day_index()` (splitmix64 — cheap, well-distributed), so they all see the
    /// same board: the mechanic behind Wordle-style daily-return puzzles (see
    /// `.notes/aiideas.md`). Falls back to wall-clock for a normal, non-daily run — the
    /// same fallback `screenshot::seed()` uses, duplicated here (rather than adding a
    /// dependency on that crate for three lines) since seeding is a `Control`-level
    /// concern once daily mode is involved: `Control` is what owns `daily_mode` itself,
    /// so it should own the seed decision built on top of it rather than splitting that
    /// decision across two crates.
    pub fn seed(&mut self) -> u64 {
        let n = if let Some(n) = std::env::var("HCG_SEED").ok().and_then(|s| s.parse().ok()) {
            n
        } else if !self.daily_mode {
            macroquad::miniquad::date::now() as u64
        } else {
            let mut x = today_day_index().wrapping_add(0x9E37_79B9_7F4A_7C15);
            x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            x ^ (x >> 31)
        };
        self.current_seed = n;
        n
    }

    /// One-shot: `Some(seed)` the frame after `Enter` confirms the `R` editor, else
    /// `None`. A game's loop calls this once per frame (right after `exit_requested()`)
    /// and, when it fires, reseeds the global RNG and restarts its current episode from
    /// scratch — same shape as a fresh `amain` start, just triggered mid-run instead of
    /// at startup. `daily_mode` stays whatever it already was; a manual reseed is an
    /// explicit opt into a specific board, not a way to leave/enter the daily challenge.
    pub fn take_reseed(&mut self) -> Option<u64> {
        self.pending_reseed.take()
    }

    pub fn handle_keys(&mut self) {
        if is_key_pressed(KeyCode::Equal) || is_key_pressed(KeyCode::KpAdd) {
            self.mult = (self.mult * STEP).min(MAX_MULT);
        }
        if is_key_pressed(KeyCode::Minus) || is_key_pressed(KeyCode::KpSubtract) {
            self.mult = (self.mult / STEP).max(MIN_MULT);
        }
        if is_key_pressed(KeyCode::Key0) || is_key_pressed(KeyCode::Kp0) {
            self.mult = 1.0;
        }
        if is_key_pressed(KeyCode::Space) {
            self.paused = !self.paused;
        }

        // `M` mutes/unmutes — cross-platform toggle (unlike fullscreen, which is fully
        // page-level JS on WASM: muting has no external DOM state that can change
        // behind our back, so there's no need to duplicate the keydown handling in JS
        // the way fullscreen's `fullscreenchange` reactivity requires). The WASM-only
        // extra step (releasing the actual audio hardware) is layered on top, same
        // shape as fullscreen's native-only `wake_lock` extra step below.
        if is_key_pressed(KeyCode::M) {
            self.muted = !self.muted;
            audio::playback::set_muted(self.muted);
            #[cfg(target_arch = "wasm32")]
            unsafe {
                hcg_set_audio_muted(self.muted as i32);
            }
        }

        // Seed editor: `R` opens/closes it, prefilled from the seed actually driving
        // the RNG right now so a bare Enter is a no-op. Cross-platform (unlike the
        // native-only popup below) — `get_char_pressed` already works on WASM the same
        // way it does natively, no page-level JS bridge needed.
        if is_key_pressed(KeyCode::R) {
            if self.seed_editing {
                self.seed_editing = false;
            } else {
                self.seed_editing = true;
                self.seed_input = self.current_seed.to_string();
            }
        }
        // Captured before Escape can close the editor below, and reused by the
        // native-only Escape handler further down so a single Esc press that closes the
        // editor doesn't *also* close the popup / exit to menu the same frame.
        let seed_editor_was_open = self.seed_editing;
        if seed_editor_was_open {
            while let Some(c) = get_char_pressed() {
                if c.is_ascii_digit() && self.seed_input.len() < 20 {
                    self.seed_input.push(c);
                }
            }
            if is_key_pressed(KeyCode::Backspace) {
                self.seed_input.pop();
            }
            if is_key_pressed(KeyCode::Enter) || is_key_pressed(KeyCode::KpEnter) {
                if let Ok(n) = self.seed_input.parse::<u64>() {
                    self.current_seed = n;
                    self.pending_reseed = Some(n);
                }
                self.seed_editing = false;
            } else if is_key_pressed(KeyCode::Escape) {
                self.seed_editing = false;
            }
        }

        // WASM fullscreen is handled entirely by page-level JS (`xtask::fullscreen_bridge`)
        // instead: `macroquad::window::set_fullscreen` on WASM fullscreens the canvas
        // itself, which the browser then forces to `width/height: 100%` via an
        // unoverridable `!important` UA style, breaking the pinned-native-resolution
        // canvas this whole rendering setup depends on (see that function's doc comment).
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut toggle_fullscreen = is_key_pressed(KeyCode::F);
            if is_mouse_button_pressed(MouseButton::Left) {
                let now = get_time();
                if now - self.last_click < DOUBLE_CLICK_SECS {
                    toggle_fullscreen = true;
                    // Consumed, so a third click starts a fresh pair instead of re-firing.
                    self.last_click = f64::NEG_INFINITY;
                } else {
                    self.last_click = now;
                }
            }
            let now = get_time();
            if toggle_fullscreen
                && now - self.last_fullscreen_toggle > FULLSCREEN_TOGGLE_COOLDOWN_SECS
            {
                self.last_fullscreen_toggle = now;
                self.fullscreen = !self.fullscreen;
                set_fullscreen(self.fullscreen);
                self.wake_lock = self
                    .fullscreen
                    .then(|| {
                        keepawake::Builder::default()
                            .display(true)
                            .reason("hcg game running fullscreen")
                            .app_name("hcg")
                            .app_reverse_domain("io.github.hcg")
                            .create()
                            .ok()
                    })
                    .flatten();
            }

            // `get_char_pressed` (layout-aware text input, unlike `KeyCode` which is a
            // physical key) rather than checking `KeyCode::Slash` + shift by hand — same
            // reasoning the web build gets for free from `e.key === '?'`.
            while let Some(c) = get_char_pressed() {
                if c == '?' {
                    self.popup_open = !self.popup_open;
                }
            }

            // Computed here, once, rather than in `exit_requested()` itself: closing the
            // popup has to *consume* this frame's Esc press so it doesn't also fire an
            // exit-to-menu the same frame, and only `handle_keys` (called before
            // `exit_requested` every frame, see that method's doc comment) can both read
            // input and mutate `popup_open` in the same pass. Only ever *sets*
            // `pending_exit`, never resets it to `None` — see `draw_overlay`, which can
            // also set it (from a mouse click) after this method already ran this frame.
            if is_quit_requested() {
                self.pending_exit = Some(ExitReason::Quit);
            } else if is_key_pressed(KeyCode::Escape) && !seed_editor_was_open {
                if self.popup_open {
                    self.popup_open = false;
                } else {
                    // `Menu` only where there's an actual menu to land on — see
                    // `IN_SHELL`. A per-game standalone binary has none, so Esc there
                    // is `Quit`, matching what it's actually going to do
                    // (`start()`'s `process::exit(0)` treats either reason the same,
                    // but the reason itself should still describe reality).
                    self.pending_exit = Some(if self.in_shell {
                        ExitReason::Menu
                    } else {
                        ExitReason::Quit
                    });
                }
            }
        }

        self.handle_touch();
    }

    /// Two-finger horizontal slide scrubs `mult` (anchored to the multiplier at gesture
    /// start, so re-crossing the same drag distance always lands on the same speed); a
    /// quick one-finger swipe sets the one-frame `variant_swipe()` flag. Touch phases with
    /// a finger count other than 1 or 2 (none, or someone's third finger) reset both
    /// gestures so a new drag/swipe starts clean.
    fn handle_touch(&mut self) {
        self.variant_swipe = false;
        let touches = touches();

        match touches.len() {
            2 => {
                self.one_finger_start = None;
                let avg_x = (touches[0].position.x + touches[1].position.x) / 2.0;
                match self.two_finger_anchor {
                    Some((start_x, start_mult)) => {
                        let steps = (avg_x - start_x) / TWO_FINGER_PX_PER_STEP;
                        self.mult = (start_mult * STEP.powf(steps)).clamp(MIN_MULT, MAX_MULT);
                    }
                    None => self.two_finger_anchor = Some((avg_x, self.mult)),
                }
            }
            1 => {
                self.two_finger_anchor = None;
                let touch = &touches[0];
                match touch.phase {
                    TouchPhase::Started => {
                        self.one_finger_start =
                            Some((touch.position.x, touch.position.y, get_time()));
                    }
                    TouchPhase::Ended | TouchPhase::Cancelled => {
                        if let Some((sx, sy, start_time)) = self.one_finger_start.take() {
                            let dx = touch.position.x - sx;
                            let dy = touch.position.y - sy;
                            let dist = (dx * dx + dy * dy).sqrt();
                            if dist >= SWIPE_MIN_DIST && get_time() - start_time <= SWIPE_MAX_SECS {
                                self.variant_swipe = true;
                            }
                        }
                    }
                    _ => {}
                }
            }
            _ => {
                self.two_finger_anchor = None;
                self.one_finger_start = None;
            }
        }
    }

    /// True for one frame when a one-finger swipe just completed — the touch equivalent
    /// of the `V` variant-cycle hotkey. Games without a variant cycle can ignore it.
    pub fn variant_swipe(&self) -> bool {
        self.variant_swipe
    }

    /// Native-only: `Some(Menu)` when the player pressed Esc (and the hotkey popup wasn't
    /// open — Esc closes that first) or clicked `draw_overlay`'s corner hint, *and* this
    /// `Control` was constructed under the shell (`IN_SHELL`/`mark_in_shell`) — that's
    /// the only context with an actual menu to return to. The exact same Esc/click
    /// outside the shell (a per-game standalone binary) instead reports `Some(Quit)`,
    /// matching what's actually about to happen (`start()`'s `process::exit(0)`, no
    /// menu to land on). `Some(Quit)` also fires when the window is closed (only
    /// because `Control::new` calls `prevent_quit()` — without that, a close click
    /// kills the process before this is ever read), else `None`. Always `None` on
    /// WASM — a browser tab has nothing to "return to", and Esc there is page-level JS
    /// for the hotkey popup (`xtask::hotkey_popup`), not a Rust binding. A game's
    /// `play_until_exit()` checks this once per frame, right after `handle_keys()`
    /// (which is what actually computes the value returned here — see its doc
    /// comment); `play()` (the browser entry point) never calls it.
    pub fn exit_requested(&self) -> Option<ExitReason> {
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.pending_exit
        }
        #[cfg(target_arch = "wasm32")]
        None
    }

    /// The hint/popup half is a no-op on WASM (the browser's own hotkey popup is
    /// page-level HTML/JS, `xtask::hotkey_popup` — this would be a second, redundant one
    /// drawn over the canvas) — an unconditional signature so every game can call it
    /// every frame without its own `#[cfg]`, same reasoning as `exit_requested`. The
    /// seed editor at the bottom (`draw_seed_editor`) is the exception: it draws on both
    /// platforms, since `R` itself is a cross-platform hotkey (see `handle_keys`).
    /// Natively, draws two things, always in this order so the popup paints over the hint: a small always-visible
    /// "Esc" hint in the bottom-right corner (chosen to avoid the top-left corner every
    /// game's own score/generation HUD text uses — says "Menu" or "Quit" depending on
    /// `in_shell`, see `IN_SHELL`), clickable as a mouse-first equivalent of pressing
    /// Esc; and, when `?` has toggled `popup_open` on, a full
    /// hotkey-reference panel mirroring the web build's own `?`-key popup. Call once per
    /// frame, after a game's own drawing is done (typically right before
    /// `next_frame().await`) — safe to call there specifically because nothing here draws
    /// for the first time inside an active `RenderCache` render-target camera (see root
    /// CLAUDE.md's "Font atlas gotcha"), only on the default camera after a frame's cache
    /// draws are already done.
    pub fn draw_overlay(&mut self) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            let hint = if self.in_shell {
                "Esc  Menu    ?  Help"
            } else {
                "Esc  Quit    ?  Help"
            };
            let fs = 16.0;
            let pad = 8.0;
            let td = measure_text(hint, None, fs as u16, 1.0);
            let w = td.width + pad * 2.0;
            let h = td.height + pad * 2.0;
            let x = screen_width() - w - 10.0;
            let y = screen_height() - h - 10.0;
            draw_rectangle(x, y, w, h, Color::new(0.0, 0.0, 0.0, 0.45));
            draw_text(
                hint,
                x + pad,
                y + h - pad - 2.0,
                fs,
                Color::new(0.85, 0.85, 0.9, 1.0),
            );
            if !self.popup_open
                && is_mouse_button_pressed(MouseButton::Left)
                && Rect::new(x, y, w, h).contains(vec2(mouse_position().0, mouse_position().1))
            {
                self.pending_exit = Some(if self.in_shell {
                    ExitReason::Menu
                } else {
                    ExitReason::Quit
                });
            }

            if self.popup_open {
                self.draw_popup();
            }
        }
        self.draw_seed_editor();
    }

    /// Centered panel shown while `seed_editing` is true (`R` to open, digits to edit,
    /// Enter to apply, Esc to cancel — see `handle_keys`). Cross-platform, unlike
    /// `draw_popup`: the seed itself is a cross-platform concept (`Control::seed`'s
    /// `HCG_SEED`/daily-hash/wall-clock precedence applies identically on native and
    /// WASM), so there's no page-level JS equivalent to defer to here.
    fn draw_seed_editor(&self) {
        if !self.seed_editing {
            return;
        }
        let label = format!("Seed: {}", self.seed_input);
        let hint = "Enter to apply  \u{b7}  Esc to cancel  \u{b7}  digits only";
        let fs = 28.0;
        let hint_fs = 16.0;
        let pad = 16.0;
        let label_w = measure_text(&label, None, fs as u16, 1.0).width;
        let hint_w = measure_text(hint, None, hint_fs as u16, 1.0).width;
        let w = label_w.max(hint_w) + pad * 2.0;
        let h = fs + hint_fs + pad * 2.5;
        let x = ((screen_width() - w) * 0.5).floor();
        let y = ((screen_height() - h) * 0.5).floor();
        draw_rectangle(x, y, w, h, Color::new(0.1, 0.1, 0.15, 0.95));
        draw_rectangle_lines(x, y, w, h, 2.0, Color::new(0.4, 0.75, 1.0, 1.0));
        draw_text(&label, x + pad, y + pad + fs * 0.7, fs, WHITE);
        draw_text(
            hint,
            x + pad,
            y + h - pad * 0.5,
            hint_fs,
            Color::new(0.7, 0.7, 0.8, 1.0),
        );
    }

    /// The panel half of `draw_overlay` — split out only for readability, not meant to be
    /// called on its own (it doesn't check `popup_open`).
    #[cfg(not(target_arch = "wasm32"))]
    fn draw_popup(&self) {
        // Not `const` (unlike every other line, which never varies): the `Esc` entry's
        // description depends on `in_shell` — "back to menu" is only true under the
        // shell, see `IN_SHELL`.
        let esc_desc = if self.in_shell {
            "back to menu (or close this help)"
        } else {
            "quit (or close this help)"
        };
        let lines: [(&str, &str); 11] = [
            ("=", "speed up"),
            ("-", "slow down"),
            ("0", "reset speed"),
            ("Space", "pause / resume"),
            ("M", "mute / unmute"),
            ("F", "toggle fullscreen (or double-click)"),
            ("V", "switch game variant (games that have one)"),
            ("R", "show / type a seed to replay"),
            ("S", "save screenshot"),
            ("?", "toggle this help"),
            ("Esc", esc_desc),
        ];

        let sw = screen_width();
        let sh = screen_height();
        draw_rectangle(0.0, 0.0, sw, sh, Color::new(0.0, 0.0, 0.0, 0.6));

        let line_h = 30.0;
        let desc_x = 120.0;
        // Sized off the actual longest desc rather than a fixed guess — "toggle
        // fullscreen (or double-click)" was overflowing a hardcoded 440px panel.
        let max_desc_w = lines
            .iter()
            .map(|(_, desc)| measure_text(desc, None, 20, 1.0).width)
            .fold(0.0f32, f32::max);
        let panel_w = desc_x + max_desc_w + 20.0;
        let panel_h = 70.0 + lines.len() as f32 * line_h;
        let px = (sw - panel_w) * 0.5;
        let py = (sh - panel_h) * 0.5;
        draw_rectangle(px, py, panel_w, panel_h, Color::new(0.1, 0.1, 0.15, 0.97));
        draw_rectangle_lines(
            px,
            py,
            panel_w,
            panel_h,
            2.0,
            Color::new(0.4, 0.75, 1.0, 1.0),
        );
        draw_text("Hotkeys", px + 20.0, py + 38.0, 26.0, WHITE);
        for (i, (key, desc)) in lines.iter().enumerate() {
            let ly = py + 70.0 + i as f32 * line_h;
            draw_text(key, px + 20.0, ly, 20.0, Color::new(0.5, 0.8, 1.0, 1.0));
            draw_text(
                desc,
                px + desc_x,
                ly,
                20.0,
                Color::new(0.85, 0.85, 0.9, 1.0),
            );
        }
    }

    /// Zero while paused, otherwise `dt` scaled by the speed multiplier.
    /// Every game funnels its per-frame `dt` through this single gate, so pausing a
    /// popup is exactly "act paused for as long as it's open" with no per-game code —
    /// same mechanism `Space` already uses, just driven by popup state instead of the
    /// `paused` flag. Covers the seed editor (cross-platform, a `Control` field) and
    /// the native hotkey popup (`popup_open`, also a `Control` field) directly; the
    /// page-level `?` hotkey popup on WASM has no Rust-side field (it's a DOM class
    /// toggled by page JS), so that one's checked live via `hcg_is_popup_open()` — see
    /// `xtask::popup_pause_bridge`.
    pub fn scale(&self, dt: f32) -> f32 {
        if self.paused || self.seed_editing {
            return 0.0;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.popup_open {
            return 0.0;
        }
        #[cfg(target_arch = "wasm32")]
        if unsafe { hcg_is_popup_open() != 0 } {
            return 0.0;
        }
        dt * self.mult
    }

    /// `x1.000`-style label for the in-canvas HUD, or `PAUSED` when paused, with a
    /// `MUTED` suffix appended whenever `muted()` is true — the same in-canvas text
    /// every game already draws for speed/pause is the natural place for mute state
    /// too, no separate UI element needed.
    pub fn label(&self) -> String {
        let base = if self.paused {
            "PAUSED".to_owned()
        } else {
            format!("x{:.3}", self.mult)
        };
        if self.muted {
            format!("{base}  MUTED")
        } else {
            base
        }
    }

    /// True after `M` has muted audio — see `handle_keys`. Every `audio::Clip` already
    /// gates itself on this (via `audio::playback::muted()`), so games don't need to
    /// check it before playing a sound; it's exposed mainly for a game's own HUD if it
    /// wants a mute indicator beyond what `label()` already shows.
    pub fn muted(&self) -> bool {
        self.muted
    }

    /// Call when a game round ends. Bumps the episode counter and reports it, with the
    /// round's final `score`, as a GA event. `daily` is tagged on from `self.daily_mode`
    /// so a completed daily-challenge run (vs. a regular random one) is filterable in GA
    /// without a second event — see `daily_mode` for why there's no separate
    /// "daily run started" event: `Control::new()` doesn't know the game's name yet, and
    /// `daily_challenge_button`'s own click already fires `daily_challenge_click` for the
    /// entering-daily-mode signal instead.
    pub fn episode_complete(&mut self, game: &str, score: i64) {
        self.episode += 1;
        ga_event(
            "episode_complete",
            &format!(
                "{{\"game\":\"{game}\",\"episode\":{},\"score\":{score},\"daily\":{}}}",
                self.episode, self.daily_mode
            ),
        );
    }
}

impl Default for Control {
    fn default() -> Self {
        Self::new()
    }
}
