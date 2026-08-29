//! `Knowledge` — everything the explorer has learned about the maze. The minimap
//! renders this and nothing else; the solver reads this and nothing else (see the
//! invariant in `.notes/labyrinth_plan.md`). `Maze` is never handed to either.
//!
//! `Game` owns the per-tick visibility feed (`reveal`) — the one place `Maze` and
//! `Knowledge` meet.

use crate::maze::{Cell, EdgeKind, Maze, opposite};

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
        let before = (e.known, e.open, e.locked, e.phantom);
        e.known |= bit;
        // Overwrite (Fog mode mutates edges), so always clear then set.
        e.open &= !bit;
        e.locked &= !bit;
        e.phantom &= !bit;
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
        }
        if before != (e.known, e.open, e.locked, e.phantom) {
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
            let nb = c.step(d);
            if self.in_bounds(nb) {
                // A mirror is only knowable from the side the bot is looking at — don't
                // leak the wall onto the far cell (it stays fully unknown, so the near
                // cell reads as a frontier and the bot walks at the glass).
                match kind {
                    EdgeKind::Mirror => {}
                    EdgeKind::Locked(k) => self.learn(nb, opposite(d), EdgeKind::Locked(k)),
                    EdgeKind::Open => self.learn(nb, opposite(d), EdgeKind::Open),
                    EdgeKind::Wall => self.learn(nb, opposite(d), EdgeKind::Wall),
                }
            }
        }
        let sees_exit = c == maze.exit
            || (0..4).any(|d| matches!(maze.edge(c, d), EdgeKind::Open) && c.step(d) == maze.exit);
        if sees_exit && self.exit_seen != Some(maze.exit) {
            self.exit_seen = Some(maze.exit);
            self.dirty = true;
        }
        // Spot a key on this cell *or* glimpsed through an open edge into an adjacent
        // one — a dead-end pocket can otherwise become fully "known" from its four
        // neighbours' reveals without its contents ever being seen.
        self.spot_keys(maze, c);
        for d in 0..4usize {
            if matches!(maze.edge(c, d), EdgeKind::Open) {
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
        self.exit_sight = exit_sight;
        self.update_pruning();
    }

    /// Fog mode: the bot tried to step through an edge it remembered as open, but the
    /// maze mutated a wall in behind it. Correct the memory and re-plan.
    pub fn stale_memory_wall(&mut self, c: Cell, d: usize) {
        self.learn(c, d, EdgeKind::Wall);
        let nb = c.step(d);
        if self.in_bounds(nb) {
            self.learn(nb, opposite(d), EdgeKind::Wall);
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
                if self.is_frontier(c) || self.exit_seen == Some(c) || self.has_locked_edge(c) {
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

    /// Best guess at the direction the bot will head next — used only to aim the idle
    /// "look down the corridor" so the view isn't frozen on a wall between steps.
    pub fn look_hint(&self) -> usize {
        let from = self.pos;
        let back = crate::maze::opposite(self.facing);
        let mut best: Option<usize> = None;
        let mut best_score = (2u8, u16::MAX);
        for (d, nb) in self.open_neighbors(from) {
            if d == back || self.pruned[self.idx(nb)] {
                continue;
            }
            let rank = if self.is_frontier(nb) || self.exit_seen == Some(nb) {
                0
            } else {
                1
            };
            let s = (rank, self.visits(nb));
            if s < best_score {
                best_score = s;
                best = Some(d);
            }
        }
        best.unwrap_or(back)
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
