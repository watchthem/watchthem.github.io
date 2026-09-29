//! Tower: an office floor plan, not a maze.
//!
//! The hall is a **corridor lattice**: a set of parallel main corridors (the *dense*
//! lines, 2-4 cells apart — room for one or two rooms back to back between them) crossed
//! by cross corridors (the *sparse* lines, 3-6 cells apart), both spanning the lattice's
//! bounding box so every corridor end is a corner or a T. Then a random share of
//! segments (mostly cross-corridor ones) is taken out again — staggered T junctions,
//! longer blocks, a few big open-plan blocks — as long as the hall stays one piece with
//! no dead end and no new cell out of a room's reach. Three kinds of hall dead end are
//! added on purpose afterwards: the entrance (a stub from the top-left start cell to
//! the lattice), the lift lobby (the exit) and up to two kitchenette nooks.
//!
//! Everything between the corridors becomes rooms (`SIZES`-shaped, each fronting a
//! corridor with its door(s)) or, where no corridor is within a room's reach, solid
//! **core** (stairs, shafts — `Maze::is_core`). Two hall cells side by side are always
//! joined, and no 2x2 square is all hall: corridors are one cell wide and never run
//! alongside each other with only a wall between (`office_is_a_floor_plan`).

use super::{Cell, Door, Maze, MeetingLayout, Room, RoomKind, generate_empty, opposite};
use macroquad::rand::gen_range;

/// Chance a cross-corridor segment is left out (the lattice's rungs).
const DROP_CROSS: f32 = 0.4;
/// Chance a main-corridor segment is left out (they're the long straight runs).
const DROP_MAIN: f32 = 0.12;
/// A room reaches at most this far from the corridor its door opens onto.
const REACH: i32 = 3;

struct Plan {
    w: i32,
    h: i32,
    hall: Vec<bool>,
}

impl Plan {
    fn inb(&self, c: Cell) -> bool {
        c.x >= 0 && c.y >= 0 && c.x < self.w && c.y < self.h
    }

    fn i(&self, c: Cell) -> usize {
        (c.y * self.w + c.x) as usize
    }

    fn is_hall(&self, c: Cell) -> bool {
        self.inb(c) && self.hall[self.i(c)]
    }

    fn set(&mut self, c: Cell, v: bool) {
        let i = self.i(c);
        self.hall[i] = v;
    }

    fn cells(&self) -> impl Iterator<Item = Cell> + use<> {
        let w = self.w;
        (0..self.w * self.h).map(move |i| Cell { x: i % w, y: i / w })
    }

    fn hall_degree(&self, c: Cell) -> usize {
        (0..4).filter(|&d| self.is_hall(c.step(d))).count()
    }

    /// BFS step distance over hall cells from `from` (`u32::MAX` = not reached).
    fn dist(&self, from: Cell) -> Vec<u32> {
        let mut dist = vec![u32::MAX; self.hall.len()];
        let mut q = std::collections::VecDeque::from([from]);
        dist[self.i(from)] = 0;
        while let Some(c) = q.pop_front() {
            for d in 0..4 {
                let nb = c.step(d);
                if self.is_hall(nb) && dist[self.i(nb)] == u32::MAX {
                    dist[self.i(nb)] = dist[self.i(c)] + 1;
                    q.push_back(nb);
                }
            }
        }
        dist
    }

    /// Non-hall cells with no hall cell within `REACH` in a straight line — no room
    /// could reach a corridor from them.
    fn orphans(&self) -> usize {
        self.cells()
            .filter(|&c| {
                !self.is_hall(c)
                    && !(0..4).any(|d| {
                        let mut p = c;
                        (0..REACH).any(|_| {
                            p = p.step(d);
                            self.is_hall(p)
                        })
                    })
            })
            .count()
    }

    /// The hall is one piece and nothing but `start` is a dead end.
    fn sound(&self, start: Cell) -> bool {
        let dist = self.dist(start);
        self.cells()
            .filter(|&c| self.is_hall(c))
            .all(|c| dist[self.i(c)] != u32::MAX && (c == start || self.hall_degree(c) >= 2))
    }

    /// `(hall cell, dir)` pairs where one cell could be added beyond `hall cell` as a
    /// clean dead end: in bounds, not hall, and touching no other hall cell.
    fn stubs(&self) -> Vec<(Cell, usize)> {
        let mut out = Vec::new();
        for c in self.cells().filter(|&c| self.is_hall(c)) {
            for d in 0..4 {
                let s = c.step(d);
                if self.inb(s)
                    && !self.is_hall(s)
                    && (0..4).all(|k| k == opposite(d) || !self.is_hall(s.step(k)))
                {
                    out.push((c, d));
                }
            }
        }
        out
    }
}

fn chance(p: f32) -> bool {
    gen_range(0.0f32, 1.0) < p
}

/// Positions of parallel corridor lines across `n` cells: a margin of 0, 2 or 3 cells
/// before the first, `gap()` cells between neighbours, 0, 2 or 3 after the last. Never a
/// one-cell margin: a strip one room deep could only hold a row of cupboard-sized
/// single cells, each with its own door.
fn lines(n: i32, gap: impl Fn() -> i32) -> Vec<i32> {
    let mut at = vec![match gen_range(0.0f32, 1.0) {
        r if r < 0.35 => 0,
        r if r < 0.85 => 2,
        _ => 3,
    }];
    loop {
        let last = at[at.len() - 1];
        let rest = n - 1 - last;
        if at.len() >= 2 && rest <= REACH || rest < 2 {
            if rest == 1 {
                // Out to the wall instead, one cell further.
                let k = at.len() - 1;
                at[k] += 1;
            }
            break;
        }
        at.push(last + 1 + gap().min(rest - 1));
    }
    if at.len() < 2 {
        at = vec![0, n - 1];
    }
    at
}

/// Main corridors: mostly two rooms deep between them, sometimes three (open plan) or
/// four (two rooms back to back, each two deep).
fn dense_gap() -> i32 {
    match gen_range(0.0f32, 1.0) {
        r if r < 0.55 => 2,
        r if r < 0.85 => 3,
        _ => 4,
    }
}

fn sparse_gap() -> i32 {
    gen_range(3i32, 7)
}

/// The corridor network, with the entrance stub from `start` (top-left) already in.
fn halls(w: i32, h: i32, start: Cell) -> Plan {
    let dense_rows = chance(if w >= h { 0.75 } else { 0.25 });
    let (rows, cols) = if dense_rows {
        (lines(h, dense_gap), lines(w, sparse_gap))
    } else {
        (lines(h, sparse_gap), lines(w, dense_gap))
    };
    let mut p = Plan {
        w,
        h,
        hall: vec![false; (w * h) as usize],
    };
    let (x0, x1, y0, y1) = (cols[0], cols[cols.len() - 1], rows[0], rows[rows.len() - 1]);
    for &y in &rows {
        for x in x0..=x1 {
            p.set(Cell { x, y }, true);
        }
    }
    for &x in &cols {
        for y in y0..=y1 {
            p.set(Cell { x, y }, true);
        }
    }
    // The entrance: along the top wall from the start, then down to the lattice's
    // corner.
    for x in start.x..=x0 {
        p.set(Cell { x, y: start.y }, true);
    }
    for y in start.y..=y0 {
        p.set(Cell { x: x0, y }, true);
    }

    // Segments between junctions: (interior cells, on a main corridor).
    let mut segs: Vec<(Vec<Cell>, bool)> = Vec::new();
    for &y in &rows {
        for pair in cols.windows(2) {
            let cells = (pair[0] + 1..pair[1]).map(|x| Cell { x, y }).collect();
            segs.push((cells, dense_rows));
        }
    }
    for &x in &cols {
        for pair in rows.windows(2) {
            let cells = (pair[0] + 1..pair[1]).map(|y| Cell { x, y }).collect();
            segs.push((cells, !dense_rows));
        }
    }
    for i in (1..segs.len()).rev() {
        segs.swap(i, gen_range(0usize, i + 1));
    }
    let mut orphans = p.orphans();
    for (cells, main) in &segs {
        if !chance(if *main { DROP_MAIN } else { DROP_CROSS }) {
            continue;
        }
        // The entrance stub may run along a border segment: keep it.
        if cells
            .iter()
            .any(|&c| c.y == start.y && c.x <= x0 || c.x == x0 && c.y <= y0)
        {
            continue;
        }
        for &c in cells {
            p.set(c, false);
        }
        let now = p.orphans();
        if now <= orphans && p.sound(start) {
            orphans = now;
        } else {
            for &c in cells {
                p.set(c, true);
            }
        }
    }
    p
}

pub(super) fn generate(w: usize, h: usize) -> Maze {
    let mut m = generate_empty(w, h);
    let start = m.start;
    let mut p = halls(w as i32, h as i32, start);

    // The lift lobby: a one-cell dead end off the hall in the far half of the floor.
    let dist = p.dist(start);
    let max = p
        .cells()
        .map(|c| dist[p.i(c)])
        .filter(|&d| d != u32::MAX)
        .max()
        .unwrap_or(0);
    let far: Vec<(Cell, usize)> = p
        .stubs()
        .into_iter()
        .filter(|&(c, _)| c != start && dist[p.i(c)] * 4 >= max * 3)
        .collect();
    let mut nooks: Vec<Cell> = Vec::new();
    if !far.is_empty() {
        let (c, d) = far[gen_range(0usize, far.len())];
        m.exit = c.step(d);
        m.lift = true;
        p.set(m.exit, true);
    } else {
        // Never seen on the generated sizes: the exit is a far hall cell.
        let c = p
            .cells()
            .filter(|&c| p.is_hall(c) && dist[p.i(c)] * 2 >= max)
            .last()
            .unwrap_or(start);
        m.exit = c;
    }
    // Kitchenette nooks: a couple more one-cell dead ends — nothing hides in them, the
    // bot sees in from the corridor.
    for _ in 0..gen_range(0usize, 3) {
        let spots: Vec<(Cell, usize)> = p
            .stubs()
            .into_iter()
            .filter(|&(c, _)| c != start && c != m.exit && (2..u32::MAX).contains(&dist[p.i(c)]))
            .collect();
        if spots.is_empty() {
            break;
        }
        let (c, d) = spots[gen_range(0usize, spots.len())];
        nooks.push(c.step(d));
        p.set(c.step(d), true);
    }
    for c in p.cells() {
        for d in [1, 2] {
            if p.is_hall(c) && p.is_hall(c.step(d)) {
                m.carve(c, d);
            }
        }
    }

    fit_rooms(&mut m, &p, &nooks);
    m
}

/// Fill the space between the corridors with rooms, each fronting a corridor, and give
/// each its door(s). Cells no room reaches stay solid core.
fn fit_rooms(m: &mut Maze, p: &Plan, nooks: &[Cell]) {
    let exit = m.exit;
    // Doors open onto the corridors — never onto the lift lobby or a nook.
    let door_ok = |c: Cell| p.is_hall(c) && c != exit && !nooks.contains(&c);
    let mut owned = vec![false; p.hall.len()];
    let free = |owned: &[bool], c: Cell| p.inb(c) && !p.is_hall(c) && !owned[p.i(c)];
    let fronts = |c: Cell| (0..4).filter(move |&d| door_ok(c.step(d)));
    let take = |owned: &mut [bool], rooms: &mut Vec<Room>, cells: &[Cell], kind| {
        for &c in cells {
            owned[p.i(c)] = true;
        }
        let x = cells.iter().map(|c| c.x).min().unwrap();
        let y = cells.iter().map(|c| c.y).min().unwrap();
        rooms.push(Room {
            x,
            y,
            w: cells.iter().map(|c| c.x).max().unwrap() - x + 1,
            h: cells.iter().map(|c| c.y).max().unwrap() - y + 1,
            kind,
            vacant: false,
            style: 0.0,
        });
    };
    let mut rooms = Vec::new();

    // Every floor has a boss: the first room is a single cell.
    let spots: Vec<Cell> = p
        .cells()
        .filter(|&c| free(&owned, c) && fronts(c).next().is_some())
        .collect();
    if !spots.is_empty() {
        let c = spots[gen_range(0usize, spots.len())];
        take(&mut owned, &mut rooms, &[c], RoomKind::OpenSpace);
    }
    // The washrooms: two single cells side by side, both fronting the same corridor.
    let mut wc_dir = None;
    let pairs: Vec<(Cell, Cell, usize)> = p
        .cells()
        .filter(|&c| free(&owned, c))
        .flat_map(|c| fronts(c).map(move |d| (c, d)))
        .flat_map(|(c, d)| [(d + 1) % 4, (d + 3) % 4].map(|a| (c, c.step(a), d)))
        .filter(|&(_, b, d)| free(&owned, b) && door_ok(b.step(d)))
        .collect();
    if !pairs.is_empty() {
        let (a, b, d) = pairs[gen_range(0usize, pairs.len())];
        take(&mut owned, &mut rooms, &[a], RoomKind::Wc);
        take(&mut owned, &mut rooms, &[b], RoomKind::Wc);
        wc_dir = Some(d);
    }

    // The rest, swept in reading order: each free cell on a corridor starts a room
    // reaching back from it (never deeper than `REACH`), running along the corridor.
    for c in p.cells() {
        if !free(&owned, c) {
            continue;
        }
        let dirs: Vec<usize> = fronts(c).collect();
        if dirs.is_empty() {
            continue;
        }
        let d = dirs[gen_range(0usize, dirs.len())];
        let inward = opposite(d);
        let along = if d.is_multiple_of(2) { 1 } else { 2 };
        let mut avail = 0;
        let mut q = c;
        while avail < 7 && free(&owned, q) {
            avail += 1;
            q = q.step(inward);
        }
        let far_hall = avail < 7 && p.is_hall(q);
        // Back to back with a room fronting the far corridor, or as deep as it goes.
        let depth = if avail <= REACH {
            if far_hall && avail >= 2 && chance(0.45) {
                gen_range(1, avail)
            } else {
                avail
            }
        } else if far_hall {
            gen_range(avail - REACH, REACH + 1)
        } else {
            REACH
        };
        let block = |wd: i32, dp: i32| -> Option<Vec<Cell>> {
            let mut cells = Vec::new();
            for i in 0..wd {
                for j in 0..dp {
                    let mut q = c;
                    for _ in 0..i {
                        q = q.step(along);
                    }
                    for _ in 0..j {
                        q = q.step(inward);
                    }
                    if !free(&owned, q) {
                        return None;
                    }
                    cells.push(q);
                }
            }
            Some(cells)
        };
        // `SIZES` shapes only: a two-cell room runs straight back from its door (a
        // peek-only room's door is on a short end), three deep needs two wide.
        let mut cells = None;
        for dp in (1..=depth).rev() {
            let widths: &[i32] = match dp {
                1 => &[1],
                2 => match gen_range(0.0f32, 1.0) {
                    r if r < 0.25 => &[1, 3, 2],
                    r if r < 0.65 => &[2, 3, 1],
                    _ => &[3, 2, 1],
                },
                _ => {
                    if chance(0.5) {
                        &[2, 3]
                    } else {
                        &[3, 2]
                    }
                }
            };
            cells = widths.iter().find_map(|&wd| block(wd, dp));
            if cells.is_some() {
                break;
            }
        }
        let cells = cells.unwrap_or_else(|| vec![c]);
        take(&mut owned, &mut rooms, &cells, RoomKind::OpenSpace);
    }
    m.rooms = rooms;
    super::name_rooms(&mut m.rooms);
    // More than half the meeting rooms and open-plan floors stand empty, their doors
    // unmarked: plain doors onto the same bare carpet under the same hum.
    // The zen room is a joke best made once or twice a floor: two in three of them
    // are left empty too.
    // And no conceit is furnished more than twice a floor: a third room of it is left empty.
    let mut furnished: Vec<MeetingLayout> = Vec::new();
    for r in &mut m.rooms {
        r.style = gen_range(0.0f32, 1.0);
        let layout = r.meeting_layout();
        let zen = layout == Some(MeetingLayout::Zen);
        r.vacant = matches!(r.kind, RoomKind::OpenSpace | RoomKind::Conference)
            && (chance(0.55) || zen && chance(2.0 / 3.0));
        if let (Some(l), false) = (layout, r.vacant) {
            r.vacant = furnished.iter().filter(|&&f| f == l).count() >= 2;
            furnished.push(l);
        }
    }

    // Half the floors, one washroom is out of order: its door is sealed shut. On most
    // of the rest one is occupied: its door looks like any other, but won't open.
    let broken_wc = chance(0.5).then(|| gen_range(0usize, 2));
    let occupied_wc = if broken_wc.is_none() && chance(0.7) {
        Some(gen_range(0usize, 2))
    } else {
        None
    };
    let mut wc_seen = 0;
    for ri in 0..m.rooms.len() {
        let r = m.rooms[ri];
        if r.kind == RoomKind::Wc {
            wc_seen += 1;
        }
        let wc = |pick: Option<usize>| r.kind == RoomKind::Wc && pick == Some(wc_seen - 1);
        let (broken, occupied) = (wc(broken_wc), wc(occupied_wc));
        for y in r.y..r.y + r.h {
            for x in r.x..r.x + r.w {
                let c = Cell { x, y };
                if x + 1 < r.x + r.w {
                    m.carve(c, 1);
                }
                if y + 1 < r.y + r.h {
                    m.carve(c, 2);
                }
            }
        }
        // Each wall's cells in order along it, and the ones a door could go in.
        let wall = |d: usize| -> Vec<Cell> {
            match d {
                0 => (r.x..r.x + r.w).map(|x| Cell { x, y: r.y }).collect(),
                2 => (r.x..r.x + r.w)
                    .map(|x| Cell {
                        x,
                        y: r.y + r.h - 1,
                    })
                    .collect(),
                3 => (r.y..r.y + r.h).map(|y| Cell { x: r.x, y }).collect(),
                _ => (r.y..r.y + r.h)
                    .map(|y| Cell {
                        x: r.x + r.w - 1,
                        y,
                    })
                    .collect(),
            }
        };
        let dirs: Vec<usize> = (0..4)
            .filter(|&d| {
                let cells = wall(d);
                // A peek-only room's door goes on a short end, so the view through it
                // runs the room's length; the washrooms' both face the same corridor.
                (!r.peek_only() || cells.len() == 1)
                    && (r.kind != RoomKind::Wc || wc_dir == Some(d))
                    && cells.iter().any(|&c| door_ok(c.step(d)))
            })
            .collect();
        if dirs.is_empty() {
            continue;
        }
        let d = dirs[gen_range(0usize, dirs.len())];
        let cells = wall(d);
        let ok: Vec<usize> = (0..cells.len())
            .filter(|&i| door_ok(cells[i].step(d)))
            .collect();
        let a = ok[gen_range(0usize, ok.len())];
        let mut at = vec![cells[a]];
        // A second door (open-plan rooms only) is rare, and onto the same stretch of
        // corridor: the hall between the two is unbroken, so it's never a shortcut.
        if chance(0.08) && !r.peek_only() {
            let b: Vec<usize> = ok
                .iter()
                .copied()
                .filter(|&b| {
                    let (lo, hi) = (a.min(b), a.max(b));
                    hi - lo >= 2 && (lo..=hi).all(|i| p.is_hall(cells[i].step(d)))
                })
                .collect();
            if !b.is_empty() {
                at.push(cells[b[gen_range(0usize, b.len())]]);
            }
        }
        for c in at {
            // Every door opens, bar the out-of-order washroom's (a locked office door
            // was a dead end that looked like a way in).
            let sealed = broken;
            if !sealed {
                m.carve(c, d);
            }
            m.doors.push(Door {
                cell: c,
                dir: d,
                sealed,
                open: false,
                done: false,
                occupied,
            });
        }
    }
}

#[cfg(test)]
pub(super) fn ascii(m: &Maze) -> String {
    let mut s = String::new();
    let room = |c: Cell| m.rooms.iter().any(|r| r.contains(c));
    for gy in 0..=2 * m.h as i32 {
        for gx in 0..=2 * m.w as i32 {
            let (cx, cy) = ((gx - 1).div_euclid(2), (gy - 1).div_euclid(2));
            let ch = match (gx % 2, gy % 2) {
                (1, 1) => {
                    let c = Cell { x: cx, y: cy };
                    if c == m.start {
                        '@'
                    } else if c == m.exit {
                        'L'
                    } else if m.is_core(c) {
                        '%'
                    } else if room(c) {
                        '.'
                    } else {
                        ' '
                    }
                }
                (0, 1) | (1, 0) => {
                    // An edge square: the cell before it and the direction across.
                    let (c, d) = if gx % 2 == 0 {
                        (
                            Cell {
                                x: gx / 2 - 1,
                                y: cy,
                            },
                            1,
                        )
                    } else {
                        (
                            Cell {
                                x: cx,
                                y: gy / 2 - 1,
                            },
                            2,
                        )
                    };
                    let c2 = c.step(d);
                    if !m.cell_in_bounds(c) || !m.cell_in_bounds(c2) {
                        '#'
                    } else if let Some(i) = m.door_at(c, d) {
                        if m.doors[i].sealed { 'x' } else { 'D' }
                    } else if m.is_open(c, d) {
                        if room(c) { '.' } else { ' ' }
                    } else if m.is_core(c) && m.is_core(c2) {
                        '%'
                    } else {
                        '#'
                    }
                }
                _ => '#',
            };
            s.push(ch);
        }
        s.push('\n');
    }
    s
}
