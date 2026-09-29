//! `Knowledge` — everything the explorer has learned about the maze. The minimap
//! renders this and nothing else; the solver reads this and nothing else (see the
//! invariant in `.notes/labyrinth_plan.md`). `Maze` is never handed to either.
//!
//! `Game` owns the per-tick visibility feed (`reveal`) — the one place `Maze` and
//! `Knowledge` meet.

use crate::maze::{Cell, DIRS, EdgeKind, Maze, WallGrid, opposite};
use macroquad::math::vec2;

/// Per-cell edge state. `known`/`open` are bitmasks over the 4 dirs; `mirror`/`locked`/
/// `phantom` flag special edges; `key[d]` holds the key id for a `locked` edge.
#[derive(Clone, Copy, Default)]
struct Edges {
    known: u8,
    open: u8,
    locked: u8,
    /// A mirror lie: the bot currently believes this edge is an open passage. Cleared
    /// (and `open` cleared, wall set) when the bot bumps the glass.
    phantom: u8,
    /// A mirror the bot has physically walked into — permanently known to be a wall, so
    /// re-revealing the cell doesn't fool it a second time.
    busted: u8,
    /// Office doors (Tower mode): `door` one the bot can push open, `sealed` one that
    /// never opens (also a wall — `open` is clear).
    door: u8,
    sealed: u8,
    /// An openable office door that's still shut (the bot will open it to look in).
    shut: u8,
    /// ...and has no name plate: until it's opened, the bot treats it as a wall — a
    /// last resort, tried only when there's nothing else left to explore.
    plain: u8,
    key: [u8; 4],
}

pub struct Knowledge {
    pub w: usize,
    pub h: usize,
    edges: Vec<Edges>,
    /// Tremaux marks: how many times the bot has stood on each cell.
    pub visits: Vec<u16>,
    /// Tick each cell was last confirmed by line of sight (Fog mode: the minimap fades
    /// cells as this ages, and past a threshold they revert to unknown).
    pub stale: Vec<u32>,
    /// Dead-end-filled cells: fully explored and leading nowhere new. BFS won't route
    /// through them, so the bot stops re-walking spent corridors.
    pub pruned: Vec<bool>,
    /// Where the bot is (always known) and which way it faces (`DIRS` index).
    pub pos: Cell,
    pub facing: usize,
    pub exit_seen: Option<Cell>,
    /// `Some(chebyshev distance)` when the exit is in the bot's current line of sight
    /// (recomputed every reveal) — drives the exit-glow effect.
    pub exit_sight: Option<i32>,
    /// Tower: the lift won't take the bot until the boss is defeated — the exit is known
    /// but not a destination (and not a cell to route through) while this is set.
    pub exit_locked: bool,
    /// Bit `k` set => the bot holds key `k` (Dungeon mode).
    pub keys_held: u32,
    /// Keys spotted on the floor but not yet collected: `(cell, key id)`.
    pub keys_seen: Vec<(Cell, u8)>,
    /// Set for the frame a reveal turned up something new — lets the minimap cache
    /// know when to redraw.
    pub dirty: bool,
    /// Current tick (bumped by `Game` before each `reveal`).
    now: u32,
}

/// From `bot`'s cell centre, is cell `c`'s centre in sight through the doorway in
/// direction `d` — through its `DOOR_OPENING`-wide gap (less a margin for the leaf and
/// lining), past the walls as the camera draws them (`WallGrid::blocks`: thin slabs in
/// the Tower)?
fn sees_through_door(grid: &WallGrid, bot: Cell, d: usize, c: Cell) -> bool {
    let centre = |c: Cell| {
        let (gx, gy) = WallGrid::cell_center(c);
        vec2(gx as f32 + 0.5, gy as f32 + 0.5)
    };
    let (eye, to) = (centre(bot), centre(c));
    let v = to - eye;
    let n = vec2(DIRS[d].0 as f32, DIRS[d].1 as f32);
    // Beyond the doorway's middle (one unit out).
    if v.dot(n) <= 1.0 {
        return false;
    }
    let steps = ((v.x.abs() + v.y.abs()) / 0.02) as i32;
    (1..steps).all(|i| !grid.blocks(eye + v * (i as f32 / steps as f32), 0.04))
}

impl Knowledge {
    pub fn new(w: usize, h: usize, start: Cell) -> Self {
        Knowledge {
            w,
            h,
            edges: vec![Edges::default(); w * h],
            visits: vec![0; w * h],
            stale: vec![0; w * h],
            pruned: vec![false; w * h],
            pos: start,
            facing: 2, // south — into the maze from the top-left start
            exit_seen: None,
            exit_sight: None,
            exit_locked: false,
            keys_held: 0,
            keys_seen: Vec::new(),
            dirty: true,
            now: 0,
        }
    }

    #[inline]
    pub fn idx(&self, c: Cell) -> usize {
        c.y as usize * self.w + c.x as usize
    }

    #[inline]
    pub fn in_bounds(&self, c: Cell) -> bool {
        c.x >= 0 && c.y >= 0 && (c.x as usize) < self.w && (c.y as usize) < self.h
    }

    /// `Some(true)` the bot believes it can walk this edge, `Some(false)` it can't,
    /// `None` never seen. A phantom (mirror lie) reads `Some(true)`; a locked door
    /// reads `Some(true)` only once the key is held.
    #[inline]
    pub fn edge_open(&self, c: Cell, d: usize) -> Option<bool> {
        let e = self.edges[self.idx(c)];
        let bit = 1u8 << d;
        if e.known & bit == 0 {
            return None;
        }
        if e.phantom & bit != 0 {
            return Some(true);
        }
        if e.locked & bit != 0 {
            return Some(self.keys_held & (1 << e.key[d]) != 0);
        }
        if e.shut & e.plain & bit != 0 {
            return Some(false);
        }
        Some(e.open & bit != 0)
    }

    /// Is this edge a locked door the bot has seen but can't yet open?
    pub fn edge_locked(&self, c: Cell, d: usize) -> Option<u8> {
        let e = self.edges[self.idx(c)];
        let bit = 1u8 << d;
        if e.known & bit != 0 && e.locked & bit != 0 && self.keys_held & (1 << e.key[d]) == 0 {
            Some(e.key[d])
        } else {
            None
        }
    }

    /// An office door the bot has seen on this edge: `Some(sealed)`.
    pub fn edge_door(&self, c: Cell, d: usize) -> Option<bool> {
        let e = self.edges[self.idx(c)];
        let bit = 1u8 << d;
        if e.known & bit == 0 {
            None
        } else if e.door & bit != 0 {
            Some(false)
        } else if e.sealed & bit != 0 {
            Some(true)
        } else {
            None
        }
    }

    /// Direction of a still-shut office door on `c`'s edges, if the bot knows of one.
    pub fn shut_door(&self, c: Cell) -> Option<usize> {
        let e = self.edges[self.idx(c)];
        (0..4).find(|&d| e.known & e.shut & (1 << d) != 0)
    }

    fn set_shut(&mut self, c: Cell, d: usize, shut: bool, plain: bool) {
        let i = self.idx(c);
        let e = &mut self.edges[i];
        let before = (e.shut, e.plain);
        for (bits, on) in [(&mut e.shut, shut), (&mut e.plain, plain)] {
            if on {
                *bits |= 1 << d;
            } else {
                *bits &= !(1 << d);
            }
        }
        if before != (e.shut, e.plain) {
            self.dirty = true;
        }
    }

    /// Direction of a still-shut unmarked door on `c`'s edges, if the bot knows of one.
    pub fn plain_door(&self, c: Cell) -> Option<usize> {
        let e = self.edges[self.idx(c)];
        (0..4).find(|&d| e.known & e.shut & e.plain & (1 << d) != 0)
    }

    /// Direction of a still-shut door with a name plate on `c`'s edges.
    pub fn marked_door(&self, c: Cell) -> Option<usize> {
        let e = self.edges[self.idx(c)];
        (0..4).find(|&d| e.known & e.shut & !e.plain & (1 << d) != 0)
    }

    pub fn edge_phantom(&self, c: Cell, d: usize) -> bool {
        self.edges[self.idx(c)].phantom & (1u8 << d) != 0
    }

    #[inline]
    pub fn cell_seen(&self, c: Cell) -> bool {
        self.edges[self.idx(c)].known != 0
    }

    /// A cell the bot has confirmed a way *into* (at least one believed-open edge) that
    /// still has an unexplored edge — the explorer's candidate goals. A cell glimpsed
    /// only through a wall edge is *not* a frontier: there's no way in, so routing to it
    /// would just fail.
    pub fn is_frontier(&self, c: Cell) -> bool {
        let e = self.edges[self.idx(c)];
        if e.known == 0 || e.known == 0b1111 {
            return false;
        }
        // Any believed-open edge (plain open, phantom, or an unlocked door)?
        (0..4).any(|d| self.edge_open(c, d) == Some(true))
    }

    pub fn visits(&self, c: Cell) -> u16 {
        self.visits[self.idx(c)]
    }

    /// Age (ticks) since this cell was last confirmed.
    pub fn age(&self, c: Cell) -> u32 {
        self.now.saturating_sub(self.stale[self.idx(c)])
    }

    /// Neighbors the bot believes it can currently walk to.
    pub fn open_neighbors(&self, c: Cell) -> impl Iterator<Item = (usize, Cell)> + '_ {
        (0..4usize)
            .filter(move |&d| self.edge_open(c, d) == Some(true))
            .map(move |d| (d, c.step(d)))
    }

    fn learn(&mut self, c: Cell, d: usize, kind: EdgeKind) {
        let i = self.idx(c);
        let bit = 1u8 << d;
        let e = &mut self.edges[i];
        let before = (e.known, e.open, e.locked, e.phantom, e.door, e.sealed);
        e.known |= bit;
        // Overwrite (Fog mode mutates edges), so always clear then set.
        e.open &= !bit;
        e.locked &= !bit;
        e.phantom &= !bit;
        e.door &= !bit;
        e.sealed &= !bit;
        // A mirror the bot has already walked into stays a known wall.
        let kind = if e.busted & bit != 0 && kind == EdgeKind::Mirror {
            EdgeKind::Wall
        } else {
            kind
        };
        match kind {
            EdgeKind::Open => e.open |= bit,
            EdgeKind::Wall => {}
            EdgeKind::Mirror => {
                // The lie: record it as an opening the bot will try to take.
                e.open |= bit;
                e.phantom |= bit;
            }
            EdgeKind::Locked(k) => {
                e.open |= bit;
                e.locked |= bit;
                e.key[d] = k;
            }
            EdgeKind::Door => {
                e.open |= bit;
                e.door |= bit;
            }
            EdgeKind::Sealed => e.sealed |= bit,
        }
        if before != (e.known, e.open, e.locked, e.phantom, e.door, e.sealed) {
            self.dirty = true;
        }
    }

    /// The bot walked into a mirror it believed was a passage: correct the map, and
    /// mark the edge busted so re-revealing the cell can't fool it again.
    pub fn hit_mirror(&mut self, c: Cell, d: usize) {
        let ci = self.idx(c);
        self.edges[ci].busted |= 1u8 << d;
        self.learn(c, d, EdgeKind::Wall);
        let nb = c.step(d);
        if self.in_bounds(nb) {
            let ni = self.idx(nb);
            self.edges[ni].busted |= 1u8 << opposite(d);
            self.learn(nb, opposite(d), EdgeKind::Wall);
        }
        self.dirty = true;
        self.update_pruning();
    }

    fn reveal_cell(&mut self, maze: &Maze, c: Cell) {
        let ci = self.idx(c);
        self.stale[ci] = self.now;
        for d in 0..4usize {
            let kind = maze.edge(c, d);
            self.learn(c, d, kind);
            // A door closed behind the bot on a room it's been through isn't one to open.
            // A door to open from the hall and look through; a big room's (an open
            // space's) is just a way in, pushed open by walking through it.
            let shut = kind == EdgeKind::Door
                && !maze.transparent(c, d)
                && maze
                    .door_at(c, d)
                    .is_none_or(|i| !maze.doors[i].done && maze.door_peeked(i));
            let plain = shut && maze.door_at(c, d).is_some_and(|i| !maze.door_marked(i));
            self.set_shut(c, d, shut, plain);
            let nb = c.step(d);
            if self.in_bounds(nb) {
                self.set_shut(nb, opposite(d), shut, plain);
            }
            if self.in_bounds(nb) {
                // A mirror is only knowable from the side the bot is looking at — don't
                // leak the wall onto the far cell (it stays fully unknown, so the near
                // cell reads as a frontier and the bot walks at the glass).
                match kind {
                    EdgeKind::Mirror => {}
                    k => self.learn(nb, opposite(d), k),
                }
            }
        }
        let sees_exit =
            c == maze.exit || (0..4).any(|d| maze.transparent(c, d) && c.step(d) == maze.exit);
        if sees_exit && self.exit_seen != Some(maze.exit) {
            self.exit_seen = Some(maze.exit);
            self.dirty = true;
        }
        // Spot a key on this cell *or* glimpsed through an open edge into an adjacent
        // one — a dead-end pocket can otherwise become fully "known" from its four
        // neighbours' reveals without its contents ever being seen.
        self.spot_keys(maze, c);
        for d in 0..4usize {
            if maze.transparent(c, d) {
                self.spot_keys(maze, c.step(d));
            }
        }
    }

    fn spot_keys(&mut self, maze: &Maze, at: Cell) {
        for &(kc, k) in &maze.keys {
            if kc == at && self.keys_held & (1 << k) == 0 && !self.keys_seen.contains(&(kc, k)) {
                self.keys_seen.push((kc, k));
                self.dirty = true;
            }
        }
    }

    /// The visibility feed: from the bot's cell, look straight down every open corridor
    /// up to `radius` cells, revealing each cell fully. Stops at a wall, a bend, a
    /// closed door, or a mirror.
    pub fn reveal(&mut self, maze: &Maze, radius: i32, tick: u32) {
        self.now = tick;
        let bot = self.pos;

        // Pick up a key the bot is standing on.
        for &(kc, k) in &maze.keys {
            if kc == bot && self.keys_held & (1 << k) == 0 {
                self.keys_held |= 1 << k;
                self.keys_seen.retain(|&(c, _)| c != kc);
                self.dirty = true;
            }
        }

        let mut exit_sight = if bot == maze.exit { Some(0) } else { None };
        // `reveal_cell(bot)` already recorded all four of the bot's own edges — any
        // adjacent door or mirror included. The march only extends down clear corridors.
        self.reveal_cell(maze, bot);
        for d in 0..4usize {
            if !maze.transparent(bot, d) {
                continue;
            }
            let mut c = bot;
            for _ in 0..radius {
                c = c.step(d);
                if !maze.in_bounds(c.x, c.y) {
                    break;
                }
                if c == maze.exit {
                    exit_sight = Some((c.x - bot.x).abs().max((c.y - bot.y).abs()));
                }
                self.reveal_cell(maze, c);
                if !maze.transparent(c, d) {
                    break;
                }
            }
        }
        // An office room is open plan (every inside edge open, no pillars): standing in
        // it, the bot takes in the whole room at a glance — no need to walk every cell
        // of an open space to map it. From the hall at its open door it sees only what
        // the doorway shows: the cells whose centre is in sight through the opening,
        // past the walls either side — the rest it has to step in to look at.
        if let Some(r) = maze.rooms.iter().find(|r| r.contains(bot)) {
            for c in r.cells() {
                self.reveal_cell(maze, c);
            }
        }
        let mut grid: Option<WallGrid> = None;
        for d in 0..4 {
            if maze.door_at(bot, d).is_none() || !maze.transparent(bot, d) {
                continue;
            }
            let behind = bot.step(d);
            let Some(r) = maze.rooms.iter().find(|r| r.contains(behind)) else {
                continue;
            };
            let grid = grid.get_or_insert_with(|| maze.wall_grid());
            for c in r.cells() {
                if sees_through_door(grid, bot, d, c) {
                    self.reveal_cell(maze, c);
                }
            }
        }
        self.exit_sight = exit_sight;
        self.update_pruning();
    }

    /// Fog's forest: the "walls" are trees, and the camera sees between the trunks into
    /// the corridors either side. Everything within `radius` cells (Euclidean) of the
    /// bot that exists is revealed too, walls or no walls, so the map shows what the view
    /// does. `reveal` must run first (it sets the tick).
    pub fn glimpse(&mut self, maze: &Maze, radius: f32) {
        let bot = self.pos;
        let r = radius.ceil() as i32;
        for y in bot.y - r..=bot.y + r {
            for x in bot.x - r..=bot.x + r {
                let c = Cell { x, y };
                let (dx, dy) = ((x - bot.x) as f32, (y - bot.y) as f32);
                if maze.cell_in_bounds(c) && maze.is_born(c) && dx.hypot(dy) <= radius {
                    self.reveal_cell(maze, c);
                }
            }
        }
        self.update_pruning();
    }

    /// Fog mode: wipe every cell flagged in `forgot` back to unknown — the maze is
    /// about to dissolve that ground. An edge shared with a cell that's still
    /// remembered survives (the maze keeps it too: it belongs to a cell that still
    /// exists), copied onto the forgotten side, so an open one leaves the forgotten cell
    /// a frontier into new ground and a wall stays a wall. Clearing those too made a fake
    /// frontier out of every remembered wall along the edge of memory: the bot walked
    /// back, found the same wall, left, forgot it, and came back again.
    pub fn forget(&mut self, forgot: &[bool]) {
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                let i = self.idx(c);
                if !forgot[i] {
                    continue;
                }
                let mut e = Edges::default();
                for d in 0..4usize {
                    let nb = c.step(d);
                    if !self.in_bounds(nb) || forgot[self.idx(nb)] {
                        continue;
                    }
                    let (o, bit, obit) = (self.edges[self.idx(nb)], 1u8 << d, 1u8 << opposite(d));
                    if o.known & obit == 0 {
                        continue;
                    }
                    e.known |= bit;
                    for (dst, src) in [
                        (&mut e.open, o.open),
                        (&mut e.locked, o.locked),
                        (&mut e.phantom, o.phantom),
                        (&mut e.busted, o.busted),
                        (&mut e.door, o.door),
                        (&mut e.sealed, o.sealed),
                        (&mut e.shut, o.shut),
                        (&mut e.plain, o.plain),
                    ] {
                        if src & obit != 0 {
                            *dst |= bit;
                        }
                    }
                    e.key[d] = o.key[opposite(d)];
                }
                self.edges[i] = e;
                self.visits[i] = 0;
            }
        }
        if let Some(e) = self.exit_seen
            && forgot[self.idx(e)]
        {
            self.exit_seen = None;
        }
        self.dirty = true;
        self.update_pruning();
    }

    /// Prune a seen cell if it can't reach a frontier / the exit / a seen key / a
    /// locked door over believed-open edges. The bot's own cell is always a keep-source,
    /// so pruning can never strand it — it collapses spent side branches without cutting
    /// the route the bot still needs.
    fn update_pruning(&mut self) {
        let n = self.w * self.h;
        let mut keep = vec![false; n];
        let mut q = std::collections::VecDeque::new();
        let push = |c: Cell, keep: &mut Vec<bool>, q: &mut std::collections::VecDeque<Cell>| {
            let i = c.y as usize * self.w + c.x as usize;
            if !keep[i] {
                keep[i] = true;
                q.push_back(c);
            }
        };

        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                if self.is_frontier(c)
                    || self.exit_seen == Some(c)
                    || self.has_locked_edge(c)
                    || self.shut_door(c).is_some()
                {
                    push(c, &mut keep, &mut q);
                }
            }
        }
        for &(kc, _) in &self.keys_seen {
            push(kc, &mut keep, &mut q);
        }
        push(self.pos, &mut keep, &mut q);
        push(Cell { x: 0, y: 0 }, &mut keep, &mut q);

        while let Some(c) = q.pop_front() {
            for (_, nb) in self.open_neighbors(c) {
                let j = self.idx(nb);
                if !keep[j] {
                    keep[j] = true;
                    q.push_back(nb);
                }
            }
        }

        for (i, p) in self.pruned.iter_mut().enumerate() {
            *p = self.edges[i].known != 0 && !keep[i];
        }
    }

    fn has_locked_edge(&self, c: Cell) -> bool {
        let e = self.edges[self.idx(c)];
        (0..4).any(|d| e.locked & (1 << d) != 0 && self.keys_held & (1 << e.key[d]) == 0)
    }

    pub fn record_step(&mut self, to: Cell, facing: usize) {
        self.pos = to;
        self.facing = facing;
        let i = self.idx(to);
        self.visits[i] = self.visits[i].saturating_add(1);
    }

    /// Fraction of cells the bot has seen — reported by `--once`.
    pub fn coverage(&self) -> f32 {
        let seen = self.edges.iter().filter(|e| e.known != 0).count();
        seen as f32 / (self.w * self.h) as f32
    }
}

/// N/E/S/W display names, indexed by `DIRS`.
pub const DIR_NAMES: [&str; 4] = ["N", "E", "S", "W"];
