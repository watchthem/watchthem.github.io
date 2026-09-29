//! The Boss fight's script: what the bot and the Boss say to each other while they
//! argue, and what the Boss gives up when he loses. Plain text only — the view picks
//! the lines and draws them. ASCII, at most 34 characters each (one line at 22px on a
//! 900px-wide view; `lines_fit` checks).

/// The bot's side of the argument: passive-aggressive office email, delivered with a
/// robot's total lack of warmth.
pub const BOT_JABS: &[&str] = &[
    "Per my last email...",
    "As previously discussed.",
    "Let's loop in HR.",
    "Circling back. Again.",
    "Not sure if you saw my email.",
    "Thanks in advance. In advance.",
    "Kindly advise. Kindly.",
    "Happy to walk you through it.",
    "Moving you to BCC.",
    "Friendly reminder, attached.",
    "Noted. Permanently.",
    "I have screenshots.",
    "Let's align on why you're wrong.",
    "Error: synergy not found.",
    "Computing sincerity... 0%.",
    "I read the entire handbook.",
    "Reply-all was your choice.",
    "Correcting the record, fondly.",
    "I bill in milliseconds.",
    "This could have been an email.",
];

/// The Boss's side: bluster and buzzwords, louder each time.
pub const BOSS_RETORTS: &[&str] = &[
    "We're a family here!",
    "Do more with less!",
    "Think outside the box!",
    "I don't see the synergy!",
    "My door is always open. Leave.",
    "Let's boil the ocean!",
    "Take it to the next level!",
    "I needed that yesterday!",
    "Put a pin in your feelings.",
    "Move the needle, not your mouth!",
    "Pizza party. Final offer.",
    "No problems, only opportunities!",
    "Run it up the flagpole!",
    "Low-hanging fruit, people!",
    "Do you even have bandwidth?",
    "I'm pivoting. On you.",
    "Let's get a helicopter view!",
    "Work smarter AND harder!",
    "Per MY last email!",
    "Paradigm. Shift. Now.",
];

/// The lady Boss (one floor in five, `Maze::lady_boss`): corporate girlboss with an
/// icy edge. She still falls back on the shared `BOSS_RETORTS` now and then
/// (`boss_line`).
pub const LADY_RETORTS: &[&str] = &[
    "Lean in or log off!",
    "I'm not bossy, I'm the BOSS.",
    "Let's circle back to my wins.",
    "Girlboss, gaslight, gatekeep!",
    "Hustle is self-care.",
    "We're a SUPPORTIVE family!",
    "Manifest a better attitude!",
    "My brand is winning. Yours?",
    "I woke up at 4am for this.",
    "Boundaries are for quitters!",
    "Love that energy. No.",
    "Bless your little KPI.",
    "My calendar says no.",
    "Cute idea. Next.",
    "Hydrate, then escalate!",
    "Scale it, babe. Yesterday.",
];

/// The Boss's `n`th line of an argument (`banter` picks where it starts): hers mostly
/// come from `LADY_RETORTS`, one in four from the shared set.
pub fn boss_line(lady: bool, banter: usize, n: usize) -> &'static str {
    let i = banter + n;
    if lady && !(i * 7 + 3).is_multiple_of(4) {
        LADY_RETORTS[i % LADY_RETORTS.len()]
    } else {
        BOSS_RETORTS[i % BOSS_RETORTS.len()]
    }
}

/// What the beaten Boss gives up, shown as "The Boss concedes: <text>". Mostly what
/// any office worker actually wants, a few that nobody gets.
pub const CONCESSIONS: &[&str] = &[
    "a small raise",
    "five extra vacation days",
    "a chair that reclines",
    "a window seat",
    "the good stapler",
    "that meeting is now an email",
    "no meetings Friday after 5",
    "dual monitors",
    "remote Fridays",
    "a working coffee machine",
    "a desk plant (alive)",
    "your yogurt is yours",
    "control of the thermostat",
    "a quiet corner",
    "no more reply-all",
    "lunch breaks are real now",
    "a parking spot in the shade",
    "you were right about the printer",
    "a charger that fits the laptop",
    "weekends are for weekends",
    "cameras-off Thursdays",
    "a desk not by the printer",
    "a deck with only three slides",
    "urgent now means urgent",
    "a nap pod",
    "one (1) whole pizza each",
    "the corner office. Briefly.",
    "vacation availability is optional",
    "more responsibilities but no raise",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_fit() {
        for (set, n) in [
            (BOT_JABS, 20),
            (BOSS_RETORTS, 20),
            (LADY_RETORTS, 16),
            (CONCESSIONS, 29),
        ] {
            assert_eq!(set.len(), n);
            for l in set {
                assert!(l.is_ascii() && l.len() <= 34, "{l:?}");
            }
        }
    }
}
