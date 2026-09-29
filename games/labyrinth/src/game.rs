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
    /// The maze only exists near the bot: it's generated as the bot approaches and
    /// dissolves once forgotten, so revisited ground comes back different.
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

    /// Name shown in the header — `label` stays the short form for `--variant`-style
    /// output (`result=… mode=Mirror`).
    pub fn title(self) -> &'static str {
        match self {
            Mode::TowerClimb => "The Backrooms",
            Mode::DungeonCrawl => "Dungeon",
            Mode::Fog => "Lost in the Fog",
            Mode::Mirror => "Mirror Garden",
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
/// (it *is* the difficulty arc). Fog's grid is only a bound — cells exist near the bot —
/// so it's roomier, leaving unborn ground to grow into.
fn dims_for(mode: Mode, generation: u32) -> (usize, usize, f32) {
    let g = generation.min(14) as usize;
    let (w, h) = match mode {
        Mode::TowerClimb => (15 + g * 2, 11 + g + g / 2),
        Mode::Fog => (21 + g, 15 + g / 2),
        _ => (15 + g * 2, 11 + g),
    };
    // (Tower ignores it: an office floor plan has no braid — `maze::office`.)
    let braid = 0.15 + (generation.min(10) as f32) * 0.01;
    (w, h, braid)
}

/// Tower: the boss. Opening the boss room's door starts an argument: the bot stands
/// its ground for `ARGUE_TICKS` ticks while the boss's (absurd) HP drains to zero, and
/// the defeated boss concedes something. The lift only runs once that's happened.
pub struct Boss {
    /// Index into `maze.doors` of the boss room's door.
    pub door: usize,
    pub hp: u32,
    pub state: BossState,
    /// Picks the argument's lines (the view walks through them from here).
    pub banter: usize,
    /// Index into `banter::CONCESSIONS`, set on defeat.
    pub concession: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BossState {
    Waiting,
    Arguing(u32),
    Defeated,
}

/// It's over 9000.
pub const BOSS_HP: u32 = 9001;
pub const ARGUE_TICKS: u32 = 48;

/// The boss's HP `t` ticks into the argument: front-loaded, a grind at the end.
pub fn boss_hp_at(t: u32) -> u32 {
    let left = (1.0 - t as f32 / ARGUE_TICKS as f32).max(0.0);
    (BOSS_HP as f32 * left.powf(1.4)) as u32
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
    /// Mirror garden: standing in the gate cell — the next tick steps out through the
    /// gate (this way) and only then is it a win.
    leaving: Option<usize>,
    /// Set for one tick when the step pushed an office door open (index into
    /// `maze.doors`) — the view swings it.
    pub opened_door: Option<usize>,
    /// Set for one tick when that door was opened from the doorway without stepping
    /// through: the direction the bot pushed.
    pub opened_from_doorway: Option<usize>,
    /// Set for one tick when the bot pulled an office door shut behind it (index into
    /// `maze.doors`, and the way it faced to do it).
    pub closed_door: Option<(usize, usize)>,
    /// Set for one tick when the bot tried an occupied washroom's door (index into
    /// `maze.doors`, the way it faced) and was told so from inside.
    pub knocked: Option<(usize, usize)>,
    /// Tower: the floor's boss, if it has a boss room.
    pub boss: Option<Boss>,
    /// Monotonic tick counter — drives Fog forgetting / dissolving.
    pub tick_no: u32,
    /// How many mirrors the bot has found out — seen itself in, or walked into (Mirror
    /// mode stat, shown as "mirrors").
    pub bumps: u32,
    /// Fog: part of the maze was generated or dissolved this tick — the view's `WallGrid` must be
    /// rebuilt. Cleared by whoever rebuilds it.
    pub walls_changed: bool,
}

/// Fog: a cell unseen for this many ticks (and outside `FOG_SIGHT`) is forgotten —
/// dropped from `Knowledge` (gone from the minimap) and dissolved back into unborn
/// ground in the same step, so memory is never wrong: what the bot still remembers is
/// true, and what it forgot no longer exists until it's generated afresh.
pub const FOG_FORGET: u32 = 30;
/// Fog: how often the forget-and-dissolve pass runs.
const FOG_REGEN_EVERY: u32 = 4;
/// Fog: cells within this many cells (Chebyshev) of the bot exist. Covers both the
/// corridor line of sight (plus the neighbour it glimpses past its end) and the
/// forest's tree range (`raycast::TREE_RANGE`, 5 cells), so ground is never generated
/// or dissolved where it could visibly pop.
const FOG_SIGHT: i32 = LOS_RADIUS + 1;
/// Fog: how far (cells, Euclidean) the bot sees between the trees, walls or not — the
/// spruces are sparse enough that the neighbouring corridors show through.
const FOG_GLIMPSE: f32 = 2.3;
/// Fog: chance per dissolve pass that an exit standing in dissolved ground moves.
const FOG_EXIT_MOVE: f32 = 0.5;

impl Game {
    pub fn with_mode(mode: Mode, generation: u32) -> Self {
        let (w, h, braid) = dims_for(mode, generation);
        let locks = match mode {
            Mode::DungeonCrawl => 2 + generation as usize / 2,
            _ => 0,
        };
        // Tower: an office floor plan (`maze::office`) — any count > 0; the rooms fill
        // whatever the corridors leave.
        let rooms = match mode {
            Mode::TowerClimb => 1,
            _ => 0,
        };
        let mut maze = match mode {
            Mode::Fog => maze::unborn(w, h),
            _ => maze::generate(w, h, braid, locks, rooms),
        };
        if mode == Mode::Mirror {
            maze.place_gate();
            maze.add_mirrors(w * h + 4 * generation as usize);
        }
        let optimal = if mode == Mode::Fog {
            // No passages to walk yet — the Manhattan lower bound stands in.
            ((maze.exit.x - maze.start.x).abs() + (maze.exit.y - maze.start.y).abs()) as u32
        } else {
            let (optimal, connected) = maze.bfs_from_start();
            debug_assert!(connected, "generated maze not fully connected");
            optimal.expect("exit reachable")
        };

        if rooms > 0 {
            maze.lady_boss = gen_range(0u32, 5) == 0;
        }
        let mut know = Knowledge::new(w, h, maze.start);
        let boss = maze
            .rooms
            .iter()
            .find(|r| r.kind == maze::RoomKind::Boss)
            .and_then(|r| maze.doors.iter().position(|o| r.contains(o.cell)))
            .filter(|_| maze.lift)
            .map(|door| Boss {
                door,
                hp: BOSS_HP,
                state: BossState::Waiting,
                banter: 0,
                concession: 0,
            });
        know.exit_locked = boss.is_some();

        // A bot that walks every corridor twice sits well under this; a lost bot trips
        // it. Fog gets a tighter cap — a memorising bot *must* be able to lose there.
        let step_cap = match mode {
            Mode::Fog => (w * h * 7) as u32,
            _ => (w * h * 8) as u32,
        };

        let mut g = Game {
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
            leaving: None,
            opened_door: None,
            closed_door: None,
            knocked: None,
            opened_from_doorway: None,
            boss,
            tick_no: 0,
            bumps: 0,
            walls_changed: false,
        };
        g.grow_fog();
        g.look(0);
        g
    }

    /// The per-tick visibility feed: corridor line of sight, plus, in Fog's forest, a
    /// glimpse between the trees of everything within `FOG_GLIMPSE`.
    /// Door `i`'s room holds nothing left to find: every edge of every cell known.
    pub fn room_known(&self, i: usize) -> bool {
        let cell = self.maze.doors[i].cell;
        self.maze
            .rooms
            .iter()
            .find(|r| r.contains(cell))
            .is_some_and(|r| {
                r.cells()
                    .all(|c| (0..4).all(|e| self.know.edge_open(c, e).is_some()))
            })
    }

    /// An open office door on the bot's hall cell whose room holds nothing left to
    /// find (`room_known`).
    fn door_to_close(&self) -> Option<(usize, usize)> {
        let pos = self.know.pos;
        (0..4).find_map(|d| {
            let i = self.maze.door_at(pos, d)?;
            let door = &self.maze.doors[i];
            // `door.cell` is the room's side.
            if !door.open || door.done || door.cell == pos {
                return None;
            }
            self.room_known(i).then_some((i, d))
        })
    }

    fn look(&mut self, tick: u32) {
        self.know.reveal(&self.maze, LOS_RADIUS, tick);
        if self.mode == Mode::Fog {
            self.know.glimpse(&self.maze, FOG_GLIMPSE);
        }
        self.see_reflection(self.know.facing);
    }

    /// Mirror garden: looking straight down a corridor that ends in glass, the bot sees
    /// itself in it — so it knows that "opening" is a mirror without walking up to it.
    /// (Seen from the side, as `reveal` glimpses side corridors, it still passes for a
    /// way through.) Returns whether it just found one out.
    fn see_reflection(&mut self, dir: usize) -> bool {
        let mut c = self.know.pos;
        for _ in 0..=LOS_RADIUS {
            if self.maze.is_mirror(c, dir) {
                // Only news if the map still believes it's open.
                if self.know.edge_open(c, dir) == Some(true) {
                    self.know.hit_mirror(c, dir);
                    self.bumps += 1;
                    return true;
                }
                return false;
            }
            if !self.maze.transparent(c, dir) {
                return false;
            }
            c = c.step(dir);
        }
        false
    }

    /// `Some(tick)` while the bot is arguing with the boss.
    pub fn arguing(&self) -> Option<u32> {
        match self.boss.as_ref()?.state {
            BossState::Arguing(t) => Some(t),
            _ => None,
        }
    }

    pub fn boss_defeated(&self) -> bool {
        self.boss
            .as_ref()
            .is_some_and(|b| b.state == BossState::Defeated)
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
        self.opened_door = None;
        self.opened_from_doorway = None;
        self.closed_door = None;
        self.knocked = None;
        self.tick_no += 1;

        // Mid-argument: the bot holds its ground while the boss's HP drains.
        if let Some(b) = &mut self.boss
            && let BossState::Arguing(t) = b.state
        {
            let t = t + 1;
            // Front-loaded but lumpy: big early hits, a grind at the end.
            b.hp = boss_hp_at(t).min(b.hp);
            if t >= ARGUE_TICKS {
                b.hp = 0;
                b.state = BossState::Defeated;
                b.concession = gen_range(0usize, crate::banter::CONCESSIONS.len());
                self.know.exit_locked = false;
                self.know.dirty = true;
            } else {
                b.state = BossState::Arguing(t);
            }
            return true;
        }

        // Out through the garden gate: a step off the grid (not counted — the optimal
        // route ends at the gate cell), then the win.
        if let Some(d) = self.leaving {
            self.last_step = Some((self.know.pos, d));
            self.know.facing = d;
            self.outcome = Some(Outcome::Won);
            return false;
        }

        // Back out in the hall by an open office door, the room behind it seen right
        // through (walked round, or taken in from the doorway): pull it shut before
        // moving on.
        if let Some((i, dir)) = self.door_to_close() {
            self.maze.doors[i].open = false;
            self.maze.doors[i].done = true;
            self.closed_door = Some((i, dir));
            self.know.facing = dir;
            self.look(self.tick_no);
            return true;
        }

        // Standing at a shut office door: push it open and look in, without stepping.
        if let Some(dir) = self.solver.door_to_open(&self.know)
            && let Some(i) = self.maze.door_at(self.know.pos, dir)
        {
            // Someone's in there: it stays shut, and the map marks it so — the bot
            // won't try it again.
            if self.maze.doors[i].occupied {
                self.maze.seal(i);
                self.knocked = Some((i, dir));
                self.know.facing = dir;
                self.look(self.tick_no);
                return true;
            }
            self.maze.doors[i].open = true;
            self.opened_door = Some(i);
            self.opened_from_doorway = Some(dir);
            self.know.facing = dir;
            self.look(self.tick_no);
            // It's the boss.
            if let Some(b) = &mut self.boss
                && b.door == i
                && b.state == BossState::Waiting
            {
                b.state = BossState::Arguing(0);
                b.banter = gen_range(0usize, 1000);
            }
            return true;
        }

        let Some(dir) = self.solver.choose_move(&self.know, self.mode == Mode::Fog) else {
            self.outcome = Some(Outcome::Stuck);
            return false;
        };

        let from = self.know.pos;

        // Mirror garden: turning to go that way shows the bot its own reflection — it
        // stops and faces it (`bump`, for the view) instead of setting off toward it.
        if self.mode == Mode::Mirror && dir != self.know.facing && self.see_reflection(dir) {
            self.know.facing = dir;
            self.bump = Some((from, dir));
            return true;
        }

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
            // A locked door with no key, or a plain wall — the solver shouldn't offer
            // these, but if it does, there's nowhere to go.
            self.outcome = Some(Outcome::Stuck);
            return false;
        }

        // Pushing an office door open: it stays open, and the bot can see through it.
        if let Some(i) = self.maze.door_at(from, dir)
            && !self.maze.doors[i].open
        {
            self.maze.doors[i].open = true;
            self.opened_door = Some(i);
        }

        let to = from.step(dir);
        self.know.record_step(to, dir);
        self.grow_fog();
        self.look(self.tick_no);
        self.steps += 1;
        self.last_step = Some((from, dir));

        if self.mode == Mode::Fog {
            self.tick_fog();
        }

        if to == self.maze.exit {
            if let Some(d) = self.maze.gate {
                self.leaving = Some(d);
                return true;
            }
            self.outcome = Some(Outcome::Won);
            return false;
        }
        if self.steps >= self.step_cap {
            self.outcome = Some(Outcome::Survived);
            return false;
        }
        true
    }

    /// Fog mode per-step upkeep: every few ticks, forget what the bot hasn't seen in
    /// a while and dissolve exactly that ground.
    fn tick_fog(&mut self) {
        if self.tick_no.is_multiple_of(FOG_REGEN_EVERY) {
            self.dissolve_forgotten();
        }
    }

    fn in_sight(&self, c: Cell) -> bool {
        let bot = self.know.pos;
        (c.x - bot.x).abs().max((c.y - bot.y).abs()) <= FOG_SIGHT
    }

    /// Fog: generate every unborn cell within `FOG_SIGHT` of the bot — run before each
    /// `reveal`, so the bot only ever looks at ground that exists. No-op in other modes
    /// (every cell is born).
    fn grow_fog(&mut self) {
        let bot = self.know.pos;
        let cells: Vec<Cell> = (bot.y - FOG_SIGHT..=bot.y + FOG_SIGHT)
            .flat_map(|y| (bot.x - FOG_SIGHT..=bot.x + FOG_SIGHT).map(move |x| Cell { x, y }))
            .filter(|&c| self.maze.cell_in_bounds(c) && !self.maze.is_born(c))
            .collect();
        if cells.is_empty() {
            return;
        }
        let (_, _, braid) = dims_for(self.mode, self.generation);
        self.maze.materialize(&cells, braid);
        self.walls_changed = true;
    }

    /// Forget every cell unseen for `FOG_FORGET` ticks and out of sight, and
    /// dissolve it back into unborn ground. Both sides of the change happen together —
    /// `Knowledge::forget` also drops the remembered side of every edge touching a
    /// forgotten cell, so nothing the bot believes points into ground that will be
    /// generated afresh. An exit standing in dissolved ground may move within it.
    fn dissolve_forgotten(&mut self) {
        let (w, h) = (self.maze.w, self.maze.h);
        let forgot: Vec<bool> = (0..w * h)
            .map(|i| {
                let c = Cell {
                    x: (i % w) as i32,
                    y: (i / w) as i32,
                };
                // Unborn cells count too: one glimpsed through a passage keeps that
                // edge in memory, which must go once the passage's own cell dissolves.
                !self.in_sight(c) && self.know.age(c) >= FOG_FORGET
            })
            .collect();
        if !forgot.iter().any(|&f| f) {
            return;
        }
        self.know.forget(&forgot);
        self.maze.dissolve(&forgot);
        if forgot[self.maze.idx(self.maze.exit)] && gen_range(0.0f32, 1.0) < FOG_EXIT_MOVE {
            let pool: Vec<usize> = (0..w * h).filter(|&i| forgot[i]).collect();
            let i = pool[gen_range(0usize, pool.len())];
            self.maze.exit = Cell {
                x: (i % w) as i32,
                y: (i / w) as i32,
            };
        }
        self.walls_changed = true;
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
            "result={outcome} steps={} optimal={} ratio={:.2} coverage={:.2} keys={}/{} bumps={} mode={} gen={}",
            self.steps,
            self.optimal,
            self.optimal_ratio(),
            self.know.coverage(),
            self.know.keys_held.count_ones(),
            self.maze.keys.len(),
            self.bumps,
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

    /// The bot opens the office doors it comes across and looks into the small rooms
    /// from the doorway — it never walks into a peek-only room.
    #[test]
    fn bot_opens_doors_and_peeks() {
        let mut opened = 0;
        for seed in 0..20u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let mut g = Game::with_mode(Mode::TowerClimb, 2);
            while g.tick() {
                let pos = g.know.pos;
                assert!(
                    !g.maze
                        .rooms
                        .iter()
                        .any(|r| r.peek_only() && r.contains(pos)),
                    "seed {seed}: walked into a peek-only room"
                );
                opened += g.opened_from_doorway.is_some() as u32;
                // A big room's door is only ever opened by walking in through it.
                if g.opened_from_doorway.is_some() {
                    let i = g.opened_door.expect("a door was opened");
                    assert!(g.maze.door_peeked(i), "seed {seed}: peeked a walk-in room");
                }
            }
            assert_eq!(g.outcome, Some(Outcome::Won), "seed {seed}");
        }
        assert!(opened > 20, "only {opened} doors opened from the doorway");
    }

    /// From the hall at an open door the bot sees only the room cells in line with the
    /// doorway — walls either side hide the rest — so a bigger room must be walked into.
    #[test]
    fn doorway_shows_only_what_is_in_line() {
        let mut checked = 0;
        for seed in 0..10u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let mut g = Game::with_mode(Mode::TowerClimb, 1);
            for i in 0..g.maze.doors.len() {
                let door = g.maze.doors[i];
                let Some(r) = g.maze.rooms.iter().copied().find(|r| r.contains(door.cell)) else {
                    continue;
                };
                if door.sealed || r.w < 2 || r.h < 2 {
                    continue;
                }
                let hall = door.cell.step(door.dir);
                g.maze.doors[i].open = true;
                let mut k = Knowledge::new(g.maze.w, g.maze.h, hall);
                k.reveal(&g.maze, LOS_RADIUS, 1);
                g.maze.doors[i].open = false;
                let vertical = maze::DIRS[door.dir].0 == 0;
                for c in r.cells() {
                    let in_line = if vertical {
                        c.x == hall.x
                    } else {
                        c.y == hall.y
                    };
                    let mapped = (0..4).all(|d| k.edge_open(c, d).is_some());
                    assert_eq!(mapped, in_line, "seed {seed} door {i} {c:?}");
                }
                checked += 1;
            }
        }
        assert!(checked > 10, "{checked}");
    }

    /// Vacant rooms' doors carry no name plate, and the bot leaves them for last: it
    /// opens far fewer of them than of the marked ones, and still gets out.
    #[test]
    fn unmarked_doors_are_a_last_resort() {
        let (mut marked, mut plain, mut plain_total) = (0, 0, 0);
        for seed in 0..20u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let mut g = Game::with_mode(Mode::TowerClimb, 1);
            plain_total += (0..g.maze.doors.len())
                .filter(|&i| !g.maze.door_marked(i))
                .count();
            while g.tick() {
                if let Some(i) = g.opened_door {
                    if g.maze.door_marked(i) {
                        marked += 1;
                    } else {
                        plain += 1;
                    }
                }
            }
            assert_eq!(g.outcome, Some(Outcome::Won), "seed {seed}");
        }
        assert!(plain_total > 20, "{plain_total}");
        assert!(
            plain * 4 < marked,
            "plain {plain} marked {marked} (of {plain_total} plain)"
        );
    }

    /// An occupied washroom: the bot tries its door once, it stays shut, and from then
    /// on the map knows it as sealed — never tried again, never walked through.
    #[test]
    fn occupied_washroom_is_tried_once() {
        let mut knocks = 0;
        for seed in 0..60u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let mut g = Game::with_mode(Mode::TowerClimb, 1);
            let Some(i) = g.maze.doors.iter().position(|o| o.occupied) else {
                continue;
            };
            assert!(g.maze.wc_door(i).is_some() && !g.maze.doors[i].sealed);
            let door = g.maze.doors[i];
            let hall = door.cell.step(door.dir);
            let mut tried = 0;
            while g.tick() {
                assert_ne!(
                    g.opened_door,
                    Some(i),
                    "seed {seed}: opened an occupied door"
                );
                assert_ne!(g.know.pos, door.cell, "seed {seed}: walked into it");
                if let Some((k, d)) = g.knocked {
                    assert_eq!((k, g.know.pos), (i, hall));
                    assert!(g.maze.doors[i].sealed);
                    assert_ne!(g.know.shut_door(hall), Some(d));
                    assert_eq!(g.know.edge_open(hall, d), Some(false));
                    tried += 1;
                }
            }
            assert!(tried <= 1, "seed {seed}: tried {tried} times");
            assert_eq!(g.outcome, Some(Outcome::Won), "seed {seed}");
            knocks += tried;
        }
        assert!(knocks >= 3, "{knocks}");
    }

    /// A door the bot opened is pulled shut behind it once its room is seen through —
    /// from the hall side, before the bot moves on — and stays shut: never reopened
    /// just to look.
    #[test]
    fn bot_closes_doors_behind_it() {
        let (mut closed, mut reopened) = (0, 0);
        for seed in 0..20u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let mut g = Game::with_mode(Mode::TowerClimb, 2);
            while g.tick() {
                if let Some((i, d)) = g.closed_door {
                    closed += 1;
                    let door = g.maze.doors[i];
                    assert!(!door.open && door.done);
                    assert_ne!(g.know.pos, door.cell, "seed {seed}: closed from inside");
                    assert_eq!(g.maze.door_at(g.know.pos, d), Some(i));
                    assert!(g.know.shut_door(g.know.pos) != Some(d), "seed {seed}");
                }
                if let Some(i) = g.opened_door {
                    reopened += g.maze.doors[i].done as u32;
                }
            }
            assert_eq!(g.outcome, Some(Outcome::Won), "seed {seed}");
        }
        assert!(closed > 20, "only {closed} doors closed");
        // Walking back through one on the way somewhere is allowed, just rare.
        assert!(reopened * 10 < closed, "{reopened} of {closed} reopened");
    }

    /// Office floors: the exit is a lift in a dead end, and the bot only takes it after
    /// out-arguing the boss (who takes exactly `ARGUE_TICKS` ticks, during which the
    /// bot stands still).
    #[test]
    fn boss_before_lift() {
        for seed in 0..30u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed);
            let mut g = Game::with_mode(Mode::TowerClimb, 1);
            assert!(g.maze.lift, "seed {seed}: no lift");
            assert_eq!(
                g.maze.degree(g.maze.exit),
                1,
                "seed {seed}: lift not a dead end"
            );
            assert!(g.boss.is_some(), "seed {seed}: no boss");
            let mut argued = 0;
            while g.tick() {
                if g.arguing().is_some() {
                    argued += 1;
                    assert!(g.last_step.is_none(), "seed {seed}: moved mid-argument");
                }
            }
            assert_eq!(g.outcome, Some(Outcome::Won), "seed {seed}");
            assert!(
                g.boss_defeated(),
                "seed {seed}: left without beating the boss"
            );
            assert_eq!(argued, ARGUE_TICKS, "seed {seed}");
        }
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
        // Mirrors fool the bot (a side glimpse reads as a way through), but it sees
        // itself once it looks down the corridor, corrects, and finds the real route.
        // (`bumps` counts mirrors found out, by reflection or by walking into one.)
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

    /// Fog's contract: whatever the bot still remembers is true. Forgetting and
    /// dissolving happen together, so a remembered edge never disagrees with the maze.
    #[test]
    fn fog_memory_is_never_wrong() {
        let mut regens = 0;
        for seed in 0..12u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed * 13 + 1);
            let mut g = Game::with_mode(Mode::Fog, 3);
            while g.tick() {
                if g.walls_changed {
                    regens += 1;
                    g.walls_changed = false;
                }
                for y in 0..g.maze.h as i32 {
                    for x in 0..g.maze.w as i32 {
                        let c = Cell { x, y };
                        for d in 0..4 {
                            if let Some(open) = g.know.edge_open(c, d) {
                                assert_eq!(
                                    open,
                                    g.maze.is_open(c, d),
                                    "seed {seed} tick {}: ({x},{y}) dir {d}",
                                    g.tick_no
                                );
                            }
                        }
                    }
                }
            }
            assert_eq!(
                g.outcome,
                Some(Outcome::Won),
                "seed {seed}: {}",
                g.result_line()
            );
        }
        assert!(regens > 0, "fog never changed anything");
    }

    /// Fog's other contract: the maze only exists near the bot, and generating it never
    /// walls the bot in — from where it stands, carved passages reach either the exit
    /// or a way into unborn ground.
    #[test]
    fn fog_grows_lazily_and_never_encloses() {
        for seed in 0..16u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed * 11 + 3);
            let mut g = Game::with_mode(Mode::Fog, 2);
            let (w, h) = (g.maze.w, g.maze.h);
            let mut max_born = 0;
            loop {
                let born: Vec<Cell> = (0..(w * h) as i32)
                    .map(|i| Cell {
                        x: i % w as i32,
                        y: i / w as i32,
                    })
                    .filter(|&c| g.maze.is_born(c))
                    .collect();
                max_born = max_born.max(born.len());
                // Right after a dissolve pass, nothing forgotten is left standing.
                if g.tick_no.is_multiple_of(FOG_REGEN_EVERY) {
                    for &c in &born {
                        assert!(
                            g.in_sight(c) || g.know.age(c) < FOG_FORGET,
                            "seed {seed}: {c:?} born, out of sight and forgotten"
                        );
                    }
                }
                let mut seen = vec![false; w * h];
                let mut q = vec![g.know.pos];
                seen[g.maze.idx(g.know.pos)] = true;
                let mut open_end = false;
                while let Some(c) = q.pop() {
                    open_end |= c == g.maze.exit || !g.maze.is_born(c);
                    for (_, nb) in g.maze.neighbors(c) {
                        let j = g.maze.idx(nb);
                        if !seen[j] {
                            seen[j] = true;
                            q.push(nb);
                        }
                    }
                }
                assert!(open_end, "seed {seed} tick {}: bot walled in", g.tick_no);
                if !g.tick() {
                    break;
                }
            }
            assert_ne!(
                g.outcome,
                Some(Outcome::Stuck),
                "seed {seed}: {}",
                g.result_line()
            );
            assert!(
                max_born < w * h,
                "seed {seed}: the whole grid got generated"
            );
        }
    }

    /// The bug this replaced: with no frontier left, "step to the stalest neighbour"
    /// bounced between two equally fresh cells until the step cap.
    #[test]
    fn fog_never_ping_pongs() {
        for seed in 0..24u64 {
            let _g = crate::rng_guard();
            macroquad::rand::srand(seed * 7 + 1);
            let mut g = Game::with_mode(Mode::Fog, 2);
            let mut trail: Vec<Cell> = Vec::new();
            while g.tick() {
                trail.push(g.know.pos);
                let n = trail.len();
                if n >= 8 {
                    let t = &trail[n - 8..];
                    let abab = (2..8).all(|i| t[i] == t[i - 2]) && t[0] != t[1];
                    assert!(!abab, "seed {seed}: ping-pong at {:?}/{:?}", t[0], t[1]);
                }
            }
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
