//! Pseudo-3D first-person view: a DDA ray per screen column against the maze's dense
//! `WallGrid`, drawn as textured vertical strips sampled from the active `Theme`.
//! Floor and ceiling are two flat gradient rects (theme palette), not per-pixel cast.
//!
//! Mirror mode: a ray hitting a mirror face reflects and keeps marching — off further
//! mirrors too, up to `MAX_BOUNCES` — so the reflected corridor renders behind the
//! glass, darkened and cooled a little more per bounce, with the bot's own reflection
//! standing in it (once per bounce it's visible at), and highlight streaks across the
//! glass, so it reads as a mirror a viewer can spot before the bot walks into it.
//!
//! `draw` returns the frame's `View` (camera + per-column wall depth) so later
//! billboard passes — `fx` motes — project and depth-test against the same scene.
//!
//! Dungeon mode: a locked doorway is an iron-barred gate in a stone frame, its lock box
//! in its key's colour — the corridor beyond shows between the bars — that swings open
//! against its jamb once the bot has unlocked it (`doors_open`, `door_swing`).
//!
//! Tower mode: an office doorway is a wall square with a door-sized opening cut through
//! it — casing round the opening on both faces, lined jambs and head — and a thick leaf
//! hinged inside it (`DoorFrame`), all intersected per column after the grid march
//! (`door_column`), so a leaf stands at any angle mid-swing. `swing` (0 shut .. 1 open,
//! flat against the jamb lining) comes from the view, which animates it. Sealed doors
//! are grey with no handle; openable ones are wood with one.
//!
//! Every mode's exit is dressing, not light: the office lift and the dungeon stair
//! (`decor`), the forest campfire, the garden's gate onto open country.
//!
//! Outdoors, foliage is billboards from the theme's `Sprites` atlas: Forest spruces
//! (`draw_trees`, painter's order, no wall faces at all) and Garden hedge leaf clumps
//! (`draw_clumps`, depth-tested per column over each hedge's recessed dark core).

use crate::maze::{STAIR_FLIGHT, WallGrid, Well};
use crate::view::fog;
use crate::view::fx::Lighting;
use crate::view::theme::{
    BLOOM_ALTS, BLOOM_KINDS, CLUMP_SPAN, CLUMPS, Ceiling, GROUND_PERIOD, Ground, Palette, Sprites,
    TREE_FOOT, TREE_SPAN, Theme, Tile, bloom_variant,
};
use macroquad::prelude::*;

mod doorway;
pub use doorway::sign_mount;
use doorway::{Bound, DOOR_H, DoorCol, DoorOut, Look, Piece, door_column, door_frames};

/// One ray per two horizontal pixels — ~450 columns at 900px.
pub const COL_STEP: f32 = 2.0;

/// Most mirrors one ray bounces off; whatever it strikes after that is drawn as a plain
/// surface. Facing mirrors would otherwise recurse without end, and by the fifth image
/// the stacked glass tint has faded it into the haze anyway.
const MAX_BOUNCES: usize = 5;

struct RayHit {
    /// Distance along the ray to the final wall, every bounce's leg summed — its depth
    /// in the unfolded (straight-ray) reflected world.
    perp: f32,
    side: i32,
    /// `Surf::line` of the face struck.
    line: i32,
    wall_x: f32,
    /// The mirrors the ray bounced off on the way, nearest first; `bounces` are set.
    mirrors: [MirrorHit; MAX_BOUNCES],
    bounces: usize,
    /// 0..1 hash of the grid square struck — per-bush variation in the Garden.
    square: f32,
    /// The grid square struck (after the bounces, if any).
    map: IVec2,
}

impl RayHit {
    fn mirrors(&self) -> &[MirrorHit] {
        &self.mirrors[..self.bounces]
    }

    /// Depth of the first glass (or the wall, if none): nothing in the real world shows
    /// past it, so billboards depth-test against this, not the reflected wall.
    fn front(&self) -> f32 {
        self.mirrors().first().map_or(self.perp, |m| m.dist)
    }
}

#[derive(Clone, Copy, Default)]
struct MirrorHit {
    /// Distance along the ray to this glass, earlier bounces' legs included.
    dist: f32,
    /// Texture-u across the mirror face (0..1) — places the streaks.
    u: f32,
    /// The camera mirrored across this glass and every one before it: the camera that
    /// sees, in the real world, what this glass shows (`View::mirrored`). Mirrored once,
    /// it's also where the bot's reflection stands in the unfolded world.
    image: Vec2,
    /// Which of the camera's direction components that chain of reflections negates.
    flip: (bool, bool),
    /// The glass faces along x (an east/west face).
    flips_x: bool,
    /// The glass's grid line (x if `flips_x`, else y) and which side of it the ray
    /// arrives from — the only side of the world this glass can show.
    plane: f32,
    side: f32,
    /// Which mirror face: its square and `FACE_N` index.
    key: (IVec2, u8),
}

/// `v` with the components `flip` names negated.
fn flip(v: Vec2, flip: (bool, bool)) -> Vec2 {
    vec2(
        if flip.0 { -v.x } else { v.x },
        if flip.1 { -v.y } else { v.y },
    )
}

/// The camera a frame was cast from, plus its per-column wall depths — what billboards
/// (keys, the bot's reflection, fx motes) need to project and depth-test against.
pub struct View {
    pub area: Rect,
    pub pos: Vec2,
    dir: Vec2,
    /// The bot's gait (`Anim::stride`), for its reflection.
    stride: f32,
    plane: Vec2,
    /// Screen y of the eye's level (the camera never pitches, so it's fixed).
    pub horizon: f32,
    /// Eye height in world units (floor 0, ceiling 1): 0.5 plus the walking head-bob.
    /// A real camera height, so near things bob more than far ones.
    eye: f32,
    /// Open country (the Garden): the horizon haze that distant things pale into, on
    /// top of the palette fog (`View::fog`).
    haze: Option<[f32; 3]>,
    zbuf: Vec<f32>,
    /// Per column, `(depth, y)`: past `depth` nothing shows above screen `y` — the
    /// head of an office doorway the column looks through (its underside's far edge).
    /// `(∞, -∞)` where there's none.
    roof: Vec<(f32, f32)>,
    /// Per column, `(depth, y)`: past `depth` nothing shows below screen `y` — the top
    /// of an open office leaf (or a gate bar) standing out in front. `(∞, ∞)` where
    /// there's none.
    under: Vec<(f32, f32)>,
    /// Per column: the ray went into the stairwell's flight (`Well`) — the only
    /// columns its set dressing shows in.
    portal: Vec<bool>,
    /// The stairwell's flight, if the maze has one.
    well: Option<Well>,
    /// Set on a mirror's camera handed to `Props::reflect`: only what lies in its depth
    /// window shows, tinted like the glass.
    glass: Option<Glass>,
}

/// A mirrored `View`'s window onto the reflected world: per column, only depths past
/// `znear` (the glass) and before `View::zbuf` (whatever the glass shows behind) are
/// visible, in columns `c0..=c1` — one glass run. `tint` is that bounce level's
/// `glass_tint`; `handed` says the image is left-right reversed (an odd number of
/// reflections), so writing in it must read backwards.
struct Glass {
    znear: Vec<f32>,
    tint: [f32; 3],
    handed: bool,
    c0: usize,
    c1: usize,
}

impl View {
    /// The ray through screen x `sx`, with unit component along the view direction —
    /// so a hit's ray parameter is its perpendicular depth.
    pub fn ray(&self, sx: f32) -> Vec2 {
        self.dir + self.plane * (2.0 * (sx - self.area.x) / self.area.w - 1.0)
    }

    /// Screen x and view depth of a world point, if it's in front of the camera.
    pub fn project(&self, p: Vec2) -> Option<(f32, f32)> {
        let rel = p - self.pos;
        let inv_det = 1.0 / (self.plane.x * self.dir.y - self.dir.x * self.plane.y);
        let tx = inv_det * (self.dir.y * rel.x - self.dir.x * rel.y);
        let depth = inv_det * (-self.plane.y * rel.x + self.plane.x * rel.y);
        (depth > 0.15).then(|| (self.area.x + self.area.w * 0.5 * (1.0 + tx / depth), depth))
    }

    /// View depth of a world point (negative behind the camera) — `project`'s depth,
    /// without its near-plane cut.
    pub fn depth(&self, p: Vec2) -> f32 {
        let rel = p - self.pos;
        let inv_det = 1.0 / (self.plane.x * self.dir.y - self.dir.x * self.plane.y);
        inv_det * (-self.plane.y * rel.x + self.plane.x * rel.y)
    }

    /// Pixels per world unit at `depth` — same on both axes, so nothing stretches.
    pub fn px_per_unit(&self, depth: f32) -> f32 {
        self.area.h / depth
    }

    /// Screen y of world height `z` at `depth`.
    pub fn screen_y(&self, z: f32, depth: f32) -> f32 {
        self.horizon + (self.eye - z) * self.px_per_unit(depth)
    }

    /// Whether something at `depth` in screen column `sx` is in front of the wall there.
    pub fn visible(&self, sx: f32, depth: f32) -> bool {
        let ci = ((sx - self.area.x) / COL_STEP) as isize;
        ci >= 0
            && (ci as usize) < self.zbuf.len()
            && depth < self.zbuf[ci as usize]
            && self
                .glass
                .as_ref()
                .is_none_or(|g| depth > g.znear[ci as usize])
    }

    /// Topmost screen y something `depth` away in column `sx` can show at: below an
    /// office doorway's head when it's seen through the doorway, else unbounded.
    pub fn roof(&self, sx: f32, depth: f32) -> f32 {
        let ci = ((sx - self.area.x) / COL_STEP) as isize;
        match self.roof.get(ci.max(0) as usize) {
            Some(&(d, y)) if ci >= 0 && depth > d => y,
            _ => f32::NEG_INFINITY,
        }
    }

    /// Per column, the `(depth, y)` doorway heads (`roof`) — for `Occluders`.
    pub fn roofs(&self) -> &[(f32, f32)] {
        &self.roof
    }

    /// Lowest screen y something `depth` away in column `sx` can show at: above the
    /// top of an open leaf standing in front of it, else unbounded.
    pub fn floor_clip(&self, sx: f32, depth: f32) -> f32 {
        let ci = ((sx - self.area.x) / COL_STEP) as isize;
        match self.under.get(ci.max(0) as usize) {
            Some(&(d, y)) if ci >= 0 && depth > d => y,
            _ => f32::INFINITY,
        }
    }

    /// Whether column `sx` looks into the stairwell's flight (through its portal, or
    /// from inside).
    pub fn through_portal(&self, sx: f32) -> bool {
        let ci = ((sx - self.area.x) / COL_STEP) as isize;
        ci >= 0 && self.portal.get(ci as usize).copied().unwrap_or(false)
    }

    /// Whether world point `p`, seen in column `sx`, lies past the stairwell's arch in
    /// a column that looks into the flight: whatever really stands there (the floor
    /// behind the wall) isn't what the portal shows — only the flight's own dressing.
    pub fn behind_portal(&self, p: Vec2, sx: f32) -> bool {
        self.through_portal(sx) && self.well.is_some_and(|w| w.local(p).0 > 0.0)
    }

    /// Per column, the `(depth, y)` tops of open leaves (`floor_clip`) — for
    /// `Occluders`.
    pub fn unders(&self) -> &[(f32, f32)] {
        &self.under
    }

    /// A camera looking into a mirror (`Props::reflect`), not at the real world.
    pub fn reflected(&self) -> bool {
        self.glass.is_some()
    }

    /// Seen through an odd number of mirrors: the picture is left-right reversed, so
    /// text in it reads backwards.
    pub fn mirror_handed(&self) -> bool {
        self.glass.as_ref().is_some_and(|g| g.handed)
    }

    /// The glass colour things seen through this view are multiplied by ([1; 3] in the
    /// real world). Like the wall strips, only a surface's own colour takes it, never
    /// the fog washed over it.
    pub fn glass_tint(&self) -> [f32; 3] {
        self.glass.as_ref().map_or([1.0; 3], |g| g.tint)
    }

    /// `c` as seen through this view's glass (unchanged in the real world).
    pub fn tint(&self, c: Color) -> Color {
        match &self.glass {
            Some(g) => Color::new(c.r * g.tint[0], c.g * g.tint[1], c.b * g.tint[2], c.a),
            None => c,
        }
    }

    /// A mirror's depth window over its whole run: the glass's nearest depth and the
    /// farthest depth anything behind it shows at. `None` for the real camera.
    pub fn window(&self) -> Option<(f32, f32)> {
        let g = self.glass.as_ref()?;
        let near = g.znear[g.c0..=g.c1]
            .iter()
            .fold(f32::INFINITY, |m, &d| m.min(d));
        let far = self.zbuf[g.c0..=g.c1].iter().fold(0.0f32, |m, &d| m.max(d));
        Some((near, far))
    }

    /// Screen x range anything can show in: the whole view, or a mirror's glass run.
    pub fn span(&self) -> (f32, f32) {
        match &self.glass {
            Some(g) => (
                self.area.x + g.c0 as f32 * COL_STEP,
                self.area.x + (g.c1 + 1) as f32 * COL_STEP,
            ),
            None => (self.area.x, self.area.x + self.area.w),
        }
    }

    /// The camera mirrored across glass (onto `image`, its direction's `flip`ped
    /// components negated): projecting a real point with it lands where the camera sees
    /// that point's reflection — the unfolded reflected world, with the glass as a
    /// window onto it. A chain of mirrors composes: each bounce reflects the previous
    /// image across the next glass, so one such camera covers any bounce level.
    fn mirrored(&self, image: Vec2, flip_dir: (bool, bool)) -> View {
        View {
            area: self.area,
            pos: image,
            dir: flip(self.dir, flip_dir),
            stride: self.stride,
            plane: flip(self.plane, flip_dir),
            horizon: self.horizon,
            eye: self.eye,
            haze: self.haze,
            zbuf: Vec::new(),
            roof: Vec::new(),
            under: Vec::new(),
            portal: Vec::new(),
            well: None,
            glass: None,
        }
    }

    /// What anything at depth `d` fades toward: the palette fog, paling far out in open
    /// country toward the horizon haze exactly as the ground does (`ground_far`) — so a
    /// long mirror tunnel's far end melts into the horizon band around it instead of
    /// standing out as a dark slot.
    fn fog(&self, pal: &Palette, d: f32) -> [f32; 3] {
        let fog = [pal.fog.r, pal.fog.g, pal.fog.b];
        match self.haze {
            Some(h) => mix(fog, h, smoothstep(10.0, 120.0, d) * 0.9),
            None => fog,
        }
    }

    /// A world point in camera space: (lateral in camera-plane units, depth). Screen x
    /// is `area.x + area.w / 2 * (1 + lateral / depth)`.
    fn cam(&self, p: Vec2) -> Vec2 {
        let rel = p - self.pos;
        let inv_det = 1.0 / (self.plane.x * self.dir.y - self.dir.x * self.plane.y);
        vec2(
            inv_det * (self.dir.y * rel.x - self.dir.x * rel.y),
            inv_det * (-self.plane.y * rel.x + self.plane.x * rel.y),
        )
    }
}

/// Per-column facts from the wall pass that the Garden's leaf-clump pass depth-tests
/// against, beyond `View::zbuf`: where the nearest wall's top edge is on screen (a
/// farther clump can still show above it), and which hedge face the column struck.
struct HedgeCols {
    ztop: Vec<f32>,
    face: Vec<Option<(IVec2, u8)>>,
}

/// Locked dungeon gates as the view shows them: `open` bit `k` = gate `k` unlocked
/// (standing open against its jamb), `swing` the one swinging open now and how far.
/// Rays pass their squares; the gates themselves are `doorway` frames.
#[derive(Clone, Copy, Default)]
struct Doors {
    open: u32,
    swing: Option<(u8, f32)>,
}

/// Most quads one column's doorway pieces add to a buffer.
const DOOR_QUADS: usize = 24;

/// What the maze holds besides walls, as the camera should show it this frame.
pub struct Props<'a> {
    /// Bit `k`: key `k`'s door has been unlocked (no longer solid).
    pub doors_open: u32,
    /// The locked door being unlocked right now and how far it has swung open, 0..1.
    pub door_swing: Option<(u8, f32)>,
    /// Office doors, indexed like `WallGrid::panels`: how far each has swung open, 0..1.
    pub swing: &'a [f32],
    /// Forest: world points of things that must sort among the trees (the campfire,
    /// critters). Trees nearer than the farthest of them are left out, for the caller to
    /// interleave with them via `draw_trees_between` — painter's order is the trees'
    /// only occlusion.
    pub amid: &'a [Vec2],
    /// Draws the world's billboards (set dressing, critters) through a mirror: called
    /// once per glass run, up to `DECOR_BOUNCES` deep, with that glass's mirrored camera
    /// (`View::reflected`) — draw exactly as for the real camera; its `visible` keeps to
    /// the glass and the depth window behind it, `tint` gives the glass colour.
    pub reflect: Option<&'a dyn Fn(&View)>,
    /// Looking down (Dungeon's stairwell descent): the horizon rises by this fraction
    /// of the view's height — a y-shear, the classic raycaster pitch.
    pub look_down: f32,
}

/// What one screen column's ray found, before anything is drawn. `Wall` carries its
/// mirror chain inline and dwarfs the others, but nearly every column is one — boxing
/// it would only add ~450 allocations a frame.
#[allow(clippy::large_enum_variant)]
enum Col {
    /// Forest: no wall faces at all (the trees stand in for them).
    Open,
    /// Stopped inside an office doorway (`door_column`) at depth `t`: on the wall face
    /// beside the opening (`face`: side, grid line, texture-u, light — drawn as a wall
    /// strip), or on the jamb lining or the leaf (a door piece).
    Door {
        t: f32,
        face: Option<doorway::FaceHit>,
    },
    Wall(RayHit),
}

/// What a column's ray finally struck: depth, its top edge on screen, and the hedge
/// face (square, `FACE_N` index) if it's a clipped hedge.
type Seen = (f32, f32, Option<(IVec2, u8)>);

/// A run of adjacent columns showing the same mirror face at the same bounce `level`
/// (1 = seen directly, 2 = in the first mirror, ...) through the same camera image.
struct GlassRun {
    c0: usize,
    c1: usize,
    level: usize,
    key: (IVec2, u8),
    image: Vec2,
    flip: (bool, bool),
    flips_x: bool,
    /// The glass's grid line (x if `flips_x`, else y) and which side of it the ray
    /// arrives from.
    plane: f32,
    side: f32,
}

impl GlassRun {
    /// Whether a world point lies on the ray's side of the glass — the only part of
    /// the real world the mirror can show.
    fn in_front(&self, p: Vec2) -> bool {
        let c = if self.flips_x { p.x } else { p.y };
        (c - self.plane) * self.side > 1e-4
    }
}

/// Silvered glass: a reflection is this much darker and cooler...
const GLASS: [f32; 3] = [0.72, 0.80, 0.88];
/// ...and each further bounce darker and cooler again, a little less so each time than
/// the first — facing mirrors fade into a blue-grey tunnel, so the depth reads, without
/// the fifth image going black.
const GLASS_DEEPER: [f32; 3] = [0.84, 0.88, 0.93];

/// The tint of what's seen through `level` panes of glass (0 = the real world).
fn glass_tint(level: usize) -> [f32; 3] {
    if level == 0 {
        return [1.0; 3];
    }
    let k = level as i32 - 1;
    [0, 1, 2].map(|i| GLASS[i] * GLASS_DEEPER[i].powi(k))
}

/// Past this many bounces a reflection skips its ground mesh (the flat backdrop still
/// fills it) — deep images are small, dim and far, and each level's pass costs as much
/// as the first.
const GROUND_BOUNCES: usize = 2;
/// Past this many bounces a reflected hedge is drawn as a plain face (the look the
/// real ones ease into at range) instead of core + leaf-clump pass.
const CLUMP_BOUNCES: usize = 2;
/// Past this many bounces a reflection leaves out the set dressing and critters
/// (`Props::reflect`).
const DECOR_BOUNCES: usize = 2;

/// `pos` is in grid units, `yaw` radians (+x east, +y south), `bob` the head-bob as a
/// change in eye height (world units, wall height = 1).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    grid: &WallGrid,
    theme: &Theme,
    area: Rect,
    pos: Vec2,
    yaw: f32,
    bob: f32,
    light: &Lighting,
    stride: f32,
    props: &Props,
) -> View {
    let light_mul = light.base;
    let doors = Doors {
        open: props.doors_open,
        swing: props.door_swing,
    };
    let dir = vec2(yaw.cos(), yaw.sin());
    // Camera plane half-width sized to the view's aspect, so a world unit spans as many
    // pixels across as up (`px_per_unit`). ~0.65 (~66deg) at the native 900x690; a wider
    // window sees more to the sides rather than stretching.
    let plane = vec2(-dir.y, dir.x) * (area.w * 0.5 / area.h);
    let pal = theme.palette;
    let horizon = area.y + area.h * (0.5 - props.look_down);
    let eye = 0.5 + bob;
    let tex = &theme.wall;

    let mut view = View {
        area,
        pos,
        dir,
        stride,
        plane,
        horizon,
        eye,
        haze: theme.ground.as_ref().and_then(|g| g.horizon),
        zbuf: Vec::with_capacity((area.w / COL_STEP) as usize + 1),
        roof: Vec::with_capacity((area.w / COL_STEP) as usize + 1),
        under: Vec::with_capacity((area.w / COL_STEP) as usize + 1),
        portal: Vec::with_capacity((area.w / COL_STEP) as usize + 1),
        well: grid.well(),
        glass: None,
    };

    // Cast every column first: the ground pass needs to know where the mirrors are.
    let mut cols: Vec<Col> = Vec::with_capacity((area.w / COL_STEP) as usize + 1);
    // Office doorways the columns look into: their pieces, and per column which.
    let mut frames = door_frames(grid, props.swing, doors);
    doorway::cull(&mut frames, pos, dir, plane);
    let mut pieces: Vec<Piece> = Vec::new();
    let mut dcols: Vec<DoorCol> = Vec::with_capacity(cols.capacity());
    let in_well = grid.well().is_some_and(|w| w.holds(pos));
    let mut sx = area.x;
    while sx < area.x + area.w {
        let ray = view.ray(sx);
        sx += COL_STEP;
        if theme.trees {
            cols.push(Col::Open);
            dcols.push(DoorCol::default());
            view.portal.push(false);
            continue;
        }
        let hit = cast(grid, pos, ray);
        view.portal
            .push(in_well || (WELL_LINE..WELL_LINE + 3).contains(&hit.line));
        let start = pieces.len();
        let out = if frames.is_empty() {
            DoorOut::default()
        } else {
            door_column(&frames, pos, ray, hit.front(), &pal, light, &mut pieces)
        };
        dcols.push(DoorCol {
            pieces: (start, pieces.len()),
            clip: out.clip,
            under: out.under,
            roof: out.roof,
        });
        cols.push(match out.stop {
            Some((t, face)) => Col::Door { t, face },
            None => Col::Wall(hit),
        });
    }
    let runs = glass_runs(&cols);

    // Sky and ground.
    let screen = area_clip(area);
    match &theme.ground {
        Some(g) => {
            draw_sky(&view, &pal, g);
            let mut bd = MeshBuf::new(None);
            draw_floor_backdrop(&view, &pal, g, light_mul, &screen, [1.0; 3], &mut bd);
            bd.flush();
            let whole = (-1.0, 1.0);
            let all = |_: IVec2| true;
            draw_ground(
                &view, grid, g, &pal, light_mul, &screen, whole, None, [1.0; 3], &all,
            );
            // The floor seen in each mirror: the real floor in front of the glass, cast
            // from the camera's mirror image, clipped to the glass. Shallowest bounce
            // first — each deeper level paints over the (wrong) floor the one before
            // drew past its next glass.
            for run in &runs {
                let mv = view.mirrored(run.image, run.flip);
                let clip = run_clip(&view, &cols, run, &screen);
                let tint = glass_tint(run.level);
                draw_floor_backdrop(&mv, &pal, g, light_mul, &clip, tint, &mut bd);
                bd.flush();
                if run.level > GROUND_BOUNCES {
                    continue;
                }
                let keep = |sq: IVec2| {
                    let c = if run.flips_x { sq.x } else { sq.y } as f32;
                    if run.side > 0.0 {
                        c >= run.plane - 1e-3
                    } else {
                        c + 1.0 <= run.plane + 1e-3
                    }
                };
                let Some(within) = run_squares(&mv, &cols, run, ground_range(g)) else {
                    continue;
                };
                let span = run_span(area, run);
                draw_ground(
                    &mv,
                    grid,
                    g,
                    &pal,
                    light_mul,
                    &clip,
                    span,
                    Some(within),
                    tint,
                    &keep,
                );
            }
        }
        None => {
            v_gradient(
                area.x,
                area.y,
                area.w,
                horizon - area.y,
                pal.ceil_top,
                pal.ceil_bot,
            );
            v_gradient(
                area.x,
                horizon,
                area.w,
                area.y + area.h - horizon,
                pal.floor_far,
                pal.floor_near,
            );
        }
    }
    match theme.ceiling {
        Ceiling::Stone => draw_stone_ceiling(&view, &pal, light, grid.thin),
        Ceiling::Tiles => draw_office_ceiling(&view, &pal, light),
        Ceiling::Sky => {}
    }

    // Wall strips. Per column: the real depth for billboards, and for mirror columns
    // what the reflected world's hedge pass depth-tests against.
    let n = cols.len();
    let mut hedge = HedgeCols {
        ztop: Vec::with_capacity(n),
        face: Vec::with_capacity(n),
    };
    let mut refl: Vec<Option<Seen>> = Vec::with_capacity(n);
    // Queued for `draw_strips`, which needs each column's neighbours to slope its edges.
    let mut strips: Vec<Option<Strip>> = Vec::with_capacity(n);
    let bottom = area.y + area.h;
    for (ci, col) in cols.iter().enumerate() {
        let sx = area.x + ci as f32 * COL_STEP;
        let ray = view.ray(sx);
        let dc = &dcols[ci];
        view.roof
            .push(dc.roof.map_or((f32::INFINITY, f32::NEG_INFINITY), |d| {
                (d, view.screen_y(DOOR_H, d))
            }));
        view.under
            .push(dc.under.map_or((f32::INFINITY, f32::INFINITY), |b| {
                (b.depth, view.screen_y(b.z, b.depth))
            }));
        let hit = match col {
            Col::Open => {
                view.zbuf.push(f32::INFINITY);
                hedge.ztop.push(area.y);
                hedge.face.push(None);
                refl.push(None);
                strips.push(None);
                continue;
            }
            &Col::Door { t, face } => {
                view.zbuf.push(t);
                hedge.ztop.push(view.screen_y(1.0, t));
                hedge.face.push(None);
                refl.push(None);
                strips.push(face.map(|(side, line, wall_x, lamp)| Strip {
                    depth: t,
                    surf: Surf {
                        side,
                        line,
                        image: pos,
                        flip: (false, false),
                    },
                    kind: StripKind::Wall {
                        side,
                        wall_x,
                        glass: [1.0; 3],
                        light_mul: lamp,
                        profile: Profile::Flat,
                    },
                    clip: dc.clip,
                    under: dc.under,
                }));
                continue;
            }
            Col::Wall(hit) => hit,
        };
        let level = hit.bounces;
        let front = hit.front();
        // The ray that struck the final wall (reflected by every bounce).
        let struck = hit.mirrors().last().map_or(ray, |m| flip(ray, m.flip));
        let glass = glass_tint(level);
        let lamp = light.at(pos + ray * front);
        // (depth, top edge on screen, hedge face) of whatever the ray finally struck.
        // The plane the final wall lies on, in the unfolded world of its mirror chain.
        let surf = Surf {
            side: hit.side,
            line: hit.line,
            image: hit.mirrors().last().map_or(pos, |m| m.image),
            flip: hit.mirrors().last().map_or((false, false), |m| m.flip),
        };
        let seen = if hit.perp > 1e5 {
            // Through the garden's gate: open country, nothing to draw.
            strips.push(None);
            (1e6, bottom, None)
        } else if theme.bushes && !grid.is_mirror(hit.map.x, hit.map.y) {
            // A clipped hedge: what the wall pass draws is its shaded interior, set just
            // behind the face — the leaf clumps (`draw_clumps`) stand in front of it and
            // it shows only in the gaps between them. Past clump range it eases back
            // into a plain lit face. Seen in a mirror, the same.
            let rn = if hit.side == 0 { ray.x } else { ray.y }.abs().max(1e-3);
            let far = if level > CLUMP_BOUNCES {
                1.0
            } else {
                smoothstep(CLUMP_FADE, CLUMP_RANGE, hit.perp)
            };
            let core = hit.perp + (CORE_RECESS / rn).min(0.3) * (1.0 - far);
            let lumps = (hit.wall_x * 9.0 + hit.square * 40.0).sin() * 0.5
                + (hit.wall_x * 23.0 + hit.square * 91.0).sin() * 0.3;
            let top = CORE_TOP + 0.008 * lumps;
            let top = top + (HEDGE_TOP - top) * far;
            strips.push(Some(Strip {
                depth: core,
                surf,
                kind: StripKind::Wall {
                    side: hit.side,
                    wall_x: hit.wall_x,
                    glass,
                    light_mul: lamp,
                    profile: Profile::Core {
                        top,
                        k: 0.5 + 0.5 * far,
                    },
                },
                clip: None,
                under: None,
            }));
            (
                core,
                view.screen_y(top, core),
                Some((hit.map, face_of(hit.side, struck))),
            )
        } else {
            strips.push(Some(Strip {
                depth: hit.perp,
                surf,
                kind: StripKind::Wall {
                    side: hit.side,
                    wall_x: hit.wall_x,
                    glass,
                    light_mul: lamp,
                    profile: Profile::Flat,
                },
                clip: dc.clip,
                under: dc.under,
            }));
            (hit.perp, view.screen_y(1.0, hit.perp), None)
        };
        match hit.mirrors().first() {
            Some(m) => {
                view.zbuf.push(m.dist);
                hedge.ztop.push(view.screen_y(1.0, m.dist));
                hedge.face.push(None);
                refl.push(Some(seen));
            }
            None => {
                view.zbuf.push(seen.0);
                hedge.ztop.push(seen.1);
                hedge.face.push(seen.2);
                refl.push(None);
            }
        }
    }

    draw_strips(&view, tex, &pal, &strips, &pieces, &dcols);

    // Then, deepest bounce first, what each level of glass shows in front of whatever
    // lies behind it (the next glass, or the final wall): the hedges seen at that level
    // as full clumped volumes — the same clump pass, cast from that level's camera image
    // — then the set dressing and critters in front of them (`Props::reflect`), then
    // the bot's own reflection at that level, then the glass over them.
    let deepest = runs.iter().map(|r| r.level).max().unwrap_or(0);
    for level in (1..=deepest).rev() {
        let clumps = theme
            .sprites
            .as_ref()
            .filter(|_| theme.bushes && level <= CLUMP_BOUNCES);
        let reflect = props.reflect.filter(|_| level <= DECOR_BOUNCES);
        if clumps.is_some() || reflect.is_some() {
            for run in runs.iter().filter(|r| r.level == level) {
                let mut mv = view.mirrored(run.image, run.flip);
                let behind = |c: usize| -> Option<Seen> {
                    let Col::Wall(hit) = &cols[c] else {
                        return None;
                    };
                    if !(run.c0..=run.c1).contains(&c) {
                        return None;
                    }
                    match hit.mirrors().get(level) {
                        Some(next) => Some((next.dist, view.screen_y(1.0, next.dist), None)),
                        None => refl[c],
                    }
                };
                let seen: Vec<Option<Seen>> = (0..n).map(behind).collect();
                mv.zbuf = seen.iter().map(|r| r.map_or(0.0, |r| r.0)).collect();
                let mh = HedgeCols {
                    ztop: seen
                        .iter()
                        .map(|r| r.map_or(f32::NEG_INFINITY, |r| r.1))
                        .collect(),
                    face: seen.iter().map(|r| r.and_then(|r| r.2)).collect(),
                };
                if let Some(sp) = clumps {
                    let keep = |p: Vec2| run.in_front(p);
                    // Clumps covering eye height hide the dressing behind them, as in
                    // the real pass.
                    for (c, d) in draw_clumps(&mv, grid, sp, &pal, light, &mh, Some((&keep, level)))
                    {
                        mv.zbuf[c] = mv.zbuf[c].min(d);
                    }
                }
                if let Some(f) = reflect {
                    mv.glass = Some(glass_window(&cols, run));
                    f(&mv);
                }
            }
        }
        // Glass over the reflections, batched: each column's sheen goes on after its
        // own reflection, and columns don't overlap.
        let mut sheen = MeshBuf::new(None);
        for (ci, col) in cols.iter().enumerate() {
            if let Col::Wall(hit) = col
                && let Some(m) = hit.mirrors().get(level - 1)
            {
                let sx = area.x + ci as f32 * COL_STEP;
                let far = hit.mirrors().get(level).map_or(hit.perp, |next| next.dist);
                draw_reflection(&view, sx, m, far, glass_tint(level), &pal, light_mul);
                let right = glass_edge(&cols, ci, level);
                draw_mirror_glass(&view, &mut sheen, ci, m, right, &pal, light_mul);
            }
        }
        sheen.flush();
    }

    match &theme.sprites {
        Some(sp) if theme.trees => {
            let split = props
                .amid
                .iter()
                .map(|&p| view.depth(p))
                .fold(0.0, f32::max);
            let trees = forest_trees(&view, grid);
            draw_trees(
                &view,
                &pal,
                light_mul,
                sp,
                &trees,
                (split, f32::INFINITY),
                true,
            );
        }
        Some(sp) if theme.bushes => {
            for (c, d) in draw_clumps(&view, grid, sp, &pal, light, &hedge, None) {
                view.zbuf[c] = view.zbuf[c].min(d);
            }
        }
        _ => {}
    }
    view
}

/// Group the mirror columns into runs, one per stretch of the same glass face seen
/// through the same camera image, shallowest bounce level first — the reflected ground
/// passes rely on that order.
fn glass_runs(cols: &[Col]) -> Vec<GlassRun> {
    let mut runs: Vec<GlassRun> = Vec::new();
    for level in 1..=MAX_BOUNCES {
        let start = runs.len();
        for (ci, col) in cols.iter().enumerate() {
            let Col::Wall(hit) = col else {
                continue;
            };
            let Some(m) = hit.mirrors().get(level - 1) else {
                continue;
            };
            if runs.len() > start
                && let Some(r) = runs.last_mut()
                && r.key == m.key
                && r.image == m.image
                && r.c1 + 1 == ci
            {
                r.c1 = ci;
                continue;
            }
            runs.push(GlassRun {
                c0: ci,
                c1: ci,
                level,
                key: m.key,
                image: m.image,
                flip: m.flip,
                flips_x: m.flips_x,
                plane: m.plane,
                side: m.side,
            });
        }
        if runs.len() == start {
            break;
        }
    }
    runs
}

/// A glass run's window for its mirrored camera: per column, the glass's own depth as
/// the near limit (whatever is nearer lies on the far side of the glass in the real
/// world), infinite outside the run.
fn glass_window(cols: &[Col], run: &GlassRun) -> Glass {
    let glass_at = |c: usize| match &cols[c] {
        Col::Wall(hit) if (run.c0..=run.c1).contains(&c) => hit
            .mirrors()
            .get(run.level - 1)
            .map_or(f32::INFINITY, |m| m.dist),
        _ => f32::INFINITY,
    };
    Glass {
        znear: (0..cols.len()).map(glass_at).collect(),
        tint: glass_tint(run.level),
        handed: run.level % 2 == 1,
        c0: run.c0,
        c1: run.c1,
    }
}

/// The stage's own edges as clip half-planes (left, right, bottom).
fn area_clip(area: Rect) -> Vec<Half> {
    vec![
        Half {
            n: vec2(-1.0, 0.0),
            d: -area.x,
        },
        Half {
            n: vec2(1.0, 0.0),
            d: area.x + area.w,
        },
        Half {
            n: vec2(0.0, 1.0),
            d: area.y + area.h,
        },
    ]
}

/// Clip for drawing inside one mirror (at any bounce level): its columns, and above the glass's foot (the
/// line where the glass meets the real floor — straight on screen, being a floor line).
fn run_clip(view: &View, cols: &[Col], run: &GlassRun, screen: &[Half]) -> Vec<Half> {
    let foot = |c: usize| {
        let x = view.area.x + c as f32 * COL_STEP;
        let d = match &cols[c] {
            Col::Wall(hit) => hit.mirrors().get(run.level - 1).map_or(1.0, |m| m.dist),
            _ => 1.0,
        };
        vec2(x, view.screen_y(0.0, d))
    };
    let (a, b) = (foot(run.c0), foot(run.c1));
    let slope = if run.c1 > run.c0 {
        (b.y - a.y) / (b.x - a.x)
    } else {
        0.0
    };
    let mut clip = screen.to_vec();
    clip.push(Half {
        n: vec2(-1.0, 0.0),
        d: -a.x,
    });
    clip.push(Half {
        n: vec2(1.0, 0.0),
        d: b.x + COL_STEP + 0.6,
    });
    clip.push(Half {
        n: vec2(-slope, 1.0),
        d: a.y - slope * a.x,
    });
    clip
}

/// A run's screen columns as a -1..1 span across the view (`View::ray`'s parameter),
/// padded a column each side.
fn run_span(area: Rect, run: &GlassRun) -> (f32, f32) {
    let x = |c: f32| 2.0 * c * COL_STEP / area.w - 1.0;
    (x(run.c0 as f32 - 1.0), x(run.c1 as f32 + 2.0))
}

/// The grid squares (min, max corner) the floor seen through a run's glass can lie in:
/// the bounding box of its wedge — between the edge columns' rays from the camera image
/// `mv`, from the glass out to depth `range` — or `None` if the glass is past `range`.
fn run_squares(mv: &View, cols: &[Col], run: &GlassRun, range: f32) -> Option<(IVec2, IVec2)> {
    let glass = |c: usize| match &cols[c] {
        Col::Wall(hit) => hit.mirrors().get(run.level - 1).map_or(0.0, |m| m.dist),
        _ => 0.0,
    };
    let (mut lo, mut hi) = (Vec2::splat(f32::MAX), Vec2::splat(f32::MIN));
    for (c, edge) in [(run.c0, run.c0), (run.c1, run.c1 + 1)] {
        // The far edge always bounds the wedge; the glass only where it's in range.
        let near = glass(c).min(range);
        let ray = mv.ray(mv.area.x + edge as f32 * COL_STEP);
        for t in [near, range] {
            let p = mv.pos + ray * t;
            (lo, hi) = (lo.min(p), hi.max(p));
        }
    }
    (glass(run.c0).min(glass(run.c1)) < range).then(|| {
        (
            lo.floor().as_ivec2() - IVec2::ONE,
            hi.floor().as_ivec2() + IVec2::ONE,
        )
    })
}

/// How a wall strip's top edge and shading are shaped.
#[derive(Clone, Copy)]
enum Profile {
    /// A plain full-height wall.
    Flat,
    /// A hedge's shaded interior behind its clumps: top at `top`, brightness `k`.
    Core { top: f32, k: f32 },
}

/// The plane a column's surface lies on. Neighbouring columns on the same one share
/// the edge between them, so the wall's top and foot run as straight sloped lines
/// instead of 2px stair steps. Walls: the grid line struck (`side`, `line`) in the
/// unfolded world of the mirror chain's last camera image (`image`, `flip`); office
/// panels: `side` 2, `line` the panel's index.
#[derive(Clone, Copy, PartialEq)]
struct Surf {
    side: i32,
    line: i32,
    image: Vec2,
    flip: (bool, bool),
}

/// One column's share of the batched wall pass (`draw_strips`), queued by the cast
/// loop: its depth (a hedge's recessed core, not the face), the plane it lies on, and
/// how to shade it.
struct Strip {
    depth: f32,
    surf: Surf,
    kind: StripKind,
    /// Seen through a doorway: nothing above this edge (the head's underside).
    clip: Option<Bound>,
    /// Behind an open leaf: nothing below this edge (its top).
    under: Option<Bound>,
}

enum StripKind {
    Wall {
        side: i32,
        wall_x: f32,
        /// `glass_tint` of the mirror level it's seen through.
        glass: [f32; 3],
        light_mul: f32,
        profile: Profile,
    },
}

impl Strip {
    /// World height of the strip's top edge.
    fn top_z(&self) -> f32 {
        match self.kind {
            StripKind::Wall {
                profile: Profile::Core { top, .. },
                ..
            } => top,
            _ => 1.0,
        }
    }
}

/// Where a surface's floor (z = 0) and ceiling (z = 1) lines cross one column edge.
#[derive(Clone, Copy)]
struct Edge {
    y0: f32,
    y1: f32,
}

impl Edge {
    fn at(view: &View, depth: f32) -> Edge {
        let d = depth.max(0.0001);
        Edge {
            y0: view.screen_y(0.0, d),
            y1: view.screen_y(1.0, d),
        }
    }

    /// Screen y of world height `z` on this edge.
    fn y(self, z: f32) -> f32 {
        self.y0 + (self.y1 - self.y0) * z
    }

    /// `y`'s place down the surface's full height, 0 at the top — texture v.
    fn v(self, y: f32) -> f32 {
        (y - self.y1) / (self.y0 - self.y1)
    }
}

/// Depth and top height of strip `c` at its right screen edge. Its left edge is where
/// its own ray was cast (`View::ray`), so that one is exact; the right edge is the next
/// column's when that strikes the same plane (the two quads then share it exactly),
/// else extrapolated along the plane from the column before (depth is linear in 1/d
/// across the screen) — so a face runs straight up to a corner instead of ending in a
/// flat step — else the column's own.
fn right_edge(strips: &[Option<Strip>], c: usize) -> (f32, f32) {
    let Some(s) = &strips[c] else {
        return (1.0, 1.0);
    };
    let same = |i: usize| {
        strips
            .get(i)
            .and_then(Option::as_ref)
            .filter(|o| o.surf == s.surf)
    };
    if let Some(n) = same(c + 1) {
        return (n.depth, n.top_z());
    }
    if let Some(p) = c.checked_sub(1).and_then(same) {
        let inv = 2.0 / s.depth.max(0.0001) - 1.0 / p.depth.max(0.0001);
        if inv > 1e-4 {
            return (1.0 / inv, s.top_z());
        }
    }
    (s.depth, s.top_z())
}

/// A quad between column edges `xs`, spanning `top`..`bot` at each. `uv`: per corner
/// (top-left, top-right, bottom-right, bottom-left) texture coordinates.
fn quad(buf: &mut MeshBuf, xs: [f32; 2], top: [f32; 2], bot: [f32; 2], col: Color, uv: [Vec2; 4]) {
    if bot[0] <= top[0] && bot[1] <= top[1] {
        return;
    }
    let (b0, b1) = (bot[0].max(top[0]), bot[1].max(top[1]));
    buf.quad([
        Vertex::new(xs[0], top[0], 0.0, uv[0].x, uv[0].y, col),
        Vertex::new(xs[1], top[1], 0.0, uv[1].x, uv[1].y, col),
        Vertex::new(xs[1], b1, 0.0, uv[2].x, uv[2].y, col),
        Vertex::new(xs[0], b0, 0.0, uv[3].x, uv[3].y, col),
    ]);
}

/// Depth at column `c`'s left edge of the plane `surf`, if the column shows it — as its
/// wall strip or as any edge of its doorway pieces.
fn plane_depth(
    strips: &[Option<Strip>],
    pieces: &[Piece],
    dcols: &[DoorCol],
    c: usize,
    surf: Surf,
) -> Option<f32> {
    if let Some(Some(s)) = strips.get(c)
        && s.surf == surf
    {
        return Some(s.depth);
    }
    let (a, b) = dcols.get(c)?.pieces;
    pieces[a..b].iter().find_map(|p| {
        [Some(p.top), Some(p.bot), p.clip, p.under]
            .into_iter()
            .flatten()
            .find(|e| e.surf == surf)
            .map(|e| e.depth)
    })
}

/// `bound`'s depth at column `c`'s right edge, found as `right_edge` finds a strip's.
fn bound_right(
    strips: &[Option<Strip>],
    pieces: &[Piece],
    dcols: &[DoorCol],
    c: usize,
    b: Bound,
) -> f32 {
    if let Some(d) = plane_depth(strips, pieces, dcols, c + 1, b.surf) {
        return d;
    }
    if let Some(p) = c
        .checked_sub(1)
        .and_then(|p| plane_depth(strips, pieces, dcols, p, b.surf))
    {
        let inv = 2.0 / b.depth.max(0.0001) - 1.0 / p.max(0.0001);
        if inv > 1e-4 {
            return 1.0 / inv;
        }
    }
    b.depth
}

/// The wall pass: every queued column as a textured quad whose top and foot follow the
/// true (sloped) edge across it, the door furniture over it, then the fog wash — three
/// `MeshBuf`s (wall texture, flat colour, translucent fog), so a whole frame of walls
/// is a handful of draw calls rather than two or more per column — then a fourth for
/// what stands in front of all of that (a gate's straps between its bars). Order holds
/// because columns never overlap, and within a column the doorway pieces and the strip
/// behind them are cut to disjoint bands (`doorway`).
fn draw_strips(
    view: &View,
    tex: &Texture2D,
    pal: &Palette,
    strips: &[Option<Strip>],
    pieces: &[Piece],
    dcols: &[DoorCol],
) {
    let area = view.area;
    let (lo, hi) = (area.y, area.y + area.h);
    let mut walls = MeshBuf::new(Some(tex));
    let mut solid = MeshBuf::new(None);
    let mut fog = MeshBuf::new(None);
    let mut front = MeshBuf::new(None);
    let tw = tex.width();
    for (c, s) in strips.iter().enumerate() {
        // The layers must reach the screen in order — walls, then door furniture, then
        // fog, then the front. A buffer that fills up flushes itself on the spot, so
        // before a later one could, flush everything under it first: a close door seen
        // head-on used to flush its furniture mid-frame and have the rest of the walls
        // painted over it, the door vanishing into the wall.
        if !solid.room_for(DOOR_QUADS) {
            walls.flush();
            solid.flush();
        }
        if !fog.room_for(DOOR_QUADS) {
            walls.flush();
            solid.flush();
            fog.flush();
        }
        if !front.room_for(DOOR_QUADS) {
            walls.flush();
            solid.flush();
            fog.flush();
            front.flush();
        }
        let xs = [
            area.x + c as f32 * COL_STEP,
            area.x + (c + 1) as f32 * COL_STEP,
        ];
        let edges_of = |b: Bound| {
            [
                Edge::at(view, b.depth),
                Edge::at(view, bound_right(strips, pieces, dcols, c, b)),
            ]
        };
        let clip_ys = |clip: Option<Bound>| {
            clip.map_or([lo; 2], |b| {
                let e = edges_of(b);
                [e[0].y(b.z).max(lo), e[1].y(b.z).max(lo)]
            })
        };
        let under_ys = |under: Option<Bound>| {
            under.map_or([hi; 2], |b| {
                let e = edges_of(b);
                [e[0].y(b.z).min(hi), e[1].y(b.z).min(hi)]
            })
        };
        // The column's doorway pieces, farthest first (the front layer paints in order).
        let dc = dcols.get(c).copied().unwrap_or_default();
        for p in pieces[dc.pieces.0..dc.pieces.1].iter().rev() {
            let (et, eb) = (edges_of(p.top), edges_of(p.bot));
            let (ct, cb) = (clip_ys(p.clip), under_ys(p.under));
            let top = [0, 1].map(|i| et[i].y(p.top.z).max(ct[i]));
            let bot = [0, 1].map(|i| eb[i].y(p.bot.z).min(cb[i]));
            if bot[0] <= top[0] && bot[1] <= top[1] {
                continue;
            }
            match p.look {
                Look::Wall {
                    side,
                    wall_x,
                    light,
                } => {
                    let lit = lit_at(p.top.depth, pal.haze, light);
                    let k = lit * if side == 0 { pal.ew_tint } else { 1.0 };
                    let u0 = (wall_x * tw).clamp(0.0, tw - 1.0) / tw;
                    let u1 = u0 + 1.0 / tw;
                    quad(
                        &mut walls,
                        xs,
                        top,
                        bot,
                        Color::new(k, k, k, 1.0),
                        [
                            vec2(u0, et[0].v(top[0])),
                            vec2(u1, et[1].v(top[1])),
                            vec2(u1, et[1].v(bot[1])),
                            vec2(u0, et[0].v(bot[0])),
                        ],
                    );
                    let wash = rgba(view.fog(pal, p.top.depth), fog_at(p.top.depth, pal.haze));
                    quad(&mut fog, xs, top, bot, wash, [Vec2::ZERO; 4]);
                }
                Look::Decal(col) | Look::Flat(col) => {
                    quad(&mut solid, xs, top, bot, col, [Vec2::ZERO; 4]);
                }
                Look::Front(col) => {
                    quad(&mut front, xs, top, bot, col, [Vec2::ZERO; 4]);
                }
                Look::Leaf { .. } | Look::Bar { .. } => {
                    doorway::draw_leaf(&mut solid, xs, et, top, bot, p.look, pal.fog);
                }
            }
        }
        let Some(s) = s else {
            continue;
        };
        let (dr, tzr) = right_edge(strips, c);
        let edges = [Edge::at(view, s.depth), Edge::at(view, dr)];
        let StripKind::Wall {
            side,
            wall_x,
            glass,
            light_mul,
            profile,
        } = s.kind;
        let (tz, round) = match profile {
            Profile::Core { top, k } => (top, k),
            Profile::Flat => (1.0, 1.0),
        };
        let tzs = [tz, tzr];
        let (ct, cb) = (clip_ys(s.clip), under_ys(s.under));
        let top = [0, 1].map(|i| edges[i].y(tzs[i]).max(ct[i]));
        let bot = [0, 1].map(|i| edges[i].y0.min(cb[i]));
        if bot[0] <= top[0] && bot[1] <= top[1] {
            continue;
        }
        let lit = lit_at(s.depth, pal.haze, light_mul);
        let face = if side == 0 { pal.ew_tint } else { 1.0 };
        // Seen in a mirror (`glass_tint`): a little darker and cooler, like
        // silvered glass.
        let k = lit * face * round;
        let tint = Color::new(k * glass[0], k * glass[1], k * glass[2], 1.0);
        // One texel column, spread across the strip's width as before.
        let u0 = (wall_x * tw).clamp(0.0, tw - 1.0) / tw;
        let u1 = u0 + 1.0 / tw;
        let v = |i: usize, y: f32| edges[i].v(y);
        quad(
            &mut walls,
            xs,
            top,
            bot,
            tint,
            [
                vec2(u0, v(0, top[0])),
                vec2(u1, v(1, top[1])),
                vec2(u1, v(1, bot[1])),
                vec2(u0, v(0, bot[0])),
            ],
        );
        // Fog over the whole strip, gaps included — in the forest that's the
        // mist hanging between the trunks at this depth.
        let wash = rgba(view.fog(pal, s.depth), fog_at(s.depth, pal.haze));
        quad(&mut fog, xs, top, bot, wash, [Vec2::ZERO; 4]);
    }
    walls.flush();
    solid.flush();
    fog.flush();
    front.flush();
}

/// Forest: how far out trees are drawn — past this the haze has fully swallowed them.
pub(crate) const TREE_RANGE: f32 = 11.0;
/// How much the mist softens a far spruce: its mip is picked as if it were `1 + MIST_BLUR·depth²`
/// times smaller than drawn.
pub(crate) const MIST_BLUR: f32 = 0.06;

fn square_hash(m: IVec2) -> f32 {
    let h = mix64((m.x as u32 as u64) << 32 | m.y as u32 as u64);
    (h >> 40) as f32 / (1u64 << 24) as f32
}

fn mix64(mut v: u64) -> u64 {
    v ^= v >> 30;
    v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    v ^= v >> 27;
    v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
    v ^ (v >> 31)
}

/// Forest: where the pines in solid grid square `(gx, gy)` stand, with each one's hash —
/// one or two at a random spot inside the square, a stable hash of it so they never
/// move between frames.
pub fn tree_spots(gx: i32, gy: i32) -> impl Iterator<Item = (Vec2, u64)> {
    let h = mix64(((gx as u64) << 32) ^ (gy as u32 as u64) ^ 0x7EE5);
    (0..1 + (h & 1)).map(move |i| {
        let t = mix64(h ^ (i + 1));
        let f = |k: u32| ((t >> (k * 12)) & 0xFFF) as f32 / 4095.0;
        // Stay inside the square: an open square next to it is walkway.
        let wp = vec2(gx as f32 + 0.2 + 0.6 * f(0), gy as f32 + 0.2 + 0.6 * f(1));
        (wp, t)
    })
}

/// Forest: the pines in view (`tree_spots` of every solid square in range) as
/// `(depth, screen x, hash, world point)`, far to near. Collected once a frame by the
/// caller that interleaves other billboards among them (`draw_trees_between`).
pub struct Trees(Vec<(f32, f32, u64, Vec2)>);

pub fn forest_trees(view: &View, grid: &WallGrid) -> Trees {
    let r = TREE_RANGE as i32 + 1;
    let (px, py) = (view.pos.x as i32, view.pos.y as i32);
    let mut trees: Vec<(f32, f32, u64, Vec2)> = Vec::with_capacity(384);
    for gy in (py - r).max(-3)..=(py + r).min(grid.gh as i32 + 2) {
        for gx in (px - r).max(-3)..=(px + r).min(grid.gw as i32 + 2) {
            if !grid.is_solid(gx, gy) {
                continue;
            }
            for (wp, t) in tree_spots(gx, gy) {
                if let Some((cx, depth)) = view.project(wp)
                    && depth < TREE_RANGE
                {
                    trees.push((depth, cx, t, wp));
                }
            }
        }
    }
    crate::util::sort_by_key(&mut trees, |t| -t.0);
    let area = view.area;
    trees.retain(|&(depth, cx, _, _)| {
        let lat = view.px_per_unit(depth);
        cx + lat >= area.x && cx - lat <= area.x + area.w
    });
    Trees(trees)
}

/// Forest: every solid grid square (walls, pillars, and a ring past the maze edge) is
/// planted with pines (`tree_spots`). Pines rather than a textured face, so a wall reads
/// as trees blocking the way; drawn far to near (painter's order is the only occlusion
/// needed) and faded into the mist. Only trees at depths in `range` are drawn, so the
/// caller can interleave other billboards (`Props::amid`); `shadows` draws every tree's
/// contact shadow first.
fn draw_trees(
    view: &View,
    pal: &Palette,
    light: f32,
    sp: &Sprites,
    trees: &Trees,
    (near, far): (f32, f32),
    shadows: bool,
) {
    let mist = pal.mist.unwrap_or(fog::FOREST);

    // Contact shadows first, all of them: they lie on the floor under everything, and
    // one untextured batch beats breaking the sprite batch twice per tree.
    if shadows {
        for &(depth, cx, t, wp) in &trees.0 {
            draw_spruce_shadow(view, &mist, sp, cx, depth, t, wp);
        }
    }
    let mut buf = MeshBuf::new(Some(&sp.tex));
    if let Some(mat) = &sp.premul {
        gl_use_material(mat);
    }
    for &(depth, cx, t, wp) in &trees.0 {
        if depth < near || depth >= far {
            continue;
        }
        draw_spruce(view, &mist, sp, cx, depth, t, wp, light, &mut buf);
    }
    buf.flush();
    if sp.premul.is_some() {
        gl_use_default_material();
    }
}

/// Forest: the trees at depths in `near..far` that `draw` left out (see `Props::amid`).
/// Skips the draw (and its material switch) when no tree falls in the range — most
/// gaps between neighbouring motes hold none.
pub fn draw_trees_between(
    view: &View,
    theme: &Theme,
    light: &Lighting,
    trees: &Trees,
    near: f32,
    far: f32,
) {
    if let Some(sp) = &theme.sprites
        && theme.trees
        && near < far
        && trees.0.iter().any(|&(d, ..)| d >= near && d < far)
    {
        let pal = theme.palette;
        draw_trees(view, &pal, light.base, sp, trees, (near, far), false);
    }
}

/// A spruce's size and pose from its hash `t` at `depth`: (height, lean, quad width,
/// quad height, ground line on screen, quad top on screen). Mature spruces tower over
/// the bot (eye height 0.5): 3-5.5 units. The sprite is painted at a spruce's own
/// proportions, so it's scaled uniformly — squeezed sideways its branches read as
/// hanging strands — capping only the tallest's width at what its square allows.
fn spruce_frame(view: &View, sp: &Sprites, depth: f32, t: u64) -> (f32, f32, f32, f32, f32) {
    let r = |i: u64| spruce_rand(t, i);
    let lat = view.px_per_unit(depth);
    let height = 3.0 + 2.5 * r(1);
    let lean = (r(2) - 0.5) * 0.05;
    let widen = 0.85 + 0.3 * r(3);
    let qh = height * TREE_SPAN * lat;
    let qw = (height * sp.aspect()).min(1.35) * TREE_SPAN * lat * widen;
    let ground = view.screen_y(0.0, depth);
    (lean, qw, qh, ground, ground + qh * TREE_FOOT - qh)
}

fn spruce_rand(t: u64, i: u64) -> f32 {
    (mix64(t ^ i.wrapping_mul(0x9E37_79B9_7F4A_7C15)) >> 40) as f32 / 16_777_216.0
}

/// How much of a tree at world `dist` survives the mist beyond its own transmittance:
/// trees are only drawn out to `TREE_RANGE`, and a crown up in the thinner air there
/// would still pop out of view, so the last stretch eases them fully into the mist.
fn tree_range_fade(dist: f32) -> f32 {
    1.0 - smoothstep(TREE_RANGE * 0.65, TREE_RANGE, dist)
}

/// A spruce's soft contact shadow, foreshortened like the floor it lies on and fogged
/// with it: three nested ellipses so it has no hard rim.
fn draw_spruce_shadow(
    view: &View,
    mist: &fog::Mist,
    sp: &Sprites,
    cx: f32,
    depth: f32,
    t: u64,
    wp: Vec2,
) {
    let (_, qw, _, ground, y0) = spruce_frame(view, sp, depth, t);
    let area = view.area;
    if cx + qw * 0.6 < area.x || cx - qw * 0.6 > area.x + area.w || y0 > area.y + area.h {
        return;
    }
    let dist = wp.distance(view.pos).max(1e-3);
    let tr = mist.transmittance((dist * dist + view.eye * view.eye).sqrt(), view.eye, 0.0)
        * tree_range_fade(dist);
    if tr <= 0.08 {
        return;
    }
    let eh = (ground - view.horizon).max(0.0) * 0.45 / depth;
    let c = mist.apply([0.02, 0.04, 0.02], tr, -view.eye / dist);
    let dark = rgba(c, 0.3 * tr / 3.0);
    for k in [1.0, 0.72, 0.45] {
        draw_ellipse(cx, ground, qw * 0.45 * k, eh * k, 0.0, dark);
    }
}

/// Vertex rows down a spruce quad: the mist changes with height (thicker low, and its
/// colour with the ray's elevation), so the fog is per row, not per tree.
const SPRUCE_ROWS: usize = 5;

/// One spruce at screen column `cx`, `depth` away (world position `wp`): a billboard of
/// one of the theme's painted spruces (`theme::paint_spruce` — branch sprays, trunk,
/// needles, shading are all in the sprite), varied per tree hash `t` by variant, mirror
/// flip, height, width, lean and a slight hue/brightness tint, dimmed by `light`. The
/// mip level follows its on-screen height, so a far tree is a downsample, not a
/// shimmer, biased coarser with distance so the mist softens far trees.
///
/// Pushed into `buf` (the atlas, drawn with `Sprites::premul`) as one vertex strip per
/// mip layer, `T` and the mist colour from `mist` per row, in the vertex colour and
/// `normal` slot: exact fogging of a premultiplied sprite, soft edges included
/// (`fog::premultiplied_material`).
#[allow(clippy::too_many_arguments)]
fn draw_spruce(
    view: &View,
    mist: &fog::Mist,
    sp: &Sprites,
    cx: f32,
    depth: f32,
    t: u64,
    wp: Vec2,
    light: f32,
    buf: &mut MeshBuf,
) {
    let r = |i: u64| spruce_rand(t, i);
    let (lean, qw, qh, ground, y0) = spruce_frame(view, sp, depth, t);
    let area = view.area;
    if cx + qw * 0.6 < area.x || cx - qw * 0.6 > area.x + area.w || y0 > area.y + area.h {
        return;
    }
    let lat = view.px_per_unit(depth);

    let variant = (r(6) * sp.variants as f32) as usize;
    let flip = r(7) < 0.5;
    let b = (0.8 + 0.2 * r(4)) * light;
    let warm = (r(5) - 0.5) * 0.16;
    let tint = [b * (1.0 + warm), b, b * (1.0 - warm)];
    // Mist softens shape with distance: pick the mip as if the tree were drawn smaller
    // the farther it stands (the linear-filtered upscale is the blur), blending the two
    // nearest levels so the softness grows smoothly instead of popping — the coarser
    // one opaque, the finer over it fading out.
    let (level, f) = sp.level_mix(qh / (1.0 + MIST_BLUR * depth * depth));

    // Per row: screen points (left, right) after the lean, transmittance, mist colour.
    let dist = wp.distance(view.pos).max(1e-3);
    let fade = tree_range_fade(dist);
    let (sn, cs) = lean.sin_cos();
    let rot = |x: f32, y: f32| {
        let (dx, dy) = (x - cx, y - ground);
        vec2(cx + dx * cs - dy * sn, ground + dx * sn + dy * cs)
    };
    let rows: [(Vec2, Vec2, f32, [f32; 3]); SPRUCE_ROWS] = std::array::from_fn(|i| {
        let s = i as f32 / (SPRUCE_ROWS - 1) as f32;
        let y = y0 + qh * s;
        let z = (ground - y) / lat;
        let dz = z - view.eye;
        let tr = mist.transmittance((dist * dist + dz * dz).sqrt(), view.eye, z.max(0.0)) * fade;
        (
            rot(cx - qw * 0.5, y),
            rot(cx + qw * 0.5, y),
            tr,
            mist.colour(dz / dist),
        )
    });
    let (tw, th) = (sp.tex.width(), sp.tex.height());
    let exact = sp.premul.is_some();
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    let mut layer = |level: usize, w: f32| {
        let src = sp.src(variant, level, false);
        let (mut u0, mut u1) = (src.x / tw, (src.x + src.w) / tw);
        if flip {
            std::mem::swap(&mut u0, &mut u1);
        }
        let strip: [[Vertex; 2]; SPRUCE_ROWS] = std::array::from_fn(|i| {
            let (l, rt, tr, fc) = rows[i];
            let v = (src.y + src.h * i as f32 / (SPRUCE_ROWS - 1) as f32) / th;
            // Without the material (it failed to build): plain alpha blending, unfogged.
            let (k, fogw) = if exact {
                (tr * w, (1.0 - tr) * w)
            } else {
                (1.0, 0.0)
            };
            let c = [0, 1, 2].map(|j| q(tint[j] * k));
            let vx = |p: Vec2, u: f32| Vertex {
                position: vec3(p.x, p.y, 0.0),
                uv: vec2(u, v),
                color: [c[0], c[1], c[2], q(w)],
                normal: vec4(fc[0] * fogw, fc[1] * fogw, fc[2] * fogw, 0.0),
            };
            [vx(l, u0), vx(rt, u1)]
        });
        buf.strip(&strip);
    };
    if f > 0.02 {
        layer(level + 1, 1.0);
    }
    if f < 0.98 {
        layer(level, 1.0 - f);
    }
}

// ── Garden hedge clumps ──────────────────────────────────────────────────────────

/// Beyond this depth a hedge is drawn as a plain face, without clumps.
const CLUMP_RANGE: f32 = 9.0;
/// Where the hedge core starts easing from shaded interior back to a lit face.
const CLUMP_FADE: f32 = 6.0;
/// How far behind the face plane the hedge's shaded interior sits.
const CORE_RECESS: f32 = 0.09;
/// Height of the interior's top edge — the clipped, level top; the top row of clumps
/// only just crowns it.
const CORE_TOP: f32 = 0.93;
/// A hedge's top as a plain face, past clump range.
const HEDGE_TOP: f32 = 0.96;
/// No clump reaches further than this out of its face into the walkway: a trimmed
/// hedge's face is nearly flat.
const MAX_PROTRUDE: f32 = 0.06;
const CLUMP_ROWS: usize = 5;
const CLUMP_COLS: usize = 4;

/// Outward normal of face `f` of a grid square: 0 east, 1 west, 2 south, 3 north.
const FACE_N: [IVec2; 4] = [
    IVec2::new(1, 0),
    IVec2::new(-1, 0),
    IVec2::new(0, 1),
    IVec2::new(0, -1),
];

/// A face's corner (u = 0), its along-the-face unit vector (u: 0..1) and its normal.
fn face_frame(sq: IVec2, f: u8) -> (Vec2, Vec2, Vec2) {
    let n = FACE_N[f as usize];
    let along = if n.x != 0 {
        vec2(0.0, 1.0)
    } else {
        vec2(1.0, 0.0)
    };
    (
        sq.as_vec2() + n.max(IVec2::ZERO).as_vec2(),
        along,
        n.as_vec2(),
    )
}

/// Which face of the struck square a ray entered through (see `FACE_N`).
fn face_of(side: i32, ray: Vec2) -> u8 {
    match (side, if side == 0 { ray.x } else { ray.y } > 0.0) {
        (0, true) => 1,
        (0, false) => 0,
        (_, true) => 3,
        _ => 2,
    }
}

/// Two faces lying in one plane and touching: parts of one continuous hedge side.
fn same_surface(a: (IVec2, u8), b: (IVec2, u8)) -> bool {
    a.1 == b.1
        && if a.1 < 2 {
            a.0.x == b.0.x && (a.0.y - b.0.y).abs() <= 1
        } else {
            a.0.y == b.0.y && (a.0.x - b.0.x).abs() <= 1
        }
}

/// One leaf clump on a hedge face, in the face's frame: `u` along it, height `z`,
/// centre `n` out of the face plane (negative = sunk into the hedge), radius `r`.
#[derive(Clone, Copy, Debug)]
struct Clump {
    u: f32,
    z: f32,
    n: f32,
    r: f32,
    bits: u64,
}

/// A hedge face's clumps: a staggered 5-row by 4 grid of small clumps (r ~0.1), barely
/// jittered — a clipped hedge — each set a little deeper or shallower (fronts 0.01–0.06
/// out of the face), the top row level at one height so the top reads as a sheared
/// line with only small lumps; from a hash of (square, face), stable frame to frame. `cont`:
/// the face carries on into the next square along +u, so the odd rows' end clump may
/// straddle the seam; otherwise it's pulled in, so a hedge end is ragged but never
/// pokes more than `MAX_PROTRUDE` into the walkway.
fn hedge_clumps(sq: IVec2, f: u8, cont: bool) -> [Clump; CLUMP_ROWS * CLUMP_COLS] {
    let h = mix64(
        ((sq.x as u32 as u64) << 32 | sq.y as u32 as u64)
            ^ (f as u64 + 1).wrapping_mul(0x6A09_E667_F3BC_C909),
    );
    std::array::from_fn(|i| {
        let t = mix64(h ^ (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let q = |k: u32| ((t >> (k * 10)) & 0x3FF) as f32 / 1023.0;
        let (row, col) = (i / CLUMP_COLS, i % CLUMP_COLS);
        let top = row + 1 == CLUMP_ROWS;
        let u = (2 * col + 1 + row % 2) as f32 / 8.0 + (q(0) - 0.5) * 0.05;
        let u = if cont { u } else { u.min(0.9) };
        let r = if top {
            0.105 + 0.012 * q(1)
        } else {
            0.095 + 0.03 * q(1)
        };
        let n = MAX_PROTRUDE - r - 0.05 * q(2);
        let jig = if top { 0.02 } else { 0.06 };
        let z = ([0.10, 0.29, 0.48, 0.67, 0.86][row] + (q(3) - 0.5) * jig).max(r * 0.8);
        Clump {
            u,
            z,
            n,
            r,
            bits: t,
        }
    })
}

/// One clump, projected: screen centre/half-size, depth of its centre and its front.
struct ClumpSprite {
    depth: f32,
    front: f32,
    cx: f32,
    cy: f32,
    half: f32,
    z: f32,
    r: f32,
    owner: (IVec2, u8),
    variant: usize,
    flip: bool,
    tint: Color,
    fog: Color,
}

/// Garden hedges as clipped volumes of foliage: every exposed, camera-facing hedge face
/// within `CLUMP_RANGE` carries 20 small leaf clumps (`hedge_clumps`) at slightly
/// different depths (at most `MAX_PROTRUDE` into the walkway), the top row just crowning
/// the level core top. Each is a billboard of one of
/// the theme's clump sprites, drawn far→near, so near clumps overlap far ones and their
/// outlines shift against each other as the camera moves; the recessed core
/// (`Profile::Core`) shows through the gaps as the hedge's dark inside.
///
/// Depth test per screen column, against the wall pass: a clump draws where its front
/// is nearer than the wall there, or where that wall is its own continuous hedge side
/// (a flat billboard against a face seen at a grazing angle would otherwise be cut in
/// half). Behind a nearer wall only the part above that wall's top edge draws, so a
/// farther hedge's crown still peeks over a nearer one. Columns a clump covers at eye
/// height pull `zbuf` in to its front, for the billboards drawn after (keys, the exit,
/// motes). Mirror squares carry no clumps, so the glass stays clear.
///
/// In a mirror (`mirror` = which real points lie in front of the glass, and the bounce
/// level): `view` is the
/// camera's mirror image, `hedge` holds the reflected columns only, clumps are tinted
/// like the glass and their sprites flipped, so the reflection shows the same clipped
/// volumes as the real hedges.
fn draw_clumps(
    view: &View,
    grid: &WallGrid,
    sp: &Sprites,
    pal: &Palette,
    light: &Lighting,
    hedge: &HedgeCols,
    mirror: Option<(&dyn Fn(Vec2) -> bool, usize)>,
) -> Vec<(usize, f32)> {
    let glass = glass_tint(mirror.map_or(0, |m| m.1));
    // An odd number of reflections shows every sprite flipped.
    let flipped = mirror.is_some_and(|m| m.1 % 2 == 1);
    let reach = CLUMP_RANGE as i32 + 1;
    let (px, py) = (view.pos.x.floor() as i32, view.pos.y.floor() as i32);
    let hedge_at = |q: IVec2| grid.is_solid(q.x, q.y) && !grid.is_mirror(q.x, q.y);
    let mut list: Vec<ClumpSprite> = Vec::with_capacity(512);
    for gy in (py - reach).max(0)..=(py + reach).min(grid.gh as i32 - 1) {
        for gx in (px - reach).max(0)..=(px + reach).min(grid.gw as i32 - 1) {
            let sq = ivec2(gx, gy);
            if !hedge_at(sq) || grid.door(gx, gy).is_some() {
                continue;
            }
            for f in 0..4u8 {
                let out = FACE_N[f as usize];
                let open = sq + out;
                if grid.is_solid(open.x, open.y) {
                    continue; // buried inside the hedge
                }
                let (origin, along, nrm) = face_frame(sq, f);
                // Facing away: the hedge's own body hides it.
                if (view.pos - origin).dot(nrm) <= 0.0
                    || !face_in_view(view, hedge, origin, origin + along)
                {
                    continue;
                }
                let next = sq + along.as_ivec2();
                let cont = hedge_at(next) && !grid.is_solid(next.x + out.x, next.y + out.y);
                let face_k = if f < 2 { pal.ew_tint } else { 1.0 };
                let bloom = face_bloom(sq, f, sp.variants);
                for c in hedge_clumps(sq, f, cont) {
                    let p = origin + along * c.u + nrm * c.n;
                    if mirror.is_some_and(|(keep, _)| !keep(p)) {
                        continue;
                    }
                    let Some((cx, depth)) = view.project(p) else {
                        continue;
                    };
                    let half = c.r * CLUMP_SPAN * view.px_per_unit(depth);
                    if !(0.3..CLUMP_RANGE).contains(&depth) || half < 1.0 {
                        continue;
                    }
                    let lit = lit_at(depth, pal.haze, light.at(p));
                    let q = |k: u32| ((c.bits >> (k * 8)) & 0xFF) as f32 / 255.0;
                    // Skylight grows with height; clumps sunk into the hedge are shaded
                    // by the ones around them. Same lit falloff as the wall strips.
                    let k = lit
                        * face_k
                        * (0.72 + 0.32 * c.z)
                        * (1.0 + 2.5 * c.n)
                        * (0.88 + 0.12 * q(5));
                    let warm = (q(6) - 0.5) * 0.14;
                    list.push(ClumpSprite {
                        depth,
                        front: depth - c.r,
                        cx,
                        cy: view.screen_y(c.z, depth),
                        half,
                        z: c.z,
                        r: c.r,
                        owner: (sq, f),
                        variant: match bloom {
                            Some((kind, share)) if ((c.bits >> 40) & 0x3FF) < share => {
                                bloom_variant(kind, (c.bits >> 52) as usize)
                            }
                            _ => (c.bits >> 50) as usize % CLUMPS.min(sp.variants),
                        },
                        flip: (c.bits >> 60 & 1 == 1) != flipped,
                        tint: Color::new(
                            (k * (1.0 + warm)).min(1.0) * glass[0],
                            k.min(1.0) * glass[1],
                            (k * (1.0 - warm)).min(1.0) * glass[2],
                            1.0,
                        ),
                        fog: Color::new(pal.fog.r, pal.fog.g, pal.fog.b, 1.0 - lit),
                    });
                }
            }
        }
    }
    crate::util::sort_by_key(&mut list, |s| -s.depth);
    let mut near = Vec::with_capacity(1024);
    for s in &list {
        draw_clump(view, sp, hedge, s, &mut near);
    }
    near
}

/// A hedge face in flower: which bloom kind (`theme::bloom_variant`) and how many of
/// its clumps carry it (out of 1024), from a hash of (square, face) — so a stretch of
/// hedge flowers in one colour, like a real rose bush, the same in every frame and in
/// every mirror. About half the faces flower; roses (red, white, half painted) lead.
/// `None` for a plain face, or an atlas without flowering variants.
fn face_bloom(sq: IVec2, f: u8, variants: usize) -> Option<(usize, u64)> {
    if variants < CLUMPS + BLOOM_KINDS * BLOOM_ALTS {
        return None;
    }
    let h = mix64(
        ((sq.x as u32 as u64) << 32 | sq.y as u32 as u64)
            ^ (f as u64 + 11).wrapping_mul(0xA54F_F53A_5F1D_36F1),
    );
    if h & 0xFF >= 128 {
        return None;
    }
    // Weights: red roses 3, white 2, pink 1.5, yellow 1, half-painted 2.5 (of 10).
    let w = ((h >> 8) & 0x3FF) as f32 / 1024.0 * 10.0;
    let kind = [3.0, 5.0, 6.5, 7.5, 10.0]
        .iter()
        .position(|&edge| w < edge)
        .unwrap_or(4);
    Some((kind, 420 + (h >> 20) % 400))
}

/// Whether any clump of the face `a`–`b` could show: some column in its screen span
/// (padded by how far clumps spill) has its wall behind the face, or a top edge low
/// enough for the face's clumps to rise above it.
fn face_in_view(view: &View, hedge: &HedgeCols, a: Vec2, b: Vec2) -> bool {
    const NEAR: f32 = 0.15;
    let (mut ca, mut cb) = (view.cam(a), view.cam(b));
    if ca.y < NEAR && cb.y < NEAR {
        return false;
    }
    if ca.y < NEAR {
        ca += (cb - ca) * ((NEAR - ca.y) / (cb.y - ca.y));
    } else if cb.y < NEAR {
        cb += (ca - cb) * ((NEAR - cb.y) / (ca.y - cb.y));
    }
    let x = |c: Vec2| view.area.w * 0.5 * (1.0 + c.x / c.y);
    let dmin = ca.y.min(cb.y);
    let pad = 0.2 * view.px_per_unit(dmin);
    let c0 = (((x(ca).min(x(cb)) - pad) / COL_STEP).floor() as isize).max(0);
    let c1 = (((x(ca).max(x(cb)) + pad) / COL_STEP) as isize).min(view.zbuf.len() as isize - 1);
    let peak = view.screen_y(1.02, dmin);
    (c0..=c1).any(|c| view.zbuf[c as usize] > dmin - 0.3 || hedge.ztop[c as usize] > peak)
}

/// One clump, column-clipped: runs of columns where it's wholly in front are one quad
/// each; a column where it's behind the wall draws only its part above the wall's top.
/// Each piece is the colour sprite, then its silhouette in the fog colour.
fn draw_clump(
    view: &View,
    sp: &Sprites,
    hedge: &HedgeCols,
    s: &ClumpSprite,
    near: &mut Vec<(usize, f32)>,
) {
    let area = view.area;
    let size = s.half * 2.0;
    let (x0, y0) = (s.cx - s.half, s.cy - s.half);
    let level = sp.level_for(size);
    let (src, sil) = (
        sp.src(s.variant, level, false),
        sp.src(s.variant, level, true),
    );
    let c0 = (((x0 - area.x) / COL_STEP).floor() as isize).max(0);
    let c1 = (((x0 + size - area.x) / COL_STEP) as isize).min(view.zbuf.len() as isize - 1);
    // Covers eye height, where keys, the exit and motes are depth-tested.
    let eye_band = s.z - s.r * 0.7 < 0.6 && s.z + s.r * 0.7 > 0.3;
    // Per column: `None` hidden, else the screen y to clip the clump's bottom at.
    let clip = |c: isize| -> Option<f32> {
        let c = c as usize;
        if s.front < view.zbuf[c] || hedge.face[c].is_some_and(|h| same_surface(h, s.owner)) {
            Some(f32::INFINITY)
        } else {
            (y0 < hedge.ztop[c]).then_some(hedge.ztop[c])
        }
    };
    let mut c = c0;
    while c <= c1 {
        let Some(bot) = clip(c) else {
            c += 1;
            continue;
        };
        let mut end = c;
        if bot.is_infinite() {
            while end < c1 && clip(end + 1).is_some_and(f32::is_infinite) {
                end += 1;
            }
            if eye_band {
                for k in c..=end {
                    let mid = area.x + (k as f32 + 0.5) * COL_STEP;
                    if (mid - s.cx).abs() < s.half * 0.6 {
                        near.push((k as usize, s.front));
                    }
                }
            }
        }
        let xa = (area.x + c as f32 * COL_STEP).max(x0);
        let xb = (area.x + (end + 1) as f32 * COL_STEP).min(x0 + size);
        let yb = bot.min(y0 + size);
        if xb > xa && yb > y0 {
            let (fx0, fx1, fy) = ((xa - x0) / size, (xb - x0) / size, (yb - y0) / size);
            // A flipped sprite's screen-left edge is its source's right edge.
            let (u0, u1) = if s.flip {
                (1.0 - fx1, 1.0 - fx0)
            } else {
                (fx0, fx1)
            };
            for (r, color) in [(src, s.tint), (sil, s.fog)] {
                if color.a < 0.01 {
                    continue;
                }
                draw_texture_ex(
                    &sp.tex,
                    xa,
                    y0,
                    color,
                    DrawTextureParams {
                        dest_size: Some(vec2(xb - xa, yb - y0)),
                        source: Some(Rect::new(r.x + u0 * r.w, r.y, (u1 - u0) * r.w, fy * r.h)),
                        flip_x: s.flip,
                        ..Default::default()
                    },
                );
            }
        }
        c = end + 1;
    }
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Wall/sprite brightness at distance `perp`: light falls off with distance, faster
/// under `haze`; `1 - lit` is how much fog colour washes over it.
/// Casts world rectangles on the ceiling plane (z = 1): each is clipped to the near
/// plane in camera space and projected as a flat polygon — straight world edges stay
/// straight on screen, so no per-pixel cast is needed. Drawn before the walls, which
/// overdraw the rest (and everything else is nearer than the ceiling anyway).
struct CeilingCast<'a> {
    view: &'a View,
    inv_det: f32,
}

impl CeilingCast<'_> {
    const NEAR: f32 = 0.05;

    fn new(view: &View) -> CeilingCast<'_> {
        CeilingCast {
            view,
            inv_det: 1.0 / (view.plane.x * view.dir.y - view.dir.x * view.plane.y),
        }
    }

    /// World point -> (lateral, depth) in camera space; screen x = mid * (1 + lat / depth).
    fn cam(&self, p: Vec2) -> Vec2 {
        let (v, r) = (self.view, p - self.view.pos);
        vec2(
            self.inv_det * (v.dir.y * r.x - v.dir.x * r.y),
            self.inv_det * (-v.plane.y * r.x + v.plane.x * r.y),
        )
    }

    /// Depth of world point `p`, never nearer than the near plane.
    fn depth(&self, p: Vec2) -> f32 {
        self.cam(p).y.max(Self::NEAR)
    }

    /// Would the rectangle `min..max` land on screen at all?
    fn in_view(&self, min: Vec2, max: Vec2) -> bool {
        let q = [min, vec2(max.x, min.y), max, vec2(min.x, max.y)].map(|p| self.cam(p));
        !(q.iter().all(|c| c.y < Self::NEAR)
            || q.iter().all(|c| c.x > c.y.abs())
            || q.iter().all(|c| -c.x > c.y.abs()))
    }

    fn fill(&self, min: Vec2, max: Vec2, col: Color) {
        let quad = [min, vec2(max.x, min.y), max, vec2(min.x, max.y)].map(|p| self.cam(p));
        // Sutherland-Hodgman against depth >= NEAR (one plane, <= 5 vertices out).
        let mut poly = [Vec2::ZERO; 5];
        let mut n = 0;
        for i in 0..4 {
            let (a, b) = (quad[i], quad[(i + 1) % 4]);
            if a.y >= Self::NEAR {
                poly[n] = a;
                n += 1;
            }
            if (a.y >= Self::NEAR) != (b.y >= Self::NEAR) {
                poly[n] = a + (b - a) * ((Self::NEAR - a.y) / (b.y - a.y));
                n += 1;
            }
        }
        if n < 3 {
            return;
        }
        let area = self.view.area;
        let to_screen = |c: Vec2| {
            vec2(
                area.x + area.w * 0.5 * (1.0 + c.x / c.y),
                self.view.screen_y(1.0, c.y),
            )
        };
        let p0 = to_screen(poly[0]);
        for i in 1..n - 1 {
            draw_triangle(p0, to_screen(poly[i]), to_screen(poly[i + 1]), col);
        }
    }

    /// Every `size` square of a grid laid from `off`, within `range` of the camera and
    /// in view: `(gx, gy, min corner)`.
    fn squares(&self, range: f32, off: f32, size: f32, mut each: impl FnMut(i32, i32, Vec2)) {
        let r = (range / size) as i32;
        let at = |v: f32| ((v - off) / size).floor() as i32;
        let (cx, cy) = (at(self.view.pos.x), at(self.view.pos.y));
        for gy in cy - r..=cy + r {
            for gx in cx - r..=cx + r {
                let o = vec2(gx as f32 * size + off, gy as f32 * size + off);
                if self.in_view(o, o + Vec2::splat(size)) {
                    each(gx, gy, o);
                }
            }
        }
    }
}

/// 0..1 hash of a grid square (no global RNG).
fn tile_hash(gx: i32, gy: i32, k: u64) -> f32 {
    let mut h = (gx as i64 as u64).wrapping_mul(0x9E3779B97F4A7C15);
    h ^= (gy as i64 as u64).wrapping_mul(0xD1B54A32D192ED03);
    h ^= k.wrapping_mul(0xC2B2AE3D27D4EB4F);
    h = h.wrapping_mul(0x2545F4914F6CDD1D);
    ((h >> 40) & 0xFF) as f32 / 255.0
}

/// Dungeon vault: one flagstone per grid square on the ceiling plane, inset so the
/// palette gradient behind shows through as mortar.
/// Among `thin` walls the slabs are three to a cell, wall middle to wall middle, so a
/// joint never runs along a corridor's middle or just beside a wall.
fn draw_stone_ceiling(view: &View, pal: &Palette, light: &Lighting, thin: bool) {
    const INSET: f32 = 0.035;
    let cast = CeilingCast::new(view);
    let (off, size) = if thin { (0.5, 2.0 / 3.0) } else { (0.0, 1.0) };
    cast.squares(12.0, off, size, |gx, gy, o| {
        let centre = o + Vec2::splat(size * 0.5);
        let lit = lit_at(cast.depth(centre), pal.haze, light.at(centre));
        let jitter = 0.8 + 0.35 * tile_hash(gx, gy, 0);
        let k = lit * jitter * 0.75;
        let col = Color::new(0.30 * k, 0.28 * k, 0.26 * k, 1.0);
        cast.fill(o + Vec2::splat(INSET), o + Vec2::splat(size - INSET), col);
    });
}

/// The office's drop ceiling: acoustic tiles in a thin T-bar grid (the palette
/// gradient between them), from wall middle to wall middle, six to a cell — a light
/// panel fills the four round each cell's centre. Tiles pick up the light: brighter
/// near a lit panel, darker by a dead one (`Lighting::at`), fading into the murk with
/// distance like the walls. A few are water-stained. A flat gradient overhead read as
/// a painted backdrop, and the panels, drawn as screen-facing trapezoids, didn't turn
/// with the view.
fn draw_office_ceiling(view: &View, pal: &Palette, light: &Lighting) {
    const TILE: f32 = 1.0 / 3.0;
    const BAR: f32 = 0.006;
    const RANGE: f32 = 9.0;
    const ACOUSTIC: [f32; 3] = [0.84, 0.81, 0.62];
    const STAIN: [f32; 3] = [0.66, 0.57, 0.36];
    let cast = CeilingCast::new(view);
    let shade = |rgb: [f32; 3], k: f32, p: Vec2| {
        let d = cast.depth(p);
        let (lit, wash) = (lit_at(d, pal.haze, light.at(p)), fog_at(d, pal.haze));
        Color::new(
            rgb[0] * k * lit + pal.fog.r * wash,
            rgb[1] * k * lit + pal.fog.g * wash,
            rgb[2] * k * lit + pal.fog.b * wash,
            1.0,
        )
    };
    cast.squares(RANGE, 0.5, TILE, |gx, gy, o| {
        let centre = o + Vec2::splat(TILE * 0.5);
        // Off the nearest cell centre (cells are 2 apart, centred on odd + 0.5).
        let from_panel =
            ((centre - Vec2::splat(1.5) + 1.0).rem_euclid(Vec2::splat(2.0)) - 1.0).length();
        let pool = 0.86 + 0.2 * (1.0 - from_panel / 1.2).max(0.0);
        let k = pool * (0.96 + 0.08 * tile_hash(gx, gy, 1));
        let rgb = if tile_hash(gx, gy, 2) < 0.03 {
            STAIN
        } else {
            ACOUSTIC
        };
        cast.fill(
            o + Vec2::splat(BAR),
            o + Vec2::splat(TILE - BAR),
            shade(rgb, k, centre),
        );
    });
    // The panels, set in the grid: a steel frame round a diffuser, glowing when lit.
    for &(p, on) in &light.panels {
        let (min, max) = (p - Vec2::splat(2.0 * TILE), p + Vec2::splat(2.0 * TILE));
        if p.distance(view.pos) > RANGE || !cast.in_view(min, max) {
            continue;
        }
        let frame = shade([0.62, 0.6, 0.5], 1.0, p);
        cast.fill(
            p - Vec2::splat(TILE - BAR),
            p + Vec2::splat(TILE - BAR),
            frame,
        );
        let face = if on {
            // Emissive: dimmed a little by the haze, never darkened like a surface.
            let lit = lit_at(cast.depth(p), pal.haze, 1.0).max(0.7);
            let c = [1.0, 1.0, 0.93];
            Color::new(
                c[0] * lit + pal.fog.r * (1.0 - lit),
                c[1] * lit + pal.fog.g * (1.0 - lit),
                c[2] * lit + pal.fog.b * (1.0 - lit),
                1.0,
            )
        } else {
            shade([0.55, 0.54, 0.47], 1.0, p)
        };
        cast.fill(
            p - Vec2::splat(TILE - 0.045),
            p + Vec2::splat(TILE - 0.045),
            face,
        );
    }
}

/// Brightness of a surface `perp` away: `1 - lit` is how much fog colour washes over it.
/// Indoors (and the Garden) light falls off with distance, faster under `haze`, and
/// `light_mul` dims it. A misty palette (`haze` ≥ `fog::MIST_HAZE`, the Forest) returns
/// the mist's level transmittance instead — exponential, and never darkened by the
/// light (lighting belongs to the object's own colour, not to how much fog is in front
/// of it) — so shared callers like the campfire fade exactly as the trees round it do.
pub(crate) fn lit_at(perp: f32, haze: f32, light_mul: f32) -> f32 {
    if haze >= fog::MIST_HAZE {
        return fog::FOREST.level(perp);
    }
    (light_mul / (1.0 + haze * (perp * 0.14 + perp * perp * 0.02))).clamp(0.05, 1.0)
}

/// How much fog colour washes over a surface `perp` away — distance alone. Light (a
/// torch, a dark `Blinky` panel) goes in the surface's own brightness (`lit_at`), never
/// the fog: a panel gone dark turned its stretch of office the fog's yellow-grey.
pub(crate) fn fog_at(perp: f32, haze: f32) -> f32 {
    1.0 - lit_at(perp, haze, 1.0)
}

/// How a point at `p`, height `z`, `depth` away fades: `(lit, fog colour)` — on the
/// palette's mist if it has one (true distance and height, the ray's elevation picking
/// the colour), else `lit_at` toward the flat palette fog. For billboards drawn as flat
/// colours (critters).
pub(crate) fn fade_at(
    view: &View,
    pal: &Palette,
    p: Vec2,
    z: f32,
    depth: f32,
    light: f32,
) -> (f32, Color) {
    match pal.mist {
        Some(m) => {
            let dist = p.distance(view.pos).max(1e-3);
            let dz = z - view.eye;
            let t = m.transmittance((dist * dist + dz * dz).sqrt(), view.eye, z);
            (t, rgba(m.colour(dz / dist), 1.0))
        }
        None => (lit_at(depth, pal.haze, light), pal.fog),
    }
}

/// The bot's own reflection behind one glass (`m`, at any bounce level), one screen
/// column at a time. `m.image` is the camera that sees the real world through that
/// glass, so it looks at the real bot — its real position and facing — and the
/// ordinary projection puts the figure exactly where the glass shows it, handedness
/// mirrored for free. Drawn only where it lies past the glass and nearer than whatever
/// is behind it in this column (`far`: the next glass, or the reflected wall), tinted
/// by `glass` (`glass_tint` of the level).
///
/// It's posed from the bot's real state, not a flat sprite: when the bot glances aside
/// the reflection turns and shows its side (the visor slides off-centre, then
/// disappears when it faces away); its legs lift and arms swing with `stride`; walking
/// or bumping moves it.
#[allow(clippy::too_many_arguments)]
fn draw_reflection(
    view: &View,
    sx: f32,
    m: &MirrorHit,
    far: f32,
    glass: [f32; 3],
    pal: &Palette,
    light: f32,
) {
    let mv = view.mirrored(m.image, m.flip);
    let bot = view.pos;
    let Some((cx, depth)) = mv.project(bot) else {
        return;
    };
    // Past the glass, not merely past its grid line elsewhere: a deeper image can see
    // the bot's side of that line through a different stretch of the maze.
    if depth <= m.dist || depth >= far {
        return;
    }
    let lat = mv.px_per_unit(depth);
    let xw = (sx + COL_STEP * 0.5 - cx) / lat;
    if xw.abs() > 0.2 {
        return;
    }

    // The bot's body frame.
    let facing = view.dir;
    let side = vec2(-facing.y, facing.x);
    // This column's line of sight through the figure: along `u` (image -> bot), offset
    // `xw` toward screen-right. The figure is small, so a parallel ray per column is
    // plenty.
    let u = (bot - mv.pos).normalize_or_zero();
    let mut across = vec2(-u.y, u.x);
    if across.dot(mv.plane) < 0.0 {
        across = -across;
    }
    let origin = bot + across * xw - u;

    let lit = lit_at(depth, pal.haze, light);
    let fog = view.fog(pal, depth);
    // Same shading as the wall strips (darkened by distance, tinted by the glass, then
    // a fog wash of `1 - lit`), folded into each part's own colour — a fog *box* over
    // the whole figure double-fogged the reflected background between its limbs.
    let tint = |c: [f32; 3], k: f32| {
        let (f, g) = (1.0 - lit, lit * lit * k);
        Color::new(
            c[0] * g * glass[0] + fog[0] * f,
            c[1] * g * glass[1] + fog[1] * f,
            c[2] * g * glass[2] + fog[2] * f,
            1.0,
        )
    };
    let rise = view.eye - 0.5;
    let stride = view.stride;
    let bar = |z0: f32, z1: f32, col: Color| {
        let top = mv.screen_y(z1 + rise, depth).max(view.area.y);
        let bot = mv.screen_y(z0 + rise, depth).min(view.area.y + view.area.h);
        if bot > top {
            draw_rectangle(sx, top, COL_STEP + 0.6, bot - top, col);
        }
    };

    // Each hit: (entry distance, part, surface normal there in (side, facing) terms,
    // side coordinate there, the heights this column sees it span).
    let mut hits: [PartHit; ROBOT.len()] = [(0.0, 0, Vec2::ZERO, 0.0, (0.0, 0.0)); ROBOT.len()];
    let mut n = 0;
    let (ds, df) = (u.dot(side), u.dot(facing));
    for (i, part) in ROBOT.iter().enumerate() {
        let (ls, lf) = part.pose(stride);
        let rel = origin - (bot + side * ls + facing * lf);
        // Where it's widest, in the part's frame scaled so its footprint is the unit
        // circle, the line of sight is `p + d t`: solve |p + d t| = 1 for its entry.
        let wide = part.taper.max(1.0);
        let (hs, hf) = (part.hs * wide, part.hf * wide);
        let p = vec2(rel.dot(side) / hs, rel.dot(facing) / hf);
        let d = vec2(ds / hs, df / hf);
        let (a, b) = (d.dot(d), p.dot(d));
        // How far the line passes from the part's axis, as a fraction of the base
        // footprint (the widest one is `wide`).
        let miss = (p.dot(p) - b * b / a).max(0.0).sqrt() * wide;
        if miss >= wide {
            continue;
        }
        let t = (-b - (b * b - a * (p.dot(p) - 1.0)).max(0.0).sqrt()) / a;
        let e = p + d * t;
        let nrm = vec2(e.x / hs, e.y / hf).normalize_or_zero();
        hits[n] = (t, i, nrm, e.x * hs, part.span(miss));
        n += 1;
    }
    let hits = &mut hits[..n];
    // Far to near, so nearer parts paint over the ones behind them in this column.
    crate::util::sort_by_key(hits, |t| -t.0);
    for &(_, i, nrm, s_at, (z0, z1)) in hits.iter() {
        let part = &ROBOT[i];
        let lift = part.lift(stride);
        let (z0, z1) = (z0 + lift, z1 + lift);
        // Lit face-on, darker toward the silhouette; a front a little brighter than a
        // back — so a turn reads as the body swinging round, not just a sliding visor.
        let view_k = -(nrm.x * ds + nrm.y * df);
        let k = (0.45 + 0.55 * view_k.max(0.0)) * (0.8 + 0.2 * nrm.y);
        let rgb = part.mat.rgb();
        bar(z0, z1, tint(rgb, k));
        if part.cap > 0.0 {
            // Underside in shadow: the light is overhead.
            bar(z0, z0 + (z1 - z0) * 0.3, tint(rgb, k * 0.78));
        }
        match part.mat {
            Mat::Head if nrm.y > 0.3 && s_at.abs() < 0.027 => {
                // Glossy black visor, rounded at its ends, a thin light bar across it —
                // it glows, no dimming.
                let h = 0.03 * (1.0 - (s_at / 0.027).powi(2)).sqrt();
                bar(0.503 - h, 0.503 + h, tint(Mat::Plate.rgb(), 1.0));
                if s_at.abs() < 0.021 {
                    bar(0.497, 0.507, Color::new(0.45, 0.92, 1.0, 1.0));
                }
            }
            _ => {}
        }
    }
}

/// A part a column's line of sight struck: (entry distance, `ROBOT` index, surface
/// normal in (side, facing) terms, side coordinate at entry, the heights it spans).
type PartHit = (f32, usize, Vec2, f32, (f32, f32));

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mat {
    /// Matte white plating.
    Shell,
    /// Black actuators / joints / waist.
    Joint,
    /// The head shell (plated, with the visor on its front).
    Head,
    Plate,
}

impl Mat {
    fn rgb(self) -> [f32; 3] {
        match self {
            Mat::Shell | Mat::Head => [0.88, 0.89, 0.92],
            Mat::Joint => [0.13, 0.13, 0.15],
            Mat::Plate => [0.03, 0.03, 0.05],
        }
    }
}

/// Which limb a part swings with.
#[derive(Clone, Copy)]
enum Limb {
    Still,
    /// Legs: `side` is `+1` the left, `-1` the right — the one whose side of `stride`
    /// is lifted; `k` scales how high (the foot most, the thigh half as much).
    Leg(f32, f32),
    /// Arms swing opposite the leg on their own side; `k` scales the reach (the hand
    /// swings furthest).
    Arm(f32, f32),
}

/// One rounded part of the robot: an elliptic cylinder centred `s` along its side
/// axis and `f` along its facing, half-extents `hs` x `hf` at its foot, spanning
/// heights `z0..z1` and capped top and bottom by half-ellipsoids `cap` tall (eye level
/// is 0.5). `taper` scales the footprint at the top (a chest widening to the shoulders,
/// a thigh to the hip), linearly in between. A ball is `z0 == z1` with `cap` = its
/// radius. Ray-tested per column in the floor plane like the old boxes, just against
/// an ellipse instead of two slabs.
struct Part {
    s: f32,
    f: f32,
    hs: f32,
    hf: f32,
    z0: f32,
    z1: f32,
    cap: f32,
    taper: f32,
    mat: Mat,
    limb: Limb,
}

impl Part {
    /// The heights a column sees the part span when its line of sight passes `miss`
    /// (a fraction of the foot's footprint) from the axis. A column near the edge of a
    /// tapered part sees only its wide end, so the silhouette slopes smoothly; the
    /// column's vertical plane cuts each rounded cap as an ellipse, shrinking toward
    /// the silhouette, which is what makes the ends read as round.
    fn span(&self, miss: f32) -> (f32, f32) {
        let cap = |w: f32| {
            let r = miss / w;
            (r < 1.0).then(|| self.cap * (1.0 - r * r).sqrt())
        };
        // Where the footprint (scale 1 at z0, `taper` at z1) is just `miss` wide.
        let edge = |over: bool| {
            let k = if self.taper == 1.0 {
                if over { 1.0 } else { 0.0 }
            } else {
                ((miss - 1.0) / (self.taper - 1.0)).clamp(0.0, 1.0)
            };
            self.z0 + (self.z1 - self.z0) * k
        };
        let lo = match cap(1.0) {
            Some(c) => self.z0 - c,
            None => edge(false),
        };
        let hi = match cap(self.taper) {
            Some(c) => self.z1 + c,
            None => edge(true),
        };
        (lo, hi)
    }

    /// Centre offset `(side, facing)` for this frame's gait.
    fn pose(&self, stride: f32) -> (f32, f32) {
        let swing = match self.limb {
            Limb::Still => 0.0,
            Limb::Leg(side, _) => 0.035 * stride * side,
            Limb::Arm(side, k) => -0.03 * k * stride * side,
        };
        (self.s, self.f + swing)
    }

    /// How far the part is lifted off its rest height (a stepping leg).
    fn lift(&self, stride: f32) -> f32 {
        match self.limb {
            Limb::Leg(side, k) => 0.05 * k * (stride * side).max(0.0),
            _ => 0.0,
        }
    }
}

/// `at` = (side, facing) centre, `half` = the foot's footprint half-extents, `z` = the
/// straight span's heights, `cap` = the rounded ends' height, `taper` = the top's
/// footprint scale.
const fn part(
    at: (f32, f32),
    half: (f32, f32),
    z: (f32, f32),
    (cap, taper): (f32, f32),
    mat: Mat,
    limb: Limb,
) -> Part {
    Part {
        s: at.0,
        f: at.1,
        hs: half.0,
        hf: half.1,
        z0: z.0,
        z1: z.1,
        cap,
        taper,
        mat,
        limb,
    }
}

/// A ball joint of radius `r` centred at height `z`.
const fn ball(at: (f32, f32), z: f32, r: f32, mat: Mat, limb: Limb) -> Part {
    part(at, (r, r), (z, z), (r, 1.0), mat, limb)
}

/// The bot, sleek-humanoid-robot style: white plating over black joints, slender
/// tapered limbs, a smooth chest widening from a narrow waist to shoulders the upper
/// arms sit flush against, and a small rounded head that's all visor from the front.
/// Head centred on eye height (0.5), since the camera *is* its eyes. (The first version
/// was 19 boxes and read bulky and blocky; a ball-jointed one with an egg-shaped chest
/// and big shoulder spheres read grotesque — the torso wants long straight flanks and
/// small caps, not roundness.)
const ROBOT: [Part; 21] = {
    use Limb::*;
    use Mat::*;
    const LS: f32 = -0.038; // leg / arm side offsets, left
    const AS: f32 = -0.091;
    [
        // Legs: foot, shin, knee, thigh (left then right).
        part(
            (LS, 0.014),
            (0.02, 0.042),
            (0.01, 0.012),
            (0.01, 1.0),
            Joint,
            Leg(1.0, 1.0),
        ),
        part(
            (LS, 0.0),
            (0.014, 0.017),
            (0.04, 0.095),
            (0.016, 1.25),
            Shell,
            Leg(1.0, 1.0),
        ),
        ball((LS, 0.003), 0.119, 0.016, Joint, Leg(1.0, 0.75)),
        part(
            (LS, 0.0),
            (0.019, 0.022),
            (0.145, 0.195),
            (0.02, 1.2),
            Shell,
            Leg(1.0, 0.5),
        ),
        part(
            (-LS, 0.014),
            (0.02, 0.042),
            (0.01, 0.012),
            (0.01, 1.0),
            Joint,
            Leg(-1.0, 1.0),
        ),
        part(
            (-LS, 0.0),
            (0.014, 0.017),
            (0.04, 0.095),
            (0.016, 1.25),
            Shell,
            Leg(-1.0, 1.0),
        ),
        ball((-LS, 0.003), 0.119, 0.016, Joint, Leg(-1.0, 0.75)),
        part(
            (-LS, 0.0),
            (0.019, 0.022),
            (0.145, 0.195),
            (0.02, 1.2),
            Shell,
            Leg(-1.0, 0.5),
        ),
        // Pelvis, waist, then the chest: long flanks widening to the shoulders, small
        // rounded ends overlapping the waist below and the neck above, so the torso
        // reads as one smooth shell.
        part(
            (0.0, 0.0),
            (0.05, 0.03),
            (0.205, 0.222),
            (0.014, 1.0),
            Shell,
            Still,
        ),
        part(
            (0.0, 0.0),
            (0.028, 0.022),
            (0.235, 0.26),
            (0.01, 1.0),
            Joint,
            Still,
        ),
        part(
            (0.0, 0.003),
            (0.046, 0.032),
            (0.265, 0.395),
            (0.02, 1.55),
            Shell,
            Still,
        ),
        // Upper arm (its top rounded into the shoulder), elbow, forearm, hand (left
        // then right).
        part(
            (AS, 0.0),
            (0.014, 0.016),
            (0.305, 0.385),
            (0.02, 1.2),
            Shell,
            Arm(1.0, 1.0),
        ),
        ball((AS, 0.0), 0.29, 0.013, Joint, Arm(1.0, 1.4)),
        part(
            (AS, 0.0),
            (0.012, 0.014),
            (0.22, 0.272),
            (0.013, 1.2),
            Shell,
            Arm(1.0, 1.8),
        ),
        ball((AS, 0.004), 0.2, 0.012, Joint, Arm(1.0, 2.2)),
        part(
            (-AS, 0.0),
            (0.014, 0.016),
            (0.305, 0.385),
            (0.02, 1.2),
            Shell,
            Arm(-1.0, 1.0),
        ),
        ball((-AS, 0.0), 0.29, 0.013, Joint, Arm(-1.0, 1.4)),
        part(
            (-AS, 0.0),
            (0.012, 0.014),
            (0.22, 0.272),
            (0.013, 1.2),
            Shell,
            Arm(-1.0, 1.8),
        ),
        ball((-AS, 0.004), 0.2, 0.012, Joint, Arm(-1.0, 2.2)),
        // Neck and head.
        part(
            (0.0, 0.0),
            (0.012, 0.013),
            (0.418, 0.44),
            (0.006, 1.0),
            Joint,
            Still,
        ),
        part(
            (0.0, 0.006),
            (0.034, 0.04),
            (0.485, 0.515),
            (0.037, 1.0),
            Head,
            Still,
        ),
    ]
};

/// The glass itself, at the mirror's own distance: a faint sheen and a couple of
/// diagonal highlight streaks, so it reads as a silvered surface rather than
/// an opening onto a mirrored corridor.
fn draw_mirror_glass(
    view: &View,
    buf: &mut MeshBuf,
    c: usize,
    m: &MirrorHit,
    right: (f32, f32),
    pal: &Palette,
    light: f32,
) {
    let area = view.area;
    let xs = [
        area.x + c as f32 * COL_STEP,
        area.x + (c + 1) as f32 * COL_STEP,
    ];
    let dist = [m.dist, right.0].map(|d| d.max(0.0001));
    let us = [m.u, right.1];
    let mh = dist.map(|d| view.px_per_unit(d));
    let top = dist.map(|d| view.screen_y(1.0, d));
    let y0 = top.map(|t| t.max(area.y));
    let y1 = [0, 1].map(|i| (top[i] + mh[i]).min(area.y + area.h));
    if y1[0] <= y0[0] && y1[1] <= y0[1] {
        return;
    }
    let lit = lit_at(m.dist, pal.haze, light);
    // Frameless: the reflection's shift in tone plus the streaks give it away. Faint
    // overall sheen plus two diagonal streaks — each follows the glass across the
    // column, so a streak runs as a smooth diagonal rather than a staircase.
    quad(
        buf,
        xs,
        y0,
        y1,
        Color::new(0.8, 0.9, 1.0, 0.06),
        [Vec2::ZERO; 4],
    );
    for (off, a) in [(0.18f32, 0.22f32), (0.34, 0.12)] {
        let v = us.map(|u| 1.0 - (u - off) * 1.8);
        if (0.1..0.9).contains(&v[0]) {
            let sy = [0, 1].map(|i| top[i] + v[i] * mh[i]);
            let a_ = [0, 1].map(|i| (sy[i] - mh[i] * 0.06).max(y0[i]));
            let b_ = [0, 1].map(|i| sy[i] + mh[i] * 0.06);
            quad(
                buf,
                xs,
                a_,
                b_,
                Color::new(0.95, 0.98, 1.0, a * lit),
                [Vec2::ZERO; 4],
            );
        }
    }
}

/// Distance and texture-u of the level-`level` glass in column `ci` at its right screen
/// edge — the next column's when it shows the same glass through the same camera image,
/// else extrapolated from the column before (1/distance and u both run linearly enough
/// across one column), else the column's own. Same reasoning as `right_edge`.
fn glass_edge(cols: &[Col], ci: usize, level: usize) -> (f32, f32) {
    let glass = |c: usize| match cols.get(c) {
        Some(Col::Wall(hit)) => hit.mirrors().get(level - 1).copied(),
        _ => None,
    };
    let Some(m) = glass(ci) else {
        return (1.0, 0.0);
    };
    let same = |o: Option<MirrorHit>| o.filter(|o| o.key == m.key && o.image == m.image);
    if let Some(n) = same(glass(ci + 1)) {
        return (n.dist, n.u);
    }
    if let Some(p) = ci.checked_sub(1).and_then(|p| same(glass(p))) {
        let inv = 2.0 / m.dist.max(0.0001) - 1.0 / p.dist.max(0.0001);
        if inv > 1e-4 {
            return (1.0 / inv, 2.0 * m.u - p.u);
        }
    }
    (m.dist, m.u)
}

/// DDA to the first solid cell; while the face struck is a mirror (up to `MAX_BOUNCES`
/// of them), reflect and keep marching from the glass. Returns the summed distance (the
/// depth in the unfolded, straight-ray world), the final wall's side and texture-u, and
/// each glass on the way with the camera image that sees through it.
fn cast(grid: &WallGrid, pos: Vec2, ray: Vec2) -> RayHit {
    let mut mirrors = [MirrorHit::default(); MAX_BOUNCES];
    let mut bounces = 0;
    let (mut from, mut r, mut total) = (pos, ray, 0.0);
    let (mut image, mut flip_dir) = (pos, (false, false));
    loop {
        let (d, side, map, line) = march(grid, from, r);
        if bounces == MAX_BOUNCES || !grid.is_mirror(map.x, map.y) {
            return RayHit {
                perp: total + d,
                side,
                line,
                wall_x: wall_u(from, r, d, side),
                mirrors,
                bounces,
                square: square_hash(map),
                map,
            };
        }
        // The glass is the struck square's near face — an exact grid line, so every
        // column through the same chain of glass computes the same bit-identical image.
        let flips_x = side == 0;
        let (rc, mc) = if flips_x { (r.x, map.x) } else { (r.y, map.y) };
        let plane = if rc > 0.0 { mc as f32 } else { mc as f32 + 1.0 };
        if flips_x {
            image.x = 2.0 * plane - image.x;
            flip_dir.0 = !flip_dir.0;
        } else {
            image.y = 2.0 * plane - image.y;
            flip_dir.1 = !flip_dir.1;
        }
        mirrors[bounces] = MirrorHit {
            dist: total + d,
            u: wall_u(from, r, d, side),
            image,
            flip: flip_dir,
            flips_x,
            plane,
            side: -rc.signum(),
            key: (map, face_of(side, r)),
        };
        bounces += 1;
        // Reflect the ray across the face struck and march on from just past the glass.
        let glass = from + r * d;
        if flips_x {
            r.x = -r.x;
        } else {
            r.y = -r.y;
        }
        from = glass + r * 1e-3;
        total += d + 1e-3;
    }
}

/// `Surf::line`s of the stairwell flight's far wall and sides (`Well`): no grid line
/// can be this low.
const WELL_LINE: i32 = i32::MIN + 8;

/// The grid march, with Dungeon's stairwell: a ray through the arch face from the exit
/// side goes on into the flight behind the portal (`Well`) and stops at its far wall or
/// a side; a camera in the flight sees those, or out back through the arch.
fn march(grid: &WallGrid, pos: Vec2, ray: Vec2) -> (f32, i32, IVec2, i32) {
    if let Some(w) = grid.well()
        && w.holds(pos)
    {
        let (t, side, line, back) = well_exit(&w, pos, ray);
        if !back {
            return (t, side, stair_square(grid), line);
        }
        let t = t + 1e-4;
        let (d, side, map, line) = march_grid(grid, pos + ray * t, ray);
        return (t + d, side, map, line);
    }
    march_grid(grid, pos, ray)
}

fn stair_square(grid: &WallGrid) -> IVec2 {
    grid.stair.map_or(IVec2::ZERO, |(x, y, _)| ivec2(x, y))
}

/// From `p` inside the flight's box, where `ray` leaves it: depth, side, `Surf::line`,
/// and whether that's back out through the arch (else a wall of the flight).
fn well_exit(w: &Well, p: Vec2, ray: Vec2) -> (f32, i32, i32, bool) {
    let (s, c) = w.local(p);
    let (va, vc) = if w.axis == 0 {
        (ray.x * w.into, ray.y)
    } else {
        (ray.y * w.into, ray.x)
    };
    let along = if va > 1e-9 {
        (STAIR_FLIGHT - s) / va
    } else if va < -1e-9 {
        -s / va
    } else {
        f32::INFINITY
    };
    let across = if vc > 1e-9 {
        (w.hi - c) / vc
    } else if vc < -1e-9 {
        (w.lo - c) / vc
    } else {
        f32::INFINITY
    };
    if along <= across {
        (along.max(0.0001), w.axis as i32, WELL_LINE, va < 0.0)
    } else {
        (
            across.max(0.0001),
            1 - w.axis as i32,
            WELL_LINE + 1 + (vc > 0.0) as i32,
            false,
        )
    }
}

fn march_grid(grid: &WallGrid, pos: Vec2, ray: Vec2) -> (f32, i32, IVec2, i32) {
    let mut map = ivec2(pos.x.floor() as i32, pos.y.floor() as i32);
    let delta = vec2(
        if ray.x == 0.0 {
            1e30
        } else {
            (1.0 / ray.x).abs()
        },
        if ray.y == 0.0 {
            1e30
        } else {
            (1.0 / ray.y).abs()
        },
    );
    let step_x = if ray.x < 0.0 { -1 } else { 1 };
    let step_y = if ray.y < 0.0 { -1 } else { 1 };
    let mut sdist = vec2(
        if ray.x < 0.0 {
            (pos.x - map.x as f32) * delta.x
        } else {
            (map.x as f32 + 1.0 - pos.x) * delta.x
        },
        if ray.y < 0.0 {
            (pos.y - map.y as f32) * delta.y
        } else {
            (map.y as f32 + 1.0 - pos.y) * delta.y
        },
    );
    // Standing in a thin wall's square (on the floor beside its slab — the stairwell
    // descent walks through one): the loop only tests the squares it steps into.
    if let Some(bars) = grid.slabs(map.x, map.y)
        && let Some(hit) = thin_hit(grid, pos, ray, map, &bars)
    {
        return hit;
    }
    let mut side = 0;
    for _ in 0..512 {
        if sdist.x < sdist.y {
            sdist.x += delta.x;
            map.x += step_x;
            side = 0;
        } else {
            sdist.y += delta.y;
            map.y += step_y;
            side = 1;
        }
        // Through the garden's gate: nothing out there but the horizon.
        if grid.open_outside && grid.out_of_bounds(map.x, map.y) {
            return (1.0e6, side, map, 0);
        }
        if grid.is_solid(map.x, map.y) {
            // A thin wall: the ray may pass the slab by, over the floor either side.
            if let Some(bars) = grid.slabs(map.x, map.y) {
                match thin_hit(grid, pos, ray, map, &bars) {
                    Some(hit) => return hit,
                    None => continue,
                }
            }
            break;
        }
    }
    let perp = if side == 0 {
        (sdist.x - delta.x).max(0.0001)
    } else {
        (sdist.y - delta.y).max(0.0001)
    };
    let line = if side == 0 { map.x } else { map.y };
    // A full square among thin walls (the stairwell block, or its inside): a face on
    // the square's boundary, the one the ray crossed.
    let line = if grid.thin {
        let stepped = if side == 0 { step_x } else { step_y };
        4 * (line + (stepped < 0) as i32)
    } else {
        line
    };
    (perp, side, map, line)
}

/// A ray's hit on thin wall square `map` (`slabs` `bars`), if it meets a bar — on into
/// the stairwell's flight when that's its arch face, from the exit side.
fn thin_hit(
    grid: &WallGrid,
    pos: Vec2,
    ray: Vec2,
    map: IVec2,
    bars: &[(Vec2, Vec2); 2],
) -> Option<(f32, i32, IVec2, i32)> {
    let hit = slab_hit(pos, ray, bars)?;
    if let Some(w) = grid.well()
        && map == stair_square(grid)
        && hit.1 == w.axis as i32
        && (if w.axis == 0 { ray.x } else { ray.y }) * w.into > 0.0
    {
        let q = pos + ray * hit.0;
        let (_, c) = w.local(q);
        if (w.lo..=w.hi).contains(&c) {
            let (t, side, line, _) = well_exit(&w, q, ray);
            return Some((hit.0 + t, side, map, line));
        }
    }
    Some((hit.0.max(0.0001), hit.1, map, hit.2))
}

/// `Surf::line` of a thin wall's face: a slab face through grid square `sq`, at its
/// low (`high` false) or high side. Boundary faces of full squares among thin walls
/// are `4 * boundary`, so the two never collide.
pub(super) fn slab_line(sq: i32, high: bool) -> i32 {
    4 * sq + if high { 3 } else { 1 }
}

/// Where a ray from `pos` first enters either bar of a thin wall square: depth, side
/// (0 = an x face), and the face's `slab_line`.
fn slab_hit(pos: Vec2, ray: Vec2, bars: &[(Vec2, Vec2); 2]) -> Option<(f32, i32, i32)> {
    let mut best: Option<(f32, i32, i32)> = None;
    for &(lo, hi) in bars {
        let (x0, x1) = doorway::slab(pos.x, ray.x, lo.x, hi.x);
        let (y0, y1) = doorway::slab(pos.y, ray.y, lo.y, hi.y);
        let (t0, t1) = (x0.max(y0), x1.min(y1));
        if t1 < t0 || t1 <= 0.0 || best.is_some_and(|b| b.0 <= t0) {
            continue;
        }
        let (side, lo, hi, d) = if x0 >= y0 {
            (0, lo.x, hi.x, ray.x)
        } else {
            (1, lo.y, hi.y, ray.y)
        };
        let face = if d > 0.0 { lo } else { hi };
        let sq = face.floor();
        best = Some((t0, side, slab_line(sq as i32, face > sq + 0.5)));
    }
    best
}

fn wall_u(pos: Vec2, ray: Vec2, perp: f32, side: i32) -> f32 {
    let u = if side == 0 {
        pos.y + perp * ray.y
    } else {
        pos.x + perp * ray.x
    }
    .fract();
    if u < 0.0 { u + 1.0 } else { u }
}

/// Vertical gradient as one quad with per-vertex colors, so the GPU interpolates it
/// per pixel (a stack of flat bands read as visible steps on a tall ceiling/floor).
fn v_gradient(x: f32, y: f32, w: f32, h: f32, top: Color, bot: Color) {
    if h <= 0.0 {
        return;
    }
    let top = Color { a: 1.0, ..top };
    let bot = Color { a: 1.0, ..bot };
    draw_mesh(&Mesh {
        vertices: vec![
            Vertex::new(x, y, 0.0, 0.0, 0.0, top),
            Vertex::new(x + w, y, 0.0, 0.0, 0.0, top),
            Vertex::new(x + w, y + h, 0.0, 0.0, 0.0, bot),
            Vertex::new(x, y + h, 0.0, 0.0, 0.0, bot),
        ],
        indices: vec![0, 1, 2, 0, 2, 3],
        texture: None,
    });
}

// ── Outdoor ground, sky and horizon ──────────────────────────────────────────────

/// A screen-space half-plane: points with `n · p <= d` are kept.
#[derive(Clone, Copy)]
struct Half {
    n: Vec2,
    d: f32,
}

/// Polygons for `draw_mesh`, clipped on the way in, flushed in chunks under
/// macroquad's per-call vertex/index caps (10000/5000). One texture per buffer, so a
/// whole ground layer is one or two draw calls.
struct MeshBuf {
    tex: Option<Texture2D>,
    v: Vec<Vertex>,
    i: Vec<u16>,
}

impl MeshBuf {
    fn new(tex: Option<&Texture2D>) -> MeshBuf {
        MeshBuf {
            tex: tex.cloned(),
            v: Vec::with_capacity(2048),
            i: Vec::with_capacity(3072),
        }
    }

    /// A convex polygon, clipped by each half-plane (Sutherland-Hodgman in screen
    /// space — attributes interpolate linearly, like the rasterizer's own affine UVs).
    fn poly(&mut self, pts: &[Vertex], clip: &[Half]) {
        const MAX: usize = 16;
        if pts.len() < 3 {
            return;
        }
        let mut a = [pts[0]; MAX];
        let mut n = pts.len().min(MAX);
        a[..n].copy_from_slice(&pts[..n]);
        for h in clip {
            let f = |v: &Vertex| h.n.dot(v.position.truncate()) - h.d;
            if a[..n].iter().all(|v| f(v) <= 0.0) {
                continue;
            }
            let mut b = [a[0]; MAX];
            let mut m = 0;
            for k in 0..n {
                let (p, q) = (a[k], a[(k + 1) % n]);
                let (fp, fq) = (f(&p), f(&q));
                if fp <= 0.0 && m < MAX {
                    b[m] = p;
                    m += 1;
                }
                if (fp <= 0.0) != (fq <= 0.0) && m < MAX {
                    b[m] = lerp_vertex(p, q, fp / (fp - fq));
                    m += 1;
                }
            }
            if m < 3 {
                return;
            }
            a = b;
            n = m;
        }
        if self.v.len() + n > 9000 || self.i.len() + 3 * (n - 2) > 4800 {
            self.flush();
        }
        let base = self.v.len() as u16;
        self.v.extend_from_slice(&a[..n]);
        for k in 1..n as u16 - 1 {
            self.i.extend_from_slice(&[base, base + k, base + k + 1]);
        }
    }

    /// One convex quad, unclipped — `poly` without its clip buffers, for the ~1000 a
    /// frame's wall pass lays down.
    /// Whether `quads` more fit before this buffer would have to flush itself.
    fn room_for(&self, quads: usize) -> bool {
        self.v.len() + 4 * quads <= 9000 && self.i.len() + 6 * quads <= 4800
    }

    fn quad(&mut self, q: [Vertex; 4]) {
        if self.v.len() + 4 > 9000 || self.i.len() + 6 > 4800 {
            self.flush();
        }
        let b = self.v.len() as u16;
        self.v.extend_from_slice(&q);
        self.i
            .extend_from_slice(&[b, b + 1, b + 2, b, b + 2, b + 3]);
    }

    /// A strip of quads between successive rows of (left, right) vertices, unclipped.
    fn strip(&mut self, rows: &[[Vertex; 2]]) {
        let n = rows.len() * 2;
        if rows.len() < 2 {
            return;
        }
        if self.v.len() + n > 9000 || self.i.len() + 6 * (rows.len() - 1) > 4800 {
            self.flush();
        }
        let base = self.v.len() as u16;
        for r in rows {
            self.v.extend_from_slice(r);
        }
        for k in 0..rows.len() as u16 - 1 {
            let (a, b) = (base + 2 * k, base + 2 * k + 2);
            self.i.extend_from_slice(&[a, a + 1, b + 1, a, b + 1, b]);
        }
    }

    fn flush(&mut self) {
        if self.i.is_empty() {
            return;
        }
        draw_mesh(&Mesh {
            vertices: std::mem::take(&mut self.v),
            indices: std::mem::take(&mut self.i),
            texture: self.tex.clone(),
        });
    }
}

fn lerp_vertex(a: Vertex, b: Vertex, t: f32) -> Vertex {
    let c = |i: usize| (a.color[i] as f32 + (b.color[i] as f32 - a.color[i] as f32) * t) as u8;
    Vertex {
        position: a.position.lerp(b.position, t),
        uv: a.uv.lerp(b.uv, t),
        color: [c(0), c(1), c(2), c(3)],
        normal: a.normal.lerp(b.normal, t),
    }
}

fn rgba(c: [f32; 3], a: f32) -> Color {
    Color::new(c[0], c[1], c[2], a)
}

fn mix(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * t)
}

/// Ground tiles are drawn out to this depth; past it the backdrop gradient carries on.
fn ground_range(g: &Ground) -> f32 {
    if g.horizon.is_some() { 18.0 } else { 9.0 }
}
/// Decals are small; past this they'd only shimmer.
const DECAL_RANGE: f32 = 8.0;
/// Near clip for the ground: the floor is off the bottom of the screen nearer than ~1.
const GROUND_NEAR: f32 = 0.6;

/// The colour the ground averages to at depth `d`: its texture's mean under the
/// walkway tint (and the glass `tint`, seen in a mirror), faded toward fog like
/// everything else — and, in open country, toward the pale horizon haze (aerial
/// perspective) far out. The glass darkens only the ground itself, not the fog it fades
/// into: tinting the whole colour sank deep reflections' far floor toward black.
fn ground_far(g: &Ground, pal: &Palette, d: f32, light: f32, tint: [f32; 3]) -> [f32; 3] {
    if let Some(m) = pal.mist {
        let own = [0, 1, 2].map(|k| g.avg[k] * g.path[k] * tint[k] * light);
        let t = m.transmittance((d * d + fog::EYE * fog::EYE).sqrt(), fog::EYE, 0.0);
        return m.apply(own, t, -fog::EYE / d);
    }
    let lit = lit_at(d, pal.haze, light);
    let fog = [pal.fog.r, pal.fog.g, pal.fog.b];
    let c = mix(fog, [0, 1, 2].map(|k| g.avg[k] * g.path[k] * tint[k]), lit);
    match g.horizon {
        Some(h) => mix(c, h, smoothstep(10.0, 120.0, d) * 0.9),
        None => c,
    }
}

/// Sky: the palette gradient; in open country it pales into a haze band at the horizon
/// with a low, far tree line standing on it (fixed to compass direction — it's at
/// infinity — so it slides past as the camera turns, never with it).
fn draw_sky(view: &View, pal: &Palette, g: &Ground) {
    let a = view.area;
    let hz = view.horizon;
    if let Some(m) = pal.mist {
        // The mist at infinite distance: its colour by elevation, in bands (the curve
        // bends, a single gradient would flatten it) — exactly what everything out
        // there converges on.
        const BANDS: usize = 8;
        let mut buf = MeshBuf::new(None);
        let row = |i: usize| {
            let y = a.y + (hz - a.y) * i as f32 / BANDS as f32;
            (y, rgba(m.colour((hz - y) / a.h), 1.0))
        };
        for i in 0..BANDS {
            let ((ya, ca), (yb, cb)) = (row(i), row(i + 1));
            buf.poly(
                &[
                    Vertex::new(a.x, ya, 0.0, 0.0, 0.0, ca),
                    Vertex::new(a.x + a.w, ya, 0.0, 0.0, 0.0, ca),
                    Vertex::new(a.x + a.w, yb, 0.0, 0.0, 0.0, cb),
                    Vertex::new(a.x, yb, 0.0, 0.0, 0.0, cb),
                ],
                &[],
            );
        }
        buf.flush();
        return;
    }
    let Some(haze) = g.horizon else {
        v_gradient(a.x, a.y, a.w, hz - a.y, pal.ceil_top, pal.ceil_bot);
        return;
    };
    let bot = [pal.ceil_bot.r, pal.ceil_bot.g, pal.ceil_bot.b];
    let mid = hz - (hz - a.y) * 0.4;
    let low = hz - a.h * 0.05;
    let pale = rgba(mix(bot, haze, 0.55), 1.0);
    v_gradient(a.x, a.y, a.w, mid - a.y, pal.ceil_top, pal.ceil_bot);
    v_gradient(a.x, mid, a.w, low - mid, pal.ceil_bot, pale);
    v_gradient(a.x, low, a.w, hz - low, pale, rgba(haze, 1.0));
    let mut buf = MeshBuf::new(None);
    let trees = rgba(mix(haze, [0.30, 0.42, 0.38], 0.5), 1.0);
    let foot = rgba(mix(haze, [0.30, 0.42, 0.38], 0.3), 1.0);
    let height = |x: f32| {
        let r = view.ray(x);
        let th = r.y.atan2(r.x);
        let ridge = 0.5
            + 0.25 * (3.0 * th + 1.3).sin()
            + 0.15 * (7.0 * th + 0.4).sin()
            + 0.1 * (13.0 * th + 2.1).sin();
        let crowns = (41.0 * th).sin().abs() * 0.5 + (97.0 * th + 1.0).sin().abs() * 0.3;
        a.h * (0.006 + 0.018 * ridge + 0.005 * crowns * ridge)
    };
    let step = 4.0;
    let mut x = a.x;
    let mut h0 = height(x);
    while x < a.x + a.w {
        let x1 = x + step;
        let h1 = height(x1);
        buf.poly(
            &[
                Vertex::new(x, hz - h0, 0.0, 0.0, 0.0, trees),
                Vertex::new(x1, hz - h1, 0.0, 0.0, 0.0, trees),
                Vertex::new(x1, hz + 1.0, 0.0, 0.0, 0.0, foot),
                Vertex::new(x, hz + 1.0, 0.0, 0.0, 0.0, foot),
            ],
            &[],
        );
        x = x1;
        h0 = h1;
    }
    buf.flush();
}

/// The floor behind the ground tiles: horizontal bands, each at the colour the ground
/// averages to at that band's depth (`ground_far`), so the textured tiles fading out
/// with distance melt into it instead of meeting a painted backdrop.
fn draw_floor_backdrop(
    view: &View,
    pal: &Palette,
    g: &Ground,
    light: f32,
    clip: &[Half],
    tint: [f32; 3],
    buf: &mut MeshBuf,
) {
    const DEPTHS: [f32; 19] = [
        0.7, 1.0, 1.25, 1.6, 2.0, 2.6, 3.4, 4.5, 6.0, 8.0, 11.0, 15.0, 21.0, 30.0, 45.0, 70.0,
        120.0, 250.0, 1e5,
    ];
    let a = view.area;
    let row = |d: f32| {
        let c = ground_far(g, pal, d, light, tint);
        let y = if d > 1e4 {
            view.horizon
        } else {
            view.screen_y(0.0, d)
        };
        (y, rgba(c, 1.0))
    };
    for w in DEPTHS.windows(2) {
        let ((ya, ca), (yb, cb)) = (row(w[0]), row(w[1]));
        buf.poly(
            &[
                Vertex::new(a.x, ya, 0.0, 0.0, 0.0, ca),
                Vertex::new(a.x + a.w, ya, 0.0, 0.0, 0.0, ca),
                Vertex::new(a.x + a.w, yb, 0.0, 0.0, 0.0, cb),
                Vertex::new(a.x, yb, 0.0, 0.0, 0.0, cb),
            ],
            clip,
        );
    }
}

/// What a ground square is: walkway, undergrowth (a solid square — hedge or trees, and
/// the unborn forest past the grid), or meadow past the garden's gate.
fn tile_at(grid: &WallGrid, sq: IVec2) -> Tile {
    if grid.out_of_bounds(sq.x, sq.y) {
        if grid.open_outside {
            Tile::Outside
        } else {
            Tile::Under
        }
    } else if grid.is_solid(sq.x, sq.y) {
        Tile::Under
    } else {
        Tile::Path
    }
}

/// Bilinear value noise on a world lattice of spacing `cell`: along a square's edge it's
/// linear, so subdivided and whole neighbouring squares agree where they meet.
fn world_noise(p: Vec2, cell: f32, salt: u64) -> f32 {
    let q = p / cell;
    let (x0, y0) = (q.x.floor(), q.y.floor());
    let (tx, ty) = (q.x - x0, q.y - y0);
    let h = |x: f32, y: f32| {
        let v = mix64(((x as i32 as u32 as u64) << 32 | y as i32 as u32 as u64) ^ salt);
        (v >> 40) as f32 / (1u64 << 24) as f32
    };
    let top = h(x0, y0) + (h(x0 + 1.0, y0) - h(x0, y0)) * tx;
    let bot = h(x0, y0 + 1.0) + (h(x0 + 1.0, y0 + 1.0) - h(x0, y0 + 1.0)) * tx;
    top + (bot - top) * ty
}

/// Contact shade at local point `l` (0..1 in the square) from the solid squares around:
/// 1 on or under foliage, easing to 0 about 0.45 out.
fn occlusion(l: Vec2, solid: &[[bool; 3]; 3]) -> f32 {
    let mut occ = 0.0f32;
    for (dy, row) in solid.iter().enumerate() {
        for (dx, &s) in row.iter().enumerate() {
            if !s {
                continue;
            }
            let (ox, oy) = (dx as f32 - 1.0, dy as f32 - 1.0);
            let ex = (ox - l.x).max(l.x - (ox + 1.0)).max(0.0);
            let ey = (oy - l.y).max(l.y - (oy + 1.0)).max(0.0);
            let k = (1.0 - (ex * ex + ey * ey).sqrt() / 0.45).max(0.0);
            occ = occ.max(k * k);
        }
    }
    occ
}

/// Atlas UV of local point `l` on decal `v`, turned/flipped by `rot` (0..8).
fn decal_uv(l: Vec2, v: usize, rot: u64) -> Vec2 {
    let (mut a, b) = match rot & 3 {
        0 => (l.x, l.y),
        1 => (1.0 - l.y, l.x),
        2 => (1.0 - l.x, 1.0 - l.y),
        _ => (l.y, 1.0 - l.x),
    };
    if rot & 4 != 0 {
        a = 1.0 - a;
    }
    vec2(((v % 2) as f32 + a) * 0.5, ((v / 2) as f32 + b) * 0.5)
}

/// The outdoor ground plane: one world-space quad per grid square within range, cast
/// with `view` (the camera, or its mirror image for the floor seen in a mirror — then
/// `keep` limits it to squares in front of the glass, and `span` to the glass's screen
/// columns, as -1..1 across the view: `clip` would cut the rest anyway, but only after
/// the square had been built). `within` bounds the squares visited (`run_squares`) —
/// a mirror shows a narrow wedge of the floor, and walking the whole range around every
/// camera image, per glass per bounce, was most of a mirror frame's cost. Each square is clipped to the near
/// plane in camera space and projected; near squares are subdivided (affine texturing
/// is only close to perspective-correct on small pieces). UVs are world position over
/// `GROUND_PERIOD` on a wrapping, mipmapped texture, so squares meet seamlessly; vertex
/// colour carries the square's tint (walkway / undergrowth / meadow), a world-space tone
/// noise that breaks up the repeat, contact shade at the foot of the foliage, mowing
/// stripes along the walkways, and alpha = `lit` fading to zero at range, over the
/// backdrop that continues it. A second layer lays one hashed decal (rotated/flipped)
/// on some near squares. Two textures, so ~2-4 draw calls.
#[allow(clippy::too_many_arguments)]
fn draw_ground(
    view: &View,
    grid: &WallGrid,
    g: &Ground,
    pal: &Palette,
    light: f32,
    clip: &[Half],
    span: (f32, f32),
    within: Option<(IVec2, IVec2)>,
    tint: [f32; 3],
    keep: &dyn Fn(IVec2) -> bool,
) {
    let range = ground_range(g);
    let area = view.area;
    let mut base = MeshBuf::new(Some(&g.base));
    let mut dec = MeshBuf::new(Some(&g.decals));
    // Mist: each layer carries its own fog in the same pass (`Ground::premul`: colour ×
    // T, plus the mist's light × (1 − T) over its coverage), so decals fog exactly like
    // the floor under them — fading each layer by alpha over a fogged backdrop fogged
    // the floor twice over and left the decals as dark patches in the haze.
    let mist = pal.mist.filter(|_| g.premul.is_some());
    // Every ground point sits at height 0: one mean density for the whole pass.
    let ground_k = mist.map_or(0.0, |m| m.mean_density(view.eye, 0.0));
    if let Some(mat) = mist.and(g.premul.as_ref()) {
        gl_use_material(mat);
    }
    let r = range as i32 + 2;
    let (px, py) = (view.pos.x.floor() as i32, view.pos.y.floor() as i32);
    let (lo, hi) = within.unwrap_or((ivec2(px - r, py - r), ivec2(px + r, py + r)));
    for gy in lo.y.max(py - r)..=hi.y.min(py + r) {
        for gx in lo.x.max(px - r)..=hi.x.min(px + r) {
            let sq = ivec2(gx, gy);
            if !keep(sq) {
                continue;
            }
            let o = sq.as_vec2();
            let cs = [
                o,
                o + vec2(1.0, 0.0),
                o + vec2(1.0, 1.0),
                o + vec2(0.0, 1.0),
            ]
            .map(|w| view.cam(w));
            if cs.iter().all(|c| c.y < GROUND_NEAR) {
                continue;
            }
            if cs.iter().all(|c| c.y > GROUND_NEAR)
                && (cs.iter().all(|c| c.x > span.1 * c.y) || cs.iter().all(|c| c.x < span.0 * c.y))
            {
                continue;
            }
            let dmin = cs
                .iter()
                .map(|c| c.y)
                .fold(f32::MAX, f32::min)
                .max(GROUND_NEAR);
            if dmin > range {
                continue;
            }
            let tile = tile_at(grid, sq);
            let under = |dx: i32, dy: i32| tile_at(grid, sq + ivec2(dx, dy)) == Tile::Under;
            let solid: [[bool; 3]; 3] =
                std::array::from_fn(|y| std::array::from_fn(|x| under(x as i32 - 1, y as i32 - 1)));
            let h = mix64(((gx as u32 as u64) << 32 | gy as u32 as u64) ^ 0x6A0D_5EED);
            let decal = if dmin < DECAL_RANGE {
                g.decal(tile, (h & 0xFFFF) as f32 / 65536.0)
            } else {
                None
            };
            let rot = (h >> 32) & 7;
            // Mowing stripes run along the walkway: E-W passages stripe across y.
            let along_x = match (gx & 1, gy & 1) {
                (0, 1) => true,
                (1, 0) => false,
                _ => !under(1, 0) || !under(-1, 0),
            };
            let stripes = if tile == Tile::Path {
                g.stripes * (1.0 - smoothstep(5.0, 8.5, dmin))
            } else {
                0.0
            };
            let col = g.tint(tile);
            let n = if dmin < 1.8 {
                4
            } else if dmin < 9.0 {
                2
            } else {
                1
            };
            let vertex = |p: Vec2, tone: f32| -> Vertex {
                let c = view.cam(p);
                let dep = c.y.max(GROUND_NEAR);
                let fade = 1.0 - smoothstep(range * 0.7, range, dep);
                // (own-colour factor, the mist's light added over the coverage)
                let (k_own, fog, fade) = match mist {
                    Some(m) => {
                        let d = p.distance(view.pos).max(1e-3);
                        let t = (-ground_k * (d * d + view.eye * view.eye).sqrt()).exp();
                        let f = m.colour(-view.eye / d);
                        let add = (1.0 - t) * fade;
                        (
                            light * t * fade,
                            vec4(f[0] * add, f[1] * add, f[2] * add, 0.0),
                            fade,
                        )
                    }
                    None => (1.0, Vec4::ZERO, lit_at(dep, pal.haze, light) * fade),
                };
                let patch = 0.88 + 0.1 * world_noise(p, 1.0, 1) + 0.16 * world_noise(p, 3.0, 2);
                let k = tone * patch * (1.0 - g.ao * occlusion(p - o, &solid)) * k_own;
                Vertex {
                    normal: fog,
                    ..Vertex::new(
                        area.x + area.w * 0.5 * (1.0 + c.x / dep),
                        view.screen_y(0.0, dep),
                        0.0,
                        p.x / GROUND_PERIOD,
                        p.y / GROUND_PERIOD,
                        rgba([0, 1, 2].map(|i| col[i] * k * tint[i]), fade),
                    )
                }
            };
            let step = 1.0 / n as f32;
            for j in 0..n {
                for i in 0..n {
                    let (s0, t0) = (i as f32 * step, j as f32 * step);
                    let quad = [
                        o + vec2(s0, t0),
                        o + vec2(s0 + step, t0),
                        o + vec2(s0 + step, t0 + step),
                        o + vec2(s0, t0 + step),
                    ];
                    let mid = o + vec2(s0 + step * 0.5, t0 + step * 0.5);
                    let across = if along_x { mid.y } else { mid.x };
                    let tone = 1.0 - stripes * ((across * 2.0).floor() as i32 & 1) as f32;
                    // Near-plane clip in camera space (on world points: the camera
                    // transform is affine, so lerping world positions is exact).
                    let mut pts = [Vec2::ZERO; 8];
                    let mut m = 0;
                    for k in 0..4 {
                        let (a, b) = (quad[k], quad[(k + 1) % 4]);
                        let (da, db) = (view.cam(a).y - GROUND_NEAR, view.cam(b).y - GROUND_NEAR);
                        if da >= 0.0 {
                            pts[m] = a;
                            m += 1;
                        }
                        if (da >= 0.0) != (db >= 0.0) {
                            pts[m] = a + (b - a) * (da / (da - db));
                            m += 1;
                        }
                    }
                    if m < 3 {
                        continue;
                    }
                    let mut vs = [Vertex::new(0.0, 0.0, 0.0, 0.0, 0.0, WHITE); 8];
                    for k in 0..m {
                        vs[k] = vertex(pts[k], tone);
                    }
                    base.poly(&vs[..m], clip);
                    if let Some(v) = decal {
                        for k in 0..m {
                            vs[k].uv = decal_uv(pts[k] - o, v, rot);
                        }
                        dec.poly(&vs[..m], clip);
                    }
                }
            }
        }
    }
    base.flush();
    dec.flush();
    if mist.is_some() {
        gl_use_default_material();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maze::DIRS;

    /// Brute-force: tiny fixed steps, reflect on entering a mirror square (up to
    /// `MAX_BOUNCES` of them). Returns the distance to the final wall, where it is, and
    /// the distance to each glass.
    fn slow(grid: &WallGrid, pos: Vec2, ray: Vec2) -> (f32, Vec2, Vec<f32>) {
        let step = 0.0005;
        let mut p = pos;
        let mut r = ray;
        let mut t = 0.0;
        let mut glass = Vec::new();
        for _ in 0..400_000 {
            let np = p + r * step;
            let (cx, cy) = (np.x.floor() as i32, np.y.floor() as i32);
            if grid.is_solid(cx, cy) {
                if grid.is_mirror(cx, cy) && glass.len() < MAX_BOUNCES {
                    // Which axis did we cross?
                    if cx != p.x.floor() as i32 {
                        r.x = -r.x;
                    } else {
                        r.y = -r.y;
                    }
                    glass.push(t);
                    continue;
                }
                return (t, p, glass);
            }
            p = np;
            t += step;
        }
        (t, p, glass)
    }

    /// Dungeon's stairwell: every maze gets one. The wall is a plain slab, flush with
    /// the rest; a ray through its arch face from the exit side goes on into the flight
    /// behind the portal and stops at its far wall (or a side); from the cell behind,
    /// it's a plain wall. A camera in the flight sees its walls, or out through the
    /// arch and on.
    #[test]
    fn stairwell_opens_only_from_the_exit() {
        let arch = crate::maze::face_dist(true);
        for seed in 0..60u64 {
            let maze = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                crate::maze::generate(15, 11, 0.2, 3, 0)
            };
            let Some(d) = maze.stairwell else {
                panic!(
                    "seed {seed}: exit {:?} edges {:?}",
                    maze.exit,
                    (0..4).map(|d| maze.edge(maze.exit, d)).collect::<Vec<_>>()
                );
            };
            let grid = maze.wall_grid();
            let into = vec2(DIRS[d].0 as f32, DIRS[d].1 as f32);
            let (gx, gy) = WallGrid::cell_center(maze.exit);
            let centre = vec2(gx as f32 + 0.5, gy as f32 + 0.5);
            let far = arch + STAIR_FLIGHT;
            let in_well = |line: i32| (WELL_LINE..WELL_LINE + 3).contains(&line);
            let (perp, _, _, line) = march(&grid, centre, into);
            assert!(
                (perp - far).abs() < 1e-3 && in_well(line),
                "seed {seed}: in from the exit {perp}"
            );
            // Slanted: stops at a side of the flight, never past its far wall.
            let slant = (into + into.perp() * 0.4).normalize();
            let (perp, _, _, _) = march(&grid, centre, slant);
            let along = perp * slant.dot(into);
            assert!(
                along > arch && along <= far + 1e-3,
                "seed {seed}: slanted {along}"
            );
            // Walking in (on the floor strip before the slab, in the square) and on
            // down, looking on down the flight.
            for k in -6..20 {
                let at = arch + k as f32 * 0.05;
                let (perp, _, _, line) = march(&grid, centre + into * at, into);
                assert!(
                    (perp - (far - at)).abs() < 1e-3 && in_well(line),
                    "seed {seed}: down the flight at {at}: {perp}"
                );
            }
            // From inside, looking back out through the arch and across the exit cell.
            let (perp, _, _, line) = march(&grid, centre + into * (arch + 0.5), -into);
            assert!(
                perp > 2.2 && !in_well(line),
                "seed {seed}: from inside {perp}"
            );
            // From the cell behind: a plain wall, flush with the rest.
            let beyond = maze.exit.step(d);
            if maze.cell_in_bounds(beyond) {
                let (bx, by) = WallGrid::cell_center(beyond);
                let from = vec2(bx as f32 + 0.5, by as f32 + 0.5);
                for k in -4..=4 {
                    let r = (-into + into.perp() * (k as f32 * 0.1)).normalize();
                    let (perp, _, _, line) = march(&grid, from, r);
                    assert!(!in_well(line), "seed {seed}: the flight from behind");
                    if k == 0 {
                        assert!(
                            (perp - arch).abs() < 1e-3,
                            "seed {seed}: from beyond {perp}"
                        );
                    }
                }
            }
        }
    }

    /// `cast` against the stepper, on a maze dense with mirrors so chains of facing
    /// glass come up: same bounce count, same distance to every glass and to the final
    /// wall — and the last bounce's camera image, looking straight down the flipped
    /// ray, lands on the very point the stepper stopped at (the unfolding composes).
    #[test]
    fn mirror_matches_brute_force() {
        let maze = {
            let _g = crate::rng_guard();
            macroquad::rand::srand(5);
            let mut m = crate::maze::generate(8, 8, 0.2, 0, 0);
            m.add_mirrors(48);
            m
        };
        let grid = maze.wall_grid();
        let mut by_depth = [0usize; MAX_BOUNCES + 1];
        for cy in 0..8 {
            for cx in 0..8 {
                for &(ox, oy) in &[(0.3f32, 0.4f32), (0.6, 0.55), (0.45, 0.2)] {
                    let pos = vec2(2.0 * cx as f32 + 1.0 + ox, 2.0 * cy as f32 + 1.0 + oy);
                    for a in 0..24 {
                        let yaw = a as f32 * 0.2618 + 0.13; // off-axis: no grazing corners
                        let ray = vec2(yaw.cos(), yaw.sin());
                        let hit = cast(&grid, pos, ray);
                        let (t, end, glass) = slow(&grid, pos, ray);
                        // The stepper is only approximate on grazing corner hits (it
                        // disagrees by up to ~0.07 on plain non-mirror rays too).
                        let at = format!("pos {pos} yaw {yaw}");
                        assert_eq!(hit.bounces, glass.len(), "{at}");
                        for (m, g) in hit.mirrors().iter().zip(&glass) {
                            assert!((m.dist - g).abs() < 0.1, "{at}: glass {} vs {g}", m.dist);
                        }
                        assert!((hit.perp - t).abs() < 0.1, "{at}: {} vs {t}", hit.perp);
                        if let Some(m) = hit.mirrors().last() {
                            let seen = m.image + flip(ray, m.flip) * hit.perp;
                            assert!(
                                seen.distance(end) < 0.1,
                                "{at}: image sees {seen}, not {end}"
                            );
                        }
                        by_depth[hit.bounces] += 1;
                    }
                }
            }
        }
        assert!(by_depth[1] > 20, "bounce counts {by_depth:?}");
        assert!(
            by_depth[2..].iter().sum::<usize>() > 20,
            "bounce counts {by_depth:?}"
        );
        assert!(
            by_depth[3..].iter().sum::<usize>() > 5,
            "bounce counts {by_depth:?}"
        );
    }

    /// Hedge clumps stay out of the walkway: at most `MAX_PROTRUDE` out of the face, and
    /// past a hedge end at most that far sideways; none sink below the floor.
    #[test]
    fn clumps_keep_the_walkway_clear() {
        for x in -20..20 {
            for y in -20..20 {
                for f in 0..4 {
                    for cont in [false, true] {
                        for c in hedge_clumps(ivec2(x, y), f, cont) {
                            assert!(c.n + c.r <= MAX_PROTRUDE + 1e-6, "{c:?}");
                            assert!(c.u - c.r >= -MAX_PROTRUDE, "{c:?}");
                            if !cont {
                                assert!(c.u + c.r <= 1.0 + MAX_PROTRUDE, "{c:?}");
                            }
                            assert!(c.z - c.r > -0.05 && c.z + c.r < 1.15, "{c:?}");
                        }
                    }
                }
            }
        }
    }

    /// The mirrored camera sees a real point exactly where the real camera sees that
    /// point's reflection — what the reflected ground and hedge passes rely on.
    #[test]
    fn mirrored_view_projects_reflections() {
        let area = Rect::new(0.0, 0.0, 900.0, 690.0);
        for (yaw, flips_x) in [(0.3f32, true), (1.9, true), (-1.2, false), (2.6, false)] {
            let dir = vec2(yaw.cos(), yaw.sin());
            let view = View {
                area,
                pos: vec2(3.3, 4.6),
                dir,
                stride: 0.0,
                plane: vec2(-dir.y, dir.x) * 0.65,
                horizon: 345.0,
                eye: 0.5,
                haze: None,
                zbuf: Vec::new(),
                roof: Vec::new(),
                under: Vec::new(),
                portal: Vec::new(),
                well: None,
                glass: None,
            };
            // Glass on the grid line x = 6 (or y = 7).
            let refl = |p: Vec2| {
                if flips_x {
                    vec2(12.0 - p.x, p.y)
                } else {
                    vec2(p.x, 14.0 - p.y)
                }
            };
            let mv = view.mirrored(refl(view.pos), (flips_x, !flips_x));
            for p in [
                vec2(1.0, 2.0),
                vec2(4.5, 5.5),
                vec2(2.2, 6.1),
                vec2(5.0, 1.0),
            ] {
                match (view.project(refl(p)), mv.project(p)) {
                    (Some(a), Some(b)) => {
                        assert!((a.0 - b.0).abs() < 1e-2 && (a.1 - b.1).abs() < 1e-4)
                    }
                    (a, b) => assert_eq!(a.is_some(), b.is_some()),
                }
            }
        }
    }

    /// What a mirror's window (`glass_window` + the depth behind it, as `Props::reflect`
    /// gets it) lets through: a real point on the glass's side, between the glass and
    /// what the reflected ray strikes next, shows in its column; the same line of sight
    /// short of the glass (the far side of the hedge, in the real world) or past the
    /// next surface doesn't, nor does anything outside the run's columns.
    #[test]
    fn reflected_window_keeps_to_the_glass() {
        let maze = {
            let _g = crate::rng_guard();
            macroquad::rand::srand(5);
            let mut m = crate::maze::generate(8, 8, 0.2, 0, 0);
            m.add_mirrors(48);
            m
        };
        let grid = maze.wall_grid();
        let area = Rect::new(0.0, 0.0, 400.0, 300.0);
        let mut checked = 0;
        for cy in 0..8 {
            for cx in 0..8 {
                for a in 0..8 {
                    let yaw = a as f32 * 0.785 + 0.21;
                    let dir = vec2(yaw.cos(), yaw.sin());
                    let view = View {
                        area,
                        pos: vec2(2.0 * cx as f32 + 1.4, 2.0 * cy as f32 + 1.55),
                        dir,
                        stride: 0.0,
                        plane: vec2(-dir.y, dir.x) * (area.w * 0.5 / area.h),
                        horizon: 150.0,
                        eye: 0.5,
                        haze: None,
                        zbuf: Vec::new(),
                        roof: Vec::new(),
                        under: Vec::new(),
                        portal: Vec::new(),
                        well: None,
                        glass: None,
                    };
                    let n = (area.w / COL_STEP) as usize;
                    let cols: Vec<Col> = (0..n)
                        .map(|c| {
                            let ray = view.ray(area.x + c as f32 * COL_STEP);
                            Col::Wall(cast(&grid, view.pos, ray))
                        })
                        .collect();
                    for run in glass_runs(&cols).iter().filter(|r| r.level == 1) {
                        let mut mv = view.mirrored(run.image, run.flip);
                        let far = |c: usize| match &cols[c] {
                            Col::Wall(h) if (run.c0..=run.c1).contains(&c) => {
                                h.mirrors().get(1).map_or(h.perp, |m| m.dist)
                            }
                            _ => 0.0,
                        };
                        mv.zbuf = (0..n).map(far).collect();
                        mv.glass = Some(glass_window(&cols, run));
                        assert!(mv.reflected() && mv.mirror_handed());
                        for (c, col) in cols.iter().enumerate().take(run.c1).skip(run.c0 + 1) {
                            let Col::Wall(hit) = col else { continue };
                            let m = &hit.mirrors()[0];
                            let sx = area.x + c as f32 * COL_STEP;
                            let ray = flip(view.ray(sx), m.flip);
                            let (near, back) = (m.dist, far(c));
                            if back - near < 0.2 {
                                continue;
                            }
                            let at = |t: f32| mv.project(m.image + ray * t);
                            let (x, d) = at((near + back) * 0.5).unwrap();
                            assert!((x - sx).abs() < 0.01, "lands in its own column");
                            assert!(mv.visible(sx + 0.5, d), "in front of the glass shows");
                            assert!(run.in_front(m.image + ray * d));
                            if near > 0.4 {
                                let (_, d) = at(near - 0.1).unwrap();
                                assert!(!mv.visible(sx + 0.5, d), "behind the glass");
                            }
                            let (_, d) = at(back + 0.1).unwrap();
                            assert!(!mv.visible(sx + 0.5, d), "past the next surface");
                            checked += 1;
                        }
                        if run.c0 > 0 {
                            let sx = area.x + (run.c0 - 1) as f32 * COL_STEP;
                            assert!(!mv.visible(sx + 0.5, 1.0) && mv.span().0 > sx);
                        }
                    }
                }
            }
        }
        assert!(checked > 200, "only {checked} mirror columns checked");
    }

    #[test]
    fn face_of_matches_face_frame() {
        // A ray going +x enters a square through its west face, and so on.
        for (side, ray, f) in [
            (0, vec2(1.0, 0.1), 1u8),
            (0, vec2(-1.0, 0.1), 0),
            (1, vec2(0.1, 1.0), 3),
            (1, vec2(0.1, -1.0), 2),
        ] {
            let got = face_of(side, ray);
            assert_eq!(got, f);
            let (_, _, n) = face_frame(ivec2(0, 0), got);
            assert!(
                n.dot(ray) < 0.0,
                "the entered face looks back along the ray"
            );
        }
    }
}
