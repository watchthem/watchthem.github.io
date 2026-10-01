use macroquad::prelude::*;
use render_cache::RenderCache;

mod game;
mod solver;
mod sound;

use game::{Bottle, CAPACITY, Game, Move, Phase};
use solver::Solver;

const TICK: f32 = 0.55;
/// A full pour now has three top-level beats (see the `pour` block in `run_ui`) —
/// lift off its grid slot, hover beside the destination and tip over it to pour, then
/// return — so it needs noticeably longer than a single tilt-in-place animation to
/// read clearly.
const ANIM_DURATION: f32 = 0.9;
/// Fraction of `ANIM_DURATION` spent lifting from the grid slot to the hover position,
/// and separately hovering-and-pouring once there. The remainder (`1.0 - LIFT_FRAC -
/// POUR_FRAC`) is the return trip back down to the grid slot.
const LIFT_FRAC: f32 = 0.30;
const POUR_FRAC: f32 = 0.40;
/// The hover-and-pour beat (`POUR_FRAC`) is itself three sub-beats: rise to peak tilt,
/// hold there while liquid actually transfers, then drip (tilt *still* pinned) while
/// the last droplets fall. Both fractions are of `POUR_FRAC`, not of the whole
/// animation; the remainder (`1.0 - POUR_RISE_FRAC - POUR_HOLD_FRAC`) is the drip.
/// The bottle only rights itself afterwards, during the return trip — so the tilt is
/// constant for the entire time any stream is on screen (see the `pour` block).
const POUR_RISE_FRAC: f32 = 0.28;
const POUR_HOLD_FRAC: f32 = 0.47;
/// `anim_t` at which the stream starts falling (end of the Rise sub-beat) — the pour's
/// glug starts here.
const STREAM_START: f32 = LIFT_FRAC + POUR_FRAC * POUR_RISE_FRAC;
const RESTART_DELAY: f64 = 2.4;
const HUD_H: f32 = 34.0;
/// Peak tip angle (radians) the source bottle rotates to while hovering beside the
/// destination — see the `pour` block's Rise/Hold sub-beats in `run_ui`. Has to be
/// past ~25° to work at all: below that the body's own shoulder corner still sticks
/// out further sideways than the mouth does (the shoulder's greater half-width beats
/// the mouth's greater lever arm), so the falling stream grazes the glass it's
/// supposedly leaving and reads as liquid dribbling down the *outside* of the bottle.
/// 70° also puts the mouth low enough — roughly level with the body's midpoint — that
/// liquid pooled at the (bottom-stacked, tilting with the bottle) base plausibly
/// reaches it; a shallower tip leaves the mouth as the bottle's highest point, so the
/// stream reads as flowing uphill out of a bottle that isn't really tipped over.
const TILT_MAX: f32 = 70.0 * std::f32::consts::PI / 180.0;
/// How far above the destination's mouth the pour lip is parked, as a fraction of
/// bottle height — i.e. the length of the visible falling stream. Applied to the
/// *lip* rather than the bottle's bottom, since tipping to `TILT_MAX` swings the lip
/// down a good fraction of `bh` (`lip_drop` in the `pour` block).
const POUR_HEIGHT_FRAC: f32 = 0.65;
/// Droplets rendered along the pour's stream curve at any one instant — see
/// `bezier` and the `STREAM_DROPLETS` loop in `run_ui`.
const STREAM_DROPLETS: u32 = 4;

/// Quadratic Bezier point at `t` (0..1) through control points `p0`/`p1`/`p2`.
fn bezier(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), t: f32) -> (f32, f32) {
    let mt = 1.0 - t;
    (
        mt * mt * p0.0 + 2.0 * mt * t * p1.0 + t * t * p2.0,
        mt * mt * p0.1 + 2.0 * mt * t * p1.1 + t * t * p2.1,
    )
}

fn lerp_pt(a: (f32, f32), b: (f32, f32), t: f32) -> (f32, f32) {
    (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
}

/// 12 hues chosen for pairwise contrast (matters more than usual — the whole puzzle is
/// "tell these apart at a glance"), indexed by `Color` (the game's `u8`, 0..MAX_COLORS).
///
/// **Fixed bug**: the original `lime`/`indigo` were both too close to a neighbor to
/// tell apart at a glance — reported directly for `lime` ("yellows look too much
/// alike"), and confirmed for both by running this workspace's `dataviz` skill's
/// palette validator (`validate_palette.js`) against the full 12-color set in
/// `--pairs all` mode (every pair checked, not just adjacent — any two colors can
/// end up stacked in the same bottle, so adjacency in this array isn't the same
/// adjacency a chart's fixed series order would give): `lime` (`#BFD926`) vs.
/// `yellow` (`#F2CC1A`) scored a normal-vision ΔE of 7.6 (OKLab, ×100 — the skill's
/// hard floor is 15, below which even full-color vision can't reliably tell a pair
/// apart), and `indigo` (`#5959D9`) vs. `purple` (`#9940D9`) scored a colorblind-
/// simulated ΔE of 1.1 (essentially identical under protan simulation). Replaced
/// both by hue (well clear of every neighbor) rather than just nudging saturation/
/// value, and re-validated: the worst remaining pair is `orange`/`red` at a
/// normal-vision ΔE of 12.5 and `brown`/`red` at a CVD ΔE of 1.8 — both *already*
/// present, unchanged, in the original palette (the validator only surfaces the
/// single worst pair per check, so they were silently failing there too, just
/// masked by the two worse failures this fixed). Left as-is: per the skill's own
/// reference palette doc, a full twelve-color set clearing every one of 66 pairs
/// under CVD simulation isn't achievable at all (it documents this ceiling at just
/// eight colors), so the realistic bar is "no worse than before," not "zero
/// failures" — chasing brown/red further would be a much larger redesign for a
/// pair nobody's reported trouble with.
const PALETTE: [macroquad::color::Color; 12] = [
    Color::new(0.90, 0.20, 0.20, 1.0), // red
    Color::new(0.15, 0.55, 0.95, 1.0), // blue
    Color::new(0.15, 0.75, 0.30, 1.0), // green
    Color::new(0.95, 0.80, 0.10, 1.0), // yellow
    Color::new(0.95, 0.45, 0.05, 1.0), // orange
    Color::new(0.60, 0.25, 0.85, 1.0), // purple
    Color::new(0.95, 0.35, 0.65, 1.0), // pink
    Color::new(0.10, 0.75, 0.75, 1.0), // teal
    Color::new(0.55, 0.35, 0.15, 1.0), // brown
    Color::new(0.27, 0.55, 0.14, 1.0), // moss green (was lime, too close to yellow)
    Color::new(0.24, 0.39, 0.60, 1.0), // steel blue (was indigo, too close to purple)
    Color::new(0.85, 0.85, 0.85, 1.0), // white/silver
];

fn liquid_color(c: game::Color) -> Color {
    PALETTE[c as usize % PALETTE.len()]
}

// ── Layout ────────────────────────────────────────────────────────────────────

struct Layout {
    pos: Vec<(f32, f32)>, // top-left of each bottle's body
    bw: f32,
    bh: f32,
}

impl Layout {
    fn compute(count: usize) -> Self {
        let sw = screen_width();
        let sh = screen_height() - HUD_H;
        let cols = count.clamp(1, 8);
        let rows = count.div_ceil(cols);

        let cell_w = sw / cols as f32;
        let cell_h = sh / rows as f32;
        let bw = (cell_w * 0.55).min(70.0);
        let bh = (bw * 2.6).min(cell_h * 0.82);

        let mut pos = Vec::with_capacity(count);
        for i in 0..count {
            let col = i % cols;
            let row = i / cols;
            let cx = cell_w * (col as f32 + 0.5);
            let cy = HUD_H + cell_h * (row as f32 + 0.5);
            pos.push((cx - bw / 2.0, cy - bh / 2.0));
        }
        Layout { pos, bw, bh }
    }

    fn top_center(&self, i: usize) -> (f32, f32) {
        let (x, y) = self.pos[i];
        (x + self.bw / 2.0, y)
    }
}

fn ease_in_out(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0_f32).powi(3) / 2.0
    }
}

// ── Rendering ─────────────────────────────────────────────────────────────────
//
// A bottle is neck (narrow rect) + shoulder (trapezoid, two triangles) + body (a
// rect with its bottom two corners rounded). Everything is expressed as explicit
// points rotated around a pivot (`rotate_pt`) rather than drawn axis-aligned and
// relying on a rectangle-rotation primitive — the only shape macroquad rotates
// natively is a plain rectangle about its own center, which can't express "tip this
// whole multi-part glyph around its base" the way a pour needs. `angle`/`pivot` are
// `0.0`/anything for a resting bottle — `rotate_pt` is the identity at `angle == 0.0`,
// so every draw call below is unconditional rather than branching on whether this
// bottle happens to be mid-pour.

const NECK_H_FRAC: f32 = 0.14;
const SHOULDER_H_FRAC: f32 = 0.08;
const NECK_W_FRAC: f32 = 0.46;

fn rotate_pt(p: (f32, f32), pivot: (f32, f32), angle: f32) -> Vec2 {
    if angle == 0.0 {
        return vec2(p.0, p.1);
    }
    let (s, c) = angle.sin_cos();
    let (dx, dy) = (p.0 - pivot.0, p.1 - pivot.1);
    vec2(pivot.0 + dx * c - dy * s, pivot.1 + dx * s + dy * c)
}

fn rect_rotated(x: f32, y: f32, w: f32, h: f32, pivot: (f32, f32), angle: f32, color: Color) {
    let p00 = rotate_pt((x, y), pivot, angle);
    let p10 = rotate_pt((x + w, y), pivot, angle);
    let p11 = rotate_pt((x + w, y + h), pivot, angle);
    let p01 = rotate_pt((x, y + h), pivot, angle);
    draw_triangle(p00, p10, p11, color);
    draw_triangle(p00, p11, p01, color);
}

fn line_rotated(
    p0: (f32, f32),
    p1: (f32, f32),
    thickness: f32,
    pivot: (f32, f32),
    angle: f32,
    color: Color,
) {
    let a = rotate_pt(p0, pivot, angle);
    let b = rotate_pt(p1, pivot, angle);
    draw_line(a.x, a.y, b.x, b.y, thickness, color);
}

/// A quarter-circle outline stroke from local (pre-tilt) angle `a0` to `a1` around
/// `center`, each sampled point going through the same `rotate_pt` every other vertex
/// in a bottle does. `rounded_bottom_rect`/`rounded_top_rect` only round the *fill* —
/// a plain circle there is rotation-invariant so filling in a corner doesn't need this
/// — but the outline is a stroke along a specific arc, which very much does depend on
/// orientation, so it needs actual samples rather than a `draw_circle`-style shortcut.
/// Paired straight outline segments (see `draw_bottle`) stop exactly where these
/// start/end, so the two together read as one continuous rounded border instead of
/// each leaving a gap at the corner the other doesn't cover.
#[allow(clippy::too_many_arguments)]
fn arc_rotated(
    center: (f32, f32),
    r: f32,
    a0: f32,
    a1: f32,
    thickness: f32,
    pivot: (f32, f32),
    angle: f32,
    color: Color,
) {
    const SEGMENTS: u32 = 6;
    let mut prev: Option<Vec2> = None;
    for i in 0..=SEGMENTS {
        let t = a0 + (a1 - a0) * (i as f32 / SEGMENTS as f32);
        let local = (center.0 + r * t.cos(), center.1 + r * t.sin());
        let p = rotate_pt(local, pivot, angle);
        if let Some(pp) = prev {
            draw_line(pp.x, pp.y, p.x, p.y, thickness, color);
        }
        prev = Some(p);
    }
}

/// A rect whose bottom two corners are rounded to radius `r` (a plain rect if
/// `rect.h < r`, to avoid the corner geometry folding over itself on a near-empty
/// fractional segment). Used for both the empty glass body's own silhouette and the
/// bottommost liquid segment, which is the only segment whose drawn rect ever
/// actually touches the body's true bottom edge (see `draw_bottle`'s segment loop —
/// every segment above it stacks squarely on the one below).
fn rounded_bottom_rect(rect: Rect, r: f32, pivot: (f32, f32), angle: f32, color: Color) {
    let Rect { x, y, w, h } = rect;
    if h < r {
        rect_rotated(x, y, w, h, pivot, angle, color);
        return;
    }
    rect_rotated(x, y, w, h - r, pivot, angle, color);
    rect_rotated(x + r, y + h - r, w - 2.0 * r, r, pivot, angle, color);
    let cl = rotate_pt((x + r, y + h - r), pivot, angle);
    let cr = rotate_pt((x + w - r, y + h - r), pivot, angle);
    draw_circle(cl.x, cl.y, r, color);
    draw_circle(cr.x, cr.y, r, color);
}

/// Mirror of `rounded_bottom_rect` for the top two corners — used for the neck, so
/// the bottle's mouth reads as an open rounded rim rather than a flat rectangular
/// cap. Same `h < r` plain-rect fallback.
fn rounded_top_rect(rect: Rect, r: f32, pivot: (f32, f32), angle: f32, color: Color) {
    let Rect { x, y, w, h } = rect;
    if h < r {
        rect_rotated(x, y, w, h, pivot, angle, color);
        return;
    }
    rect_rotated(x, y + r, w, h - r, pivot, angle, color);
    rect_rotated(x + r, y, w - 2.0 * r, r, pivot, angle, color);
    let cl = rotate_pt((x + r, y + r), pivot, angle);
    let cr = rotate_pt((x + w - r, y + r), pivot, angle);
    draw_circle(cl.x, cl.y, r, color);
    draw_circle(cr.x, cr.y, r, color);
}

/// `shown_len` is the animated fill level (in units), which during a pour differs
/// from `b.liquid.len()` — `b` already holds the *post*-pour contents (game state
/// updates instantly; only the picture animates), so:
/// - the destination's `shown_len` ramps up from its pre-pour count to
///   `b.liquid.len()`, always `<= b.liquid.len()` — every shown slot is a real,
///   already-present entry in `b.liquid`.
/// - the source's `shown_len` ramps *down* from its pre-pour count to
///   `b.liquid.len()`, meaning it's briefly `>` `b.liquid.len()` — the slots above
///   what's left have already been truncated out of `b.liquid`, so `extra_color`
///   (the color that was poured — uniform across that whole run) fills in for them.
///
/// A resting bottle just passes `b.liquid.len() as f32` — `extra_color` then goes
/// unused, since `shown_len` never exceeds `b.liquid.len()`. `angle`/`pivot` tip the
/// whole bottle for a pour (see the module doc comment above) — a resting bottle
/// passes `angle: 0.0`.
#[allow(clippy::too_many_arguments)]
fn draw_bottle(
    x: f32,
    y: f32,
    bw: f32,
    bh: f32,
    b: &Bottle,
    shown_len: f32,
    extra_color: Color,
    angle: f32,
    pivot: (f32, f32),
) {
    let neck_h = bh * NECK_H_FRAC;
    let shoulder_h = bh * SHOULDER_H_FRAC;
    let body_y = y + neck_h + shoulder_h;
    let body_h = bh - neck_h - shoulder_h;
    let seg_h = body_h / CAPACITY as f32;
    let corner_r = (bw * 0.16).min(10.0);
    let glass = Color::new(0.08, 0.09, 0.13, 1.0);

    // Neck (narrow, centered) and the trapezoid shoulder flaring it out to the body's
    // full width — drawn before the body so the body's square top edge overlaps and
    // hides the seam between the two.
    let neck_w = bw * NECK_W_FRAC;
    let neck_x = x + (bw - neck_w) / 2.0;
    // A subtle bevel, not a real curve — `0.28`/`6.0` (matching the body's own
    // `corner_r`) read as a cartoonish blob on a neck this narrow. Real bottle mouths
    // have a rim, not a pronounced round-over.
    let mouth_r = (neck_w * 0.12).min(2.5);
    rounded_top_rect(
        Rect::new(neck_x, y, neck_w, neck_h),
        mouth_r,
        pivot,
        angle,
        glass,
    );
    let sl = rotate_pt((neck_x, y + neck_h), pivot, angle);
    let sr = rotate_pt((neck_x + neck_w, y + neck_h), pivot, angle);
    let bl = rotate_pt((x, body_y), pivot, angle);
    let br = rotate_pt((x + bw, body_y), pivot, angle);
    draw_triangle(sl, sr, br, glass);
    draw_triangle(sl, br, bl, glass);

    rounded_bottom_rect(
        Rect::new(x, body_y, bw, body_h),
        corner_r,
        pivot,
        angle,
        glass,
    );

    let whole = shown_len.floor() as usize;
    let frac = shown_len - whole as f32;
    // The topmost segment actually drawn (the liquid's visible surface) — `whole`
    // itself only if its own fraction is enough to draw at all, else one below.
    // Bounds-checked below by every caller (`liquid_r` only applies when `idx ==
    // top_idx`, and the loop never reaches an `idx` this could underflow onto).
    let top_idx = if frac > 0.001 {
        whole
    } else {
        whole.wrapping_sub(1)
    };
    let liquid_r = corner_r * 0.55;
    for idx in 0..CAPACITY {
        // Bottle contents are stored bottom-to-top; segment 0 is the bottom slot.
        if idx > whole || (idx == whole && frac <= 0.001) {
            continue;
        }
        let draw_color = if idx < b.liquid.len() {
            liquid_color(b.liquid[idx])
        } else {
            extra_color
        };
        let seg_y = body_y + body_h - (idx as f32 + 1.0) * seg_h;
        let h = if idx == whole { seg_h * frac } else { seg_h };
        let draw_y = seg_y + (seg_h - h);
        // Flush with the container on every side — full `bw` width, no vertical inset
        // either (matching the bottom-most/topmost rounding radii to the container's
        // own `corner_r` below). A horizontal-only inset here used to leave a 3px gap
        // on the sides but none top/bottom, which read as an inconsistent margin
        // around the liquid rather than it actually filling the glass; the outline
        // stroke (drawn after this, further down) sits right on top of the liquid's
        // edge and reads as the glass wall instead of a manually-drawn gap.
        //
        // Segments butt directly against each other too — no inset gap between color
        // bands (used to be `h - 1.0`, which read as a hairline seam cut into a
        // single column of liquid rather than a natural boundary between colors). The
        // bottom-most segment rounds to match the glass exactly; the topmost visible
        // segment (the liquid's actual surface, not just whichever slot happens to be
        // full) gets a smaller, deliberately-not-matching meniscus curve instead of a
        // flat rectangular top edge — both, on the rare single-segment bottle where
        // they're the same slot.
        let rect = Rect::new(x, draw_y, bw, h);
        match (idx == 0, idx == top_idx) {
            (true, true) => {
                rounded_bottom_rect(rect, corner_r, pivot, angle, draw_color);
                rounded_top_rect(rect, liquid_r.min(h * 0.5), pivot, angle, draw_color);
            }
            (true, false) => {
                rounded_bottom_rect(rect, corner_r, pivot, angle, draw_color);
            }
            (false, true) => {
                rounded_top_rect(rect, liquid_r.min(h * 0.5), pivot, angle, draw_color);
            }
            (false, false) => {
                rect_rotated(rect.x, rect.y, rect.w, rect.h, pivot, angle, draw_color);
            }
        }
    }

    // Fog: opaque frosted panel over the bottom `fog` slots, cleared once the bottle
    // has been poured down to (or below) that count — see `Bottle::fog`'s doc comment.
    if b.fog > 0 && b.liquid.len() > b.fog {
        let fog_h = seg_h * b.fog as f32;
        rect_rotated(
            x,
            body_y + body_h - fog_h,
            bw,
            fog_h,
            pivot,
            angle,
            Color::new(0.65, 0.68, 0.72, 0.88),
        );
        let tp = rotate_pt(
            (x + bw / 2.0 - 4.0, body_y + body_h - fog_h / 2.0 + 6.0),
            pivot,
            angle,
        );
        draw_text("?", tp.x, tp.y, 20.0, Color::new(0.3, 0.3, 0.35, 1.0));
    }

    let outline = Color::new(0.55, 0.60, 0.68, 0.9);
    // Side lines start below the mouth's rounded corners (`mouth_r`) rather than at
    // `y` itself — the rounded fill already reads as the corner, an outline stroke
    // running straight into it would just redraw a sharp corner on top.
    line_rotated(
        (neck_x, y + mouth_r),
        (neck_x, y + neck_h),
        2.5,
        pivot,
        angle,
        outline,
    );
    line_rotated(
        (neck_x + neck_w, y + mouth_r),
        (neck_x + neck_w, y + neck_h),
        2.5,
        pivot,
        angle,
        outline,
    );
    line_rotated(
        (neck_x, y + neck_h),
        (x, body_y),
        2.5,
        pivot,
        angle,
        outline,
    );
    line_rotated(
        (neck_x + neck_w, y + neck_h),
        (x + bw, body_y),
        2.5,
        pivot,
        angle,
        outline,
    );
    line_rotated(
        (x, body_y),
        (x, body_y + body_h - corner_r),
        2.5,
        pivot,
        angle,
        outline,
    );
    line_rotated(
        (x + bw, body_y),
        (x + bw, body_y + body_h - corner_r),
        2.5,
        pivot,
        angle,
        outline,
    );
    line_rotated(
        (x + corner_r, body_y + body_h),
        (x + bw - corner_r, body_y + body_h),
        2.5,
        pivot,
        angle,
        outline,
    );

    // The four corner arcs the straight segments above stop short for — see
    // `arc_rotated`'s doc comment. Angles are local (pre-tilt) bottle-space: 0 points
    // right, increasing clockwise (screen y grows downward), matching `rotate_pt`.
    let pi = std::f32::consts::PI;
    arc_rotated(
        (x + corner_r, body_y + body_h - corner_r),
        corner_r,
        pi,
        pi / 2.0,
        2.5,
        pivot,
        angle,
        outline,
    );
    arc_rotated(
        (x + bw - corner_r, body_y + body_h - corner_r),
        corner_r,
        0.0,
        pi / 2.0,
        2.5,
        pivot,
        angle,
        outline,
    );
    arc_rotated(
        (neck_x + mouth_r, y + mouth_r),
        mouth_r,
        pi,
        1.5 * pi,
        2.5,
        pivot,
        angle,
        outline,
    );
    arc_rotated(
        (neck_x + neck_w - mouth_r, y + mouth_r),
        mouth_r,
        0.0,
        -pi / 2.0,
        2.5,
        pivot,
        angle,
        outline,
    );

    if b.is_locked() {
        rect_rotated(x, y, bw, bh, pivot, angle, Color::new(0.0, 0.0, 0.0, 0.45));
        draw_lock_icon(x + bw / 2.0, y - 14.0, liquid_color(b.lock_target.unwrap()));
    }
}

fn draw_lock_icon(cx: f32, cy: f32, target: Color) {
    draw_rectangle(
        cx - 9.0,
        cy - 2.0,
        18.0,
        16.0,
        Color::new(0.2, 0.2, 0.24, 1.0),
    );
    draw_rectangle_lines(cx - 9.0, cy - 2.0, 18.0, 16.0, 2.0, GRAY);
    draw_line(cx - 5.0, cy - 2.0, cx - 5.0, cy - 8.0, 2.5, GRAY);
    draw_line(cx + 5.0, cy - 2.0, cx + 5.0, cy - 8.0, 2.5, GRAY);
    draw_line(cx - 5.0, cy - 8.0, cx + 5.0, cy - 8.0, 2.5, GRAY);
    draw_circle(cx, cy + 6.0, 3.0, target);
}

/// Solved bottles are removed from the board entirely (see root-level "Remove
/// complete bottles" request) rather than just drawn differently — their grid slot
/// is simply left empty, position of every other bottle stays put. `in_flight`
/// additionally skips the two bottles a pour is actively animating (drawn live on
/// top instead — see `run_ui`), which briefly includes a destination that the pour
/// itself just solved: it still finishes visibly filling in the live overlay before
/// vanishing here the moment the animation settles and `display_game` catches up.
fn draw_game(game: &Game, layout: &Layout, in_flight: &[usize]) {
    for (i, b) in game.bottles.iter().enumerate() {
        if in_flight.contains(&i) || b.is_solved() {
            continue;
        }
        let (x, y) = layout.pos[i];
        let pivot = (x + layout.bw / 2.0, y + layout.bh);
        draw_bottle(
            x,
            y,
            layout.bw,
            layout.bh,
            b,
            b.liquid.len() as f32,
            BLACK,
            0.0,
            pivot,
        );
    }
}

fn draw_hud(game: &Game, speed_label: &str, daily_mode: bool) {
    let sw = screen_width();
    let (bg, txt) = match game.phase {
        Phase::Won => (
            Color::new(0.05, 0.18, 0.08, 1.0),
            Color::new(0.28, 1.0, 0.52, 1.0),
        ),
        Phase::Stuck => (
            Color::new(0.18, 0.05, 0.05, 1.0),
            Color::new(1.0, 0.38, 0.38, 1.0),
        ),
        Phase::Playing => (
            Color::new(0.07, 0.07, 0.12, 1.0),
            Color::new(0.68, 0.68, 0.85, 1.0),
        ),
    };
    draw_rectangle(0.0, 0.0, sw, HUD_H, bg);

    // Daily-challenge runs freeze here rather than advancing (see
    // `control::Control::daily_mode`) — "Next level..."/"Restarting..." would lie.
    let status = match (game.phase, daily_mode) {
        (Phase::Playing, _) => String::new(),
        (Phase::Won, true) => "  - SOLVED!".to_owned(),
        (Phase::Stuck, true) => "  - STUCK.".to_owned(),
        (Phase::Won, false) => "  - SOLVED! Next level...".to_owned(),
        (Phase::Stuck, false) => "  - STUCK. Restarting...".to_owned(),
    };
    let msg = format!(
        "Water Sort   Level {}   Colors {}   Moves {}{}",
        game.level, game.colors, game.moves, status
    );
    draw_text(&msg, 10.0, 24.0, 20.0, txt);

    let sd = measure_text(speed_label, None, 20, 1.0);
    draw_text(speed_label, sw - 8.0 - sd.width, 24.0, 20.0, txt);
}

fn bottle_render_cache(size: (f32, f32)) -> RenderCache {
    RenderCache::new(Rect::new(0.0, HUD_H, size.0, size.1 - HUD_H))
}

// ── CLI args (native only — meaningless in a browser tab) ───────────────────────

pub struct CliArgs {
    debug: bool,
    once: bool,
    #[cfg(not(target_arch = "wasm32"))]
    no_ui: bool,
}

#[cfg(not(target_arch = "wasm32"))]
pub fn parse_cli_args() -> CliArgs {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (base, rest) = game_common::parse_base_args(&args);
    if let Some(other) = rest.first() {
        eprintln!("unknown argument '{other}' (expected --debug, --once, --no-ui)");
        std::process::exit(2);
    }
    CliArgs {
        debug: base.debug,
        once: base.once,
        no_ui: base.no_ui,
    }
}

#[cfg(target_arch = "wasm32")]
pub fn parse_cli_args() -> CliArgs {
    bundled_cli()
}

/// Exactly the `CliArgs` a browser build gets — no argv. Shared by the wasm
/// `parse_cli_args` above and `play()`, and compiles on native too (`no_ui: false`).
fn bundled_cli() -> CliArgs {
    CliArgs {
        debug: false,
        once: false,
        #[cfg(not(target_arch = "wasm32"))]
        no_ui: false,
    }
}

fn log_move(game: &Game, m: Move) {
    eprintln!(
        "level={} gen={} move={:?} amount={} moves={} bottles={}",
        game.level,
        game.generation,
        m,
        game.pour_amount(m),
        game.moves,
        game.bottles.len(),
    );
    // `--debug` alone logs which pour was chosen but not what the board looked like,
    // which isn't enough to tell a bad *choice* from a bad *position* after the fact.
    // `HCG_DIAG=1` adds the full board (bottom-to-top letters per bottle, `*` = locked)
    // before every pour, so a soak log can be replayed/audited offline — same
    // env-gated instrumentation pattern as bubble-shooter's. This is what caught the
    // empty-bottle score-farming bug in `solver.rs`.
    if std::env::var("HCG_DIAG").is_ok() {
        let board: Vec<String> = game
            .bottles
            .iter()
            .enumerate()
            .map(|(i, b)| {
                let cs: String = b.liquid.iter().map(|c| (b'a' + c) as char).collect();
                let lk = if b.is_locked() { "*" } else { "" };
                format!("{i}:{cs}{lk}")
            })
            .collect();
        eprintln!("  board {}", board.join(" "));
    }
}

fn print_result(game: &Game) {
    let solved = game.bottles.iter().filter(|b| b.is_solved()).count();
    println!(
        "result={} level={} moves={} solved={}/{}",
        if game.phase == Phase::Won {
            "won"
        } else {
            "stuck"
        },
        game.level,
        game.moves,
        solved,
        game.colors,
    );
}

#[cfg(not(target_arch = "wasm32"))]
pub fn run_headless(cli: CliArgs) -> ! {
    macroquad::rand::srand(screenshot::seed());
    let mut level = 1u32;
    let mut generation = 0u32;
    let mut game = Game::new(level, generation);
    let mut solver = Solver::new();

    loop {
        match game.phase {
            Phase::Playing => {
                if let Some(m) = solver.choose_move(&game) {
                    if cli.debug {
                        log_move(&game, m);
                    }
                    game.apply(m);
                } else {
                    game.phase = Phase::Stuck;
                }
            }
            Phase::Won | Phase::Stuck => {
                print_result(&game);
                if cli.once {
                    std::process::exit(0);
                }
                level = if game.phase == Phase::Won {
                    level + 1
                } else {
                    level
                };
                generation += 1;
                game = Game::new(level, generation);
                solver = Solver::new();
            }
        }
    }
}

pub fn conf() -> Conf {
    Conf {
        window_title: "Water Sort".to_owned(),
        window_width: 900,
        window_height: 720,
        high_dpi: true,
        ..Default::default()
    }
}

/// Entry point for the standalone per-game binary — same window/`--no-ui` branching
/// `main()` used to do.
pub fn start() {
    let cli = parse_cli_args();
    #[cfg(not(target_arch = "wasm32"))]
    if cli.no_ui {
        run_headless(cli);
    }
    macroquad::Window::from_config(conf(), async move {
        run_ui(cli).await;
    });
}

/// Entry point for the merged multi-game binary (see `bundle/`): no argv parsing —
/// the bundled build gets the same `CliArgs` the browser build does.
pub async fn play() {
    run_ui(bundled_cli()).await;
}

/// Entry point for the standalone shell (see
/// `.notes/steam-standalone-menu-handoff.md`): runs until the player asks to leave
/// (Esc) or closes the window, then returns instead of looping forever. `play()` above
/// stays as-is for the browser, where there is nothing to return to.
#[cfg(not(target_arch = "wasm32"))]
pub async fn play_until_exit() -> control::ExitReason {
    run_ui(bundled_cli()).await
}

/// Pour animation state for the two bottles currently mid-transfer.
struct Pour {
    m: Move,
    from_before: usize,
    to_before: usize,
    amount: usize,
    color: game::Color,
}

/// Which shape the stream is drawn in — see the `pour` block's Hold/Fall beats in
/// `run_ui`. `Drops`' `taper` (0..1 across the Fall beat) shrinks the droplets as the
/// last of the flow drips out, rather than the stream just vanishing outright.
enum StreamShape {
    Ribbon,
    Drops { taper: f32 },
}

pub async fn run_ui(cli: CliArgs) -> control::ExitReason {
    let mut control = control::Control::new();
    macroquad::rand::srand(control.seed());

    let mut level = 1u32;
    let mut generation = 0u32;
    let mut game = Game::new(level, generation);
    let mut display_game = game.clone();
    let mut solver = Solver::new();
    let mut accum = 0.0f32;
    let mut anim_t: f32 = 1.0;
    let mut pour: Option<Pour> = None;
    let mut end_time: Option<f64> = None;
    let mut shot = screenshot::Capture::from_env();
    // Set once a daily-challenge run ends (see `control::Control::daily_mode`) — the
    // board freezes on its final frame instead of starting a new level.
    let mut daily_done = false;

    render_cache::prewarm_glyphs(&["?"], &[16, 20]);

    // The ambient wall runs every game at once: stay silent there.
    if control.stream_mode() {
        audio::playback::set_muted(true);
    }
    let sound = sound::Sound::load(screenshot::seed()).await;

    let mut cached_size = (screen_width(), screen_height());
    let mut board_cache = bottle_render_cache(cached_size);
    board_cache.mark_dirty();

    loop {
        control.handle_keys();
        if let Some(reason) = control.exit_requested() {
            break reason;
        }
        if let Some(seed) = control.take_reseed() {
            macroquad::rand::srand(seed);
            level = 1;
            generation = 0;
            game = Game::new(level, generation);
            display_game = game.clone();
            solver = Solver::new();
            accum = 0.0;
            anim_t = 1.0;
            pour = None;
            end_time = None;
            daily_done = false;
            board_cache.mark_dirty();
        }
        let now = macroquad::miniquad::date::now();
        let dt = control.scale(get_frame_time().min(0.1));

        let cur_size = (screen_width(), screen_height());
        if cur_size != cached_size {
            board_cache = bottle_render_cache(cur_size);
            cached_size = cur_size;
        }
        let prev_t = anim_t;
        anim_t = (anim_t + dt / ANIM_DURATION).min(1.0);
        if let Some(p) = &pour
            && prev_t < STREAM_START
            && anim_t >= STREAM_START
        {
            sound.pour(p.to_before, p.to_before + p.amount);
        }
        if prev_t < 1.0 && anim_t >= 1.0 {
            // Everything the pour caused lands now, with the bottle back in its slot:
            // `display_game` is still the pre-pour board.
            sound.set_down();
            if let Some(p) = &pour
                && game.bottles[p.m.to].is_solved()
            {
                if game.phase == Phase::Won {
                    sound.won();
                } else {
                    sound.sorted(p.color);
                }
            }
            if game.phase == Phase::Stuck {
                sound.stuck();
            }
            let unlocked = |g: &Game| g.bottles.iter().filter(|b| b.unlocked).count();
            if unlocked(&game) > unlocked(&display_game) {
                sound.unlock();
            }
            display_game = game.clone();
            pour = None;
            board_cache.mark_dirty();
        }

        match game.phase {
            Phase::Playing => {
                accum += dt;
                if anim_t >= 1.0 && accum >= TICK {
                    accum -= TICK;
                    if let Some(m) = solver.choose_move(&game) {
                        if cli.debug {
                            log_move(&game, m);
                        }
                        let from_before = game.bottles[m.from].liquid.len();
                        let to_before = game.bottles[m.to].liquid.len();
                        let amount = game.pour_amount(m);
                        let color = *game.bottles[m.from].liquid.last().unwrap();
                        game.apply(m);
                        board_cache.mark_dirty();
                        pour = Some(Pour {
                            m,
                            from_before,
                            to_before,
                            amount,
                            color,
                        });
                        anim_t = 0.0;
                    } else {
                        game.phase = Phase::Stuck;
                        sound.stuck();
                    }
                }
            }
            Phase::Won | Phase::Stuck => {
                let t = *end_time.get_or_insert(now);
                if !daily_done && now - t > RESTART_DELAY {
                    if cli.once {
                        print_result(&game);
                        std::process::exit(0);
                    }
                    let solved = game.bottles.iter().filter(|b| b.is_solved()).count() as i64;
                    control.episode_complete("water-sort", solved * 100 + game.level as i64);
                    if control.daily_mode() {
                        let result_clause = if game.phase == Phase::Won {
                            format!("solve level {}", game.level)
                        } else {
                            format!("get stuck on level {}", game.level)
                        };
                        control::share_result(&control::daily_verdict_text(
                            "Water Sort",
                            control::daily_puzzle_number(),
                            &result_clause,
                        ));
                        daily_done = true;
                        board_cache.mark_dirty();
                    } else {
                        if game.phase == Phase::Won {
                            level += 1;
                        }
                        generation += 1;
                        game = Game::new(level, generation);
                        display_game = game.clone();
                        solver = Solver::new();
                        end_time = None;
                        accum = 0.0;
                        anim_t = 1.0;
                        pour = None;
                        board_cache.mark_dirty();
                    }
                }
            }
        }

        // Computed from `display_game`, not `game`, and only after the phase-transition
        // branch above (which can replace both with a differently-sized board on the
        // same frame a level changes) — using a bottle count that's already stale by one
        // reassignment index-panics `layout.pos` the moment level N+1 has a different
        // bottle count than level N.
        let layout = Layout::compute(display_game.bottles.len());

        clear_background(Color::new(0.05, 0.06, 0.10, 1.0));
        if !control.stream_mode() {
            draw_hud(&game, &control.label(), control.daily_mode());
        }

        let in_flight: Vec<usize> = pour
            .as_ref()
            .map(|p| vec![p.m.from, p.m.to])
            .unwrap_or_default();
        board_cache.draw(|| draw_game(&display_game, &layout, &in_flight));

        if let Some(p) = &pour {
            let bw = layout.bw;
            let bh = layout.bh;
            let home = layout.pos[p.m.from];
            let (tx, ty) = layout.pos[p.m.to];
            let stream_color = liquid_color(p.color);

            // Sign picks which way the source leans once it's pouring; see
            // `rotate_pt`'s convention (positive angle leans the bottle's top toward
            // +x). Default is "lean toward the destination", which — since the hover
            // spot below is offset the *opposite* way — also keeps the hovering bottle
            // on its own side of the destination. Flipped if that would push it off
            // the near screen edge (a pour into an edge column from further in).
            let (scx, _) = layout.top_center(p.m.from);
            let (dcx, _) = layout.top_center(p.m.to);
            let neck_w = bw * NECK_W_FRAC;
            // How far the pour lip (the rim corner on the leaning side — the low one,
            // hence the one liquid actually runs off) travels sideways from the
            // bottle's own centerline once tipped to `TILT_MAX`: it rotates about the
            // body's bottom-center, so the whole bottle height is the lever arm.
            let (sin_t, cos_t) = TILT_MAX.sin_cos();
            let lip_reach = bh * sin_t + neck_w / 2.0 * cos_t;
            let mut sign = if dcx >= scx { 1.0 } else { -1.0 };
            // Hover offset the *other* way by exactly `lip_reach`, so that tipping to
            // `TILT_MAX` lands the lip directly over the destination's mouth and the
            // stream falls straight down into it. Anchoring the bottle's *body* over
            // the destination instead (what this used to do) put the lip a full
            // `lip_reach` past it, pointing away, with the stream forced to hook back.
            let hover_x = |sign: f32| tx - sign * lip_reach;
            let fits = |x: f32| x >= 4.0 && x + bw <= screen_width() - 4.0;
            if !fits(hover_x(sign)) && fits(hover_x(-sign)) {
                sign = -sign;
            }
            // Tipping over swings the lip down this far below the bottle's own top
            // edge, so the hover height is measured from the lip, not from `src_pos`
            // — otherwise a big `TILT_MAX` silently eats most of the stream's length.
            let lip_drop = bh * (1.0 - cos_t) + neck_w / 2.0 * sin_t;
            // The tilted silhouette's highest point (the top-left corner, swung down
            // by the tilt) is what has to clear the HUD — not `src_pos.1`, which by
            // then is well above anything actually drawn.
            let top_gap = bh * (1.0 - cos_t) - bw / 2.0 * sin_t;
            let hover = (
                hover_x(sign).clamp(4.0, (screen_width() - bw - 4.0).max(4.0)),
                (ty - bh * POUR_HEIGHT_FRAC - lip_drop).max(HUD_H + 4.0 - top_gap),
            );

            // The hover-and-pour beat is itself three sub-beats — Rise (tilt 0 ->
            // peak), Hold (tilt pinned at peak, liquid actually transfers), Drip (tilt
            // *still* pinned, last droplets fall). The bottle only rights itself in
            // the Return beat, together with the trip back to its grid slot, so the
            // tilt — and with it the pour lip the stream hangs off — never moves while
            // any stream is on screen. Rotating the bottle back upright underneath a
            // live stream is what produced both earlier versions of this animation:
            // the stream either swept back through the source as the tilt eased down,
            // or (when frozen to compensate) detached from the bottle's real neck.
            // Righting after the stream is gone sidesteps the choice entirely.
            let pour_end = LIFT_FRAC + POUR_FRAC;
            let rise_end = STREAM_START;
            let hold_end = rise_end + POUR_FRAC * POUR_HOLD_FRAC;

            let (src_pos, tilt, pour_progress, stream) = if anim_t < LIFT_FRAC {
                let lt = ease_in_out(anim_t / LIFT_FRAC);
                (lerp_pt(home, hover, lt), 0.0, 0.0, None)
            } else if anim_t < rise_end {
                let lt = ease_in_out((anim_t - LIFT_FRAC) / (rise_end - LIFT_FRAC));
                (hover, TILT_MAX * lt * sign, 0.0, None)
            } else if anim_t < hold_end {
                let lt = ease_in_out((anim_t - rise_end) / (hold_end - rise_end));
                (hover, TILT_MAX * sign, lt, Some(StreamShape::Ribbon))
            } else if anim_t < pour_end {
                let lt = (anim_t - hold_end) / (pour_end - hold_end);
                (
                    hover,
                    TILT_MAX * sign,
                    1.0,
                    Some(StreamShape::Drops { taper: lt }),
                )
            } else {
                let lt = ease_in_out((anim_t - pour_end) / (1.0 - pour_end));
                (
                    lerp_pt(hover, home, lt),
                    TILT_MAX * (1.0 - lt) * sign,
                    1.0,
                    None,
                )
            };

            let from_len = p.from_before as f32 - p.amount as f32 * pour_progress;
            let to_len = p.to_before as f32 + p.amount as f32 * pour_progress;

            let src_pivot = (src_pos.0 + bw / 2.0, src_pos.1 + bh);
            let dst_pivot = (tx + bw / 2.0, ty + bh);

            draw_bottle(
                src_pos.0,
                src_pos.1,
                bw,
                bh,
                &game.bottles[p.m.from],
                from_len,
                stream_color,
                tilt,
                src_pivot,
            );
            draw_bottle(
                tx,
                ty,
                bw,
                bh,
                &game.bottles[p.m.to],
                to_len,
                stream_color,
                0.0,
                dst_pivot,
            );

            if let Some(shape) = stream {
                let neck_h = bh * NECK_H_FRAC;
                let shoulder_h = bh * SHOULDER_H_FRAC;
                let body_h = bh - neck_h - shoulder_h;
                let seg_h = body_h / CAPACITY as f32;
                // The stream hangs off the *pour lip* — the rim corner on the leaning
                // side, which the same rotation `draw_bottle` uses puts lower than the
                // other one, so it's the corner liquid actually runs off — not the
                // rim's center. Run through `rotate_pt` with the bottle's own live
                // `tilt`/`src_pivot` so it is by construction the point on screen the
                // drawn glass ends at. `hover` (above) is placed so this lands
                // directly over `dst_surface` at peak tilt, which is the only tilt any
                // stream is ever drawn at.
                let lip_v = rotate_pt(
                    (src_pos.0 + bw / 2.0 + sign * neck_w / 2.0, src_pos.1),
                    src_pivot,
                    tilt,
                );
                let lip = (lip_v.x, lip_v.y);
                let dst_surface = (
                    tx + bw / 2.0,
                    ty + neck_h + shoulder_h + body_h - to_len.min(CAPACITY as f32) * seg_h,
                );
                // Gravity shape: leaves the lip with whatever small sideways offset is
                // left over, ends falling vertically into the surface. With the lip
                // parked over the destination this degenerates to the straight
                // vertical drop it should be, so the bow is really just insurance for
                // the clamped-hover cases (a destination in the top row, where `hover`
                // can't sit a full bottle height up).
                let control = (dst_surface.0, (lip.1 + dst_surface.1) / 2.0);

                match shape {
                    StreamShape::Ribbon => {
                        // The neck's interior, full of the liquid on its way out — drawn
                        // in the bottle's own (tilted) frame, inset just inside the neck
                        // outline. Without it the fall appears to start out of empty
                        // glass: the liquid segments only ever fill the *body*, leaving
                        // the neck dark exactly where the stream begins. Anchored at the
                        // rim and shrinking back toward it as the transfer finishes, so
                        // the neck reads as draining behind the stream rather than
                        // staying brim-full right up to the moment the flow stops.
                        rect_rotated(
                            src_pos.0 + (bw - neck_w) / 2.0 + 4.0,
                            src_pos.1 + 3.0,
                            neck_w - 8.0,
                            (neck_h + shoulder_h) * (1.0 - 0.45 * pour_progress),
                            src_pivot,
                            tilt,
                            stream_color,
                        );

                        const RIBBON_SEGMENTS: u32 = 6;
                        let mut prev = lip;
                        for k in 1..=RIBBON_SEGMENTS {
                            let s = k as f32 / RIBBON_SEGMENTS as f32;
                            let pt = bezier(lip, control, dst_surface, s);
                            // Narrows slightly on the way down, the way a real falling
                            // stream does as it speeds up.
                            let thickness = 7.0 - 2.0 * s;
                            draw_line(prev.0, prev.1, pt.0, pt.1, thickness, stream_color);
                            prev = pt;
                        }
                    }
                    StreamShape::Drops { taper } => {
                        let phase = (now * 3.0).fract() as f32;
                        let radius = 3.5 - 1.5 * taper;
                        for k in 0..STREAM_DROPLETS {
                            let dt = (phase + k as f32 / STREAM_DROPLETS as f32).fract();
                            let pos = bezier(lip, control, dst_surface, dt);
                            draw_circle(pos.0, pos.1, radius, stream_color);
                        }
                    }
                }
            }
        }

        shot.tick();
        screenshot::handle_hotkey();
        control.draw_overlay();
        next_frame().await;
    }
}
