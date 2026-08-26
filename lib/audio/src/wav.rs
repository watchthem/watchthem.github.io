//! In-memory PCM -> WAV encoding (`hound`, zero further deps, no filesystem) — the
//! format `macroquad::audio::load_sound_from_bytes` auto-detects and decodes on both
//! native (via `audrey`) and wasm32 (via the browser's `decodeAudioData`).

use std::io::Cursor;

/// Every clip in this crate renders at this rate. quad-snd's Linux ALSA backend panics
/// ("Can't set rate") if asked to open a device at a rate the running audio server isn't
/// already using — 44100 is the safe common choice (see quad-snd issue #49).
pub const SAMPLE_RATE: u32 = 44_100;

/// Encodes mono `f32` samples (expected in `[-1.0, 1.0]`, out-of-range values are
/// clamped) as a 16-bit PCM WAV, returned as bytes ready for
/// `macroquad::audio::load_sound_from_bytes`.
pub fn encode_mono_wav(samples: &[f32], sample_rate: u32) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(&mut cursor, spec).expect("valid wav spec");
        for &s in samples {
            let clamped = s.clamp(-1.0, 1.0);
            writer
                .write_sample((clamped * i16::MAX as f32) as i16)
                .expect("wav sample write");
        }
        writer.finalize().expect("wav finalize");
    }
    cursor.into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_hound() {
        let samples: Vec<f32> = (0..1000).map(|i| (i as f32 * 0.01).sin()).collect();
        let bytes = encode_mono_wav(&samples, SAMPLE_RATE);
        let mut reader = hound::WavReader::new(Cursor::new(bytes)).expect("valid wav");
        assert_eq!(reader.spec().sample_rate, SAMPLE_RATE);
        assert_eq!(reader.spec().channels, 1);
        let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded.len(), samples.len());
    }

    #[test]
    fn clamps_out_of_range_samples() {
        let bytes = encode_mono_wav(&[2.0, -2.0], SAMPLE_RATE);
        let mut reader = hound::WavReader::new(Cursor::new(bytes)).expect("valid wav");
        let decoded: Vec<i16> = reader.samples::<i16>().map(|s| s.unwrap()).collect();
        assert_eq!(decoded, vec![i16::MAX, -i16::MAX]);
    }

    #[test]
    fn empty_samples_still_valid_wav() {
        let bytes = encode_mono_wav(&[], SAMPLE_RATE);
        let reader = hound::WavReader::new(Cursor::new(bytes)).expect("valid wav");
        assert_eq!(reader.len(), 0);
    }
}
