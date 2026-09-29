//! Signs and printed text in the 3D view: each label (text lines, optionally beside a
//! pictogram, on a plate or bare) is baked once into its own texture, then drawn onto
//! its wall or door one screen column at a time — every column intersects its ray with
//! the sign's plane, samples its own slice of the texture and is depth-tested on its
//! own. So a sign foreshortens, skews and is cut by nearer things exactly like the wall
//! it hangs on. (Text used to be a screen-space string scaled and pinned at one
//! projected point: no perspective, and all-or-nothing occlusion.)
//!
//! Baking draws text into a render target, so every string must already be in the
//! font atlas at `BAKE_PX` (see `render_cache::prewarm_glyphs`) — growing the atlas
//! under a render-target camera corrupts screen text for the rest of the run.

use crate::view::raycast::{COL_STEP, View, fog_at, lit_at};
use macroquad::prelude::*;
use std::cell::RefCell;
use std::collections::HashMap;

/// The one font size labels are rasterized at (the texture is scaled, not re-rendered).
pub const BAKE_PX: u16 = 64;
/// Columns nearer than this aren't drawn (the sign is at the camera).
const NEAR: f32 = 0.2;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Icon {
    Man,
    Woman,
    /// An arrow pointing down (at whatever's under the sign).
    Down,
    /// Arrows pointing left / right (a signpost's "THIS WAY").
    Left,
    Right,
    /// A round clock face stopped at six o'clock (always tea-time).
    Clock,
}

impl Icon {
    /// Width over height of the pictogram's box.
    fn aspect(self) -> f32 {
        match self {
            Icon::Man | Icon::Woman | Icon::Down => 0.5,
            Icon::Left | Icon::Right | Icon::Clock => 1.0,
        }
    }
}

/// What a label shows. Also its bake-cache key.
#[derive(Clone, Debug, PartialEq)]
pub struct Spec {
    /// Text lines, each centred, stacked top to bottom.
    pub lines: Vec<&'static str>,
    /// A pictogram left of the text.
    pub icon: Option<Icon>,
    pub ink: Color,
    /// The plate behind the text; `None` for text straight on the surface.
    pub plate: Option<Color>,
    /// Mirror-writing: the whole label drawn flipped left to right, from either side.
    pub mirror: bool,
}

impl Spec {
    pub fn new(lines: &[&'static str], ink: Color, plate: Option<Color>) -> Spec {
        Spec {
            lines: lines.to_vec(),
            icon: None,
            ink,
            plate,
            mirror: false,
        }
    }

    /// One name, broken into two balanced lines at a space if it's longer than
    /// `max_chars` — a long name on one line would shrink to fit and go unreadable.
    pub fn wrapped(text: &'static str, max_chars: usize, ink: Color, plate: Option<Color>) -> Spec {
        let lines = match text
            .match_indices(' ')
            .map(|(i, _)| i)
            .min_by_key(|&i| i.abs_diff(text.len() / 2))
        {
            Some(i) if text.len() > max_chars => vec![&text[..i], &text[i + 1..]],
            _ => vec![text],
        };
        Spec {
            lines,
            icon: None,
            ink,
            plate,
            mirror: false,
        }
    }
}

/// A baked label: its texture and the texture's cap height (px), for sizing.
#[derive(Clone)]
pub struct Label {
    target: RenderTarget,
    cap: f32,
    mirror: bool,
    /// On a plate (opaque) rather than bare text over its surface.
    plate: bool,
}

/// How a label fades into the distance, like the wall strips: the palette's `haze`,
/// the light where it hangs, and the fog colour it fades toward.
#[derive(Clone, Copy)]
pub struct Fade {
    pub haze: f32,
    pub light: f32,
    pub fog: Color,
}

/// Fogged darker than this, a label isn't drawn at all — it's lost in the murk.
const LOST: f32 = 0.1;

impl Label {
    fn tex(&self) -> &Texture2D {
        &self.target.texture
    }

    /// World size giving capitals `cap` tall, shrunk to at most `max_w` wide.
    pub fn size(&self, cap: f32, max_w: f32) -> Vec2 {
        let (w, h) = (self.tex().width(), self.tex().height());
        let k = (cap / self.cap).min(max_w / w);
        vec2(w, h) * k
    }
}

thread_local! {
    static BAKED: RefCell<HashMap<String, Label>> = RefCell::new(HashMap::new());
}

/// The label for `spec`, baked on first use and kept for the rest of the run (the set
/// of distinct labels is small and fixed).
pub fn get(spec: &Spec) -> Label {
    let key = format!("{spec:?}");
    BAKED.with(|b| {
        b.borrow_mut()
            .entry(key)
            .or_insert_with(|| bake(spec))
            .clone()
    })
}

/// Where everything goes in a label's texture (px): its size, the pictogram's box and
/// each text line's pen position (left, baseline).
#[derive(Debug)]
struct Layout {
    size: Vec2,
    icon: Option<Rect>,
    pens: Vec<Vec2>,
}

/// Lays out `spec` given the font's cap height and each line's measured width: the
/// content (pictogram, then the text column) centred in the texture with even padding,
/// every line centred in the column, the pictogram and text centred on each other.
fn layout(spec: &Spec, cap: f32, widths: &[f32]) -> Layout {
    let gap = cap * 0.6;
    let pad = if spec.plate.is_some() {
        cap * 0.55
    } else {
        cap * 0.2
    };
    let n = spec.lines.len() as f32;
    let text_h = cap * n + gap * (n - 1.0).max(0.0);
    let text_w = widths.iter().copied().fold(0.0, f32::max);
    let (icon_w, icon_h) = if let Some(icon) = spec.icon {
        let h = (cap * 2.4).max(text_h);
        (h * icon.aspect(), h)
    } else {
        (0.0, 0.0)
    };
    let sep = if icon_w > 0.0 && text_w > 0.0 {
        cap * 0.5
    } else {
        0.0
    };
    let content_h = text_h.max(icon_h);
    let size = vec2(icon_w + sep + text_w + pad * 2.0, content_h + pad * 2.0).ceil();
    // Rounding the texture up leaves a fraction of a pixel: split it evenly.
    let left = (size.x - (icon_w + sep + text_w)) * 0.5;
    let top = (size.y - content_h) * 0.5;
    let icon = spec
        .icon
        .map(|_| Rect::new(left, top + (content_h - icon_h) * 0.5, icon_w, icon_h));
    let x0 = left + icon_w + sep;
    let y0 = top + (content_h - text_h) * 0.5;
    let pens = widths
        .iter()
        .enumerate()
        .map(|(i, w)| vec2(x0 + (text_w - w) * 0.5, y0 + cap + i as f32 * (cap + gap)))
        .collect();
    Layout { size, icon, pens }
}

fn bake(spec: &Spec) -> Label {
    let cap = measure_text("H", None, BAKE_PX, 1.0).height;
    let widths: Vec<f32> = spec
        .lines
        .iter()
        .map(|l| measure_text(l, None, BAKE_PX, 1.0).width)
        .collect();
    let lay = layout(spec, cap, &widths);
    let (w, h) = (lay.size.x, lay.size.y);

    // `sample_count: 0`: see `RenderCache::build` — the default takes an MSAA resolve
    // path that traps on some WebGL contexts.
    let target = render_target_ex(
        w as u32,
        h as u32,
        RenderTargetParams {
            sample_count: 0,
            ..Default::default()
        },
    );
    target.texture.set_filter(FilterMode::Linear);
    let mut camera = Camera2D::from_display_rect(Rect::new(0.0, 0.0, w, h));
    camera.render_target = Some(target.clone());
    set_camera(&camera);
    // Bare text clears to the ink's own colour at zero alpha, so glyph edges blend
    // toward the ink rather than toward black.
    let ink = spec.ink;
    clear_background(spec.plate.unwrap_or(Color::new(ink.r, ink.g, ink.b, 0.0)));
    if let (Some(icon), Some(r)) = (spec.icon, lay.icon) {
        pictogram(icon, r, ink);
    }
    for (line, pen) in spec.lines.iter().zip(&lay.pens) {
        draw_text_ex(
            line,
            pen.x,
            pen.y,
            TextParams {
                font_size: BAKE_PX,
                color: ink,
                ..Default::default()
            },
        );
    }
    set_default_camera();
    Label {
        target,
        cap,
        mirror: spec.mirror,
        plate: spec.plate.is_some(),
    }
}

/// The standard washroom figures, filling `r` (about 1:2).
fn pictogram(icon: Icon, r: Rect, col: Color) {
    if matches!(icon, Icon::Left | Icon::Right | Icon::Clock) {
        return sign_icon(icon, r, col);
    }
    let u = r.w;
    let cx = r.x + r.w * 0.5;
    let head = u * 0.17;
    draw_circle(cx, r.y + head, head, col);
    let body = r.y + head * 2.0 + u * 0.08;
    let leg_w = u * 0.17;
    match icon {
        Icon::Left | Icon::Right | Icon::Clock => {}
        Icon::Down => {
            let head = r.h * 0.4;
            draw_rectangle(cx - u * 0.14, r.y, u * 0.28, r.h - head, col);
            draw_triangle(
                vec2(r.x, r.y + r.h - head),
                vec2(r.x + r.w, r.y + r.h - head),
                vec2(cx, r.y + r.h),
                col,
            );
        }
        Icon::Man => {
            let (tw, th) = (u * 0.56, r.h * 0.36);
            draw_rectangle(cx - tw * 0.5, body, tw, th, col);
            let arm = u * 0.13;
            let arm_h = th * 0.9;
            draw_rectangle(cx - tw * 0.5 - arm - u * 0.04, body, arm, arm_h, col);
            draw_rectangle(cx + tw * 0.5 + u * 0.04, body, arm, arm_h, col);
            let legs = body + th;
            let lh = r.y + r.h - legs;
            draw_rectangle(cx - tw * 0.5, legs, leg_w, lh, col);
            draw_rectangle(cx + tw * 0.5 - leg_w, legs, leg_w, lh, col);
        }
        Icon::Woman => {
            // The dress: a trapezoid flaring from the shoulders.
            let th = r.h * 0.46;
            let (top_w, hem_w) = (u * 0.4, u * 0.84);
            let (tl, tr) = (vec2(cx - top_w * 0.5, body), vec2(cx + top_w * 0.5, body));
            let (bl, br) = (
                vec2(cx - hem_w * 0.5, body + th),
                vec2(cx + hem_w * 0.5, body + th),
            );
            draw_triangle(tl, tr, br, col);
            draw_triangle(tl, br, bl, col);
            // Arms hang along the dress's flaring sides, a small gap off them — upright
            // bars beside a flared dress read as sticks jammed into it.
            let (gap, arm) = (u * 0.05, u * 0.11);
            for (top, hem) in [(tl, bl), (tr, br)] {
                let e = hem - top;
                let out = vec2(e.y, -e.x).normalize() * (top.x - cx).signum() * -1.0;
                let out = if out.x * (top.x - cx) < 0.0 {
                    -out
                } else {
                    out
                };
                let (s0, s1) = (top + out * gap, top + out * (gap + arm));
                let (e0, e1) = (s0 + e * 0.55, s1 + e * 0.55);
                draw_triangle(s0, s1, e1, col);
                draw_triangle(s0, e1, e0, col);
                // A rounded hand at the end.
                let hand = (e0 + e1) * 0.5;
                draw_circle(hand.x, hand.y, arm * 0.5, col);
            }
            let legs = body + th;
            let lh = r.y + r.h - legs;
            draw_rectangle(cx - u * 0.2, legs, leg_w, lh, col);
            draw_rectangle(cx + u * 0.2 - leg_w, legs, leg_w, lh, col);
        }
    }
}

/// The garden's signpost arrows and clock face, filling `r` (1:1).
fn sign_icon(icon: Icon, r: Rect, col: Color) {
    let cx = r.x + r.w * 0.5;
    match icon {
        Icon::Left | Icon::Right => {
            let cy = r.y + r.h * 0.5;
            let head = r.w * 0.45;
            let (tip, base) = if icon == Icon::Left {
                (r.x, r.x + head)
            } else {
                (r.x + r.w, r.x + r.w - head)
            };
            let shaft = r.h * 0.26;
            let (s0, s1) = if icon == Icon::Left {
                (base, r.x + r.w)
            } else {
                (r.x, base)
            };
            draw_rectangle(s0, cy - shaft * 0.5, s1 - s0, shaft, col);
            draw_triangle(
                vec2(tip, cy),
                vec2(base, cy - r.h * 0.42),
                vec2(base, cy + r.h * 0.42),
                col,
            );
        }
        Icon::Clock => {
            let c = vec2(cx, r.y + r.h * 0.5);
            let rad = r.w.min(r.h) * 0.5;
            draw_circle(c.x, c.y, rad, col);
            draw_circle(c.x, c.y, rad * 0.88, Color::new(0.96, 0.93, 0.84, 1.0));
            for i in 0..12 {
                let a = i as f32 * std::f32::consts::TAU / 12.0;
                let k = if i % 3 == 0 { 0.07 } else { 0.04 };
                draw_circle(
                    c.x + a.sin() * rad * 0.74,
                    c.y - a.cos() * rad * 0.74,
                    rad * k,
                    col,
                );
            }
            // Six o'clock: minute hand up, hour hand down.
            draw_line(c.x, c.y, c.x, c.y - rad * 0.66, rad * 0.07, col);
            draw_line(c.x, c.y, c.x, c.y + rad * 0.46, rad * 0.11, col);
            draw_circle(c.x, c.y, rad * 0.08, col);
        }
        _ => {}
    }
}

/// What set dressing stands in each screen column, for hiding whatever is behind it:
/// per column, the view depth and screen-y span `(depth, top, bottom)` of every piece
/// painted there. By height, not just depth — a desk in front of a poster hides the
/// poster's bottom edge, not the whole column (it used to, leaving a sliver of the IT
/// room's poster).
#[derive(Default)]
pub struct Occluders(pub Vec<Vec<(f32, f32, f32)>>);

/// A piece within this much in front of a thing doesn't hide it (it stands on it).
const OCCLUDE_TOL: f32 = 0.03;

impl Occluders {
    pub fn columns(&self) -> usize {
        self.0.len()
    }

    /// Is screen height `y` in column `ci`, `depth` away, behind a piece of dressing?
    pub fn hides(&self, ci: usize, depth: f32, y: f32) -> bool {
        self.0.get(ci).is_some_and(|c| {
            c.iter()
                .any(|&(d, top, bot)| d + OCCLUDE_TOL <= depth && (top..=bot).contains(&y))
        })
    }

    /// The parts of `y0..y1` in column `ci`, `depth` away, that no nearer piece covers.
    fn gaps(&self, ci: usize, depth: f32, y0: f32, y1: f32) -> Vec<(f32, f32)> {
        let mut cover: Vec<(f32, f32)> = self
            .0
            .get(ci)
            .map(|c| {
                c.iter()
                    .filter(|&&(d, top, bot)| d + OCCLUDE_TOL <= depth && bot > y0 && top < y1)
                    .map(|&(_, top, bot)| (top, bot))
                    .collect()
            })
            .unwrap_or_default();
        crate::util::sort_by_key(&mut cover, |t| t.0);
        let mut out = Vec::new();
        let mut y = y0;
        for (top, bot) in cover {
            if top > y {
                out.push((y, top.min(y1)));
            }
            y = y.max(bot);
            if y >= y1 {
                break;
            }
        }
        if y < y1 {
            out.push((y, y1));
        }
        out
    }
}

/// Draws `label` flat on a vertical plane through floor point `centre`, its width
/// running along unit vector `along`, centred at height `z`, `size` in world units.
/// Reads left to right from whichever side it's seen — callers skip the back face.
/// `occ`: the dressing in front, which hides the parts of the label it covers.
/// `fade`: fogged with distance as the wall strips are — a plate darkened and washed
/// toward the fog, bare text faded out — and not drawn once lost in it (a bright door
/// plate used to float in the murk where its door had gone dark).
#[allow(clippy::too_many_arguments)]
pub fn draw_on_wall(
    view: &View,
    occ: &Occluders,
    fade: Fade,
    label: &Label,
    centre: Vec2,
    along: Vec2,
    z: f32,
    size: Vec2,
) {
    let hw = size.x * 0.5;
    let (mut a, mut b) = (centre - along * hw, centre + along * hw);
    let (da, db) = (view.depth(a), view.depth(b));
    if da < NEAR && db < NEAR {
        return;
    }
    if da < NEAR {
        a += (b - a) * ((NEAR - da) / (db - da));
    } else if db < NEAR {
        b += (a - b) * ((NEAR - db) / (da - db));
    }
    let (Some((xa, _)), Some((xb, _))) = (view.project(a), view.project(b)) else {
        return;
    };
    let area = view.area;
    let col = |x: f32| ((x - area.x) / COL_STEP).floor();
    let last = occ.columns() as f32 - 1.0;
    let (c0, c1) = (col(xa.min(xb)).max(0.0), col(xa.max(xb)).min(last));
    if c0 > c1 {
        return;
    }
    let d = centre - view.pos;
    // Along-plane coordinate and depth where the ray through screen x meets the plane.
    let hit = |x: f32| {
        let r = view.ray(x);
        let den = r.perp_dot(along);
        (den.abs() > 1e-6).then(|| (d.perp_dot(r) / den, d.perp_dot(along) / den))
    };
    let tex = label.tex();
    let (tw, th) = (tex.width(), tex.height());
    let (z_top, z_bot) = (z + size.y * 0.5, z - size.y * 0.5);
    let (c0, c1) = (c0 as usize, c1 as usize);
    for ci in c0..=c1 {
        let xl = area.x + ci as f32 * COL_STEP;
        let xc = xl + COL_STEP * 0.5;
        let (Some((u, t)), Some((ul, _)), Some((ur, _))) = (hit(xc), hit(xl), hit(xl + COL_STEP))
        else {
            continue;
        };
        // Every label stands ≥0.01 proud of its surface, so a tight tolerance: with 0.03
        // a sign on a door showed through the wall edge the door had turned behind.
        if u.abs() > hw || t < NEAR || !view.visible(xc, t - 0.005) {
            continue;
        }
        let (y0, y1) = (view.screen_y(z_top, t), view.screen_y(z_bot, t));
        let lit = lit_at(t, fade.haze, fade.light);
        if y1 - y0 < 3.0 || lit < LOST {
            continue;
        }
        let (tint, wash) = if label.plate {
            let k = lit * lit;
            (
                Color::new(k, k, k, 1.0),
                Some(Color::new(
                    fade.fog.r,
                    fade.fog.g,
                    fade.fog.b,
                    fog_at(t, fade.haze),
                )),
            )
        } else {
            (Color::new(lit, lit, lit, lit), None)
        };
        // Texture x across this column; mirrored when `along` runs right to left on
        // screen, so it reads correctly either way.
        // Mirror-writing flips it once more.
        let s = |u: f32| {
            let s = (u + hw) / size.x;
            // Seen in a mirror (an odd number of them), it reads backwards.
            if (ur < ul) != (label.mirror != view.mirror_handed()) {
                1.0 - s
            } else {
                s
            }
        };
        let (s0, s1) = (s(ul).clamp(0.0, 1.0), s(ur).clamp(0.0, 1.0));
        let (lo, hi) = (s0.min(s1), s0.max(s1));
        for (g0, g1) in occ.gaps(ci, t, y0, y1) {
            // The texture rows for this stretch; render targets come out upside
            // down (`flip_y`), so row 0 of the source is the label's bottom.
            let (v0, v1) = ((y1 - g1) / (y1 - y0), (y1 - g0) / (y1 - y0));
            draw_texture_ex(
                tex,
                xl,
                g0,
                view.tint(tint),
                DrawTextureParams {
                    dest_size: Some(vec2(COL_STEP, g1 - g0)),
                    source: Some(Rect::new(
                        lo * tw,
                        v0 * th,
                        ((hi - lo) * tw).max(0.05),
                        ((v1 - v0) * th).max(0.05),
                    )),
                    flip_y: true,
                    ..Default::default()
                },
            );
            if let Some(w) = wash {
                draw_rectangle(xl, g0, COL_STEP, g1 - g0, w);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAP: f32 = 46.0;

    fn check(spec: &Spec, widths: &[f32]) {
        let l = layout(spec, CAP, widths);
        assert_eq!(l.size, l.size.ceil());
        let x0 = l.icon.map_or(l.pens[0].x, |r| r.x);
        let x1 = l
            .pens
            .iter()
            .zip(widths)
            .map(|(p, w)| p.x + w)
            .fold(f32::MIN, f32::max)
            .max(l.icon.map_or(f32::MIN, |r| r.right()));
        let y0 = l.pens[0].y - CAP;
        let y1 = l.pens.last().unwrap().y;
        let (y0, y1) = match l.icon {
            Some(r) => (y0.min(r.y), y1.max(r.bottom())),
            None => (y0, y1),
        };
        // Even margins all round, and inside the texture.
        assert!((x0 - (l.size.x - x1)).abs() < 1e-3, "{spec:?} {l:?}");
        assert!((y0 - (l.size.y - y1)).abs() < 1e-3, "{spec:?} {l:?}");
        assert!(x0 > 0.0 && y0 > 0.0, "{spec:?} {l:?}");
        // Every line centred on the widest.
        let mid = |i: usize| l.pens[i].x + widths[i] * 0.5;
        for i in 0..widths.len() {
            assert!((mid(i) - mid(0)).abs() < 1e-3, "{spec:?} {l:?}");
        }
        // The pictogram and the text share a centre line.
        if let Some(r) = l.icon {
            let text_mid = (l.pens[0].y - CAP + l.pens.last().unwrap().y) * 0.5;
            assert!((r.center().y - text_mid).abs() < 1e-3, "{spec:?} {l:?}");
        }
    }

    #[test]
    fn labels_are_boxed_and_centred() {
        let plate = Some(BLACK);
        check(&Spec::new(&["IT"], WHITE, plate), &[61.3]);
        check(
            &Spec::new(&["OUT OF", "ORDER"], WHITE, plate),
            &[190.2, 171.7],
        );
        check(&Spec::new(&["THE CAKE IS A LIE"], WHITE, None), &[512.9]);
        let wc = Spec {
            icon: Some(Icon::Woman),
            ..Spec::new(&["WC"], WHITE, plate)
        };
        check(&wc, &[88.4]);
    }

    #[test]
    fn long_names_wrap_at_the_middle_space() {
        let s = Spec::wrapped("MACRODATA REFINEMENT", 11, WHITE, None);
        assert_eq!(s.lines, ["MACRODATA", "REFINEMENT"]);
        let s = Spec::wrapped("LOW-HANGING FRUIT", 11, WHITE, None);
        assert_eq!(s.lines, ["LOW-HANGING", "FRUIT"]);
        let s = Spec::wrapped("IDEATION STATION", 11, WHITE, None);
        assert_eq!(s.lines, ["IDEATION", "STATION"]);
        assert_eq!(
            Spec::wrapped("BOSS ROOM", 11, WHITE, None).lines,
            ["BOSS ROOM"]
        );
        assert_eq!(
            Spec::wrapped("HOLISTIC", 5, WHITE, None).lines,
            ["HOLISTIC"]
        );
    }
}
