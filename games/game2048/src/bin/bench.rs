// Headless benchmark for the 2048 AI.
// Usage: cargo run --bin bench --release [N_GAMES]
// Default: 100 games. Pass a number to override.

use game2048::{N, can_move, choose_dir, slide};
use std::time::Instant;

// --- Minimal xorshift64 RNG (no external deps) ---

struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        let mut r = Self(seed | 1);
        r.next();
        r
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn range(&mut self, n: usize) -> usize {
        (self.next() as usize) % n
    }
    fn tile(&mut self) -> u32 {
        if self.next().is_multiple_of(10) { 4 } else { 2 }
    }
}

// --- Headless game ---

fn spawn(board: &mut [[u32; N]; N], rng: &mut Rng) {
    let empties: Vec<(usize, usize)> = (0..N)
        .flat_map(|r| (0..N).map(move |c| (r, c)))
        .filter(|&(r, c)| board[r][c] == 0)
        .collect();
    if empties.is_empty() {
        return;
    }
    let (r, c) = empties[rng.range(empties.len())];
    board[r][c] = rng.tile();
}

struct GameResult {
    score: u32,
    max_tile: u32,
    moves: u32,
}

fn run_game(seed: u64) -> GameResult {
    let mut rng = Rng::new(seed);
    let mut board = [[0u32; N]; N];
    let mut score = 0u32;
    let mut moves = 0u32;

    spawn(&mut board, &mut rng);
    spawn(&mut board, &mut rng);

    loop {
        if !can_move(&board) {
            break;
        }
        match choose_dir(&board) {
            None => break,
            Some(dir) => {
                let Some((new_board, pts)) = slide(board, dir) else {
                    break;
                };
                board = new_board;
                score += pts;
                moves += 1;
                spawn(&mut board, &mut rng);
            }
        }
    }

    let max_tile = board
        .iter()
        .flat_map(|r| r.iter())
        .copied()
        .max()
        .unwrap_or(0);
    GameResult {
        score,
        max_tile,
        moves,
    }
}

// --- Statistics ---

fn percentile(sorted: &[u32], p: f64) -> u32 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() as f64 - 1.0) * p / 100.0).round() as usize]
}

fn fmt(n: u32) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out.chars().rev().collect()
}

fn main() {
    let mut args = std::env::args().skip(1);
    let n: usize = args.next().and_then(|s| s.parse().ok()).unwrap_or(100);
    let seed_arg: Option<u64> = args.next().and_then(|s| s.parse().ok());

    println!(
        "2048 AI Benchmark  {} games  (use --release for accurate timings)",
        n
    );

    // Native-only bench binary: wall-clock is fine here.
    #[allow(clippy::disallowed_methods)]
    let seed_base = seed_arg.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(42, |d| d.as_nanos() as u64)
    });

    #[allow(clippy::disallowed_methods)]
    let t0 = Instant::now();
    let mut results: Vec<GameResult> = Vec::with_capacity(n);
    for i in 0..n {
        results.push(run_game(
            seed_base.wrapping_add(i as u64 * 6364136223846793005),
        ));
        if (i + 1) % 10 == 0 {
            eprint!("\r  {}/{}", i + 1, n);
        }
    }
    eprintln!();

    let elapsed = t0.elapsed().as_secs_f64();
    let total_moves: u64 = results.iter().map(|r| r.moves as u64).sum();

    let mut scores: Vec<u32> = results.iter().map(|r| r.score).collect();
    scores.sort_unstable();
    let mean_score = scores.iter().map(|&s| s as f64).sum::<f64>() / n as f64;
    let mean_moves = total_moves as f64 / n as f64;

    // Max tile distribution (exact, sums to 100%)
    let mut tile_dist: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    for r in &results {
        *tile_dist.entry(r.max_tile).or_insert(0) += 1;
    }
    let mut tile_rows: Vec<(u32, usize)> = tile_dist.into_iter().collect();
    tile_rows.sort_by_key(|&(tile, _)| std::cmp::Reverse(tile)); // highest first

    let bar_w = 20usize;
    let sep = "━".repeat(50);

    println!("{sep}");
    println!(
        "Score   min {:>8}   max {:>8}   mean {:>8}",
        fmt(scores[0]),
        fmt(*scores.last().unwrap()),
        fmt(mean_score as u32)
    );
    println!(
        "        p25 {:>8}   p50 {:>8}   p75  {:>8}",
        fmt(percentile(&scores, 25.0)),
        fmt(percentile(&scores, 50.0)),
        fmt(percentile(&scores, 75.0))
    );
    println!(
        "Moves   mean {:>7}   total {:>10}",
        fmt(mean_moves as u32),
        fmt(total_moves as u32)
    );
    println!(
        "Speed   {:.0} moves/s  ({:.1}s)",
        total_moves as f64 / elapsed,
        elapsed
    );
    println!();
    println!("Max tile distribution:");
    for (tile, count) in &tile_rows {
        let pct = *count as f64 / n as f64;
        let filled = (pct * bar_w as f64).round() as usize;
        let bar = format!("{}{}", "█".repeat(filled), "░".repeat(bar_w - filled));
        println!("  {:>5}  {}  {:>3}  {:.0}%", tile, bar, count, pct * 100.0);
    }
    println!("{sep}");
}
