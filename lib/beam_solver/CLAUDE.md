# lib/beam_solver

Generic beam search for any single-player, fully-observable, deterministic game state
where branching by `clone()` + `apply()` is cheap (true for every solitaire-style game
here — a board is a handful of small `Vec`s). Used by `games/klondike` and
`games/spider`. See CLAUDE.md (root)'s "Self-playing solver games" section for when
this fits vs. a technique-escalation solver (Sudoku) or a plain greedy 1-ply eval
(match-3) — this crate isn't the right tool for either of those shapes.

The engine itself (`src/lib.rs`, ~220 lines) is thoroughly doc-commented in place —
read it directly rather than looking for a design doc here, especially `BeamSearch`'s
`node_budget` field docs and `choose_move`'s inline comments (revisit hard-exclusion,
per-node budget checking, carry-terminal-lines-forward). This file only covers what
those comments don't: the integration recipe and why the crate exists at all.

## Extracted from Spider, given to Klondike

Klondike's original solver was 1-ply greedy (score every legal move once, pick the max,
no lookahead) and reliably walked into avoidable dead ends. Spider had already solved
this with a hand-built beam search. Rather than duplicate that search-loop plumbing, the
domain-agnostic part was pulled into this crate; both games' solvers now delegate to it
and keep only their own tuned `is_pointless`/scoring closures. Each game's own
`CLAUDE.md` (`games/klondike/CLAUDE.md`, `games/spider/CLAUDE.md`) has its actual
width/depth/node_budget values and why they're set that way — don't assume one game's
tuning transfers to another; Klondike's Yukon variant and Spider's empty-tableau-column
states have very different pathological-branching profiles.

## Adding a new game

1. Implement `SearchState` for the game's `Game` type: `type Move: Copy`,
   `legal_moves()`, `apply()`, `state_hash()`, `is_terminal()`.
2. Construct `BeamSearch::new(width, depth, node_budget, is_revisit_exempt_fn)` — the
   4th arg marks moves exempt from revisit hard-exclusion (legitimately repeatable
   actions that consume a resource rather than "returning" to an earlier board, e.g.
   Klondike's `DrawStock`/`ResetStock`, Spider's `Deal`).
3. Drive it via `choose_move(game, is_pointless, score_root, score_step)` from the
   game's own `Solver::choose_move`. `score_root` runs once per real (ply-0) candidate,
   so a pricier lookahead there is affordable; `score_step` runs at every later ply for
   every surviving beam line — keep it cheap.
4. Start from Spider's tuning (`width=32, depth=8, node_budget=1500`) as a reference
   point, not Klondike's (`width=8, depth=5, node_budget=usize::MAX` — chosen because
   Klondike hasn't needed the pathological-branching safety net Spider does), and
   re-measure for the new game's actual branching factor rather than assuming either
   transfers — see `games/spider/CLAUDE.md`'s solver section for the latency-jitter
   caveat around pushing width/depth higher than currently shipped.

## Scoring lesson (match-3, measured)

Lines are scored by **cumulative sum**, so a path-additive goal bonus lets the search defer
the winning move to a deeper ply, where it rides a phantom (preview-RNG) refill and banks the
root move's incidental score on top. If a game has an endgame/goal bonus, apply the strong
emphasis in `score_root` only (match-3's `JELLY_ENDGAME_ROOT_BONUS`: +11pp win rate on its
hardest jelly level). See `games/match-3/CLAUDE.md` for the full ablation trail, including
the rejected tuning ideas — don't retry those without new evidence.
