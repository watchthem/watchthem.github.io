//! Screen-space HUD text, placed by measured metrics rather than hand-tuned baselines:
//! a line's ink box is centred on the point asked for, and a panel is sized to its
//! text plus padding.

use macroquad::prelude::*;

/// Baseline that vertically centres a line of capitals at `cy`. From the cap height,
/// not the string's own extent, so lines with and without descenders sit level.
pub fn baseline(cy: f32, size: u16) -> f32 {
    cy + measure_text("H", None, size, 1.0).height * 0.5
}

/// `text` centred on `(cx, cy)`.
pub fn centred(text: &str, cx: f32, cy: f32, size: u16, col: Color) {
    let w = measure_text(text, None, size, 1.0).width;
    draw_text(text, cx - w * 0.5, baseline(cy, size), size as f32, col);
}

/// A translucent box `pad` px round the widest of `lines` (each `(text, size, colour)`),
/// stacked `gap` px apart and all centred on `(cx, cy)`.
pub fn panel(lines: &[(&str, u16, Color)], cx: f32, cy: f32, pad: f32, gap: f32, bg: Color) {
    let caps: Vec<f32> = lines
        .iter()
        .map(|&(_, s, _)| measure_text("H", None, s, 1.0).height)
        .collect();
    let w = lines
        .iter()
        .map(|&(t, s, _)| measure_text(t, None, s, 1.0).width)
        .fold(0.0, f32::max);
    let h = caps.iter().sum::<f32>() + gap * (lines.len() as f32 - 1.0).max(0.0);
    draw_rectangle(
        cx - w * 0.5 - pad,
        cy - h * 0.5 - pad,
        w + pad * 2.0,
        h + pad * 2.0,
        bg,
    );
    let mut y = cy - h * 0.5;
    for (&(t, s, col), cap) in lines.iter().zip(&caps) {
        centred(t, cx, y + cap * 0.5, s, col);
        y += cap + gap;
    }
}
