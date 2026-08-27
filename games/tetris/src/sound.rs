//! Tetris's procedural sound effects, built on `lib/audio`'s shared synth primitives
//! and its `chiptune` instrument/percussion palette. Every clip here is synthesized
//! once at startup (see `Sfx::load`, called from `amain`) and triggered off game
//! *status* — a piece locking, a line clearing, the episode ending — exactly where a
//! human-played game would trigger it; the solver choosing a move is what causes those
//! status changes, same as a human's button presses would, so nothing here is "the
//! AI's" sound in any special sense (see the procedural-audio plan notes in root
//! CLAUDE.md / project memory).
//!
//! Deliberately low-register and percussion-first rather than tonal/melodic: an early
//! version used square-wave arpeggios climbing above A5 for line clears, which read as
//! shrill and grating once heard on every single clear (as opposed to a one-shot jingle,
//! heard once per session). Line clears in particular are a noise-burst "explosion"
//! (`explosion`, built on `audio::filter`/`audio::mix`), not a melodic flourish — closer
//! to a genuine SFX (see the `chiptune` module's `kick`) than a chiptune tune.
//!
//! Module named `sound` rather than `audio` to keep it visually distinct from the
//! `audio` crate it wraps (`audio::sfx`, `audio::chiptune`, ...) inside its own body.

use audio::chiptune;
use audio::filter::one_pole_lowpass;
use audio::mix;
use audio::pitch::note_freq;
use audio::playback::Clip;
use audio::sfx::{SfxParams, SfxWave};
use audio::synth::Envelope;
use audio::wav::SAMPLE_RATE;
use macroquad::prelude::next_frame;
use std::f32::consts::PI;

/// Renders a short monophonic melody: each entry is `(Some(semitones from A4), duration
/// secs)` for a note, or `(None, duration secs)` for a rest, played back-to-back.
/// Envelope release ends every note at exactly zero gain (see `Envelope::gain_at`), so
/// plain concatenation has no audible click between notes. `sustain_level` is kept
/// moderate (not full-scale) and the caller is expected to round off the raw
/// waveform's harsher harmonics afterward (see `melody_and_comp`'s lowpass) — an
/// un-softened full-amplitude square/triangle lead reads as aggressive/buzzy rather
/// than "chiptune-warm".
///
/// A note immediately followed by a rest gets a shorter release (`0.12` of its own
/// length, matching `comp_chord`'s own tightened fraction, instead of the usual `0.35`)
/// — `korobeiniki`'s two written rests (end of m4, end of m8) each follow a note whose
/// normal 35%-of-length release starts fading a good third of the way before the note's
/// own notated end. That's inaudible when something else is still sounding, but stacked
/// directly onto a following rest it measurably widens the perceived gap well past the
/// rest's own notated length (verified: the note was down to background level ~125ms
/// before its own end, on top of the ~360ms rest itself). Shortening just this one
/// note's release keeps it full closer to its true end, tightening that gap, without
/// touching the rest's own length, the comp, or the bass at all.
fn render_arpeggio(notes: &[(Option<f32>, f32)], wave: SfxWave) -> Vec<f32> {
    let mut out = Vec::new();
    for (i, &(pitch, secs)) in notes.iter().enumerate() {
        let Some(st) = pitch else {
            out.extend(std::iter::repeat_n(
                0.0,
                (secs * SAMPLE_RATE as f32) as usize,
            ));
            continue;
        };
        let before_rest = matches!(notes.get(i + 1), Some((None, _)));
        let attack = (secs * 0.08).min(0.02);
        let release = secs * if before_rest { 0.12 } else { 0.35 };
        let params = SfxParams {
            wave,
            envelope: Envelope {
                attack,
                decay: secs * 0.15,
                sustain_level: 0.55,
                sustain: (secs - attack - secs * 0.15 - release).max(0.0),
                release,
            },
            start_freq: note_freq(st),
            freq_slide: 0.0,
            freq_delta_slide: 0.0,
            arp_semitones: 0.0,
            arp_time_frac: 0.5,
            vibrato_depth: 0.0,
            vibrato_speed: 0.0,
        };
        out.extend(params.render(SAMPLE_RATE, 1));
    }
    out
}

/// Semitones from A4 for the notes "Korobeiniki" ("Коробе́йники", an 1861 Russian folk
/// tune — public domain, not Nintendo's specific Game Boy arrangement) needs, in the
/// octave the melody sits in.
const A4: f32 = 0.0;
const B4: f32 = 2.0;
const C5: f32 = 3.0;
const D5: f32 = 5.0;
const E5: f32 = 7.0;
const F5: f32 = 8.0;
const G5: f32 = 10.0;
const A5: f32 = 12.0;

/// Tempo for `korobeiniki`/`CHORD_GUIDE_TONES` — an eighth note, in seconds. Every
/// measure of both the melody and the chord progression below sums to exactly 8 of
/// these (`MEASURE_SECS`), which is what lets the two be mixed with no manual alignment.
/// `0.18` (a quarter note = 2 eighths = 0.36s, ~167 BPM) rather than the original
/// `0.13` (~231 BPM) — the original tempo was closer to a sprint than a jazz combo's
/// pocket; slower gives the bass/drum groove room to actually swing instead of just
/// keeping up.
const EIGHTH: f32 = 0.18;
const MEASURE_SECS: f32 = 8.0 * EIGHTH;
const NUM_MEASURES: f32 = 8.0;

/// Total length of `intro` — `lib.rs` holds gameplay off the opening screen for this
/// long (skipped entirely, along with the sound itself, during a screenshot/clip
/// capture — see `screenshot::is_capturing`).
pub const INTRO_SECS: f32 = MEASURE_SECS * NUM_MEASURES;

/// The widely-recognized opening phrase of "Korobeiniki" — the tune most people just
/// call "the Tetris theme" (Game Boy Tetris's Type A music, 1989). A traditional
/// 19th-century Russian folk melody: the notes themselves are public domain, and this
/// is a fresh transcription/synthesis, not a copy of any specific recording or
/// arrangement. 8 measures, each summing to `MEASURE_SECS` — see `CHORD_GUIDE_TONES`,
/// which harmonizes one chord per measure in that same order.
fn korobeiniki() -> Vec<(Option<f32>, f32)> {
    let n = |semi: f32, units: f32| (Some(semi), units * EIGHTH);
    let rest = |units: f32| (None, units * EIGHTH);
    vec![
        // m1
        n(E5, 2.0),
        n(B4, 1.0),
        n(C5, 1.0),
        n(D5, 2.0),
        n(C5, 1.0),
        n(B4, 1.0),
        // m2
        n(A4, 2.0),
        n(A4, 1.0),
        n(C5, 1.0),
        n(E5, 2.0),
        n(D5, 1.0),
        n(C5, 1.0),
        // m3
        n(B4, 3.0),
        n(C5, 1.0),
        n(D5, 2.0),
        n(E5, 2.0),
        // m4
        n(C5, 2.0),
        n(A4, 2.0),
        n(A4, 2.0),
        rest(2.0),
        // m5
        n(D5, 3.0),
        n(F5, 1.0),
        n(A5, 2.0),
        n(G5, 1.0),
        n(F5, 1.0),
        // m6
        n(E5, 3.0),
        n(C5, 1.0),
        n(E5, 2.0),
        n(D5, 1.0),
        n(C5, 1.0),
        // m7
        n(B4, 2.0),
        n(B4, 1.0),
        n(C5, 1.0),
        n(D5, 2.0),
        n(E5, 2.0),
        // m8
        n(C5, 2.0),
        n(A4, 2.0),
        n(A4, 2.0),
        rest(2.0),
    ]
}

/// One (3rd, 7th) guide-tone pair per measure of `korobeiniki`, in semitones from A4, a
/// jazz reharmonization of the tune: **i - i - v - i - iv - ♭III - V - I** (Am11 - Am7 -
/// Em7 - Am7 - Dm7 - C6 - E7 - **A6**), one octave below the melody. Guide tones only
/// (not full triads) — the 3rd and 7th are the two notes that actually define a chord's
/// quality/color, the standard jazz-comping shorthand; a full stacked chord here would
/// just read as muddy under a single-line melody.
///
/// **m8 ends on `A6`, a Picardy third** — the final chord raised to major (`3rd`: C ->
/// C#) rather than closing on the tonic minor, a classic device for a brighter, more
/// resolved ending. A `6` voicing (matching `C6`'s own precedent at m5, `F#` instead of
/// a major 7th) rather than `Amaj7`, specifically to sidestep the question of whether a
/// major 7th (G#) sitting a half-step under the melody's own held root note reads as
/// "sophisticated" or "unresolved" — `6` has no such tension at all. One real, accepted
/// clash remains: the measure's melody opens on C5 (the *minor* 3rd, scale degree 3 of
/// the tune) against the comp's now-major C#, a half-step apart, for roughly the first
/// quarter of the bar before the melody itself moves on to the held A4 root. This is
/// inherent to a Picardy third landing on a bar whose own melody hasn't also switched to
/// major — the brief tension is the sound of the melody "catching up" to a harmony that
/// got there first, not a bug to fix, but real and worth knowing about if it's ever
/// flagged again as a clash by the checklist below.
///
/// **The melody is A natural minor, full stop** — it uses F♮/G♮ (m5) and never F#/G#,
/// and closes the whole phrase on a held A4 preceded by C5, an A-minor tonic outline.
/// An earlier draft analyzed this as E minor instead (Em7-Am7-Bm7-Em7-... for
/// m1-m2-m3-m4-...), which is wrong on two of its four distinct chords: `Bm7` (m3/m7)
/// isn't diatonic to A minor at all — it needs an F# that directly contradicts the
/// melody's own F♮ four bars later — and `Em7` as the "i" in m1/m4 is really the v,
/// leaving the piece with no true tonic until an ad-hoc fix singled out m8 alone (see
/// the git history for that fix's reasoning, now generalized here). None of this showed
/// up in a plain per-measure clash check, because every one of these guide-tone pairs
/// *is* individually consonant against its own measure's melody — the bug was
/// functional (wrong chord for the key), not a literal clashing note, which is why a
/// second full harmonic analysis (checking the whole 8-measure key center, not just
/// pairwise semitone distances per bar) was needed to find it.
///
/// Every pair here is verified clash-free against its measure's actual melody notes
/// (pairwise semitone distance, not by ear) — see `games/tetris/CLAUDE.md`'s "Fixing
/// bad voicings" section for the method — **and** against the actual A-minor key center.
/// **m1** keeps its original `(G, D)` pair but is now read as `Am11` (the root moved to
/// A in `BASS_ROOTS` below; G+D over A is the ♭7 and 11, a quartal jazz voicing) rather
/// than `Em7` over an E root — one number moved (the bass), not the guide tones. **m3**
/// is `Em7` (the true v) and **m4** is `Am7` (the true i, matching m8's identical
/// closing melody C5-A4-A4) — m1/m3 and m3/m4 no longer share a chord, so a plain swap
/// wasn't available; both changed. **m7** is `E7` (`G#4`, the raised leading tone that
/// gives the final cadence real pull, `-1.0`/`-7.0`) rather than `Bm7` — the dominant
/// this key actually has. First draft used `B7♭9` (guide tones D#, A) for that same
/// slot — the textbook altered dominant — but the melody there is a plain diatonic run
/// (B-C-D-E, no raised leading tone implied) so D# clashed with the melody's D and E;
/// `E7`'s own guide tones don't have that problem. `Cmaj7` (guide tones E, B) was tried
/// for m6 for the same textbook reason and had the same clash (B a semitone from the
/// melody's repeated C); swapped for the plain `6`-voicing instead — less flashy, but
/// actually consonant.
const CHORD_GUIDE_TONES: [(f32, f32); 8] = [
    (-2.0, -7.0),  // m1 Am11 (over A root): 11th=G, b7=D
    (-9.0, -2.0),  // m2 Am7: 3rd=C, 7th=G
    (-2.0, -7.0),  // m3 Em7: 3rd=G, 7th=D
    (-9.0, -2.0),  // m4 Am7: 3rd=C, 7th=G
    (-4.0, -9.0),  // m5 Dm7: 3rd=F, 7th=C
    (-5.0, -12.0), // m6 C6:  3rd=E, 6th=A
    (-1.0, -7.0),  // m7 E7:  3rd=G#, 7th=D
    (-8.0, -3.0),  // m8 A6 (Picardy third): 3rd=C#, 6th=F#
];

/// One measure's worth of soft comping: the 3rd and 7th (or 3rd and 6th, for `C6`)
/// sustained together, sine (not square/triangle — the lead already claims the bright
/// register, comping sits under it, not in competition with it), with a "swell" attack
/// for a jazzy comp feel rather than a percussive stab. Envelope fractions sum to
/// exactly `1.0` so this renders to precisely `MEASURE_SECS` — matching the melody's
/// own per-measure duration is what keeps the two in sync; an earlier draft hardcoded
/// `attack: 0.12` (not `MEASURE_SECS`-proportional), rendering ~0.04s short of a
/// measure and drifting the comp further behind the melody every measure.
///
/// `attack`/`release` were originally `0.15`/`0.2` (a slower swell) but that meant the
/// outgoing chord's release and the incoming chord's attack overlapped for the first
/// ~1.6 eighths of *every* measure — e.g. m5's F4 release tail was still audible under
/// m6's E4 attack, a real (minor-2nd, ~20Hz-apart) beating dissonance at the bar line,
/// not a wrong-note bug. Tightened so the chord is fully present under the downbeat
/// instead of still swelling in after it; `0.72` absorbs what `attack`/`release` gave up
/// so the fractions still sum to `1.0`.
///
/// Normalized to `0.8`, not `0.5` — at `0.5` the comp (the only layer actually carrying
/// the harmony) ended up roughly 6dB under the melody/bass, which both peak at `1.0`
/// before the final mix's own `normalize_peak` scales everything down together (see
/// `korobeiniki_track`): the result was thumping roots and a barely-audible chord.
fn comp_chord(third: f32, seventh: f32) -> Vec<f32> {
    let voice = |semi: f32| {
        SfxParams {
            wave: SfxWave::Sine,
            envelope: Envelope {
                attack: MEASURE_SECS * 0.06,
                decay: MEASURE_SECS * 0.1,
                sustain_level: 0.45,
                sustain: MEASURE_SECS * 0.72,
                release: MEASURE_SECS * 0.12,
            },
            start_freq: note_freq(semi),
            freq_slide: 0.0,
            freq_delta_slide: 0.0,
            arp_semitones: 0.0,
            arp_time_frac: 0.5,
            vibrato_depth: 0.0,
            vibrato_speed: 0.0,
        }
        .render(SAMPLE_RATE, 1)
    };
    let mut out = mix::sum(&[&voice(third), &voice(seventh)]);
    mix::normalize_peak(&mut out, 0.8);
    out
}

fn comp_track() -> Vec<f32> {
    let mut out = Vec::new();
    for &(third, seventh) in &CHORD_GUIDE_TONES {
        out.extend(comp_chord(third, seventh));
    }
    out
}

/// Root note per measure, one octave below `CHORD_GUIDE_TONES`'s own octave so the bass
/// sits under the comp rather than doubling it: Am-Am-Em-Am-Dm-C-E-A6 (matching
/// `CHORD_GUIDE_TONES`'s A-minor progression, Picardy-third ending included — see its
/// doc comment). Not simply "3rd minus a 3rd" from the guide tones any more: m1's guide
/// tones are voiced as the 11th and ♭7 over an A root (an `Am11` reading), not a 3rd/7th
/// pair a root could be mechanically derived from, and m7's root (E) is the *5th* below
/// its guide tones' implied E-something, not a 3rd below — both come from the
/// key-center analysis, not arithmetic on the pair stored above. m8's root (A) is
/// unaffected by its own major/minor swap — only the chord's 3rd changes, not its root.
const BASS_ROOTS: [f32; 8] = [
    -24.0, // m1 A
    -24.0, // m2 A
    -17.0, // m3 E
    -24.0, // m4 A
    -19.0, // m5 D
    -21.0, // m6 C
    -17.0, // m7 E
    -24.0, // m8 A
];

/// Semitones above `BASS_ROOTS`' root for the funk figure's 3rd staccato hit — the
/// chord's own color tone, an octave-independent interval so it stays close to the
/// root rather than jumping registers: a ♭7 (root + 10) for every `m7`/dominant-7
/// chord here (`E7`'s ♭7 is the same interval size as a minor 7th's, only the 3rd
/// differs between the two qualities, and this table never touches the 3rd), but a
/// major 6th (root + 9) for the two `6`-voiced measures (`C6` at m6, `A6` at m8) — a `6`
/// chord has no 7th, and root + 10 there would land on a b7 clashing with the chord's
/// actual 6th.
const BASS_COLOR_TONE: [f32; 8] = [10.0, 10.0, 10.0, 10.0, 10.0, 9.0, 10.0, 9.0];

/// One punchy bass note. `secs` fully determines the envelope's total length (attack +
/// decay + release always sums to it, `sustain` left at `0.0`), so a note never bleeds
/// into whatever comes right after it in the track — a plain zero-length gap or the
/// next note both stay click-free.
///
/// Sawtooth, not `Triangle` (the first draft): a triangle wave is nearly a pure
/// fundamental with very weak upper harmonics — exactly what reads as thin/breathy
/// ("flute-y") rather than "bassy" at this register, especially on the held long note,
/// which is essentially a sustained pure tone. A sawtooth's full harmonic series gives
/// it body; `funky_bass_measure` low-passes the whole rendered line afterward to tame
/// the raw saw's harshness back down without losing that harmonic weight.
fn bass_hit(semi: f32, secs: f32, sustain_level: f32) -> Vec<f32> {
    let attack = (secs * 0.08).min(0.006);
    let decay = secs * 0.35;
    let release = (secs - attack - decay).max(0.0);
    SfxParams {
        wave: SfxWave::Sawtooth,
        envelope: Envelope {
            attack,
            decay,
            sustain_level,
            sustain: 0.0,
            release,
        },
        start_freq: note_freq(semi),
        freq_slide: 0.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(SAMPLE_RATE, 1)
}

/// A "3+1" funk bass figure: three short staccato hits (each hit + the rest after it
/// exactly one `EIGHTH`), then one longer note held through the rest of the measure
/// (the remaining 5 eighths) — `da-da-da-DAAA` rather than a walking line. The 1st and
/// 4th (held) notes are both the root; the 2nd and 3rd are two different color notes —
/// a perfect 5th (root + 7, diatonically correct for every chord quality used here)
/// then the chord's own 7th/6th (`BASS_COLOR_TONE`) — not three repeats of the root,
/// which read as static rather than a real bassline. Sums to exactly `MEASURE_SECS`,
/// same discipline as `comp_chord`.
fn funky_bass_measure(root: f32, color: f32) -> Vec<f32> {
    let hit_on = EIGHTH * 0.65;
    let hit_rest = EIGHTH - hit_on;
    let rest_samples = (hit_rest * SAMPLE_RATE as f32).round() as usize;
    let mut out = Vec::new();
    for &pitch in &[root, root + 7.0, root + color] {
        out.extend(bass_hit(pitch, hit_on, 0.0));
        out.extend(std::iter::repeat_n(0.0, rest_samples));
    }
    let held = MEASURE_SECS - 3.0 * EIGHTH;
    out.extend(bass_hit(root, held, 0.5));
    out
}

fn bass_track() -> Vec<f32> {
    let mut out = Vec::new();
    for (&root, &color) in BASS_ROOTS.iter().zip(BASS_COLOR_TONE.iter()) {
        out.extend(funky_bass_measure(root, color));
    }
    // Tames the raw sawtooth's harsh upper harmonics (see `bass_hit`) back down to a
    // rounder "synth bass" tone — filtered here, once, over the whole line rather than
    // per hit, so the filter's own state carries smoothly through the rests instead of
    // resetting at every note boundary.
    one_pole_lowpass(&mut out, 900.0, SAMPLE_RATE);
    // Every bass hit's own envelope peaks at 1.0 (see `bass_hit`'s attack), same as the
    // melody, while `comp_chord` — the only layer actually carrying the harmony — peaks
    // at 0.8. Left alone, the bass's own transients are what set the peak
    // `korobeiniki_track`'s final `normalize_peak` scales *everything* against, burying
    // the comp under thumping roots. Attenuating the bass here, before that shared
    // scaling, buys the comp real headroom for the least audible loss (a bassline reads
    // fine well under full scale; a chord that's inaudible doesn't read at all).
    for s in out.iter_mut() {
        *s *= 0.55;
    }
    out
}

/// Adds `clip` into `track` starting at sample `at`, growing `track` with silence
/// first if needed. Additive placement, not `mix::sum` (which only aligns buffers
/// starting at index 0) — a percussion hit lands mid-track, and needs its own tail
/// free to ring past its grid slot without being truncated or clicking.
fn place_at(track: &mut Vec<f32>, at: usize, clip: &[f32]) {
    let end = at + clip.len();
    if track.len() < end {
        track.resize(end, 0.0);
    }
    for (i, &s) in clip.iter().enumerate() {
        track[at + i] += s;
    }
}

/// Light jazz-funk kit for one measure: a steady closed-hihat pulse on every eighth (a
/// soft "ride" feel), a soft kick on beat 1, and a soft snare backbeat on beats 2 and 4
/// (eighth positions 2 and 6). `hat`/`kick`/`snare` are each rendered once by the caller
/// (`drum_track`) and placed repeatedly here rather than re-synthesized per hit —
/// `chiptune::hihat`/`snare`'s noise components use a fixed internal seed regardless
/// (so every hit was already bit-identical audio, just wastefully recomputed 64/16
/// times over the full track for no difference in the result).
fn drum_measure(
    track: &mut Vec<f32>,
    measure_start_secs: f32,
    hat: &[f32],
    kick: &[f32],
    snare: &[f32],
) {
    let at = |eighth: f32| {
        ((measure_start_secs + eighth * EIGHTH) * SAMPLE_RATE as f32).round() as usize
    };
    for e in 0..8 {
        place_at(track, at(e as f32), hat);
    }
    place_at(track, at(0.0), kick);
    for &beat in &[2.0, 6.0] {
        place_at(track, at(beat), snare);
    }
}

fn drum_track() -> Vec<f32> {
    // Every voice scaled well under its own natural loudness before mixing in — a
    // full-volume kit read as a different, busier song competing with the melody, not a
    // light pulse sitting under it.
    let mut hat = chiptune::hihat(false, SAMPLE_RATE);
    for s in hat.iter_mut() {
        *s *= 0.12;
    }
    let mut kick = chiptune::kick(SAMPLE_RATE);
    for s in kick.iter_mut() {
        *s *= 0.35;
    }
    let mut snare = chiptune::snare(SAMPLE_RATE);
    for s in snare.iter_mut() {
        *s *= 0.3;
    }

    let mut out = Vec::new();
    for m in 0..NUM_MEASURES as usize {
        drum_measure(&mut out, m as f32 * MEASURE_SECS, &hat, &kick, &snare);
    }
    out
}

/// The melody alone, low-passed (to round off the square-wave lead's harsher upper
/// harmonics — an un-filtered square reads as aggressive/clippy even well under digital
/// clipping, since the WAV encoder clamps to `[-1.0, 1.0]` regardless; this isn't about
/// literal clipping, it's timbre). Kept split from `comp_track`/`bass_track`/
/// `drum_track` (rather than one `melody_and_comp` doing melody+comp together) so
/// `Sfx::load` can synthesize all four layers as four separate steps with a yield
/// between each — see `Sfx::load`'s own comment on why the whole intro isn't built as
/// one atomic block. Only the melody is filtered — filtering the whole mix instead
/// would dull the drum kit's noise-based hihat/snare, which needs its own
/// high-frequency content to read as percussion rather than a thump.
fn melody_only() -> Vec<f32> {
    let mut melody = render_arpeggio(&korobeiniki(), SfxWave::Square { duty: 0.35 });
    one_pole_lowpass(&mut melody, 3200.0, SAMPLE_RATE);
    melody
}

/// The full intro: melody, comping, a funky bass figure, and a light drum pulse, mixed
/// together — see `melody_only`, `comp_track`, `bass_track`, `drum_track`. Test-only:
/// `Sfx::load` needs this same mix but built as separate yielded steps (see its own
/// comment), so it inlines this function's body rather than calling it; kept here as a
/// convenience for the diagnostic tests below that want the real, fully-mixed intro
/// audio.
///
/// Normalized to a lower peak (`0.65`, not `0.85`) for real headroom, not just style:
/// `render_arpeggio`'s envelope ramps to full `1.0` gain at the end of *every* note's
/// attack regardless of `sustain_level` (standard ADSR shape — sustain only caps the
/// plateau after decay, not the attack peak), so a busy melody has many moments near
/// the mix's true peak, not one. `normalize_peak` only measures discrete sample values;
/// it can't see inter-sample ("true peak") overshoot introduced when a decoder
/// reconstructs the waveform between samples — square waves, with their steep edges and
/// strong high harmonics, are exactly the content most prone to that. `0.85` left too
/// little margin and read as distortion at the loudest moments; `0.65` gives real room.
/// `normalize_peak` only ever scales down, so adding more layers here stays safe
/// regardless — it just raises the overall loudness the peak constraint allows for.
#[cfg(test)]
fn korobeiniki_track() -> Vec<f32> {
    let mut track = mix::sum(&[&melody_only(), &comp_track(), &bass_track(), &drum_track()]);
    mix::normalize_peak(&mut track, 0.65);
    track
}

/// One continuous low-pass pass over `samples` whose cutoff moves linearly from
/// `start_cutoff` down to `end_cutoff` across the buffer — a bright "crack" narrowing
/// into a dull "rumble", rather than `audio::filter::one_pole_lowpass`'s single fixed
/// cutoff (which read as flat/static, more like a muffled thump than an explosion).
/// Written as one continuous IIR pass (filter state carried sample-to-sample) rather
/// than calling the fixed-cutoff filter repeatedly on chunks — chunking would reset the
/// filter's internal state at each chunk boundary, clicking exactly like the desync bug
/// this whole redesign is trying to get away from.
fn swept_lowpass(samples: &mut [f32], start_cutoff: f32, end_cutoff: f32, sample_rate: u32) {
    let n = samples.len().max(1) as f32;
    let dt = 1.0 / sample_rate as f32;
    let mut prev = 0.0f32;
    for (i, s) in samples.iter_mut().enumerate() {
        let cutoff = start_cutoff + (end_cutoff - start_cutoff) * (i as f32 / n);
        let rc = 1.0 / (2.0 * PI * cutoff.max(20.0));
        let alpha = dt / (rc + dt);
        prev += alpha * (*s - prev);
        *s = prev;
    }
}

/// A noise-burst "explosion", timed to exactly `flash_dur` — the visual line-clear
/// flash's own duration (`FLASH_DUR` in `lib.rs`) — so the boom and the blink start and
/// end together. An earlier draft's duration instead scaled with `size` (0.14s for a
/// single line, up to 0.35s for a tetris) while the flash always blinks for a fixed
/// 0.28s, so the sound and the animation drifted out of sync in both directions
/// depending on how many lines cleared.
///
/// The noise sweeps from a bright crack down to a dull rumble over its own length
/// (`swept_lowpass`) instead of one static cutoff, and is mixed *louder* than the
/// sub-bass layer below it — a static-cutoff noise burst plus a plain `chiptune::kick`
/// (the first draft's approach) read as barely distinguishable from `lock`'s own bare
/// kick, not as a distinct "explosion". The sub-bass layer here is also its own
/// deeper/longer voice, not literally `chiptune::kick`, for the same reason. `size` (0
/// for a single line, up to 3 for a tetris) scales loudness/depth, not duration.
fn explosion(size: usize, flash_dur: f32, rng_seed: u64, sample_rate: u32) -> Vec<f32> {
    let grow = size as f32;
    let mut rumble = SfxParams {
        wave: SfxWave::Noise,
        envelope: Envelope {
            attack: flash_dur * 0.02,
            decay: flash_dur * 0.28,
            sustain_level: 0.0,
            sustain: 0.0,
            release: flash_dur * 0.70,
        },
        // `start_freq`/slides are irrelevant for `SfxWave::Noise`.
        start_freq: 1.0,
        freq_slide: 0.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(sample_rate, rng_seed);
    // Bigger clears end up rumblier (a lower cutoff floor), not longer.
    swept_lowpass(
        &mut rumble,
        5500.0,
        (500.0 - grow * 80.0).max(120.0),
        sample_rate,
    );

    let mut boom = SfxParams {
        wave: SfxWave::Triangle,
        envelope: Envelope {
            attack: flash_dur * 0.01,
            decay: flash_dur * 0.5,
            sustain_level: 0.0,
            sustain: 0.0,
            release: flash_dur * 0.49,
        },
        start_freq: 90.0,
        freq_slide: -220.0 - grow * 20.0,
        freq_delta_slide: 0.0,
        arp_semitones: 0.0,
        arp_time_frac: 0.5,
        vibrato_depth: 0.0,
        vibrato_speed: 0.0,
    }
    .render(sample_rate, rng_seed + 50);

    for s in rumble.iter_mut() {
        *s *= 1.4;
    }
    for s in boom.iter_mut() {
        *s *= 0.8;
    }
    let mut out = mix::sum(&[&rumble, &boom]);
    mix::normalize_peak(&mut out, 0.95);
    out
}

/// All of Tetris's synthesized clips, loaded once at startup.
pub struct Sfx {
    pub intro: Clip,
    pub rotate: Clip,
    pub drop: Clip,
    pub lock: Clip,
    /// Indexed by `lines_cleared - 1` (0 = single, 3 = tetris).
    pub clear: [Clip; 4],
    pub game_over: Clip,
}

impl Sfx {
    /// `flash_dur` is `lib.rs`'s `FLASH_DUR` — the visual line-clear flash's own
    /// duration, threaded in so `explosion` can render to exactly that length rather
    /// than guessing/duplicating the constant.
    pub async fn load(flash_dur: f32) -> Self {
        // Every clip used to be synthesized back-to-back in one unbroken block, only
        // `.await`ing (yielding to the browser/event loop) once all nine were already
        // rendered — measured as several 80-230ms "long tasks" blocking the main thread
        // right at page load (Playwright + a `longtask` `PerformanceObserver`), heavy
        // enough on top of whatever else is happening during startup to read as an
        // audio glitch ("sounds like a buffer underrun") even though the rendered
        // samples themselves are click-free. Fix: load each clip (`Clip::from_samples`,
        // already a yield point) right after synthesizing it instead of synthesizing
        // all nine first, plus an explicit extra `next_frame().await` after the
        // heaviest chunks — `rotate`/`drop`/`lock`/`game_over` are each cheap enough
        // that their own `Clip::from_samples().await` is yield enough on its own, so no
        // extra `next_frame` after those specifically.

        // The actual Tetris theme, jazz-comped (see `korobeiniki_track`) — fine as a
        // melodic, once-per-session intro even though the rest of this palette avoids
        // tonal/melodic sounds, since it isn't heard on every move the way
        // rotate/drop/lock/clear are. Built as 4 separate synthesis steps (melody, comp,
        // bass, drums), not one call to `korobeiniki_track`, each followed by its own
        // yield — adding the bass/drum layers made even a single combined
        // melody+comp+bass+drums call heavy enough to reintroduce the long-task problem
        // above (measured: 1 task of 119ms right after the original fix -> 3 tasks up
        // to 202ms once bass/drums were added as one extra synthesis+yield step -> 2
        // tasks up to 159ms splitting melody+comp from bass+drums); splitting all four
        // layers individually is what actually keeps each chunk small.
        let melody = melody_only();
        next_frame().await;
        let comp = comp_track();
        next_frame().await;
        let bass = bass_track();
        next_frame().await;
        let drums = drum_track();
        next_frame().await;
        let mut intro_samples = mix::sum(&[&melody, &comp, &bass, &drums]);
        mix::normalize_peak(&mut intro_samples, 0.65);
        let intro = Clip::from_samples(&intro_samples).await;
        next_frame().await;

        // A soft low click, not a beep — this fires on every piece, so it has to stay
        // unobtrusive. Heavily low-passed noise rather than a tonal square blip.
        let rotate = {
            let mut n = SfxParams {
                wave: SfxWave::Noise,
                envelope: Envelope {
                    attack: 0.001,
                    decay: 0.008,
                    sustain_level: 0.0,
                    sustain: 0.0,
                    release: 0.02,
                },
                start_freq: 1.0,
                freq_slide: 0.0,
                freq_delta_slide: 0.0,
                arp_semitones: 0.0,
                arp_time_frac: 0.5,
                vibrato_depth: 0.0,
                vibrato_speed: 0.0,
            }
            .render(SAMPLE_RATE, 5);
            one_pole_lowpass(&mut n, 700.0, SAMPLE_RATE);
            n
        };
        let rotate = Clip::from_samples(&rotate).await;

        // Hard-drop: a short, low descending thump rather than a mid-range zap. Total
        // envelope kept under `ROTATE_FRAC`'s real-time offset (~0.083s at default
        // speed/`ANIM_SPEED` — see `lib.rs`): the old 0.121s envelope's release tail was
        // still sounding when `rotate`'s own onset (a real "click" by design) fired on
        // top of it, on *every* piece regardless of whether it actually rotated — heard
        // as one "hit" sound with a stray click stapled to its end, not two distinct
        // cues. Shortening it (rather than delaying `rotate`, a visual-timing constant)
        // keeps the fix local to sound.
        let drop = SfxParams {
            wave: SfxWave::Triangle,
            envelope: Envelope {
                attack: 0.001,
                decay: 0.035,
                sustain_level: 0.3,
                sustain: 0.0,
                release: 0.025,
            },
            start_freq: note_freq(-5.0),
            freq_slide: -500.0,
            freq_delta_slide: 0.0,
            arp_semitones: 0.0,
            arp_time_frac: 0.5,
            vibrato_depth: 0.0,
            vibrato_speed: 0.0,
        }
        .render(SAMPLE_RATE, 1);
        let drop = Clip::from_samples(&drop).await;

        // Piece settling with no clear: the same sub-bass thump percussion uses for a
        // kick drum — a real "clunk", not a chirp.
        let lock = Clip::from_samples(&chiptune::kick(SAMPLE_RATE)).await;

        // Line clears: noise-burst explosions, bigger and rumblier per extra line —
        // not a melodic sweep. The 4 explosion renders were the second-heaviest chunk
        // measured (8ms native, more on wasm) — yield once after the batch rather than
        // once per explosion, since 4 extra `next_frame` waits would cost more latency
        // than the batch itself.
        let clear = [
            Clip::from_samples(&explosion(0, flash_dur, 11, SAMPLE_RATE)).await,
            Clip::from_samples(&explosion(1, flash_dur, 12, SAMPLE_RATE)).await,
            Clip::from_samples(&explosion(2, flash_dur, 13, SAMPLE_RATE)).await,
            Clip::from_samples(&explosion(3, flash_dur, 14, SAMPLE_RATE)).await,
        ];
        next_frame().await;

        // Game over: the same descending-arpeggio shape as before, but an octave lower
        // and on the softer triangle voice instead of a sawtooth, so it reads as
        // "deflating" rather than harsh — plus a low rumble underneath for weight.
        let mut game_over = render_arpeggio(
            &[
                (Some(-12.0), 0.18),
                (Some(-15.0), 0.18),
                (Some(-19.0), 0.18),
                (Some(-24.0), 0.5),
            ],
            SfxWave::Triangle,
        );
        let mut rumble = SfxParams {
            wave: SfxWave::Noise,
            envelope: Envelope {
                attack: 0.01,
                decay: 0.2,
                sustain_level: 0.0,
                sustain: 0.0,
                release: 0.5,
            },
            start_freq: 1.0,
            freq_slide: 0.0,
            freq_delta_slide: 0.0,
            arp_semitones: 0.0,
            arp_time_frac: 0.5,
            vibrato_depth: 0.0,
            vibrato_speed: 0.0,
        }
        .render(SAMPLE_RATE, 15);
        one_pole_lowpass(&mut rumble, 300.0, SAMPLE_RATE);
        game_over = mix::sum(&[&game_over, &rumble]);
        mix::normalize_peak(&mut game_over, 0.9);
        let game_over = Clip::from_samples(&game_over).await;

        Self {
            intro,
            rotate,
            drop,
            lock,
            clear,
            game_over,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression test for the melody/comp desync bug: `comp_chord`'s envelope must
    /// render to exactly `MEASURE_SECS` per measure, same as the melody's own notes
    /// (which sum to `MEASURE_SECS` by construction — see `korobeiniki`'s `// mN`
    /// groupings). A hardcoded (non-`MEASURE_SECS`-proportional) envelope field
    /// silently drifts the two apart, worse each measure — allow at most 4 samples of
    /// rounding slop per measure (scales with `EIGHTH`/`MEASURE_SECS`'s own size, hence
    /// not a tighter fixed value), not the ~1,500-sample (~0.036s) gap the original bug
    /// produced.
    #[test]
    fn comp_track_len_matches_melody_len_per_measure() {
        let melody = render_arpeggio(&korobeiniki(), SfxWave::Square { duty: 0.35 });
        let comp = comp_track();
        let tolerance = 4 * CHORD_GUIDE_TONES.len();
        assert!(
            melody.len().abs_diff(comp.len()) <= tolerance,
            "melody {} samples vs comp {} samples (diff {}, tolerance {tolerance})",
            melody.len(),
            comp.len(),
            melody.len().abs_diff(comp.len()),
        );
    }

    /// Same discipline as `comp_track_len_matches_melody_len_per_measure`, for the two
    /// newer layers: `bass_track`'s per-measure staccato-hits-plus-rest arithmetic and
    /// `drum_track`'s per-hit sample-offset rounding can each drift by a few samples
    /// per measure without either being a real bug — `funky_bass_measure` alone rounds
    /// 4 separate times per measure (3 hit+rest pairs, 1 held note), vs. `comp_chord`'s
    /// single render call, hence the wider tolerance here. This catches a *real* desync
    /// (a dropped/duplicated measure, a wrong constant) rather than that rounding noise.
    #[test]
    fn bass_and_drum_tracks_len_matches_melody_len_per_measure() {
        let melody = render_arpeggio(&korobeiniki(), SfxWave::Square { duty: 0.35 });
        let tolerance = 4 * BASS_ROOTS.len();
        let bass = bass_track();
        assert!(
            melody.len().abs_diff(bass.len()) <= tolerance,
            "melody {} samples vs bass {} samples (diff {}, tolerance {tolerance})",
            melody.len(),
            bass.len(),
            melody.len().abs_diff(bass.len()),
        );
        // `drum_track` isn't checked against `melody.len()` the same way: it's only as
        // long as its very last hit's own short release tail reaches (the final
        // eighth's hihat, here — its ~0.03s release doesn't reach all the way to the
        // measure's true end), which is expected, not a desync — nothing plays there
        // regardless. Sanity-check it's in the right ballpark instead of empty/runaway.
        let drums = drum_track();
        assert!(!drums.is_empty());
        assert!(drums.len() < melody.len() + SAMPLE_RATE as usize);
    }

    /// No guide tone should sit a semitone (or its octave-equivalent, 11 semitones) away
    /// from another guide tone in the *same* chord — that would just be a mistake (e.g.
    /// a chord tone typo'd a half-step off), not intentional jazz tension. This doesn't
    /// check guide tones against the melody itself (that needs the actual per-measure
    /// note lists, hand-verified instead — see `CHORD_GUIDE_TONES`'s doc comment for the
    /// method), just a cheap internal sanity check on the table.
    #[test]
    fn chord_guide_tones_not_a_semitone_apart() {
        for &(third, seventh) in &CHORD_GUIDE_TONES {
            let interval = (third - seventh).rem_euclid(12.0);
            assert!(
                interval != 1.0 && interval != 11.0,
                "guide tones {third} and {seventh} are a semitone apart"
            );
        }
    }

    /// Diagnostic: how long the pure-CPU synthesis side of `Sfx::load` actually takes
    /// (not the async decode) — a slow synth on a loaded main thread would stutter
    /// audio right as playback starts, which can read as a "buffer underrun" even
    /// though the rendered samples themselves are clean (see `scan_for_clicks`).
    /// `cargo test -p tetris sound::tests::time_synthesis --release -- --ignored
    /// --nocapture`.
    #[test]
    #[ignore]
    fn time_synthesis() {
        let start = std::time::Instant::now();
        let intro = korobeiniki_track();
        let t_intro = start.elapsed();

        let start = std::time::Instant::now();
        let _clear: Vec<Vec<f32>> = (0..4)
            .map(|i| explosion(i, 0.28, 11 + i as u64, SAMPLE_RATE))
            .collect();
        let t_clear = start.elapsed();

        let start = std::time::Instant::now();
        let _lock = chiptune::kick(SAMPLE_RATE);
        let t_lock = start.elapsed();

        println!(
            "intro: {:?} ({} samples, {:.2}s of audio)",
            t_intro,
            intro.len(),
            intro.len() as f32 / SAMPLE_RATE as f32
        );
        println!("clear x4: {t_clear:?}");
        println!("lock: {t_lock:?}");
        println!(
            "total (excludes rotate/drop/game_over, all cheaper): {:?}",
            t_intro + t_clear + t_lock
        );
    }

    /// Diagnostic, not a real assertion yet: scans every clip this module renders for
    /// sample-to-sample jumps way outside what a smooth signal (even a square wave's
    /// edges, which are the sharpest legitimate feature here) should produce, and
    /// prints where they are. `cargo test -p tetris sound::tests::scan_for_clicks --
    /// --ignored --nocapture`.
    #[test]
    #[ignore]
    fn scan_for_clicks() {
        fn report(name: &str, samples: &[f32]) {
            // A legitimate square-wave edge jumps by up to 2.0 in one sample; anything
            // meaningfully bigger than that in a signal that's already been
            // low-passed/mixed/normalized is not a waveform feature.
            const THRESHOLD: f32 = 1.2;
            let mut worst: Vec<(usize, f32)> = Vec::new();
            for w in samples.windows(2) {
                let d = (w[1] - w[0]).abs();
                if d > THRESHOLD {
                    worst.push((0, d));
                }
            }
            let count = worst.len();
            worst.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
            for (i, w) in samples.windows(2).enumerate() {
                let d = (w[1] - w[0]).abs();
                if d > THRESHOLD && worst.iter().take(5).any(|&(_, dd)| dd == d) {
                    println!(
                        "{name}: jump {d:.3} at sample {i} (t={:.4}s) {:.3} -> {:.3}",
                        i as f32 / SAMPLE_RATE as f32,
                        w[0],
                        w[1]
                    );
                }
            }
            println!(
                "{name}: {count} jumps > {THRESHOLD}, {} samples total",
                samples.len()
            );
        }

        report("korobeiniki_track", &korobeiniki_track());
        report("lock", &chiptune::kick(SAMPLE_RATE));
        report("explosion[3]", &explosion(3, 0.28, 14, SAMPLE_RATE));
        report("game_over", &{
            let mut go = render_arpeggio(
                &[
                    (Some(-12.0), 0.18),
                    (Some(-15.0), 0.18),
                    (Some(-19.0), 0.18),
                    (Some(-24.0), 0.5),
                ],
                SfxWave::Triangle,
            );
            let mut rumble = SfxParams {
                wave: SfxWave::Noise,
                envelope: Envelope {
                    attack: 0.01,
                    decay: 0.2,
                    sustain_level: 0.0,
                    sustain: 0.0,
                    release: 0.5,
                },
                start_freq: 1.0,
                freq_slide: 0.0,
                freq_delta_slide: 0.0,
                arp_semitones: 0.0,
                arp_time_frac: 0.5,
                vibrato_depth: 0.0,
                vibrato_speed: 0.0,
            }
            .render(SAMPLE_RATE, 15);
            one_pole_lowpass(&mut rumble, 300.0, SAMPLE_RATE);
            go = mix::sum(&[&go, &rumble]);
            mix::normalize_peak(&mut go, 0.9);
            go
        });
    }
}
