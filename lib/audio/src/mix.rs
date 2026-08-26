//! Combining multiple rendered voice buffers into one — percussion layering today
//! (`chiptune::snare` sums a tonal body with filtered noise), a full multi-voice
//! mixdown later (music sequencing).

/// Sums buffers sample-by-sample, padding shorter ones with silence. No gain
/// compensation — call `normalize_peak` afterward if the sum can plausibly clip.
pub fn sum(buffers: &[&[f32]]) -> Vec<f32> {
    let len = buffers.iter().map(|b| b.len()).max().unwrap_or(0);
    let mut out = vec![0.0f32; len];
    for buf in buffers {
        for (o, s) in out.iter_mut().zip(buf.iter()) {
            *o += *s;
        }
    }
    out
}

/// Scales `samples` down (never up) so the peak absolute amplitude is at most
/// `target_peak`. A no-op if already under that peak.
pub fn normalize_peak(samples: &mut [f32], target_peak: f32) {
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    if peak > target_peak && peak > 0.0 {
        let scale = target_peak / peak;
        for s in samples.iter_mut() {
            *s *= scale;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_equal_length_buffers() {
        let a = [0.1, 0.2, 0.3];
        let b = [0.5, -0.1, 0.0];
        let out = sum(&[&a, &b]);
        assert!((out[0] - 0.6).abs() < 1e-6);
        assert!((out[1] - 0.1).abs() < 1e-6);
        assert!((out[2] - 0.3).abs() < 1e-6);
    }

    #[test]
    fn pads_shorter_buffer_with_silence() {
        let a = [1.0, 1.0, 1.0, 1.0];
        let b = [1.0];
        let out = sum(&[&a, &b]);
        assert_eq!(out.len(), 4);
        assert!((out[0] - 2.0).abs() < 1e-6);
        assert!((out[1] - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalize_scales_down_to_target() {
        let mut s = [2.0, -1.0, 0.5];
        normalize_peak(&mut s, 1.0);
        assert!((s[0] - 1.0).abs() < 1e-6);
        assert!((s[1] - -0.5).abs() < 1e-6);
    }

    #[test]
    fn normalize_leaves_quiet_signal_untouched() {
        let mut s = [0.1, -0.2, 0.05];
        let before = s;
        normalize_peak(&mut s, 1.0);
        assert_eq!(s, before);
    }

    #[test]
    fn normalize_handles_silence() {
        let mut s = [0.0, 0.0];
        normalize_peak(&mut s, 1.0);
        assert_eq!(s, [0.0, 0.0]);
    }
}
