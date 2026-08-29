//! Maze generation: seed -> deterministic grid maze.
//!
//! Randomized-DFS carve of a perfect maze, then a **braid** pass that removes a
//! fraction of dead ends by knocking one extra wall. Perfect mazes make wall-follow
//! trivially optimal and dead-end backtracking monotonous; loops are what turn the
//! explorer's frontier choice into a real decision (see `.notes/labyrinth_plan.md`).
//!
//! Determinism: generation draws only from `macroquad::rand`, so `rand::srand(seed)`
//! before `generate()` makes the same seed produce the same maze — asserted in tests.

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
    /// Locked doors (sparse — at most a handful). Stored once per edge.
    pub locks: Vec<Lock>,
    /// Keys lying on the floor, `(cell, key id)` (Dungeon mode).
    pub keys: Vec<(Cell, u8)>,
    pub start: Cell,
    pub exit: Cell,
}

impl Maze {
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

    pub fn edge(&self, c: Cell, d: usize) -> EdgeKind {
        if !self.is_open(c, d) {
            return if self.is_mirror(c, d) {
                EdgeKind::Mirror
            } else {
                EdgeKind::Wall
            };
        }
        match self.lock_at(c, d) {
            Some(k) => EdgeKind::Locked(k),
            None => EdgeKind::Open,
        }
    }

    /// Can line of sight pass through this edge? (No, through a closed door or a mirror.)
    #[inline]
    pub fn transparent(&self, c: Cell, d: usize) -> bool {
        matches!(self.edge(c, d), EdgeKind::Open)
    }

    /// Can the bot walk through this edge, holding `keys` (bit `k` set == holds key k)?
    pub fn passable(&self, c: Cell, d: usize, keys: u32) -> bool {
        match self.edge(c, d) {
            EdgeKind::Open => true,
            EdgeKind::Locked(k) => keys & (1 << k) != 0,
            EdgeKind::Wall | EdgeKind::Mirror => false,
        }
    }

    /// Flip one edge open<->wall, both sides (Fog mode mutation). No-op at the border.
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

    fn carve(&mut self, a: Cell, d: usize) {
        let b = a.step(d);
        let (ia, ib) = (self.idx(a), self.idx(b));
        self.open[ia] |= 1 << d;
        self.open[ib] |= 1 << opposite(d);
    }

    /// Shortest path length (in steps) from `start` to `exit` over open passages, and
    /// whether every cell is reachable from `start`. Used only for tests and for the
    /// solver's optimal-ratio metric — never by the solver itself.
    pub fn bfs_from_start(&self) -> (Option<u32>, bool) {
        let mut dist = vec![u32::MAX; self.w * self.h];
        let mut q = std::collections::VecDeque::new();
        dist[self.idx(self.start)] = 0;
        q.push_back(self.start);
        let mut seen = 1usize;
        while let Some(c) = q.pop_front() {
            let d0 = dist[self.idx(c)];
            for (_, nb) in self.neighbors(c) {
                let j = self.idx(nb);
                if dist[j] == u32::MAX {
                    dist[j] = d0 + 1;
                    seen += 1;
                    q.push_back(nb);
                }
            }
        }
        let exit_d = dist[self.idx(self.exit)];
        (
            (exit_d != u32::MAX).then_some(exit_d),
            seen == self.w * self.h,
        )
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
}

impl WallGrid {
    #[inline]
    pub fn is_solid(&self, gx: i32, gy: i32) -> bool {
        if gx < 0 || gy < 0 || gx as usize >= self.gw || gy as usize >= self.gh {
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
    /// Grid coordinate of a cell's center square.
    #[inline]
    pub fn cell_center(c: Cell) -> (i32, i32) {
        (2 * c.x + 1, 2 * c.y + 1)
    }
}

impl Maze {
    pub fn wall_grid(&self) -> WallGrid {
        let gw = 2 * self.w + 1;
        let gh = 2 * self.h + 1;
        let mut solid = vec![true; gw * gh];
        let mut mirror = vec![false; gw * gh];
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                solid[(2 * y as usize + 1) * gw + (2 * x as usize + 1)] = false;
                for (d, &(dx, dy)) in DIRS.iter().enumerate() {
                    let ex = (2 * x + 1 + dx) as usize;
                    let ey = (2 * y + 1 + dy) as usize;
                    // A locked doorway is geometrically open (you see a passage); a
                    // mirror is a solid wall flagged for the raycaster to reflect off.
                    if self.is_open(c, d) {
                        solid[ey * gw + ex] = false;
                    } else if self.is_mirror(c, d) {
                        mirror[ey * gw + ex] = true;
                    }
                }
            }
        }
        WallGrid {
            gw,
            gh,
            solid,
            mirror,
        }
    }
}

/// Carve a perfect maze with randomized DFS (iterative — an explicit stack, so a large
/// maze can't blow the call stack), then braid away `braid` (0..1) of the dead ends.
/// `start` is the top-left cell, `exit` the bottom-right.
pub fn generate(w: usize, h: usize, braid: f32) -> Maze {
    assert!(w >= 2 && h >= 2);
    let mut m = Maze {
        w,
        h,
        open: vec![0u8; w * h],
        mirror: vec![0u8; w * h],
        locks: Vec::new(),
        keys: Vec::new(),
        start: Cell { x: 0, y: 0 },
        exit: Cell {
            x: w as i32 - 1,
            y: h as i32 - 1,
        },
    };

    let mut visited = vec![false; w * h];
    let mut stack = vec![m.start];
    visited[m.idx(m.start)] = true;

    while let Some(&cur) = stack.last() {
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
        let d = opts[gen_range(0usize, n)];
        m.carve(cur, d);
        let nb = cur.step(d);
        visited[m.idx(nb)] = true;
        stack.push(nb);
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
                    if m.cell_in_bounds(nb) {
                        cand[n] = d;
                        n += 1;
                    }
                }
                if n > 0 {
                    m.carve(c, cand[gen_range(0usize, n)]);
                }
            }
        }
    }

    m
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

    /// Cells reachable from `start` over open geometry while treating locks with index
    /// `>= barrier` as shut (locks below that are assumed already keyed).
    fn reachable_with_locks_below(&self, barrier: usize) -> Vec<bool> {
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
                // A lock at index >= barrier blocks; below barrier is passable.
                let blocked = self.locks.iter().enumerate().any(|(i, l)| {
                    i >= barrier
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
    pub fn add_dungeon(&mut self, n_locks: usize) {
        let path = self.path_to(self.exit);
        if path.len() < 6 || n_locks == 0 {
            return;
        }
        let n_locks = n_locks.min(4).min((path.len() - 2) / 3);
        for i in 0..n_locks {
            // Space the doors down the path (avoid the first/last cell).
            let frac = (i + 1) as f32 / (n_locks + 1) as f32;
            let pi = ((path.len() - 1) as f32 * frac) as usize;
            let (a, b) = (path[pi], path[pi + 1]);
            let d = (0..4usize)
                .find(|&d| a.step(d) == b)
                .expect("path step is cardinal");
            self.locks.push(Lock {
                cell: a,
                dir: d,
                key: i as u8,
            });
        }
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

    /// Mirror mode: turn `n` corridor-facing walls into mirrors — a wall the bot's line
    /// of sight will read as an opening. Chosen at dead ends / straight runs so the lie
    /// looks like a real branch.
    pub fn add_mirrors(&mut self, n: usize) {
        let mut placed = 0;
        let mut tries = 0;
        while placed < n && tries < 400 {
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
        generate(24, 18, braid)
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
}
