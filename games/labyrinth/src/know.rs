//! `Knowledge` — everything the explorer has learned about the maze. The minimap
//! renders this and nothing else; the solver reads this and nothing else (see the
//! invariant in `.notes/labyrinth_plan.md`). `Maze` is never handed to either.
//!
//! `Game` owns the per-tick visibility feed (`reveal`) — the one place `Maze` and
//! `Knowledge` meet.

use crate::maze::{Cell, Maze, opposite};

/// Per-cell edge state, packed: `known` bit set => we've seen whether that edge is a
/// wall or a passage; `open` bit set => it's a passage (only meaningful when `known`).
#[derive(Clone, Copy, Default)]
struct Edges {
    known: u8,
    open: u8,
}

pub struct Knowledge {
    pub w: usize,
    pub h: usize,
    edges: Vec<Edges>,
    /// Tremaux marks: how many times the bot has stood on each cell.
    pub visits: Vec<u16>,
    /// Dead-end-filled cells: fully explored and leading nowhere new. BFS won't route
    /// through them, so the bot stops re-walking spent corridors.
    pub pruned: Vec<bool>,
    /// Where the bot is (always known) and which way it faces (`DIRS` index).
    pub pos: Cell,
    pub facing: usize,
    pub exit_seen: Option<Cell>,
    /// Set for the frame a reveal turned up something new — lets the minimap cache
    /// know when to redraw.
    pub dirty: bool,
}

impl Knowledge {
    pub fn new(w: usize, h: usize, start: Cell) -> Self {
        Knowledge {
            w,
            h,
            edges: vec![Edges::default(); w * h],
            visits: vec![0; w * h],
            pruned: vec![false; w * h],
            pos: start,
            facing: 2, // south — into the maze from the top-left start
            exit_seen: None,
            dirty: true,
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

    /// `Some(true)` known passage, `Some(false)` known wall, `None` never seen.
    #[inline]
    pub fn edge_open(&self, c: Cell, d: usize) -> Option<bool> {
        let e = self.edges[self.idx(c)];
        (e.known & (1 << d) != 0).then_some(e.open & (1 << d) != 0)
    }

    #[inline]
    pub fn cell_seen(&self, c: Cell) -> bool {
        self.edges[self.idx(c)].known != 0
    }

    /// A cell we've seen that still has at least one unseen edge — the explorer's
    /// candidate goals. (Mutually exclusive with `pruned`, which requires every edge
    /// known, so no `pruned` check is needed here.)
    pub fn is_frontier(&self, c: Cell) -> bool {
        let e = self.edges[self.idx(c)];
        e.known != 0 && e.known != 0b1111
    }

    pub fn visits(&self, c: Cell) -> u16 {
        self.visits[self.idx(c)]
    }

    /// Known-open neighbors of `c` (regardless of pruning).
    pub fn open_neighbors(&self, c: Cell) -> impl Iterator<Item = (usize, Cell)> + '_ {
        (0..4usize)
            .filter(move |&d| self.edge_open(c, d) == Some(true))
            .map(move |d| (d, c.step(d)))
    }

    fn learn_edge(&mut self, c: Cell, d: usize, open: bool) {
        let i = self.idx(c);
        let before = self.edges[i].known;
        self.edges[i].known |= 1 << d;
        if open {
            self.edges[i].open |= 1 << d;
        }
        if before != self.edges[i].known {
            self.dirty = true;
        }
    }

    /// Record every edge of `c` from ground truth (both sides, so the neighbor learns
    /// the shared wall too).
    fn reveal_cell(&mut self, maze: &Maze, c: Cell) {
        for d in 0..4usize {
            let open = maze.is_open(c, d);
            self.learn_edge(c, d, open);
            let nb = c.step(d);
            if self.in_bounds(nb) {
                self.learn_edge(nb, opposite(d), open);
            }
        }
        if c == maze.exit && self.exit_seen.is_none() {
            self.exit_seen = Some(c);
            self.dirty = true;
        }
    }

    /// The visibility feed: from the bot's cell, look straight down every open corridor
    /// up to `radius` cells, revealing each cell fully. A corridor maze means line of
    /// sight is "see down the passage until it walls off or turns".
    pub fn reveal(&mut self, maze: &Maze, radius: i32) {
        let bot = self.pos;
        self.reveal_cell(maze, bot);
        for d in 0..4usize {
            if !maze.is_open(bot, d) {
                continue;
            }
            let mut c = bot;
            for _ in 0..radius {
                c = c.step(d);
                if !maze.in_bounds(c.x, c.y) {
                    break;
                }
                self.reveal_cell(maze, c);
                // Stop at a wall ahead or at a bend (corridor no longer runs straight).
                if !maze.is_open(c, d) {
                    break;
                }
            }
        }
        self.update_pruning();
    }

    /// Reverse-reachability dead-end fill: a seen cell is *pruned* if it can't reach any
    /// frontier or the exit over known-open edges without routing through `start`. That
    /// collapses every spent corridor (including a fully-walked branch off the start)
    /// while keeping every cell that still bridges the bot to something unexplored — a
    /// simpler local "≤1 live neighbor" rule wrongly severs a frontier reachable only
    /// through a chain of dead-end-looking cells.
    fn update_pruning(&mut self) {
        let n = self.w * self.h;
        let start_i = self.idx(Cell { x: 0, y: 0 });
        let mut keep = vec![false; n];
        let mut q = std::collections::VecDeque::new();

        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                let i = self.idx(c);
                if self.is_frontier(c) || self.exit_seen == Some(c) {
                    keep[i] = true;
                    q.push_back(c);
                }
            }
        }
        while let Some(c) = q.pop_front() {
            for (_, nb) in self.open_neighbors(c) {
                let j = self.idx(nb);
                if keep[j] {
                    continue;
                }
                keep[j] = true;
                // `start` is a valid destination but not a valid transit — a spent
                // branch off it must not keep itself alive by reaching back to start.
                if j != start_i {
                    q.push_back(nb);
                }
            }
        }
        keep[start_i] = true;

        for (i, p) in self.pruned.iter_mut().enumerate() {
            *p = self.edges[i].known != 0 && !keep[i];
        }
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
