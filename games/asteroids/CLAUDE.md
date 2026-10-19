# Asteroids

Self-playing Asteroids, 900×720, wrapping on both axes, vector lines only. The bot drives the
same controls a human has: `solver::choose(&Game) -> Controls` is its only input and
`Game::step(Controls)` the only way it acts, so its motion stays physically real.

| File | Contents |
|------|----------|
| `src/game.rs` | `Vec2`, own `Rng`, entities, `step()`, collisions, waves, `Event`s |
| `src/solver.rs` | `choose()`: threat → shoot-or-evade → brake → attack; fire decision |
| `src/lib.rs` | CLI, `run_headless`, fixed-step loop, drawing, visual-only `Effects` |
| `src/sound.rs` | one-shots off `Event`s; thrust/saucer loops + heartbeat from state |

## Simulation

- **Fixed timestep:** `step()` = one 1/60s tick, reads no clock. The window loop runs whole
  steps from `control.scale(get_frame_time())`, capped at `MAX_STEPS_PER_FRAME`.
- **RNG:** the sim owns its `Rng`; nothing (sim, solver, effects, sound) touches macroquad's
  global RNG. Same seed ⇒ same game, headless or windowed.
- Collisions are circles on the torus (`wrap_delta` = offset to the nearest image).
- **3 lives, no extra lives.** With classic extra lives (1 per 10k) the bot earns lives
  faster than it dies and never loses. A wave cap was rejected because nearly every episode
  would end "survived".
- Difficulty comes from rock speed rising per wave (flat after wave 21), saucers arriving
  sooner, and small saucers aiming tighter. There is no other per-wave content.

## Solver (~9µs/tick, deterministic)

1. **Threat:** earliest straight-line contact time against rocks, saucer and saucer
   bullets.
2. **Shoot or evade it.** Shoot if turning + bullet flight beats contact. Otherwise
   `plan_escape` rolls out 16 headings, the current facing and coasting, each turning at
   the real turn rate before thrusting, against everything nearby. It flies the roomiest.
   It hyperspaces only when no option stays clear for `ESCAPE_SOON` ticks.
3. **Brake** above `CRUISE` speed.
4. **Attack:** the cheapest lead-angle intercept among targets that are not already
   covered by an in-flight bullet and are `safe_to_hit`.

**Fire** is separate from steering: it fires whenever a bullet launched now would hit an
uncovered, safe target.

Two lessons that shaped this:
- **Evasion must be simulated, not aimed.** "Turn to face away, then thrust" sits still for
  a 40-tick 180° turn and ends in hyperspace.
- **`safe_to_hit`:** never break a medium/large rock within `SPLIT_SAFE` unless it is
  already coming at the ship. Close fragments outrun any dodge.

Measured with no effect: an earlier `REACT`, and proactive repositioning (8x per-tick cost).

**Benchmark** (200 seeds): median 9.8 min, wave 25. Deaths: small rock 35%, medium 24%,
hyperspace 22%, large 13%, saucer 7%. Episode length is not a tuning target.

**Perf gotcha:** `f32::rem_euclid` compiles to a very slow soft-float `fmodf` and made the
solver ~1ms/tick. Use floor/round/compare in per-object code, as `wrap`/`wrap_delta`/
`angle_diff` do.

## Rendering

- Everything is drawn live each frame. `glow_line` puts a wide low-alpha line under a crisp
  one. `for_images` draws objects straddling an edge at their wrapped position too.
- Ship and rocks get a dark solid body (`fill_fan`; outlines must be star-shaped about
  their origin). An outline-only ship reads as a mouse cursor. Rock craters rotate with the
  rock.
- `Effects` (dust, hull debris, hyperspace warp) is visual-only, with its own `Rng`, and
  advances per sim step, so pause and speed apply to it.
- The HUD is the only `RenderCache`.
- Screenshot/clip capture pre-rolls 900 ticks so previews show a game in progress.

## Sound

- Each sound plays at most once per frame, however many sim steps ran.
- Heartbeat: tick-scheduled, faster as the wave's remaining rock hits drop, with a
  wall-clock floor so high speed doesn't buzz. It is a low 55/62Hz thump at a quiet
  `BEAT_VOLUME`, per the user's preference; a brighter version was too present.
- Thrust must stay above ~120Hz to be audible on laptop speakers. Its loop is held
  `THRUST_HOLD_FRAMES` because the bot's thrust flickers.

## Running

```bash
mise run run asteroids
HCG_SEED=3 target/release/asteroids --no-ui --once --debug   # deaths with cause, waves, µs/tick
```
