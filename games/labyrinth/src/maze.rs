//! Maze generation: seed -> deterministic grid maze.
//!
//! Randomized-DFS carve of a perfect maze, then a **braid** pass that removes a
//! fraction of dead ends by knocking one extra wall. Perfect mazes make wall-follow
//! trivially optimal and dead-end backtracking monotonous; loops are what turn the
//! explorer's frontier choice into a real decision (see `.notes/labyrinth_plan.md`).
//! Office floors (Tower) are the exception: a floor plan, not a maze — see `office`.
//!
//! Determinism: generation draws only from `macroquad::rand`, so `rand::srand(seed)`
//! before `generate()` makes the same seed produce the same maze — asserted in tests.

mod office;

use macroquad::math::{Vec2, vec2};
use macroquad::rand::gen_range;

/// N, E, S, W. `opposite(d) == (d + 2) % 4`.
pub const DIRS: [(i32, i32); 4] = [(0, -1), (1, 0), (0, 1), (-1, 0)];

#[inline]
pub fn opposite(d: usize) -> usize {
    (d + 2) % 4
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Cell {
    pub x: i32,
    pub y: i32,
}

impl Cell {
    pub fn step(self, d: usize) -> Cell {
        Cell {
            x: self.x + DIRS[d].0,
            y: self.y + DIRS[d].1,
        }
    }
}

/// What lies on one edge of a cell, from ground truth.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EdgeKind {
    Wall,
    Open,
    /// A wall that reflects — looks like an opening but can't be walked through
    /// (Mirror mode).
    Mirror,
    /// A doorway gated by key `id` (Dungeon mode).
    Locked(u8),
    /// An office door (Tower mode): shut until the bot pushes it open, no key needed.
    Door,
    /// An office door that never opens — a wall that looks like a door.
    Sealed,
}

/// Width of an office doorway's opening, centred in its unit wall square — what the
/// view draws and what the bot can see a room through.
pub const DOOR_OPENING: f32 = 0.62;

/// Thin-wall themes (Tower, Dungeon — `Maze::thin_walls`): a wall is a slab this thick
/// down the middle of its grid square, not the whole square. Garden hedges and Forest
/// trees keep the full square.
pub const WALL_T: f32 = 0.2;

/// Dungeon's stairwell flight (`Decor::stairwell` builds it, `Anim::descend` walks
/// it), from the arch face (0) to the flight's far wall (`STAIR_FLIGHT`): a sill to
/// `STAIR_SILL`, then `STAIR_STEPS` treads `STAIR_TREAD` deep, each `STAIR_RISE` lower.
/// It lies behind the portal (`Well`), not in the grid.
pub const STAIR_FLIGHT: f32 = 1.0;
pub const STAIR_SILL: f32 = 0.1;
pub const STAIR_STEPS: usize = 8;
pub const STAIR_TREAD: f32 = (STAIR_FLIGHT - STAIR_SILL) / STAIR_STEPS as f32;
pub const STAIR_RISE: f32 = 0.05;

/// How far a cell's centre is from the face of a wall: half a square for a full-square
/// wall, then on past the floor that a thin wall leaves either side of its slab.
pub const fn face_dist(thin: bool) -> f32 {
    if thin { 1.0 - WALL_T * 0.5 } else { 0.5 }
}

/// An office door on one edge of a room (Tower mode). `cell` is the room side, `dir`
/// points out into the hall.
#[derive(Clone, Copy, Debug)]
pub struct Door {
    pub cell: Cell,
    pub dir: usize,
    pub sealed: bool,
    /// Pushed open by the bot — line of sight passes through it.
    pub open: bool,
    /// Its room explored and the door pulled shut behind the bot: closed again, but
    /// nothing left to look at in there — the bot never reopens it just to look.
    pub done: bool,
    /// A washroom someone's in: an ordinary door that won't open — the bot tries it,
    /// hears so, and it's `seal`ed from then on.
    pub occupied: bool,
}

/// An office room (Tower mode): a `w x h` block of cells with every interior edge open
/// and no pillars, reached only through its `Door`s — one or two, always onto the same
/// stretch of hall, so a room is a detour, never a shortcut.
#[derive(Clone, Copy, Debug)]
pub struct Room {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
    /// Empty: no name on its door, nothing inside — one more bare room in the maze.
    pub vacant: bool,
    /// Which of its kind's layouts decor gives it, 0..1 (a meeting room: below 0.5
    /// the chart room, above it the zen room; a boardroom: throne or culture).
    pub style: f32,
    pub kind: RoomKind,
}

/// What an office room is for — only dresses it (`view::decor`), never changes the maze.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RoomKind {
    /// Rows of desks with computers.
    OpenSpace,
    /// The boss's office: one grand desk, a throne of a chair, a health bar.
    Boss,
    /// A cleaning-supplies closet.
    Janitor,
    /// A two-cell kitchen with a dining table and a TV.
    Kitchen,
    /// A meeting room with an ironic name, over-designed.
    Conference,
    /// The IT department: a server rack, and the Internet (a small black box).
    It,
    /// Washrooms: a pair of single-cell rooms back to back, MEN and WOMEN.
    Wc,
}

/// Which over-designed conceit a lived-in meeting room gets (`view::decor`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MeetingLayout {
    /// One cell, crowded round a chart.
    Chart,
    /// One cell, beanbags and calm.
    Zen,
    /// Two cells, a throne at the head of the table.
    Throne,
    /// Two cells, company culture.
    Culture,
}

impl Room {
    /// The layout a `Conference` room is dressed as, picked by its `style`; `None` for
    /// every other kind.
    pub fn meeting_layout(&self) -> Option<MeetingLayout> {
        (self.kind == RoomKind::Conference).then(|| match (self.w * self.h, self.style) {
            (1, s) if s < 0.5 => MeetingLayout::Chart,
            (1, _) => MeetingLayout::Zen,
            (_, s) if s < 0.6 => MeetingLayout::Throne,
            _ => MeetingLayout::Culture,
        })
    }

    pub fn contains(&self, c: Cell) -> bool {
        c.x >= self.x && c.y >= self.y && c.x < self.x + self.w && c.y < self.y + self.h
    }

    pub fn cells(&self) -> impl Iterator<Item = Cell> + '_ {
        (self.y..self.y + self.h)
            .flat_map(move |y| (self.x..self.x + self.w).map(move |x| Cell { x, y }))
    }

    /// A one- or two-cell room with its single door on its short end: opening the door
    /// shows all of it, so the bot looks in from the doorway and never walks inside —
    /// its furniture may fill the floor.
    pub fn peek_only(&self) -> bool {
        self.w * self.h <= 2
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Lock {
    pub cell: Cell,
    pub dir: usize,
    pub key: u8,
}

pub struct Maze {
    pub w: usize,
    pub h: usize,
    /// Per-cell bitmask: bit `d` set => passage open toward `DIRS[d]` (pure geometry —
    /// a locked door still has its bit set; a mirror does not). Symmetric across both
    /// cells of an edge.
    open: Vec<u8>,
    /// Per-cell bitmask: bit `d` set => that edge is a mirror (a special wall).
    mirror: Vec<u8>,
    /// Generated cells (`is_born`) — all of them except in Fog, which grows the maze
    /// lazily around the bot.
    born: Vec<bool>,
    /// Locked doors (sparse — at most a handful). Stored once per edge.
    pub locks: Vec<Lock>,
    /// Keys lying on the floor, `(cell, key id)` (Dungeon mode).
    pub keys: Vec<(Cell, u8)>,
    /// Office rooms and their doors (Tower mode). A sealed door's edge is a wall in
    /// `open`; an openable one's is open, like a locked door's.
    pub rooms: Vec<Room>,
    pub doors: Vec<Door>,
    /// Tower: the exit is a lift at the end of a one-cell lobby (`office`).
    pub lift: bool,
    /// Tower: this floor's boss is a woman (one floor in five) — her model, voice and
    /// half her retorts differ.
    pub lady_boss: bool,
    /// Mirror garden: the exit is a gap in the outer hedge, this way out of the exit
    /// cell (`place_gate`) — through it, open country to the horizon.
    pub gate: Option<usize>,
    /// Dungeon: the exit is a stairwell cut into this wall of the exit cell
    /// (`place_stairwell`) — the steps go down inside the wall square beyond it.
    /// Rendering only: the wall stays a wall to `Knowledge`, the solver and `is_open`.
    pub stairwell: Option<usize>,
    pub start: Cell,
    pub exit: Cell,
}

impl Maze {
    /// Tower and Dungeon: walls are `WALL_T` slabs (rendering and sight lines only —
    /// the maze, `Knowledge` and the solver don't care how thick a wall is).
    pub fn thin_walls(&self) -> bool {
        self.lift || !self.locks.is_empty() || self.stairwell.is_some()
    }

    /// Distance from a cell's centre to the face of any of its walls (`face_dist`) — the
    /// stairwell's arch face included.
    pub fn wall_face(&self) -> f32 {
        face_dist(self.thin_walls())
    }

    #[inline]
    pub fn idx(&self, c: Cell) -> usize {
        c.y as usize * self.w + c.x as usize
    }

    #[inline]
    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && y >= 0 && (x as usize) < self.w && (y as usize) < self.h
    }

    #[inline]
    pub fn cell_in_bounds(&self, c: Cell) -> bool {
        self.in_bounds(c.x, c.y)
    }

    /// Is there passage geometry from `c` toward `DIRS[d]`? (A locked door counts; a
    /// mirror does not.) For actual traversal use `passable`; for line of sight use
    /// `transparent`.
    #[inline]
    pub fn is_open(&self, c: Cell, d: usize) -> bool {
        self.open[self.idx(c)] & (1 << d) != 0
    }

    #[inline]
    pub fn is_mirror(&self, c: Cell, d: usize) -> bool {
        self.mirror[self.idx(c)] & (1 << d) != 0
    }

    pub fn lock_at(&self, c: Cell, d: usize) -> Option<u8> {
        let nb = c.step(d);
        self.locks.iter().find_map(|l| {
            if (l.cell == c && l.dir == d) || (l.cell == nb && l.dir == opposite(d)) {
                Some(l.key)
            } else {
                None
            }
        })
    }

    /// Index into `doors` of the office door on this edge, if any.
    pub fn door_at(&self, c: Cell, d: usize) -> Option<usize> {
        let nb = c.step(d);
        self.doors
            .iter()
            .position(|o| (o.cell == c && o.dir == d) || (o.cell == nb && o.dir == opposite(d)))
    }

    pub fn edge(&self, c: Cell, d: usize) -> EdgeKind {
        if !self.is_open(c, d) {
            return if self.is_mirror(c, d) {
                EdgeKind::Mirror
            } else if self.door_at(c, d).is_some() {
                EdgeKind::Sealed
            } else {
                EdgeKind::Wall
            };
        }
        if let Some(k) = self.lock_at(c, d) {
            return EdgeKind::Locked(k);
        }
        if self.door_at(c, d).is_some() {
            return EdgeKind::Door;
        }
        EdgeKind::Open
    }

    /// Can line of sight pass through this edge? (No, through a closed door or a mirror;
    /// an office door once it's been pushed open.)
    #[inline]
    pub fn transparent(&self, c: Cell, d: usize) -> bool {
        match self.edge(c, d) {
            EdgeKind::Open => true,
            EdgeKind::Door => self.door_at(c, d).is_some_and(|i| self.doors[i].open),
            _ => false,
        }
    }

    /// Is `c` inside a room whose every door is sealed? Nothing can reach it — the one
    /// kind of cell `bfs_from_start` allows to be cut off.
    pub fn sealed_off(&self, c: Cell) -> bool {
        self.rooms.iter().any(|r| {
            r.contains(c)
                && self
                    .doors
                    .iter()
                    .filter(|o| r.contains(o.cell))
                    .all(|o| o.sealed)
        })
    }

    /// Office floors: solid core between the corridors (stairs, shafts) — a cell in no
    /// room with no open side. Never reachable, never seen; `bfs_from_start` lets it be.
    pub fn is_core(&self, c: Cell) -> bool {
        !self.rooms.is_empty() && self.degree(c) == 0 && !self.rooms.iter().any(|r| r.contains(c))
    }

    /// Can the bot walk through this edge, holding `keys` (bit `k` set == holds key k)?
    pub fn passable(&self, c: Cell, d: usize, keys: u32) -> bool {
        match self.edge(c, d) {
            EdgeKind::Open | EdgeKind::Door => true,
            EdgeKind::Locked(k) => keys & (1 << k) != 0,
            EdgeKind::Wall | EdgeKind::Mirror | EdgeKind::Sealed => false,
        }
    }

    /// Has this cell been generated yet? Always true outside Fog mode. In Fog an unborn
    /// cell doesn't exist yet: an edge between two unborn cells is **undecided** (its
    /// open bit is 0, but it may yet be carved either way); an edge touching a born cell
    /// is decided.
    #[inline]
    pub fn is_born(&self, c: Cell) -> bool {
        self.born[self.idx(c)]
    }

    /// Fog mode: generate `cells` (unborn ones — born cells are skipped). Decides every
    /// undecided edge touching them, Kruskal-style: all decided-open edges and every
    /// edge still undecided afterwards (between two cells left unborn) count as
    /// connections, then the new edges are shuffled and each is carved where it joins
    /// two separate components, or with `braid / 2` odds otherwise.
    ///
    /// Invariant this keeps (induction from the fully-undecided start): the grid is
    /// connected when every undecided edge is counted open. So from the bot, any cell —
    /// the exit included — is either reachable over carved passages, or the route to it
    /// runs through a carved passage into unborn ground: generation never walls the bot
    /// in, it only decides which way the ground it hasn't reached yet goes.
    pub fn materialize(&mut self, cells: &[Cell], braid: f32) {
        let n = self.w * self.h;
        let mut new = vec![false; n];
        for &c in cells {
            if !self.is_born(c) {
                new[self.idx(c)] = true;
            }
        }
        if !new.iter().any(|&b| b) {
            return;
        }
        let mut parent: Vec<usize> = (0..n).collect();
        fn find(p: &mut [usize], mut i: usize) -> usize {
            while p[i] != i {
                p[i] = p[p[i]];
                i = p[i];
            }
            i
        }
        let mut cand: Vec<(Cell, usize)> = Vec::new();
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                for d in [1usize, 2] {
                    let nb = c.step(d);
                    if !self.cell_in_bounds(nb) {
                        continue;
                    }
                    let (i, j) = (self.idx(c), self.idx(nb));
                    let undecided = !self.born[i] && !self.born[j];
                    if undecided && (new[i] || new[j]) {
                        cand.push((c, d));
                        continue;
                    }
                    if self.is_open(c, d) || undecided {
                        let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                        parent[a] = b;
                    }
                }
            }
        }
        for i in (1..cand.len()).rev() {
            cand.swap(i, gen_range(0usize, i + 1));
        }
        for &(c, d) in &cand {
            let (a, b) = (
                find(&mut parent, self.idx(c)),
                find(&mut parent, self.idx(c.step(d))),
            );
            if a != b {
                self.carve(c, d);
                parent[a] = b;
            } else if gen_range(0.0f32, 1.0) < braid * 0.5 {
                self.carve(c, d);
            }
        }
        for (b, nw) in self.born.iter_mut().zip(&new) {
            *b |= *nw;
        }
    }

    /// Fog mode: un-generate every cell flagged in `gone`. Edges left between two unborn
    /// cells go back to undecided; an edge to a cell that's still born stays as it is
    /// (it belongs to that cell too). Only ever adds undecided edges, so the
    /// `materialize` connectivity invariant still holds.
    pub fn dissolve(&mut self, gone: &[bool]) {
        for (b, &g) in self.born.iter_mut().zip(gone) {
            *b &= !g;
        }
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                if !gone[self.idx(c)] {
                    continue;
                }
                for d in 0..4usize {
                    let nb = c.step(d);
                    if self.cell_in_bounds(nb) && !self.is_born(nb) && self.is_open(c, d) {
                        self.toggle_wall(c, d);
                    }
                }
            }
        }
    }

    /// Flip one edge open<->wall, both sides. No-op at the border.
    pub fn toggle_wall(&mut self, c: Cell, d: usize) {
        let b = c.step(d);
        if !self.cell_in_bounds(b) {
            return;
        }
        let (ic, ib) = (self.idx(c), self.idx(b));
        self.open[ic] ^= 1 << d;
        self.open[ib] ^= 1 << opposite(d);
    }

    fn set_mirror(&mut self, c: Cell, d: usize) {
        let b = c.step(d);
        let (ic, ib) = (self.idx(c), self.idx(b));
        self.mirror[ic] |= 1 << d;
        if self.cell_in_bounds(b) {
            self.mirror[ib] |= 1 << opposite(d);
        }
    }

    /// Number of open passages out of `c` (1 == dead end).
    #[inline]
    pub fn degree(&self, c: Cell) -> u32 {
        self.open[self.idx(c)].count_ones()
    }

    /// Open neighbors of `c` as `(dir, neighbor cell)`.
    pub fn neighbors(&self, c: Cell) -> impl Iterator<Item = (usize, Cell)> + '_ {
        (0..4usize)
            .filter(move |&d| self.is_open(c, d))
            .map(move |d| (d, c.step(d)))
    }

    /// Door `i` is shut for good (an occupied washroom the bot has tried): its edge
    /// becomes `Sealed` — no longer open or passable.
    pub fn seal(&mut self, i: usize) {
        let Door {
            cell: a, dir: d, ..
        } = self.doors[i];
        let b = a.step(d);
        let (ia, ib) = (self.idx(a), self.idx(b));
        self.open[ia] &= !(1 << d);
        self.open[ib] &= !(1 << opposite(d));
        self.doors[i].sealed = true;
    }

    /// Door `i` is one the bot looks through from the hall before deciding: a small
    /// room (`Room::peek_only`, seen whole from its door) or an unmarked one it leaves
    /// for last. A bigger room's door it only opens by walking in through it.
    pub fn door_peeked(&self, i: usize) -> bool {
        let cell = self.doors[i].cell;
        !self.door_marked(i)
            || self
                .rooms
                .iter()
                .find(|r| r.contains(cell))
                .is_none_or(|r| r.peek_only())
    }

    /// Door `i` has a name plate on it — every room's but a vacant one's. The bot can
    /// see that from the hall, and leaves unmarked doors for last.
    pub fn door_marked(&self, i: usize) -> bool {
        let cell = self.doors[i].cell;
        self.rooms
            .iter()
            .find(|r| r.contains(cell))
            .is_none_or(|r| !r.vacant)
    }

    /// Door `i` is a washroom's: the men's (the first washroom listed) or not.
    pub fn wc_door(&self, i: usize) -> Option<bool> {
        let cell = self.doors[i].cell;
        let mut wcs = self.rooms.iter().filter(|r| r.kind == RoomKind::Wc);
        let first = wcs.next()?;
        if first.contains(cell) {
            return Some(true);
        }
        wcs.any(|r| r.contains(cell)).then_some(false)
    }

    fn carve(&mut self, a: Cell, d: usize) {
        let b = a.step(d);
        let (ia, ib) = (self.idx(a), self.idx(b));
        self.open[ia] |= 1 << d;
        self.open[ib] |= 1 << opposite(d);
    }

    /// Shortest path length (in steps) from `start` to `exit` over open passages, and
    /// whether every cell is reachable from `start` (a room behind only sealed doors, and
    /// an office floor's solid core, excepted). Used only for tests and for the solver's optimal-ratio metric — never
    /// by the solver itself.
    pub fn bfs_from_start(&self) -> (Option<u32>, bool) {
        let dist = self.dist_from_start();
        let exit_d = dist[self.idx(self.exit)];
        let all = (0..dist.len()).all(|i| {
            let c = Cell {
                x: (i % self.w) as i32,
                y: (i / self.w) as i32,
            };
            dist[i] != u32::MAX || self.sealed_off(c) || self.is_core(c)
        });
        ((exit_d != u32::MAX).then_some(exit_d), all)
    }

    /// Step distance from `start` to every cell over open passages (`u32::MAX` =
    /// unreachable), indexed by `idx`.
    fn dist_from_start(&self) -> Vec<u32> {
        let mut dist = vec![u32::MAX; self.w * self.h];
        let mut q = std::collections::VecDeque::new();
        dist[self.idx(self.start)] = 0;
        q.push_back(self.start);
        while let Some(c) = q.pop_front() {
            let d0 = dist[self.idx(c)];
            for (_, nb) in self.neighbors(c) {
                let j = self.idx(nb);
                if dist[j] == u32::MAX {
                    dist[j] = d0 + 1;
                    q.push_back(nb);
                }
            }
        }
        dist
    }

    /// Put the exit on a random cell in the far half of the maze (by walking distance
    /// from `start`), so it's neither a fixed corner nor a trivial few steps away.
    fn place_exit(&mut self, walled: bool) {
        let dist = self.dist_from_start();
        let max = dist
            .iter()
            .copied()
            .filter(|&d| d != u32::MAX)
            .max()
            .unwrap_or(0);
        // Never in a peek-only room — the bot doesn't walk into those.
        let peek = |i: usize| {
            let c = Cell {
                x: (i % self.w) as i32,
                y: (i / self.w) as i32,
            };
            self.rooms.iter().any(|r| r.peek_only() && r.contains(c))
        };
        let far: Vec<usize> = (0..dist.len())
            .filter(|&i| dist[i] != u32::MAX && dist[i] * 2 >= max && dist[i] > 0 && !peek(i))
            .collect();
        let k = gen_range(0usize, far.len());
        // Dungeon (`walled`): the stairwell needs a wall to be cut into — a crossroads
        // cell passes the pick on to the next candidate (no extra draw, so any maze
        // whose pick was already walled comes out the same).
        let i = (0..far.len())
            .map(|j| far[(k + j) % far.len()])
            .find(|&i| !walled || self.open[i].count_ones() < 4)
            .unwrap_or(far[k]);
        self.exit = Cell {
            x: (i % self.w) as i32,
            y: (i / self.w) as i32,
        };
    }
}

/// Materialize the maze as a dense wall grid for the raycaster: a `(2w+1) x (2h+1)`
/// bool array (`true` == solid). Cell centers land on odd coordinates and are always
/// floor; even/even are pillars; an even/odd or odd/even coordinate is floor only where
/// the corresponding passage is open. Rebuilt once per maze.
pub struct WallGrid {
    pub gw: usize,
    pub gh: usize,
    solid: Vec<bool>,
    mirror: Vec<bool>,
    /// Locked doorway squares: `key id + 1`, 0 = none. Solid to the camera until the
    /// bot holds that key.
    door: Vec<u8>,
    /// Office doors, indexed like `Maze::doors`.
    pub panels: Vec<Panel>,
    /// The garden's gate is open: past the grid's edge is open ground, not wall.
    pub open_outside: bool,
    /// Dungeon's stairwell: the wall square `(gx, gy)` past the exit cell on side `d`.
    /// A wall like any other (from the corridor behind it, a plain one); a ray through
    /// its arch face from the exit side goes on into the flight behind the portal
    /// (`Well`, `raycast::march`) — the steps there are set dressing.
    pub stair: Option<(i32, i32, usize)>,
    /// Thin walls (`WALL_T` slabs, see `Maze::thin_walls`): a solid square is a slab
    /// along its run — a post plus an arm toward each `mass` neighbour — and the rest of
    /// it is floor.
    pub thin: bool,
    /// Squares a thin wall joins up with: solid ones, plus doorways (their frame is the
    /// wall across them).
    mass: Vec<bool>,
}

/// How far an office door's pivot sits in from its jamb. The panel still spans the
/// whole doorway when shut: it reaches back past the pivot by the same amount.
pub const PANEL_INSET: f32 = 0.06;

/// An office door as the raycaster sees it: a unit-long panel pivoting on `hinge`
/// (`PANEL_INSET` of it behind the pivot), lying along `closed` when shut and along
/// `open` (into the room) when fully open — it swings between the two.
#[derive(Clone, Copy, Debug)]
pub struct Panel {
    pub hinge: Vec2,
    pub closed: Vec2,
    pub open: Vec2,
}

/// Dungeon's flight of steps behind the stairwell's portal: a box of its own, reached
/// only through the arch face — `axis` (0 = x) the way in, `into` its sign, `arch` the
/// arch face's coordinate on that axis (the far wall is `STAIR_FLIGHT` on), `lo..hi`
/// across.
/// It overlaps the floor behind the wall; seen from there, it isn't there.
#[derive(Clone, Copy, Debug)]
pub struct Well {
    pub axis: usize,
    pub into: f32,
    pub arch: f32,
    pub lo: f32,
    pub hi: f32,
}

impl Well {
    /// How far past the arch face (along the way in), and across, point `p` is.
    pub fn local(&self, p: Vec2) -> (f32, f32) {
        let (a, c) = if self.axis == 0 {
            (p.x, p.y)
        } else {
            (p.y, p.x)
        };
        ((a - self.arch) * self.into, c)
    }

    /// Whether `p` is in the flight's box (past the arch, before the far wall).
    pub fn holds(&self, p: Vec2) -> bool {
        let (s, c) = self.local(p);
        s > 0.0 && s < STAIR_FLIGHT && c > self.lo && c < self.hi
    }
}

impl WallGrid {
    #[inline]
    pub fn out_of_bounds(&self, gx: i32, gy: i32) -> bool {
        gx < 0 || gy < 0 || gx as usize >= self.gw || gy as usize >= self.gh
    }

    #[inline]
    pub fn is_solid(&self, gx: i32, gy: i32) -> bool {
        if self.out_of_bounds(gx, gy) {
            return true;
        }
        self.solid[gy as usize * self.gw + gx as usize]
    }

    #[inline]
    pub fn is_mirror(&self, gx: i32, gy: i32) -> bool {
        if gx < 0 || gy < 0 || gx as usize >= self.gw || gy as usize >= self.gh {
            return false;
        }
        self.mirror[gy as usize * self.gw + gx as usize]
    }

    #[inline]
    pub fn door(&self, gx: i32, gy: i32) -> Option<u8> {
        if gx < 0 || gy < 0 || gx as usize >= self.gw || gy as usize >= self.gh {
            return None;
        }
        self.door[gy as usize * self.gw + gx as usize].checked_sub(1)
    }
    /// Grid coordinate of a cell's center square.
    #[inline]
    pub fn cell_center(c: Cell) -> (i32, i32) {
        (2 * c.x + 1, 2 * c.y + 1)
    }

    fn is_mass(&self, gx: i32, gy: i32) -> bool {
        !self.out_of_bounds(gx, gy) && self.mass[gy as usize * self.gw + gx as usize]
    }

    /// The stairwell's flight, if any, as the camera sees it behind the portal.
    pub fn well(&self) -> Option<Well> {
        let (sx, sy, d) = self.stair?;
        let (dx, dy) = DIRS[d];
        let axis = if dx != 0 { 0 } else { 1 };
        let into = (dx + dy) as f32;
        let (along, across) = if axis == 0 { (sx, sy) } else { (sy, sx) };
        let centre = along as f32 + 0.5;
        let half = if self.thin { WALL_T * 0.5 } else { 0.5 };
        let arch = centre - into * half;
        Some(Well {
            axis,
            into,
            arch,
            lo: across as f32,
            hi: across as f32 + 1.0,
        })
    }

    /// Is floor point `p` inside a wall — a full square, or a thin wall's slab (the
    /// floor either side of a slab is open)? Outside the grid counts as wall.
    pub fn solid_at(&self, p: Vec2) -> bool {
        let (gx, gy) = (p.x.floor() as i32, p.y.floor() as i32);
        if self.out_of_bounds(gx, gy) {
            return true;
        }
        match self.slabs(gx, gy) {
            Some(bars) => bars
                .iter()
                .any(|&(lo, hi)| p.cmpge(lo).all() && p.cmple(hi).all()),
            None => self.is_solid(gx, gy),
        }
    }

    /// A thin wall's solid square as its two bars, `(min, max)` each: the run along x
    /// and the run along y, both through the post in the middle (a bar with no arm is
    /// just the post). `None` when the square is a full block: not thin, or not solid.
    #[inline]
    pub fn slabs(&self, gx: i32, gy: i32) -> Option<[(Vec2, Vec2); 2]> {
        if !self.thin || !self.is_solid(gx, gy) {
            return None;
        }
        let h = WALL_T * 0.5;
        let c = vec2(gx as f32 + 0.5, gy as f32 + 0.5);
        let (x0, x1) = (
            if self.is_mass(gx - 1, gy) {
                gx as f32
            } else {
                c.x - h
            },
            if self.is_mass(gx + 1, gy) {
                gx as f32 + 1.0
            } else {
                c.x + h
            },
        );
        let (y0, y1) = (
            if self.is_mass(gx, gy - 1) {
                gy as f32
            } else {
                c.y - h
            },
            if self.is_mass(gx, gy + 1) {
                gy as f32 + 1.0
            } else {
                c.y + h
            },
        );
        Some([
            (vec2(x0, c.y - h), vec2(x1, c.y + h)),
            (vec2(c.x - h, y0), vec2(c.x + h, y1)),
        ])
    }

    /// Whether world point `p` is inside a wall as the camera draws it — thin slabs,
    /// the frame beside a doorway's opening (`DOOR_OPENING`, less `margin` either
    /// side), full squares otherwise.
    pub fn blocks(&self, p: Vec2, margin: f32) -> bool {
        let (gx, gy) = (p.x.floor() as i32, p.y.floor() as i32);
        if self.out_of_bounds(gx, gy) {
            return true;
        }
        if !self.thin {
            return self.is_solid(gx, gy);
        }
        if let Some(bars) = self.slabs(gx, gy) {
            return bars
                .iter()
                .any(|&(lo, hi)| p.cmpge(lo).all() && p.cmple(hi).all());
        }
        if self.is_solid(gx, gy) {
            return true;
        }
        if !self.is_mass(gx, gy) {
            return false;
        }
        // A doorway: its frame across the middle, but for the opening. A cell column
        // (odd x) means the doorway joins the cells above and below.
        let l = p - vec2(gx as f32 + 0.5, gy as f32 + 0.5);
        let (n, a) = if gx % 2 == 1 { (l.y, l.x) } else { (l.x, l.y) };
        n.abs() <= WALL_T * 0.5 && a.abs() > DOOR_OPENING * 0.5 - margin
    }
}

impl Maze {
    pub fn wall_grid(&self) -> WallGrid {
        let gw = 2 * self.w + 1;
        let gh = 2 * self.h + 1;
        let mut solid = vec![true; gw * gh];
        let mut mirror = vec![false; gw * gh];
        let mut door = vec![0u8; gw * gh];
        let mut panels = Vec::with_capacity(self.doors.len());
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                // Unborn ground (Fog) is solid: nothing there yet but forest.
                solid[(2 * y as usize + 1) * gw + (2 * x as usize + 1)] = !self.is_born(c);
                for (d, &(dx, dy)) in DIRS.iter().enumerate() {
                    let ex = (2 * x + 1 + dx) as usize;
                    let ey = (2 * y + 1 + dy) as usize;
                    // A locked doorway is an open square flagged as a door (the camera
                    // shuts it while the key is missing); a mirror is a solid wall
                    // flagged for the raycaster to reflect off.
                    if self.is_open(c, d) {
                        solid[ey * gw + ex] = false;
                        if let Some(k) = self.lock_at(c, d) {
                            door[ey * gw + ex] = k + 1;
                        }
                    } else if self.is_mirror(c, d) {
                        mirror[ey * gw + ex] = true;
                    }
                }
            }
        }
        // Rooms are open floor: no pillars between their cells.
        for r in &self.rooms {
            for gy in 2 * r.y + 2..2 * (r.y + r.h) {
                for gx in 2 * r.x + 2..2 * (r.x + r.w) {
                    solid[gy as usize * gw + gx as usize] = false;
                }
            }
        }
        // Office doorways are open squares; the door itself is a `Panel` the raycaster
        // intersects, so it can swing.
        for (i, o) in self.doors.iter().enumerate() {
            let (dx, dy) = DIRS[o.dir];
            let ex = 2 * o.cell.x + 1 + dx;
            let ey = 2 * o.cell.y + 1 + dy;
            solid[ey as usize * gw + ex as usize] = false;
            let (sq, room) = (vec2(ex as f32, ey as f32), vec2(-dx as f32, -dy as f32));
            // Hinge on one jamb (hashed off the door so it's stable), panel spanning the
            // doorway across its middle; it swings into the room.
            let across = vec2(dy.abs() as f32, dx.abs() as f32);
            let flip = (ex * 7 + ey * 13 + i as i32) % 2 == 0;
            let mid = sq + vec2(0.5, 0.5) - across * 0.5;
            let (hinge, closed) = if flip {
                (mid, across)
            } else {
                (mid + across, -across)
            };
            panels.push(Panel {
                // Pivot set into the frame: an open door then stands just in front of
                // the doorway's side wall. Hinged on the jamb itself it lay exactly in
                // the wall's plane and the two z-fought into vertical stripes.
                hinge: hinge + closed * PANEL_INSET,
                closed,
                open: room,
            });
        }
        // The garden gate: the outer hedge's square past the exit cell is open.
        if let Some(d) = self.gate {
            let (dx, dy) = DIRS[d];
            let (ex, ey) = (2 * self.exit.x + 1 + dx, 2 * self.exit.y + 1 + dy);
            solid[ey as usize * gw + ex as usize] = false;
        }
        let stair = self.stairwell.map(|d| {
            let (dx, dy) = DIRS[d];
            (2 * self.exit.x + 1 + dx, 2 * self.exit.y + 1 + dy, d)
        });
        let mut mass = solid.clone();
        for (i, m) in mass.iter_mut().enumerate() {
            *m |= door[i] != 0;
        }
        for o in &self.doors {
            let (dx, dy) = DIRS[o.dir];
            let (ex, ey) = (2 * o.cell.x + 1 + dx, 2 * o.cell.y + 1 + dy);
            mass[ey as usize * gw + ex as usize] = true;
        }
        WallGrid {
            gw,
            gh,
            solid,
            mirror,
            door,
            panels,
            open_outside: self.gate.is_some(),
            stair,
            thin: self.thin_walls(),
            mass,
        }
    }
}

/// Carve a perfect maze with randomized DFS (iterative — an explicit stack, so a large
/// maze can't blow the call stack), then braid away `braid` (0..1) of the dead ends.
/// `start` is the top-left cell; `exit` is a random cell in the far half (`place_exit`).
///
/// `locks > 0` (Dungeon mode) places doors + keys on the still-perfect maze, where every
/// edge of the start→exit path is a cut edge, and the braid then skips any knock that
/// would open a way around a door.
///
/// `rooms > 0` (Tower) is a different generator altogether: an office floor plan —
/// corridors, rooms and core, no DFS, no braid (`office`; `braid` and the count are
/// unused there).
pub fn generate(w: usize, h: usize, braid: f32, locks: usize, rooms: usize) -> Maze {
    if rooms > 0 {
        return office::generate(w, h);
    }
    let mut m = generate_empty(w, h);
    let mut visited = vec![false; w * h];
    let mut stack = vec![(m.start, usize::MAX)];
    visited[m.idx(m.start)] = true;

    while let Some(&(cur, came)) = stack.last() {
        let mut opts = [0usize; 4];
        let mut n = 0;
        for (d, &(dx, dy)) in DIRS.iter().enumerate() {
            let (nx, ny) = (cur.x + dx, cur.y + dy);
            if m.in_bounds(nx, ny) && !visited[ny as usize * w + nx as usize] {
                opts[n] = d;
                n += 1;
            }
        }
        if n == 0 {
            stack.pop();
            continue;
        }
        // A draw left from the DFS office halls' straight-on bias (never taken outside
        // the office): kept so every other mode's mazes stay exactly as they were.
        if opts[..n].contains(&came) {
            let _ = gen_range(0.0f32, 1.0);
        }
        let d = opts[gen_range(0usize, n)];
        m.carve(cur, d);
        let nb = cur.step(d);
        visited[m.idx(nb)] = true;
        stack.push((nb, d));
    }

    // Likewise the old room fitting's out-of-order-washroom draw.
    if gen_range(0.0f32, 1.0) < 0.5 {
        let _ = gen_range(0usize, 2);
    }
    m.place_exit(locks > 0);
    if locks > 0 {
        m.add_dungeon(locks);
    }

    if braid > 0.0 {
        for y in 0..h as i32 {
            for x in 0..w as i32 {
                let c = Cell { x, y };
                if m.degree(c) != 1 || gen_range(0.0f32, 1.0) >= braid {
                    continue;
                }
                // Knock a wall toward a random in-bounds neighbor we're not already
                // connected to — turns the dead end into a loop.
                let mut cand = [0usize; 4];
                let mut n = 0;
                for d in 0..4usize {
                    if m.is_open(c, d) {
                        continue;
                    }
                    let nb = c.step(d);
                    // Never the dungeon exit's last wall — its stairwell is cut into
                    // one.
                    let exit_walled = locks > 0 && nb == m.exit && m.degree(nb) >= 3;
                    if m.cell_in_bounds(nb) && !exit_walled {
                        cand[n] = d;
                        n += 1;
                    }
                }
                if n > 0 {
                    let d = cand[gen_range(0usize, n)];
                    m.carve(c, d);
                    if m.door_bypassed() {
                        m.toggle_wall(c, d);
                    }
                }
            }
        }
    }
    if locks > 0 {
        m.place_stairwell();
    }

    m
}

/// A `w x h` grid of walls, every cell born — `generate` carves into it.
fn generate_empty(w: usize, h: usize) -> Maze {
    assert!(w >= 2 && h >= 2);
    Maze {
        w,
        h,
        open: vec![0u8; w * h],
        mirror: vec![0u8; w * h],
        born: vec![true; w * h],
        locks: Vec::new(),
        keys: Vec::new(),
        rooms: Vec::new(),
        doors: Vec::new(),
        lift: false,
        lady_boss: false,
        gate: None,
        stairwell: None,
        start: Cell { x: 0, y: 0 },
        exit: Cell {
            x: w as i32 - 1,
            y: h as i32 - 1,
        },
    }
}

/// Fog mode: a `w x h` maze with nothing generated yet — every edge undecided, cells
/// appear through `Maze::materialize` as the bot nears them. Start top-left; the exit is
/// a random cell at least half the grid's span away (Manhattan — there are no passages
/// to measure a walk along yet).
pub fn unborn(w: usize, h: usize) -> Maze {
    let mut m = generate_empty(w, h);
    m.born = vec![false; w * h];
    let span = (w + h - 2) as i32;
    let far: Vec<Cell> = (0..(w * h) as i32)
        .map(|i| Cell {
            x: i % w as i32,
            y: i / w as i32,
        })
        .filter(|c| (c.x + c.y) * 2 >= span)
        .collect();
    m.exit = far[gen_range(0usize, far.len())];
    m
}

/// Tower: what each office room is for. Kinds by size, in placement order — no RNG draw,
/// so dressing rooms never changes the maze. The washrooms are already named.
fn name_rooms(rooms: &mut [Room]) {
    // Kinds by size, in placement order — no RNG draw, so dressing rooms never
    // changes the maze. One cell: the boss, then the janitor, then meeting rooms.
    // Two cells: the kitchen, then boardrooms. Bigger: open space.
    let (mut ones, mut twos) = (0, 0);
    for r in rooms.iter_mut().filter(|r| r.kind != RoomKind::Wc) {
        r.kind = match r.w * r.h {
            1 => {
                ones += 1;
                match ones {
                    1 => RoomKind::Boss,
                    2 => RoomKind::Janitor,
                    _ => RoomKind::Conference,
                }
            }
            // IT is a two-cell room: server racks and the Internet on one side,
            // Roy's and Moss's desks on the other.
            2 => {
                twos += 1;
                match twos {
                    1 => RoomKind::Kitchen,
                    2 => RoomKind::It,
                    _ => RoomKind::Conference,
                }
            }
            _ => RoomKind::OpenSpace,
        };
    }
}

impl Maze {
    /// Shortest path `start -> to` over `is_open` geometry, as a cell list including
    /// both ends (empty if unreachable). BFS parent-trace.
    pub fn path_to(&self, to: Cell) -> Vec<Cell> {
        let n = self.w * self.h;
        let mut prev = vec![usize::MAX; n];
        let mut seen = vec![false; n];
        let si = self.idx(self.start);
        seen[si] = true;
        let mut q = std::collections::VecDeque::new();
        q.push_back(self.start);
        while let Some(c) = q.pop_front() {
            if c == to {
                break;
            }
            for (_, nb) in self.neighbors(c) {
                let j = self.idx(nb);
                if !seen[j] {
                    seen[j] = true;
                    prev[j] = self.idx(c);
                    q.push_back(nb);
                }
            }
        }
        let ti = self.idx(to);
        if !seen[ti] {
            return Vec::new();
        }
        let mut path = vec![to];
        let mut cur = ti;
        while cur != si {
            cur = prev[cur];
            path.push(Cell {
                x: (cur % self.w) as i32,
                y: (cur / self.w) as i32,
            });
        }
        path.reverse();
        path
    }

    /// Some door can be walked around: with only that door shut, the exit is still
    /// reachable — a braid knock just opened a loop past it.
    fn door_bypassed(&self) -> bool {
        let exit_i = self.idx(self.exit);
        (0..self.locks.len()).any(|i| self.reachable_with(|j| j == i)[exit_i])
    }

    /// Cells reachable from `start` over open geometry while treating locks with index
    /// `>= barrier` as shut (locks below that are assumed already keyed).
    fn reachable_with_locks_below(&self, barrier: usize) -> Vec<bool> {
        self.reachable_with(|i| i >= barrier)
    }

    /// Cells reachable from `start` over open geometry, with lock `i` shut iff `shut(i)`.
    fn reachable_with(&self, shut: impl Fn(usize) -> bool) -> Vec<bool> {
        let n = self.w * self.h;
        let mut seen = vec![false; n];
        seen[self.idx(self.start)] = true;
        let mut q = std::collections::VecDeque::new();
        q.push_back(self.start);
        while let Some(c) = q.pop_front() {
            for d in 0..4usize {
                if !self.is_open(c, d) {
                    continue;
                }
                let blocked = self.locks.iter().enumerate().any(|(i, l)| {
                    shut(i)
                        && ((l.cell == c && l.dir == d)
                            || (l.cell == c.step(d) && l.dir == opposite(d)))
                });
                if blocked {
                    continue;
                }
                let nb = c.step(d);
                let j = self.idx(nb);
                if !seen[j] {
                    seen[j] = true;
                    q.push_back(nb);
                }
            }
        }
        seen
    }

    /// Dungeon mode: place `n_locks` doors along the solution path and a matching key
    /// for each in a region reachable before that door — solvable by construction
    /// (collect keys in path order, each opens the next barrier).
    fn add_dungeon(&mut self, n_locks: usize) {
        let path = self.path_to(self.exit);
        if path.len() < 6 || n_locks == 0 {
            return;
        }
        let n_locks = n_locks.min(4).min((path.len() - 2) / 3);
        // A door only goes on a path edge that, shut on its own, cuts the exit off — searched outward from its evenly spaced target
        // spot, and always past the previous door so the keys are needed in order. On
        // the perfect maze `generate` calls this on, the first candidate always cuts.
        let mut after = 0usize;
        for i in 0..n_locks {
            let frac = (i + 1) as f32 / (n_locks + 1) as f32;
            let target = ((path.len() - 1) as f32 * frac) as usize;
            let mut order: Vec<usize> = (after.max(1)..path.len() - 1).collect();
            crate::util::sort_by_key(&mut order, |&pi| pi.abs_diff(target) as f32);
            let exit_i = self.idx(self.exit);
            let placed = order.into_iter().find(|&pi| {
                let (a, b) = (path[pi], path[pi + 1]);
                let d = (0..4usize)
                    .find(|&d| a.step(d) == b)
                    .expect("path step is cardinal");
                self.locks.push(Lock {
                    cell: a,
                    dir: d,
                    key: i as u8,
                });
                let cuts = !self.reachable_with(|j| j == i)[exit_i];
                if !cuts {
                    self.locks.pop();
                }
                cuts
            });
            match placed {
                Some(pi) => after = pi + 1,
                None => break,
            }
        }
        let n_locks = self.locks.len();
        // Keys: for door i, anywhere reachable with doors < i open but not door i.
        for i in 0..n_locks {
            let before = self.reachable_with_locks_below(i);
            let on_path: std::collections::HashSet<usize> =
                path.iter().map(|&c| self.idx(c)).collect();
            let mut cands: Vec<Cell> = (0..self.w * self.h)
                .filter(|&j| before[j] && !on_path.contains(&j))
                .map(|j| Cell {
                    x: (j % self.w) as i32,
                    y: (j / self.w) as i32,
                })
                .collect();
            if cands.is_empty() {
                // Fall back to any reachable cell that isn't start.
                cands = (0..self.w * self.h)
                    .filter(|&j| before[j] && j != self.idx(self.start))
                    .map(|j| Cell {
                        x: (j % self.w) as i32,
                        y: (j / self.w) as i32,
                    })
                    .collect();
            }
            // One key per cell: two floating in the same spot read as one. If every
            // off-path cell before this lock already has a key, the path ones will do.
            let taken: Vec<Cell> = self.keys.iter().map(|k| k.0).collect();
            cands.retain(|c| !taken.contains(c));
            if cands.is_empty() {
                cands = (0..self.w * self.h)
                    .filter(|&j| before[j] && j != self.idx(self.start))
                    .map(|j| Cell {
                        x: (j % self.w) as i32,
                        y: (j / self.w) as i32,
                    })
                    .filter(|c| !taken.contains(c))
                    .collect();
            }
            if cands.is_empty() {
                continue;
            }
            // Prefer a dead end so the key sits at the end of a side branch.
            let dead_ends: Vec<Cell> = cands
                .iter()
                .copied()
                .filter(|&c| self.degree(c) == 1)
                .collect();
            let pool = if dead_ends.is_empty() {
                &cands
            } else {
                &dead_ends
            };
            let pick = pool[gen_range(0usize, pool.len())];
            self.keys.push((pick, i as u8));
        }
    }

    /// Mirror garden: move the exit to a cell on the outer hedge in the far half of the
    /// garden (by walking distance) and open a gate in the hedge there.
    pub fn place_gate(&mut self) {
        let dist = self.dist_from_start();
        let max = dist
            .iter()
            .copied()
            .filter(|&d| d != u32::MAX)
            .max()
            .unwrap_or(0);
        let border: Vec<(Cell, usize)> = (0..self.w * self.h)
            .filter(|&i| dist[i] != u32::MAX && dist[i] * 2 >= max && dist[i] > 0)
            .filter_map(|i| {
                let c = Cell {
                    x: (i % self.w) as i32,
                    y: (i / self.w) as i32,
                };
                (0..4)
                    .find(|&d| !self.cell_in_bounds(c.step(d)))
                    .map(|d| (c, d))
            })
            .collect();
        if !border.is_empty() {
            let (c, d) = border[gen_range(0usize, border.len())];
            self.exit = c;
            self.gate = Some(d);
        }
    }

    /// Dungeon: pick the exit cell's wall the stairwell is cut into — the one straight
    /// ahead of the shortest route's last step if that's a wall (the bot walks in facing
    /// it), else a side wall, else any. No RNG: the maze stream is untouched. `None` if
    /// the exit cell has no plain wall at all — `generate` keeps one for it (the braid
    /// never opens the exit's last wall), so only a hand-built maze lacks it.
    pub fn place_stairwell(&mut self) {
        let path = self.path_to(self.exit);
        let ahead = path
            .len()
            .checked_sub(2)
            .and_then(|i| (0..4).find(|&d| path[i].step(d) == self.exit));
        let wall = |d: usize| self.edge(self.exit, d) == EdgeKind::Wall;
        self.stairwell = match ahead {
            Some(a) => [a, (a + 1) % 4, (a + 3) % 4, opposite(a)]
                .into_iter()
                .find(|&d| wall(d)),
            None => (0..4).find(|&d| wall(d)),
        };
    }

    /// Tower: the exit is a lift (see `office`); its one open side faces this way.
    pub fn lift_door(&self) -> Option<usize> {
        if !self.lift {
            return None;
        }
        (0..4).find(|&d| self.is_open(self.exit, d))
    }

    /// Mirror mode: turn `n` corridor-facing walls into mirrors — a wall the bot's line
    /// of sight will read as an opening. Chosen at dead ends / straight runs so the lie
    /// looks like a real branch.
    pub fn add_mirrors(&mut self, n: usize) {
        let mut placed = 0;
        let mut tries = 0;
        while placed < n && tries < n * 20 + 400 {
            tries += 1;
            let c = Cell {
                x: gen_range(0i32, self.w as i32),
                y: gen_range(0i32, self.h as i32),
            };
            // Only where the bot actually travels: a cell it can stand in with a wall
            // opposite an existing opening (so the mirror faces down a corridor).
            let openings: Vec<usize> = (0..4).filter(|&d| self.is_open(c, d)).collect();
            if openings.is_empty() {
                continue;
            }
            let want = opposite(openings[gen_range(0usize, openings.len())]);
            if self.is_open(c, want)
                || self.is_mirror(c, want)
                || !self.cell_in_bounds(c.step(want))
            {
                continue;
            }
            self.set_mirror(c, want);
            placed += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeded(seed: u64, braid: f32) -> Maze {
        let _g = crate::rng_guard();
        macroquad::rand::srand(seed);
        generate(24, 18, braid, 0, 0)
    }

    /// `solid_at` among thin walls: a wall square's slab is solid, the floor either
    /// side of it open; cells always open; off the grid solid.
    #[test]
    fn solid_at_knows_the_slabs() {
        let m = {
            let _g = crate::rng_guard();
            macroquad::rand::srand(3);
            generate(24, 18, 0.2, 3, 0)
        };
        let g = m.wall_grid();
        assert!(m.thin_walls());
        let mut strips = 0;
        for gy in 0..g.gh as i32 {
            for gx in 0..g.gw as i32 {
                let c = vec2(gx as f32 + 0.5, gy as f32 + 0.5);
                if !g.is_solid(gx, gy) {
                    assert!(!g.solid_at(c));
                    continue;
                }
                if let Some(bars) = g.slabs(gx, gy) {
                    assert!(g.solid_at(c), "slab post at {gx},{gy}");
                    // A corner of the square outside both bars is floor.
                    let corner = vec2(gx as f32 + 0.05, gy as f32 + 0.05);
                    let inside = bars
                        .iter()
                        .any(|&(lo, hi)| corner.cmpge(lo).all() && corner.cmple(hi).all());
                    assert_eq!(g.solid_at(corner), inside);
                    strips += (!inside) as u32;
                }
            }
        }
        assert!(strips > 50, "{strips}");
        assert!(g.solid_at(vec2(-0.5, 3.0)));
    }

    #[test]
    fn every_door_is_required() {
        for seed in 0..60u64 {
            let m = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                generate(24, 18, 0.2, 3, 0)
            };
            assert!(!m.locks.is_empty(), "seed {seed}: no doors placed");
            let exit_i = m.idx(m.exit);
            for i in 0..m.locks.len() {
                let alone = m.reachable_with(|j| j == i);
                assert!(!alone[exit_i], "seed {seed}: door {i} can be walked around");
                let (kc, _) = m.keys[i];
                let before = m.reachable_with_locks_below(i);
                assert!(
                    before[m.idx(kc)],
                    "seed {seed}: key {i} behind its own door"
                );
            }
        }
    }

    /// Office rooms: walled except for their doors, one or two on one wall, and never
    /// a shortcut — the hall stays connected with every room cell removed.
    #[test]
    fn rooms_are_detours() {
        for seed in 0..60u64 {
            let m = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                generate(25, 17, 0.2, 0, 5)
            };
            assert!(!m.rooms.is_empty(), "seed {seed}: no rooms");
            let (exit_d, all) = m.bfs_from_start();
            assert!(all && exit_d.is_some(), "seed {seed}: unreachable cells");
            assert!(!m.sealed_off(m.start) && !m.sealed_off(m.exit));
            let in_room = |c: Cell| m.rooms.iter().any(|r| r.contains(c));
            for r in &m.rooms {
                let doors: Vec<&Door> = m.doors.iter().filter(|o| r.contains(o.cell)).collect();
                assert!((1..=2).contains(&doors.len()), "seed {seed}: {r:?}");
                assert!(doors.iter().all(|o| o.dir == doors[0].dir), "seed {seed}");
                for y in r.y..r.y + r.h {
                    for x in r.x..r.x + r.w {
                        let c = Cell { x, y };
                        for d in 0..4 {
                            let nb = c.step(d);
                            if m.cell_in_bounds(nb) && !r.contains(nb) && m.is_open(c, d) {
                                assert!(m.door_at(c, d).is_some(), "seed {seed}: hole in {r:?}");
                            }
                        }
                    }
                }
            }
            // The hall alone is one piece.
            let hall: Vec<Cell> = (0..(m.w * m.h) as i32)
                .map(|i| Cell {
                    x: i % m.w as i32,
                    y: i / m.w as i32,
                })
                // Office core (solid, walled in) isn't hall.
                .filter(|&c| !in_room(c) && !m.is_core(c))
                .collect();
            let mut seen = vec![false; m.w * m.h];
            let mut q = vec![m.start];
            seen[m.idx(m.start)] = true;
            while let Some(c) = q.pop() {
                for (_, nb) in m.neighbors(c) {
                    if !in_room(nb) && !seen[m.idx(nb)] {
                        seen[m.idx(nb)] = true;
                        q.push(nb);
                    }
                }
            }
            assert!(
                hall.iter().all(|&c| seen[m.idx(c)]),
                "seed {seed}: hall split by a room"
            );
        }
    }

    /// No meeting-room conceit is furnished more than twice on one floor.
    #[test]
    fn meeting_layouts_at_most_twice() {
        for seed in 0..200u64 {
            let g = (seed % 10) as usize;
            let m = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                generate(15 + g * 2, 11 + g + g / 2, 0.0, 0, 1)
            };
            let lived: Vec<_> = m
                .rooms
                .iter()
                .filter(|r| !r.vacant)
                .filter_map(Room::meeting_layout)
                .collect();
            for l in &lived {
                let n = lived.iter().filter(|&o| o == l).count();
                assert!(n <= 2, "seed {seed} gen {g}: {n} × {l:?}");
            }
        }
    }

    /// Office floors get their washrooms: two one-cell rooms side by side, each with
    /// its own door, neither door leading into the other, the doors side by side on the
    /// same wall and the hall outside them joined.
    #[test]
    fn washrooms_back_to_back() {
        let mut pairs = 0;
        for seed in 0..40u64 {
            let m = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                generate(25, 17, 0.2, 0, 12)
            };
            let wc: Vec<&Room> = m.rooms.iter().filter(|r| r.kind == RoomKind::Wc).collect();
            if wc.is_empty() {
                continue;
            }
            assert_eq!(wc.len(), 2, "seed {seed}");
            let (a, b) = (wc[0], wc[1]);
            assert_eq!(
                (a.x - b.x).abs() + (a.y - b.y).abs(),
                1,
                "seed {seed}: not adjacent"
            );
            for r in [a, b] {
                let door = m.doors.iter().find(|o| r.contains(o.cell));
                let door = door.unwrap_or_else(|| panic!("seed {seed}: washroom with no door"));
                let out = door.cell.step(door.dir);
                assert!(!m.rooms.iter().any(|o| o.contains(out)), "seed {seed}");
            }
            let door = |r: &Room| *m.doors.iter().find(|o| r.contains(o.cell)).unwrap();
            let (da, db) = (door(a), door(b));
            assert_eq!(da.dir, db.dir, "seed {seed}: doors on different walls");
            let (oa, ob) = (da.cell.step(da.dir), db.cell.step(db.dir));
            let d = (0..4).find(|&k| oa.step(k) == ob);
            assert!(
                d.is_some_and(|d| m.is_open(oa, d)),
                "seed {seed}: hall outside the doors not joined"
            );
            pairs += 1;
        }
        assert!(pairs >= 30, "only {pairs}/40 floors got washrooms");
    }

    /// Never two keys in one cell.
    #[test]
    fn keys_in_distinct_cells() {
        for seed in 0..60u64 {
            let m = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                generate(
                    15 + (seed % 8) as usize * 2,
                    11,
                    0.2,
                    2 + (seed % 5) as usize,
                    0,
                )
            };
            for (i, a) in m.keys.iter().enumerate() {
                for b in &m.keys[i + 1..] {
                    assert_ne!(a.0, b.0, "seed {seed}: two keys in {:?}", a.0);
                }
            }
        }
    }

    #[test]
    fn same_seed_same_maze() {
        let a = seeded(42, 0.2);
        let b = seeded(42, 0.2);
        for y in 0..a.h as i32 {
            for x in 0..a.w as i32 {
                let c = Cell { x, y };
                for d in 0..4 {
                    assert_eq!(a.is_open(c, d), b.is_open(c, d), "cell {c:?} dir {d}");
                }
            }
        }
    }

    #[test]
    fn different_seed_different_maze() {
        let a = seeded(1, 0.2);
        let b = seeded(2, 0.2);
        let differ = (0..a.h as i32).any(|y| {
            (0..a.w as i32).any(|x| {
                let c = Cell { x, y };
                (0..4).any(|d| a.is_open(c, d) != b.is_open(c, d))
            })
        });
        assert!(differ);
    }

    #[test]
    fn fully_connected_and_exit_reachable() {
        for seed in 0..30u64 {
            let m = seeded(seed, 0.0);
            let (exit_d, all) = m.bfs_from_start();
            assert!(all, "seed {seed}: not every cell reachable");
            assert!(exit_d.is_some(), "seed {seed}: exit unreachable");
        }
    }

    #[test]
    fn braiding_adds_loops() {
        // A perfect maze on N cells has exactly N-1 passages; braiding adds more.
        let perfect = seeded(7, 0.0);
        let braided = seeded(7, 0.3);
        let count = |m: &Maze| -> u32 {
            (0..m.h as i32)
                .map(|y| {
                    (0..m.w as i32)
                        .map(|x| m.degree(Cell { x, y }))
                        .sum::<u32>()
                })
                .sum::<u32>()
        };
        assert!(count(&braided) > count(&perfect));
    }

    /// An office floor plan, not a maze: every two hall cells side by side are joined
    /// and no 2x2 square is all hall — corridors are one cell wide and never run
    /// alongside each other with only a wall between; the hall is one piece; the only
    /// dead ends are the entrance, the lift lobby and at most two nooks; rooms and core
    /// fill the rest.
    #[test]
    fn office_is_a_floor_plan() {
        let (mut cores, mut cells, mut ends) = (0, 0, 0);
        for seed in 0..60u64 {
            let g = (seed % 9) as usize;
            let (w, h) = (15 + g * 2, 11 + g + g / 2);
            let m = {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                generate(w, h, 0.0, 0, 1)
            };
            let hall = |c: Cell| {
                m.cell_in_bounds(c) && !m.is_core(c) && !m.rooms.iter().any(|r| r.contains(c))
            };
            let all: Vec<Cell> = (0..(w * h) as i32)
                .map(|i| Cell {
                    x: i % w as i32,
                    y: i / w as i32,
                })
                .collect();
            let (exit_d, reach) = m.bfs_from_start();
            assert!(reach && exit_d.is_some(), "seed {seed}: unreachable");
            assert!(m.lift && hall(m.start) && hall(m.exit), "seed {seed}");
            let mut dead = 0;
            for &c in all.iter().filter(|&&c| hall(c)) {
                for d in 0..4 {
                    if hall(c.step(d)) {
                        assert!(
                            m.is_open(c, d),
                            "seed {seed}: hall {c:?} walled off dir {d}"
                        );
                    }
                }
                let square = [(1, 0), (0, 1), (1, 1)].iter().all(|&(dx, dy)| {
                    hall(Cell {
                        x: c.x + dx,
                        y: c.y + dy,
                    })
                });
                assert!(
                    !square,
                    "seed {seed}: 2x2 hall at {c:?}\n{}",
                    office::ascii(&m)
                );
                let n = (0..4).filter(|&d| hall(c.step(d))).count();
                assert!(n >= 1, "seed {seed}: lone hall cell {c:?}");
                if n == 1 {
                    assert!(
                        c == m.start || c == m.exit || m.degree(c) == 1,
                        "seed {seed}: {c:?}"
                    );
                    // One cell long (the entrance aside): off a junction.
                    let nb = (0..4).map(|d| c.step(d)).find(|&nb| hall(nb)).unwrap();
                    let fork = (0..4).filter(|&d| hall(nb.step(d))).count();
                    assert!(c == m.start || fork >= 3, "seed {seed}: long stub at {c:?}");
                    dead += 1;
                }
            }
            assert!(
                dead <= 4,
                "seed {seed}: {dead} dead ends\n{}",
                office::ascii(&m)
            );
            assert_eq!(m.degree(m.exit), 1, "seed {seed}: lift not a dead end");
            ends += dead;
            cores += all.iter().filter(|&&c| m.is_core(c)).count();
            cells += w * h;
        }
        println!("dead ends {ends}/60 floors, core {cores}/{cells} cells");
        assert!(cores * 10 < cells, "too much core: {cores}/{cells}");
    }

    /// Prints a few office floors: `cargo test -p labyrinth --release -- --ignored
    /// office_floors --nocapture`. `@` start, `L` lift, `.` room, `D`/`x` open/sealed
    /// door, `%` core.
    #[test]
    #[ignore]
    fn office_floors() {
        for (seed, generation) in [(1u64, 1usize), (2, 1), (3, 4), (4, 8)] {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let g = generation;
            let m = generate(15 + g * 2, 11 + g + g / 2, 0.0, 0, 1);
            let sealed = m.doors.iter().filter(|o| o.sealed).count();
            println!(
                "seed {seed} gen {generation}: {} rooms, {} doors ({sealed} sealed)\n{}",
                m.rooms.len(),
                m.doors.len(),
                office::ascii(&m)
            );
        }
    }

    /// Tower walk stats (steps / optimal, win rate, steps) over 40 seeds at a few
    /// generations, plus a fingerprint of the non-office modes' mazes and walks — run
    /// before and after a generator change: `cargo test -p labyrinth --release --
    /// --ignored office_stats --nocapture`.
    #[test]
    #[ignore]
    fn office_stats() {
        use crate::game::{Game, Mode, Outcome};
        for generation in [1u32, 4, 8] {
            let (mut ratios, mut steps, mut wins) = (Vec::new(), Vec::new(), 0);
            for seed in 0..40u64 {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                let mut g = Game::with_mode(Mode::TowerClimb, generation);
                while g.tick() {}
                wins += (g.outcome == Some(Outcome::Won)) as u32;
                ratios.push(g.optimal_ratio());
                steps.push(g.steps);
            }
            ratios.sort_by(|a, b| a.total_cmp(b));
            steps.sort();
            println!(
                "tower gen {generation}: wins {wins}/40 ratio median {:.2} p90 {:.2} steps median {}",
                ratios[20], ratios[36], steps[20]
            );
        }
        let mut fp: u64 = 0;
        for mode in [Mode::DungeonCrawl, Mode::Fog, Mode::Mirror] {
            for seed in 0..20u64 {
                let _g = crate::rng_guard();
                macroquad::rand::srand(seed);
                let mut g = Game::with_mode(mode, 1 + (seed % 5) as u32);
                for y in 0..g.maze.h as i32 {
                    for x in 0..g.maze.w as i32 {
                        for d in 0..4 {
                            let e = match g.maze.edge(Cell { x, y }, d) {
                                EdgeKind::Wall => 0,
                                EdgeKind::Open => 1,
                                EdgeKind::Mirror => 2,
                                EdgeKind::Locked(k) => 3 + k as u64,
                                EdgeKind::Door => 90,
                                EdgeKind::Sealed => 91,
                            };
                            fp = fp.wrapping_mul(0x100_0000_01b3).wrapping_add(e + 1);
                        }
                    }
                }
                while g.tick() {}
                fp = fp
                    .wrapping_mul(0x100_0000_01b3)
                    .wrapping_add(g.steps as u64);
            }
        }
        println!("other modes fingerprint {fp:016x}");
    }
}
