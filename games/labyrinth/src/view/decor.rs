//! Set dressing: furniture and fittings, as axis-aligned boxes standing in the maze,
//! plus flat overlays on them (whiteboard marker, door signs).
//!
//! Office (Backrooms): hall equipment (no kitchen appliances out there), whiteboards on
//! the hall wall facing a door and on open-space walls, a reception desk at the start,
//! a small kitchenette at a couple of hall dead ends, and each room dressed for its
//! `RoomKind`: open-space desk pods, the boss's office (a crowned throne, a BOSS sign,
//! the boss himself), a janitor's closet, a kitchen with a dining
//! table and a TV, and over-designed meeting rooms with ironic names. Dungeon: wall
//! shackles, ceiling chains, barrels, crates, bones, banners. Mirror garden (Alice):
//! a checkerboard lawn (flat tiles) with a Red and a White Queen on plinths, the Mad
//! Hatter's tea table, red roses half-painted white and clocks stopped at six set into
//! the hedges, EAT ME / DRINK ME / THIS WAY signs and a mirror-written Jabberwocky,
//! corner chess statues and croquet flamingos, a two-tier stone fountain in one set-piece
//! cell (`Decor::fountain` for its sound), and a Cheshire Cat grin fading in and out. Hedge fittings stand proud of the leaf clumps; nothing hangs on a mirror, and
//! free-standing pieces keep out of cells with one (dressing isn't reflected).
//!
//! Placement is hashed off the grid (never `macroquad::rand` — dressing must not move
//! the maze RNG) and keeps clear of every walked line: wall fittings stay within
//! `MAX_DEPTH` of their wall, free-standing furniture goes only in open-space pillar
//! squares (no path crosses them) or in peek-only rooms (the bot looks in from the
//! doorway and never walks inside).
//!
//! Drawing: every box is ray-tested per screen column (2D slab test in the floor plane)
//! against the wall `zbuf`; each column then paints its hits far to near. A column
//! shows a box's face at its entry distance and its top between entry and exit (the
//! eye, at 0.5, is above almost everything). A block standing on (or sunk into) another's
//! top with the eye above it — or hanging under its underside with the eye below — is
//! painted after it (`stacked`): the support's top runs on behind it, so plain
//! far-to-near order let a cooler's body top paint over its own bottle. So is a
//! feature on a round block's face (`on_face`: the boss's brows, eyes, nose).
//! Flat boxes (`z0..z1` within `FLAT` of the floor: floor tiles, stains) are painted before anything standing
//! in their column and don't count as the nearest decor for overlays.
//!
//! Round props: a block's `Shape` makes it an elliptic cylinder, ellipsoid, or upper /
//! lower half-ellipsoid (dome / bowl) inscribed in its box. A column ray-tests the
//! ellipse footprint (`ellipse`) instead of the slab; a cylinder then paints like a box
//! (face at entry, top chord) shaded by its entry normal, and an ellipsoid cuts the
//! column's vertical plane in an ellipse whose screen extent is found from the two
//! tangent lines from the eye (the flat face's ends for a half), shaded lit crown to
//! dark underside. `Finish::Glass` paints translucent (water bottles, the fountain's
//! water), `Finish::Leafy` mottles foliage. Whole fittings wholly behind walls are culled
//! (`hidden`) and `columns` clips each footprint to the near cut, so a box beside or
//! behind the camera no longer ray-tests every column of the view.

use crate::maze::{
    Cell, DIRS, EdgeKind, Maze, MeetingLayout, Panel, Room, RoomKind, WALL_T, opposite,
};
use crate::view::fx::Lighting;
use crate::view::label::{self, Icon, Occluders, Spec};
use crate::view::raycast::{COL_STEP, View, fog_at, lit_at};
use crate::view::theme::{Palette, ThemeKind};
use macroquad::prelude::*;
use std::mem::take;

/// How far a wall fitting may stand out from its wall (grid units). The walked line
/// runs 0.5 from the wall, so this leaves the camera a clear 0.28.
const MAX_DEPTH: f32 = 0.22;
/// Boxes farther than this aren't drawn (the haze has them by then).
const RANGE: f32 = 12.0;
/// Door name plates: brass letters on a dark plate.
const PLATE: Color = Color::new(0.12, 0.1, 0.08, 1.0);
const BRASS: Color = Color::new(0.95, 0.8, 0.45, 1.0);
const PAPER: Color = Color::new(0.95, 0.95, 0.92, 1.0);
const PAPER_RGB: [f32; 3] = [0.95, 0.95, 0.92];
const INK: Color = Color::new(0.1, 0.1, 0.1, 1.0);
/// Door names longer than this go on two lines.
const SIGN_CHARS: usize = 11;

/// Meeting-room names, handed out without repeats on a floor.
const MEETING_ROOMS: &[&str] = &[
    "SYNERGY",
    "BLUE SKY",
    "DEEP DIVE",
    "THE PIVOT",
    "CIRCLE BACK",
    "BANDWIDTH",
    "MOONSHOT",
    "QUICK SYNC",
    "ALIGNMENT",
    "TOUCH BASE",
    "VALUE ADD",
    "IDEATION STATION",
    "THE PARKING LOT",
    "PARADIGM SHIFT",
    "LOW-HANGING FRUIT",
    "THOUGHT SHOWER",
    "WIN-WIN",
    "DISRUPTION",
    "OPEN DOOR POLICY",
    "HOLISTIC",
];
const PEPE: &str = "PEPE SILVIA";
const CAKE: &str = "THE CAKE IS A LIE";
/// Three short lines, so it can be set big enough to read from the IT room's door.
const IT_POSTER: [&str; 3] = ["HAVE YOU TRIED", "TURNING IT OFF", "AND ON AGAIN?"];
/// The boss's motivational poster, the janitor's safety notice, the wet-floor sign.
/// The lady boss's mug.
const BOSS_MUG: &str = "#1 BOSS";
const BOSS_POSTER: [&str; 3] = ["LEADERSHIP", "MY WAY OR", "THE LIFT"];
const CLOSET_POSTER: [&str; 2] = ["DAYS WITHOUT", "A SPILL: 0"];
const WET_FLOOR: [&str; 2] = ["CAUTION", "WET FLOOR"];
/// The zen room's neon sign.
const ZEN_NEON: &str = "CALM";
/// The "culture" boardroom's.
const CULTURE_NEON: [&str; 3] = ["GOOD", "VIBES", "ONLY"];

/// Every string this module draws — prewarm the font atlas with these at
/// `label::BAKE_PX`.
pub fn labels() -> Vec<&'static str> {
    let mut v = vec![
        "BOSS ROOM",
        "WC",
        "OUT OF ORDER",
        "THE INTERNET",
        "JANITOR",
        "KITCHEN",
        "IT",
        PEPE,
        CAKE,
        "TPS REPORTS",
        "MACRODATA REFINEMENT",
        "427",
        IT_POSTER[0],
        IT_POSTER[1],
        IT_POSTER[2],
        "THE BOSS",
        BOSS_MUG,
        ZEN_NEON,
    ];
    v.extend_from_slice(&BOSS_POSTER);
    v.extend_from_slice(&CLOSET_POSTER);
    v.extend_from_slice(&WET_FLOOR);
    v.extend_from_slice(&CULTURE_NEON);
    v.extend_from_slice(MEETING_ROOMS);
    v.extend(DUNGEON_NOTICES.iter().flat_map(|n| n.iter().copied()));
    v.extend_from_slice(&[EAT_ME, DRINK_ME, THIS_WAY]);
    v.extend_from_slice(&JABBERWOCKY);
    v
}

#[derive(Clone, Copy, PartialEq)]
enum Finish {
    Plain,
    /// The face toward `DIRS[d]` glows in this colour (screens).
    Screen(usize, [f32; 3]),
    /// A screen showing a bar chart — up and to the right, obviously.
    Chart(usize),
    /// Glows all over, unlit (the lava lamp, a neon sign).
    Lamp,
    /// Dark horizontal lines every `step` (drawers, barrel hoops, shelves).
    Bands(f32),
    /// Chain links: alternating dark/light every few hundredths.
    Links,
    /// A dark outline round every face (door leaves: their seams show when shut).
    Framed,
    /// A server rack's face toward `DIRS[d]`: rows of little status LEDs, blinking.
    Rack(usize),
    /// A light that blinks on and off (the Internet's red light).
    Blink,
    /// See-through (a water-cooler bottle, a DRINK ME bottle): painted translucent over
    /// whatever lies behind, more opaque toward the silhouette, with a highlight streak.
    Glass,
    /// Foliage: a round shape mottled light and dark in small leaf-sized patches, so
    /// a bush reads as leaves rather than a smooth green ball.
    Leafy,
    /// A lava lamp's liquid: glowing and see-through (the wax blobs inside show), more
    /// opaque toward the silhouette; no top face (it's a stack of slices).
    Lava,
    /// A window behind blinds: a pale glow crossed by dark slats every `step`.
    Blinds(f32),
}

/// One piece of the boss's head for `Decor::put`: `a`, `b`, `z` spans, colour, finish,
/// shape.
type FacePart = ((f32, f32), (f32, f32), (f32, f32), [f32; 3], Finish, Shape);

/// How a moving block moves (`Decor::moves`, re-placed every frame in `draw`).
#[derive(Clone, Copy)]
enum Motion {
    /// The boss's head (and face): shakes side to side along `across` while he argues,
    /// hangs once he's conceded.
    Head { across: Vec2 },
    /// The boss's fist: pounds the desk while he argues.
    Fist,
    /// A lava-lamp wax blob, `phase` 0..1: rises from the pool to the cap and sinks
    /// back, stretched while it moves, inside glass `lo..hi` around `axis`.
    Blob {
        axis: Vec2,
        lo: f32,
        hi: f32,
        phase: f32,
    },
}

/// What a block's footprint box and z-span hold. Rounded shapes are inscribed in the
/// box, axis-aligned: an elliptic footprint `min..max`, heights `z0..z1`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shape {
    Box,
    /// An upright elliptic cylinder: flat top and bottom, a round side.
    Cyl,
    /// An ellipsoid filling the box.
    Ball,
    /// The upper half of an ellipsoid standing on its flat base at `z0` (a bottle's
    /// shoulder, a bush's crown, a lid).
    Dome,
    /// The lower half of an ellipsoid hanging from its flat rim at `z1` (a teacup, the
    /// underside of a cooler bottle); the rim's face shows dark from above, like the
    /// inside.
    Bowl,
}

#[derive(Clone, Copy)]
struct Block {
    min: Vec2,
    max: Vec2,
    z0: f32,
    z1: f32,
    rgb: [f32; 3],
    finish: Finish,
    /// The fitting it belongs to (culled as a whole).
    group: u32,
    shape: Shape,
}

impl Block {
    /// A patch on the ground (a floor tile, a stain): painted under everything standing.
    /// The stairwell's treads end below the floor too, but aren't flat.
    fn flat(&self) -> bool {
        self.z1 < FLAT && self.z0 > -FLAT
    }

    /// See-through: paints over what's behind rather than hiding it.
    fn clear(&self) -> bool {
        matches!(self.finish, Finish::Glass | Finish::Lava)
    }
}

/// A part of a wall fitting: `half` its width along the wall, centred `along` from the
/// fitting's centre, standing `inset` off the wall and `depth` deep, heights `z0..z1`.
#[derive(Clone, Copy)]
struct Part {
    half: f32,
    along: f32,
    inset: f32,
    depth: f32,
    z0: f32,
    z1: f32,
    rgb: [f32; 3],
    finish: Finish,
    shape: Shape,
}

impl Part {
    const fn shaped(self, shape: Shape) -> Part {
        Part { shape, ..self }
    }
    const fn with(self, finish: Finish) -> Part {
        Part { finish, ..self }
    }
}

/// A round wall-fitting part: centred `along` the wall and `out` from it, half-extents
/// `(along, out)`.
const fn r(along: f32, out: f32, half: (f32, f32), z: (f32, f32), rgb: [f32; 3], s: Shape) -> Part {
    pa(
        along,
        half.0,
        out - half.1,
        2.0 * half.1,
        z,
        rgb,
        Finish::Plain,
    )
    .shaped(s)
}

/// A round wall-fitting part with a circular footprint of radius `rad`.
const fn rc(along: f32, out: f32, rad: f32, z: (f32, f32), rgb: [f32; 3], s: Shape) -> Part {
    r(along, out, (rad, rad), z, rgb, s)
}

const fn p(half: f32, inset: f32, depth: f32, z: (f32, f32), rgb: [f32; 3]) -> Part {
    pa(0.0, half, inset, depth, z, rgb, Finish::Plain)
}

const fn pf(half: f32, inset: f32, depth: f32, z: (f32, f32), rgb: [f32; 3], f: Finish) -> Part {
    pa(0.0, half, inset, depth, z, rgb, f)
}

const fn pa(
    along: f32,
    half: f32,
    inset: f32,
    depth: f32,
    z: (f32, f32),
    rgb: [f32; 3],
    finish: Finish,
) -> Part {
    Part {
        half,
        along,
        inset,
        depth,
        z0: z.0,
        z1: z.1,
        rgb,
        finish,
        shape: Shape::Box,
    }
}

// Palette.
const WHITE: [f32; 3] = [0.86, 0.86, 0.84];
const GREY: [f32; 3] = [0.55, 0.57, 0.6];
const DARK: [f32; 3] = [0.14, 0.14, 0.16];
const WOOD: [f32; 3] = [0.62, 0.46, 0.3];
const DARK_WOOD: [f32; 3] = [0.3, 0.17, 0.1];
const LEAF: [f32; 3] = [0.22, 0.47, 0.2];
const LEAF_LIGHT: [f32; 3] = [0.33, 0.6, 0.25];
const LEAF_DARK: [f32; 3] = [0.13, 0.34, 0.13];
const POT: [f32; 3] = [0.66, 0.36, 0.22];
const SOIL: [f32; 3] = [0.2, 0.13, 0.08];
const WATER: [f32; 3] = [0.45, 0.72, 0.98];
const SCREEN: [f32; 3] = [0.45, 0.72, 0.98];
const IRON: [f32; 3] = [0.3, 0.3, 0.33];
const BONE: [f32; 3] = [0.86, 0.82, 0.7];
const GOLD: [f32; 3] = [0.92, 0.74, 0.28];
const YELLOW: [f32; 3] = [0.95, 0.8, 0.12];
const STEEL: [f32; 3] = [0.7, 0.72, 0.75];
const CUPBOARD: [f32; 3] = [0.82, 0.8, 0.74];
const RED: [f32; 3] = [0.85, 0.08, 0.08];
const STONE: [f32; 3] = [0.36, 0.33, 0.3];

/// "The face toward the room" in a wall recipe — resolved when the fitting is placed.
const FRONT: usize = usize::MAX;

// Office hall fittings.
const COPIER: &[Part] = &[
    p(0.2, 0.01, 0.2, (0.0, 0.3), [0.76, 0.77, 0.79]),
    p(0.19, 0.02, 0.18, (0.3, 0.34), [0.32, 0.33, 0.36]),
    // Output tray, paper in it, the lit control panel.
    pa(-0.12, 0.07, 0.03, 0.17, (0.2, 0.22), GREY, Finish::Plain),
    pa(-0.12, 0.05, 0.05, 0.12, (0.22, 0.23), WHITE, Finish::Plain),
    pa(
        0.1,
        0.05,
        0.19,
        0.02,
        (0.26, 0.3),
        DARK,
        Finish::Screen(FRONT, [0.5, 0.9, 0.6]),
    ),
];
const FILING: &[Part] = &[
    pf(0.12, 0.02, 0.19, (0.0, 0.42), GREY, Finish::Bands(0.14)),
    // Drawer handles.
    p(0.035, 0.21, 0.01, (0.1, 0.115), DARK),
    p(0.035, 0.21, 0.01, (0.24, 0.255), DARK),
    p(0.035, 0.21, 0.01, (0.38, 0.395), DARK),
];
const PLANT: &[Part] = &[
    // Terracotta pot, its rolled rim, the soil.
    rc(0.0, 0.1, 0.052, (0.0, 0.1), POT, Shape::Cyl),
    rc(0.0, 0.1, 0.062, (0.1, 0.125), POT, Shape::Cyl),
    rc(0.0, 0.1, 0.054, (0.125, 0.127), SOIL, Shape::Cyl),
    // A bushy crown of leaf balls in three greens, spreading wider than the pot.
    rc(0.0, 0.1, 0.085, (0.127, 0.29), LEAF, Shape::Ball).with(Finish::Leafy),
    rc(-0.08, 0.09, 0.06, (0.17, 0.32), LEAF_DARK, Shape::Ball).with(Finish::Leafy),
    r(
        0.08,
        0.11,
        (0.065, 0.06),
        (0.18, 0.33),
        LEAF_LIGHT,
        Shape::Ball,
    ),
    r(-0.035, 0.1, (0.065, 0.06), (0.26, 0.41), LEAF, Shape::Ball).with(Finish::Leafy),
    r(
        0.045,
        0.12,
        (0.055, 0.05),
        (0.28, 0.42),
        LEAF_DARK,
        Shape::Ball,
    ),
    rc(0.0, 0.1, 0.045, (0.36, 0.47), LEAF_LIGHT, Shape::Ball).with(Finish::Leafy),
];
/// A water cooler: a white cabinet with its taps and drip tray, and the big blue
/// bottle upside down on top — neck-down shoulder, ribbed barrel, rounded end.
const COOLER: &[Part] = &[
    p(0.075, 0.03, 0.15, (0.0, 0.32), WHITE),
    pa(0.0, 0.05, 0.18, 0.004, (0.17, 0.26), GREY, Finish::Plain),
    pa(0.0, 0.05, 0.16, 0.04, (0.155, 0.165), DARK, Finish::Plain),
    rc(
        -0.025,
        0.19,
        0.009,
        (0.215, 0.245),
        [0.85, 0.15, 0.12],
        Shape::Cyl,
    ),
    rc(
        0.025,
        0.19,
        0.009,
        (0.215, 0.245),
        [0.2, 0.4, 0.9],
        Shape::Cyl,
    ),
    rc(0.0, 0.105, 0.05, (0.32, 0.335), GREY, Shape::Cyl),
    rc(0.0, 0.105, 0.058, (0.335, 0.375), WATER, Shape::Bowl).with(Finish::Glass),
    rc(0.0, 0.105, 0.058, (0.375, 0.47), WATER, Shape::Cyl).with(Finish::Glass),
    rc(0.0, 0.105, 0.06, (0.405, 0.41), WATER, Shape::Cyl).with(Finish::Glass),
    rc(0.0, 0.105, 0.06, (0.44, 0.445), WATER, Shape::Cyl).with(Finish::Glass),
    rc(0.0, 0.105, 0.058, (0.47, 0.49), WATER, Shape::Dome).with(Finish::Glass),
];
const BIN: &[Part] = &[
    rc(0.0, 0.09, 0.048, (0.0, 0.11), DARK, Shape::Cyl),
    rc(0.0, 0.09, 0.053, (0.11, 0.12), GREY, Shape::Cyl),
    rc(
        0.0,
        0.09,
        0.044,
        (0.12, 0.121),
        [0.04, 0.04, 0.05],
        Shape::Cyl,
    ),
];
const EXTINGUISHER: &[Part] = &[
    rc(0.0, 0.04, 0.03, (0.22, 0.35), [0.82, 0.1, 0.1], Shape::Cyl),
    rc(0.0, 0.04, 0.03, (0.35, 0.37), [0.82, 0.1, 0.1], Shape::Dome),
    rc(0.0, 0.04, 0.01, (0.37, 0.4), DARK, Shape::Cyl),
    pa(0.035, 0.008, 0.04, 0.016, (0.26, 0.39), DARK, Finish::Plain),
];
const CORKBOARD: &[Part] = &[
    p(0.22, 0.0, 0.02, (0.4, 0.7), [0.62, 0.45, 0.28]),
    pa(-0.1, 0.04, 0.02, 0.005, (0.58, 0.65), WHITE, Finish::Plain),
    pa(
        0.08,
        0.05,
        0.02,
        0.005,
        (0.46, 0.55),
        [0.95, 0.9, 0.5],
        Finish::Plain,
    ),
];
const WHITEBOARD: &[Part] = &[
    p(0.32, 0.0, 0.02, (0.32, 0.72), [0.93, 0.94, 0.96]),
    p(0.32, 0.0, 0.045, (0.31, 0.33), GREY),
    // Markers in the tray.
    pa(
        0.18,
        0.03,
        0.02,
        0.02,
        (0.33, 0.34),
        [0.8, 0.1, 0.1],
        Finish::Plain,
    ),
    pa(
        0.24,
        0.03,
        0.02,
        0.02,
        (0.33, 0.34),
        [0.1, 0.2, 0.8],
        Finish::Plain,
    ),
];
const OFFICE_HALL: &[&[Part]] = &[FILING, COOLER, PLANT, PLANT, BIN, EXTINGUISHER, CORKBOARD];
// One copier to a dozen-odd fittings: they were lining every wall.
const OFFICE_ROOM_WALL: &[&[Part]] = &[FILING, PLANT, BIN, FILING, PLANT, BIN, COPIER];

// Dungeon fittings.
const SHACKLES: &[Part] = &[
    p(0.03, 0.0, 0.03, (0.7, 0.74), IRON),
    pf(0.012, 0.01, 0.02, (0.44, 0.7), IRON, Finish::Links),
    p(0.03, 0.0, 0.035, (0.4, 0.44), IRON),
];
const BARREL: &[Part] = &[
    rc(0.0, 0.12, 0.09, (0.0, 0.25), [0.42, 0.27, 0.15], Shape::Cyl).with(Finish::Bands(0.08)),
    rc(
        0.0,
        0.12,
        0.085,
        (0.25, 0.255),
        [0.3, 0.19, 0.1],
        Shape::Cyl,
    ),
];
const CRATES: &[Part] = &[
    pf(
        0.1,
        0.02,
        0.19,
        (0.0, 0.19),
        [0.55, 0.42, 0.25],
        Finish::Bands(0.19),
    ),
    pf(
        0.07,
        0.05,
        0.13,
        (0.19, 0.32),
        [0.5, 0.38, 0.22],
        Finish::Bands(0.13),
    ),
];
const BONES: &[Part] = &[
    p(0.035, 0.06, 0.07, (0.0, 0.06), BONE),
    p(0.09, 0.03, 0.12, (0.0, 0.018), BONE),
    pa(0.06, 0.012, 0.1, 0.02, (0.018, 0.03), BONE, Finish::Plain),
];
const BANNER: &[Part] = &[
    p(0.12, 0.0, 0.015, (0.45, 0.9), [0.55, 0.1, 0.12]),
    p(0.04, 0.0, 0.02, (0.6, 0.72), GOLD),
    p(0.14, 0.0, 0.03, (0.9, 0.92), GOLD),
];
const BLOOD: [f32; 3] = [0.32, 0.03, 0.03];
const LEATHER: [f32; 3] = [0.16, 0.1, 0.07];
/// A stretching rack stood against the wall: a plank bed on a frame, a roller with a
/// winding wheel at each end.
const RACK: &[Part] = &[
    pa(
        -0.26,
        0.025,
        0.0,
        0.2,
        (0.0, 0.45),
        DARK_WOOD,
        Finish::Plain,
    ),
    pa(0.26, 0.025, 0.0, 0.2, (0.0, 0.45), DARK_WOOD, Finish::Plain),
    pf(
        0.24,
        0.02,
        0.16,
        (0.3, 0.34),
        [0.42, 0.3, 0.2],
        Finish::Bands(0.02),
    ),
    pf(0.24, 0.0, 0.03, (0.34, 0.37), IRON, Finish::Bands(0.01)),
    pa(0.31, 0.01, 0.06, 0.08, (0.3, 0.42), IRON, Finish::Plain),
    pa(-0.31, 0.01, 0.06, 0.08, (0.3, 0.42), IRON, Finish::Plain),
];
/// An iron maiden, shut (clean-ish: no spikes showing), a face cast in its lid.
const MAIDEN: &[Part] = &[
    pf(
        0.13,
        0.0,
        0.2,
        (0.0, 0.82),
        [0.2, 0.2, 0.22],
        Finish::Framed,
    ),
    p(0.05, 0.2, 0.012, (0.62, 0.74), [0.55, 0.5, 0.42]),
    p(0.03, 0.2, 0.01, (0.3, 0.33), IRON),
];
/// Stocks: a post with a hinged board, three holes (dark bands) for head and hands.
const STOCKS: &[Part] = &[
    p(0.03, 0.08, 0.06, (0.0, 0.5), DARK_WOOD),
    pf(
        0.2,
        0.06,
        0.06,
        (0.5, 0.6),
        [0.45, 0.32, 0.2],
        Finish::Bands(0.05),
    ),
    pa(
        -0.12,
        0.018,
        0.12,
        0.004,
        (0.53, 0.57),
        [0.05, 0.03, 0.02],
        Finish::Plain,
    ),
    pa(
        0.0,
        0.025,
        0.12,
        0.004,
        (0.52, 0.58),
        [0.05, 0.03, 0.02],
        Finish::Plain,
    ),
    pa(
        0.12,
        0.018,
        0.12,
        0.004,
        (0.53, 0.57),
        [0.05, 0.03, 0.02],
        Finish::Plain,
    ),
];
/// Old blood: a dried splash on the wall with a couple of runs, a stain on the floor.
const STAIN: &[Part] = &[
    pa(0.0, 0.09, 0.0, 0.003, (0.36, 0.5), BLOOD, Finish::Plain),
    pa(0.05, 0.05, 0.0, 0.004, (0.46, 0.56), BLOOD, Finish::Plain),
    pa(-0.06, 0.012, 0.0, 0.004, (0.2, 0.38), BLOOD, Finish::Plain),
    pa(0.03, 0.01, 0.0, 0.004, (0.26, 0.37), BLOOD, Finish::Plain),
    pa(
        0.0,
        0.14,
        0.0,
        0.14,
        (0.0, 0.003),
        [0.22, 0.03, 0.03],
        Finish::Plain,
    ),
];
/// A leather whip hung coiled on a hook, a studded collar beside it.
const WHIP: &[Part] = &[
    p(0.012, 0.0, 0.05, (0.72, 0.74), IRON),
    pf(0.01, 0.03, 0.015, (0.38, 0.72), LEATHER, Finish::Links),
    pf(0.05, 0.02, 0.02, (0.6, 0.66), LEATHER, Finish::Bands(0.02)),
    pa(0.14, 0.04, 0.0, 0.02, (0.58, 0.61), LEATHER, Finish::Plain),
    pa(
        0.14,
        0.035,
        0.0,
        0.024,
        (0.585, 0.6),
        IRON,
        Finish::Bands(0.01),
    ),
];
const DUNGEON_WALL: &[&[Part]] = &[
    SHACKLES, SHACKLES, BARREL, CRATES, BONES, BANNER, RACK, MAIDEN, STOCKS, STAIN, STAIN, WHIP,
];
/// Notices nailed up in the dungeon (Gachimuchi — "Dungeon Master" Billy Herrington,
/// "the leather club's two blocks down", "boy next door", "three hundred bucks"),
/// handed out without repeats on a floor.
const DUNGEON_NOTICES: &[&[&str]] = &[
    &["DUNGEON", "MASTER"],
    &["LEATHER CLUB", "TWO BLOCKS DOWN"],
    &["BOY NEXT DOOR"],
    &["DEEP DARK", "FANTASIES"],
    &["300 BUCKS"],
];
const PARCHMENT: Color = Color::new(0.82, 0.74, 0.55, 1.0);
const SEPIA: Color = Color::new(0.25, 0.12, 0.06, 1.0);

// Garden (Mirror mode): Alice in Wonderland / Through the Looking-Glass.
const CLOTH: [f32; 3] = [0.93, 0.92, 0.88];
const CHINA: [f32; 3] = [0.95, 0.95, 0.97];
const TEAPOT: [f32; 3] = [0.35, 0.5, 0.85];
const HAT: [f32; 3] = [0.1, 0.09, 0.1];
const HAT_BAND: [f32; 3] = [0.62, 0.12, 0.2];
const CARD: [f32; 3] = [0.95, 0.93, 0.85];
const ROSE: [f32; 3] = [0.82, 0.06, 0.1];
const ROSE_LEAF: [f32; 3] = [0.1, 0.3, 0.12];
const ROSE_LEAF_LIGHT: [f32; 3] = [0.16, 0.4, 0.15];
const PAINT: [f32; 3] = [0.97, 0.97, 0.95];
const TIN: [f32; 3] = [0.55, 0.57, 0.6];
const MARBLE: [f32; 3] = [0.82, 0.8, 0.74];
const IVORY: [f32; 3] = [0.94, 0.91, 0.84];
const CHESS_RED: [f32; 3] = [0.78, 0.1, 0.1];
const PINK: [f32; 3] = [0.96, 0.52, 0.62];
const PINK_DARK: [f32; 3] = [0.88, 0.38, 0.5];
const LEG_PINK: [f32; 3] = [0.9, 0.46, 0.55];
const BEAK: [f32; 3] = [0.95, 0.85, 0.8];
const HEDGEHOG: [f32; 3] = [0.36, 0.26, 0.16];
const SNOUT: [f32; 3] = [0.8, 0.65, 0.5];
const BOTTLE: [f32; 3] = [0.35, 0.55, 0.8];
const SPONGE: [f32; 3] = [0.9, 0.78, 0.55];
const ICING: [f32; 3] = [0.98, 0.8, 0.86];
const TILE_DARK: [f32; 3] = [0.15, 0.14, 0.13];
const TILE_LIGHT: [f32; 3] = [0.9, 0.88, 0.82];
const SIGN_PLATE: Color = Color::new(0.93, 0.9, 0.8, 1.0);
const SIGN_INK: Color = Color::new(0.16, 0.1, 0.06, 1.0);
/// A box no taller than this lies flat on the ground (a floor tile, a stain): it
/// can't hide anything standing, so it's painted first and never occludes overlays.
const FLAT: f32 = 0.01;
/// How far in from its two walls a corner figure's centre stands: its footprint (half
/// ≤ 0.095) then ends 0.275 short of the walked lines through the cell's middle.
const CORNER: f32 = 0.13;

const EAT_ME: &str = "EAT ME";
const DRINK_ME: &str = "DRINK ME";
const TPS_REPORTS: &[&str] = &["TPS REPORTS"];
const THIS_WAY: &str = "THIS WAY";
/// Looking-Glass House's poem, in mirror-writing as Alice first finds it.
const JABBERWOCKY: [&str; 2] = ["'TWAS BRILLIG, AND THE", "SLITHY TOVES"];

/// The Mad Hatter's tea party against a hedge: a clothed table, a teapot, cups on
/// saucers and the Hatter's top hat, its "10/6" price card still in the band.
const TEA: &[Part] = &[
    pa(
        -0.3,
        0.015,
        0.04,
        0.02,
        (0.0, 0.3),
        DARK_WOOD,
        Finish::Plain,
    ),
    pa(0.3, 0.015, 0.04, 0.02, (0.0, 0.3), DARK_WOOD, Finish::Plain),
    pa(
        -0.3,
        0.015,
        0.16,
        0.02,
        (0.0, 0.3),
        DARK_WOOD,
        Finish::Plain,
    ),
    pa(0.3, 0.015, 0.16, 0.02, (0.0, 0.3), DARK_WOOD, Finish::Plain),
    // The cloth: its front drop, then the top.
    p(0.36, 0.195, 0.005, (0.24, 0.3), CLOTH),
    p(0.36, 0.02, 0.18, (0.3, 0.33), CLOTH),
    // Teapot: a round pot, its lid and knob, spout and handle.
    r(
        -0.14,
        0.115,
        (0.06, 0.05),
        (0.33, 0.42),
        TEAPOT,
        Shape::Ball,
    ),
    rc(-0.14, 0.115, 0.034, (0.42, 0.445), TEAPOT, Shape::Dome),
    rc(-0.14, 0.115, 0.01, (0.445, 0.465), TEAPOT, Shape::Ball),
    rc(-0.205, 0.115, 0.011, (0.36, 0.405), TEAPOT, Shape::Cyl),
    rc(-0.215, 0.115, 0.009, (0.4, 0.42), TEAPOT, Shape::Ball),
    pa(
        -0.07,
        0.01,
        0.105,
        0.02,
        (0.35, 0.41),
        TEAPOT,
        Finish::Plain,
    ),
    // Cups on saucers.
    rc(-0.3, 0.115, 0.035, (0.33, 0.336), CHINA, Shape::Cyl),
    rc(-0.3, 0.115, 0.024, (0.336, 0.372), CHINA, Shape::Bowl),
    rc(0.03, 0.135, 0.035, (0.33, 0.336), CHINA, Shape::Cyl),
    rc(0.03, 0.135, 0.024, (0.336, 0.372), CHINA, Shape::Bowl),
    rc(0.3, 0.105, 0.035, (0.33, 0.336), CHINA, Shape::Cyl),
    rc(0.3, 0.105, 0.024, (0.336, 0.372), CHINA, Shape::Bowl),
    // The top hat: brim, crown, band, price card.
    rc(0.16, 0.125, 0.065, (0.33, 0.342), HAT, Shape::Cyl),
    rc(0.16, 0.125, 0.045, (0.342, 0.47), HAT, Shape::Cyl),
    rc(0.16, 0.125, 0.047, (0.352, 0.375), HAT_BAND, Shape::Cyl),
    pa(0.19, 0.014, 0.172, 0.003, (0.36, 0.39), CARD, Finish::Plain),
];

/// Red roses the Queen's gardeners are painting white: clumps set into the hedge's
/// surface — red, white, and some half done — the paint tin and brush at its foot and
/// a white splash on the grass.
const ROSES: &[Part] = &[
    // The bush: low overlapping leafy mounds in two greens.
    r(
        -0.26,
        0.08,
        (0.12, 0.09),
        (0.0, 0.3),
        ROSE_LEAF,
        Shape::Ball,
    )
    .with(Finish::Leafy),
    r(
        -0.08,
        0.09,
        (0.14, 0.1),
        (0.0, 0.36),
        ROSE_LEAF_LIGHT,
        Shape::Ball,
    )
    .with(Finish::Leafy),
    r(0.12, 0.09, (0.14, 0.1), (0.0, 0.34), ROSE_LEAF, Shape::Ball).with(Finish::Leafy),
    r(
        0.3,
        0.08,
        (0.1, 0.09),
        (0.0, 0.28),
        ROSE_LEAF_LIGHT,
        Shape::Ball,
    )
    .with(Finish::Leafy),
    r(
        -0.16,
        0.07,
        (0.12, 0.08),
        (0.2, 0.46),
        ROSE_LEAF,
        Shape::Ball,
    )
    .with(Finish::Leafy),
    r(
        0.04,
        0.08,
        (0.13, 0.09),
        (0.22, 0.5),
        ROSE_LEAF_LIGHT,
        Shape::Ball,
    )
    .with(Finish::Leafy),
    r(
        0.22,
        0.07,
        (0.1, 0.08),
        (0.18, 0.42),
        ROSE_LEAF,
        Shape::Ball,
    )
    .with(Finish::Leafy),
    // Blooms on its face: red, painted white, and half done (a white cap on a red one).
    rc(-0.3, 0.155, 0.026, (0.154, 0.206), ROSE, Shape::Ball),
    rc(-0.2, 0.135, 0.026, (0.274, 0.326), PAINT, Shape::Ball),
    rc(-0.1, 0.116, 0.026, (0.374, 0.426), ROSE, Shape::Ball),
    rc(-0.1, 0.116, 0.029, (0.4, 0.429), PAINT, Shape::Dome),
    rc(0.0, 0.157, 0.026, (0.214, 0.266), ROSE, Shape::Ball),
    rc(0.08, 0.148, 0.026, (0.394, 0.446), ROSE, Shape::Ball),
    rc(0.16, 0.177, 0.026, (0.124, 0.176), PAINT, Shape::Ball),
    rc(0.22, 0.137, 0.026, (0.314, 0.366), ROSE, Shape::Ball),
    rc(0.22, 0.137, 0.029, (0.34, 0.369), PAINT, Shape::Dome),
    rc(0.32, 0.151, 0.026, (0.174, 0.226), ROSE, Shape::Ball),
    rc(-0.02, 0.161, 0.026, (0.074, 0.126), ROSE, Shape::Ball),
    rc(-0.02, 0.161, 0.029, (0.1, 0.129), PAINT, Shape::Dome),
    rc(-0.15, 0.156, 0.026, (0.074, 0.126), ROSE, Shape::Ball),
    rc(0.12, 0.158, 0.026, (0.254, 0.306), PAINT, Shape::Ball),
    // The paint tin, its brush, a splash on the grass.
    rc(0.36, 0.14, 0.04, (0.0, 0.09), TIN, Shape::Cyl).with(Finish::Bands(0.03)),
    rc(0.36, 0.14, 0.034, (0.09, 0.093), PAINT, Shape::Cyl),
    pa(
        0.38,
        0.006,
        0.13,
        0.012,
        (0.093, 0.22),
        DARK_WOOD,
        Finish::Plain,
    ),
    r(0.24, 0.15, (0.07, 0.05), (0.0, 0.002), PAINT, Shape::Cyl),
];

/// A two-tier stone fountain backed into a hedge (a half-oval basin, as garden wall
/// fountains are): basin and rim, its pool, a pedestal, the upper bowl brimming over in
/// a see-through sheet of water, a finial and a little jet on top. Centred
/// `FOUNTAIN_OUT` from the hedge; its front stays within `MAX_DEPTH`.
const FOUNTAIN_OUT: f32 = 0.09;
const FOUNTAIN_STONE: [f32; 3] = [0.72, 0.7, 0.66];
const POOL: [f32; 3] = [0.28, 0.52, 0.78];
const SPRAY: [f32; 3] = [0.78, 0.9, 1.0];
const FOUNTAIN: &[Part] = &[
    r(
        0.0,
        0.09,
        (0.29, 0.125),
        (0.0, 0.12),
        FOUNTAIN_STONE,
        Shape::Cyl,
    ),
    r(0.0, 0.09, (0.3, 0.13), (0.12, 0.135), MARBLE, Shape::Cyl),
    r(0.0, 0.09, (0.27, 0.11), (0.135, 0.138), POOL, Shape::Cyl),
    rc(0.0, 0.075, 0.108, (0.14, 0.34), SPRAY, Shape::Cyl).with(Finish::Glass),
    rc(0.0, 0.075, 0.035, (0.138, 0.34), FOUNTAIN_STONE, Shape::Cyl),
    rc(0.0, 0.075, 0.11, (0.34, 0.4), MARBLE, Shape::Bowl),
    rc(0.0, 0.075, 0.1, (0.4, 0.402), POOL, Shape::Cyl),
    rc(0.0, 0.075, 0.02, (0.402, 0.45), FOUNTAIN_STONE, Shape::Ball),
    rc(0.0, 0.075, 0.007, (0.45, 0.52), SPRAY, Shape::Cyl).with(Finish::Glass),
    rc(0.0, 0.075, 0.035, (0.5, 0.535), SPRAY, Shape::Dome).with(Finish::Glass),
];

/// A clock set into the hedge: a wooden case just proud of the leaves (its face is a
/// label, stopped at six — always tea-time).
const CLOCK_CASE: &[Part] = &[p(0.12, 0.03, 0.04, (0.48, 0.72), DARK_WOOD)];
/// Height of the clock face's centre, and how far out of the wall it sits.
const CLOCK_Z: f32 = 0.6;
const CLOCK_OUT: f32 = 0.072;

/// A little sign on a stake in front of the hedge, its board's face `SIGN_OUT` out.
const SIGN_OUT: f32 = 0.141;
const STAKE: Part = p(0.012, 0.1, 0.02, (0.0, 0.45), WOOD);
const EAT_ME_SIGN: &[Part] = &[
    STAKE,
    p(0.13, 0.12, 0.02, (0.4, 0.52), WOOD),
    // The cake, iced, a cherry on top.
    rc(0.28, 0.145, 0.05, (0.0, 0.07), SPONGE, Shape::Cyl),
    rc(0.28, 0.145, 0.052, (0.07, 0.085), ICING, Shape::Cyl),
    rc(0.28, 0.145, 0.012, (0.085, 0.109), ROSE, Shape::Ball),
];
const DRINK_ME_SIGN: &[Part] = &[STAKE, p(0.14, 0.12, 0.02, (0.4, 0.52), WOOD)];
/// The DRINK ME bottle at the sign's foot: glass, shoulder, neck, cork, paper tag.
const DRINK_ME_BOTTLE: &[Part] = &[
    rc(0.28, 0.145, 0.025, (0.0, 0.1), BOTTLE, Shape::Cyl).with(Finish::Glass),
    rc(0.28, 0.145, 0.025, (0.1, 0.13), BOTTLE, Shape::Dome).with(Finish::Glass),
    rc(0.28, 0.145, 0.009, (0.13, 0.17), BOTTLE, Shape::Cyl).with(Finish::Glass),
    rc(0.28, 0.145, 0.008, (0.17, 0.185), WOOD, Shape::Cyl),
    pa(0.28, 0.02, 0.17, 0.002, (0.03, 0.08), CARD, Finish::Plain),
];
const JABBER_SIGN: &[Part] = &[STAKE, p(0.2, 0.12, 0.02, (0.38, 0.58), WOOD)];
/// A signpost: a post and two boards, the upper pointing left, the lower right —
/// both saying THIS WAY.
const SIGNPOST: &[Part] = &[
    p(0.015, 0.1, 0.03, (0.0, 0.78), WOOD),
    p(0.02, 0.095, 0.04, (0.78, 0.8), WOOD),
    pa(-0.1, 0.16, 0.13, 0.02, (0.6, 0.7), WOOD, Finish::Plain),
    pa(0.1, 0.16, 0.13, 0.02, (0.46, 0.56), WOOD, Finish::Plain),
];

/// A free-standing figure's layer: `(forward, side)` offset of its centre, half sizes
/// `(forward, side)`, heights, colour, finish, shape.
#[derive(Clone, Copy)]
struct Layer {
    at: (f32, f32),
    half: (f32, f32),
    z: (f32, f32),
    rgb: [f32; 3],
    finish: Finish,
    shape: Shape,
}

impl Layer {
    const fn shaped(self, shape: Shape) -> Layer {
        Layer { shape, ..self }
    }
    const fn with(self, finish: Finish) -> Layer {
        Layer { finish, ..self }
    }
}

const fn l(at: (f32, f32), half: (f32, f32), z: (f32, f32), rgb: [f32; 3]) -> Layer {
    Layer {
        at,
        half,
        z,
        rgb,
        finish: Finish::Plain,
        shape: Shape::Box,
    }
}

/// A round layer: circular footprint of radius `rad` at `at`.
const fn lr(at: (f32, f32), rad: f32, z: (f32, f32), rgb: [f32; 3], shape: Shape) -> Layer {
    l(at, (rad, rad), z, rgb).shaped(shape)
}

/// A round marble plinth for the chess statues, its top at 0.22.
const PLINTH: &[Layer] = &[
    lr((0.0, 0.0), 0.085, (0.0, 0.2), MARBLE, Shape::Cyl),
    lr((0.0, 0.0), 0.095, (0.2, 0.22), MARBLE, Shape::Cyl),
];

/// A chess queen on a plinth (Looking-Glass country is a chessboard), turned: a base
/// ring, a flared skirt, a slender waist, a collar, a round head and a gold crown with
/// four points and a ball.
fn queen(rgb: [f32; 3]) -> Vec<Layer> {
    let mut v = PLINTH.to_vec();
    let o = (0.0, 0.0);
    v.extend_from_slice(&[
        lr(o, 0.075, (0.22, 0.25), rgb, Shape::Cyl),
        lr(o, 0.062, (0.25, 0.31), rgb, Shape::Dome),
        lr(o, 0.034, (0.28, 0.45), rgb, Shape::Cyl),
        lr(o, 0.054, (0.45, 0.48), rgb, Shape::Bowl),
        lr(o, 0.058, (0.48, 0.494), rgb, Shape::Cyl),
        lr(o, 0.038, (0.494, 0.56), rgb, Shape::Ball),
        lr(o, 0.042, (0.55, 0.58), GOLD, Shape::Cyl),
    ]);
    for (a, b) in [(1.0, 1.0), (1.0, -1.0), (-1.0, 1.0), (-1.0, -1.0)] {
        v.push(lr(
            (a * 0.028, b * 0.028),
            0.011,
            (0.58, 0.61),
            GOLD,
            Shape::Ball,
        ));
    }
    v.push(lr(o, 0.016, (0.58, 0.625), GOLD, Shape::Ball));
    v
}

/// A chess knight on a plinth: turned base and body, a neck, a mane down its back,
/// the head thrust forward with its muzzle, two ears.
fn knight(rgb: [f32; 3]) -> Vec<Layer> {
    let mut v = PLINTH.to_vec();
    v.extend_from_slice(&[
        lr((0.0, 0.0), 0.075, (0.22, 0.26), rgb, Shape::Cyl),
        lr((0.0, 0.0), 0.062, (0.26, 0.3), rgb, Shape::Dome),
        lr((0.0, 0.0), 0.05, (0.28, 0.38), rgb, Shape::Cyl),
        l((-0.01, 0.0), (0.042, 0.04), (0.36, 0.5), rgb).shaped(Shape::Cyl),
        l((-0.045, 0.0), (0.012, 0.02), (0.42, 0.56), rgb),
        l((0.015, 0.0), (0.05, 0.036), (0.48, 0.58), rgb).shaped(Shape::Ball),
        l((0.058, 0.0), (0.036, 0.028), (0.455, 0.535), rgb).shaped(Shape::Ball),
        l((-0.01, 0.02), (0.01, 0.008), (0.56, 0.605), rgb).shaped(Shape::Dome),
        l((-0.01, -0.02), (0.01, 0.008), (0.56, 0.605), rgb).shaped(Shape::Dome),
    ]);
    v
}

/// A croquet flamingo standing on one leg, the rolled-up hedgehog (the ball) at its
/// feet.
const FLAMINGO: &[Layer] = &[
    // The standing leg with its knee and foot; the other folded up under the body.
    lr((0.0, 0.01), 0.005, (0.0, 0.3), LEG_PINK, Shape::Cyl),
    lr((0.0, 0.01), 0.008, (0.14, 0.16), LEG_PINK, Shape::Ball),
    l((0.015, 0.01), (0.022, 0.009), (0.0, 0.012), LEG_PINK).shaped(Shape::Ball),
    lr((-0.02, -0.012), 0.005, (0.2, 0.29), LEG_PINK, Shape::Cyl),
    lr((-0.02, -0.012), 0.008, (0.19, 0.21), LEG_PINK, Shape::Ball),
    // Oval body, darker folded wings, the tail feathers.
    l((-0.02, 0.0), (0.065, 0.04), (0.28, 0.38), PINK).shaped(Shape::Ball),
    l((-0.03, 0.0), (0.05, 0.043), (0.31, 0.37), PINK_DARK).shaped(Shape::Ball),
    l((-0.075, 0.0), (0.025, 0.022), (0.33, 0.37), PINK_DARK).shaped(Shape::Ball),
    // The S of the neck, bead by bead (close enough to read as one curve).
    lr((0.035, 0.0), 0.011, (0.355, 0.389), PINK, Shape::Ball),
    lr((0.044, 0.0), 0.011, (0.371, 0.405), PINK, Shape::Ball),
    lr((0.052, 0.0), 0.011, (0.386, 0.42), PINK, Shape::Ball),
    lr((0.056, 0.0), 0.011, (0.4, 0.434), PINK, Shape::Ball),
    lr((0.057, 0.0), 0.011, (0.416, 0.45), PINK, Shape::Ball),
    lr((0.054, 0.0), 0.011, (0.434, 0.468), PINK, Shape::Ball),
    lr((0.05, 0.0), 0.011, (0.45, 0.484), PINK, Shape::Ball),
    lr((0.045, 0.0), 0.011, (0.464, 0.498), PINK, Shape::Ball),
    lr((0.038, 0.0), 0.011, (0.479, 0.513), PINK, Shape::Ball),
    lr((0.035, 0.0), 0.011, (0.496, 0.53), PINK, Shape::Ball),
    lr((0.034, 0.0), 0.011, (0.513, 0.547), PINK, Shape::Ball),
    lr((0.036, 0.0), 0.011, (0.528, 0.562), PINK, Shape::Ball),
    lr((0.041, 0.0), 0.011, (0.544, 0.578), PINK, Shape::Ball),
    lr((0.048, 0.0), 0.011, (0.558, 0.592), PINK, Shape::Ball),
    // Head, eyes, the pale beak bent down to its black tip.
    lr((0.052, 0.0), 0.02, (0.56, 0.6), PINK, Shape::Ball),
    lr((0.058, 0.017), 0.004, (0.582, 0.59), DARK, Shape::Ball),
    lr((0.058, -0.017), 0.004, (0.582, 0.59), DARK, Shape::Ball),
    l((0.076, 0.0), (0.016, 0.009), (0.566, 0.586), BEAK).shaped(Shape::Ball),
    lr((0.09, 0.0), 0.007, (0.553, 0.571), DARK, Shape::Ball),
    // The hedgehog, rolled up: a bristly ball, its snout and nose poking out.
    l((0.05, 0.06), (0.035, 0.03), (0.0, 0.056), HEDGEHOG)
        .shaped(Shape::Ball)
        .with(Finish::Bands(0.011)),
    l((0.083, 0.06), (0.014, 0.012), (0.012, 0.034), SNOUT).shaped(Shape::Ball),
    lr((0.096, 0.06), 0.004, (0.02, 0.028), DARK, Shape::Ball),
];

/// A garden label on a hedge-side sign or clock: `spec` drawn flat at floor point
/// `at`, facing `out`, centred at height `z`, sized by `Label::size(cap, width)`.
struct Mark {
    at: Vec2,
    out: Vec2,
    spec: Spec,
    z: f32,
    cap: f32,
    width: f32,
}

/// The Cheshire Cat's grin, floating just out of a hedge face at `at` (facing `out`),
/// fading in and out on its own `phase`.
struct Grin {
    at: Vec2,
    out: Vec2,
    phase: f32,
}

/// Something in the Garden worth a look: its bounding box in `cell` (floor-plane
/// corners and height range), and — for a flat thing on a hedge face (a grin) — the way
/// it faces, since it can't be made out edge-on. Looked at from its own cell or the
/// open cells next door, from whichever it fits the view from (`Decor::looks_at`).
struct Sight {
    cell: Cell,
    min: Vec2,
    max: Vec2,
    z: (f32, f32),
    face: Option<Vec2>,
}

/// A glance at something from a cell: `key` (what it is — a sight's index for
/// `looks_at`, the wall's `DIRS` index for `reads_at` — so it's looked at only once),
/// the world yaw toward it, how far to look down (`Anim::pitch` units) and how far to
/// stoop (eye drop, grid units) to get a low thing into view.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Glance {
    pub key: usize,
    pub yaw: f32,
    pub pitch: f32,
    pub stoop: f32,
}

/// Look-down limit: a y-shear past this smears.
const MAX_PITCH: f32 = 0.3;
/// Stoop limit — as low as the eye sinks on the Dungeon's stairs.
const MAX_STOOP: f32 = 0.2;
/// Of the view's half-height/-width, how much a glanced-at thing may fill.
const FILL: f32 = 0.45;
/// The view's width over its height, at least (the 3D view is landscape).
const ASPECT: f32 = 1.2;

/// Aim the eye, standing at `eye` (floor point, eye height 0.5), at a box (`min`..`max`
/// on the floor, `z` high): yaw toward its middle, and the pitch — then a stoop, once
/// the pitch runs out — that puts its middle mid-view. `None` when it won't fit the
/// view whole from here: the near edge too close for its height or width, or its
/// bottom still out of view at full stoop. The view shows a world unit a view-height
/// tall at depth 1 (`raycast`), so at depth `d` a point `z` high sits
/// `(eye_z - z) / d - pitch` view heights below the middle.
fn aim(eye: Vec2, min: Vec2, max: Vec2, z: (f32, f32)) -> Option<(f32, f32, f32)> {
    let v = (min + max) * 0.5 - eye;
    let dist = v.length();
    if dist < 0.15 {
        return None;
    }
    let (fwd, side) = (v / dist, vec2(-v.y, v.x) / dist);
    let size = max - min;
    let (deep, wide) = (fwd.abs().dot(size.abs()) * 0.5, side.abs().dot(size.abs()));
    let near = dist - deep;
    let mid = (z.0 + z.1) * 0.5;
    let want = (0.5 - mid) / dist;
    let pitch = want.clamp(0.0, MAX_PITCH);
    let stoop = ((want - pitch) * dist).clamp(0.0, MAX_STOOP);
    let eye_z = 0.5 - stoop;
    let fits = near > 0.1
        && z.1 - z.0 <= 2.0 * FILL * near
        && wide <= 2.0 * FILL * ASPECT * near
        && (eye_z - z.0) / near - pitch <= FILL
        && (eye_z - z.1) / near - pitch >= -FILL;
    fits.then_some((v.y.atan2(v.x), pitch, stoop))
}

/// A whiteboard's writing surface, for the marker overlay.
struct Board {
    /// Centre of the board's face, on the floor plane.
    at: Vec2,
    /// Along the board, and out of the wall.
    along: Vec2,
    out: Vec2,
    half: f32,
    writing: Writing,
    seed: f32,
}

#[derive(Clone, Copy, PartialEq)]
enum Writing {
    /// Marker scribbles.
    Notes,
    /// A conspiracy wall: pinned papers, red string, "PEPE SILVIA" (It's Always Sunny).
    Pepe,
    /// "THE CAKE IS A LIE", scrawled (Portal).
    Cake,
}

/// A printed sheet on a wall, its text centred at height `z`, at most `width` wide.
struct Poster {
    at: Vec2,
    out: Vec2,
    lines: &'static [&'static str],
    /// A pictogram beside the text.
    icon: Option<Icon>,
    z: f32,
    width: f32,
    /// Cap height of its lettering (grid units); `width` caps it for long lines.
    cap: f32,
    ink: Color,
    paper: Color,
    /// Lit from within (neon): not dimmed by distance the way a printed sheet is.
    glow: bool,
}

/// Tower's lift: the two door leaves' closed footprints and the way each slides open.
struct Lift {
    leaves: [(usize, Vec2, Vec2, Vec2); 2],
}

#[derive(Clone, Copy)]
struct Hit {
    key: f32,
    tn: f32,
    tf: f32,
    normal: Vec2,
    /// How far off a round block's axis the ray passes, as a fraction of its footprint
    /// (0 for a box).
    miss: f32,
    block: u32,
    /// Nothing of it below this screen y: the part sunk inside a block it stands in
    /// (a mop handle in its bucket), which that block's faces hide.
    clip: f32,
}

/// What moves in the dressing from frame to frame.
#[derive(Clone, Copy)]
pub struct Dynamic {
    /// Wall-clock seconds (blinking lights).
    pub t: f32,
    /// The boss's HP as a fraction of full.
    pub boss_hp: f32,
    /// How far the lift doors have slid open, 0..1.
    pub lift: f32,
    /// Forest: where the campfire (the exit) burns this frame, if its ground exists.
    pub fire: Option<Vec2>,
}

pub struct Decor {
    blocks: Vec<Block>,
    boards: Vec<Board>,
    /// Door index -> the sign on its hall side.
    signs: Vec<(usize, Spec)>,
    /// A washroom door with an OUT OF ORDER sheet taped under its sign.
    out_of_order: Option<usize>,
    /// The hall cell outside that door and the direction to it — the sheet is read
    /// from there (`reads_at`).
    out_of_order_seen: Option<(Cell, usize)>,
    posters: Vec<Poster>,
    lift: Option<Lift>,
    /// Forest: the campfire's blocks (a range of `blocks`) and their offsets from its
    /// centre.
    fire: Option<(usize, usize)>,
    fire_base: Vec<(Vec2, Vec2)>,
    /// Garden: sign and clock labels, and the Cheshire Cat's grins.
    marks: Vec<Mark>,
    grins: Vec<Grin>,
    /// Garden: where the fountain stands (grid coords), if one was placed.
    fountain: Option<Vec2>,
    /// Garden: things worth turning to look at (`looks_at`).
    sights: Vec<Sight>,
    /// Dungeon: the stairwell's arch plane, `(axis, coordinate, sign into the well)`.
    /// The exit cell's floor hides anything below it past that plane, so a sunk block
    /// (`z0 < 0`) is clipped at the floor's edge there, column by column.
    well: Option<(usize, f32, f32)>,
    group: u32,
    /// Blocks that move every frame: index, resting placement, and how.
    moves: Vec<(u32, Block, Motion)>,
    /// Each fitting's block range and footprint bounds, for culling whole fittings.
    fittings: Vec<Fitting>,
    buckets: Buckets,
    /// Thin walls (`Maze::thin_walls`) — where the walls' faces are (`face`).
    thin: bool,
    /// Dungeon: the stairwell's blocks (a range of `blocks`), and of those the flight's
    /// — behind the wall, seen only through the portal (`View::through_portal`), where
    /// nothing else past the arch shows (`View::behind_portal`).
    stairs: (u32, u32),
    flight: (u32, u32),
    /// Per-frame scratch: each screen column's hits, and its nearest decor depth.
    cols: Vec<Vec<Hit>>,
    front: Occluders,
}

/// One fitting's blocks (`start..end`, contiguous) and the bounds of their footprints.
/// `moving` ones (lift leaves, the campfire) change place each frame: never culled.
struct Fitting {
    start: u32,
    end: u32,
    min: Vec2,
    max: Vec2,
    moving: bool,
}

/// Whether footprint bounds `min..max` lie wholly behind the walls in every screen
/// column (within `lo..hi`) they can cover: no point in them is nearer than their
/// nearest corner's view depth.
fn hidden(view: &View, min: Vec2, max: Vec2, lo: f32, hi: f32) -> bool {
    let Some((x0, x1, near)) = columns(view, min, max) else {
        return true;
    };
    let mut x = x0.max(lo);
    while x <= x1.min(hi - 1.0) {
        if view.visible(x, near) {
            return false;
        }
        x += COL_STEP;
    }
    true
}

/// The blocks bucketed by the grid square their footprint's centre lies in, so a
/// mirror's pass (`Decor::draw_reflection`, a few per frame) visits only those inside
/// its glass's view wedge rather than every block in the maze. `moving` blocks (lift
/// leaves, the campfire) are re-placed each frame and always visited.
#[derive(Default)]
struct Buckets {
    min: IVec2,
    w: i32,
    h: i32,
    cells: Vec<Vec<u32>>,
    /// Largest half-extent of any block: how far past a square its blocks can reach.
    pad: f32,
    moving: Vec<u32>,
}

impl Buckets {
    fn new(blocks: &[Block], moving: Vec<u32>) -> Buckets {
        let at = |b: &Block| ((b.min + b.max) * 0.5).floor().as_ivec2();
        let fixed = || (0..blocks.len() as u32).filter(|i| !moving.contains(i));
        let (mut lo, mut hi) = (IVec2::MAX, IVec2::MIN);
        let mut pad: f32 = 0.0;
        for i in fixed() {
            let b = &blocks[i as usize];
            (lo, hi) = (lo.min(at(b)), hi.max(at(b)));
            pad = pad.max((b.max - b.min).max_element() * 0.5);
        }
        if lo.x > hi.x {
            return Buckets {
                moving,
                ..Buckets::default()
            };
        }
        let (w, h) = (hi.x - lo.x + 1, hi.y - lo.y + 1);
        let mut cells = vec![Vec::new(); (w * h) as usize];
        for i in fixed() {
            let q = at(&blocks[i as usize]) - lo;
            cells[(q.y * w + q.x) as usize].push(i);
        }
        Buckets {
            min: lo,
            w,
            h,
            cells,
            pad,
            moving,
        }
    }

    /// Every block that can overlap the box `a`..`b` (plus the moving ones).
    fn within(&self, a: Vec2, b: Vec2) -> Vec<u32> {
        let mut out = self.moving.clone();
        let q0 = ((a - Vec2::splat(self.pad)).floor().as_ivec2() - self.min).max(IVec2::ZERO);
        let q1 = ((b + Vec2::splat(self.pad)).floor().as_ivec2() - self.min)
            .min(ivec2(self.w - 1, self.h - 1));
        for y in q0.y..=q1.y {
            for x in q0.x..=q1.x {
                out.extend_from_slice(&self.cells[(y * self.w + x) as usize]);
            }
        }
        out
    }
}

fn hash(seed: u64, c: Cell, k: u64) -> f32 {
    let mut v = seed ^ ((c.x as u64) << 32 | c.y as u32 as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    v ^= k.wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    v = v.wrapping_mul(0xD1B5_4A32_D192_ED03);
    v ^= v >> 29;
    v = v.wrapping_mul(0x94D0_49BB_1331_11EB);
    v ^= v >> 32;
    (v & 0xFFFF) as f32 / 65_536.0
}

fn pick<T: Copy>(items: &[T], h: f32) -> T {
    items[((h * items.len() as f32) as usize).min(items.len() - 1)]
}

/// A point on room floor `(min, size)` in `place`'s frame facing `toward`: `a` from the
/// far side (0) to the `toward` side (1), `b` across (0..1).
fn room_point((min, size): (Vec2, Vec2), toward: usize, a: f32, b: f32) -> Vec2 {
    let v = match toward {
        2 => vec2(b, a),
        0 => vec2(b, 1.0 - a),
        1 => vec2(a, b),
        _ => vec2(1.0 - a, b),
    };
    min + v * size
}

/// Floor `f` (in `place`'s frame facing `toward`) moved `da` toward `toward` and `db`
/// across, in world units: lays a fitting out against the real floor of a thin-walled
/// room (`Decor::recess` wider each side) without scaling it.
fn shifted((min, size): (Vec2, Vec2), toward: usize, da: f32, db: f32) -> (Vec2, Vec2) {
    let across = if toward.is_multiple_of(2) {
        Vec2::X
    } else {
        Vec2::Y
    };
    (min + dir_vec(toward) * da + across * db, size)
}

/// Top-left of a cell's floor square in grid coordinates.
fn square(c: Cell) -> Vec2 {
    vec2(2.0 * c.x as f32 + 1.0, 2.0 * c.y as f32 + 1.0)
}

fn dir_vec(d: usize) -> Vec2 {
    vec2(DIRS[d].0 as f32, DIRS[d].1 as f32)
}

/// A room's floor (cells plus the squares between them) as `(top-left, size)`.
fn floor(r: &Room) -> (Vec2, Vec2) {
    let min = square(Cell { x: r.x, y: r.y });
    (min, vec2(2.0 * r.w as f32 - 1.0, 2.0 * r.h as f32 - 1.0))
}

/// A room's real floor, out to its walls' faces, as a frame facing its door (`dir`):
/// `a` runs from the far wall's face (0) toward the door (`depth`), `b` across it from
/// the middle (±`half`), both in world units. The conference rooms are laid out on it
/// directly — authored as fractions of the full-square floor, they huddled in the middle
/// of the wider thin-walled room.
#[derive(Clone, Copy)]
struct Floor {
    far: Vec2,
    fwd: Vec2,
    side: Vec2,
    dir: usize,
    depth: f32,
    half: f32,
}

impl Floor {
    /// Floor `f` (`floor`) grown by `grow` each side, door toward `dir`.
    fn new((min, size): (Vec2, Vec2), grow: f32, dir: usize) -> Floor {
        let (min, size) = (min - Vec2::splat(grow), size + Vec2::splat(2.0 * grow));
        let fwd = dir_vec(dir);
        let side = vec2(-fwd.y, fwd.x);
        let depth = (size * fwd).length();
        Floor {
            far: min + size * 0.5 - fwd * depth * 0.5,
            fwd,
            side,
            dir,
            depth,
            half: (size * side).length() * 0.5,
        }
    }

    fn at(&self, a: f32, b: f32) -> Vec2 {
        self.far + self.fwd * a + self.side * b
    }

    /// World bounds of the box `a` × `b`.
    fn rect(&self, a: (f32, f32), b: (f32, f32)) -> (Vec2, Vec2) {
        let (p, q) = (self.at(a.0, b.0), self.at(a.1, b.1));
        (p.min(q), p.max(q))
    }

    /// World half-extents of `ha` along `a`, `hb` across.
    fn span(&self, ha: f32, hb: f32) -> Vec2 {
        (self.fwd * ha + self.side * hb).abs()
    }
}

impl Decor {
    /// `lamp_at(p)`: is a lamp (a torch) already mounted near wall point `p`?
    pub fn build(kind: ThemeKind, maze: &Maze, lamp_at: impl Fn(Vec2) -> bool, seed: u64) -> Decor {
        let mut d = Decor {
            blocks: Vec::new(),
            boards: Vec::new(),
            signs: Vec::new(),
            out_of_order: None,
            out_of_order_seen: None,
            posters: Vec::new(),
            lift: None,
            fire: None,
            fire_base: Vec::new(),
            marks: Vec::new(),
            grins: Vec::new(),
            fountain: None,
            sights: Vec::new(),
            well: None,
            group: 0,
            moves: Vec::new(),
            fittings: Vec::new(),
            cols: Vec::new(),
            front: Occluders::default(),
            buckets: Buckets::default(),
            thin: maze.thin_walls(),
            stairs: (0, 0),
            flight: (0, 0),
        };
        match kind {
            ThemeKind::Backrooms => d.office(maze, seed),
            ThemeKind::Dungeon => d.dungeon(maze, &lamp_at, seed),
            ThemeKind::Forest => d.campfire(),
            ThemeKind::Garden => d.garden(maze, seed),
        }
        let moving: Vec<u32> = d
            .lift
            .iter()
            .flat_map(|l| l.leaves.iter().map(|leaf| leaf.0 as u32))
            .chain(d.fire.into_iter().flat_map(|(a, b)| a as u32..b as u32))
            .chain(d.moves.iter().map(|m| m.0))
            .collect();
        let mut start = 0;
        while start < d.blocks.len() {
            let g = d.blocks[start].group;
            let end = (start..d.blocks.len())
                .find(|&i| d.blocks[i].group != g)
                .unwrap_or(d.blocks.len());
            let bs = &d.blocks[start..end];
            d.fittings.push(Fitting {
                start: start as u32,
                end: end as u32,
                min: bs.iter().fold(Vec2::MAX, |m, b| m.min(b.min)),
                max: bs.iter().fold(Vec2::MIN, |m, b| m.max(b.max)),
                moving: (start..end).any(|i| moving.contains(&(i as u32))),
            });
            start = end;
        }
        d.buckets = Buckets::new(&d.blocks, moving);
        d
    }

    /// Distance from a cell's centre to the face of any of its walls (`face_dist`).
    fn face(&self) -> f32 {
        crate::maze::face_dist(self.thin)
    }

    /// More desks round an open-plan room: one against most free stretches of its outer
    /// walls, facing the wall, the chair toward the room — in the floor a thin wall
    /// leaves beside its slab, clear of the walked lines through the cells. (Only the
    /// inner pillar squares had desks: a 2×2 room got one and read empty.) Not in a
    /// door's cell (its leaf swings there), nor on a wall already fitted.
    fn open_space_desks(&mut self, maze: &Maze, r: &Room, fitted: &[(Cell, usize)], seed: u64) {
        if !self.thin {
            return;
        }
        for c in r.cells() {
            if (0..4).any(|d| maze.door_at(c, d).is_some()) {
                continue;
            }
            // One per cell — two in a corner cell would collide — on a hashed free wall.
            let free: Vec<usize> = (0..4)
                .filter(|&d| maze.edge(c, d) == EdgeKind::Wall && !fitted.contains(&(c, d)))
                .collect();
            if free.is_empty() || hash(seed, c, 60) >= 0.85 {
                continue;
            }
            let d = pick(&free, hash(seed, c, 61));
            // `desk` lays out a unit square with the desk's back 0.35 in from its far
            // side from the chair: put that back against the wall face.
            let mid = square(c) + vec2(0.5, 0.5) + dir_vec(d) * (self.face() - 0.16);
            let h = hash(seed, c, 62);
            self.desk((mid - Vec2::splat(0.5), Vec2::ONE), opposite(d), h);
        }
    }

    /// How far every wall stands back from where a full-square wall's face would be: the
    /// floor a thin wall leaves either side of its slab (0 for full squares).
    fn recess(&self) -> f32 {
        if self.thin { 0.5 - WALL_T * 0.5 } else { 0.0 }
    }

    /// Room `r`'s dressing (blocks from `b0`, posters from `p0`) was laid out on the
    /// full-square floor; among thin walls the room is wider by `recess` each side.
    /// Every fitting standing against a wall (within `TOUCH`) moves back with it, whole
    /// and unscaled — except toward the door's wall, where the open leaf stands; in a
    /// peek-only room everything moves back from the door with the far wall (the view
    /// in is composed against it, and the leaf needs the floor inside the door). Wall
    /// posters go with their wall, the rest with the fitting they're on.
    fn settle_room(&mut self, r: &Room, door: usize, b0: usize, p0: usize) {
        const TOUCH: f32 = 0.1;
        // A wall-hung fitting this close to a wall belongs on it.
        const HUNG: f32 = 0.25;
        let m = self.recess();
        if m == 0.0 {
            return;
        }
        let (min, size) = floor(r);
        let max = min + size;
        let door_wall = dir_vec(door);
        let shift_of = |lo: Vec2, hi: Vec2| {
            let mut v = Vec2::ZERO;
            for k in 0..2 {
                let (a, b) = (lo[k] - min[k] < TOUCH, max[k] - hi[k] < TOUCH);
                v[k] = match (a, b) {
                    (true, false) => -m,
                    (false, true) => m,
                    _ => 0.0,
                };
            }
            // Never toward the door's wall.
            if v.dot(door_wall) > 0.0 || r.peek_only() {
                v -= door_wall * v.dot(door_wall);
            }
            if r.peek_only() {
                v -= door_wall * m;
            }
            v
        };
        // Each fitting (a run of one `group`): its block range, bounds and shift.
        let mut groups: Vec<(usize, usize, Vec2, Vec2, Vec2, bool)> = Vec::new();
        // Standing against a wall of the room (so moved by it, not by a neighbour).
        let walled =
            |lo: Vec2, hi: Vec2| (0..2).any(|k| lo[k] - min[k] < TOUCH || max[k] - hi[k] < TOUCH);
        let mut i = b0;
        while i < self.blocks.len() {
            let g = self.blocks[i].group;
            let end = (i..self.blocks.len())
                .find(|&j| self.blocks[j].group != g)
                .unwrap_or(self.blocks.len());
            let lo = self.blocks[i..end]
                .iter()
                .fold(Vec2::MAX, |a, b| a.min(b.min));
            let hi = self.blocks[i..end]
                .iter()
                .fold(Vec2::MIN, |a, b| a.max(b.max));
            // Hung on a wall (off the floor, resting on nothing): snap it flush to the
            // wall's new face — mounted a few cm off the old face (the washroom mirror,
            // the kitchen TV), it stayed put and hung in mid-air.
            let bs = &self.blocks[i..end];
            let off_floor = bs.iter().all(|b| b.z0 > 0.02);
            let rests = || {
                bs.iter().any(|b| {
                    self.blocks[b0..].iter().any(|o| {
                        o.group != g
                            && o.z1 <= b.z0 + 0.02
                            && o.z1 >= b.z0 - 0.2
                            && o.min.cmplt(b.max).all()
                            && b.min.cmplt(o.max).all()
                    })
                })
            };
            let hung = off_floor && !rests();
            let v = if hung {
                let mut v = shift_of(lo, hi);
                for k in 0..2 {
                    let (a, b) = (lo[k] - min[k], max[k] - hi[k]);
                    if a < HUNG && a <= b {
                        v[k] = -(a + m);
                    } else if b < HUNG {
                        v[k] = b + m;
                    }
                }
                v
            } else {
                shift_of(lo, hi)
            };
            let wall = walled(lo, hi) || hung;
            groups.push((i, end, lo, hi, v, wall));
            i = end;
        }
        // What stands right up against a fitting that moves goes with it — a desk's
        // chair, left behind, opened a gap as wide as the recess between them.
        // The nearest one, not the first: the IT room's chairs followed the racks
        // across the aisle, away from their desks.
        const WITH: f32 = 0.3;
        let gap = |(lo, hi): (Vec2, Vec2), (lo2, hi2): (Vec2, Vec2)| {
            (lo - hi2).max(lo2 - hi).max(Vec2::ZERO).length()
        };
        for k in 0..groups.len() {
            if groups[k].5 {
                continue;
            }
            let (lo, hi) = (groups[k].2, groups[k].3);
            if let Some((d, v)) = groups
                .iter()
                .filter(|o| o.5 && o.4 != Vec2::ZERO)
                .map(|o| (gap((lo, hi), (o.2, o.3)), o.4))
                .min_by(|a, b| a.0.total_cmp(&b.0))
                && d < WITH
            {
                groups[k].4 = v;
            }
        }
        for &(i, end, _, _, v, _) in &groups {
            for b in &mut self.blocks[i..end] {
                b.min += v;
                b.max += v;
            }
            for mv in self
                .moves
                .iter_mut()
                .filter(|mv| (i..end).contains(&(mv.0 as usize)))
            {
                mv.1.min += v;
                mv.1.max += v;
                // A blob rises round its lamp's axis: that moves too.
                if let Motion::Blob { axis, .. } = &mut mv.2 {
                    *axis += v;
                }
            }
        }
        for p in &mut self.posters[p0..] {
            // On a wall: its face is where the poster's back is.
            let back = p.at - p.out * 0.02;
            let on_wall = (0..2).any(|k| {
                p.out[k].abs() > 0.9
                    && ((back[k] - min[k]).abs() < 0.03 || (back[k] - max[k]).abs() < 0.03)
            });
            let v = if on_wall {
                -p.out * m
            } else {
                groups
                    .iter()
                    .map(|&(_, _, lo, hi, v, _)| {
                        ((lo - p.at).max(p.at - hi).max(Vec2::ZERO).length(), v)
                    })
                    .min_by(|a, b| a.0.total_cmp(&b.0))
                    .map_or(Vec2::ZERO, |g| g.1)
            };
            p.at += v;
        }
    }

    fn push(&mut self, min: Vec2, max: Vec2, z: (f32, f32), rgb: [f32; 3], finish: Finish) {
        self.blocks.push(Block {
            min,
            max,
            z0: z.0,
            z1: z.1,
            rgb,
            finish,
            group: self.group,
            shape: Shape::Box,
        });
    }

    /// Make the last pushed block `shape`: a round one gets a circular footprint (the
    /// largest inside its box), since a `place` box scales with an oblong room.
    fn shaped(&mut self, shape: Shape) {
        let b = self.blocks.last_mut().expect("a block to shape");
        b.shape = shape;
        if shape != Shape::Box {
            let (c, h) = ((b.min + b.max) * 0.5, (b.max - b.min) * 0.5);
            let rad = Vec2::splat(h.min_element());
            (b.min, b.max) = (c - rad, c + rad);
        }
    }

    /// Wall fitting `parts` against wall `dir` of cell `c`, centred `along` the wall
    /// (-0.5..0.5 from its middle).
    fn fit(&mut self, c: Cell, dir: usize, along: f32, parts: &[Part]) {
        self.group += 1;
        // Out to the wall's face (`face`): laid out against a full square's.
        let sq = square(c) + dir_vec(dir) * (self.face() - 0.5);
        let front = opposite(dir);
        for part in parts {
            debug_assert!(part.inset + part.depth <= MAX_DEPTH + 1e-4);
            let mid = 0.5 + along + part.along;
            let (a0, a1) = (mid - part.half, mid + part.half);
            let (d0, d1) = (part.inset, part.inset + part.depth);
            let (min, max) = match dir {
                0 => (vec2(a0, d0), vec2(a1, d1)),
                2 => (vec2(a0, 1.0 - d1), vec2(a1, 1.0 - d0)),
                3 => (vec2(d0, a0), vec2(d1, a1)),
                _ => (vec2(1.0 - d1, a0), vec2(1.0 - d0, a1)),
            };
            let finish = match part.finish {
                Finish::Screen(FRONT, col) => Finish::Screen(front, col),
                f => f,
            };
            self.push(sq + min, sq + max, (part.z0, part.z1), part.rgb, finish);
            self.blocks.last_mut().expect("just pushed").shape = part.shape;
        }
    }

    /// A box on floor `(min, size)`, in a frame facing `toward`: `a` runs from the far
    /// side (0) to the `toward` side (1), `b` across it (0..1).
    #[allow(clippy::too_many_arguments)]
    fn place(
        &mut self,
        (min, size): (Vec2, Vec2),
        toward: usize,
        a: (f32, f32),
        b: (f32, f32),
        z: (f32, f32),
        rgb: [f32; 3],
        finish: Finish,
    ) {
        let (lo, hi) = match toward {
            2 => (vec2(b.0, a.0), vec2(b.1, a.1)),
            0 => (vec2(b.0, 1.0 - a.1), vec2(b.1, 1.0 - a.0)),
            1 => (vec2(a.0, b.0), vec2(a.1, b.1)),
            _ => (vec2(1.0 - a.1, b.0), vec2(1.0 - a.0, b.1)),
        };
        self.push(min + lo * size, min + hi * size, z, rgb, finish);
    }

    /// A whiteboard on wall `dir` of cell `c`, marker overlay included.
    fn whiteboard(&mut self, c: Cell, dir: usize, writing: Writing, seed: f32) {
        self.fit(c, dir, 0.0, WHITEBOARD);
        let out = dir_vec(opposite(dir));
        self.boards.push(Board {
            at: square(c) + vec2(0.5, 0.5) + dir_vec(dir) * self.face() + out * 0.021,
            along: vec2(-out.y, out.x),
            out,
            half: 0.32,
            writing,
            seed,
        });
    }

    fn office(&mut self, maze: &Maze, seed: u64) {
        let lift_hall = self.lift(maze);
        let room_of = |c: Cell| maze.rooms.iter().find(|r| r.contains(c));
        // An open office door lies flat along its room cell's side wall: nothing goes on
        // a cell with a door.
        let doored = |c: Cell| (0..4).any(|d| maze.door_at(c, d).is_some());

        // Reception: a counter along the start cell's back wall, a bell on it, a
        // monitor on the receptionist's side.
        let start = maze.start;
        if let Some(back) = (0..4).find(|&d| maze.edge(start, d) == EdgeKind::Wall && d != 2) {
            self.fit(
                start,
                back,
                0.0,
                &[
                    p(0.4, 0.1, 0.12, (0.0, 0.36), WOOD),
                    p(0.42, 0.08, 0.14, (0.36, 0.39), DARK_WOOD),
                    pa(0.2, 0.02, 0.14, 0.03, (0.39, 0.41), GOLD, Finish::Plain),
                    pf(
                        0.09,
                        0.03,
                        0.02,
                        (0.26, 0.38),
                        DARK,
                        Finish::Screen(back, SCREEN),
                    ),
                ],
            );
        }

        // Whiteboards on the hall wall facing a door.
        // The hall outside the lift has its call button; nothing else on that cell.
        let mut taken: Vec<Cell> = lift_hall.into_iter().collect();
        // Vacant rooms (`Room::vacant`, decided with the floor plan): nothing inside,
        // nothing on their walls, no name on the door.
        let vacant = |r: &Room| r.vacant;
        // Open-space walls with a fitting on them.
        let mut fitted: Vec<(Cell, usize)> = Vec::new();
        for o in &maze.doors {
            let out = o.cell.step(o.dir);
            if o.sealed
                || !maze.cell_in_bounds(out)
                || room_of(out).is_some()
                || taken.contains(&out)
                || out == maze.start
                || out == maze.exit
            {
                continue;
            }
            // The wall straight across from the door, if that's a wall and no other
            // door's panel sweeps along it — now and then (every other door had one:
            // too many once every door opened).
            let doors_here = (0..4).filter(|&d| maze.door_at(out, d).is_some()).count();
            if doors_here == 1
                && maze.edge(out, o.dir) == EdgeKind::Wall
                && hash(seed, out, 11) < 0.15
            {
                let w = if hash(seed, out, 14) < 0.03 {
                    Writing::Cake
                } else {
                    Writing::Notes
                };
                self.whiteboard(out, o.dir, w, hash(seed, out, 12));
                taken.push(out);
            }
        }

        // Every hall nook (the floor plan's few purposeful dead ends) is dressed: a
        // kitchenette — a counter with a sink and a microwave under a wall cupboard — or,
        // now and then, somebody's desk.
        let mut kitchenettes = 0;
        for y in 0..maze.h as i32 {
            for x in 0..maze.w as i32 {
                let c = Cell { x, y };
                if kitchenettes >= 3
                    || c == maze.start
                    || c == maze.exit
                    || room_of(c).is_some()
                    || doored(c)
                    || taken.contains(&c)
                    || maze.degree(c) != 1
                {
                    continue;
                }
                let Some(open) = (0..4).find(|&d| maze.is_open(c, d)) else {
                    continue;
                };
                let back = opposite(open);
                if maze.edge(c, back) != EdgeKind::Wall {
                    continue;
                }
                kitchenettes += 1;
                taken.push(c);
                if hash(seed, c, 13) < 0.3 {
                    // Somebody's desk, moved as far from everyone as it would go, with a
                    // red stapler (Office Space).
                    self.fit(
                        c,
                        back,
                        0.0,
                        &[
                            p(0.35, 0.0, 0.2, (0.34, 0.37), WOOD),
                            pa(-0.33, 0.02, 0.0, 0.2, (0.0, 0.34), DARK_WOOD, Finish::Plain),
                            pa(0.33, 0.02, 0.0, 0.2, (0.0, 0.34), DARK_WOOD, Finish::Plain),
                            pa(0.12, 0.045, 0.06, 0.03, (0.37, 0.395), RED, Finish::Plain),
                            pa(-0.2, 0.012, 0.04, 0.02, (0.37, 0.5), DARK, Finish::Plain),
                            pa(
                                -0.2,
                                0.04,
                                0.03,
                                0.07,
                                (0.5, 0.53),
                                [1.0, 0.85, 0.5],
                                Finish::Lamp,
                            ),
                        ],
                    );
                    continue;
                }
                self.fit(
                    c,
                    back,
                    0.0,
                    &[
                        pf(0.45, 0.0, 0.2, (0.0, 0.3), CUPBOARD, Finish::Bands(0.15)),
                        p(0.46, 0.0, 0.21, (0.3, 0.315), DARK),
                        // Sink basin and tap.
                        pa(-0.2, 0.09, 0.05, 0.12, (0.315, 0.32), STEEL, Finish::Plain),
                        pa(-0.2, 0.012, 0.02, 0.03, (0.315, 0.39), STEEL, Finish::Plain),
                        pa(-0.2, 0.012, 0.02, 0.07, (0.39, 0.4), STEEL, Finish::Plain),
                        // Microwave.
                        pa(
                            0.2,
                            0.1,
                            0.02,
                            0.13,
                            (0.315, 0.41),
                            DARK,
                            Finish::Screen(open, [0.25, 0.3, 0.3]),
                        ),
                        // Wall cupboard.
                        pf(0.45, 0.0, 0.14, (0.55, 0.78), CUPBOARD, Finish::Bands(0.23)),
                    ],
                );
            }
        }

        for y in 0..maze.h as i32 {
            for x in 0..maze.w as i32 {
                let c = Cell { x, y };
                // Core (walled-in, unreachable) cells: nobody would ever see it.
                if c == maze.start
                    || c == maze.exit
                    || doored(c)
                    || taken.contains(&c)
                    || maze.is_core(c)
                {
                    continue;
                }
                let walls: Vec<usize> = (0..4)
                    .filter(|&d| maze.edge(c, d) == EdgeKind::Wall)
                    .collect();
                if walls.is_empty() {
                    continue;
                }
                let open_space = match room_of(c) {
                    None => false,
                    Some(r) if vacant(r) => continue,
                    Some(r) if r.kind == RoomKind::OpenSpace => true,
                    // Dressed below.
                    Some(_) => continue,
                };
                let chance = if open_space { 0.45 } else { 0.3 };
                if hash(seed, c, 1) >= chance {
                    continue;
                }
                let dir = pick(&walls, hash(seed, c, 2));
                if open_space {
                    // The wall desks (`open_space_desks`) keep off this wall.
                    fitted.push((c, dir));
                    if hash(seed, c, 5) < 0.4 {
                        // Open-space whiteboards — and now and then one's been taken
                        // over by somebody's mail-room investigation.
                        let w = match hash(seed, c, 6) {
                            h if h < 0.25 => Writing::Pepe,
                            h if h < 0.28 => Writing::Cake,
                            _ => Writing::Notes,
                        };
                        self.whiteboard(c, dir, w, hash(seed, c, 7));
                        continue;
                    }
                }
                let set = if open_space {
                    OFFICE_ROOM_WALL
                } else {
                    OFFICE_HALL
                };
                let along = (hash(seed, c, 3) - 0.5) * 0.3;
                let parts = pick(set, hash(seed, c, 4));
                self.fit(c, dir, along, parts);
                if std::ptr::eq(parts, CORKBOARD) && hash(seed, c, 8) < 0.5 {
                    // A TPS report cover sheet pinned up (Office Space).
                    let out = dir_vec(opposite(dir));
                    // `fit` offsets `along` on the world axis the wall runs along (+x
                    // or +y), not a turned-with-the-wall one: follow it the same way.
                    let axis = if dir.is_multiple_of(2) {
                        Vec2::X
                    } else {
                        Vec2::Y
                    };
                    self.posters.push(Poster {
                        at: square(c)
                            + vec2(0.5, 0.5)
                            + dir_vec(dir) * self.face()
                            + out * 0.03
                            + axis * along,
                        out,
                        lines: TPS_REPORTS,
                        icon: None,
                        z: 0.6,
                        width: 0.2,
                        ink: INK,
                        cap: 0.028,
                        paper: PAPER,
                        glow: false,
                    });
                }
            }
        }

        let mut names: Vec<&'static str> = MEETING_ROOMS.to_vec();
        let mut open_spaces = 0;
        let mut wcs = Vec::new();
        for r in &maze.rooms {
            let Some((di, door)) = maze
                .doors
                .iter()
                .enumerate()
                .find(|(_, o)| r.contains(o.cell))
            else {
                continue;
            };
            let toward = door.dir;
            let rc = Cell { x: r.x, y: r.y };
            let (b0, p0) = (self.blocks.len(), self.posters.len());
            match r.kind {
                // Vacant: a plain door, nothing inside (`Maze::door_marked`).
                _ if vacant(r) => {}
                RoomKind::OpenSpace => {
                    // A couple of the open-plan floors have names on the door
                    // (Severance, The Stanley Parable).
                    open_spaces += 1;
                    match open_spaces {
                        1 => self.sign(di, "MACRODATA REFINEMENT"),
                        2 => self.sign(di, "427"),
                        _ => {}
                    }
                    let pillars: Vec<Vec2> = (r.y..r.y + r.h - 1)
                        .flat_map(|y| (r.x..r.x + r.w - 1).map(move |x| Cell { x, y }))
                        .map(|c| square(c) + vec2(1.0, 1.0))
                        .collect();
                    for (i, &sq) in pillars.iter().enumerate() {
                        let t = if i % 2 == 0 { toward } else { opposite(toward) };
                        let h = hash(seed, rc, 20 + i as u64);
                        match hash(seed, rc, 40 + i as u64) {
                            k if k < 0.25 => self.desk((sq, Vec2::ONE), t, h),
                            k => self.desk_pod(sq, t, h, k > 0.7),
                        }
                    }
                }
                RoomKind::Boss => {
                    self.sign(di, "BOSS ROOM");
                    self.boss(floor(r), toward, maze.lady_boss);
                }
                RoomKind::Wc => {
                    let icon = if maze.wc_door(di) == Some(true) {
                        Icon::Man
                    } else {
                        Icon::Woman
                    };
                    wcs.push(di);
                    self.signs.push((
                        di,
                        Spec {
                            icon: Some(icon),
                            ..Spec::new(&["WC"], BRASS, Some(PLATE))
                        },
                    ));
                    self.wc(floor(r), toward);
                }
                RoomKind::It => {
                    self.sign(di, "IT");
                    self.it(floor(r), toward);
                }
                RoomKind::Janitor => {
                    self.sign(di, "JANITOR");
                    self.janitor(floor(r), toward);
                }
                RoomKind::Kitchen => {
                    self.sign(di, "KITCHEN");
                    self.kitchen(floor(r), toward);
                }
                RoomKind::Conference => {
                    let k =
                        ((hash(seed, rc, 30) * names.len() as f32) as usize).min(names.len() - 1);
                    let name = names.swap_remove(k);
                    if names.is_empty() {
                        names = MEETING_ROOMS.to_vec();
                    }
                    self.sign(di, name);
                    let fl = Floor::new(floor(r), self.recess(), toward);
                    match r.meeting_layout() {
                        Some(MeetingLayout::Chart) => self.meeting_crowded(&fl),
                        Some(MeetingLayout::Zen) => self.meeting_zen(&fl),
                        Some(MeetingLayout::Throne) => self.boardroom_throne(&fl),
                        Some(MeetingLayout::Culture) | None => self.boardroom_culture(&fl),
                    }
                }
            }
            // Laid out on the real floor already (`Floor`, `shifted`).
            if !matches!(
                r.kind,
                RoomKind::Conference | RoomKind::Janitor | RoomKind::Kitchen
            ) {
                self.settle_room(r, toward, b0, p0);
            }
            if r.kind == RoomKind::OpenSpace && !vacant(r) {
                self.open_space_desks(maze, r, &fitted, seed);
            }
        }
        // The out-of-order washroom (`fit_rooms` seals its door): a sheet taped to it.
        self.out_of_order = wcs.iter().copied().find(|&di| maze.doors[di].sealed);
        self.out_of_order_seen = self.out_of_order.map(|di| {
            let door = &maze.doors[di];
            (door.cell.step(door.dir), opposite(door.dir))
        });
    }

    /// A name plate on door `di`.
    fn sign(&mut self, di: usize, name: &'static str) {
        self.signs
            .push((di, Spec::wrapped(name, SIGN_CHARS, BRASS, Some(PLATE))));
    }

    /// An open-space desk pod: desk with a drawer unit, monitor facing the chair,
    /// keyboard, mouse, a mug and papers, and a swivel chair.
    /// Two desks back to back across a floor square `sq` (1×1, grid coords), facing
    /// along `t` either way, split by a fabric divider — and, as a `cubicle`, walled in
    /// on both ends too. Chairs reach just past the square, well clear of the walked
    /// lines through the cells.
    fn desk_pod(&mut self, sq: Vec2, t: usize, h: f32, cubicle: bool) {
        let n = dir_vec(t);
        // `desk` puts its back 0.35 into the unit square from the side away from its
        // chair: shift each square so the backs stand either side of the divider.
        self.desk((sq + n * 0.165, Vec2::ONE), t, h);
        self.desk((sq - n * 0.165, Vec2::ONE), opposite(t), (h + 0.5).fract());
        self.group += 1;
        const FABRIC: [f32; 3] = [0.42, 0.47, 0.55];
        const HIGH: f32 = 0.6;
        let c = sq + Vec2::splat(0.5);
        let side = vec2(n.y.abs(), n.x.abs());
        let along = vec2(n.x.abs(), n.y.abs());
        // The divider between the two desks.
        let half = side * 0.46 + along * 0.015;
        self.push(c - half, c + half, (0.0, HIGH), FABRIC, Finish::Plain);
        if cubicle {
            for s in [-1.0, 1.0] {
                let mid = c + side * (0.475 * s);
                let half = side * 0.015 + along * 0.4;
                self.push(mid - half, mid + half, (0.0, HIGH), FABRIC, Finish::Plain);
            }
        }
    }

    fn desk(&mut self, f: (Vec2, Vec2), t: usize, h: f32) {
        self.group += 1;
        let top = [0.8, 0.79, 0.74];
        self.place(
            f,
            t,
            (0.35, 0.68),
            (0.08, 0.92),
            (0.34, 0.37),
            top,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.36, 0.67),
            (0.08, 0.11),
            (0.0, 0.34),
            GREY,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.36, 0.64),
            (0.7, 0.89),
            (0.0, 0.34),
            GREY,
            Finish::Bands(0.11),
        );
        // Monitor on its stand, screen toward the chair.
        self.place(
            f,
            t,
            (0.42, 0.46),
            (0.46, 0.54),
            (0.37, 0.375),
            DARK,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.43, 0.45),
            (0.49, 0.51),
            (0.375, 0.41),
            DARK,
            Finish::Plain,
        );
        let screen = if h < 0.2 {
            Finish::Chart(t)
        } else {
            Finish::Screen(t, SCREEN)
        };
        self.place(f, t, (0.4, 0.43), (0.3, 0.7), (0.41, 0.56), DARK, screen);
        self.place(
            f,
            t,
            (0.52, 0.58),
            (0.36, 0.6),
            (0.37, 0.378),
            DARK,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.54, 0.57),
            (0.64, 0.67),
            (0.37, 0.378),
            DARK,
            Finish::Plain,
        );
        if (0.2..0.35).contains(&h) {
            // The red Swingline (Office Space).
            self.place(
                f,
                t,
                (0.5, 0.53),
                (0.2, 0.3),
                (0.37, 0.39),
                RED,
                Finish::Plain,
            );
        } else if (0.35..0.45).contains(&h) {
            // A Dundie on the desk (The Office).
            self.place(
                f,
                t,
                (0.43, 0.47),
                (0.78, 0.83),
                (0.37, 0.38),
                DARK,
                Finish::Plain,
            );
            self.place(
                f,
                t,
                (0.44, 0.46),
                (0.795, 0.815),
                (0.38, 0.45),
                GOLD,
                Finish::Plain,
            );
        }
        let mug = pick(&[[0.9, 0.9, 0.9], [0.8, 0.2, 0.2], [0.2, 0.4, 0.8]], h);
        self.place(
            f,
            t,
            (0.44, 0.48),
            (0.12, 0.16),
            (0.37, 0.41),
            mug,
            Finish::Plain,
        );
        self.shaped(Shape::Cyl);
        self.place(
            f,
            t,
            (0.5, 0.62),
            (0.72, 0.86),
            (0.37, 0.373),
            WHITE,
            Finish::Plain,
        );
        // The swivel chair, its back to the room.
        self.swivel_chair(room_point(f, t, 0.83, 0.5), dir_vec(t), DARK);
    }

    /// A block on room floor `f` in `place`'s frame, made `shape` — a round one keeps
    /// the ellipse its box gives it (`shaped` would make it a circle).
    #[allow(clippy::too_many_arguments)]
    fn put(
        &mut self,
        f: (Vec2, Vec2),
        t: usize,
        a: (f32, f32),
        b: (f32, f32),
        z: (f32, f32),
        rgb: [f32; 3],
        finish: Finish,
        shape: Shape,
    ) {
        self.place(f, t, a, b, z, rgb, finish);
        self.blocks.last_mut().expect("just placed").shape = shape;
    }

    /// A round block centred at `(a, b)` in `place`'s frame, half-extents `(ra, rb)`.
    #[allow(clippy::too_many_arguments)]
    fn round(
        &mut self,
        f: (Vec2, Vec2),
        t: usize,
        (a, b): (f32, f32),
        (ra, rb): (f32, f32),
        z: (f32, f32),
        rgb: [f32; 3],
        finish: Finish,
        shape: Shape,
    ) {
        self.put(
            f,
            t,
            (a - ra, a + ra),
            (b - rb, b + rb),
            z,
            rgb,
            finish,
            shape,
        );
    }

    /// The last pushed block moves: re-placed every frame by `motion`.
    fn animate(&mut self, motion: Motion) {
        let i = self.blocks.len() - 1;
        self.moves.push((i as u32, self.blocks[i], motion));
    }

    /// The boss's office (one cell, peeked at from the door): a bookcase, a window with
    /// its blinds half down and a trophy cabinet under a motivational poster on the
    /// back wall, a rug, a potted plant and an antique globe, the crowned throne with
    /// the boss in it — bald, jowly, scowling, cigar in his mouth, fists on a vast desk
    /// (nameplate THE BOSS, phone with a line on hold, mug, papers, monitor turned his
    /// way). His health bar is the HUD's (`view/boss.rs`). He shakes his head and pounds the desk
    /// while he argues (`Motion::Head`, `Motion::Fist`) and hangs his head once beaten.
    /// `lady`: the boss is a woman this floor (`Maze::lady_boss`, `boss_lady`) —
    /// same pose, same office, same nameplate.
    fn boss(&mut self, f: (Vec2, Vec2), t: usize, lady: bool) {
        const RUG: [f32; 3] = [0.36, 0.07, 0.09];
        const RUG_EDGE: [f32; 3] = [0.72, 0.56, 0.26];
        const VELVET: [f32; 3] = [0.5, 0.06, 0.1];
        const BOOKS: [[f32; 3]; 6] = [
            [0.6, 0.15, 0.1],
            [0.15, 0.3, 0.55],
            [0.8, 0.7, 0.3],
            [0.2, 0.45, 0.25],
            [0.45, 0.3, 0.2],
            [0.7, 0.65, 0.6],
        ];
        let (plain, bx) = (Finish::Plain, Shape::Box);
        let across = if t.is_multiple_of(2) {
            Vec2::X
        } else {
            Vec2::Y
        };

        // The rug, with a gold border.
        self.group += 1;
        self.put(f, t, (0.16, 0.84), (0.2, 0.8), (0.0, 0.004), RUG, plain, bx);
        for (a, b) in [
            ((0.12, 0.16), (0.16, 0.84)),
            ((0.84, 0.88), (0.16, 0.84)),
            ((0.16, 0.84), (0.16, 0.2)),
            ((0.16, 0.84), (0.8, 0.84)),
        ] {
            self.put(f, t, a, b, (0.0, 0.004), RUG_EDGE, plain, bx);
        }

        // Bookcase, left of the window: rows of books standing proud of the shelves.
        self.group += 1;
        self.put(
            f,
            t,
            (0.0, 0.12),
            (0.12, 0.34),
            (0.0, 0.82),
            DARK_WOOD,
            Finish::Bands(0.2),
            bx,
        );
        for (row, z) in [0.205, 0.405, 0.605].into_iter().enumerate() {
            let mut b = 0.135;
            let mut k = row * 2;
            while b < 0.31 {
                let w = 0.022 + 0.008 * ((k * 7) % 3) as f32;
                let hgt = 0.12 + 0.02 * ((k * 5) % 3) as f32;
                self.put(
                    f,
                    t,
                    (0.03, 0.125),
                    (b, (b + w).min(0.325)),
                    (z, z + hgt),
                    BOOKS[k % BOOKS.len()],
                    plain,
                    bx,
                );
                b += w + 0.003;
                k += 1;
            }
        }

        // The window: frame, sill, daylight through the blinds.
        self.group += 1;
        self.put(
            f,
            t,
            (0.0, 0.015),
            (0.36, 0.64),
            (0.3, 0.92),
            WHITE,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.0, 0.02),
            (0.375, 0.625),
            (0.315, 0.905),
            [0.82, 0.8, 0.72],
            Finish::Blinds(0.028),
            bx,
        );
        self.put(
            f,
            t,
            (0.0, 0.035),
            (0.35, 0.65),
            (0.29, 0.305),
            WHITE,
            plain,
            bx,
        );

        // Trophy cabinet, right of the window; two trophies on it.
        self.group += 1;
        self.put(
            f,
            t,
            (0.0, 0.12),
            (0.66, 0.88),
            (0.0, 0.42),
            DARK_WOOD,
            Finish::Bands(0.17),
            bx,
        );
        self.put(
            f,
            t,
            (0.04, 0.08),
            (0.71, 0.75),
            (0.42, 0.45),
            DARK,
            plain,
            bx,
        );
        let cup = Finish::Plain;
        self.round(
            f,
            t,
            (0.06, 0.73),
            (0.008, 0.008),
            (0.45, 0.48),
            GOLD,
            cup,
            Shape::Cyl,
        );
        self.round(
            f,
            t,
            (0.06, 0.73),
            (0.026, 0.026),
            (0.48, 0.54),
            GOLD,
            cup,
            Shape::Bowl,
        );
        for b in [0.7, 0.76] {
            self.round(
                f,
                t,
                (0.06, b),
                (0.007, 0.007),
                (0.505, 0.53),
                GOLD,
                cup,
                Shape::Ball,
            );
        }
        self.put(
            f,
            t,
            (0.04, 0.08),
            (0.8, 0.84),
            (0.42, 0.44),
            DARK,
            plain,
            bx,
        );
        self.round(
            f,
            t,
            (0.06, 0.82),
            (0.01, 0.01),
            (0.44, 0.5),
            GOLD,
            cup,
            Shape::Cyl,
        );
        self.round(
            f,
            t,
            (0.06, 0.82),
            (0.016, 0.016),
            (0.5, 0.53),
            GOLD,
            cup,
            Shape::Ball,
        );
        // Over it, a motivational poster.
        let out = dir_vec(t);
        self.posters.push(Poster {
            at: room_point(f, t, 0.0, 0.77) + out * 0.004,
            out,
            lines: &BOSS_POSTER,
            icon: None,
            z: 0.72,
            width: 0.2,
            cap: 0.036,
            ink: Color::new(0.95, 0.9, 0.75, 1.0),
            paper: Color::new(0.06, 0.06, 0.07, 1.0),
            glow: false,
        });

        // A potted plant in one front corner, a globe on its stand in the other.
        self.group += 1;
        self.round(
            f,
            t,
            (0.27, 0.2),
            (0.05, 0.05),
            (0.0, 0.2),
            POT,
            plain,
            Shape::Cyl,
        );
        self.round(
            f,
            t,
            (0.27, 0.2),
            (0.055, 0.055),
            (0.2, 0.215),
            POT,
            plain,
            Shape::Cyl,
        );
        for (a, b, r, z, rgb) in [
            (0.27, 0.2, 0.058, (0.21, 0.37), LEAF),
            (0.25, 0.17, 0.045, (0.32, 0.47), LEAF_DARK),
            (0.29, 0.23, 0.045, (0.3, 0.45), LEAF_LIGHT),
            (0.265, 0.21, 0.045, (0.42, 0.56), LEAF),
            (0.28, 0.18, 0.035, (0.52, 0.63), LEAF_LIGHT),
        ] {
            self.round(f, t, (a, b), (r, r), z, rgb, Finish::Leafy, Shape::Ball);
        }
        self.group += 1;
        let g = (0.29, 0.8);
        self.round(
            f,
            t,
            g,
            (0.05, 0.05),
            (0.0, 0.015),
            DARK_WOOD,
            plain,
            Shape::Cyl,
        );
        self.round(
            f,
            t,
            g,
            (0.012, 0.012),
            (0.015, 0.355),
            DARK_WOOD,
            plain,
            Shape::Cyl,
        );
        self.round(
            f,
            t,
            g,
            (0.065, 0.065),
            (0.35, 0.48),
            [0.72, 0.62, 0.4],
            Finish::Leafy,
            Shape::Ball,
        );
        self.round(
            f,
            t,
            g,
            (0.072, 0.072),
            (0.412, 0.418),
            GOLD,
            plain,
            Shape::Cyl,
        );

        // The throne: red velvet in a gilt frame, a crown on top.
        self.group += 1;
        self.put(
            f,
            t,
            (0.15, 0.2),
            (0.36, 0.64),
            (0.0, 0.74),
            VELVET,
            plain,
            bx,
        );
        for b in [0.365, 0.635] {
            self.round(
                f,
                t,
                (0.175, b),
                (0.02, 0.01),
                (0.0, 0.78),
                GOLD,
                plain,
                Shape::Cyl,
            );
            self.round(
                f,
                t,
                (0.175, b),
                (0.016, 0.016),
                (0.78, 0.81),
                GOLD,
                plain,
                Shape::Ball,
            );
        }
        self.put(
            f,
            t,
            (0.2, 0.38),
            (0.38, 0.62),
            (0.0, 0.18),
            DARK_WOOD,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.2, 0.4),
            (0.37, 0.63),
            (0.18, 0.26),
            VELVET,
            plain,
            bx,
        );
        for b in [(0.34, 0.38), (0.62, 0.66)] {
            self.put(f, t, (0.2, 0.38), b, (0.0, 0.34), VELVET, plain, bx);
            let mid = (b.0 + b.1) * 0.5;
            self.round(
                f,
                t,
                (0.38, mid),
                (0.02, 0.02),
                (0.32, 0.36),
                GOLD,
                plain,
                Shape::Ball,
            );
        }
        self.put(
            f,
            t,
            (0.16, 0.19),
            (0.44, 0.56),
            (0.74, 0.77),
            GOLD,
            plain,
            bx,
        );
        for (b, top) in [
            ((0.44, 0.47), 0.8),
            ((0.485, 0.515), 0.82),
            ((0.53, 0.56), 0.8),
        ] {
            self.put(f, t, (0.16, 0.19), b, (0.77, top), GOLD, plain, bx);
        }
        self.round(
            f,
            t,
            (0.194, 0.5),
            (0.007, 0.01),
            (0.748, 0.766),
            RED,
            plain,
            Shape::Ball,
        );

        // The desk: top, modesty panel, pedestals.
        self.group += 1;
        self.put(
            f,
            t,
            (0.45, 0.74),
            (0.14, 0.86),
            (0.36, 0.41),
            DARK_WOOD,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.7, 0.74),
            (0.16, 0.84),
            (0.0, 0.36),
            DARK_WOOD,
            Finish::Framed,
            bx,
        );
        self.put(
            f,
            t,
            (0.47, 0.7),
            (0.16, 0.28),
            (0.0, 0.36),
            DARK_WOOD,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.47, 0.7),
            (0.72, 0.84),
            (0.0, 0.36),
            DARK_WOOD,
            plain,
            bx,
        );
        // On it: the monitor turned his way, the nameplate, phone, mug, papers.
        let screen = Finish::Screen(opposite(t), SCREEN);
        self.put(
            f,
            t,
            (0.49, 0.54),
            (0.22, 0.28),
            (0.41, 0.415),
            DARK,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.505, 0.525),
            (0.245, 0.255),
            (0.415, 0.44),
            DARK,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.5, 0.53),
            (0.16, 0.34),
            (0.44, 0.56),
            DARK,
            screen,
            bx,
        );
        self.put(
            f,
            t,
            (0.64, 0.7),
            (0.42, 0.58),
            (0.41, 0.422),
            DARK_WOOD,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.66, 0.69),
            (0.43, 0.57),
            (0.422, 0.462),
            GOLD,
            plain,
            bx,
        );
        self.posters.push(Poster {
            at: room_point(f, t, 0.692, 0.5),
            out,
            lines: &["THE BOSS"],
            icon: None,
            z: 0.442,
            width: 0.13,
            cap: 0.022,
            ink: Color::new(0.12, 0.08, 0.02, 1.0),
            paper: Color::new(0.92, 0.74, 0.28, 1.0),
            glow: false,
        });
        self.put(
            f,
            t,
            (0.5, 0.58),
            (0.66, 0.74),
            (0.41, 0.43),
            DARK,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.5, 0.58),
            (0.71, 0.735),
            (0.43, 0.445),
            DARK,
            plain,
            bx,
        );
        let hold = [1.0, 0.25, 0.15];
        self.put(
            f,
            t,
            (0.56, 0.57),
            (0.68, 0.69),
            (0.43, 0.434),
            hold,
            Finish::Blink,
            bx,
        );
        if lady {
            self.boss_lady(f, t, across);
        } else {
            // (World's best, the mug says, if you could read it.)
            self.round(
                f,
                t,
                (0.6, 0.78),
                (0.02, 0.02),
                (0.41, 0.46),
                WHITE,
                plain,
                Shape::Cyl,
            );
            self.put(
                f,
                t,
                (0.595, 0.605),
                (0.75, 0.76),
                (0.42, 0.45),
                WHITE,
                plain,
                bx,
            );
            self.put(
                f,
                t,
                (0.6, 0.68),
                (0.28, 0.38),
                (0.41, 0.418),
                PAPER_RGB,
                plain,
                bx,
            );
            self.boss_man(f, t, across);
        }
    }

    /// The lady boss (`Maze::lady_boss`), in his pose — seated in the throne, forearms
    /// on the desk, one fist pounding it (`Motion::Fist`), head shaking while she argues
    /// and hanging once beaten (`Motion::Head`): a royal-blue power suit with squared
    /// shoulder pads and gold buttons, an ivory silk blouse, a string of pearls, big
    /// honey-blonde hair, pearl drop earrings to match, arched stern brows and red
    /// lipstick. No cigar: a "#1 BOSS" mug and a green smoothie.
    fn boss_lady(&mut self, f: (Vec2, Vec2), t: usize, across: Vec2) {
        const SUIT: [f32; 3] = [0.13, 0.24, 0.66];
        const LAPEL: [f32; 3] = [0.08, 0.15, 0.44];
        const BLOUSE: [f32; 3] = [0.97, 0.94, 0.86];
        const SKIN: [f32; 3] = [0.92, 0.72, 0.6];
        const SKIN_DARK: [f32; 3] = [0.8, 0.56, 0.46];
        const HAIR: [f32; 3] = [0.84, 0.66, 0.36];
        const HAIR_DARK: [f32; 3] = [0.72, 0.54, 0.28];
        const BROW: [f32; 3] = [0.3, 0.19, 0.1];
        const LIPSTICK: [f32; 3] = [0.8, 0.07, 0.15];
        const PEARL: [f32; 3] = [0.97, 0.95, 0.9];
        const SMOOTHIE: [f32; 3] = [0.46, 0.74, 0.2];
        const CUP: [f32; 3] = [0.86, 0.95, 0.88];
        const STRAW: [f32; 3] = [0.95, 0.45, 0.62];
        let (plain, bx) = (Finish::Plain, Shape::Box);

        // On the desk: the mug that says so, and a green smoothie with a straw.
        self.group += 1;
        self.round(
            f,
            t,
            (0.6, 0.8),
            (0.028, 0.028),
            (0.41, 0.475),
            RED,
            plain,
            Shape::Cyl,
        );
        self.put(
            f,
            t,
            (0.593, 0.607),
            (0.828, 0.842),
            (0.425, 0.46),
            RED,
            plain,
            bx,
        );
        self.posters.push(Poster {
            at: room_point(f, t, 0.629, 0.8),
            out: dir_vec(t),
            lines: &[BOSS_MUG],
            icon: None,
            z: 0.443,
            width: 0.046,
            cap: 0.011,
            ink: Color::new(1.0, 0.96, 0.9, 1.0),
            paper: Color::new(0.85, 0.08, 0.08, 1.0),
            glow: false,
        });
        self.round(
            f,
            t,
            (0.67, 0.64),
            (0.016, 0.016),
            (0.41, 0.468),
            SMOOTHIE,
            plain,
            Shape::Cyl,
        );
        self.put(
            f,
            t,
            (0.668, 0.676),
            (0.644, 0.652),
            (0.44, 0.53),
            STRAW,
            plain,
            bx,
        );
        self.round(
            f,
            t,
            (0.67, 0.64),
            (0.019, 0.019),
            (0.41, 0.48),
            CUP,
            Finish::Glass,
            Shape::Cyl,
        );

        // Seated: skirt over her knees, the blazer — squared shoulder pads, dark
        // lapels over the blouse, two gold buttons — pearls at her throat, forearms on
        // the desk, hands at the end of them.
        self.group += 1;
        for b in [(0.43, 0.49), (0.51, 0.57)] {
            self.put(f, t, (0.24, 0.44), b, (0.26, 0.32), SUIT, plain, bx);
        }
        self.put(
            f,
            t,
            (0.21, 0.325),
            (0.405, 0.595),
            (0.27, 0.49),
            SUIT,
            plain,
            Shape::Cyl,
        );
        self.put(
            f,
            t,
            (0.21, 0.322),
            (0.39, 0.61),
            (0.43, 0.515),
            SUIT,
            plain,
            Shape::Ball,
        );
        for b in [(0.365, 0.435), (0.565, 0.635)] {
            self.put(f, t, (0.22, 0.3), b, (0.48, 0.518), SUIT, plain, bx);
        }
        for b in [(0.375, 0.415), (0.585, 0.625)] {
            self.put(f, t, (0.22, 0.3), b, (0.34, 0.49), SUIT, plain, Shape::Cyl);
        }
        for b in [(0.458, 0.478), (0.522, 0.542)] {
            self.put(f, t, (0.321, 0.333), b, (0.38, 0.48), LAPEL, plain, bx);
        }
        self.put(
            f,
            t,
            (0.324, 0.334),
            (0.478, 0.522),
            (0.4, 0.49),
            BLOUSE,
            plain,
            bx,
        );
        for z in [0.338, 0.368] {
            self.round(
                f,
                t,
                (0.328, 0.5),
                (0.006, 0.006),
                (z, z + 0.012),
                GOLD,
                plain,
                Shape::Ball,
            );
        }
        self.put(
            f,
            t,
            (0.245, 0.295),
            (0.478, 0.522),
            (0.49, 0.545),
            SKIN,
            plain,
            Shape::Cyl,
        );
        self.put(
            f,
            t,
            (0.24, 0.3),
            (0.472, 0.528),
            (0.485, 0.503),
            BLOUSE,
            plain,
            Shape::Cyl,
        );
        // Pearls: a string round the front of her neck, dipping in the middle.
        for k in -3..=3 {
            let th = k as f32 * 0.33;
            let (a, b) = (0.29 + 0.036 * th.cos(), 0.5 + 0.036 * th.sin());
            let z = 0.518 - 0.02 * th.cos();
            self.round(
                f,
                t,
                (a, b),
                (0.007, 0.007),
                (z - 0.007, z + 0.007),
                PEARL,
                plain,
                Shape::Ball,
            );
        }
        for b in [(0.38, 0.425), (0.575, 0.62)] {
            self.put(
                f,
                t,
                (0.27, 0.53),
                b,
                (0.41, 0.45),
                SUIT,
                plain,
                Shape::Ball,
            );
        }
        self.round(
            f,
            t,
            (0.55, 0.4),
            (0.026, 0.026),
            (0.41, 0.44),
            SKIN,
            plain,
            Shape::Ball,
        );
        self.group += 1;
        self.round(
            f,
            t,
            (0.55, 0.6),
            (0.026, 0.028),
            (0.41, 0.445),
            SKIN,
            plain,
            Shape::Ball,
        );
        self.animate(Motion::Fist);

        // Her head: big blonde hair (a bouffant crown, flicked sides framing her
        // face), pearl drops at her ears, brows arched and stern.
        self.group += 1;
        let (hc, hr, hz, hh) = (0.27, 0.047, 0.6, 0.062);
        let front = |bo: f32, zo: f32| {
            hc + hr
                * (1.0 - (bo / hr).powi(2) - (zo / hh).powi(2))
                    .max(0.0)
                    .sqrt()
        };
        let mut face: Vec<FacePart> = vec![
            (
                (0.195, 0.305),
                (0.43, 0.57),
                (0.555, 0.705),
                HAIR,
                plain,
                Shape::Ball,
            ),
            (
                (0.21, 0.3),
                (0.445, 0.555),
                (0.66, 0.73),
                HAIR,
                plain,
                Shape::Ball,
            ),
            (
                (hc - hr, hc + hr),
                (0.5 - hr, 0.5 + hr),
                (hz - hh, hz + hh),
                SKIN,
                plain,
                Shape::Ball,
            ),
            (
                (0.26, 0.315),
                (0.472, 0.528),
                (0.535, 0.572),
                SKIN,
                plain,
                Shape::Ball,
            ),
        ];
        for s in [-1.0f32, 1.0] {
            let b = 0.5 + s * 0.05;
            face.push((
                (0.225, 0.295),
                (b - 0.02, b + 0.02),
                (0.53, 0.625),
                HAIR_DARK,
                plain,
                Shape::Ball,
            ));
            let b = 0.5 + s * 0.047;
            face.push((
                (0.28, 0.302),
                (b - 0.011, b + 0.011),
                (0.53, 0.556),
                PEARL,
                plain,
                Shape::Ball,
            ));
            let (eb, ez) = (0.5 + s * 0.02, 0.607);
            let fa = front(eb - 0.5, ez - hz);
            face.push((
                (fa - 0.008, fa + 0.004),
                (eb - 0.01, eb + 0.01),
                (ez - 0.0065, ez + 0.0065),
                BLOUSE,
                plain,
                Shape::Ball,
            ));
            face.push((
                (fa + 0.001, fa + 0.0055),
                (eb - 0.0042, eb + 0.0042),
                (ez - 0.0045, ez + 0.004),
                DARK,
                plain,
                Shape::Ball,
            ));
            // Liner along the upper lid.
            face.push((
                (fa - 0.006, fa + 0.005),
                (eb - 0.011, eb + 0.011),
                (ez + 0.005, ez + 0.008),
                DARK,
                plain,
                bx,
            ));
            // Brows: a high arch, the inner ends drawn down — not pleased.
            for (bo, half, z) in [
                (0.011, 0.006, 0.618),
                (0.022, 0.0065, 0.627),
                (0.033, 0.005, 0.623),
            ] {
                let b = 0.5 + s * bo;
                let fa = front(bo, z + 0.0025 - hz);
                face.push((
                    (fa - 0.008, fa + 0.004),
                    (b - half, b + half),
                    (z, z + 0.0075),
                    BROW,
                    plain,
                    bx,
                ));
            }
        }
        // Red lips pressed into a firm line — proud of the chin's curve too, which
        // otherwise swallows their middle.
        let fa = front(0.0, 0.562 - hz);
        face.push((
            (fa - 0.004, fa + 0.012),
            (0.484, 0.516),
            (0.557, 0.567),
            LIPSTICK,
            plain,
            bx,
        ));
        let fa = front(0.0, 0.592 - hz);
        face.push((
            (fa - 0.008, fa + 0.01),
            (0.493, 0.507),
            (0.582, 0.602),
            SKIN_DARK,
            plain,
            Shape::Ball,
        ));
        for (a, b, z, rgb, finish, shape) in face {
            self.put(f, t, a, b, z, rgb, finish, shape);
            self.animate(Motion::Head { across });
        }
    }

    /// The boss himself, in the throne behind the desk: seated, suit stretched over his
    /// chest, shirt and tie, round shoulders, forearms on the desk with a fist that
    /// pounds it (`Motion::Fist`), bald jowly head with a grey fringe, a scowl and a lit
    /// cigar — all of the head shaking with it (`Motion::Head`). His mug, plain.
    fn boss_man(&mut self, f: (Vec2, Vec2), t: usize, across: Vec2) {
        const SUIT: [f32; 3] = [0.18, 0.2, 0.28];
        const LAPEL: [f32; 3] = [0.11, 0.12, 0.18];
        const SHIRT: [f32; 3] = [0.95, 0.95, 0.97];
        const SKIN: [f32; 3] = [0.9, 0.68, 0.56];
        const SKIN_DARK: [f32; 3] = [0.78, 0.52, 0.42];
        const HAIR: [f32; 3] = [0.36, 0.34, 0.33];
        const BROW: [f32; 3] = [0.22, 0.18, 0.16];
        const LIPS: [f32; 3] = [0.42, 0.14, 0.13];
        let (plain, bx) = (Finish::Plain, Shape::Box);
        // The man himself. Seated: thighs, a suit stretched over his chest, shirt and
        // tie, round shoulders; forearms on the desk, hands at the end of them.
        self.group += 1;
        for b in [(0.42, 0.49), (0.51, 0.58)] {
            self.put(f, t, (0.24, 0.44), b, (0.26, 0.32), SUIT, plain, bx);
        }
        self.put(
            f,
            t,
            (0.2, 0.33),
            (0.39, 0.61),
            (0.27, 0.5),
            SUIT,
            plain,
            Shape::Cyl,
        );
        self.put(
            f,
            t,
            (0.205, 0.325),
            (0.375, 0.625),
            (0.44, 0.53),
            SUIT,
            plain,
            Shape::Ball,
        );
        for b in [(0.455, 0.475), (0.525, 0.545)] {
            self.put(f, t, (0.322, 0.334), b, (0.36, 0.49), LAPEL, plain, bx);
        }
        self.put(
            f,
            t,
            (0.326, 0.336),
            (0.475, 0.525),
            (0.36, 0.5),
            SHIRT,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.336, 0.342),
            (0.492, 0.508),
            (0.34, 0.47),
            RED,
            plain,
            bx,
        );
        let knot = [0.6, 0.05, 0.05];
        self.put(
            f,
            t,
            (0.336, 0.345),
            (0.488, 0.512),
            (0.47, 0.495),
            knot,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.24, 0.3),
            (0.475, 0.525),
            (0.5, 0.545),
            SKIN,
            plain,
            Shape::Cyl,
        );
        self.put(
            f,
            t,
            (0.235, 0.305),
            (0.47, 0.53),
            (0.49, 0.515),
            SHIRT,
            plain,
            Shape::Cyl,
        );
        for b in [(0.36, 0.405), (0.595, 0.64)] {
            self.put(f, t, (0.22, 0.3), b, (0.34, 0.5), SUIT, plain, Shape::Cyl);
        }
        for b in [(0.37, 0.42), (0.58, 0.63)] {
            self.put(
                f,
                t,
                (0.27, 0.53),
                b,
                (0.41, 0.455),
                SUIT,
                plain,
                Shape::Ball,
            );
        }
        self.round(
            f,
            t,
            (0.55, 0.4),
            (0.03, 0.03),
            (0.41, 0.445),
            SKIN,
            plain,
            Shape::Ball,
        );
        self.group += 1;
        self.round(
            f,
            t,
            (0.55, 0.6),
            (0.03, 0.032),
            (0.41, 0.45),
            SKIN,
            plain,
            Shape::Ball,
        );
        self.animate(Motion::Fist);

        // His head: bald on top, a grey fringe round the back, jowls; small eyes
        // under brows knitted down in the middle, a frown, a cigar.
        self.group += 1;
        let (hc, hr, hz, hh) = (0.27, 0.055, 0.595, 0.07);
        // Where the head's front surface is, `bo` across and `zo` up from its centre.
        let front = |bo: f32, zo: f32| {
            hc + hr
                * (1.0 - (bo / hr).powi(2) - (zo / hh).powi(2))
                    .max(0.0)
                    .sqrt()
        };
        let mut face: Vec<FacePart> = vec![
            (
                (0.205, 0.3),
                (0.437, 0.563),
                (0.54, 0.645),
                HAIR,
                plain,
                Shape::Ball,
            ),
            (
                (hc - hr, hc + hr),
                (0.5 - hr, 0.5 + hr),
                (hz - hh, hz + hh),
                SKIN,
                plain,
                Shape::Ball,
            ),
            (
                (0.26, 0.33),
                (0.462, 0.538),
                (0.52, 0.562),
                SKIN,
                plain,
                Shape::Ball,
            ),
        ];
        for s in [-1.0f32, 1.0] {
            let b = 0.5 + s * 0.0555;
            face.push((
                (0.255, 0.285),
                (b - 0.008, b + 0.008),
                (0.575, 0.615),
                SKIN_DARK,
                plain,
                Shape::Ball,
            ));
            let (eb, ez) = (0.5 + s * 0.022, 0.605);
            let fa = front(eb - 0.5, ez - hz);
            face.push((
                (fa - 0.008, fa + 0.004),
                (eb - 0.011, eb + 0.011),
                (ez - 0.007, ez + 0.007),
                SHIRT,
                plain,
                Shape::Ball,
            ));
            face.push((
                (fa + 0.001, fa + 0.0055),
                (eb - 0.0045, eb + 0.0045),
                (ez - 0.005, ez + 0.004),
                DARK,
                plain,
                Shape::Ball,
            ));
            // Brows: the inner half lower — a scowl.
            for (bo, half, z) in [(0.031, 0.0065, 0.622), (0.016, 0.0085, 0.615)] {
                let b = 0.5 + s * bo;
                let fa = front(bo, z + 0.0035 - hz);
                face.push((
                    (fa - 0.008, fa + 0.004),
                    (b - half, b + half),
                    (z, z + 0.007),
                    BROW,
                    plain,
                    Shape::Box,
                ));
            }
            // The corners of his mouth turned down.
            let b = 0.5 + s * 0.018;
            let fa = front(0.018, 0.56 - hz);
            face.push((
                (fa - 0.008, fa + 0.004),
                (b - 0.008, b + 0.008),
                (0.557, 0.563),
                LIPS,
                plain,
                Shape::Box,
            ));
        }
        let fa = front(0.0, 0.565 - hz);
        face.push((
            (fa - 0.008, fa + 0.004),
            (0.49, 0.51),
            (0.563, 0.568),
            LIPS,
            plain,
            Shape::Box,
        ));
        let fa = front(0.0, 0.59 - hz);
        face.push((
            (fa - 0.008, fa + 0.013),
            (0.49, 0.51),
            (0.579, 0.603),
            SKIN_DARK,
            plain,
            Shape::Ball,
        ));
        let fa = front(0.018, 0.561 - hz);
        // The cigar, out of the corner of his mouth and off to the side, lit.
        let cigar = [0.42, 0.26, 0.14];
        let z = (0.556, 0.566);
        face.push((
            (fa - 0.004, fa + 0.022),
            (0.513, 0.525),
            z,
            cigar,
            plain,
            bx,
        ));
        face.push(((fa + 0.01, fa + 0.022), (0.525, 0.585), z, cigar, plain, bx));
        let ember = [1.0, 0.42, 0.1];
        let lamp = Finish::Lamp;
        face.push(((fa + 0.01, fa + 0.022), (0.585, 0.592), z, ember, lamp, bx));
        for (a, b, z, rgb, finish, shape) in face {
            self.put(f, t, a, b, z, rgb, finish, shape);
            self.animate(Motion::Head { across });
        }
    }

    /// The IT department (The IT Crowd), two cells seen down their length from the
    /// door: blinking server racks and "the Internet" (a black box with a red light) along
    /// one side, Roy's and Moss's desks — monitors, keyboards, swivel chairs — along the
    /// other, and the "turned it off and on again" poster on the back wall, square to
    /// the view through the door. Further along, a storage rack of spare laptops and
    /// kit (`it_storage`) and the department turtle's aquarium (`aquarium`). All of it
    /// stays out of the strip the door sweeps.
    fn it(&mut self, f: (Vec2, Vec2), t: usize) {
        // The across axis: `b` runs 0..1 toward this direction.
        let across = if t.is_multiple_of(2) { 1 } else { 2 };
        for a in [(0.03, 0.2), (0.22, 0.39)] {
            self.group += 1;
            self.place(
                f,
                t,
                a,
                (0.02, 0.28),
                (0.0, 0.86),
                DARK,
                Finish::Rack(across),
            );
        }
        // The Internet, on a low stand beside the racks.
        self.group += 1;
        self.place(
            f,
            t,
            (0.44, 0.52),
            (0.05, 0.25),
            (0.0, 0.3),
            GREY,
            Finish::Plain,
        );
        let black = [0.03, 0.03, 0.035];
        self.place(
            f,
            t,
            (0.46, 0.5),
            (0.1, 0.2),
            (0.3, 0.335),
            black,
            Finish::Plain,
        );
        let led = [1.0, 0.08, 0.05];
        self.place(
            f,
            t,
            (0.475, 0.485),
            (0.145, 0.155),
            (0.335, 0.345),
            led,
            Finish::Blink,
        );
        // Labelled, on a little stand on top of it turned to face the door (the room is
        // only ever seen from there), so nobody mistakes it for anything else.
        self.place(
            f,
            t,
            (0.505, 0.515),
            (0.145, 0.155),
            (0.335, 0.42),
            GREY,
            Finish::Plain,
        );
        self.posters.push(Poster {
            at: room_point(f, t, 0.525, 0.15),
            out: dir_vec(t),
            lines: &["THE INTERNET"],
            icon: Some(Icon::Down),
            z: 0.47,
            width: 0.34,
            cap: 0.05,
            ink: INK,
            paper: PAPER,
            glow: false,
        });
        // Two desks against the far side, each with its monitor facing the chair.
        let face = opposite(across);
        for (a0, a1) in [(0.04, 0.3), (0.34, 0.6)] {
            let mid = (a0 + a1) * 0.5;
            self.group += 1;
            self.place(
                f,
                t,
                (a0, a1),
                (0.62, 0.97),
                (0.34, 0.37),
                WOOD,
                Finish::Plain,
            );
            self.place(
                f,
                t,
                (a0 + 0.01, a0 + 0.03),
                (0.64, 0.95),
                (0.0, 0.34),
                DARK_WOOD,
                Finish::Plain,
            );
            self.place(
                f,
                t,
                (a1 - 0.03, a1 - 0.01),
                (0.64, 0.95),
                (0.0, 0.34),
                DARK_WOOD,
                Finish::Plain,
            );
            self.place(
                f,
                t,
                (mid - 0.06, mid + 0.06),
                (0.84, 0.88),
                (0.37, 0.52),
                DARK,
                Finish::Screen(face, SCREEN),
            );
            self.place(
                f,
                t,
                (mid - 0.05, mid + 0.05),
                (0.7, 0.76),
                (0.37, 0.38),
                black,
                Finish::Plain,
            );
            // The swivel chair, pulled up to the desk.
            self.group += 1;
            self.place(
                f,
                t,
                (mid - 0.02, mid + 0.02),
                (0.52, 0.56),
                (0.0, 0.2),
                DARK,
                Finish::Plain,
            );
            self.place(
                f,
                t,
                (mid - 0.06, mid + 0.06),
                (0.48, 0.61),
                (0.2, 0.24),
                DARK,
                Finish::Plain,
            );
            self.place(
                f,
                t,
                (mid - 0.06, mid + 0.06),
                (0.44, 0.48),
                (0.24, 0.46),
                DARK,
                Finish::Plain,
            );
        }
        // Big and bold on the back wall, clear of the racks, so it reads from the door.
        self.posters.push(Poster {
            at: room_point(f, t, 0.0, 0.63) + dir_vec(t) * 0.01,
            out: dir_vec(t),
            lines: &IT_POSTER,
            icon: None,
            z: 0.67,
            width: 0.64,
            cap: 0.06,
            ink: INK,
            paper: Color::new(1.0, 0.92, 0.35, 1.0),
            glow: false,
        });
        self.it_storage(f, t);
        self.aquarium(f, t);
    }

    /// The IT room's storage rack, against the desks' wall toward the door: open steel
    /// shelving with boxes of kit at the bottom, stacks of closed laptops, a row of them
    /// standing on edge charging (a green light each), and a network switch, a spare
    /// keyboard and a coil of cable on top. `a` in world units here (`it`'s floor is
    /// three long, one across); `b` from that side wall. (On the racks' side its posts
    /// stood across THE INTERNET's label in the view from the door.)
    fn it_storage(&mut self, f: (Vec2, Vec2), t: usize) {
        const SHELF: [f32; 3] = [0.62, 0.64, 0.66];
        const CARDBOARD: [f32; 3] = [0.62, 0.47, 0.3];
        const SILVER: [f32; 3] = [0.72, 0.74, 0.77];
        const GRAPHITE: [f32; 3] = [0.2, 0.2, 0.22];
        let len = (f.1 * dir_vec(t)).length();
        let a = |w0: f32, w1: f32| (w0 / len, w1 / len);
        let b = |b0: f32, b1: f32| (1.0 - b1, 1.0 - b0);
        // Its open side faces the room: away from DIRS[across].
        let room = opposite(if t.is_multiple_of(2) { 1 } else { 2 });
        let (plain, bx) = (Finish::Plain, Shape::Box);
        let (a0, a1) = (1.9, 2.44);
        let put = |d: &mut Decor, (a0, a1), (b0, b1), z, rgb, finish| {
            d.put(f, t, a(a0, a1), b(b0, b1), z, rgb, finish, bx);
        };
        self.group += 1;
        for pa in [(a0, a0 + 0.015), (a1 - 0.015, a1)] {
            for pb in [(0.0, 0.015), (0.185, 0.2)] {
                put(self, pa, pb, (0.0, 0.8), SHELF, plain);
            }
        }
        let boards = [0.03, 0.24, 0.45, 0.66];
        for z in boards {
            put(self, (a0, a1), (0.0, 0.2), (z - 0.015, z), SHELF, plain);
        }
        // Boxes of kit.
        let z = boards[0];
        put(
            self,
            (1.92, 2.1),
            (0.03, 0.17),
            (z, z + 0.12),
            CARDBOARD,
            plain,
        );
        put(
            self,
            (2.13, 2.3),
            (0.04, 0.17),
            (z, z + 0.09),
            CARDBOARD,
            plain,
        );
        put(
            self,
            (2.32, 2.41),
            (0.05, 0.16),
            (z, z + 0.14),
            CARDBOARD,
            plain,
        );
        // Closed laptops, stacked, silver and graphite.
        let z = boards[1];
        for (s, n) in [(1.93, 4), (2.1, 3), (2.27, 5)] {
            for k in 0..n {
                let z0 = z + 0.012 * k as f32;
                let rgb = if (k + n) % 2 == 0 { SILVER } else { GRAPHITE };
                let d = 0.004 * (k % 2) as f32;
                put(
                    self,
                    (s + d, s + d + 0.13),
                    (0.04 - d, 0.14 - d),
                    (z0, z0 + 0.011),
                    rgb,
                    plain,
                );
            }
        }
        // Laptops on edge in a charging rack, a green light on each.
        let z = boards[2];
        for k in 0..9 {
            let s = 1.94 + 0.052 * k as f32;
            let rgb = if k % 3 == 1 { SILVER } else { GRAPHITE };
            put(self, (s, s + 0.012), (0.03, 0.16), (z, z + 0.1), rgb, plain);
            put(
                self,
                (s + 0.002, s + 0.01),
                (0.16, 0.163),
                (z + 0.012, z + 0.018),
                [0.3, 1.0, 0.4],
                Finish::Lamp,
            );
        }
        // On top: the switch (LEDs blinking), a spare keyboard, a coil of patch cable.
        let z = boards[3];
        put(
            self,
            (1.94, 2.19),
            (0.03, 0.16),
            (z, z + 0.03),
            DARK,
            Finish::Rack(room),
        );
        put(self, (2.22, 2.36), (0.05, 0.1), (z, z + 0.012), GREY, plain);
        self.round(
            f,
            t,
            (2.34 / len, 0.86),
            (0.035 / len, 0.035),
            (z, z + 0.035),
            [0.15, 0.3, 0.75],
            Finish::Bands(0.009),
            Shape::Cyl,
        );
    }

    /// The IT room's aquarium, along the racks' wall past the Internet, low enough to
    /// keep under its label: a cabinet, a tank of water (see-through — the rest shows
    /// through it) over gravel, weed, a rock, and a little turtle paddling in it. `a` in
    /// world units (`it_storage`).
    fn aquarium(&mut self, f: (Vec2, Vec2), t: usize) {
        const GRAVEL: [f32; 3] = [0.72, 0.64, 0.48];
        const SHELL: [f32; 3] = [0.26, 0.42, 0.18];
        const SKIN: [f32; 3] = [0.5, 0.62, 0.32];
        let len = (f.1 * dir_vec(t)).length();
        let a = |w0: f32, w1: f32| (w0 / len, w1 / len);
        let (plain, bx) = (Finish::Plain, Shape::Box);
        let (a0, a1, b0, b1) = (2.0, 2.45, 0.0, 0.19);
        let (floor, top) = (0.24, 0.42);
        self.group += 1;
        self.put(
            f,
            t,
            a(a0, a1),
            (b0, b1),
            (0.0, floor),
            DARK_WOOD,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            a(a0 + 0.01, a1 - 0.01),
            (b0 + 0.01, b1 - 0.01),
            (floor, floor + 0.025),
            GRAVEL,
            plain,
            bx,
        );
        let g = floor + 0.025;
        let at = |x: f32, b: f32| (x / len, b);
        // At the far end, out of the way of the view in: a rock, and weed in two clumps
        // of blades.
        self.round(
            f,
            t,
            at(a0 + 0.12, 0.07),
            (0.05 / len, 0.04),
            (g, g + 0.05),
            STONE,
            plain,
            Shape::Dome,
        );
        for (x, b, h) in [(a0 + 0.04, 0.05, 0.12), (a0 + 0.07, 0.15, 0.09)] {
            for (dx, db, k, rgb) in [
                (0.0, 0.0, 1.0, LEAF),
                (0.018, 0.012, 0.7, LEAF_LIGHT),
                (-0.014, 0.016, 0.8, LEAF_DARK),
                (0.008, -0.014, 0.55, LEAF_LIGHT),
            ] {
                self.round(
                    f,
                    t,
                    at(x + dx, b + db),
                    (0.005 / len, 0.005),
                    (g, g + h * k),
                    rgb,
                    plain,
                    Shape::Cyl,
                );
            }
        }
        // The turtle, mid-paddle toward the door (and the view): shell, belly, head,
        // four flippers.
        let (x, b, z, k) = (a0 + 0.28, 0.11, g + 0.06, 1.3);
        let parts = [
            (
                (0.0, 0.0),
                (0.035, 0.03),
                (0.0, 0.022),
                SHELL,
                Finish::Leafy,
                Shape::Dome,
            ),
            ((0.0, 0.0), (0.033, 0.028), (-0.006, 0.0), SKIN, plain, bx),
            (
                (0.045, 0.0),
                (0.013, 0.011),
                (-0.004, 0.016),
                SKIN,
                plain,
                Shape::Ball,
            ),
            (
                (0.022, -0.032),
                (0.012, 0.008),
                (-0.006, 0.002),
                SKIN,
                plain,
                Shape::Ball,
            ),
            (
                (0.022, 0.032),
                (0.012, 0.008),
                (-0.006, 0.002),
                SKIN,
                plain,
                Shape::Ball,
            ),
            (
                (-0.024, -0.026),
                (0.012, 0.008),
                (-0.006, 0.002),
                SKIN,
                plain,
                Shape::Ball,
            ),
            (
                (-0.024, 0.026),
                (0.012, 0.008),
                (-0.006, 0.002),
                SKIN,
                plain,
                Shape::Ball,
            ),
        ];
        for ((da, db), (ra, rb), (z0, z1), rgb, finish, shape) in parts {
            self.round(
                f,
                t,
                at(x + da * k, b + db * k),
                (ra * k / len, rb * k),
                (z + z0 * k, z + z1 * k),
                rgb,
                finish,
                shape,
            );
        }
        // The water to just under the lid, then the lid.
        self.put(
            f,
            t,
            a(a0 + 0.005, a1 - 0.005),
            (b0 + 0.005, b1 - 0.005),
            (floor, top - 0.01),
            WATER,
            Finish::Glass,
            bx,
        );
        self.put(
            f,
            t,
            a(a0, a1),
            (b0, b1),
            (top - 0.01, top + 0.015),
            DARK,
            plain,
            bx,
        );
    }

    /// Tower's exit: the lift car fills the dead end — steel-lined, a handrail, a
    /// button panel, a light — with two door leaves across its open side that slide
    /// apart (`draw`'s `lift`), and an up-arrow over them.
    fn lift(&mut self, maze: &Maze) -> Option<Cell> {
        let open = maze.lift_door()?;
        let c = maze.exit;
        // The car's half-width: out to its walls' faces. Among thin walls its front is
        // a steel wall of their thickness, the doors sliding inside it (`t` = how far
        // it stands out past the face, 0 for a full square: the doors are the front).
        let w = self.face();
        let t = if self.thin { WALL_T } else { 0.0 };
        for d in (0..4).filter(|&d| d != open) {
            self.fit(c, d, 0.0, &[p(w, 0.0, 0.03, (0.0, 1.0), STEEL)]);
        }
        let back = opposite(open);
        self.fit(
            c,
            back,
            0.0,
            &[p(w - 0.1, 0.03, 0.03, (0.3, 0.32), [0.85, 0.85, 0.88])],
        );
        if self.thin {
            for along in [-1.0, 1.0] {
                self.fit(
                    c,
                    open,
                    along * (w + 0.5) * 0.5,
                    &[p((w - 0.5) * 0.5, -t, t, (0.0, 1.0), STEEL)],
                );
            }
        }
        let side = (open + 1) % 4;
        self.fit(
            c,
            side,
            0.25,
            &[
                p(0.05, 0.03, 0.015, (0.36, 0.56), DARK),
                pf(
                    0.015,
                    0.045,
                    0.005,
                    (0.48, 0.5),
                    [1.0, 0.8, 0.3],
                    Finish::Lamp,
                ),
                pf(
                    0.015,
                    0.045,
                    0.005,
                    (0.42, 0.44),
                    [1.0, 0.8, 0.3],
                    Finish::Lamp,
                ),
            ],
        );
        self.group += 1;
        let sq = square(c);
        self.push(
            sq + vec2(0.3, 0.3),
            sq + vec2(0.7, 0.7),
            (0.98, 1.0),
            WHITE,
            Finish::Lamp,
        );
        self.fit(c, open, 0.0, &[p(w, -t, t.max(0.04), (0.86, 1.0), STEEL)]);
        self.fit(
            c,
            open,
            0.0,
            &[pf(
                0.04,
                -t - 0.01,
                0.01,
                (0.9, 0.96),
                [1.0, 0.7, 0.2],
                Finish::Lamp,
            )],
        );
        let slide = if open % 2 == 0 {
            vec2(1.0, 0.0)
        } else {
            vec2(0.0, 1.0)
        };
        let mut leaves = [(0, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO); 2];
        for (k, (along, dir)) in [(-0.25, -1.0), (0.25, 1.0)].into_iter().enumerate() {
            let leaf = [0.8, 0.82, 0.86];
            self.fit(
                c,
                open,
                along,
                &[pf(
                    0.25,
                    if self.thin { -t * 0.5 - 0.015 } else { 0.0 },
                    0.03,
                    (0.0, 0.86),
                    leaf,
                    Finish::Framed,
                )],
            );
            let i = self.blocks.len() - 1;
            leaves[k] = (i, self.blocks[i].min, self.blocks[i].max, slide * dir);
        }
        self.lift = Some(Lift { leaves });
        // Outside: a call-button plate on the hall wall beside the doors, its button lit.
        let hall = c.step(open);
        let to_lift = opposite(open);
        let (lx, ly) = (DIRS[to_lift].0 as f32, DIRS[to_lift].1 as f32);
        if let Some(wall) = [(open + 1) % 4, (open + 3) % 4]
            .into_iter()
            .find(|&d| maze.cell_in_bounds(hall) && maze.edge(hall, d) == EdgeKind::Wall)
        {
            let k = self.face() - 0.1;
            let along = if wall % 2 == 0 { k * lx } else { k * ly };
            self.fit(
                hall,
                wall,
                along,
                &[
                    p(0.035, 0.0, 0.015, (0.38, 0.54), STEEL),
                    pf(
                        0.013,
                        0.015,
                        0.006,
                        (0.44, 0.47),
                        [1.0, 0.7, 0.2],
                        Finish::Lamp,
                    ),
                ],
            );
        }
        Some(hall)
    }

    /// A washroom (one cell, peek-only): toilet with its cistern against the back wall,
    /// a paper roll, a pedestal sink under a mirror on one side wall, a hand dryer on the
    /// other — each flush against its wall, so `settle_room` takes it back with the
    /// wall (held 0.1 off for the old wall-long door sweep, they stayed put and hung in
    /// mid-air once walls went thin).
    fn wc(&mut self, f: (Vec2, Vec2), t: usize) {
        let porcelain = [0.93, 0.94, 0.95];
        self.group += 1;
        self.place(
            f,
            t,
            (0.0, 0.08),
            (0.36, 0.64),
            (0.2, 0.44),
            porcelain,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.08, 0.3),
            (0.39, 0.61),
            (0.0, 0.18),
            porcelain,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.08, 0.3),
            (0.39, 0.61),
            (0.18, 0.2),
            [0.85, 0.86, 0.88],
            Finish::Plain,
        );
        self.group += 1;
        self.place(
            f,
            t,
            (0.02, 0.06),
            (0.7, 0.75),
            (0.26, 0.3),
            WHITE,
            Finish::Plain,
        );
        self.group += 1;
        self.place(
            f,
            t,
            (0.5, 0.66),
            (0.02, 0.1),
            (0.0, 0.29),
            porcelain,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.46, 0.7),
            (0.0, 0.16),
            (0.29, 0.34),
            porcelain,
            Finish::Plain,
        );
        self.place(
            f,
            t,
            (0.56, 0.6),
            (0.01, 0.05),
            (0.34, 0.4),
            STEEL,
            Finish::Plain,
        );
        self.group += 1;
        self.place(
            f,
            t,
            (0.44, 0.72),
            (0.0, 0.012),
            (0.45, 0.78),
            [0.6, 0.72, 0.8],
            Finish::Plain,
        );
        self.group += 1;
        self.place(
            f,
            t,
            (0.5, 0.64),
            (0.92, 1.0),
            (0.46, 0.58),
            STEEL,
            Finish::Plain,
        );
    }

    /// Forest's exit: a campfire — a ring of stones, crossed logs. It's placed each
    /// frame where the exit is (`Dynamic::fire`), since fog moves the exit about; the
    /// flames and glow are drawn over it (`draw_fire`).
    fn campfire(&mut self) {
        let first = self.blocks.len();
        let stone = [0.45, 0.44, 0.42];
        for i in 0..9 {
            let a = i as f32 / 9.0 * std::f32::consts::TAU;
            let at = vec2(a.cos(), a.sin()) * 0.15;
            self.group += 1;
            self.push(
                at - vec2(0.03, 0.03),
                at + vec2(0.03, 0.03),
                (0.0, 0.045),
                stone,
                Finish::Plain,
            );
        }
        let log = [0.3, 0.19, 0.1];
        self.group += 1;
        self.push(
            vec2(-0.13, -0.025),
            vec2(0.13, 0.025),
            (0.0, 0.045),
            log,
            Finish::Plain,
        );
        self.push(
            vec2(-0.025, -0.12),
            vec2(0.025, 0.12),
            (0.045, 0.085),
            log,
            Finish::Plain,
        );
        let embers = [1.0, 0.45, 0.1];
        self.push(
            vec2(-0.06, -0.06),
            vec2(0.06, 0.06),
            (0.0, 0.02),
            embers,
            Finish::Lamp,
        );
        self.fire = Some((first, self.blocks.len()));
        self.fire_base = self.blocks[first..]
            .iter()
            .map(|b| (b.min, b.max))
            .collect();
    }

    /// Dungeon's exit: a stairwell cut into the exit cell's wall (`Maze::stairwell`) —
    /// the steps go down inside the wall square beyond it, which the raycaster lets
    /// rays from the exit cell into (`WallGrid::stair`). A dressed-stone portal (quoined
    /// jambs, stepped voussoirs, a keystone with a skull on it), a portcullis wound half
    /// up with its spikes showing, then a vaulted flight of steps sinking into the dark
    /// between masonry walls, a warm glimmer from something burning round the turn
    /// below. Two torches flank the portal (`fx`).
    fn stairwell(&mut self, maze: &Maze) {
        let Some(d) = maze.stairwell else {
            return;
        };
        let (dx, dy) = DIRS[d];
        // The wall square, moved in to put its exit-side face on the arch (the slab's
        // face among thin walls): `s` below runs from there.
        let sq = square(maze.exit) + vec2(dx as f32, dy as f32) * (1.0 + self.recess());
        let f = (sq, Vec2::ONE);
        let t = opposite(d);
        // `s`: depth past the arch face (0) to the flight's far wall (`STAIR_FLIGHT`);
        // negative stands out into the exit cell. `b` across it, 0..1. The portal is as
        // deep as the wall (`deep`); the flight behind it is seen only through it.
        let put = |me: &mut Self,
                   s: (f32, f32),
                   b: (f32, f32),
                   z: (f32, f32),
                   rgb: [f32; 3],
                   fin: Finish| {
            me.place(f, t, (1.0 - s.1, 1.0 - s.0), b, z, rgb, fin);
        };
        let axis = if dx != 0 { 0 } else { 1 };
        let into = (dx + dy) as f32;
        let plane = if into > 0.0 {
            [sq.x, sq.y][axis]
        } else {
            [sq.x, sq.y][axis] + 1.0
        };
        self.well = Some((axis, plane, into));
        let stairs = self.blocks.len() as u32;
        let dim = |rgb: [f32; 3], k: f32| rgb.map(|c| c * k);
        const ASHLAR: [f32; 3] = [0.5, 0.45, 0.39];
        const DEEP: f32 = -1.0;
        // Short of the wall's far face (never coplanar with it).
        let deep = if self.thin { WALL_T - 0.02 } else { 0.1 };

        // The portal: quoined jambs, alternate courses long and short.
        const OPEN: (f32, f32) = (0.24, 0.76);
        const SPRING: f32 = 0.5;
        let r = (OPEN.1 - OPEN.0) * 0.5;
        self.group += 1;
        for (i, z) in [
            (0.0, 0.17),
            (0.17, 0.34),
            (0.34, SPRING),
            (SPRING, 0.66),
            (0.66, 0.82),
            (0.82, 1.0),
        ]
        .into_iter()
        .enumerate()
        {
            let long = i % 2 == 0;
            let tone = if long { 1.0 } else { 0.9 };
            let out = if long { -0.05 } else { -0.035 };
            put(
                self,
                (out, deep),
                (0.0, OPEN.0),
                z,
                dim(ASHLAR, tone),
                Finish::Framed,
            );
            put(
                self,
                (out, deep),
                (OPEN.1, 1.0),
                z,
                dim(ASHLAR, tone * 0.97),
                Finish::Framed,
            );
        }
        // Voussoirs: the arch filled above its curve, slice by slice.
        let n = 11;
        let w = (OPEN.1 - OPEN.0) / n as f32;
        for i in 0..n {
            let b0 = OPEN.0 + i as f32 * w;
            let x = (b0 + w * 0.5 - 0.5).abs();
            let under = SPRING + (r * r - x * x).max(0.0).sqrt();
            let key = i == n / 2;
            let (z0, out) = if key {
                (under - 0.03, -0.075)
            } else {
                (under, -0.045)
            };
            let tone = if key {
                1.08
            } else if i % 2 == 0 {
                1.0
            } else {
                0.92
            };
            put(
                self,
                (out, deep),
                (b0, b0 + w),
                (z0, 1.0),
                dim(ASHLAR, tone),
                Finish::Framed,
            );
        }
        // The skull on the keystone.
        self.group += 1;
        put(
            self,
            (-0.125, -0.07),
            (0.465, 0.535),
            (0.79, 0.87),
            BONE,
            Finish::Plain,
        );
        self.shaped(Shape::Ball);
        put(
            self,
            (-0.115, -0.075),
            (0.48, 0.52),
            (0.77, 0.8),
            dim(BONE, 0.85),
            Finish::Plain,
        );
        self.shaped(Shape::Cyl);
        for b in [(0.475, 0.494), (0.506, 0.525)] {
            put(self, (-0.13, -0.12), b, (0.815, 0.832), DARK, Finish::Plain);
        }

        // The portcullis, wound half up: its spikes hang in the top of the arch.
        self.group += 1;
        let iron = dim(IRON, 0.8);
        for k in 0..7 {
            let b = OPEN.0 + 0.03 + k as f32 * 0.0767;
            put(
                self,
                (0.1, 0.122),
                (b, b + 0.018),
                (0.62, 1.0),
                iron,
                Finish::Plain,
            );
            put(
                self,
                (0.104, 0.118),
                (b + 0.004, b + 0.014),
                (0.575, 0.62),
                iron,
                Finish::Plain,
            );
        }
        for z in [0.68, 0.84] {
            put(
                self,
                (0.098, 0.124),
                OPEN,
                (z, z + 0.02),
                iron,
                Finish::Plain,
            );
        }

        // The well: a sill, then steps each lower and darker than the last, between
        // coursed walls under a stepped vault, down to where the flight turns.
        self.group += 1;
        let flight = self.blocks.len() as u32;
        const WALL: (f32, f32) = (0.26, 0.74);
        put(
            self,
            (-0.02, 0.1),
            OPEN,
            (DEEP, 0.0),
            dim(STONE, 1.3),
            Finish::Plain,
        );
        // Shallow risers: from the top, looking down, every tread shows.
        use crate::maze::{STAIR_RISE as RISE, STAIR_SILL, STAIR_STEPS as STEPS, STAIR_TREAD};
        let tread = STAIR_TREAD;
        for k in 0..STEPS {
            let s0 = STAIR_SILL + tread * k as f32;
            let dark = 1.0 - 0.8 * k as f32 / (STEPS - 1) as f32;
            let top = -RISE * (k + 1) as f32;
            let s1 = s0 + tread + 0.002;
            // The lowest treads catch the glow of a torch round the turn below.
            let warm = [0.0, 0.0, 0.0, 0.0, 0.0, 0.25, 0.5, 0.75][k];
            let rgb = dim(STONE, 1.7 * dark).map(|c| c * (1.0 - warm));
            let rgb = [0, 1, 2].map(|i| rgb[i] + [0.55, 0.27, 0.09][i] * warm);
            put(self, (s0, s1), WALL, (DEEP, top), rgb, Finish::Framed);
            // The vault overhead follows the flight down, a pair of treads at a time.
            if k % 2 == 0 {
                let roof = 0.76 - RISE * k as f32;
                let s1 = s0 + 2.0 * tread + 0.002;
                put(
                    self,
                    (s0, s1),
                    (0.0, 1.0),
                    (roof, 1.0),
                    dim(STONE, 0.9 * dark),
                    Finish::Plain,
                );
            }
        }
        // Side walls, three lengths each side, darkening down the flight.
        for (k, tone) in [0.95, 0.55, 0.22].into_iter().enumerate() {
            let s0 = 0.1 + 0.3 * k as f32;
            let side = dim(STONE, tone);
            put(
                self,
                (s0, s0 + 0.302),
                (0.0, WALL.0),
                (DEEP, 1.0),
                side,
                Finish::Bands(0.25),
            );
            put(
                self,
                (s0, s0 + 0.302),
                (WALL.1, 1.0),
                (DEEP, 1.0),
                side,
                Finish::Bands(0.25),
            );
        }
        // Where the flight turns out of sight: black, and a sliver of torchlight on the
        // right-hand wall low down, round the corner.
        put(
            self,
            (0.97, 1.0),
            (0.0, 1.0),
            (DEEP, 1.0),
            [0.012, 0.01, 0.008],
            Finish::Lamp,
        );
        put(
            self,
            (0.96, 0.97),
            (0.7, 0.74),
            (DEEP, 0.0),
            [0.22, 0.09, 0.03],
            Finish::Lamp,
        );
        put(
            self,
            (0.955, 0.96),
            (0.72, 0.74),
            (DEEP, -0.25),
            [0.45, 0.2, 0.06],
            Finish::Lamp,
        );
        self.flight = (flight, self.blocks.len() as u32);
        self.stairs = (stairs, self.blocks.len() as u32);
    }

    /// Janitor's closet: open steel shelving along the back wall stocked with toilet
    /// rolls, cleaners, cans and boxes; a broom leaning on a side wall; a wheeled mop
    /// bucket (wringer, dirty water, the mop standing in it) and a WET FLOOR sign down
    /// one side; a boombox with its speaker cones down the other — and in the back
    /// corner, on a crate under a safety poster, a lava lamp: tapered glass on a chrome
    /// base, wax blobs rising and sinking in glowing liquid (`Motion::Blob`). This
    /// closet has a second life. The bot steps in here (`Knowledge::lure`), so the
    /// middle and the way in from the door stay clear: everything hugs the back and
    /// side walls — laid out against the real walls (`shifted` by `recess`), not the
    /// full-square floor: moved back fitting by fitting, the side ones stood in a huddle
    /// half a unit off their walls.
    fn janitor(&mut self, f0: (Vec2, Vec2), t: usize) {
        let m = self.recess();
        const SHELF: [f32; 3] = [0.62, 0.64, 0.66];
        const ROLL: [f32; 3] = [0.95, 0.95, 0.93];
        const CARDBOARD: [f32; 3] = [0.62, 0.47, 0.3];
        const CHROME: [f32; 3] = [0.78, 0.78, 0.82];
        const LIQUID: [f32; 3] = [0.62, 0.14, 0.55];
        const WAX: [f32; 3] = [1.0, 0.5, 0.16];
        const DIRTY: [f32; 3] = [0.36, 0.34, 0.26];
        let (plain, bx) = (Finish::Plain, Shape::Box);
        let (cyl, ball) = (Shape::Cyl, Shape::Ball);

        // Shelving: four posts, four boards, on the back wall left of the middle.
        let f = shifted(f0, t, -m, -0.75 * m);
        self.group += 1;
        for a in [(0.0, 0.015), (0.185, 0.2)] {
            for b in [(0.12, 0.135), (0.605, 0.62)] {
                self.put(f, t, a, b, (0.0, 0.8), SHELF, plain, bx);
            }
        }
        let boards = [0.03, 0.25, 0.47, 0.69];
        for z in boards {
            self.put(
                f,
                t,
                (0.0, 0.2),
                (0.12, 0.62),
                (z - 0.015, z),
                SHELF,
                plain,
                bx,
            );
        }
        // Bottom shelf: two stacks of toilet rolls, a paint tin, a bleach jug, a case.
        let z = boards[0];
        for b in [0.17, 0.235] {
            for k in 0..3 {
                let z0 = z + 0.055 * k as f32;
                self.round(
                    f,
                    t,
                    (0.1, b),
                    (0.03, 0.03),
                    (z0, z0 + 0.055),
                    ROLL,
                    plain,
                    cyl,
                );
            }
            let top = z + 0.165;
            self.round(
                f,
                t,
                (0.1, b),
                (0.01, 0.01),
                (top, top + 0.002),
                GREY,
                plain,
                cyl,
            );
        }
        self.round(
            f,
            t,
            (0.1, 0.37),
            (0.04, 0.04),
            (z, z + 0.09),
            TIN,
            Finish::Bands(0.03),
            cyl,
        );
        self.put(
            f,
            t,
            (0.05, 0.15),
            (0.45, 0.53),
            (z, z + 0.13),
            ROLL,
            plain,
            bx,
        );
        self.round(
            f,
            t,
            (0.1, 0.49),
            (0.012, 0.012),
            (z + 0.13, z + 0.15),
            RED,
            plain,
            cyl,
        );
        self.put(
            f,
            t,
            (0.04, 0.16),
            (0.545, 0.6),
            (z, z + 0.08),
            CARDBOARD,
            plain,
            bx,
        );
        // Cleaners: bottle, shoulder, neck, a spray head on some.
        let z = boards[1];
        for (i, (b, rgb)) in [
            (0.16, [0.2, 0.5, 0.9]),
            (0.22, [0.3, 0.75, 0.3]),
            (0.28, [0.95, 0.55, 0.1]),
            (0.34, [0.85, 0.2, 0.3]),
        ]
        .into_iter()
        .enumerate()
        {
            let r = (0.022, 0.022);
            self.round(f, t, (0.1, b), r, (z, z + 0.09), rgb, plain, cyl);
            self.round(
                f,
                t,
                (0.1, b),
                r,
                (z + 0.09, z + 0.11),
                rgb,
                plain,
                Shape::Dome,
            );
            let n = (0.007, 0.007);
            self.round(f, t, (0.1, b), n, (z + 0.11, z + 0.13), WHITE, plain, cyl);
            if i % 2 == 0 {
                let h = z + 0.13;
                self.put(
                    f,
                    t,
                    (0.09, 0.13),
                    (b - 0.008, b + 0.008),
                    (h, h + 0.02),
                    WHITE,
                    plain,
                    bx,
                );
            }
        }
        self.put(
            f,
            t,
            (0.06, 0.14),
            (0.4, 0.47),
            (z, z + 0.03),
            YELLOW,
            plain,
            bx,
        );
        self.round(
            f,
            t,
            (0.1, 0.54),
            (0.06, 0.05),
            (z, z + 0.05),
            [0.55, 0.6, 0.72],
            plain,
            ball,
        );
        // Cans; a box of light bulbs.
        let z = boards[2];
        for (i, b) in [0.16, 0.21, 0.26, 0.31, 0.36].into_iter().enumerate() {
            let rgb = [[0.75, 0.15, 0.12], [0.2, 0.35, 0.7], [0.8, 0.75, 0.2]][i % 3];
            let r = (0.021, 0.021);
            self.round(
                f,
                t,
                (0.1, b),
                r,
                (z, z + 0.065),
                rgb,
                Finish::Bands(0.022),
                cyl,
            );
        }
        self.put(
            f,
            t,
            (0.04, 0.16),
            (0.44, 0.58),
            (z, z + 0.06),
            CARDBOARD,
            plain,
            bx,
        );
        // A toilet-roll multipack, a spare box.
        let z = boards[3];
        let pack = [0.9, 0.9, 0.95];
        self.put(
            f,
            t,
            (0.03, 0.17),
            (0.15, 0.35),
            (z, z + 0.11),
            pack,
            Finish::Bands(0.055),
            bx,
        );
        self.put(
            f,
            t,
            (0.04, 0.16),
            (0.4, 0.58),
            (z, z + 0.08),
            CARDBOARD,
            plain,
            bx,
        );

        // The lava lamp on its crate, in the back right corner, under the poster.
        let f = shifted(f0, t, -m, 1.1 * m);
        self.group += 1;
        self.put(
            f,
            t,
            (0.03, 0.19),
            (0.68, 0.86),
            (0.0, 0.2),
            WOOD,
            Finish::Bands(0.066),
            bx,
        );
        let at = (0.11, 0.77);
        let mut z = 0.2;
        // A chrome cone, in thin slices so it doesn't read as a stack of tins.
        for (r, h) in (0..7).map(|i| (0.05 - 0.0028 * i as f32, 0.013)) {
            self.round(f, t, at, (r, r), (z, z + h), CHROME, plain, cyl);
            z += h;
        }
        const GLASS: f32 = 0.034;
        let (lo, hi) = (z, z + 0.2);
        self.round(
            f,
            t,
            at,
            (0.6 * GLASS, 0.6 * GLASS),
            (lo, lo + 0.02),
            WAX,
            Finish::Lamp,
            Shape::Dome,
        );
        let axis = room_point(f, t, at.0, at.1);
        for k in 0..4 {
            self.round(
                f,
                t,
                at,
                (GLASS, GLASS),
                (lo, lo + 0.04),
                WAX,
                Finish::Lamp,
                ball,
            );
            self.animate(Motion::Blob {
                axis,
                lo,
                hi,
                phase: k as f32 * 0.27 + 0.1,
            });
        }
        for i in 0..6 {
            let r = GLASS * (LAVA_GLASS[i] + LAVA_GLASS[i + 1]) * 0.5;
            let z0 = lo + (hi - lo) * i as f32 / 6.0;
            let z1 = lo + (hi - lo) * (i + 1) as f32 / 6.0;
            self.round(f, t, at, (r, r), (z0, z1), LIQUID, Finish::Lava, cyl);
        }
        let mut z = hi;
        for (r, h) in (0..5).map(|i| (0.022 - 0.0025 * i as f32, 0.008)) {
            self.round(f, t, at, (r, r), (z, z + h), CHROME, plain, cyl);
            z += h;
        }
        let out = dir_vec(t);
        self.posters.push(Poster {
            at: room_point(f, t, 0.0, 0.77) + out * 0.004,
            out,
            lines: &CLOSET_POSTER,
            icon: None,
            z: 0.72,
            width: 0.2,
            cap: 0.028,
            ink: Color::new(0.75, 0.08, 0.06, 1.0),
            paper: PAPER,
            glow: false,
        });

        // A broom stood upright in the back corner — a round handle, since a leaning one
        // could only be stepped boxes (blocks are upright), which read as pixelated.
        let f = shifted(f0, t, -1.5 * m, -1.25 * m);
        self.group += 1;
        self.put(
            f,
            t,
            (0.24, 0.275),
            (0.15, 0.225),
            (0.0, 0.04),
            [0.75, 0.6, 0.3],
            Finish::Bands(0.01),
            bx,
        );
        self.put(
            f,
            t,
            (0.235, 0.28),
            (0.155, 0.225),
            (0.04, 0.07),
            [0.5, 0.3, 0.15],
            plain,
            bx,
        );
        self.round(
            f,
            t,
            (0.257, 0.19),
            (0.007, 0.007),
            (0.07, 0.72),
            WOOD,
            plain,
            Shape::Cyl,
        );

        // The mop bucket on its casters against the side wall: walls round dirty water,
        // a wringer on the door end, the mop standing in it.
        let f = shifted(f0, t, 0.0, -1.25 * m);
        self.group += 1;
        let (a0, a1, b0, b1) = (0.36, 0.56, 0.11, 0.225);
        self.put(f, t, (a0, a1), (b0, b1), (0.02, 0.03), YELLOW, plain, bx);
        let w = 0.012;
        for (a, b) in [
            ((a0, a1), (b0, b0 + w)),
            ((a0, a1), (b1 - w, b1)),
            ((a0, a0 + w), (b0 + w, b1 - w)),
            ((a1 - w, a1), (b0 + w, b1 - w)),
        ] {
            self.put(f, t, a, b, (0.03, 0.16), YELLOW, plain, bx);
        }
        self.put(
            f,
            t,
            (a0 + w, a1 - w),
            (b0 + w, b1 - w),
            (0.03, 0.13),
            DIRTY,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.5, 0.56),
            (0.115, 0.225),
            (0.16, 0.22),
            [0.5, 0.52, 0.55],
            plain,
            bx,
        );
        for a in [a0 + 0.015, a1 - 0.015] {
            for b in [b0 + 0.015, b1 - 0.015] {
                self.round(
                    f,
                    t,
                    (a, b),
                    (0.012, 0.012),
                    (0.0, 0.024),
                    DARK,
                    plain,
                    ball,
                );
            }
        }
        self.round(
            f,
            t,
            (0.405, 0.165),
            (0.025, 0.025),
            (0.08, 0.14),
            [0.8, 0.8, 0.76],
            plain,
            ball,
        );
        self.round(
            f,
            t,
            (0.406, 0.166),
            (0.006, 0.006),
            (0.06, 0.72),
            WOOD,
            plain,
            cyl,
        );

        // The WET FLOOR sign by the door (clear of its swing): a yellow A-frame,
        // stepped in, its warning facing whoever comes in.
        let f = shifted(f0, t, 0.75 * m, -1.1 * m);
        self.group += 1;
        for i in 0..8 {
            let half = 0.06 - 0.006 * i as f32;
            let z0 = 0.035 * i as f32;
            let a = (0.78 - half, 0.78 + half);
            self.put(f, t, a, (0.12, 0.22), (z0, z0 + 0.035), YELLOW, plain, bx);
        }
        self.put(
            f,
            t,
            (0.765, 0.795),
            (0.15, 0.19),
            (0.28, 0.3),
            YELLOW,
            plain,
            bx,
        );
        self.posters.push(Poster {
            at: room_point(f, t, 0.8205, 0.17),
            out,
            lines: &WET_FLOOR,
            icon: None,
            z: 0.15,
            width: 0.09,
            cap: 0.015,
            ink: INK,
            paper: Color::new(0.95, 0.8, 0.12, 1.0),
            glow: false,
        });

        // The boombox against the other side wall, its speakers toward the room.
        let f = shifted(f0, t, 0.0, 1.2 * m);
        self.group += 1;
        let body = [0.28, 0.28, 0.32];
        self.put(f, t, (0.3, 0.52), (0.79, 0.9), (0.0, 0.12), body, plain, bx);
        for a in [0.355, 0.465] {
            let c = |r: f32, b: f32| (r, b);
            for (r, b, hb, rgb) in [
                (0.036, 0.791, 0.006, DARK),
                (0.028, 0.788, 0.006, [0.45, 0.45, 0.48]),
                (0.009, 0.785, 0.005, DARK),
            ] {
                let (r, b) = c(r, b);
                self.round(
                    f,
                    t,
                    (a, b),
                    (r, hb),
                    (0.06 - r, 0.06 + r),
                    rgb,
                    plain,
                    ball,
                );
            }
        }
        self.put(
            f,
            t,
            (0.4, 0.42),
            (0.786, 0.79),
            (0.03, 0.095),
            DARK,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.33, 0.49),
            (0.786, 0.79),
            (0.1, 0.112),
            [1.0, 0.6, 0.2],
            Finish::Lamp,
            bx,
        );
        for a in [(0.33, 0.34), (0.48, 0.49)] {
            self.put(f, t, a, (0.84, 0.85), (0.12, 0.16), STEEL, plain, bx);
        }
        self.put(
            f,
            t,
            (0.33, 0.49),
            (0.838, 0.852),
            (0.16, 0.172),
            STEEL,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            (0.505, 0.51),
            (0.87, 0.875),
            (0.12, 0.3),
            STEEL,
            plain,
            bx,
        );
    }

    /// Kitchen (two cells, door on the short end): fridge in the back corner, a counter
    /// with a sink, the microwave and the kettle along the rest of the back wall, a
    /// dining table with chairs down the middle, a vending machine and a bin down one
    /// side and a TV on the other. Laid out against the real walls (`shifted`, as in
    /// `janitor`): moved back with them, the fridge and counter sat in the middle of
    /// the back wall and the table in its half of the room.
    fn kitchen(&mut self, f0: (Vec2, Vec2), t: usize) {
        let m = self.recess();
        // `b` runs toward DIRS[across].
        let across = if t.is_multiple_of(2) { 1 } else { 2 };
        let (plain, bx) = (Finish::Plain, Shape::Box);
        let len = (f0.1 * dir_vec(t)).length();
        let a = |w0: f32, w1: f32| (w0 / len, w1 / len);
        let (lo, hi) = (-m, 1.0 + m);
        // Along the back wall.
        let f = shifted(f0, t, -m, 0.0);
        self.group += 1;
        self.put(
            f,
            t,
            a(0.0, 0.24),
            (lo, lo + 0.24),
            (0.0, 0.66),
            WHITE,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            a(0.24, 0.255),
            (lo + 0.02, lo + 0.22),
            (0.35, 0.36),
            GREY,
            plain,
            bx,
        );
        self.group += 1;
        let (c0, c1) = (lo + 0.28, hi);
        self.put(
            f,
            t,
            a(0.0, 0.21),
            (c0, c1),
            (0.0, 0.3),
            CUPBOARD,
            Finish::Bands(0.15),
            bx,
        );
        self.put(f, t, a(0.0, 0.225), (c0, c1), (0.3, 0.315), DARK, plain, bx);
        // The sink, its tap reaching over from the wall.
        let s = c0 + 0.22;
        self.put(
            f,
            t,
            a(0.03, 0.19),
            (s, s + 0.26),
            (0.311, 0.317),
            STEEL,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            a(0.06, 0.16),
            (s + 0.03, s + 0.23),
            (0.312, 0.319),
            GREY,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            a(0.01, 0.03),
            (s + 0.12, s + 0.14),
            (0.315, 0.4),
            STEEL,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            a(0.01, 0.08),
            (s + 0.12, s + 0.14),
            (0.385, 0.4),
            STEEL,
            plain,
            bx,
        );
        let micro = Finish::Screen(t, [0.25, 0.3, 0.3]);
        self.put(
            f,
            t,
            a(0.03, 0.15),
            (hi - 0.55, hi - 0.4),
            (0.315, 0.41),
            DARK,
            micro,
            bx,
        );
        self.put(
            f,
            t,
            a(0.03, 0.12),
            (hi - 0.2, hi - 0.14),
            (0.315, 0.45),
            DARK,
            plain,
            bx,
        );
        // The table and chairs, down the middle of the real floor.
        let f = f0;
        self.group += 1;
        self.place(f, t, (0.28, 0.68), (0.3, 0.7), (0.28, 0.31), WOOD, plain);
        self.place(
            f,
            t,
            (0.3, 0.33),
            (0.32, 0.35),
            (0.0, 0.28),
            DARK_WOOD,
            plain,
        );
        self.place(
            f,
            t,
            (0.63, 0.66),
            (0.65, 0.68),
            (0.0, 0.28),
            DARK_WOOD,
            plain,
        );
        self.place(
            f,
            t,
            (0.46, 0.5),
            (0.45, 0.55),
            (0.31, 0.34),
            [0.9, 0.6, 0.15],
            plain,
        );
        for a in [0.34, 0.54] {
            for b in [(0.17, 0.27), (0.73, 0.83)] {
                self.group += 1;
                self.place(f, t, (a, a + 0.1), b, (0.18, 0.21), WOOD, plain);
                let leg = (b.0 + 0.04, b.1 - 0.04);
                self.place(
                    f,
                    t,
                    (a + 0.04, a + 0.06),
                    leg,
                    (0.0, 0.18),
                    DARK_WOOD,
                    plain,
                );
            }
        }
        // Down one side: a snack machine glowing behind its glass, and a pedal bin
        // short of the door's swing.
        self.group += 1;
        self.put(
            f,
            t,
            a(1.0, 1.42),
            (lo, lo + 0.27),
            (0.0, 0.72),
            DARK,
            plain,
            bx,
        );
        let glass = Finish::Screen(across, [0.8, 0.86, 0.9]);
        let front = (lo + 0.27, lo + 0.28);
        self.put(f, t, a(1.04, 1.3), front, (0.14, 0.66), DARK, glass, bx);
        self.put(f, t, a(1.33, 1.39), front, (0.4, 0.55), GREY, plain, bx);
        self.put(
            f,
            t,
            a(1.04, 1.3),
            front,
            (0.04, 0.1),
            [0.08, 0.08, 0.09],
            plain,
            bx,
        );
        self.group += 1;
        self.put(
            f,
            t,
            a(2.35, 2.53),
            (lo, lo + 0.16),
            (0.0, 0.2),
            GREY,
            plain,
            bx,
        );
        self.put(
            f,
            t,
            a(2.35, 2.53),
            (lo, lo + 0.16),
            (0.2, 0.215),
            DARK,
            plain,
            bx,
        );
        // The TV on the other side wall, screen to the room.
        self.group += 1;
        let tv = Finish::Screen(opposite(across), [0.9, 0.55, 0.3]);
        self.put(
            f,
            t,
            a(1.05, 1.8),
            (hi - 0.03, hi),
            (0.45, 0.62),
            DARK,
            tv,
            bx,
        );
    }

    /// Nine chairs round a table for two, and a screen the size of the wall. A
    /// credenza with the coffee along one side wall, a plant by the door on the other.
    fn meeting_crowded(&mut self, fl: &Floor) {
        self.group += 1;
        let t = fl.dir;
        let w = fl.half - 0.1;
        self.slab(
            fl,
            (0.0, 0.025),
            (-w, w),
            (0.28, 0.88),
            DARK,
            Finish::Chart(t),
        );
        // The table: a round white top on a pedestal, a speakerphone in the middle.
        let (ta, tr) = (0.66, 0.17);
        self.group += 1;
        let at = fl.at(ta, 0.0);
        self.round_at(at, Vec2::splat(0.1), (0.0, 0.015), GREY, Shape::Cyl);
        self.round_at(at, Vec2::splat(0.025), (0.015, 0.35), GREY, Shape::Cyl);
        self.round_at(at, Vec2::splat(tr), (0.35, 0.37), WHITE, Shape::Cyl);
        self.round_at(at, Vec2::splat(0.045), (0.37, 0.385), DARK, Shape::Dome);
        // Nine swivel chairs packed round it, one facing the screen's wall.
        for k in 0..9 {
            let phi = k as f32 * std::f32::consts::TAU / 9.0;
            let (c, s) = (phi.cos(), phi.sin());
            let at = fl.at(ta - 0.4 * c, 0.4 * s);
            let back = if c.abs() >= s.abs() {
                -fl.fwd * c.signum()
            } else {
                fl.side * s.signum()
            };
            self.swivel_chair(at, back, [0.3, 0.34, 0.42]);
        }
        // The credenza: a low cabinet, a coffee flask and a stack of cups on it.
        self.group += 1;
        let b = -fl.half;
        self.slab(
            fl,
            (0.4, 1.1),
            (b, b + 0.2),
            (0.0, 0.3),
            WOOD,
            Finish::Plain,
        );
        self.slab(
            fl,
            (0.42, 1.08),
            (b + 0.2, b + 0.205),
            (0.03, 0.27),
            DARK_WOOD,
            Finish::Bands(0.12),
        );
        self.round_at(
            fl.at(0.6, b + 0.1),
            Vec2::splat(0.035),
            (0.3, 0.42),
            [0.75, 0.2, 0.15],
            Shape::Cyl,
        );
        self.round_at(
            fl.at(0.6, b + 0.1),
            Vec2::splat(0.012),
            (0.42, 0.44),
            DARK,
            Shape::Cyl,
        );
        for (a, z) in [(0.8, 0.3), (0.8, 0.325), (0.8, 0.35), (0.9, 0.3)] {
            self.round_at(
                fl.at(a, b + 0.1),
                Vec2::splat(0.025),
                (z, z + 0.025),
                WHITE,
                Shape::Cyl,
            );
        }
        self.plant_at(fl.at(fl.depth - 0.35, fl.half - 0.1), fl.side);
    }

    /// The zen room: a neon CALM on the back wall, a round brass gong on its frame in
    /// one corner and a plant in the other, three slumped beanbags round a raked sand
    /// garden with rocks. (Boxes read as a painting in a doorway and a pile of crates:
    /// everything here is round.)
    fn meeting_zen(&mut self, fl: &Floor) {
        let out = fl.fwd;
        self.posters.push(Poster {
            at: fl.at(0.0, 0.0) + out * 0.004,
            out,
            lines: &[ZEN_NEON],
            icon: None,
            z: 0.68,
            width: 0.4,
            cap: 0.085,
            ink: Color::new(0.35, 1.0, 0.85, 1.0),
            paper: Color::new(0.05, 0.07, 0.07, 1.0),
            glow: true,
        });
        // The gong: two posts and a crossbar, the disc hung between on two cords.
        self.group += 1;
        let (g0, g1) = (-fl.half + 0.12, -fl.half + 0.52);
        let gm = (g0 + g1) * 0.5;
        let a = (0.1, 0.14);
        for b in [g0, g1 - 0.03] {
            self.slab(fl, a, (b, b + 0.03), (0.0, 0.7), DARK_WOOD, Finish::Plain);
        }
        self.slab(fl, a, (g0, g1), (0.67, 0.71), DARK_WOOD, Finish::Plain);
        for b in [gm - 0.07, gm + 0.07] {
            self.slab(
                fl,
                (0.118, 0.122),
                (b, b + 0.006),
                (0.62, 0.67),
                DARK,
                Finish::Plain,
            );
        }
        let disc = fl.at(0.12, gm);
        self.round_at(disc, fl.span(0.01, 0.15), (0.32, 0.62), GOLD, Shape::Ball);
        let boss = disc + fl.fwd * 0.01;
        self.round_at(
            boss,
            fl.span(0.008, 0.045),
            (0.44, 0.5),
            [0.6, 0.45, 0.15],
            Shape::Ball,
        );
        // A plant in the other back corner.
        self.plant_at(fl.at(0.1, fl.half - 0.3), -fl.fwd);
        // The sand garden: raked sand in a low wooden rim, three rocks.
        self.group += 1;
        let sand = [0.85, 0.8, 0.65];
        let (a0, a1, b0, b1) = (0.5, 1.0, -0.32, 0.28);
        self.slab(
            fl,
            (a0, a1),
            (b0, b1),
            (0.0, 0.025),
            sand,
            Finish::Bands(0.02),
        );
        let e = 0.02;
        for (a, b) in [
            ((a0, a1), (b0, b0 + e)),
            ((a0, a1), (b1 - e, b1)),
            ((a0, a0 + e), (b0, b1)),
            ((a1 - e, a1), (b0, b1)),
        ] {
            self.slab(fl, a, b, (0.0, 0.04), DARK_WOOD, Finish::Plain);
        }
        let rock = [0.42, 0.42, 0.44];
        for (a, b, r) in [(0.64, -0.14, 0.06), (0.84, 0.12, 0.042), (0.62, 0.14, 0.03)] {
            self.round_at(
                fl.at(a, b),
                fl.span(r, r * 1.2),
                (0.02, 0.02 + r * 1.4),
                rock,
                Shape::Ball,
            );
        }
        // Beanbags round the garden, backs slumped away from it.
        for (a, b, back, col) in [
            (1.25, -0.6, fl.fwd, [0.85, 0.35, 0.2]),
            (0.55, 0.6, fl.side, [0.3, 0.5, 0.8]),
            (1.2, 0.58, fl.side, [0.5, 0.7, 0.3]),
        ] {
            self.beanbag(fl.at(a, b), back, 0.17, col);
        }
    }

    /// Twelve chairs down a long table, a throne at its head, a screen behind that, a
    /// speakerphone, a water glass at every place, a bowl of fruit.
    fn boardroom_throne(&mut self, fl: &Floor) {
        let t = fl.dir;
        self.group += 1;
        self.slab(
            fl,
            (0.0, 0.02),
            (-0.72, 0.72),
            (0.44, 0.9),
            DARK,
            Finish::Chart(t),
        );
        // The table and its two pedestals.
        self.group += 1;
        let (t0, t1, tw) = (0.62, 2.85, 0.3);
        self.slab(
            fl,
            (t0, t1),
            (-tw, tw),
            (0.34, 0.37),
            DARK_WOOD,
            Finish::Plain,
        );
        for a in [t0 + 0.35, t1 - 0.35] {
            self.round_at(
                fl.at(a, 0.0),
                Vec2::splat(0.04),
                (0.02, 0.34),
                DARK,
                Shape::Cyl,
            );
            self.slab(
                fl,
                (a - 0.03, a + 0.03),
                (-0.2, 0.2),
                (0.0, 0.02),
                DARK,
                Finish::Plain,
            );
        }
        self.round_at(
            fl.at(1.75, 0.0),
            fl.span(0.06, 0.06),
            (0.37, 0.39),
            DARK,
            Shape::Dome,
        );
        let bowl = fl.at(1.2, 0.0);
        self.round_at(bowl, Vec2::splat(0.07), (0.37, 0.4), WHITE, Shape::Bowl);
        for (d, col) in [
            (vec2(-0.025, 0.0), [0.95, 0.6, 0.1]),
            (vec2(0.02, 0.02), [0.8, 0.15, 0.1]),
            (vec2(0.015, -0.025), [0.5, 0.75, 0.2]),
        ] {
            self.round_at(
                bowl + d,
                Vec2::splat(0.025),
                (0.385, 0.435),
                col,
                Shape::Ball,
            );
        }
        // Six a side, each with its glass of water.
        for i in 0..6 {
            let a = 0.9 + i as f32 * 0.36;
            for s in [-1.0, 1.0] {
                self.swivel_chair(fl.at(a, 0.47 * s), fl.side * s, [0.3, 0.2, 0.16]);
                self.group += 1;
                let glass = [0.7, 0.85, 0.95];
                self.round_at(
                    fl.at(a - 0.08, 0.22 * s),
                    Vec2::splat(0.014),
                    (0.37, 0.405),
                    glass,
                    Shape::Cyl,
                );
            }
        }
        // The throne at the head, facing the door: gilt frame, crimson velvet, a back
        // that stands over the table, gold knobs on top.
        self.group += 1;
        let velvet = [0.55, 0.06, 0.1];
        self.slab(
            fl,
            (0.2, 0.46),
            (-0.17, 0.17),
            (0.0, 0.2),
            GOLD,
            Finish::Plain,
        );
        self.slab(
            fl,
            (0.21, 0.45),
            (-0.15, 0.15),
            (0.2, 0.25),
            velvet,
            Finish::Plain,
        );
        self.slab(
            fl,
            (0.15, 0.21),
            (-0.17, 0.17),
            (0.0, 0.7),
            GOLD,
            Finish::Plain,
        );
        self.slab(
            fl,
            (0.21, 0.215),
            (-0.13, 0.13),
            (0.26, 0.66),
            velvet,
            Finish::Plain,
        );
        for s in [-1.0, 1.0] {
            let b = (0.155 * s - 0.03, 0.155 * s + 0.03);
            self.slab(fl, (0.21, 0.45), b, (0.25, 0.36), GOLD, Finish::Plain);
            self.round_at(
                fl.at(0.18, 0.15 * s),
                Vec2::splat(0.035),
                (0.7, 0.77),
                GOLD,
                Shape::Ball,
            );
        }
    }

    /// "Culture": a ping-pong table, beanbags, neon on the back wall, an arcade cabinet
    /// in the corner.
    fn boardroom_culture(&mut self, fl: &Floor) {
        let t = fl.dir;
        // GOOD VIBES ONLY, one line each, VIBES in its own colour.
        let pink = Color::new(1.0, 0.45, 0.95, 1.0);
        let lime = Color::new(0.45, 1.0, 0.55, 1.0);
        for (line, z, ink) in [
            (&CULTURE_NEON[0..1], 0.8, pink),
            (&CULTURE_NEON[1..2], 0.66, lime),
            (&CULTURE_NEON[2..3], 0.52, pink),
        ] {
            self.posters.push(Poster {
                at: fl.at(0.0, -0.2) + fl.fwd * 0.004,
                out: fl.fwd,
                lines: line,
                icon: None,
                z,
                width: 0.7,
                cap: 0.09,
                ink,
                paper: Color::new(0.07, 0.04, 0.08, 1.0),
                glow: true,
            });
        }
        // The arcade cabinet, back in the far corner, screen and marquee toward the door.
        self.group += 1;
        let cab = [0.2, 0.15, 0.4];
        let (b0, b1) = (fl.half - 0.38, fl.half);
        self.slab(fl, (0.0, 0.4), (b0, b1), (0.0, 0.44), cab, Finish::Plain);
        self.slab(fl, (0.0, 0.3), (b0, b1), (0.44, 0.92), cab, Finish::Plain);
        self.slab(
            fl,
            (0.3, 0.4),
            (b0 + 0.02, b1 - 0.02),
            (0.44, 0.46),
            DARK,
            Finish::Plain,
        );
        let game = Finish::Screen(t, [0.3, 1.0, 0.4]);
        self.slab(
            fl,
            (0.3, 0.305),
            (b0 + 0.05, b1 - 0.05),
            (0.52, 0.76),
            DARK,
            game,
        );
        self.slab(
            fl,
            (0.3, 0.31),
            (b0 + 0.03, b1 - 0.03),
            (0.8, 0.88),
            [1.0, 0.8, 0.3],
            Finish::Lamp,
        );
        self.round_at(
            fl.at(0.35, b0 + 0.13),
            Vec2::splat(0.018),
            (0.46, 0.51),
            RED,
            Shape::Ball,
        );
        // The ping-pong table: blue top, white lines, the net, four legs, bats, a ball.
        self.group += 1;
        let (a0, a1, w) = (1.2, 2.57, 0.38);
        let top = (0.36, 0.38);
        for a in [a0 + 0.2, a1 - 0.22] {
            for b in [-w + 0.08, w - 0.1] {
                self.slab(
                    fl,
                    (a, a + 0.02),
                    (b, b + 0.02),
                    (0.0, top.0),
                    GREY,
                    Finish::Plain,
                );
            }
        }
        self.slab(fl, (a0, a1), (-w, w), top, [0.1, 0.3, 0.6], Finish::Plain);
        let line = (top.1, top.1 + 0.002);
        self.slab(fl, (a0, a1), (-0.006, 0.006), line, WHITE, Finish::Plain);
        let mid = (a0 + a1) * 0.5;
        let net = (-w - 0.03, w + 0.03);
        self.slab(
            fl,
            (mid - 0.004, mid + 0.004),
            net,
            (top.1, top.1 + 0.068),
            [0.2, 0.2, 0.24],
            Finish::Plain,
        );
        self.slab(
            fl,
            (mid - 0.005, mid + 0.005),
            net,
            (top.1 + 0.068, top.1 + 0.076),
            WHITE,
            Finish::Plain,
        );
        for (a, b, col) in [(a0 + 0.3, 0.2, RED), (a1 - 0.35, -0.15, [0.1, 0.1, 0.1])] {
            self.round_at(
                fl.at(a, b),
                Vec2::splat(0.05),
                (top.1, top.1 + 0.012),
                col,
                Shape::Cyl,
            );
        }
        self.round_at(
            fl.at(mid + 0.25, 0.05),
            Vec2::splat(0.012),
            (top.1, top.1 + 0.024),
            WHITE,
            Shape::Ball,
        );
        self.beanbag(fl.at(0.5, -0.55), -fl.fwd, 0.2, [0.85, 0.35, 0.2]);
        self.beanbag(fl.at(3.05, 0.58), fl.side, 0.2, [0.95, 0.8, 0.2]);
    }

    /// A beanbag at `at`: a squat round seat, its back slumped up behind it (toward the
    /// unit axis `back`). Its own fitting.
    fn beanbag(&mut self, at: Vec2, back: Vec2, r: f32, col: [f32; 3]) {
        self.group += 1;
        self.round_at(at, Vec2::splat(r), (0.0, r * 0.75), col, Shape::Dome);
        let dark = [col[0] * 0.85, col[1] * 0.85, col[2] * 0.85];
        let side = vec2(back.y.abs(), back.x.abs());
        let half = back.abs() * (r * 0.55) + side * (r * 0.85);
        self.round_at(
            at + back * (r * 0.5),
            half,
            (r * 0.1, r * 1.15),
            dark,
            Shape::Ball,
        );
    }

    /// A block round `at` with half-extents `half`, made `shape` (an ellipse if `half`
    /// isn't square).
    fn round_at(&mut self, at: Vec2, half: Vec2, z: (f32, f32), rgb: [f32; 3], shape: Shape) {
        self.push(at - half, at + half, z, rgb, Finish::Plain);
        self.blocks.last_mut().expect("just pushed").shape = shape;
    }

    /// A box on the real room floor `fl`: `a` from its far wall's face, `b` across.
    fn slab(
        &mut self,
        fl: &Floor,
        a: (f32, f32),
        b: (f32, f32),
        z: (f32, f32),
        rgb: [f32; 3],
        finish: Finish,
    ) {
        let (lo, hi) = fl.rect(a, b);
        self.push(lo, hi, z, rgb, finish);
    }

    /// A potted `PLANT` against the wall `wall` (a unit axis) of the room cell holding
    /// floor point `at`, centred along the wall where `at` is.
    fn plant_at(&mut self, at: Vec2, wall: Vec2) {
        let c = Cell {
            x: ((at.x - 1.5) * 0.5).round() as i32,
            y: ((at.y - 1.5) * 0.5).round() as i32,
        };
        let d = (0..4).find(|&d| dir_vec(d) == wall).expect("an axis");
        let axis = if d % 2 == 0 { Vec2::X } else { Vec2::Y };
        let along = (at - square(c) - Vec2::splat(0.5)).dot(axis);
        self.fit(c, d, along, PLANT);
    }

    /// The office swivel chair (`desk`'s): star base, post, seat, backrest, armrests —
    /// centred on `at`, its back toward the unit axis `back`. Its own fitting.
    fn swivel_chair(&mut self, at: Vec2, back: Vec2, cover: [f32; 3]) {
        self.group += 1;
        let side = vec2(-back.y, back.x);
        // (along `back`, across, z) spans.
        type Span = (f32, f32);
        let parts: [(Span, Span, Span, [f32; 3]); 7] = [
            ((-0.09, 0.09), (-0.01, 0.01), (0.0, 0.02), DARK),
            ((-0.01, 0.01), (-0.09, 0.09), (0.0, 0.02), DARK),
            ((-0.01, 0.01), (-0.01, 0.01), (0.02, 0.19), GREY),
            ((-0.09, 0.07), (-0.08, 0.08), (0.19, 0.23), cover),
            ((0.05, 0.09), (-0.08, 0.08), (0.23, 0.44), cover),
            ((-0.07, 0.03), (-0.1, -0.08), (0.23, 0.29), DARK),
            ((-0.07, 0.03), (0.08, 0.1), (0.23, 0.29), DARK),
        ];
        for (a, b, z, rgb) in parts {
            let p = at + back * a.0 + side * b.0;
            let q = at + back * a.1 + side * b.1;
            self.push(p.min(q), p.max(q), z, rgb, Finish::Plain);
        }
    }

    fn dungeon(&mut self, maze: &Maze, lamp_at: &impl Fn(Vec2) -> bool, seed: u64) {
        self.stairwell(maze);
        let mut notices: Vec<&'static [&'static str]> = DUNGEON_NOTICES.to_vec();
        for y in 0..maze.h as i32 {
            for x in 0..maze.w as i32 {
                let c = Cell { x, y };
                if c == maze.start || c == maze.exit {
                    continue;
                }
                let sq = square(c);
                // Walls with nothing on them yet (a torch already claims its wall).
                let walls: Vec<usize> = (0..4)
                    .filter(|&d| {
                        let face = sq + vec2(0.5, 0.5) + dir_vec(d) * (maze.wall_face() - 0.05);
                        maze.edge(c, d) == EdgeKind::Wall && !lamp_at(face)
                    })
                    .collect();
                let mut used = None;
                if !walls.is_empty() && hash(seed, c, 1) < 0.4 {
                    let dir = pick(&walls, hash(seed, c, 2));
                    used = Some(dir);
                    let parts = pick(DUNGEON_WALL, hash(seed, c, 4));
                    if std::ptr::eq(parts, SHACKLES) {
                        self.fit(c, dir, -0.12, parts);
                        self.fit(c, dir, 0.12, parts);
                    } else if std::ptr::eq(parts, RACK) {
                        // Wide: centred, or it would overhang the wall's end.
                        self.fit(c, dir, 0.0, parts);
                    } else {
                        self.fit(c, dir, (hash(seed, c, 3) - 0.5) * 0.3, parts);
                    }
                }
                // Now and then a notice nailed to a bare wall.
                let bare: Vec<usize> = walls.iter().copied().filter(|&d| Some(d) != used).collect();
                if !bare.is_empty() && !notices.is_empty() && hash(seed, c, 9) < 0.07 {
                    let dir = pick(&bare, hash(seed, c, 10));
                    let k = ((hash(seed, c, 11) * notices.len() as f32) as usize)
                        .min(notices.len() - 1);
                    let out = dir_vec(opposite(dir));
                    self.posters.push(Poster {
                        at: sq + vec2(0.5, 0.5) + dir_vec(dir) * self.face() + out * 0.01,
                        out,
                        lines: notices.swap_remove(k),
                        icon: None,
                        z: 0.62,
                        width: 0.32,
                        ink: SEPIA,
                        cap: 0.028,
                        paper: PARCHMENT,
                        glow: false,
                    });
                }
                // A chain hanging from the ceiling in a corner of the cell, clear of
                // the walked lines through its middle.
                if hash(seed, c, 5) < 0.12 {
                    self.group += 1;
                    // Into the corner as far as the walls stand back.
                    let k = self.recess() - 0.02;
                    let corner = vec2(
                        if hash(seed, c, 6) < 0.5 {
                            0.05 - k
                        } else {
                            0.92 + k
                        },
                        if hash(seed, c, 7) < 0.5 {
                            0.05 - k
                        } else {
                            0.92 + k
                        },
                    );
                    let drop = 0.45 + 0.25 * hash(seed, c, 8);
                    let at = sq + corner;
                    self.push(
                        at,
                        at + vec2(0.025, 0.025),
                        (drop, 1.0),
                        IRON,
                        Finish::Links,
                    );
                    self.push(
                        at - vec2(0.01, 0.01),
                        at + vec2(0.035, 0.035),
                        (drop - 0.04, drop),
                        IRON,
                        Finish::Plain,
                    );
                }
            }
        }
    }

    /// Things to read on the walls of cell `c` (a notice, a poster, marker on a
    /// whiteboard), keyed by wall (`DIRS` index) — the bot turns to read them on its
    /// first pass, aimed at the writing (`aim`; level at it when it won't fit whole).
    pub fn reads_at(&self, c: Cell) -> Vec<Glance> {
        let eye = square(c) + vec2(0.5, 0.5);
        let read = |at: Vec2, out: Vec2, half: f32, z: f32| {
            // Step back into the cell the wall bounds (its centre is under a unit from
            // the face, thin walls or not); cell centres sit at 2c + 1.5.
            let p = at + out * 0.3;
            let cell = Cell {
                x: ((p.x - 1.5) * 0.5).round() as i32,
                y: ((p.y - 1.5) * 0.5).round() as i32,
            };
            let key = (0..4).find(|&d| dir_vec(d).dot(-out) > 0.9)?;
            if cell != c {
                return None;
            }
            let run = vec2(-out.y, out.x).abs() * half;
            let (yaw, pitch, stoop) = aim(eye, at - run, at + run, (z - 0.08, z + 0.08))
                .unwrap_or(((at - eye).y.atan2((at - eye).x), 0.0, 0.0));
            Some(Glance {
                key,
                yaw,
                pitch,
                stoop,
            })
        };
        let mut v: Vec<Glance> = self
            .posters
            .iter()
            // A TPS cover sheet on every other corkboard: seen one, seen them all.
            .filter(|p| p.lines != TPS_REPORTS)
            .filter_map(|p| read(p.at, p.out, p.width * 0.5, p.z))
            .chain(
                self.boards
                    .iter()
                    .filter(|b| b.writing != Writing::Notes)
                    .filter_map(|b| read(b.at, b.out, b.half, 0.5)),
            )
            // Garden signs with words on them (not the bare clock faces).
            .chain(
                self.marks
                    .iter()
                    .filter(|m| m.spec.lines.iter().any(|l| !l.is_empty()))
                    .filter_map(|m| read(m.at, m.out, m.width * 0.5, m.z)),
            )
            .collect();
        // The OUT OF ORDER sheet on a washroom door.
        v.extend(
            self.out_of_order_seen
                .filter(|&(hall, _)| hall == c)
                .map(|(_, d)| Glance {
                    key: d,
                    yaw: dir_vec(d).y.atan2(dir_vec(d).x),
                    pitch: 0.0,
                    stoop: 0.0,
                }),
        );
        // One look per wall.
        crate::util::sort_by_key(&mut v, |g| g.key as f32);
        v.dedup_by_key(|g| g.key);
        v
    }

    /// Mirror garden: Wonderland and Looking-Glass country. A checkerboard lawn with a
    /// Red and a White Queen in its corners and the Mad Hatter's tea table (clean dead
    /// ends first), red roses half-painted white and clocks stopped at six set into
    /// hedges, EAT ME / DRINK ME / THIS WAY signs and a mirror-written Jabberwocky,
    /// corner chess statues and croquet flamingos, and a Cheshire Cat grin or two.
    /// Nothing hangs on a mirror, and set pieces and corner figures keep out of any
    /// cell with one (dressing isn't reflected, so a statue beside the glass would give
    /// it away).
    fn garden(&mut self, maze: &Maze, seed: u64) {
        let hedge = |c: Cell, d: usize| maze.edge(c, d) == EdgeKind::Wall;
        let clean = |c: Cell| (0..4).all(|d| !maze.is_mirror(c, d));
        let opens = |c: Cell| (0..4).filter(|&d| maze.is_open(c, d)).count();
        let cells: Vec<Cell> = (0..maze.h as i32)
            .flat_map(|y| (0..maze.w as i32).map(move |x| Cell { x, y }))
            .filter(|&c| c != maze.start && c != maze.exit)
            .collect();

        // Set pieces: dead ends first, then any clean cell, in hashed order.
        let mut spots: Vec<Cell> = cells.iter().copied().filter(|&c| clean(c)).collect();
        // `hash` is in [0, 1), so this orders by `opens` first, then by hash.
        crate::util::sort_by_key(&mut spots, |&c| opens(c) as f32 + hash(seed, c, 100));
        let mut taken: Vec<Cell> = Vec::new();
        let mut tea = false;
        for &c in spots.iter().take(3) {
            let back = (0..4)
                .find(|&d| maze.is_open(c, d))
                .map(opposite)
                .filter(|&d| hedge(c, d))
                .or_else(|| (0..4).find(|&d| hedge(c, d)));
            if taken.len() == 1
                && !tea
                && let Some(back) = back
            {
                self.fit(c, back, 0.0, TEA);
                self.sight(c);
                tea = true;
                taken.push(c);
                continue;
            }
            if taken.len() == 2
                && self.fountain.is_none()
                && let Some(back) = back
            {
                self.fit(c, back, 0.0, FOUNTAIN);
                let at = square(c) + vec2(0.5, 0.5) + dir_vec(back) * (0.5 - FOUNTAIN_OUT);
                self.fountain = Some(at);
                self.sight(c);
                taken.push(c);
                continue;
            }
            self.checkerboard(c);
            self.sight(c);
            let mut queens = [queen(CHESS_RED), queen(IVORY)].into_iter();
            for (a, b) in corners(back) {
                if hedge(c, a) && hedge(c, b) {
                    let Some(q) = queens.next() else { break };
                    self.corner(c, a, b, opposite(a), &q);
                    self.sight(c);
                }
            }
            taken.push(c);
        }

        let mut signs = vec![0usize, 1, 2, 3];
        let mut grins = 0;
        for &c in cells.iter().filter(|c| !taken.contains(c)) {
            let sq = square(c);
            let walls: Vec<usize> = (0..4).filter(|&d| hedge(c, d)).collect();
            if walls.is_empty() {
                continue;
            }
            let mut used = None;
            if hash(seed, c, 101) < 0.26 {
                let dir = pick(&walls, hash(seed, c, 102));
                used = Some(dir);
                if hash(seed, c, 103) < 0.6 {
                    // Roses and clocks are everywhere: not worth stopping for.
                    self.fit(c, dir, 0.0, ROSES);
                } else {
                    let along = (hash(seed, c, 104) - 0.5) * 0.4;
                    self.fit(c, dir, along, CLOCK_CASE);
                    self.mark(c, dir, along, CLOCK_OUT, CLOCK_Z, clock(), (1.0, 0.2));
                }
            }
            let bare: Vec<usize> = walls.iter().copied().filter(|&d| Some(d) != used).collect();
            if !bare.is_empty() && !signs.is_empty() && hash(seed, c, 105) < 0.06 {
                let dir = pick(&bare, hash(seed, c, 106));
                used = Some(dir);
                let k = ((hash(seed, c, 107) * signs.len() as f32) as usize).min(signs.len() - 1);
                let sign = |lines: &[&'static str]| Spec::new(lines, SIGN_INK, Some(SIGN_PLATE));
                match signs.swap_remove(k) {
                    0 => {
                        self.fit(c, dir, 0.0, EAT_ME_SIGN);
                        self.mark(c, dir, 0.0, SIGN_OUT, 0.46, sign(&[EAT_ME]), (0.05, 0.24));
                    }
                    1 => {
                        self.fit(c, dir, 0.0, DRINK_ME_SIGN);
                        self.mark(c, dir, 0.0, SIGN_OUT, 0.46, sign(&[DRINK_ME]), (0.05, 0.26));
                        // Read the sign, then look down at the bottle below it.
                        self.fit(c, dir, 0.0, DRINK_ME_BOTTLE);
                        self.sight(c);
                    }
                    2 => {
                        self.fit(c, dir, 0.0, SIGNPOST);
                        let arrow = |icon| Spec {
                            icon: Some(icon),
                            ..sign(&[THIS_WAY])
                        };
                        let out = 0.151;
                        self.mark(c, dir, -0.1, out, 0.65, arrow(Icon::Left), (0.03, 0.3));
                        self.mark(c, dir, 0.1, out, 0.51, arrow(Icon::Right), (0.03, 0.3));
                    }
                    _ => {
                        self.fit(c, dir, 0.0, JABBER_SIGN);
                        let spec = Spec {
                            mirror: true,
                            ..Spec::new(&JABBERWOCKY, INK, Some(PAPER))
                        };
                        self.mark(c, dir, 0.0, SIGN_OUT, 0.48, spec, (0.03, 0.38));
                    }
                }
            }
            let bare: Vec<usize> = walls.iter().copied().filter(|&d| Some(d) != used).collect();
            if !bare.is_empty() && grins < 2 && hash(seed, c, 108) < 0.03 {
                let dir = pick(&bare, hash(seed, c, 109));
                let out = dir_vec(opposite(dir));
                self.grins.push(Grin {
                    at: sq + vec2(0.5, 0.5) + dir_vec(dir) * 0.5 + out * 0.08,
                    out,
                    phase: hash(seed, c, 110) * std::f32::consts::TAU,
                });
                grins += 1;
                let at = sq + vec2(0.5, 0.5) + dir_vec(dir) * 0.42;
                self.sights.push(Sight {
                    cell: c,
                    min: at - Vec2::splat(0.08),
                    max: at + Vec2::splat(0.08),
                    z: (0.55, 0.65),
                    face: Some(out),
                });
                used = Some(dir);
            }
            // A figure in a corner of an otherwise bare cell, between two hedges.
            if used.is_none() && clean(c) && hash(seed, c, 111) < 0.1 {
                let fits: Vec<(usize, usize)> = corners(None)
                    .into_iter()
                    .filter(|&(a, b)| hedge(c, a) && hedge(c, b))
                    .collect();
                if fits.is_empty() {
                    continue;
                }
                let (a, b) = pick(&fits, hash(seed, c, 112));
                let facing = opposite(if hash(seed, c, 113) < 0.5 { a } else { b });
                let kind = (hash(seed, c, 114) * 5.0) as usize;
                let figure = match kind {
                    0 => knight(IVORY),
                    1 => knight(CHESS_RED),
                    2 => queen(if hash(seed, c, 115) < 0.5 {
                        CHESS_RED
                    } else {
                        IVORY
                    }),
                    _ => FLAMINGO.to_vec(),
                };
                self.corner(c, a, b, facing, &figure);
                self.sight(c);
            }
        }
    }

    /// The fitting just placed (the last `group`) in cell `c`, as a `Sight`.
    fn sight(&mut self, c: Cell) {
        let group = self.group;
        let (mut min, mut max, mut z) = (Vec2::MAX, Vec2::MIN, (f32::MAX, f32::MIN));
        for b in self.blocks.iter().rev().take_while(|b| b.group == group) {
            (min, max) = (min.min(b.min), max.max(b.max));
            z = (z.0.min(b.z0), z.1.max(b.z1));
        }
        self.sights.push(Sight {
            cell: c,
            min,
            max,
            z,
            face: None,
        });
    }

    /// Dungeon: an iron chain (a hanging one, shackles) within `r` of floor point `p`.
    /// `r` as among full-square walls: it reaches as far again as thin ones stand back.
    pub fn chain_near(&self, p: Vec2, r: f32) -> bool {
        let r = r + self.recess();
        self.blocks.iter().any(|b| {
            b.finish == Finish::Links && b.rgb == IRON && ((b.min + b.max) * 0.5).distance(p) < r
        })
    }

    /// Garden things to turn and look at from cell `c`: every sight in it or in the
    /// cells next door through edges `open` says are open, that fits the view from here
    /// (`aim`) — so a set piece too big to take in from inside its own cell is looked
    /// at from the doorway, and a grin only from in front. Empty outside the Garden.
    pub fn looks_at(&self, c: Cell, open: impl Fn(usize) -> bool) -> Vec<Glance> {
        let eye = square(c) + vec2(0.5, 0.5);
        let near: Vec<Cell> = (0..4).filter(|&d| open(d)).map(|d| c.step(d)).collect();
        self.sights
            .iter()
            .enumerate()
            .filter(|(_, s)| s.cell == c || near.contains(&s.cell))
            .filter(|(_, s)| {
                s.face
                    .is_none_or(|f| f.dot((eye - (s.min + s.max) * 0.5).normalize()) > 0.5)
            })
            .filter_map(|(key, s)| {
                let (yaw, pitch, stoop) = aim(eye, s.min, s.max, s.z)?;
                Some(Glance {
                    key,
                    yaw,
                    pitch,
                    stoop,
                })
            })
            .collect()
    }

    /// A Looking-Glass chessboard laid in the grass of cell `c`: 6×6 flat tiles.
    fn checkerboard(&mut self, c: Cell) {
        self.group += 1;
        let (lo, n) = (0.08, 6);
        let step = (1.0 - 2.0 * lo) / n as f32;
        let sq = square(c);
        for i in 0..n {
            for j in 0..n {
                let min = sq + vec2(lo + i as f32 * step, lo + j as f32 * step);
                let rgb = if (i + j) % 2 == 0 {
                    TILE_LIGHT
                } else {
                    TILE_DARK
                };
                self.push(
                    min,
                    min + Vec2::splat(step),
                    (0.0, 0.003),
                    rgb,
                    Finish::Plain,
                );
            }
        }
    }

    /// A figure standing in cell `c`'s corner between walls `a` and `b`, facing
    /// `facing`, built from `layers`.
    fn corner(&mut self, c: Cell, a: usize, b: usize, facing: usize, layers: &[Layer]) {
        self.group += 1;
        let at = square(c) + vec2(0.5, 0.5) + (dir_vec(a) + dir_vec(b)) * (0.5 - CORNER);
        let fwd = dir_vec(facing);
        let side = vec2(-fwd.y, fwd.x);
        for ly in layers {
            let ((f, s), (hf, hs)) = (ly.at, ly.half);
            debug_assert!(f.abs() + hf <= 0.1 && s.abs() + hs <= 0.1);
            let mid = at + fwd * f + side * s;
            let half = if fwd.x != 0.0 {
                vec2(hf, hs)
            } else {
                vec2(hs, hf)
            };
            self.push(mid - half, mid + half, ly.z, ly.rgb, ly.finish);
            self.blocks.last_mut().expect("just pushed").shape = ly.shape;
        }
    }

    /// A label on wall `dir` of cell `c`, `along` it, `out` from its face, centred at
    /// height `z`; `(cap, width)` size it as `Label::size` does.
    #[allow(clippy::too_many_arguments)]
    fn mark(
        &mut self,
        c: Cell,
        dir: usize,
        along: f32,
        out: f32,
        z: f32,
        spec: Spec,
        size: (f32, f32),
    ) {
        let o = dir_vec(opposite(dir));
        let run = if dir.is_multiple_of(2) {
            vec2(1.0, 0.0)
        } else {
            vec2(0.0, 1.0)
        };
        self.marks.push(Mark {
            at: square(c) + vec2(0.5, 0.5) + dir_vec(dir) * 0.5 + o * out + run * along,
            out: o,
            spec,
            z,
            cap: size.0,
            width: size.1,
        });
    }

    /// Where the garden's fountain stands (grid coords, the same space as the camera's
    /// position), for its trickle to be heard from. `None` outside the Garden or when no
    /// set-piece cell was left for it.
    pub fn fountain(&self) -> Option<Vec2> {
        self.fountain
    }

    /// The nearest dressing per screen column from the last `draw` — for what's drawn
    /// after it (`Fx::draw_lamps`) to hide behind.
    pub fn front(&self) -> &Occluders {
        &self.front
    }

    /// The same, for what's drawn after the dressing to add itself (the keys), so what
    /// comes later still (the lamps) hides behind it too.
    pub fn front_mut(&mut self) -> &mut Occluders {
        &mut self.front
    }

    /// Draw every box in range, then the overlays. `panels`/`swing` place the door
    /// signs on the doors as they stand this frame.
    pub fn draw(
        &mut self,
        view: &View,
        pal: &Palette,
        light: &Lighting,
        panels: &[Panel],
        swing: &[f32],
        dynamic: Dynamic,
    ) {
        if self.blocks.is_empty() {
            return;
        }
        if let Some(l) = &self.lift {
            let k = dynamic.lift.clamp(0.0, 1.0) * 0.48;
            // The doorway: both leaves shut. A leaf slides into the wall beside it, so
            // it's cut to the doorway as it goes — whole, it slid out over the wall.
            let (lo, hi) = l
                .leaves
                .iter()
                .fold((Vec2::MAX, Vec2::MIN), |(lo, hi), &(_, a, b, _)| {
                    (lo.min(a), hi.max(b))
                });
            for &(i, min, max, slide) in &l.leaves {
                let (a, b) = (min + slide * k, max + slide * k);
                let axis = slide.abs();
                let (a, b) = (
                    a * (Vec2::ONE - axis) + a.max(lo) * axis,
                    b * (Vec2::ONE - axis) + b.min(hi) * axis,
                );
                self.blocks[i].min = a;
                self.blocks[i].max = b.max(a);
            }
        }
        for &(i, base, motion) in &self.moves {
            self.blocks[i as usize] = moved(base, motion, dynamic);
        }
        if let Some((a, b)) = self.fire {
            for (i, &(min, max)) in (a..b).zip(&self.fire_base) {
                // No fire ground this frame: park the blocks out of range.
                let at = dynamic.fire.unwrap_or(Vec2::splat(-1.0e4));
                self.blocks[i].min = at + min;
                self.blocks[i].max = at + max;
            }
        }
        let (mut cols, mut front) = (take(&mut self.cols), take(&mut self.front));
        self.render(
            view, pal, light, panels, swing, dynamic, &mut cols, &mut front,
        );
        (self.cols, self.front) = (cols, front);
    }

    /// The dressing as seen in a mirror: `view` is the glass's mirrored camera handed to
    /// `raycast::Props::reflect`, whose `visible` keeps everything to the glass's columns
    /// and to depths between the glass and what lies behind it. Same boxes and overlays
    /// as `draw` (last frame's lift/fire placement), tinted like the glass; any writing
    /// comes out mirror-reversed.
    pub fn draw_reflection(&self, view: &View, pal: &Palette, light: &Lighting, dynamic: Dynamic) {
        if self.blocks.is_empty() {
            return;
        }
        let (mut cols, mut front) = (Vec::new(), Occluders::default());
        self.render(view, pal, light, &[], &[], dynamic, &mut cols, &mut front);
    }

    /// `draw`'s body, with the per-column scratch passed in.
    #[allow(clippy::too_many_arguments)]
    fn render(
        &self,
        view: &View,
        pal: &Palette,
        light: &Lighting,
        panels: &[Panel],
        swing: &[f32],
        dynamic: Dynamic,
        cols: &mut Vec<Vec<Hit>>,
        front: &mut Occluders,
    ) {
        let area = view.area;
        let ncols = (area.w / COL_STEP).ceil() as usize + 1;
        cols.resize_with(ncols, Vec::new);
        for c in cols.iter_mut() {
            c.clear();
        }
        front.0.resize_with(ncols, Vec::new);
        for c in front.0.iter_mut() {
            c.clear();
        }
        // A doorway's head hides what lies past it above the opening.
        for (c, &(d, y)) in front.0.iter_mut().zip(view.roofs()) {
            if d.is_finite() {
                c.push((d, f32::NEG_INFINITY, y));
            }
        }
        // An open leaf standing in front hides what lies past it below its top.
        for (c, &(d, y)) in front.0.iter_mut().zip(view.unders()) {
            if d.is_finite() {
                c.push((d, y, f32::INFINITY));
            }
        }
        let (lo, hi) = view.span();

        // Gather: every block's hits, bucketed by screen column. A mirror's camera
        // visits only the blocks inside its glass's view wedge.
        let wedge = view.window().map(|(near, far)| {
            let far = far.min(RANGE + 1.0);
            let near = near.min(far);
            let (ra, rb) = (view.ray(lo), view.ray(hi));
            let pts = [ra * near, ra * far, rb * near, rb * far].map(|r| view.pos + r);
            let a = pts.iter().fold(Vec2::MAX, |m, &p| m.min(p));
            let b = pts.iter().fold(Vec2::MIN, |m, &p| m.max(p));
            self.buckets.within(a, b)
        });
        // The real camera: whole fittings wholly behind the walls in every column they
        // could cover are skipped before any per-column ray test.
        let seen = || {
            self.fittings
                .iter()
                .filter(|g| g.moving || !hidden(view, g.min, g.max, lo, hi))
                .flat_map(|g| g.start..g.end)
        };
        let visit: &mut dyn Iterator<Item = u32> = match &wedge {
            Some(list) => &mut list.iter().copied(),
            None => &mut seen(),
        };
        for bi in visit {
            let (bi, b) = (bi as usize, &self.blocks[bi as usize]);
            if ((b.min + b.max) * 0.5).distance(view.pos) > RANGE {
                continue;
            }
            let Some((x0, x1, _)) = columns(view, b.min, b.max) else {
                continue;
            };
            let (x0, x1) = (x0.max(lo), x1.min(hi - 1.0));
            let mut x = x0;
            while x <= x1 {
                let ci = ((x - area.x) / COL_STEP).round() as usize;
                let ray = view.ray(x + COL_STEP * 0.5);
                let bi32 = bi as u32;
                let shows = if (self.flight.0..self.flight.1).contains(&bi32) {
                    view.through_portal(x)
                } else {
                    (self.stairs.0..self.stairs.1).contains(&bi32)
                        || !view.behind_portal((b.min + b.max) * 0.5, x)
                };
                if ci < ncols
                    && shows
                    && let Some((tn, tf, normal, miss)) = hit(view.pos, ray, b)
                    && view.visible(x, tn)
                {
                    // Flat on the ground: under everything standing, so painted first.
                    let flat = if b.flat() { 1.0e6 } else { 0.0 };
                    cols[ci].push(Hit {
                        key: tn + flat,
                        tn,
                        tf,
                        normal,
                        miss,
                        block: bi as u32,
                        clip: f32::INFINITY,
                    });
                }
                x += COL_STEP;
            }
        }

        // Paint each column far to near by entry depth — except where one block stands
        // on (or sinks into) another's top, or hangs under its underside, on the eye's
        // side of it: it can hide that block's top/underside and never the reverse, so
        // it paints after it (see `stacked`); likewise a feature on a round block's face
        // (`on_face`). See `paint_order`.
        let eye = (view.screen_y(0.0, 1.0) - view.horizon) / area.h;
        let (mut order, mut after) = (Vec::new(), Vec::new());
        for (ci, hits) in cols.iter_mut().enumerate() {
            if hits.is_empty() {
                continue;
            }
            let ray = (
                view.pos,
                view.ray(area.x + ci as f32 * COL_STEP + COL_STEP * 0.5),
            );
            paint_order(
                &self.blocks,
                hits,
                eye,
                ray,
                (&mut order, &mut after),
                |z, d| view.screen_y(z, d),
            );
            let x = area.x + ci as f32 * COL_STEP;
            // Past the stairwell's arch the exit cell's floor hides what's below it.
            let floor_edge = self.well.and_then(|(axis, plane, into)| {
                let (p, r) = (view.pos[axis], view.ray(x + COL_STEP * 0.5)[axis]);
                ((plane - p) * into > 0.0 && r * into > 0.0)
                    .then(|| view.screen_y(0.0, (plane - p) / r))
            });
            let clip_of = |b: &Block, h: &Hit| match floor_edge {
                Some(y) if b.z0 < -0.01 => y.min(h.clip),
                _ => h.clip,
            };
            for h in hits.iter() {
                let b = &self.blocks[h.block as usize];
                paint(view, pal, light, b, h, x, dynamic.t, clip_of(b, h));
                if !b.flat() {
                    // Its screen span in this column, over the near and far side.
                    let ys = [(b.z1, h.tn), (b.z1, h.tf), (b.z0, h.tn), (b.z0, h.tf)]
                        .map(|(z, d)| view.screen_y(z, d.clamp(0.05, RANGE * 2.0)));
                    let top = ys.iter().copied().fold(f32::INFINITY, f32::min);
                    let bot = ys.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    front.0[ci].push((h.tn, top, bot));
                }
            }
        }
        let front = &*front;

        // Labels fade into the distance as the walls do.
        let fade = |at: Vec2| label::Fade {
            haze: pal.haze,
            light: light.at(at),
            fog: pal.fog,
        };
        for board in &self.boards {
            draw_board(view, front, fade(board.at), board);
        }
        for (di, spec) in &self.signs {
            if let Some(p) = panels.get(*di) {
                let s = swing.get(*di).copied().unwrap_or(0.0);
                door_sign(
                    view,
                    front,
                    fade(p.hinge),
                    p,
                    s,
                    spec,
                    self.out_of_order == Some(*di),
                );
            }
        }
        for p in &self.posters {
            draw_poster(view, front, fade(p.at), p);
        }
        for m in &self.marks {
            draw_mark(view, front, fade(m.at), m);
        }
        for g in &self.grins {
            draw_grin(view, front, g, dynamic.t);
        }
        if let Some(at) = dynamic.fire {
            draw_fire(view, at, dynamic.t, pal);
        }
    }
}

/// A lava lamp's glass: radius (as a fraction of its widest) at seven evenly spaced
/// heights, bottom to top — narrow foot, a bulge low down, a long taper to the cap.
const LAVA_GLASS: [f32; 7] = [0.62, 0.9, 1.0, 0.95, 0.84, 0.72, 0.6];

fn lava_radius(u: f32) -> f32 {
    let x = u.clamp(0.0, 1.0) * 6.0;
    let i = (x.floor() as usize).min(5);
    LAVA_GLASS[i] + (LAVA_GLASS[i + 1] - LAVA_GLASS[i]) * (x - i as f32)
}

/// `base` as `motion` places it this frame.
fn moved(base: Block, motion: Motion, dynamic: Dynamic) -> Block {
    let t = dynamic.t;
    let arguing = dynamic.boss_hp > 0.0 && dynamic.boss_hp < 1.0;
    let beaten = dynamic.boss_hp <= 0.0;
    let shift = |b: Block, d: Vec2, dz: f32| Block {
        min: b.min + d,
        max: b.max + d,
        z0: b.z0 + dz,
        z1: b.z1 + dz,
        ..b
    };
    match motion {
        Motion::Head { across } => {
            if arguing {
                // Shaking his head at every word, a little off the beat.
                let k = (t * 7.0).sin() * 0.7 + (t * 11.3).sin() * 0.3;
                shift(base, across * 0.009 * k, 0.004 * (t * 3.1).sin().abs())
            } else if beaten {
                shift(base, Vec2::ZERO, -0.018)
            } else {
                shift(base, across * 0.003 * (t * 0.7).sin(), 0.0)
            }
        }
        Motion::Fist => {
            let lift = if arguing {
                // Up slowly, down hard.
                let s = (t * 1.6).fract();
                0.05 * if s < 0.8 {
                    s / 0.8
                } else {
                    1.0 - (s - 0.8) / 0.2
                }
            } else {
                0.0
            };
            shift(base, Vec2::ZERO, lift)
        }
        Motion::Blob {
            axis,
            lo,
            hi,
            phase,
        } => {
            let glass = (base.max.x - base.min.x) * 0.5;
            let period = 11.0 + 7.0 * phase;
            let s = (t / period + phase).fract() * std::f32::consts::TAU;
            // Rests in the pool, rises, lingers under the cap, sinks.
            let u = 0.5 - 0.5 * s.cos();
            let v = s.sin().abs();
            let r = 0.55 * glass * (1.0 - 0.3 * v);
            let hz = (0.8 * glass * (1.0 + 0.7 * v)).min((hi - lo) * 0.2);
            let zc = lo + hz + (hi - lo - 2.0 * hz) * u;
            let r = r.min(0.8 * glass * lava_radius((zc - lo) / (hi - lo)));
            let drift = vec2(
                (t * 0.37 + phase * 17.0).sin(),
                (t * 0.29 + phase * 9.0).cos(),
            ) * (glass - r).max(0.0)
                * 0.35;
            let c = axis + drift;
            Block {
                min: c - Vec2::splat(r),
                max: c + Vec2::splat(r),
                z0: zc - hz,
                z1: zc + hz,
                ..base
            }
        }
    }
}

/// Sorts one column's `hits` into paint order: far to near by entry depth, except
/// that what must paint after a nearer block — `stacked` on it (clipped where it's sunk
/// into it, through `screen_y(z, depth)`), a feature `on_face` of a round one, or
/// stacked in turn on one of those — moves up to just after it, in its order.
///
/// It used to go the other way: the nearer block was inserted before the first block
/// stacked on it, which also put it under everything placed in between — farther by
/// entry, so meant to be under it. A brow proud of the boss's face (and so "over" the
/// chin below it) went in before the face, which painted over it.
/// `ray`: the column's eye and direction; `order`, `after`: reusable buffers.
fn paint_order(
    blocks: &[Block],
    hits: &mut Vec<Hit>,
    eye: f32,
    ray: (Vec2, Vec2),
    (order, after): (&mut Vec<Hit>, &mut Vec<Hit>),
    screen_y: impl Fn(f32, f32) -> f32,
) {
    crate::util::sort_by_key(hits, |h| -h.key);
    order.clear();
    for h in hits.iter() {
        let a = &blocks[h.block as usize];
        let curved = |b: &Block| matches!(b.shape, Shape::Ball | Shape::Dome | Shape::Bowl);
        let round = curved(a);
        // Split what's placed into what stays under it and what goes after it, both
        // in order (`order` keeps the first, compacted in place).
        after.clear();
        let mut kept = 0;
        for k in 0..order.len() {
            let mut x = order[k];
            let xb = &blocks[x.block as usize];
            let mut follows = match stacked(a, h, xb, &x, eye) {
                Some(clip) => {
                    if let Some(z) = clip {
                        x.clip = x.clip.min(screen_y(z, x.tn.max(0.05)));
                    }
                    true
                }
                None => round && on_face(a, h, xb, &x, ray),
            };
            // What's stacked on a follower follows too, and so does what was placed
            // over one in front of it — left behind, the move put the follower on top
            // (an arcade cabinet's body, following its base, over its own screen).
            // Not for round blocks: their entry is their widest girth, not their front.
            follows = follows
                || after.iter().any(|y| {
                    let yb = &blocks[y.block as usize];
                    (x.tn < y.tn
                        && !curved(xb)
                        && !curved(yb)
                        && in_sight_together(xb, &x, yb, y, eye))
                        || stacked(yb, y, xb, &x, eye).is_some()
                });
            if follows {
                after.push(x);
            } else {
                order[kept] = x;
                kept += 1;
            }
        }
        order.truncate(kept);
        order.push(*h);
        order.extend_from_slice(after);
    }
    hits.clear();
    hits.extend_from_slice(order);
}

/// Whether `a` and `b` can overlap on screen in this column: the heights they span,
/// seen over the stretch of the ray each occupies, as slopes from the eye.
fn in_sight_together(a: &Block, ha: &Hit, b: &Block, hb: &Hit, eye: f32) -> bool {
    let range = |x: &Block, h: &Hit| {
        let (tn, tf) = (h.tn.max(0.05), h.tf.max(0.05));
        let (lo, hi) = (x.z0 - eye, x.z1 - eye);
        ((lo / tn).min(lo / tf), (hi / tn).max(hi / tf))
    };
    let (a, b) = (range(a, ha), range(b, hb));
    a.0 < b.1 && b.0 < a.1
}

/// Whether `x` (hit `hx`), entered no nearer than `a` (hit `ha`) in this column, must
/// still paint after it: `x` starts within `a`'s span along the ray and lies on the
/// eye's side of `a`'s top (standing on it, the eye above) or underside (hanging under
/// it, the eye below) — then `x` can hide part of `a`'s top/underside and `a` can never
/// hide `x`. Also a block sunk into `a`'s top and rising above it, wholly inside `a`'s
/// span (a mop handle in its bucket): painted after, clipped at `a`'s top (`Some(z)`).
/// Entry order alone put every block standing on a support at the support's depth,
/// all in a tie, so a lamp and a radio side by side on a shelf painted in list order,
/// whichever of them was nearer.
fn stacked(a: &Block, ha: &Hit, x: &Block, hx: &Hit, eye: f32) -> Option<Option<f32>> {
    const EPS: f32 = 0.004;
    if a.flat() || x.flat() || hx.tn < ha.tn || hx.tn >= ha.tf {
        return None;
    }
    // `x` must also reach a line of sight to `a`'s top/underside: at `x`'s depth those
    // run no farther from the eye than `reach` of the way to `a`'s face. Without that,
    // a nose "stood on" the boss's shirt front and a cigar "hung over" his shoulders,
    // dragging his suit over the shirt in dark stripes.
    let reach = hx.tn / ha.tf.max(1e-6);
    if eye >= a.z1 && x.z0 >= a.z1 - EPS && x.z0 < eye + (a.z1 - eye) * reach {
        return Some(None);
    }
    if eye <= a.z0 && x.z1 <= a.z0 + EPS && x.z1 > eye + (a.z0 - eye) * reach {
        return Some(None);
    }
    // (Not in glass: what's inside it shows through.)
    let sunk = x.z0 >= a.z0 - EPS && x.z0 < a.z1 - EPS && x.z1 > a.z1 + EPS;
    (!a.clear() && eye > a.z1 && sunk && hx.tf <= ha.tf + EPS).then_some(Some(a.z1))
}

/// Whether `x` (hit `hx`), entered no nearer than round `a` (hit `ha`) in this column,
/// is a feature on `a`'s curved face — short (≤ 0.3 of `a`'s height, within it) and
/// entered in front of `a`'s cross-section at its own mid-height (a brow, an eye, the
/// nose on the boss's head) — and so paints right after it. A round block's entry is
/// its widest girth, so a feature proud of the face above or below that girth still
/// entered a few mm behind it, and the head shaking swapped the order from frame to
/// frame (brows, nose and eyes flickering in and out). Only for short features: for
/// two big intersecting balls (hair round the head) one mid-height says nothing.
fn on_face(a: &Block, ha: &Hit, x: &Block, hx: &Hit, (o, r): (Vec2, Vec2)) -> bool {
    !x.flat()
        && hx.tn >= ha.tn
        && hx.tn < ha.tf
        && x.z0 >= a.z0
        && x.z1 <= a.z1
        && x.z1 - x.z0 <= 0.3 * (a.z1 - a.z0)
        && girth_at(a, (x.z0 + x.z1) * 0.5, o, r).is_some_and(|tn| hx.tn <= tn)
}

/// Where the ray `o + r·t` enters round block `b`'s horizontal cross-section at
/// height `z`: `None` for a box or cylinder (their entry already holds at every
/// height), outside the shape's heights, or where the ray misses that cross-section.
fn girth_at(b: &Block, z: f32, o: Vec2, r: Vec2) -> Option<f32> {
    let (zc, hz) = match b.shape {
        Shape::Box | Shape::Cyl => return None,
        Shape::Ball => ((b.z0 + b.z1) * 0.5, (b.z1 - b.z0) * 0.5),
        Shape::Dome if z >= b.z0 => (b.z0, b.z1 - b.z0),
        Shape::Bowl if z <= b.z1 => (b.z1, b.z1 - b.z0),
        Shape::Dome | Shape::Bowl => return None,
    };
    let u = (z - zc) / hz.max(1e-6);
    if u.abs() >= 1.0 {
        return None;
    }
    let (c, h) = (
        (b.min + b.max) * 0.5,
        (b.max - b.min) * 0.5 * (1.0 - u * u).sqrt(),
    );
    ellipse(o, r, c - h, c + h).map(|(tn, ..)| tn)
}

/// Column-aligned screen x range footprint bounds `min..max` can cover, and the
/// nearest view depth in them — the footprint clipped to what lies past `slab`'s and
/// `ellipse`'s 0.05 near cut, so a box beside or behind the camera no longer claims
/// the whole view. `None` if off screen or wholly behind.
fn columns(view: &View, min: Vec2, max: Vec2) -> Option<(f32, f32, f32)> {
    const NEAR: f32 = 0.05;
    let area = view.area;
    let dir = view.ray(area.x + area.w * 0.5);
    let plane = view.ray(area.x + area.w) - dir;
    let corners = [min, vec2(max.x, min.y), max, vec2(min.x, max.y)];
    let depths = corners.map(|c| view.depth(c));
    let (mut x0, mut x1, mut near) = (f32::INFINITY, f32::NEG_INFINITY, f32::INFINITY);
    let mut take = |p: Vec2, d: f32| {
        let u = (p - view.pos).dot(plane) / (plane.length_squared() * d);
        let x = area.x + (u + 1.0) * area.w * 0.5;
        (x0, x1, near) = (x0.min(x), x1.max(x), near.min(d));
    };
    for i in 0..4 {
        let (a, b) = (depths[i], depths[(i + 1) % 4]);
        if a >= NEAR {
            take(corners[i], a);
        }
        if (a - NEAR) * (b - NEAR) < 0.0 {
            let k = (NEAR - a) / (b - a);
            take(corners[i].lerp(corners[(i + 1) % 4], k), NEAR);
        }
    }
    if x1 < area.x || x0 > area.x + area.w {
        return None;
    }
    let x0 = area.x + ((x0.max(area.x) - area.x) / COL_STEP).floor() * COL_STEP;
    Some((x0, x1.min(area.x + area.w - 1.0), near))
}

/// One column of one box: its top (or underside), then the face it's entered by.
/// `clip`: nothing below this screen y (a sunk block behind the floor's edge).
#[allow(clippy::too_many_arguments)]
fn paint(
    view: &View,
    pal: &Palette,
    light: &Lighting,
    b: &Block,
    h: &Hit,
    x: f32,
    t: f32,
    clip: f32,
) {
    let area = view.area;
    let w = COL_STEP + 0.6;
    let lit = lit_at(h.tn, pal.haze, light.at((b.min + b.max) * 0.5));
    let wash = fog_at(h.tn, pal.haze);
    let g = view.glass_tint();
    let glass = b.finish == Finish::Glass;
    // See-through: thin face-on, thicker toward the silhouette.
    let lava = b.finish == Finish::Lava;
    let alpha = if glass {
        0.35 + 0.55 * h.miss * h.miss
    } else if lava {
        0.3 + 0.55 * h.miss * h.miss
    } else {
        1.0
    };
    let shade = |rgb: [f32; 3], k: f32| {
        Color::new(
            rgb[0] * g[0] * k * lit + pal.fog.r * wash,
            rgb[1] * g[1] * k * lit + pal.fog.g * wash,
            rgb[2] * g[2] * k * lit + pal.fog.b * wash,
            alpha,
        )
    };
    // Seen through a doorway, nothing shows above its head.
    let roof = view.roof(x, h.tn).max(area.y);
    // Behind an open leaf, nothing below its top.
    let clip = clip.min(view.floor_clip(x, h.tn));
    let span = |ya: f32, yb: f32, col: Color| {
        let (ya, yb) = (ya.max(roof), yb.min(area.y + area.h).min(clip));
        if yb > ya {
            draw_rectangle(x, ya, w, yb - ya, col);
        }
    };
    let bar =
        |za: f32, zb: f32, col: Color| span(view.screen_y(zb, h.tn), view.screen_y(za, h.tn), col);
    let lamp = b.finish == Finish::Lamp;
    let flat = |k: f32| {
        if lamp {
            view.tint(Color::new(b.rgb[0], b.rgb[1], b.rgb[2], 1.0))
        } else {
            shade(b.rgb, k)
        }
    };
    // A round surface is lit face-on and darkens toward its silhouette; `across` is
    // where on it this column is, -1..1 left to right (the glass highlight's place).
    let rn = view.ray(x + COL_STEP * 0.5).normalize_or_zero();
    let face = (-h.normal.dot(rn)).max(0.0);
    let across = h.normal.perp_dot(rn);
    let round_k = 0.5 + 0.5 * face;
    let streak = |za: f32, zb: f32| {
        if glass && (0.3..0.55).contains(&across) {
            let c = view.tint(Color::new(1.0, 1.0, 1.0, 0.45 * lit));
            span(view.screen_y(zb, h.tn), view.screen_y(za, h.tn), c);
        }
    };
    if matches!(b.shape, Shape::Ball | Shape::Dome | Shape::Bowl) {
        // The column's vertical plane cuts the ellipsoid in an ellipse (centre `tm`,
        // half-length `ta` along the ray, half-height `za` at height `zc`); its screen
        // extent runs between the two lines of sight from the eye that graze it —
        // or, for a half, the flat face's ends where the grazing point is cut away.
        let eye = (view.screen_y(0.0, 1.0) - view.horizon) / area.h;
        let (tm, ta) = ((h.tn + h.tf) * 0.5, (h.tf - h.tn) * 0.5);
        let cut = (1.0 - h.miss * h.miss).max(0.0).sqrt();
        let (zc, za, keep) = match b.shape {
            Shape::Dome => (b.z0, b.z1 - b.z0, 1.0),
            Shape::Bowl => (b.z1, b.z1 - b.z0, -1.0),
            _ => ((b.z0 + b.z1) * 0.5, (b.z1 - b.z0) * 0.5, 0.0),
        };
        let za = za * cut;
        if za < 1e-4 || ta < 1e-5 {
            return;
        }
        let (mut top, mut bot) = (f32::INFINITY, f32::NEG_INFINITY);
        let mut take = |t: f32, z: f32| {
            let y = view.screen_y(z, t);
            (top, bot) = (top.min(y), bot.max(y));
        };
        // The eye in the ellipse's unit-circle frame; tangent points T satisfy T·P = 1.
        let pe = vec2(-tm / ta, (eye - zc) / za);
        let pp = pe.length_squared();
        if pp > 1.0 {
            let k = (pp - 1.0).sqrt();
            for sg in [1.0, -1.0] {
                let tp = (pe + vec2(-pe.y, pe.x) * (k * sg)) / pp;
                if keep == 0.0 || tp.y * keep >= 0.0 {
                    take(tm + ta * tp.x, zc + za * tp.y);
                }
            }
        }
        if keep != 0.0 {
            take(h.tn, zc);
            take(h.tf, zc);
        }
        if bot <= top {
            return;
        }
        let k = if lamp { 1.0 } else { round_k };
        span(top, bot, flat(k));
        let hgt = bot - top;
        match b.finish {
            Finish::Lamp => {}
            Finish::Bands(step) => {
                // Bristles / hoops: dark rings across it.
                let n = ((b.z1 - b.z0) / step).round().max(1.0);
                let dark = shade(b.rgb, round_k * 0.45);
                let mut i = 1.0;
                while i < n {
                    let y = top + hgt * i / n;
                    span(y - 0.8, y + 0.8, dark);
                    i += 1.0;
                }
            }
            _ => {
                // Light from overhead: bright at the crown shading down to a dark
                // underside, in a few steps (tall shapes get more, so no step shows).
                let (k0, k1) = match b.shape {
                    Shape::Dome => (1.15, 0.9),
                    Shape::Bowl => (0.95, 0.65),
                    _ => (1.15, 0.66),
                };
                let leafy = b.finish == Finish::Leafy;
                let n = if leafy {
                    (hgt / 4.0).clamp(1.0, 14.0).floor()
                } else {
                    (hgt / 5.0).clamp(1.0, 12.0).floor()
                };
                // Leaf patches are fixed to the surface (the hit point), not the screen.
                let q = ((view.pos + rn * h.tn) * 110.0).floor();
                let mut i = 0.0;
                while i < n {
                    let mut k = k0 + (k1 - k0) * (i + 0.5) / n;
                    if leafy {
                        let v = (q.x * 12.9898 + q.y * 78.233 + (i * 14.0 / n).floor() * 37.719)
                            .sin()
                            * 43_758.547;
                        k *= 0.78 + 0.36 * v.fract().abs();
                    }
                    let (ya, yb) = (top + hgt * i / n, top + hgt * (i + 1.0) / n);
                    span(ya, yb + 0.5, shade(b.rgb, round_k * k));
                    i += 1.0;
                }
            }
        }
        // A flat face: a bowl's rim seen from above shows its dark inside, a dome's
        // base from below its shadowed bottom.
        if !glass {
            let (ya, yb) = (view.screen_y(zc, h.tf), view.screen_y(zc, h.tn));
            if keep < 0.0 && yb > ya {
                span(ya, yb, shade(b.rgb, 0.4));
            }
            if keep > 0.0 && ya > yb && b.z0 > 0.0 {
                span(yb, ya, shade(b.rgb, 0.5));
            }
        }
        if glass && (0.3..0.55).contains(&across) {
            let c = view.tint(Color::new(1.0, 1.0, 1.0, 0.45 * lit));
            span(top + hgt * 0.15, bot - hgt * 0.15, c);
        }
        return;
    }
    // Top (the eye is above it) or underside (something hanging above eye height).
    let (ya, yb) = (view.screen_y(b.z1, h.tf), view.screen_y(b.z1, h.tn));
    if yb > ya && !lava {
        span(ya, yb, flat(1.08));
    }
    let (ya, yb) = (view.screen_y(b.z0, h.tn), view.screen_y(b.z0, h.tf));
    if yb > ya && b.z0 > 0.0 && !lava {
        span(ya, yb, flat(0.5));
    }
    let side = match b.shape {
        Shape::Box if h.normal.x != 0.0 => 0.8,
        Shape::Box => 0.95,
        _ => round_k,
    };
    let facing = |d: usize| h.normal == dir_vec(d);
    let glow = |c: [f32; 3]| {
        let k = 0.55 + 0.45 * lit;
        view.tint(Color::new(c[0] * k, c[1] * k, c[2] * k, 1.0))
    };
    match b.finish {
        Finish::Screen(d, col) if facing(d) => {
            bar(b.z0, b.z1, shade(b.rgb, side));
            // The lit panel, inset in its bezel; it gives off its own light.
            let m = (b.z1 - b.z0) * 0.1;
            bar(b.z0 + m, b.z1 - m, glow(col));
        }
        Finish::Chart(d) if facing(d) => {
            bar(b.z0, b.z1, shade(b.rgb, side));
            let m = (b.z1 - b.z0) * 0.08;
            bar(b.z0 + m, b.z1 - m, glow([0.1, 0.16, 0.3]));
            // Where across the screen this column is: five bars, each taller than the
            // one before (bar three dips — nobody talks about Q3).
            let hit = view.pos + view.ray(x + COL_STEP * 0.5) * h.tn;
            let u = if h.normal.x != 0.0 {
                (hit.y - b.min.y) / (b.max.y - b.min.y)
            } else {
                (hit.x - b.min.x) / (b.max.x - b.min.x)
            };
            let slot = u * 6.0 - 0.5;
            let i = slot.floor();
            if (0.0..5.0).contains(&i) && slot.fract() > 0.2 {
                let height = [0.2, 0.35, 0.3, 0.6, 0.95][i as usize];
                let (lo, hi) = (b.z0 + 2.0 * m, b.z1 - 2.0 * m);
                let col = if i as usize == 4 {
                    [0.35, 0.95, 0.45]
                } else {
                    [0.4, 0.7, 1.0]
                };
                bar(lo, lo + (hi - lo) * height, glow(col));
            }
        }
        Finish::Lamp => bar(b.z0, b.z1, flat(1.0)),
        Finish::Lava => {
            // Glows from inside: brightest face-on, dimmer toward the silhouette.
            let k = 0.7 + 0.3 * face;
            let c = view.tint(Color::new(b.rgb[0] * k, b.rgb[1] * k, b.rgb[2] * k, alpha));
            bar(b.z0, b.z1, c);
        }
        Finish::Blinds(step) => {
            // Daylight between the slats; the slats themselves lit a little by it.
            let day = glow([0.86, 0.9, 0.95]);
            let slat = shade(b.rgb, 0.85);
            bar(b.z0, b.z1, day);
            let mut z = b.z0;
            while z < b.z1 - 0.002 {
                bar(z, (z + step * 0.62).min(b.z1), slat);
                bar(z + step * 0.62 - 0.002, z + step * 0.62, shade(b.rgb, 0.45));
                z += step;
            }
        }
        Finish::Framed => {
            bar(b.z0, b.z1, shade(b.rgb, side));
            let hit = view.pos + view.ray(x + COL_STEP * 0.5) * h.tn;
            let u = if h.normal.x != 0.0 {
                (hit.y - b.min.y) / (b.max.y - b.min.y)
            } else {
                (hit.x - b.min.x) / (b.max.x - b.min.x)
            };
            let dark = shade(b.rgb, 0.25);
            if !(0.03..=0.97).contains(&u) {
                bar(b.z0, b.z1, dark);
            } else {
                bar(b.z1 - 0.012, b.z1, dark);
            }
        }
        Finish::Blink => {
            let on = (t * 1.4).fract() < 0.5;
            let k = if on { 1.0 } else { 0.25 };
            bar(
                b.z0,
                b.z1,
                Color::new(b.rgb[0] * k, b.rgb[1] * k, b.rgb[2] * k, 1.0),
            );
        }
        Finish::Rack(d) if facing(d) => {
            bar(b.z0, b.z1, shade(b.rgb, side));
            let hit = view.pos + view.ray(x + COL_STEP * 0.5) * h.tn;
            let u = if h.normal.x != 0.0 {
                (hit.y - b.min.y) / (b.max.y - b.min.y)
            } else {
                (hit.x - b.min.x) / (b.max.x - b.min.x)
            };
            let slot = (u * 7.0).floor();
            let fr = (u * 7.0).fract();
            if (1.0..6.0).contains(&slot) && (0.3..0.7).contains(&fr) {
                let mut z = b.z0 + 0.06;
                let mut row = 0.0;
                while z < b.z1 - 0.04 {
                    let n = |k: f32| {
                        ((slot * 12.99 + row * 78.23 + k * 37.7).sin() * 43_758.5)
                            .fract()
                            .abs()
                    };
                    let blink = (t * (1.5 + 4.0 * n(1.0)) + n(2.0)).floor();
                    if n(3.0 + blink) > 0.35 {
                        let col = if n(4.0) < 0.75 {
                            [0.3, 1.0, 0.4]
                        } else {
                            [1.0, 0.7, 0.2]
                        };
                        bar(z, z + 0.012, Color::new(col[0], col[1], col[2], 1.0));
                    }
                    z += 0.055;
                    row += 1.0;
                }
            }
        }
        Finish::Bands(step) => {
            bar(b.z0, b.z1, shade(b.rgb, side));
            let dark = shade(b.rgb, side * 0.45);
            let mut z = b.z0 + step;
            while z < b.z1 - 0.005 {
                bar(z - 0.004, z + 0.004, dark);
                z += step;
            }
        }
        Finish::Links => {
            let (hi, lo) = (shade(b.rgb, 1.2), shade(b.rgb, 0.45));
            let mut z = b.z0;
            let mut odd = false;
            while z < b.z1 {
                let top = (z + 0.03).min(b.z1);
                bar(z, top, if odd { lo } else { hi });
                z = top;
                odd = !odd;
            }
        }
        _ => bar(b.z0, b.z1, shade(b.rgb, side)),
    }
    streak(b.z0, b.z1);
}

/// A world point on a wall surface, projected: screen `(x, y)` and depth — if it's in
/// front of the camera and not behind a wall or a nearer piece of furniture.
fn onto(view: &View, front: &Occluders, p: Vec2, z: f32) -> Option<(Vec2, f32)> {
    let (sx, depth) = view.project(p)?;
    let ci = ((sx - view.area.x) / COL_STEP).floor();
    if ci < 0.0 || ci as usize >= front.columns() {
        return None;
    }
    let y = view.screen_y(z, depth);
    (view.visible(sx, depth - 0.03) && !front.hides(ci as usize, depth, y))
        .then_some((vec2(sx, y), depth))
}

/// A quad on a wall, corners `(u, z)` along `origin + along * u`, if all are visible.
fn wall_quad(
    view: &View,
    front: &Occluders,
    origin: Vec2,
    along: Vec2,
    q: [(f32, f32); 4],
    col: Color,
) {
    let pts: Vec<Vec2> = q
        .iter()
        .filter_map(|&(u, z)| onto(view, front, origin + along * u, z).map(|v| v.0))
        .collect();
    if pts.len() == 4 {
        draw_triangle(pts[0], pts[1], pts[2], col);
        draw_triangle(pts[0], pts[2], pts[3], col);
    }
}

/// Marker on a whiteboard: a few notes — or the conspiracy wall.
/// In a mirror, whether something `reach` either side of `at` can overlap the glass
/// at all — a cheap reject before an overlay's label lookup and per-column work, as
/// most of them lie outside any one glass run. Always true for the real camera.
fn in_span(view: &View, at: Vec2, reach: f32) -> bool {
    if !view.reflected() {
        return true;
    }
    let Some((x, depth)) = view.project(at) else {
        return false;
    };
    let (lo, hi) = view.span();
    let r = reach * view.px_per_unit(depth);
    x + r >= lo && x - r <= hi
}

fn draw_board(view: &View, front: &Occluders, fade: label::Fade, b: &Board) {
    if b.out.dot(view.pos - b.at) < 0.05 || !in_span(view, b.at, b.half) {
        return;
    }
    let pt = |u: f32| b.at + b.along * u;
    let line = |u0: f32, z0: f32, u1: f32, z1: f32, w: f32, col: Color| {
        if let (Some((a, _)), Some((c, _))) =
            (onto(view, front, pt(u0), z0), onto(view, front, pt(u1), z1))
        {
            draw_line(a.x, a.y, c.x, c.y, w, col);
        }
    };
    let h = |k: f32| ((b.seed * 97.0 + k * 13.7).sin() * 0.5 + 0.5).fract();
    if b.writing == Writing::Cake {
        let cake = Spec::new(&[CAKE], Color::new(0.1, 0.1, 0.12, 1.0), None);
        board_text(view, front, fade, b, &cake, 0.55, 0.55);
        return;
    }
    if b.writing == Writing::Notes {
        // Scribbled notes: short wavy lines in two marker colours, and a boxed word.
        for i in 0..5 {
            let z = 0.66 - i as f32 * 0.06;
            let u0 = -b.half * 0.8 + h(i as f32) * 0.1;
            let len = 0.2 + h(i as f32 + 7.0) * 0.3;
            let col = if i % 2 == 0 {
                Color::new(0.1, 0.2, 0.7, 1.0)
            } else {
                Color::new(0.1, 0.1, 0.1, 1.0)
            };
            let mut u = u0;
            while u < u0 + len {
                let d = (u * 40.0).sin() * 0.006;
                line(u, z + d, u + 0.02, z - d, 1.2, col);
                u += 0.02;
            }
        }
        let red = Color::new(0.8, 0.1, 0.1, 1.0);
        let (u, z) = (b.half * 0.45, 0.45);
        line(u - 0.08, z - 0.04, u + 0.08, z - 0.04, 1.2, red);
        line(u + 0.08, z - 0.04, u + 0.08, z + 0.04, 1.2, red);
        line(u + 0.08, z + 0.04, u - 0.08, z + 0.04, 1.2, red);
        line(u - 0.08, z + 0.04, u - 0.08, z - 0.04, 1.2, red);
        return;
    }
    // Pepe Silvia: papers pinned all over, red string between them, the name on top.
    let papers: [(f32, f32); 7] = [
        (-0.24, 0.6),
        (-0.1, 0.52),
        (0.04, 0.62),
        (0.18, 0.5),
        (-0.2, 0.4),
        (0.05, 0.41),
        (0.24, 0.62),
    ];
    let paper = Color::new(0.97, 0.96, 0.9, 1.0);
    for &(u, z) in &papers {
        let q = [
            (u - 0.035, z + 0.045),
            (u + 0.035, z + 0.045),
            (u + 0.035, z - 0.045),
            (u - 0.035, z - 0.045),
        ];
        wall_quad(view, front, b.at, b.along, q, paper);
    }
    let string = Color::new(0.85, 0.05, 0.05, 1.0);
    for (i, &(u, z)) in papers.iter().enumerate() {
        for j in [(i + 2) % papers.len(), (i + 3) % papers.len()] {
            let (u2, z2) = papers[j];
            line(u, z, u2, z2, 1.3, string);
        }
        line(u - 0.005, z + 0.04, u + 0.005, z + 0.04, 2.5, string);
    }
    board_text(
        view,
        front,
        fade,
        b,
        &Spec::new(&[PEPE], string, None),
        0.69,
        0.5,
    );
}

/// Marker lettering across a whiteboard, centred at height `z`, at most `width` wide.
fn board_text(
    view: &View,
    front: &Occluders,
    fade: label::Fade,
    b: &Board,
    spec: &Spec,
    z: f32,
    width: f32,
) {
    let l = label::get(spec);
    let size = l.size(0.05, width);
    label::draw_on_wall(view, front, fade, &l, b.at, b.along, z, size);
}

/// A name plate on a door's hall side, turning with the door — and, on a washroom
/// that's `out_of_order`, a sheet taped under it.
fn door_sign(
    view: &View,
    front: &Occluders,
    fade: label::Fade,
    p: &Panel,
    swing: f32,
    spec: &Spec,
    out_of_order: bool,
) {
    // The middle of the leaf's hall face as it stands, turning with it.
    let (centre, e, n) = crate::view::raycast::sign_mount(p, swing);
    if n.dot(view.pos - centre) < 0.02 {
        return;
    }
    let plate = label::get(spec);
    let size = plate.size(0.035, 0.44);
    label::draw_on_wall(
        view,
        front,
        fade,
        &plate,
        centre,
        e,
        0.68 - size.y * 0.5,
        size,
    );
    if out_of_order {
        let sheet = label::get(&Spec::new(&["OUT OF", "ORDER"], INK, Some(PAPER)));
        let size = sheet.size(0.03, 0.34);
        label::draw_on_wall(view, front, fade, &sheet, centre + n * 0.002, e, 0.5, size);
    }
}

/// The campfire's flames: a warm glow on the ground, then tongues of flame licking up
/// from the logs — each a flickering teardrop, hotter (yellower, smaller) inside —
/// and a few sparks rising. Not culled by its own base point (`onto`): the logs in
/// front of it would hide the whole fire, as would its centre sliding off screen up
/// close. The trees are its only occlusion (painter's order); the mist fades it like them.
fn draw_fire(view: &View, at: Vec2, t: f32, pal: &Palette) {
    let Some((sx, depth)) = view.project(at) else {
        return;
    };
    // Up close the centre can leave the screen while the flames still show: test the
    // edge column nearest it.
    let edge = sx.clamp(view.area.x, view.area.x + view.area.w - COL_STEP);
    if !view.visible(edge, depth - 0.03) {
        return;
    }
    let base = vec2(sx, view.screen_y(0.06, depth));
    let px = view.px_per_unit(depth);
    let lit = lit_at(depth, pal.haze, 1.0);
    let glow = |r: f32, a: f32, c: [f32; 3]| {
        draw_circle(
            base.x,
            base.y,
            r * px,
            Color::new(c[0], c[1], c[2], a * lit),
        );
    };
    glow(0.45, 0.08, [1.0, 0.55, 0.2]);
    glow(0.25, 0.12, [1.0, 0.6, 0.25]);
    for (i, (dx, h, w, col)) in [
        (-0.05, 0.2, 0.06, [0.95, 0.35, 0.08]),
        (0.05, 0.22, 0.06, [0.95, 0.35, 0.08]),
        (0.0, 0.28, 0.08, [1.0, 0.5, 0.1]),
        (-0.02, 0.18, 0.05, [1.0, 0.75, 0.25]),
        (0.02, 0.14, 0.035, [1.0, 0.95, 0.6]),
    ]
    .into_iter()
    .enumerate()
    {
        let f = i as f32 * 1.7;
        let flick = 0.8 + 0.2 * (t * 9.0 + f).sin() + 0.1 * (t * 23.0 + f * 3.0).sin();
        let sway = 0.02 * (t * 5.0 + f).sin();
        let x = base.x + (dx + sway) * px;
        let top = vec2(x + sway * px, base.y - h * flick * px);
        let (l, r) = (vec2(x - w * px, base.y), vec2(x + w * px, base.y));
        let mid = vec2(x, base.y - h * flick * 0.45 * px);
        let c = Color::new(col[0], col[1], col[2], 0.9 * lit);
        draw_triangle(l, r, top, c);
        draw_circle(mid.x, base.y - w * 0.4 * px, w * px, c);
    }
    for i in 0..4 {
        let k = (t * 0.6 + i as f32 * 0.27).fract();
        let x = base.x + (0.06 * (i as f32 * 2.3 + t).sin()) * px;
        let y = base.y - (0.1 + k * 0.5) * px;
        let a = (1.0 - k) * lit;
        draw_circle(x, y, (0.006 * px).max(1.0), Color::new(1.0, 0.7, 0.3, a));
    }
}

/// Printed text on a white sheet, flat on a wall.
fn draw_poster(view: &View, front: &Occluders, fade: label::Fade, p: &Poster) {
    if p.out.dot(view.pos - p.at) < 0.05 || !in_span(view, p.at, p.width * 0.5) {
        return;
    }
    let sheet = label::get(&Spec {
        icon: p.icon,
        ..Spec::new(p.lines, p.ink, Some(p.paper))
    });
    let size = sheet.size(p.cap, p.width);
    let along = vec2(-p.out.y, p.out.x);
    // Neon carries its own light: no fading into the murk.
    let fade = if p.glow {
        label::Fade {
            haze: 0.0,
            light: 1.0,
            ..fade
        }
    } else {
        fade
    };
    label::draw_on_wall(view, front, fade, &sheet, p.at, along, p.z, size);
}

/// A cell's four corners as (N/S wall, E/W wall), those against wall `first` first.
fn corners(first: Option<usize>) -> Vec<(usize, usize)> {
    let mut v = vec![(0, 1), (0, 3), (2, 1), (2, 3)];
    if let Some(f) = first {
        crate::util::sort_by_key(&mut v, |&(a, b)| (a != f && b != f) as u8 as f32);
    }
    v
}

/// The garden clocks' face: a bare pictogram, no text.
fn clock() -> Spec {
    Spec {
        icon: Some(Icon::Clock),
        ..Spec::new(&[], INK, None)
    }
}

/// A garden sign's or clock's label, flat on its board.
fn draw_mark(view: &View, front: &Occluders, fade: label::Fade, m: &Mark) {
    if m.out.dot(view.pos - m.at) < 0.05 || !in_span(view, m.at, m.width * 0.5) {
        return;
    }
    let sheet = label::get(&m.spec);
    let size = sheet.size(m.cap, m.width);
    label::draw_on_wall(
        view,
        front,
        fade,
        &sheet,
        m.at,
        vec2(-m.out.y, m.out.x),
        m.z,
        size,
    );
}

/// The Cheshire Cat: a wide toothy grin and two slit-pupilled eyes hanging in front of
/// a hedge, fading in, lingering and fading out again (about a 14 s cycle, the cat
/// gone for a good part of it). Screen-space shapes at the projected centre, squeezed
/// by the mouth's foreshortened width; hidden whole when that centre is.
fn draw_grin(view: &View, front: &Occluders, g: &Grin, t: f32) {
    if g.out.dot(view.pos - g.at) < 0.05 || !in_span(view, g.at, 0.2) {
        return;
    }
    let a = ((t * 0.45 + g.phase).sin() * 1.8 - 0.4).clamp(0.0, 1.0);
    if a <= 0.0 {
        return;
    }
    let z = 0.6;
    let Some((c, depth)) = onto(view, front, g.at, z) else {
        return;
    };
    if depth > 8.0 {
        return;
    }
    let along = vec2(-g.out.y, g.out.x);
    let (Some((xa, _)), Some((xb, _))) = (
        view.project(g.at - along * 0.15),
        view.project(g.at + along * 0.15),
    ) else {
        return;
    };
    let half = (xb - xa).abs() * 0.5;
    // Screen-space shapes aren't column-clipped: in a mirror, only a grin wholly inside
    // the glass shows.
    if half < 3.0
        || view.reflected()
            && !(view.visible(c.x - half * 1.1, depth) && view.visible(c.x + half * 1.1, depth))
    {
        return;
    }
    let px = view.px_per_unit(depth);
    // Screen y of the upper and lower lip at `u` across the mouth (-1..1): the tips
    // meet high at the corners, the lower lip sags deep in the middle.
    let lip = |u: f32, sag: f32| c.y + px * (sag * (1.0 - u * u) - 0.012);
    let dark = view.tint(Color::new(0.12, 0.02, 0.1, a));
    let teeth = view.tint(Color::new(0.98, 0.97, 0.9, a));
    let n = 14;
    let pt = |u: f32, sag: f32| vec2(c.x + u * half, lip(u, sag));
    for i in 0..n {
        let (u0, u1) = (
            -1.0 + 2.0 * i as f32 / n as f32,
            -1.0 + 2.0 * (i + 1) as f32 / n as f32,
        );
        // Dark lips round it, then the teeth inside.
        for (top, bot, col, grow) in [(0.03, 0.08, dark, 0.012), (0.03, 0.08, teeth, 0.0)] {
            let (a0, a1) = (
                pt(u0, top) - vec2(0.0, grow * px),
                pt(u1, top) - vec2(0.0, grow * px),
            );
            let (b0, b1) = (
                pt(u0, bot) + vec2(0.0, grow * px),
                pt(u1, bot) + vec2(0.0, grow * px),
            );
            draw_triangle(a0, a1, b1, col);
            draw_triangle(a0, b1, b0, col);
        }
    }
    // The gap between the rows of teeth, and the gaps between teeth.
    let w = (0.006 * px).max(1.0);
    for i in 0..n {
        let (u0, u1) = (
            -1.0 + 2.0 * i as f32 / n as f32,
            -1.0 + 2.0 * (i + 1) as f32 / n as f32,
        );
        let (p0, p1) = (pt(u0, 0.055), pt(u1, 0.055));
        draw_line(p0.x, p0.y, p1.x, p1.y, w, dark);
    }
    for k in -3..=3 {
        let u = k as f32 * 0.24;
        let (p0, p1) = (pt(u, 0.03), pt(u, 0.08));
        draw_line(p0.x, p0.y, p1.x, p1.y, w, dark);
    }
    // Eyes above: yellow-green, slit pupils.
    let squeeze = half / (0.15 * px);
    for s in [-1.0, 1.0] {
        let (ex, ey) = (c.x + s * half * 0.55, c.y - 0.12 * px);
        let (rx, ry) = (0.04 * px * squeeze, 0.028 * px);
        draw_ellipse(ex, ey, rx + w, ry + w, 0.0, dark);
        draw_ellipse(
            ex,
            ey,
            rx,
            ry,
            0.0,
            view.tint(Color::new(0.78, 0.9, 0.2, a)),
        );
        draw_ellipse(ex, ey, rx * 0.22, ry * 0.95, 0.0, dark);
    }
}

/// Ray vs a block's footprint in the floor plane: `(entry, exit, entry normal, miss)`
/// — a slab test for a box, the ellipse for anything round.
fn hit(o: Vec2, r: Vec2, b: &Block) -> Option<(f32, f32, Vec2, f32)> {
    match b.shape {
        Shape::Box => slab(o, r, b.min, b.max).map(|(tn, tf, n)| (tn, tf, n, 0.0)),
        _ => ellipse(o, r, b.min, b.max),
    }
}

/// Ray vs the ellipse inscribed in `min..max`: `(entry, exit, entry normal, miss)`,
/// `miss` being how far the ray passes from the centre (0) to grazing the rim (1).
/// `None` if it misses or the ellipse is behind the camera (or round it).
fn ellipse(o: Vec2, r: Vec2, min: Vec2, max: Vec2) -> Option<(f32, f32, Vec2, f32)> {
    let (c, h) = ((min + max) * 0.5, (max - min) * 0.5);
    // Scaled so the ellipse is the unit circle: solve |p + d t| = 1.
    let (p, d) = ((o - c) / h, r / h);
    let (a, b) = (d.dot(d), p.dot(d));
    let disc = b * b - a * (p.dot(p) - 1.0);
    if disc <= 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let (tn, tf) = ((-b - s) / a, (-b + s) / a);
    if tn < 0.05 {
        return None;
    }
    let normal = ((p + d * tn) / h).normalize_or_zero();
    Some((tn, tf, normal, (1.0 - disc / a).max(0.0).sqrt()))
}

/// Ray vs axis-aligned box in the floor plane: `(entry, exit, entry face normal)`, or
/// `None` if the ray misses or the box is behind the camera.
fn slab(o: Vec2, r: Vec2, min: Vec2, max: Vec2) -> Option<(f32, f32, Vec2)> {
    let axis = |o: f32, r: f32, lo: f32, hi: f32| -> Option<(f32, f32, f32)> {
        if r.abs() < 1e-7 {
            return (o >= lo && o <= hi).then_some((f32::NEG_INFINITY, f32::INFINITY, 0.0));
        }
        let (a, b) = ((lo - o) / r, (hi - o) / r);
        // Entering through the low face means that face's normal points back at -r.
        Some(if a < b { (a, b, -1.0) } else { (b, a, 1.0) })
    };
    let (x0, x1, sx) = axis(o.x, r.x, min.x, max.x)?;
    let (y0, y1, sy) = axis(o.y, r.y, min.y, max.y)?;
    let (tn, tf) = (x0.max(y0), x1.min(y1));
    if tn > tf || tn < 0.05 {
        return None;
    }
    let normal = if x0 > y0 {
        vec2(sx, 0.0)
    } else {
        vec2(0.0, sy)
    };
    Some((tn, tf, normal))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::maze::generate;

    /// Distance from point `p` to block `b`'s footprint: its box, or for a round block
    /// the ellipse inscribed in it — exact for a circle; for an oblong ellipse the larger
    /// of two lower bounds (the box, and the circle round its longer half-axis).
    fn gap(b: &Block, p: Vec2) -> f32 {
        let boxed = (b.min - p).max(p - b.max).max(Vec2::ZERO).length();
        if b.shape == Shape::Box {
            return boxed;
        }
        let (c, h) = ((b.min + b.max) * 0.5, (b.max - b.min) * 0.5);
        boxed.max(p.distance(c) - h.max_element())
    }

    /// Open spaces are crowded but nothing shares space: no two fittings (desks,
    /// chairs, dividers, wall fittings, whiteboards) overlap, bar one resting on
    /// another.
    #[test]
    fn open_space_fittings_do_not_overlap() {
        let mut checked = 0;
        for seed in 0..12u64 {
            let m = office(seed);
            let decor = Decor::build(ThemeKind::Backrooms, &m, |_| false, seed);
            let recess = decor.recess();
            for r in m.rooms.iter().filter(|r| r.kind == RoomKind::OpenSpace) {
                let (min, size) = floor(r);
                let pad = Vec2::splat(recess + 0.01);
                let (lo, hi) = (min - pad, min + size + pad);
                let here: Vec<&Block> = decor
                    .blocks
                    .iter()
                    .filter(|b| b.min.cmpge(lo).all() && b.max.cmple(hi).all())
                    .filter(|b| !b.flat())
                    .collect();
                for (i, a) in here.iter().enumerate() {
                    for b in here[i + 1..].iter().filter(|b| b.group != a.group) {
                        let over = (a.max.min(b.max) - a.min.max(b.min)).min_element();
                        let zover = a.z1.min(b.z1) - a.z0.max(b.z0);
                        assert!(
                            over <= 0.005 || zover <= 0.005,
                            "seed {seed}: parts {}..{} z {}-{} and {}..{} z {}-{} overlap",
                            a.min,
                            a.max,
                            a.z0,
                            a.z1,
                            b.min,
                            b.max,
                            b.z0,
                            b.z1
                        );
                        checked += 1;
                    }
                }
            }
        }
        assert!(checked > 100, "{checked}");
    }

    /// Nothing in an office room hangs in mid-air: every fitting (a `group`) either
    /// stands on the floor or touches the room's wall face (among thin walls, where
    /// `settle_room` moved it) — a fixture held off its wall is left floating once the
    /// wall goes thin (the washroom mirror and dryer were).
    #[test]
    fn room_fittings_stand_or_hang_on_a_wall() {
        for seed in 0..12u64 {
            let m = office(seed);
            let decor = Decor::build(ThemeKind::Backrooms, &m, |_| false, seed);
            let recess = decor.recess();
            for r in &m.rooms {
                let (min, size) = floor(r);
                let (lo, hi) = (min - Vec2::splat(recess), min + size + Vec2::splat(recess));
                let inside =
                    |b: &Block| b.min.cmpge(lo - 0.01).all() && b.max.cmple(hi + 0.01).all();
                let mut i = 0;
                while i < decor.blocks.len() {
                    let g = decor.blocks[i].group;
                    let end = (i..decor.blocks.len())
                        .find(|&j| decor.blocks[j].group != g)
                        .unwrap_or(decor.blocks.len());
                    let bs = &decor.blocks[i..end];
                    i = end;
                    if !bs.iter().all(inside) || bs.iter().any(|b| b.z0 <= 0.02) {
                        continue;
                    }
                    // Resting on something else (a mug on a desk, the boss in his chair).
                    let rests = bs.iter().any(|b| {
                        decor.blocks.iter().any(|o| {
                            o.group != g
                                && o.z1 <= b.z0 + 0.02
                                && o.z1 >= b.z0 - 0.2
                                && o.min.cmplt(b.max).all()
                                && b.min.cmplt(o.max).all()
                        })
                    });
                    if rests {
                        continue;
                    }
                    let (bl, bh) = bs.iter().fold((Vec2::MAX, Vec2::MIN), |(a, z), b| {
                        (a.min(b.min), z.max(b.max))
                    });
                    let touches = (0..2).any(|k| bl[k] - lo[k] < 0.03 || hi[k] - bh[k] < 0.03);
                    assert!(
                        touches,
                        "seed {seed} {:?} room: fitting {bl}..{bh} hangs in the air",
                        r.kind
                    );
                }
            }
        }
    }

    fn office(seed: u64) -> Maze {
        let _g = crate::rng_guard();
        macroquad::rand::srand(seed);
        generate(21, 15, 0.2, 0, 8)
    }

    /// A Mirror-mode garden, generated as `Game::with_mode` does at generation 0.
    fn garden(seed: u64) -> Maze {
        let _g = crate::rng_guard();
        macroquad::rand::srand(seed);
        let mut m = generate(15, 11, 0.15, 0, 0);
        m.place_gate();
        m.add_mirrors(15 * 11);
        m
    }

    /// The garden's dressing turns up, and its labels (signs, clocks) hang only on
    /// hedge faces, never on a mirror.
    #[test]
    fn garden_dressing_shows_up_away_from_mirrors() {
        let (mut marks, mut grins, mut flat, mut fountains) = (0, 0, 0, 0);
        for seed in 0..30u64 {
            let m = garden(seed);
            let decor = Decor::build(ThemeKind::Garden, &m, |_| false, seed);
            marks += decor.marks.len();
            grins += decor.grins.len();
            flat += decor.blocks.iter().filter(|b| b.flat()).count();
            fountains += decor.fountain().is_some() as usize;
            for mk in &decor.marks {
                let p = (mk.at + mk.out * 0.4 - Vec2::splat(1.5)) / 2.0;
                let c = Cell {
                    x: p.x.round() as i32,
                    y: p.y.round() as i32,
                };
                let dir = (0..4).find(|&d| dir_vec(d) == -mk.out).unwrap();
                assert_eq!(
                    m.edge(c, dir),
                    EdgeKind::Wall,
                    "seed {seed}: label off a hedge"
                );
            }
        }
        assert!(
            marks > 30 && grins > 5 && flat > 30 * 36 && fountains >= 27,
            "{marks} {grins} {flat} {fountains}"
        );
    }

    /// Nothing stands on a walked line: every step the bot can take runs centre to
    /// centre between two connected cells (never into a peek-only room), and every box
    /// keeps clear of it.
    #[test]
    fn decor_keeps_the_paths_clear() {
        let themes = [ThemeKind::Backrooms, ThemeKind::Dungeon, ThemeKind::Garden];
        for (seed, theme) in (0..30u64).flat_map(|s| themes.map(|t| (s, t))) {
            let m = match theme {
                ThemeKind::Dungeon => {
                    let _g = crate::rng_guard();
                    macroquad::rand::srand(seed);
                    generate(21, 15, 0.2, 3, 0)
                }
                ThemeKind::Garden => garden(seed),
                _ => office(seed),
            };
            let decor = Decor::build(theme, &m, |_| false, seed);
            assert!(!decor.blocks.is_empty());
            let peek = |c: Cell| m.rooms.iter().any(|r| r.peek_only() && r.contains(c));
            let centre = |c: Cell| square(c) + vec2(0.5, 0.5);
            for y in 0..m.h as i32 {
                for x in 0..m.w as i32 {
                    let c = Cell { x, y };
                    for d in 0..4 {
                        // The exit cell is the last step: its lift / stairwell is walked into once.
                        let last = c == m.exit || c.step(d) == m.exit;
                        // The janitor's closet is the one small room the bot walks into.
                        let closet = |c: Cell| {
                            m.rooms
                                .iter()
                                .any(|r| r.kind == RoomKind::Janitor && r.contains(c))
                        };
                        let skip = |c: Cell| peek(c) && !closet(c);
                        if !m.is_open(c, d) || skip(c) || skip(c.step(d)) || last {
                            continue;
                        }
                        let (a, z) = (centre(c), centre(c.step(d)));
                        // Flat ground patches (the garden's chessboard) are walked over;
                        // the stairwell's flight is only there through its portal.
                        let (f0, f1) = (decor.flight.0 as usize, decor.flight.1 as usize);
                        for (_, b) in decor
                            .blocks
                            .iter()
                            .enumerate()
                            .filter(|(i, b)| !b.flat() && !(f0..f1).contains(i))
                        {
                            for k in 0..=20 {
                                let p = a.lerp(z, k as f32 / 20.0);
                                assert!(
                                    gap(b, p) >= 0.27,
                                    "seed {seed}: box {:?}-{:?} on the path {c:?} dir {d}",
                                    b.min,
                                    b.max
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// Everything in a peek-only room stays inside it (out to its thin walls' faces) and
    /// out of the floor the open door swings over — a leaf `DOOR_OPENING` wide standing
    /// perpendicular to the wall from either jamb.
    #[test]
    fn peek_rooms_keep_inside_and_clear_of_the_door() {
        let mut rooms = 0;
        // Each maze twice: with him, and with the lady boss behind the desk.
        for (seed, lady) in (0..30u64).flat_map(|s| [(s, false), (s, true)]) {
            let mut m = office(seed);
            m.lady_boss = lady;
            let decor = Decor::build(ThemeKind::Backrooms, &m, |_| false, seed);
            let grow = decor.recess();
            for r in m.rooms.iter().filter(|r| r.peek_only()) {
                rooms += 1;
                let (min, size) = floor(r);
                let (min, max) = (min - Vec2::splat(grow), min + size + Vec2::splat(grow));
                let door = m.doors.iter().find(|o| r.contains(o.cell)).unwrap();
                let out = dir_vec(door.dir);
                // The frame's room-side face, and the swept floor inside it.
                let face = square(door.cell) + vec2(0.5, 0.5) + out * (1.0 - WALL_T * 0.5);
                let across = vec2(out.y.abs(), out.x.abs()) * (crate::maze::DOOR_OPENING * 0.5);
                let deep = face - out * 0.64;
                let (s0, s1) = (face.min(deep) - across, face.max(deep) + across);
                for b in &decor.blocks {
                    let c = (b.min + b.max) * 0.5;
                    let inside = c.cmpgt(min).all() && c.cmplt(max).all();
                    if !inside {
                        continue;
                    }
                    assert!(b.min.cmpge(min - 1e-4).all() && b.max.cmple(max + 1e-4).all());
                    let swept = b.min.cmplt(s1).all() && b.max.cmpgt(s0).all();
                    assert!(
                        !swept,
                        "seed {seed}: {:?}-{:?} in the door's swing ({:?})",
                        b.min, b.max, r.kind
                    );
                }
            }
        }
        assert!(rooms > 20, "too few peek-only rooms generated: {rooms}");
    }

    /// The lava lamp's blobs rise round the lamp where it stands — rooms moved back
    /// with the thin walls take the axis along (it's stored apart from the blocks).
    #[test]
    fn lava_stays_in_its_lamp() {
        let mut lamps = 0;
        for seed in 0..30u64 {
            let decor = Decor::build(ThemeKind::Backrooms, &office(seed), |_| false, seed);
            for (_, base, motion) in &decor.moves {
                if let Motion::Blob { axis, .. } = *motion {
                    lamps += 1;
                    let c = (base.min + base.max) * 0.5;
                    assert!(
                        axis.distance(c) < 0.02,
                        "seed {seed}: blob at {axis} off {c}"
                    );
                }
            }
        }
        assert!(lamps > 0);
    }

    /// Where a 3D ray — the column ray `o + r·t` in the floor plane, height
    /// `eye - v·t` — first enters block `b`'s true solid, if it does.
    fn enter_3d(b: &Block, o: Vec2, r: Vec2, eye: f32, v: f32) -> Option<f32> {
        let span = |lo: f32, hi: f32| -> (f32, f32) {
            if v.abs() < 1e-9 {
                return if (lo..=hi).contains(&eye) {
                    (f32::NEG_INFINITY, f32::INFINITY)
                } else {
                    (1.0, 0.0)
                };
            }
            let (a, c) = ((eye - hi) / v, (eye - lo) / v);
            (a.min(c), a.max(c))
        };
        let (mut lo, mut hi) = span(b.z0, b.z1);
        let (t0, t1) = match b.shape {
            Shape::Box => slab(o, r, b.min, b.max).map(|(a, c, _)| (a, c))?,
            Shape::Cyl => ellipse(o, r, b.min, b.max).map(|(a, c, ..)| (a, c))?,
            _ => {
                let (zc, hz) = match b.shape {
                    Shape::Dome => (b.z0, b.z1 - b.z0),
                    Shape::Bowl => (b.z1, b.z1 - b.z0),
                    _ => ((b.z0 + b.z1) * 0.5, (b.z1 - b.z0) * 0.5),
                };
                let (c, h) = ((b.min + b.max) * 0.5, (b.max - b.min) * 0.5);
                let (p, d) = ((o - c) / h, r / h);
                let (q, e) = ((eye - zc) / hz, -v / hz);
                let qa = d.dot(d) + e * e;
                let qb = 2.0 * (p.dot(d) + q * e);
                let qc = p.dot(p) + q * q - 1.0;
                let disc = qb * qb - 4.0 * qa * qc;
                if disc <= 0.0 {
                    return None;
                }
                let sq = disc.sqrt();
                ((-qb - sq) / (2.0 * qa), (-qb + sq) / (2.0 * qa))
            }
        };
        lo = lo.max(t0).max(0.05);
        hi = hi.min(t1);
        (lo <= hi).then_some(lo)
    }

    /// Pixels (of a 900×720 view from `o` looking along `fwd`, eye at 0.5) where the
    /// painter's last block over a pixel is not the one a 3D ray meets first, in a
    /// clearly different colour; `only` limits the blocks drawn. Glass is skipped (see
    /// through), and near-coincident surfaces count either way.
    fn painter_errors(
        blocks: &[Block],
        only: &[u32],
        o: Vec2,
        fwd: Vec2,
        counts: impl Fn(u32, u32) -> bool,
    ) -> (usize, usize) {
        let (w, h) = (900.0f32, 720.0f32);
        let plane = vec2(-fwd.y, fwd.x) * (w / (2.0 * h));
        let (mut order, mut moved) = (Vec::new(), Vec::new());
        let (mut bad, mut pixels) = (0, 0);
        let mut sx = 0.0;
        while sx < w {
            let r = fwd + plane * (2.0 * (sx + COL_STEP * 0.5) / w - 1.0);
            sx += COL_STEP;
            let mut hits: Vec<Hit> = only
                .iter()
                .filter_map(|&i| {
                    let b = &blocks[i as usize];
                    let (tn, tf, normal, miss) = hit(o, r, b)?;
                    Some(Hit {
                        key: tn + if b.flat() { 1.0e6 } else { 0.0 },
                        tn,
                        tf,
                        normal,
                        miss,
                        block: i,
                        clip: f32::INFINITY,
                    })
                })
                .collect();
            if hits.is_empty() {
                continue;
            }
            paint_order(
                blocks,
                &mut hits,
                0.5,
                (o, r),
                (&mut order, &mut moved),
                |z, _| z,
            );
            for y in 0..h as usize {
                let v = (y as f32 + 0.5 - h * 0.5) / h;
                let mut ts: Vec<(f32, u32)> = hits
                    .iter()
                    .filter_map(|x| {
                        enter_3d(&blocks[x.block as usize], o, r, 0.5, v).map(|t| (t, x.block))
                    })
                    .collect();
                if ts.is_empty() {
                    continue;
                }
                ts.sort_by(|a, b| a.0.total_cmp(&b.0));
                let painted = hits
                    .iter()
                    .rev()
                    .find(|x| ts.iter().any(|t| t.1 == x.block))
                    .expect("covered")
                    .block;
                let truth = ts[0].1;
                let (pb, tb) = (&blocks[painted as usize], &blocks[truth as usize]);
                if pb.clear() || tb.clear() || ts.iter().any(|t| blocks[t.1 as usize].clear()) {
                    continue;
                }
                pixels += 1;
                let tied = ts.iter().any(|t| t.1 == painted && t.0 - ts[0].0 < 2e-4);
                let far = (0..3).map(|i| (pb.rgb[i] - tb.rgb[i]).abs()).sum::<f32>() > 0.15;
                if painted != truth && !tied && far && counts(painted, truth) {
                    bad += 1;
                }
            }
        }
        (bad, pixels)
    }
    /// The boss's head (both figures), shaking as he argues, seen from his doorway:
    /// no feature on the face (brows, eyes, nose, lips) is ever painted under the face
    /// where the eye really meets it first. They sit a few mm proud of the face above or
    /// below its widest girth, so entered it behind that girth, and the shake flipped
    /// them in and out from frame to frame (`on_face`, `paint_order`).
    #[test]
    fn boss_face_features_stay_on_top() {
        let seed = (0..30u64)
            .find(|&s| office(s).rooms.iter().any(|r| r.kind == RoomKind::Boss))
            .expect("a boss room");
        for lady in [false, true] {
            let mut m = office(seed);
            m.lady_boss = lady;
            let d = Decor::build(ThemeKind::Backrooms, &m, |_| false, 1);
            let head: Vec<(u32, Block, Motion)> = d
                .moves
                .iter()
                .copied()
                .filter(|x| matches!(x.2, Motion::Head { .. }))
                .collect();
            let vol = |b: &Block| (b.max - b.min).element_product() * (b.z1 - b.z0);
            let mid = |b: &Block| (b.min + b.max) * 0.5;
            let on = |f: &Block, b: &Block| {
                vol(b) < vol(f) * 0.01
                    && b.z0 > f.z0
                    && b.z1 < f.z1
                    && mid(b).cmpge(f.min - 0.01).all()
                    && mid(b).cmple(f.max + 0.01).all()
            };
            // The face: the ball with the most small parts on it (the hair's is bigger).
            let (fi, face) = head
                .iter()
                .map(|x| (x.0, x.1))
                .filter(|(_, b)| b.shape == Shape::Ball)
                .max_by_key(|(_, f)| head.iter().filter(|x| on(f, &x.1)).count())
                .expect("a head");
            let feature = |b: &Block| on(&face, b);
            let feats: Vec<Vec2> = head
                .iter()
                .map(|x| x.1)
                .filter(feature)
                .map(|b| mid(&b))
                .collect();
            assert!(feats.len() >= 8, "{}", feats.len());
            // Toward the door: the features are on the front of the head.
            let fwd =
                (feats.iter().copied().sum::<Vec2>() / feats.len() as f32 - mid(&face)).normalize();
            let ids: Vec<u32> = head.iter().map(|x| x.0).collect();
            let mut hidden = 0;
            for k in 0..30 {
                let dynamic = Dynamic {
                    t: 1000.0 + k as f32 / 60.0,
                    boss_hp: 0.5,
                    lift: 0.0,
                    fire: None,
                };
                let mut blocks = d.blocks.clone();
                for &(i, base, motion) in &head {
                    blocks[i as usize] = moved(base, motion, dynamic);
                }
                for side in [0.0, 0.07] {
                    let o = mid(&face) + fwd * 1.7 + vec2(-fwd.y, fwd.x) * side;
                    let is_feature = |i: u32| feature(&d.blocks[i as usize]);
                    hidden +=
                        painter_errors(&blocks, &ids, o, -fwd, |p, t| p == fi && is_feature(t)).0;
                }
            }
            // 6313 / 4934 of ~1M pixels before; a few stragglers at the rim now.
            assert!(
                hidden < 100,
                "lady {lady}: {hidden} feature pixels under the face"
            );
        }
    }

    /// Diagnostic, not a check: how many pixels the per-column painter gets wrong
    /// (against `painter_errors`' 3D ray test) in every dressed office room, seen from
    /// its door — for weighing a change to `paint_order`/`stacked` across all the
    /// dressing at once. Intersecting round parts (plants, beanbags, the boss's hair)
    /// and conference rooms account for most of it.
    /// `cargo test -p labyrinth --release painter_scene_errors -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn painter_scene_errors() {
        for lady in [false, true] {
            let (mut bad, mut px) = (0, 0);
            let mut per = std::collections::BTreeMap::new();
            for seed in 0..8u64 {
                let mut m = office(seed);
                m.lady_boss = lady;
                let d = Decor::build(ThemeKind::Backrooms, &m, |_| false, 1);
                let mut blocks = d.blocks.clone();
                let dynamic = Dynamic {
                    t: 1000.3,
                    boss_hp: 0.5,
                    lift: 0.0,
                    fire: None,
                };
                for &(i, base, motion) in &d.moves {
                    blocks[i as usize] = moved(base, motion, dynamic);
                }
                for r in &m.rooms {
                    if r.vacant || (lady && r.kind != RoomKind::Boss) {
                        continue;
                    }
                    let (min, size) = floor(r);
                    let inside =
                        |p: Vec2| p.cmpge(min - 0.3).all() && p.cmple(min + size + 0.3).all();
                    let only: Vec<u32> = (0..blocks.len() as u32)
                        .filter(|&i| {
                            inside((blocks[i as usize].min + blocks[i as usize].max) * 0.5)
                        })
                        .collect();
                    for door in &m.doors {
                        let c = door.cell;
                        let into_room = |c: Cell| {
                            c.x >= r.x && c.x < r.x + r.w && c.y >= r.y && c.y < r.y + r.h
                        };
                        let n = Cell {
                            x: c.x + DIRS[door.dir].0,
                            y: c.y + DIRS[door.dir].1,
                        };
                        let look = if into_room(n) && !into_room(c) {
                            dir_vec(door.dir)
                        } else if into_room(c) && !into_room(n) {
                            -dir_vec(door.dir)
                        } else {
                            continue;
                        };
                        let gap = square(c) + 0.5 + dir_vec(door.dir);
                        for (back, side) in [(0.9, 0.0), (0.4, 0.1), (1.3, -0.15)] {
                            let o = gap - look * back + vec2(-look.y, look.x) * side;
                            let (b, p) = painter_errors(&blocks, &only, o, look, |_, _| true);
                            bad += b;
                            px += p;
                            *per.entry(format!("{:?}", r.kind)).or_insert(0usize) += b;
                        }
                    }
                }
            }
            eprintln!("lady={lady}: scenes {bad} of {px} wrong; {per:?}");
        }
    }

    /// The lady boss replaces him, not joins him: a different figure, the same
    /// animated head-and-fist rig, the #1 BOSS mug, and nothing of hers when he's in.
    #[test]
    fn lady_boss_swaps_the_figure() {
        let m = (0..30u64)
            .map(office)
            .find(|m| m.rooms.iter().any(|r| r.kind == RoomKind::Boss))
            .expect("a boss room");
        let him = Decor::build(ThemeKind::Backrooms, &m, |_| false, 1);
        let mut m = m;
        m.lady_boss = true;
        let her = Decor::build(ThemeKind::Backrooms, &m, |_| false, 1);
        let mug = |d: &Decor| d.posters.iter().any(|p| p.lines == [BOSS_MUG]);
        assert!(mug(&her) && !mug(&him));
        let fists = |d: &Decor| {
            d.moves
                .iter()
                .filter(|m| matches!(m.2, Motion::Fist))
                .count()
        };
        assert_eq!((fists(&him), fists(&her)), (1, 1));
        let heads = |d: &Decor| {
            d.moves
                .iter()
                .filter(|m| matches!(m.2, Motion::Head { .. }))
                .count()
        };
        assert!(heads(&her) >= 20 && heads(&him) >= 20);
        assert_ne!(him.blocks.len(), her.blocks.len());
    }

    /// Garden sights: every one fits the view from its own cell or an open neighbour
    /// (the big set pieces only from next door), within the pitch/stoop limits; the
    /// DRINK ME bottle takes a stoop; and outside the Garden there's nothing to look at.
    #[test]
    fn looks_at_finds_the_garden_sights() {
        let (mut here, mut next_door, mut stooped) = (0, 0, 0);
        for seed in 0..30u64 {
            let m = garden(seed);
            let decor = Decor::build(ThemeKind::Garden, &m, |_| false, seed);
            assert!(!decor.sights.is_empty(), "seed {seed}: nothing to look at");
            let inside = |n: Cell| (0..m.w as i32).contains(&n.x) && (0..m.h as i32).contains(&n.y);
            for (key, s) in decor.sights.iter().enumerate() {
                let from = |c: Cell| {
                    decor
                        .looks_at(c, |d| m.is_open(c, d))
                        .into_iter()
                        .find(|g| g.key == key)
                };
                let near: Vec<Glance> = (0..4)
                    .filter(|&d| m.is_open(s.cell, d))
                    .map(|d| s.cell.step(d))
                    .filter(|&n| inside(n))
                    .filter_map(from)
                    .collect();
                match from(s.cell) {
                    Some(_) => here += 1,
                    None if !near.is_empty() => next_door += 1,
                    None => panic!("seed {seed}: sight {key} fits no view"),
                }
                for g in from(s.cell).iter().chain(&near) {
                    assert!((0.0..=MAX_PITCH).contains(&g.pitch));
                    assert!((0.0..=MAX_STOOP).contains(&g.stoop));
                    if g.stoop > 0.0 {
                        stooped += 1;
                    }
                }
            }
        }
        assert!(
            here > 10 && next_door > 10 && stooped > 0,
            "{here} {next_door} {stooped}"
        );
        let m = office(1);
        let decor = Decor::build(ThemeKind::Backrooms, &m, |_| false, 1);
        assert!(decor.looks_at(m.start, |_| true).is_empty());
    }

    /// `aim`: something tall close up doesn't fit; the same from further off does, level;
    /// a bottle at the feet takes a full look down and a stoop.
    #[test]
    fn aim_fits_the_view() {
        let eye = Vec2::ZERO;
        let post = |d: f32| aim(eye, vec2(d, -0.05), vec2(d + 0.1, 0.05), (0.0, 0.7));
        assert!(post(0.4).is_none());
        let (yaw, pitch, stoop) = post(2.0).expect("fits from afar");
        assert!(yaw.abs() < 1e-4 && stoop == 0.0 && pitch < 0.1);
        let (_, pitch, stoop) =
            aim(eye, vec2(0.4, -0.03), vec2(0.46, 0.03), (0.0, 0.19)).expect("bottle");
        assert!(pitch == MAX_PITCH && stoop > 0.1, "{pitch} {stoop}");
    }
}
