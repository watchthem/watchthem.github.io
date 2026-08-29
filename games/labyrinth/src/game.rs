//! Game state: the maze, the bot's `Knowledge`, and the step loop that ties the
//! visibility feed to the solver.

use crate::know::Knowledge;
use crate::maze::{self, Cell, Maze};
use crate::solver::Solver;
use macroquad::rand::gen_range;

/// Cardinal line-of-sight range down an open corridor, in cells. Generous (the "360°
/// corridor LOS" option in the plan) — see `.notes/labyrinth_todo.md` for the
/// forward-FOV alternative still to be measured.
pub const LOS_RADIUS: i32 = 6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// Reached the exit.
    Won,
    /// Solver had no reachable frontier or exit.
    Stuck,
    /// Hit the step cap still going.
    Survived,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Stacked floors — the exit is a stairway up. Endless, difficulty ramps.
    TowerClimb,
    /// Keys and locked doors gate the route to the exit.
    DungeonCrawl,
    /// The map decays and mutates behind the bot; the exit roams.
    Fog,
    /// Some walls are mirrors — they read as open passages until the bot walks into one.
    Mirror,
}

impl Mode {
    pub const ALL: [Mode; 4] = [
        Mode::TowerClimb,
        Mode::DungeonCrawl,
        Mode::Fog,
        Mode::Mirror,
    ];

    pub fn next(self) -> Mode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            Mode::TowerClimb => "Tower",
            Mode::DungeonCrawl => "Dungeon",
            Mode::Fog => "Fog",
            Mode::Mirror => "Mirror",
        }
    }

    pub fn parse(s: &str) -> Option<Mode> {
        Some(match s.to_ascii_lowercase().as_str() {
            "tower" | "tower-climb" | "climb" => Mode::TowerClimb,
            "dungeon" | "dungeon-crawl" | "crawl" => Mode::DungeonCrawl,
            "fog" | "lost" => Mode::Fog,
            "mirror" | "mirrors" => Mode::Mirror,
            _ => return None,
        })
    }

    /// The word for reaching the exit in this mode.
    pub fn win_verb(self) -> &'static str {
        match self {
            Mode::TowerClimb => "UP",
            _ => "OUT",
        }
    }
}

/// `(width, height, braid rate)` for a generation, per mode. Tower climb ramps hardest
/// (it *is* the difficulty arc); fog stays smaller so the roaming exit and decay carry
/// the difficulty instead of raw size.
fn dims_for(mode: Mode, generation: u32) -> (usize, usize, f32) {
    let g = generation.min(14) as usize;
    let (w, h) = match mode {
        Mode::TowerClimb => (15 + g * 2, 11 + g + g / 2),
        Mode::Fog => (15 + g, 11 + g / 2),
        _ => (15 + g * 2, 11 + g),
    };
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
    /// Set for one tick when the bot walked into a mirror it believed was open — the
    /// view plays the `Bump` recoil.
    pub bump: Option<(Cell, usize)>,
    /// Monotonic tick counter — drives Fog staleness / mutation / exit roaming.
    pub tick_no: u32,
    /// How many mirrors the bot has walked into (Mirror mode stat).
    pub bumps: u32,
}

/// Fog: mutate an out-of-sight wall this often. The bot keeps its memory of the layout
/// (so the maze stays traversable) but the map goes visibly stale and is sometimes now
/// *wrong* — memorising it doesn't help.
const FOG_MUTATE_EVERY: u32 = 4;
/// Fog: roam the exit this often, when the bot still has ground to cover.
const FOG_ROAM_EVERY: u32 = 44;
/// Fog: a cell is "out of sight" (fair game to mutate / roam into) once this stale.
const FOG_STALE: u32 = 10;

impl Game {
    pub fn with_mode(mode: Mode, generation: u32) -> Self {
        let (w, h, braid) = dims_for(mode, generation);
        let mut maze = maze::generate(w, h, braid);
        match mode {
            Mode::DungeonCrawl => maze.add_dungeon(1 + generation as usize / 2),
            Mode::Mirror => maze.add_mirrors(2 + generation as usize / 2),
            _ => {}
        }
        let (optimal, connected) = maze.bfs_from_start();
        debug_assert!(connected, "generated maze not fully connected");
        let optimal = optimal.expect("exit reachable");

        let mut know = Knowledge::new(w, h, maze.start);
        know.reveal(&maze, LOS_RADIUS, 0);

        // A bot that walks every corridor twice sits well under this; a lost bot trips
        // it. Fog gets a tighter cap — a memorising bot *must* be able to lose there.
        let step_cap = match mode {
            Mode::Fog => (w * h * 7) as u32,
            _ => (w * h * 8) as u32,
        };

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
            bump: None,
            tick_no: 0,
            bumps: 0,
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
        self.bump = None;
        self.tick_no += 1;

        let Some(dir) = self.solver.choose_move(&self.know, self.mode == Mode::Fog) else {
            self.outcome = Some(Outcome::Stuck);
            return false;
        };

        let from = self.know.pos;

        // The solver picked a direction the map says is open, but the maze disagrees.
        if !self.maze.passable(from, dir, self.know.keys_held) {
            if self.maze.is_mirror(from, dir) {
                // Mirror mode: bump the glass, correct the map, stay put.
                self.know.hit_mirror(from, dir);
                self.bump = Some((from, dir));
                self.bumps += 1;
                self.steps += 1;
                self.know.facing = dir;
                if self.steps >= self.step_cap {
                    self.outcome = Some(Outcome::Survived);
                    return false;
                }
                return true;
            }
            if self.mode == Mode::Fog && self.know.edge_open(from, dir) == Some(true) {
                // Fog: a wall mutated in where the bot remembered a passage. Correct
                // the memory and try again next tick — no penalty, just a dead end.
                self.know.stale_memory_wall(from, dir);
                self.know.facing = dir;
                self.steps += 1;
                if self.steps >= self.step_cap {
                    self.outcome = Some(Outcome::Survived);
                    return false;
                }
                return true;
            }
            // A locked door with no key, or a plain wall — the solver shouldn't offer
            // these, but if it does, there's nowhere to go.
            self.outcome = Some(Outcome::Stuck);
            return false;
        }

        let to = from.step(dir);
        self.know.record_step(to, dir);
        self.know.reveal(&self.maze, LOS_RADIUS, self.tick_no);
        self.steps += 1;
        self.last_step = Some((from, dir));

        if self.mode == Mode::Fog {
            self.tick_fog();
        }

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

    /// Fog mode per-step upkeep: forget stale cells, and every so often mutate an
    /// out-of-sight patch of wall and (rarely) let the exit jump.
    fn tick_fog(&mut self) {
        if self.tick_no.is_multiple_of(FOG_MUTATE_EVERY) {
            self.mutate_out_of_sight();
        }
        // Let the bot close the deal if it's nearly there — the exit only bolts when
        // the bot still has ground to cover.
        let d =
            (self.maze.exit.x - self.know.pos.x).abs() + (self.maze.exit.y - self.know.pos.y).abs();
        if self.tick_no.is_multiple_of(FOG_ROAM_EVERY) && d > 4 {
            self.roam_exit();
        }
    }

    /// Toggle one wall in a cell the bot can't currently see and isn't standing near,
    /// keeping the maze fully connected (revert if the flip breaks it).
    fn mutate_out_of_sight(&mut self) {
        let bot = self.know.pos;
        for _ in 0..12 {
            let c = Cell {
                x: gen_range(0i32, self.maze.w as i32),
                y: gen_range(0i32, self.maze.h as i32),
            };
            let near = (c.x - bot.x).abs() + (c.y - bot.y).abs() < LOS_RADIUS + 1;
            let fresh = self.know.age(c) < FOG_STALE;
            if near || fresh {
                continue;
            }
            let d = gen_range(0usize, 4);
            let nb = c.step(d);
            if !self.maze.cell_in_bounds(nb) || self.maze.is_mirror(c, d) {
                continue;
            }
            self.maze.toggle_wall(c, d);
            let (reach, connected) = self.maze.bfs_from_start();
            if connected && reach.is_some() {
                return;
            }
            self.maze.toggle_wall(c, d); // revert
        }
    }

    /// Move the exit to a random cell the bot hasn't seen recently.
    fn roam_exit(&mut self) {
        let bot = self.know.pos;
        for _ in 0..30 {
            let c = Cell {
                x: gen_range(0i32, self.maze.w as i32),
                y: gen_range(0i32, self.maze.h as i32),
            };
            if c == bot
                || (c.x - bot.x).abs() + (c.y - bot.y).abs() < LOS_RADIUS + 2
                || self.know.age(c) < FOG_STALE
            {
                continue;
            }
            self.maze.exit = c;
            self.know.exit_seen = None;
            self.know.dirty = true;
            return;
        }
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
        play_mode(Mode::TowerClimb, seed, generation)
    }

    fn play_mode(mode: Mode, seed: u64, generation: u32) -> Game {
        let _g = crate::rng_guard();
        macroquad::rand::srand(seed);
        let mut g = Game::with_mode(mode, generation);
        while g.tick() {}
        g
    }

    fn win_rate(mode: Mode, gens: &[u32], seeds: u64) -> f32 {
        let mut won = 0;
        let mut total = 0;
        for &generation in gens {
            for seed in 0..seeds {
                total += 1;
                if play_mode(mode, seed * 7 + 1, generation).outcome == Some(Outcome::Won) {
                    won += 1;
                }
            }
        }
        won as f32 / total as f32
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
    fn dungeon_solvable() {
        // Keys/locks are placed solvable-by-construction — the bot should nearly
        // always get out.
        let wr = win_rate(Mode::DungeonCrawl, &[1, 3, 6], 20);
        assert!(wr > 0.9, "dungeon win rate {wr:.2}");
    }

    #[test]
    fn mirror_wins_after_bumping() {
        // Mirrors fool the bot into a wall, but it corrects and finds the real route.
        let mut any_bump = false;
        let mut won = 0;
        for seed in 0..24u64 {
            let g = play_mode(Mode::Mirror, seed * 3 + 1, 4);
            any_bump |= g.bumps > 0;
            won += (g.outcome == Some(Outcome::Won)) as u32;
        }
        assert!(any_bump, "no mirror was ever bumped");
        assert!(won >= 20, "mirror wins {won}/24");
    }

    #[test]
    fn fog_is_beatable_but_not_a_gimme() {
        // The mode's whole job: a memorising bot must be able to lose here.
        let wr = win_rate(Mode::Fog, &[2, 5, 9], 24);
        assert!(wr > 0.05 && wr < 0.95, "fog win rate {wr:.2}");
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
