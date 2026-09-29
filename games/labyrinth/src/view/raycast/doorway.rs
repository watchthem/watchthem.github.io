//! Doorways with depth: an office door or a dungeon gate is a thin wall (`WALL_T`)
//! across its grid square with a door-sized opening cut through it (`OPENING` wide,
//! `DOOR_H` tall, centred), and a leaf of real thickness hinged on one jamb by the
//! room-side face. The leaf swings a quarter turn into the room and stands open
//! perpendicular to the wall, `GAP` off the jamb's lining — a door, not a slab in the
//! wall's plane — so it's always drawn, in or out of the frame.
//!
//! The doorway squares are open in `WallGrid`, so the grid march runs straight through
//! them; `door_column` then walks the doorways each column's ray crosses, nearest first,
//! and turns what it meets into `Piece`s: the wall face beside or above the opening,
//! the casing round it, the head's underside (soffit), the jamb linings, the leaf.
//! Each piece is a screen-y band of one column bounded by `Bound`s — a height on some
//! plane — so `draw_strips` can slope its edges exactly like a wall strip's. A ray that
//! passes through the opening leaves a `clip` (the soffit's far edge) that everything
//! behind the doorway is cut to, so pieces in a column never overlap on screen and the
//! three wall-pass layers can't fight. A leaf standing out in the room is an event of
//! its own: where it stops the ray, it leaves an `under` bound (its top edge) that
//! everything behind is cut to instead. Gates are a grille: round iron bars stop the
//! ray, the straps and lock box are `Look::Front` pieces drawn over whatever shows
//! between.

use super::{Doors, Edge, MeshBuf, Surf, fog_at, lit_at, quad, slab_line};
use crate::maze::{DOOR_OPENING, PANEL_INSET, Panel, WALL_T, WallGrid};
use crate::view::KEY_COLORS;
use crate::view::fx::Lighting;
use crate::view::theme::Palette;
use macroquad::prelude::*;

/// Height of a doorway's opening (the wall is 1 high).
pub(super) const DOOR_H: f32 = 0.82;
/// Width of the opening, centred in its unit wall square.
const OPENING: f32 = DOOR_OPENING;
/// Office casing: the trim round the opening on both wall faces.
const CASING: f32 = 0.05;
/// Clearance between the leaf and its frame (and the lining it opens against).
const GAP: f32 = 0.006;
/// Half the frame's depth: the wall's.
const HALF: f32 = WALL_T * 0.5;
/// How far a door swings: all the way round, to lie flat along the room-side wall
/// face (`GAP` off it). A quarter turn left it standing out into the room across half
/// the view in, so a peek from the doorway saw the leaf, not the room.
const OPEN_ANGLE: f32 = std::f32::consts::PI;
const LEAF_W: f32 = OPENING - 2.0 * GAP;
/// An office leaf laps this far past the opening's edges on both sides, as a door
/// closes against its frame: cut to the opening, the `GAP` either side showed the room
/// through a transparent sliver a few pixels wide. The lap sits behind the wall face,
/// so it's never seen.
const LAP: f32 = 2.0 * GAP + 0.01;
const OFFICE_T: f32 = 0.045;
/// Gate grille: bars (stiles included) across the leaf, each this thick both ways.
const BARS: usize = 7;
const BAR_W: f32 = 0.034;
/// Opening edges across the square.
const S0: f32 = 0.5 - OPENING * 0.5;
const S1: f32 = 0.5 + OPENING * 0.5;
/// Nothing is drawn nearer the eye than this.
const NEAR: f32 = 0.05;
/// A gate's iron straps, as height bands.
const STRAPS: [(f32, f32); 3] = [(0.05, 0.1), (0.4, 0.45), (DOOR_H - 0.07, DOOR_H - 0.02)];
/// An office door's lever handle, on both faces: (across the leaf as a fraction from
/// the hinge, out from the face, height). A round rose plate proud of the face, a neck
/// out to the lever, and the lever itself standing off the face, pointing to the hinge
/// — real boxes the rays hit, so the lever shows its depth against the leaf.
/// A `lo..hi` range, as a pair.
type Span = (f32, f32);
const HANDLE: [(Span, Span, Span); 3] = [
    ((0.83, 0.91), (0.0, 0.012), (0.36, 0.47)),
    ((0.855, 0.885), (0.0, 0.05), (0.402, 0.43)),
    ((0.66, 0.885), (0.034, 0.05), (0.402, 0.43)),
];
const BRASS: Color = Color::new(0.86, 0.72, 0.38, 1.0);
/// A gate's lock box: across the leaf (fraction from the hinge) and height.
const LOCK_U: (f32, f32) = (0.76, 0.95);
const LOCK_Z: (f32, f32) = (0.34, 0.5);

#[derive(Clone, Copy, PartialEq)]
pub(super) enum Kind {
    Office,
    Gate { key: u8 },
}

/// One doorway as this frame's rays see it.
pub(super) struct DoorFrame {
    /// Distinct per doorway (`Surf::line` of its own planes).
    id: i32,
    kind: Kind,
    /// Square centre, the way through (outer face toward the side the leaf opens into)
    /// and across the opening (from the hinge jamb toward the other).
    c: Vec2,
    n: Vec2,
    a: Vec2,
    /// The grid's `Surf` for the frame's two faces — the same as the wall beside it,
    /// so a face runs on unbroken across the doorway's edge: `side`, and the square
    /// along it (`slab_line`).
    side: i32,
    sq: i32,
    /// The leaf: hinge corner, along it (hinge → free edge), through it (its thickness,
    /// toward the outer face when shut, away from the hinge jamb when open).
    pivot: Vec2,
    e: Vec2,
    m: Vec2,
    thick: f32,
}

impl DoorFrame {
    fn new(id: i32, kind: Kind, c: Vec2, n: Vec2, a: Vec2, swing: f32) -> DoorFrame {
        let ang = swing.clamp(0.0, 1.0) * OPEN_ANGLE;
        let (cos, sin) = (ang.cos(), ang.sin());
        let sq = (c - 0.5).round();
        let side = if n.x.abs() > 0.5 { 0 } else { 1 };
        DoorFrame {
            id,
            kind,
            c,
            n,
            a,
            side,
            sq: if side == 0 { sq.x } else { sq.y } as i32,
            // On the room-side face (just proud of it): swung right round, the leaf's
            // thickness then lies out in the room, never inside the wall.
            pivot: c + a * (S0 + GAP - 0.5) + n * (HALF + GAP),
            e: a * cos + n * sin,
            m: -n * cos + a * sin,
            thick: match kind {
                Kind::Office => OFFICE_T,
                Kind::Gate { .. } => BAR_W,
            },
        }
    }

    /// An office door (`Panel`): its square is centred half a unit across from the
    /// hinge jamb point, and it opens into the room.
    fn office(id: usize, p: &Panel, swing: f32) -> DoorFrame {
        let jamb = p.hinge - p.closed * PANEL_INSET;
        DoorFrame::new(
            id as i32,
            Kind::Office,
            jamb + p.closed * 0.5,
            p.open,
            p.closed,
            swing,
        )
    }

    /// The ray's first touch of each handle part on both faces, nearest first: depth,
    /// and whether it met a face looking out from the leaf (else a side or end).
    fn handle_hits(&self, pos: Vec2, ray: Vec2) -> Vec<(f32, bool, (f32, f32))> {
        let o = pos - self.pivot;
        let (oe, de) = (o.dot(self.e), ray.dot(self.e));
        let (om, dm) = (o.dot(self.m), ray.dot(self.m));
        let mut v = Vec::new();
        for (u, out, z) in HANDLE {
            let (e0, e1) = slab(oe, de, u.0 * LEAF_W, u.1 * LEAF_W);
            // Outer face at m = 0 (handle at negative m), inner at m = thick.
            for (lo, hi) in [(-out.1, -out.0), (self.thick + out.0, self.thick + out.1)] {
                let (m0, m1) = slab(om, dm, lo, hi);
                let (t0, t1) = (e0.max(m0), e1.min(m1));
                if t1 > t0 && t0 >= NEAR {
                    v.push((t0, m0 >= e0, z));
                }
            }
        }
        crate::util::sort_by_key(&mut v, |t| t.0);
        v
    }

    /// `Surf::line` of the face a ray going `ray` enters the frame by.
    fn face_line(&self, ray: Vec2) -> i32 {
        let d = if self.side == 0 { ray.x } else { ray.y };
        slab_line(self.sq, d < 0.0)
    }

    /// The ray's run through the leaf's box: entry and exit depth, whether it enters
    /// by a broad face (else an edge), and its place along the leaf at both ends.
    fn leaf_run(&self, pos: Vec2, ray: Vec2) -> Option<(f32, f32, bool, f32, f32)> {
        let o = pos - self.pivot;
        let (oe, de) = (o.dot(self.e), ray.dot(self.e));
        let (om, dm) = (o.dot(self.m), ray.dot(self.m));
        let lap = if self.kind == Kind::Office { LAP } else { 0.0 };
        let (e0, e1) = slab(oe, de, -lap, LEAF_W + lap);
        let (m0, m1) = slab(om, dm, 0.0, self.thick);
        let (t0, t1) = (e0.max(m0), e1.min(m1));
        if t1 <= t0 || t0 < NEAR {
            return None;
        }
        Some((
            t0,
            t1,
            m0 > e0,
            (oe + de * t0) / LEAF_W,
            (oe + de * t1) / LEAF_W,
        ))
    }
}

/// The ray parameters where `o + d·t` is inside `lo..hi` (empty: `(∞, -∞)`).
pub(super) fn slab(o: f32, d: f32, lo: f32, hi: f32) -> (f32, f32) {
    if d.abs() < 1e-9 {
        return if (lo..=hi).contains(&o) {
            (f32::NEG_INFINITY, f32::INFINITY)
        } else {
            (f32::INFINITY, f32::NEG_INFINITY)
        };
    }
    let (a, b) = ((lo - o) / d, (hi - o) / d);
    (a.min(b), a.max(b))
}

/// Every doorway this frame: the office doors (`WallGrid::panels`, swung by `swing`) and
/// the dungeon's locked gates (the grid's door squares; `doors` says which are open or
/// swinging).
pub(super) fn door_frames(grid: &WallGrid, swing: &[f32], doors: Doors) -> Vec<DoorFrame> {
    let mut out: Vec<DoorFrame> = grid
        .panels
        .iter()
        .enumerate()
        .map(|(i, p)| DoorFrame::office(i, p, swing.get(i).copied().unwrap_or(0.0)))
        .collect();
    for gy in 0..grid.gh as i32 {
        for gx in 0..grid.gw as i32 {
            let Some(k) = grid.door(gx, gy) else {
                continue;
            };
            let h = (gx * 7 + gy * 13) as u32;
            let (fa, fb) = (
                if h & 1 == 0 { 1.0 } else { -1.0 },
                if h & 2 == 0 { 1.0 } else { -1.0 },
            );
            // A cell column (odd x) means the doorway joins cells above and below.
            let (n, a) = if gx % 2 == 1 {
                (vec2(0.0, fa), vec2(fb, 0.0))
            } else {
                (vec2(fa, 0.0), vec2(0.0, fb))
            };
            let open = if doors.open & (1 << k) != 0 {
                1.0
            } else {
                match doors.swing {
                    Some((sk, f)) if sk == k => f,
                    _ => 0.0,
                }
            };
            out.push(DoorFrame::new(
                1000 + k as i32,
                Kind::Gate { key: k },
                vec2(gx as f32 + 0.5, gy as f32 + 0.5),
                n,
                a,
                open,
            ));
        }
    }
    out
}

/// Drop the doorways wholly outside the view wedge (from `pos`, edge rays `dir ∓
/// plane`) — every column tests every doorway left, so most of the list is waste.
pub(super) fn cull(frames: &mut Vec<DoorFrame>, pos: Vec2, dir: Vec2, plane: Vec2) {
    let cross = |a: Vec2, b: Vec2| a.x * b.y - a.y * b.x;
    let (l, r) = (dir - plane, dir + plane);
    let turn = cross(l, r).signum();
    // The square, and the room side's floor the open leaf stands on.
    let k = 0.5 + LEAF_W;
    frames.retain(|f| {
        let corners = [(-k, -k), (k, -k), (k, k), (-k, k)].map(|(x, y)| f.c + vec2(x, y) - pos);
        let all = |out: &dyn Fn(Vec2) -> bool| corners.iter().all(|&p| out(p));
        !(all(&|p| p.dot(dir) < NEAR)
            || all(&|p| cross(l, p) * turn < 0.0)
            || all(&|p| cross(p, r) * turn < 0.0))
    });
}

/// Where an office door's name plate goes, as the leaf stands at `swing`: the middle
/// of its outer (hall) face, the leaf's direction, and that face's outward normal.
pub fn sign_mount(p: &Panel, swing: f32) -> (Vec2, Vec2, Vec2) {
    let f = DoorFrame::office(0, p, swing);
    (
        f.pivot + f.e * (LEAF_W * 0.5) + f.m * (f.thick + 0.003),
        f.e,
        f.m,
    )
}

/// A band's edge: height `z` on the plane `surf`, `depth` away at this column's left
/// edge (the right edge's depth comes from the neighbouring column on the same plane).
#[derive(Clone, Copy)]
pub(super) struct Bound {
    pub z: f32,
    pub depth: f32,
    pub surf: Surf,
}

#[derive(Clone, Copy)]
pub(super) enum Look {
    /// Wall, textured like the grid's (and fogged by its own wash quad).
    Wall { side: i32, wall_x: f32, light: f32 },
    /// Flat paint on a wall piece or strip at the same depth: its fog comes from that.
    Decal(Color),
    /// Flat, already fogged.
    Flat(Color),
    /// Flat, already fogged, over whatever the column shows behind it (gate straps).
    Front(Color),
    /// An office leaf: `u` from the hinge, `face` 0 outer / 1 inner / 2 an edge.
    Leaf {
        u: f32,
        face: u8,
        lit: (f32, f32),
        shade: f32,
    },
    /// A gate bar (straps and lock box painted across it).
    Bar {
        u: f32,
        key: u8,
        lit: (f32, f32),
        shade: f32,
    },
}

/// One band of one column: between `top` and `bot`, cut to below `clip` (the soffit
/// edge of a nearer doorway the column looks through).
#[derive(Clone, Copy)]
pub(super) struct Piece {
    pub top: Bound,
    pub bot: Bound,
    pub clip: Option<Bound>,
    /// Nothing below this edge: the top of an open leaf standing in front.
    pub under: Option<Bound>,
    pub look: Look,
}

/// Per column: its pieces (a range of the frame's piece list), and — if the ray went on
/// through a doorway — the clip for what lies behind and the roof depth (`View::roof`).
#[derive(Clone, Copy, Default)]
pub(super) struct DoorCol {
    pub pieces: (usize, usize),
    pub clip: Option<Bound>,
    pub under: Option<Bound>,
    pub roof: Option<f32>,
}

/// A doorway's wall face where a ray stopped on it: side, grid line, texture-u, light.
pub(super) type FaceHit = (i32, i32, f32, f32);

/// What `door_column` found: where the ray stopped, if in a doorway (depth, and the
/// wall face's side/line/texture-u/light when that's what stopped it); `clip`/`roof`
/// as in `DoorCol`.
#[derive(Clone, Copy, Default)]
pub(super) struct DoorOut {
    pub stop: Option<(f32, Option<FaceHit>)>,
    pub clip: Option<Bound>,
    pub under: Option<Bound>,
    pub roof: Option<f32>,
}

/// `c` lit and fogged like a wall: `lit` bright under a `wash` of fog (`lit_at`,
/// `fog_at` — the wall strip's tint, then its fog wash).
fn fogged(c: Color, k: f32, (lit, wash): (f32, f32), fog: Color) -> Color {
    let (s, w) = (lit * (1.0 - wash) * k, wash);
    Color::new(
        c.r * s + fog.r * w,
        c.g * s + fog.g * w,
        c.b * s + fog.b * w,
        1.0,
    )
}

fn scaled(c: Color, k: f32) -> Color {
    Color::new(c.r * k, c.g * k, c.b * k, 1.0)
}

/// (casing, lining) of an office door's frame: painted steel, a shade off the walls,
/// so the wooden leaf stands out against it.
const TRIM: (Color, Color) = (
    Color::new(0.58, 0.56, 0.50, 1.0),
    Color::new(0.68, 0.66, 0.60, 1.0),
);

const STONE_SOFFIT: Color = Color::new(0.24, 0.21, 0.19, 1.0);
const IRON: Color = Color::new(0.30, 0.30, 0.32, 1.0);

/// Walk the doorways `ray` crosses before `limit` (the grid hit), nearest first,
/// pushing their pieces onto `out`.
#[allow(clippy::too_many_arguments)]
pub(super) fn door_column(
    frames: &[DoorFrame],
    pos: Vec2,
    ray: Vec2,
    limit: f32,
    pal: &Palette,
    light: &Lighting,
    out: &mut Vec<Piece>,
) -> DoorOut {
    // Per doorway the ray meets: its frame (the wall's depth across the square), and
    // its leaf where that stands outside the frame — each an event, nearest first.
    let mut hits = [(0.0f32, 0.0f32, 0usize, false); 12];
    let mut nh = 0;
    for (i, f) in frames.iter().enumerate() {
        let o = pos - f.c;
        let (s0, s1) = slab(o.dot(f.a), ray.dot(f.a), -0.5, 0.5);
        let (p0, p1) = slab(o.dot(f.n), ray.dot(f.n), -HALF, HALF);
        let (t0, t1) = (s0.max(p0), s1.min(p1));
        let framed = t1 > t0 && t1 > NEAR && t0 < limit;
        if framed && nh < hits.len() {
            hits[nh] = (t0, t1, i, true);
            nh += 1;
        }
        if let Some((l0, l1, ..)) = f.leaf_run(pos, ray)
            && l0 < limit
            && (!framed || l0 < t0 - 1e-4 || l0 > t1 + 1e-4)
            && nh < hits.len()
        {
            hits[nh] = (l0, l1, i, false);
            nh += 1;
        }
    }
    let mut res = DoorOut::default();
    if nh == 0 {
        return res;
    }
    crate::util::sort_by_key(&mut hits[..nh], |t| t.0);
    let rn = ray.normalize();
    let lit_at_t = |t: f32| {
        (
            lit_at(t, pal.haze, light.at(pos + ray * t)),
            fog_at(t, pal.haze),
        )
    };
    for &(t0, t1, i, framed) in &hits[..nh] {
        let f = &frames[i];
        if !framed {
            // Behind a leaf that stops the ray, another is hidden whole: it stands no
            // taller.
            if res.under.is_none() {
                free_leaf(f, pos, ray, (t0, t1), pal, &lit_at_t, &mut res, out);
            }
            continue;
        }
        let (clip, under) = (res.clip, res.under);
        let at = |t: f32| pos + ray * t;
        let s_at = |t: f32| (at(t) - f.c).dot(f.a) + 0.5;
        let line = f.face_line(ray);
        let face_surf = Surf {
            side: f.side,
            line,
            image: pos,
            flip: (false, false),
        };
        let own = |k: i32| Surf {
            side: k,
            line: f.id,
            image: pos,
            flip: (false, false),
        };
        let band = |z0: f32, z1: f32, t: f32, surf: Surf, look: Look| Piece {
            top: Bound {
                z: z1,
                depth: t,
                surf,
            },
            bot: Bound {
                z: z0,
                depth: t,
                surf,
            },
            clip,
            under,
            look,
        };
        let office = f.kind == Kind::Office;
        let face_tint = |side: i32| if side == 0 { pal.ew_tint } else { 1.0 };
        let (tin, top_surf) = if t0 > NEAR {
            let wall_x = {
                let p = at(t0);
                if f.side == 0 { p.y } else { p.x }.rem_euclid(1.0)
            };
            let s = s_at(t0);
            let lamp = light.at(at(t0));
            let k = lit_at(t0, pal.haze, lamp) * face_tint(f.side);
            if !(S0..=S1).contains(&s) {
                // The wall beside the opening; the casing along its edge.
                if office && (S0 - CASING..S1 + CASING).contains(&s) {
                    let c = scaled(TRIM.0, k);
                    out.push(band(0.0, DOOR_H + CASING, t0, face_surf, Look::Decal(c)));
                }
                res.stop = Some((t0, Some((f.side, line, wall_x, lamp))));
                return res;
            }
            // The head over the opening, and the casing along its foot.
            out.push(band(
                DOOR_H,
                1.0,
                t0,
                face_surf,
                Look::Wall {
                    side: f.side,
                    wall_x,
                    light: lamp,
                },
            ));
            if office {
                let c = scaled(TRIM.0, k);
                out.push(band(DOOR_H, DOOR_H + CASING, t0, face_surf, Look::Decal(c)));
            }
            (t0, face_surf)
        } else {
            // Standing in the doorway.
            (NEAR, own(40))
        };
        let ds = ray.dot(f.a);
        let s_in = s_at(tin);
        let t_rev = if ds > 1e-6 {
            tin + (S1 - s_in) / ds
        } else if ds < -1e-6 {
            tin + (S0 - s_in) / ds
        } else {
            f32::INFINITY
        };
        let t_stop = t1.min(t_rev);
        let soffit = |to_t: f32, surf: Surf| {
            let lit = lit_at_t((tin + to_t) * 0.5);
            let c = if office {
                scaled(TRIM.1, 0.62)
            } else {
                STONE_SOFFIT
            };
            Piece {
                top: Bound {
                    z: DOOR_H,
                    depth: tin,
                    surf: top_surf,
                },
                bot: Bound {
                    z: DOOR_H,
                    depth: to_t,
                    surf,
                },
                clip,
                under,
                look: Look::Flat(fogged(c, 1.0, lit, pal.fog)),
            }
        };
        let leaf = f.leaf_run(pos, ray);
        // The handle, wherever it stands before the leaf, a jamb or the far side.
        if office {
            let before = leaf.map_or(t_stop, |l| l.0.min(t_stop));
            for (t, face_on, (z0, z1)) in f.handle_hits(pos, ray) {
                if t >= tin - 1e-4 && t < before {
                    let shade = if face_on { 1.0 } else { 0.7 };
                    let c = fogged(BRASS, shade, lit_at_t(t), pal.fog);
                    out.push(band(z0, z1, t, own(50 + face_on as i32), Look::Front(c)));
                }
            }
        }
        // The leaf.
        if let Some((l0, l1, broad, u0, u1)) = leaf
            && l0 >= tin - 1e-4
            && l0 <= t_stop + 1e-4
        {
            match f.kind {
                Kind::Office => {
                    let (face, n) = if broad {
                        (if ray.dot(f.m) > 0.0 { 1 } else { 0 }, f.m)
                    } else {
                        (2, f.e)
                    };
                    let shade = (0.72 + 0.28 * n.dot(rn).abs()) * if face == 2 { 0.9 } else { 1.0 };
                    let surf = own(30 + face as i32);
                    out.push(soffit(l0, surf));
                    out.push(band(
                        0.0,
                        DOOR_H,
                        l0,
                        surf,
                        Look::Leaf {
                            u: u0.clamp(0.0, 1.0),
                            face,
                            lit: lit_at_t(l0),
                            shade,
                        },
                    ));
                    res.stop = Some((l0, None));
                    return res;
                }
                Kind::Gate { key } => {
                    if let Some((t, u, shade)) = bar_hit(f, pos, rn, (l0, l1), (u0, u1)) {
                        let surf = own(30);
                        out.push(soffit(t, surf));
                        out.push(band(
                            0.0,
                            DOOR_H,
                            t,
                            surf,
                            Look::Bar {
                                u,
                                key,
                                lit: lit_at_t(t),
                                shade,
                            },
                        ));
                        res.stop = Some((t, None));
                        return res;
                    }
                    // Between the bars: the lock box and straps cross in front (pushed
                    // top layer first — a column's pieces paint in reverse).
                    let lit = lit_at_t(l0);
                    let surf = own(31);
                    let u = u0.clamp(0.0, 1.0);
                    if (LOCK_U.0..LOCK_U.1).contains(&u) {
                        for (z0, z1, c) in lock_bands(u, key).into_iter().rev() {
                            let c = fogged(c, 1.0, lit, pal.fog);
                            out.push(band(z0, z1, l0, surf, Look::Front(c)));
                        }
                    }
                    for (z0, z1) in STRAPS {
                        let c = fogged(IRON, 0.8, lit, pal.fog);
                        out.push(band(z0, z1, l0, surf, Look::Front(c)));
                    }
                }
            }
        }
        if t_rev < t1 {
            // The jamb lining.
            let side = if f.a.x.abs() > 0.5 { 0 } else { 1 };
            let surf = own(20 + (ds > 0.0) as i32);
            out.push(soffit(t_rev, surf));
            let look = match office {
                true => {
                    let lit = lit_at_t(t_rev);
                    Look::Flat(fogged(TRIM.1, face_tint(side), lit, pal.fog))
                }
                false => {
                    let p = at(t_rev);
                    Look::Wall {
                        side,
                        wall_x: if side == 0 { p.y } else { p.x }.rem_euclid(1.0),
                        light: light.at(p),
                    }
                }
            };
            out.push(band(0.0, DOOR_H, t_rev, surf, look));
            res.stop = Some((t_rev, None));
            return res;
        }
        // Through: what lies beyond shows only under the soffit's far edge.
        let exit = own(10);
        out.push(soffit(t1, exit));
        res.clip = Some(Bound {
            z: DOOR_H,
            depth: t1,
            surf: exit,
        });
        res.roof = res.roof.or(Some(t1));
    }
    res
}

/// A leaf standing outside its frame (open, or on its way) that the ray meets at
/// `l0..l1`: an office leaf (and its handle in front) stops the ray — what lies behind
/// shows only above its top edge (`DoorOut::under`); a gate stops it on a bar, else
/// leaves its straps and lock box across the view.
#[allow(clippy::too_many_arguments)]
fn free_leaf(
    f: &DoorFrame,
    pos: Vec2,
    ray: Vec2,
    (l0, l1): (f32, f32),
    pal: &Palette,
    lit_at_t: &dyn Fn(f32) -> (f32, f32),
    res: &mut DoorOut,
    out: &mut Vec<Piece>,
) {
    let Some((_, _, broad, u0, u1)) = f.leaf_run(pos, ray) else {
        return;
    };
    let own = |k: i32| Surf {
        side: k,
        line: f.id,
        image: pos,
        flip: (false, false),
    };
    let (clip, under) = (res.clip, res.under);
    let band = |z0: f32, z1: f32, t: f32, surf: Surf, look: Look| Piece {
        top: Bound {
            z: z1,
            depth: t,
            surf,
        },
        bot: Bound {
            z: z0,
            depth: t,
            surf,
        },
        clip,
        under,
        look,
    };
    let rn = ray.normalize();
    let (t, surf, look) = match f.kind {
        Kind::Office => {
            for (t, face_on, (z0, z1)) in f.handle_hits(pos, ray) {
                if t < l0 {
                    let shade = if face_on { 1.0 } else { 0.7 };
                    let c = fogged(BRASS, shade, lit_at_t(t), pal.fog);
                    out.push(band(z0, z1, t, own(50 + face_on as i32), Look::Front(c)));
                }
            }
            let (face, n) = if broad {
                (if ray.dot(f.m) > 0.0 { 1 } else { 0 }, f.m)
            } else {
                (2, f.e)
            };
            let shade = (0.72 + 0.28 * n.dot(rn).abs()) * if face == 2 { 0.9 } else { 1.0 };
            let look = Look::Leaf {
                u: u0.clamp(0.0, 1.0),
                face,
                lit: lit_at_t(l0),
                shade,
            };
            (l0, own(30 + face as i32), look)
        }
        Kind::Gate { key } => match bar_hit(f, pos, rn, (l0, l1), (u0, u1)) {
            Some((t, u, shade)) => {
                let look = Look::Bar {
                    u,
                    key,
                    lit: lit_at_t(t),
                    shade,
                };
                (t, own(30), look)
            }
            None => {
                let lit = lit_at_t(l0);
                let u = u0.clamp(0.0, 1.0);
                if (LOCK_U.0..LOCK_U.1).contains(&u) {
                    for (z0, z1, c) in lock_bands(u, key).into_iter().rev() {
                        let c = fogged(c, 1.0, lit, pal.fog);
                        out.push(band(z0, z1, l0, own(31), Look::Front(c)));
                    }
                }
                for (z0, z1) in STRAPS {
                    let c = fogged(IRON, 0.8, lit, pal.fog);
                    out.push(band(z0, z1, l0, own(31), Look::Front(c)));
                }
                return;
            }
        },
    };
    out.push(band(0.0, DOOR_H, t, surf, look));
    res.under = Some(Bound {
        z: DOOR_H,
        depth: t,
        surf,
    });
}

/// The first gate bar the ray's run through the leaf box meets: depth, place across the
/// leaf, and a round bar's shading at the ray's offset from its axis.
fn bar_hit(
    f: &DoorFrame,
    pos: Vec2,
    rn: Vec2,
    (t0, t1): (f32, f32),
    (u0, u1): (f32, f32),
) -> Option<(f32, f32, f32)> {
    let bw = BAR_W / LEAF_W;
    let (lo, hi) = (u0.min(u1), u0.max(u1));
    let mut best: Option<(f32, f32, f32)> = None;
    for k in 0..BARS {
        let c = bw * 0.5 + k as f32 * (1.0 - bw) / (BARS - 1) as f32;
        let (b0, b1) = (c - bw * 0.5, c + bw * 0.5);
        if b1 < lo || b0 > hi {
            continue;
        }
        // Where the run first touches this bar.
        let edge = if (b0..=b1).contains(&u0) {
            u0
        } else if u1 > u0 {
            b0
        } else {
            b1
        };
        let frac = if (u1 - u0).abs() > 1e-6 {
            ((edge - u0) / (u1 - u0)).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let t = t0 + (t1 - t0) * frac;
        if best.is_some_and(|b| b.0 <= t) {
            continue;
        }
        // Round bar: lit by how squarely the ray meets it at its offset from the axis.
        let axis = f.pivot + f.e * (c * LEAF_W) + f.m * (f.thick * 0.5);
        let rel = axis - pos;
        let off = (rel.x * rn.y - rel.y * rn.x) / (BAR_W * 0.5);
        let q = off.clamp(-1.0, 1.0);
        let shade =
            0.35 + 0.65 * (1.0 - q * q).sqrt() + 0.25 * (-(q + 0.4) * (q + 0.4) * 12.0).exp();
        best = Some((t, c, shade));
    }
    best
}

/// A gate's lock box in the key's colour, seen at `u` across the leaf: (z0, z1, colour).
fn lock_bands(u: f32, key: u8) -> [(f32, f32, Color); 2] {
    let rim = (u - LOCK_U.0).min(LOCK_U.1 - u) < 0.03;
    let k = KEY_COLORS[key as usize % KEY_COLORS.len()];
    let dark = Color::new(0.16, 0.15, 0.14, 1.0);
    let body = if rim { scaled(k, 0.55) } else { k };
    [
        (LOCK_Z.0, LOCK_Z.1, body),
        // Keyhole.
        (
            0.4,
            0.44,
            if (u - (LOCK_U.0 + LOCK_U.1) * 0.5).abs() < 0.02 {
                dark
            } else {
                body
            },
        ),
    ]
}

/// One column of an office leaf or gate bar between column edges `xs` (the leaf's
/// floor/ceiling lines there: `edges`), inside `top..bot` per edge.
#[allow(clippy::too_many_arguments)]
pub(super) fn draw_leaf(
    buf: &mut MeshBuf,
    xs: [f32; 2],
    edges: [Edge; 2],
    top: [f32; 2],
    bot: [f32; 2],
    look: Look,
    fog: Color,
) {
    let mut span = |z0: f32, z1: f32, col: Color| {
        let t = [0, 1].map(|i| edges[i].y(z1).max(top[i]));
        let b = [0, 1].map(|i| edges[i].y(z0).min(bot[i]));
        quad(buf, xs, t, b, col, [Vec2::ZERO; 4]);
    };
    match look {
        Look::Leaf {
            u,
            face,
            lit,
            shade,
        } => {
            let k = |c: Color| fogged(c, shade, lit, fog);
            let (slab, frame) = (
                Color::new(0.52, 0.36, 0.20, 1.0),
                Color::new(0.36, 0.24, 0.13, 1.0),
            );
            if face == 2 {
                // The edge: a thin strip of lighter end grain / bare metal.
                span(0.0, DOOR_H, k(scaled(slab, 1.15)));
                return;
            }
            if !(0.07..=0.93).contains(&u) {
                span(0.0, DOOR_H, k(frame));
                return;
            }
            // Wood grain: the veneer's tone wanders across the leaf in fine vertical
            // streaks.
            let grain =
                1.0 + 0.07 * (u * 97.0).sin() * (u * 13.0).cos() + 0.04 * (u * 211.0 + 1.3).sin();
            span(0.0, DOOR_H, k(scaled(slab, grain)));
            span(DOOR_H - 0.05, DOOR_H, k(frame));
            span(0.0, 0.06, k(frame));
            // Two raised panels between the stiles: the field a shade lighter, lit from
            // above — a bright bevel on top, a shadowed one below, and the vertical
            // bevels light on the hinge side, dark on the other.
            const PU: (f32, f32) = (0.2, 0.8);
            const BEVEL: f32 = 0.025;
            if (PU.0..=PU.1).contains(&u) {
                for (z0, z1) in [(0.1, 0.38), (0.47, 0.74)] {
                    if u < PU.0 + BEVEL * 1.6 {
                        span(z0, z1, k(scaled(slab, grain * 1.18)));
                    } else if u > PU.1 - BEVEL * 1.6 {
                        span(z0, z1, k(scaled(slab, grain * 0.72)));
                    } else {
                        span(z0, z1, k(scaled(slab, grain * 1.08)));
                        span(z1 - BEVEL, z1, k(scaled(slab, grain * 1.25)));
                        span(z0, z0 + BEVEL, k(scaled(slab, grain * 0.68)));
                    }
                }
            }
            // The lever's shadow on the face just under it.
            if (0.68..0.9).contains(&u) {
                span(0.385, 0.402, k(scaled(slab, grain * 0.55)));
            }
        }
        Look::Bar { u, key, lit, shade } => {
            span(0.0, DOOR_H, fogged(IRON, shade, lit, fog));
            for (z0, z1) in STRAPS {
                span(z0, z1, fogged(IRON, shade * 0.85, lit, fog));
            }
            if (LOCK_U.0..LOCK_U.1).contains(&u) {
                for (z0, z1, c) in lock_bands(u, key) {
                    span(z0, z1, fogged(c, shade, lit, fog));
                }
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(swing: f32) -> DoorFrame {
        let (c, n, a) = (vec2(3.5, 4.5), vec2(0.0, 1.0), vec2(1.0, 0.0));
        DoorFrame::new(0, Kind::Office, c, n, a, swing)
    }

    /// Shut, the leaf stands across the opening at the room-side face of the wall;
    /// open, it has swung right round to lie flat along that face beside the doorway,
    /// `GAP` off it — out of the way of a look in, never inside the wall.
    #[test]
    fn leaf_shuts_the_opening_and_opens_flat_against_the_wall() {
        let shut = frame(0.0);
        let down = vec2(0.0, 1.0);
        let (t0, _, broad, u, _) = shut.leaf_run(vec2(3.5, 3.0), down).unwrap();
        assert!(broad && (u - 0.5).abs() < 0.05);
        assert!((t0 - (1.5 + HALF + GAP - OFFICE_T)).abs() < 1e-4, "{t0}");

        // No crack at either jamb: straight in right beside each edge of the opening,
        // the shut leaf is there.
        let jamb = 3.0 + S0;
        for x in [jamb + GAP * 0.5, 3.0 + S1 - GAP * 0.5] {
            assert!(shut.leaf_run(vec2(x, 3.0), down).is_some(), "crack at {x}");
        }

        let open = frame(1.0);
        // Straight in through the opening, and at a slant across it: no leaf.
        assert!(open.leaf_run(vec2(3.5, 3.0), down).is_none());
        assert!(open.leaf_run(vec2(3.5, 3.0), vec2(-0.15, 1.0)).is_none());
        let jamb = 3.0 + S0;
        let face = 4.5 + HALF;
        for (u, w) in [
            (0.0, 0.0),
            (LEAF_W, 0.0),
            (0.0, open.thick),
            (LEAF_W, open.thick),
        ] {
            let p = open.pivot + open.e * u + open.m * w;
            assert!(
                p.x <= jamb + GAP + 1e-4 && p.x >= jamb + GAP - LEAF_W - 1e-4,
                "{p}"
            );
            assert!(
                p.y >= face + GAP - 1e-4 && p.y <= face + GAP + open.thick + 1e-4,
                "{p}"
            );
        }
    }
}
