//! Manual listening test for `chiptune`'s instrument palette — there's no way to assert
//! "sounds like a kick drum" in a unit test, so this exists to actually listen to.
//!
//! Run with `cargo run -p audio --example chiptune_demo --release` (native only) and
//! listen: a kick/snare/hi-hat pattern under a short pulse-lead + triangle-bass riff,
//! four bars, then it exits on its own.

use audio::chiptune;
use audio::pitch::note_freq;
use audio::playback::Clip;
use audio::wav::SAMPLE_RATE;
use macroquad::prelude::*;

#[macroquad::main("chiptune demo")]
async fn main() {
    let kick = Clip::from_samples(&chiptune::kick(SAMPLE_RATE)).await;
    let snare = Clip::from_samples(&chiptune::snare(SAMPLE_RATE)).await;
    let hihat_closed = Clip::from_samples(&chiptune::hihat(false, SAMPLE_RATE)).await;
    let hihat_open = Clip::from_samples(&chiptune::hihat(true, SAMPLE_RATE)).await;

    let mut lead = Vec::new();
    for &st in &[0.0, 4.0, 7.0, 12.0, 7.0, 4.0, 0.0, -5.0] {
        lead.push(
            Clip::from_samples(&chiptune::pulse_lead(note_freq(st), 0.18, 0.4, SAMPLE_RATE)).await,
        );
    }
    let mut bass = Vec::new();
    for &st in &[-12.0, -12.0, -5.0, -5.0] {
        bass.push(
            Clip::from_samples(&chiptune::triangle_bass(note_freq(st), 0.36, SAMPLE_RATE)).await,
        );
    }

    // 8-step pattern: kick on 1/5, snare on 3/7, closed hat on every step but a final
    // open-hat flourish on step 8.
    let kicks = [true, false, false, false, true, false, false, false];
    let snares = [false, false, true, false, false, false, true, false];
    let open_on_last = 7;

    let step_secs = 0.18;
    let mut accum = 0.0f32;
    let mut step = 0usize;
    let mut bar = 0;

    loop {
        clear_background(BLACK);
        draw_text(
            "chiptune demo -- listen, don't look",
            20.0,
            40.0,
            24.0,
            WHITE,
        );
        draw_text(format!("bar {bar} / 4").as_str(), 20.0, 70.0, 20.0, GRAY);

        accum += get_frame_time();
        if accum >= step_secs {
            accum -= step_secs;
            let i = step % 8;
            if kicks[i] {
                kick.play_once(0.9);
            }
            if snares[i] {
                snare.play_once(0.8);
            }
            if i == open_on_last {
                hihat_open.play_once(0.5);
            } else {
                hihat_closed.play_once(0.35);
            }
            lead[i].play_once(0.5);
            if i.is_multiple_of(2) {
                bass[(i / 2) % bass.len()].play_once(0.7);
            }
            step += 1;
            if step.is_multiple_of(8) {
                bar += 1;
            }
        }

        if bar >= 4 {
            break;
        }

        next_frame().await;
    }
}
