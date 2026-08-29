//! Game state: the maze, the bot's `Knowledge`, and the step loop that ties the
//! visibility feed to the solver.

use crate::know::Knowledge;
use crate::maze::{self, Cell, Maze};
use crate::solver::Solver;

/// Cardinal line-of-sight range down an open corridor, in cells. Generous (the "360°
/// corridor LOS" option in the plan) — see `.notes/labyrinth_todo.md` for the
/// forward-FOV alternative still to be measured.
pub const LOS_RADIUS: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Reached the exit.
    Won,
    /// Solver had no reachable frontier or exit (near-impossible with full LOS).
    Stuck,
    /// Hit the step cap still exploring.
    Survived,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// The only mode until phase 6: explore a single flat maze, reach the exit.
    Explore,
}

impl Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mode::Explore => "Explore",
        }
    }
}

/// `(width, height, braid rate)` for a given generation. A gentle ramp so early rounds
/// read quickly and later ones give the explorer more rope. Phase 6's tower-climb mode
/// will drive this far harder.
fn dims_for(generation: u32) -> (usize, usize, f32) {
    let g = generation.min(12) as usize;
    let w = 15 + g * 2;
    let h = 11 + g;
    let braid = 0.15 + (generation.min(10) as f32) * 0.01;
    (w, h, braid)
}

pub struct Game {
    pub maze: Maze,
    pub know: Knowledge,
    pub solver: Solver,
    pub mode: Mode,
    pub generation: u32,
    pub steps: u32,
    pub step_cap: u32,
    /// BFS shortest path on the *true* maze — the optimal-ratio metric only. The solver
    /// never sees this.
    pub optimal: u32,
    pub outcome: Option<Outcome>,
    /// Set for one tick after a move, for the view's walk animation: `(from, dir)`.
    pub last_step: Option<(Cell, usize)>,
}

impl Game {
    pub fn new(generation: u32) -> Self {
        Self::with_mode(Mode::Explore, generation)
    }

    pub fn with_mode(mode: Mode, generation: u32) -> Self {
        let (w, h, braid) = dims_for(generation);
        let maze = maze::generate(w, h, braid);
        let (optimal, connected) = maze.bfs_from_start();
        debug_assert!(connected, "generated maze not fully connected");
        let optimal = optimal.expect("exit reachable");

        let mut know = Knowledge::new(w, h, maze.start);
        know.reveal(&maze, LOS_RADIUS);

        // Cap: a bot that explores every cell once and backtracks each corridor twice
        // is comfortably under this; a genuinely lost bot trips it.
        let step_cap = (w * h * 8) as u32;

        Game {
            maze,
            know,
            solver: Solver::new(),
            mode,
            generation,
            steps: 0,
            step_cap,
            optimal,
            outcome: None,
            last_step: None,
        }
    }

    pub fn done(&self) -> bool {
        self.outcome.is_some()
    }

    /// Advance one step. Returns `false` once the episode has ended.
    pub fn tick(&mut self) -> bool {
        if self.outcome.is_some() {
            return false;
        }
        self.last_step = None;

        let Some(dir) = self.solver.choose_move(&self.know) else {
            self.outcome = Some(Outcome::Stuck);
            return false;
        };

        let from = self.know.pos;
        let to = from.step(dir);
        self.know.record_step(to, dir);
        self.know.reveal(&self.maze, LOS_RADIUS);
        self.steps += 1;
        self.last_step = Some((from, dir));

        if to == self.maze.exit {
            self.outcome = Some(Outcome::Won);
            return false;
        }
        if self.steps >= self.step_cap {
            self.outcome = Some(Outcome::Survived);
            return false;
        }
        true
    }

    /// `steps / optimal` — the wasted-walk ratio. `>1` is the point of the game;
    /// `~1.0` means line of sight is too generous.
    pub fn optimal_ratio(&self) -> f32 {
        self.steps as f32 / self.optimal.max(1) as f32
    }

    pub fn result_line(&self) -> String {
        let outcome = match self.outcome {
            Some(Outcome::Won) => "won",
            Some(Outcome::Stuck) => "stuck",
            Some(Outcome::Survived) => "survived",
            None => "running",
        };
        format!(
            "result={outcome} steps={} optimal={} ratio={:.2} coverage={:.2} mode={} gen={}",
            self.steps,
            self.optimal,
            self.optimal_ratio(),
            self.know.coverage(),
            self.mode.label(),
            self.generation,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play_out(seed: u64, generation: u32) -> Game {
        let _g = crate::rng_guard();
        macroquad::rand::srand(seed);
        let mut g = Game::new(generation);
        while g.tick() {}
        g
    }

    #[test]
    fn explorer_always_wins_small_mazes() {
        for seed in 0..40u64 {
            let g = play_out(seed, 1);
            assert_eq!(
                g.outcome,
                Some(Outcome::Won),
                "seed {seed}: {}",
                g.result_line()
            );
        }
    }

    #[test]
    fn explorer_wanders_more_than_optimal() {
        // The wasted walk is the content: across seeds the bot should average clearly
        // above 1.0x optimal. (Individual easy seeds can still come in near 1.0.)
        let mut total = 0.0f32;
        let n = 40;
        for seed in 0..n as u64 {
            total += play_out(seed, 3).optimal_ratio();
        }
        let avg = total / n as f32;
        assert!(
            avg > 1.15,
            "average optimal ratio {avg:.2} — LOS too generous?"
        );
    }

    #[test]
    fn deterministic_replay() {
        let a = play_out(99, 2);
        let b = play_out(99, 2);
        assert_eq!(a.steps, b.steps);
        assert_eq!(a.outcome, b.outcome);
    }
}
