//! The boss fight's HUD (Tower): while the bot argues with the boss, a game-style boss
//! bar across the top of the view — "THE BOSS", HP counting down from over 9000 — the
//! latest hit, and the argument itself as alternating lines at the bottom. Once the
//! boss's HP hits zero, a banner with what he concedes.

use crate::ARGUE_LINE;
use crate::banter::{BOSS_RETORTS, BOT_JABS, CONCESSIONS, LADY_RETORTS, boss_line};
use crate::game::{BOSS_HP, Game, boss_hp_at};
use crate::view::{HEADER_H, hud};
use macroquad::prelude::*;

/// Font sizes this HUD draws at (prewarmed with `texts`).
pub const SIZES: [u16; 3] = [20, 22, 28];
/// How long the concession banner stays up (seconds).
const BANNER_SECS: f32 = 4.5;

/// Every string the HUD can draw, for prewarming the font atlas.
pub fn texts() -> Vec<&'static str> {
    let mut v = vec![
        "THE BOSS",
        "HP 0123456789/ -",
        "BOSS DEFEATED",
        "The Boss concedes:",
        "ROBOT:",
        "BOSS:",
        BUSY,
    ];
    v.extend_from_slice(BOT_JABS);
    v.extend_from_slice(BOSS_RETORTS);
    v.extend_from_slice(LADY_RETORTS);
    v.extend_from_slice(CONCESSIONS);
    v
}

/// An occupied washroom's answer, captioned while it plays.
pub const BUSY: &str = "(from inside) Occupied!";
const BUSY_SECS: f32 = 1.6;

/// The caption for an occupied washroom's answer, `since` seconds after the knock.
pub fn draw_busy(stage: Rect, since: f32) {
    if since >= BUSY_SECS {
        return;
    }
    let a = ((BUSY_SECS - since) / 0.4).clamp(0.0, 1.0);
    let cx = stage.x + stage.w * 0.5;
    let col = Color::new(0.95, 0.9, 0.8, a);
    let bg = Color::new(PANEL.r, PANEL.g, PANEL.b, PANEL.a * a);
    hud::panel(
        &[(BUSY, 22, col)],
        cx,
        stage.y + stage.h - 48.0,
        12.0,
        0.0,
        bg,
    );
}

/// HUD panels are translucent: the view stays readable behind them.
const PANEL: Color = Color::new(0.02, 0.02, 0.04, 0.5);

pub fn draw(stage: Rect, game: &Game, since_defeat: Option<f32>) {
    let Some(boss) = &game.boss else {
        return;
    };
    let cx = stage.x + stage.w * 0.5;
    // Tick 0 is the door swinging open and the bot stepping up to the desk: the bar
    // comes up with the first line of the argument, not before.
    if let Some(t) = game.arguing().filter(|&t| t > 0) {
        // The bar, under the header.
        let (w, h) = (stage.w * 0.6, 20.0);
        let (x, y) = (cx - w * 0.5, HEADER_H + 40.0);
        // On its own dark plate: bare gold text vanished into the ceiling panels.
        hud::panel(
            &[("THE BOSS", 28, Color::new(1.0, 0.85, 0.4, 1.0))],
            cx,
            y - 20.0,
            6.0,
            0.0,
            Color::new(0.05, 0.02, 0.02, 0.75),
        );
        draw_rectangle(
            x - 3.0,
            y - 3.0,
            w + 6.0,
            h + 6.0,
            Color::new(0.05, 0.02, 0.02, 0.6),
        );
        draw_rectangle(x, y, w, h, Color::new(0.25, 0.04, 0.04, 0.6));
        let frac = boss.hp as f32 / BOSS_HP as f32;
        draw_rectangle(x, y, w * frac, h, Color::new(0.85, 0.1, 0.12, 0.85));
        draw_rectangle(x, y, w * frac, h * 0.35, Color::new(1.0, 0.45, 0.4, 0.85));
        hud::centred(
            &format!("HP {} / {}", boss.hp, BOSS_HP),
            cx,
            y + h * 0.5,
            20,
            WHITE,
        );
        // The latest hit, just under the bar where the HP ends.
        let hit = boss_hp_at(t.saturating_sub(1)).saturating_sub(boss_hp_at(t));
        if hit > 0 {
            hud::centred(
                &format!("-{hit}"),
                x + w * frac,
                y + h + 18.0,
                22,
                Color::new(1.0, 0.9, 0.3, 1.0),
            );
        }
        // The argument: speakers alternate every `ARGUE_LINE` ticks.
        let k = (t / ARGUE_LINE) as usize;
        let (who, line, col) = if k.is_multiple_of(2) {
            (
                "ROBOT:",
                BOT_JABS[(boss.banter + k / 2) % BOT_JABS.len()],
                Color::new(0.5, 0.95, 1.0, 1.0),
            )
        } else {
            (
                "BOSS:",
                boss_line(game.maze.lady_boss, boss.banter, k / 2),
                Color::new(1.0, 0.7, 0.35, 1.0),
            )
        };
        let text = format!("{who} {line}");
        let cy = stage.y + stage.h - 48.0;
        hud::panel(&[(&text, 22, col)], cx, cy, 12.0, 0.0, PANEL);
        return;
    }
    let Some(since) = since_defeat.filter(|&s| s < BANNER_SECS) else {
        return;
    };
    let a = ((BANNER_SECS - since) / 0.8).clamp(0.0, 1.0);
    let concession = CONCESSIONS[boss.concession % CONCESSIONS.len()];
    hud::panel(
        &[
            ("BOSS DEFEATED", 28, Color::new(1.0, 0.85, 0.3, a)),
            ("The Boss concedes:", 20, Color::new(0.85, 0.85, 0.9, a)),
            (concession, 22, Color::new(1.0, 1.0, 1.0, a)),
        ],
        cx,
        stage.y + stage.h * 0.3,
        24.0,
        12.0,
        Color::new(PANEL.r, PANEL.g, PANEL.b, PANEL.a * a),
    );
}
