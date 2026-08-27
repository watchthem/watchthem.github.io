use macroquad::prelude::*;

#[cfg(target_arch = "wasm32")]
unsafe extern "C" {
    /// `rgba_ptr`/`rgba_len` is raw, top-row-first RGBA8 pixel data (`width * height * 4`
    /// bytes, already flipped from `get_screen_data()`'s bottom-up GL readback order —
    /// see `handle_hotkey`) ready to hand straight to `new ImageData(...)`. No encoding
    /// done in Rust at all — the JS side (`xtask::screenshot_bridge`) builds a 2D canvas
    /// from it and does the PNG encoding + download there, so this crate stays
    /// dependency-free.
    fn hcg_save_screenshot(
        rgba_ptr: *const u8,
        rgba_len: u32,
        width: u32,
        height: u32,
        name_ptr: *const u8,
        name_len: u32,
    );
}

/// True when this run is capturing a screenshot or clip (`HCG_SCREENSHOT`/
/// `HCG_CLIP_FIFO` set — same detection `Capture::from_env` uses). For anything a game
/// wants to skip specifically *because* it's non-deterministic or time-consuming during
/// an automated capture — e.g. a game's own intro jingle/opening-screen delay, which
/// would otherwise push every capture's "after a few seconds" frame earlier into a
/// still-not-started game, or just waste the fixed capture window sitting on a title
/// screen instead of showing actual gameplay.
pub fn is_capturing() -> bool {
    std::env::var("HCG_SCREENSHOT").is_ok() || stream_mode_from_env().is_some()
}

/// RNG seed: `HCG_SEED` env override for reproducible screenshots, else wall-clock.
/// `std::time::SystemTime::now()` panics on WASM, so this always goes through miniquad's clock.
pub fn seed() -> u64 {
    std::env::var("HCG_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| macroquad::miniquad::date::now() as u64)
}

enum Mode {
    None,
    /// Single PNG after `after_secs`, then exit.
    Single {
        after_secs: f64,
        path: String,
    },
    /// Y4M frames at `fps` written to a named pipe (`HCG_CLIP_FIFO`) until `total_secs`
    /// elapsed, then exit — `mise run clip` has an `ffmpeg` already reading the other end
    /// of that pipe, encoding straight to `dist/<game>/clip.mp4`. No intermediate files on
    /// this side: `file` opens lazily on the first captured frame (not in `from_env`),
    /// since the Y4M stream header needs the real captured width/height, which is only
    /// known once `get_screen_data()` has been called at least once.
    #[cfg(feature = "stream")]
    Stream {
        fifo_path: String,
        file: Option<std::fs::File>,
        fps: f64,
        total_secs: f64,
        next_index: u32,
    },
}

/// Saves screenshot(s) after a few seconds of (real, wall-clock) play and exits the
/// process, when `HCG_SCREENSHOT` (single PNG) or, with the `stream` feature enabled,
/// `HCG_CLIP_FIFO` (Y4M frame stream) is set. No-op (near-zero cost) when neither is set,
/// so it's safe to leave wired into every game's loop.
///
/// Triggers on elapsed wall-clock time rather than frame count: headless/software-rendered
/// runs are unthrottled and can blow through hundreds of frames in milliseconds, which would
/// capture the game barely past its initial state instead of a representative mid-play frame.
pub struct Capture {
    start: f64,
    mode: Mode,
}

impl Capture {
    pub fn from_env() -> Self {
        let mode = if let Ok(path) = std::env::var("HCG_SCREENSHOT") {
            let after_secs = std::env::var("HCG_SCREENSHOT_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3.0);
            Mode::Single { after_secs, path }
        } else if let Some(mode) = stream_mode_from_env() {
            mode
        } else {
            Mode::None
        };
        Self {
            start: macroquad::miniquad::date::now(),
            mode,
        }
    }

    /// Call once per frame, after drawing, before `next_frame().await`.
    pub fn tick(&mut self) {
        let elapsed = macroquad::miniquad::date::now() - self.start;
        match &mut self.mode {
            Mode::None => {}
            Mode::Single { after_secs, path } => {
                if elapsed >= *after_secs {
                    get_screen_data().export_png(path);
                    std::process::exit(0);
                }
            }
            #[cfg(feature = "stream")]
            Mode::Stream {
                fifo_path,
                file,
                fps,
                total_secs,
                next_index,
            } => {
                if elapsed >= *total_secs {
                    // Drop the write end so the reader (ffmpeg) sees EOF and finishes
                    // muxing on its own — this process doesn't own or wait on it.
                    *file = None;
                    std::process::exit(0);
                }
                // Capture whenever wall-clock has caught up to the next frame's slot,
                // rather than every tick — headless runs render far faster than `fps`.
                if elapsed < *next_index as f64 / *fps {
                    return;
                }
                let img = get_screen_data();
                let f = file.get_or_insert_with(|| {
                    // Opening a FIFO for writing blocks until a reader attaches — `mise
                    // run clip` starts ffmpeg reading it before launching the game, so
                    // this returns immediately in practice.
                    let mut f = std::fs::OpenOptions::new()
                        .write(true)
                        .open(&*fifo_path)
                        .expect("open HCG_CLIP_FIFO — start the reader before the game");
                    use std::io::Write;
                    writeln!(
                        f,
                        "YUV4MPEG2 W{} H{} F{}:1 Ip A1:1 C444",
                        img.width, img.height, *fps as u32
                    )
                    .expect("write Y4M header");
                    f
                });
                write_y4m_frame(f, &img);
                *next_index += 1;
            }
        }
    }
}

#[cfg(feature = "stream")]
fn stream_mode_from_env() -> Option<Mode> {
    let fifo_path = std::env::var("HCG_CLIP_FIFO").ok()?;
    let fps = std::env::var("HCG_CLIP_FPS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(10.0);
    let total_secs = std::env::var("HCG_CLIP_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(15.0);
    Some(Mode::Stream {
        fifo_path,
        file: None,
        fps,
        total_secs,
        next_index: 0,
    })
}

#[cfg(not(feature = "stream"))]
fn stream_mode_from_env() -> Option<Mode> {
    None
}

/// Encodes one captured frame as a Y4M `FRAME` (BT.601 full-range RGB→YCbCr, `C444` — no
/// chroma subsampling, so no separate downsample pass) and writes it straight to `file`.
/// Folds in the same bottom-row-first→top-row-first flip `export_png`/the WASM hotkey
/// path already need (`get_screen_data()` reads in OpenGL's native bottom-up order),
/// rather than a separate pass over the pixels.
#[cfg(feature = "stream")]
fn write_y4m_frame(file: &mut std::fs::File, img: &Image) {
    use std::io::Write;
    let (w, h) = (img.width as usize, img.height as usize);
    let mut y_plane = vec![0u8; w * h];
    let mut u_plane = vec![0u8; w * h];
    let mut v_plane = vec![0u8; w * h];
    for row in 0..h {
        let src_row = h - 1 - row;
        for col in 0..w {
            let si = (src_row * w + col) * 4;
            let di = row * w + col;
            let r = img.bytes[si] as f32;
            let g = img.bytes[si + 1] as f32;
            let b = img.bytes[si + 2] as f32;
            let clamp = |v: f32| v.round().clamp(0.0, 255.0) as u8;
            y_plane[di] = clamp(0.299 * r + 0.587 * g + 0.114 * b);
            u_plane[di] = clamp(-0.168736 * r - 0.331264 * g + 0.5 * b + 128.0);
            v_plane[di] = clamp(0.5 * r - 0.418688 * g - 0.081312 * b + 128.0);
        }
    }
    file.write_all(b"FRAME\n").expect("write Y4M frame marker");
    file.write_all(&y_plane).expect("write Y4M Y plane");
    file.write_all(&u_plane).expect("write Y4M U plane");
    file.write_all(&v_plane).expect("write Y4M V plane");
}

/// `S` hotkey: save a screenshot of the current frame. Native writes a timestamped PNG
/// straight to the working directory via `std::fs`.
///
/// WASM has no filesystem, so it hands raw pixel bytes to a JS plugin
/// (`xtask::screenshot_bridge`) that encodes and downloads them instead — reading pixels
/// via `get_screen_data()` here (synchronously, inside this same Rust frame) rather than
/// the page listening for `S` itself and reading the canvas with `canvas.toBlob()`, which
/// this replaced. That approach depended on `preserveDrawingBuffer: true` on the WebGL
/// context, which `mq_js_bundle.js` (fetched, not ours to patch — see root CLAUDE.md's
/// "Site generation" section) doesn't set; without it the browser is free to discard the
/// drawing buffer as soon as it composites a frame, and by the time an async
/// `keydown`-triggered `toBlob()` callback ran, the buffer was reliably already gone —
/// every capture came back fully transparent/black (reproduced both mid-play and with
/// the game paused, so it wasn't about content still changing). `get_screen_data()`
/// reads pixels before any browser-side compositing/clearing can happen, sidestepping
/// the timing issue entirely — the same mechanism `Capture` (this file, used by the
/// build's own screenshot step) already relied on, just never wired to the live hotkey
/// on WASM before.
pub fn handle_hotkey() {
    if !is_key_pressed(KeyCode::S) {
        return;
    }
    let name = format!("screenshot-{}", macroquad::miniquad::date::now() as u64);
    #[cfg(not(target_arch = "wasm32"))]
    {
        let filename = format!("{name}.png");
        get_screen_data().export_png(&filename);
        println!("Saved screenshot to {filename}");
    }
    #[cfg(target_arch = "wasm32")]
    {
        let img = get_screen_data();
        let (w, h) = (img.width as usize, img.height as usize);
        // `get_screen_data()` reads bottom-row-first (OpenGL's native order) — flip to
        // top-row-first, same as `Image::export_png` does before writing a file, so the
        // JS side can hand this straight to `new ImageData(...)` without its own flip.
        let mut flipped = vec![0u8; img.bytes.len()];
        let row_bytes = w * 4;
        for y in 0..h {
            let src = (h - y - 1) * row_bytes;
            let dst = y * row_bytes;
            flipped[dst..dst + row_bytes].copy_from_slice(&img.bytes[src..src + row_bytes]);
        }
        unsafe {
            hcg_save_screenshot(
                flipped.as_ptr(),
                flipped.len() as u32,
                w as u32,
                h as u32,
                name.as_ptr(),
                name.len() as u32,
            );
        }
    }
}
