//! The explorer. Picks one cardinal step per tick from `Knowledge` alone — it never
//! sees `Maze`. That single signature (`choose_move(&Knowledge, wander)`) is what makes
//! "the minimap shows exactly what the bot knows" true by construction.
//!
//! Strategy, in order:
//! 1. Exit seen and reachable over believed-open, non-pruned edges → head straight for
//!    it (BFS shortest known path).
//! 2. Nearest frontier (a cell with a believed-open way in and an unexplored edge),
//!    same BFS. Tremaux visit counts break equal-length ties so the bot spreads out.
//! 3. Dungeon: nothing reachable, but a spotted key we don't hold → go collect it; the
//!    lock it opens then unblocks a frontier.
//! 4. Fog (`wander`): the decaying map left no frontier — step toward the stalest
//!    neighbour, i.e. back into the part of the map that's gone blank.
//!
//! Otherwise `None` → the episode ends `Stuck`.

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
    /// `wander` (Fog mode) enables a last-resort "head for the stalest neighbour" step
    /// when the decaying map leaves no frontier to aim at.
    pub fn choose_move(&mut self, k: &Knowledge, wander: bool) -> Option<usize> {
        // 1. Make for the exit if we've seen it and can get there.
        if let Some(exit) = k.exit_seen
            && let Some(dir) = self.first_step_toward(k, |c| c == exit)
        {
            return Some(dir);
        }
        // 2. The nearest unexplored frontier.
        if let Some(dir) = self.first_step_toward(k, |c| k.is_frontier(c)) {
            return Some(dir);
        }
        // 3. Nothing reachable — but if a lock is holding us back and we've spotted a
        //    key we don't hold, go collect it, then the frontier opens up (Dungeon).
        let need_key = !k.keys_seen.is_empty()
            && k.keys_seen
                .iter()
                .any(|&(_, kk)| k.keys_held & (1 << kk) == 0);
        if need_key {
            let keyset: Vec<crate::maze::Cell> = k
                .keys_seen
                .iter()
                .filter(|&&(_, kk)| k.keys_held & (1 << kk) == 0)
                .map(|&(c, _)| c)
                .collect();
            if let Some(dir) = self.first_step_toward(k, |c| keyset.contains(&c)) {
                return Some(dir);
            }
        }

        // 4. Fog: the map decayed out from under the bot and there's no frontier to
        //    aim at. Wander toward the stalest neighbour — that's where the map has
        //    gone blank and needs re-walking.
        if wander {
            return k
                .open_neighbors(k.pos)
                .max_by_key(|(_, nb)| k.age(*nb))
                .map(|(d, _)| d);
        }
        None
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
