//! The explorer. Picks one cardinal step per tick from `Knowledge` alone — it never
//! sees `Maze`. That single signature (`choose_move(&Knowledge) -> Option<usize>`) is
//! what makes "the minimap shows exactly what the bot knows" true by construction.
//!
//! Strategy:
//! 1. If the exit has been seen and is reachable over known-open, non-pruned edges,
//!    head straight for it (BFS shortest known path).
//! 2. Otherwise head for the nearest frontier cell (a seen cell with an unseen edge),
//!    again by BFS over known-open, non-pruned edges. Tremaux visit counts break ties
//!    so the bot spreads out instead of grinding one branch.
//! 3. No frontier and no exit reachable => `None` (the episode ends `Stuck`; with full
//!    corridor line-of-sight in a connected maze this is effectively unreachable).

use crate::know::Knowledge;
use crate::maze::Cell;
use std::collections::VecDeque;

#[derive(Default)]
pub struct Solver;

impl Solver {
    pub fn new() -> Self {
        Solver
    }

    /// The chosen `DIRS` index to step, or `None` if nowhere useful is reachable.
    pub fn choose_move(&mut self, k: &Knowledge) -> Option<usize> {
        // 1. Make for the exit if we've seen it and can get there.
        if let Some(exit) = k.exit_seen
            && let Some(dir) = self.first_step_toward(k, |c| c == exit)
        {
            return Some(dir);
        }
        // 2. Otherwise the nearest unexplored frontier.
        self.first_step_toward(k, |c| k.is_frontier(c))
    }

    /// BFS from the bot over known-open, non-pruned edges to the nearest cell matching
    /// `goal`; return the first step of that path. Among equal-distance goals the one
    /// reached along the lowest-visit path wins (Tremaux tie-break), so the explorer
    /// favours ground it has trodden less.
    fn first_step_toward(&self, k: &Knowledge, goal: impl Fn(Cell) -> bool) -> Option<usize> {
        let n = k.w * k.h;
        // prev[cell] = (parent cell, dir taken from parent). Start has no parent.
        let mut prev: Vec<Option<(Cell, usize)>> = vec![None; n];
        let mut seen = vec![false; n];
        // Cost = (path length, cumulative visit count) for the Tremaux tie-break — a
        // plain queue is BFS by length; we settle length-ties by preferring the lower
        // accumulated visits, tracked alongside.
        let mut best_visits = vec![u32::MAX; n];

        let start = k.pos;
        let si = k.idx(start);
        seen[si] = true;
        best_visits[si] = 0;
        let mut q = VecDeque::new();
        q.push_back(start);

        let mut found: Option<Cell> = None;
        while let Some(c) = q.pop_front() {
            if c != start && goal(c) {
                found = Some(c);
                break;
            }
            let base = best_visits[k.idx(c)];
            for (d, nb) in k.open_neighbors(c) {
                let j = k.idx(nb);
                // Can't route *through* a pruned cell, but the goal itself (a frontier
                // or the exit) is never pruned, so it stays reachable.
                if k.pruned[j] && !goal(nb) {
                    continue;
                }
                let cost = base.saturating_add(k.visits(nb) as u32);
                if !seen[j] {
                    seen[j] = true;
                    best_visits[j] = cost;
                    prev[j] = Some((c, d));
                    q.push_back(nb);
                } else if cost < best_visits[j] && prev[j].is_some() {
                    // Same BFS layer, cheaper Tremaux cost — re-parent. (Doesn't change
                    // BFS distance ordering; only which equal-length path we keep.)
                    best_visits[j] = cost;
                    prev[j] = Some((c, d));
                }
            }
        }

        let mut cur = found?;
        let mut step_dir = None;
        while let Some((parent, dir)) = prev[k.idx(cur)] {
            step_dir = Some(dir);
            if parent == start {
                break;
            }
            cur = parent;
        }
        step_dir
    }
}
