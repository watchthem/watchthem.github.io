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

pub struct Maze {
    pub w: usize,
    pub h: usize,
    /// Per-cell bitmask: bit `d` set => passage open toward `DIRS[d]`. A wall between
    /// two cells is recorded on *both* cells (symmetric).
    open: Vec<u8>,
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

    /// Is there a passage from `c` toward `DIRS[d]`?
    #[inline]
    pub fn is_open(&self, c: Cell, d: usize) -> bool {
        self.open[self.idx(c)] & (1 << d) != 0
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
}

impl WallGrid {
    #[inline]
    pub fn is_solid(&self, gx: i32, gy: i32) -> bool {
        if gx < 0 || gy < 0 || gx as usize >= self.gw || gy as usize >= self.gh {
            return true;
        }
        self.solid[gy as usize * self.gw + gx as usize]
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
        for y in 0..self.h as i32 {
            for x in 0..self.w as i32 {
                let c = Cell { x, y };
                solid[(2 * y as usize + 1) * gw + (2 * x as usize + 1)] = false;
                for d in [1usize, 2] {
                    // East / South openings carve the shared edge square.
                    if self.is_open(c, d) {
                        let ex = (2 * x + 1 + DIRS[d].0) as usize;
                        let ey = (2 * y + 1 + DIRS[d].1) as usize;
                        solid[ey * gw + ex] = false;
                    }
                }
            }
        }
        WallGrid { gw, gh, solid }
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
