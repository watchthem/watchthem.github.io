//! Equal-temperament frequency helper, shared by anything that names notes by pitch
//! instead of raw Hz (`chiptune`'s instrument voices, a game's own melody/jingle code).

/// `440 * 2^(semitones/12)` — frequency, in Hz, `semitones` above/below A4 (440 Hz).
pub fn note_freq(semitones: f32) -> f32 {
    440.0 * 2f32.powf(semitones / 12.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a4_is_440() {
        assert!((note_freq(0.0) - 440.0).abs() < 1e-3);
    }

    #[test]
    fn octave_up_doubles() {
        assert!((note_freq(12.0) - 880.0).abs() < 1e-2);
    }

    #[test]
    fn octave_down_halves() {
        assert!((note_freq(-12.0) - 220.0).abs() < 1e-2);
    }
}
