//! House star from `assets/sigils/pixie.txt`. Splash and wait spinner
//! are frames of this glyph — not a generic ticker.

use std::time::Instant;

/// Canonical 7-line star (same art as the hero SVG).
pub const SIGIL: [&str; 7] = [
    "   *   ", r"  \|/  ", "   |   ", "---*---", "   |   ", r"  /|\  ", "   *   ",
];

/// Full-pane splash: the star with arms pulsing / rotating.
pub const SPLASH: [[&str; 7]; 4] = [
    SIGIL,
    [
        r" \ * / ", r"  \|/  ", "   |   ", "---*---", "   |   ", r"  /|\  ", r" / * \ ",
    ],
    [
        " *   * ", r"  \ /  ", "   *   ", "-- * --", "   *   ", r"  / \  ", " *   * ",
    ],
    [
        "       ", "   *   ", r"  \|/  ", "---*---", r"  /|\  ", "   *   ", "       ",
    ],
];

/// One-line spinner: the four distinctive rows of the house star, in order.
pub const SPIN: [&str; 4] = ["   *   ", r"  \|/  ", "---*---", r"  /|\  "];

pub const WAIT: [&str; 8] = [
    "waking the menagerie…",
    "poking around…",
    "dusting off her spellbook…",
    "braiding a reply out of moonlight…",
    "counting sparkles in the cache…",
    "whispering to the inode fairies…",
    "humming a little compile song…",
    "listening for a good idea…",
];

const FRAME_MS: u128 = 140;
const SPLASH_FRAME_MS: u128 = 350;
const WAIT_MS: u128 = 5000;

pub fn splash_frame(born: Instant) -> &'static [&'static str] {
    &SPLASH[idx(born, SPLASH_FRAME_MS, SPLASH.len())]
}

pub fn spin_line(born: Instant) -> &'static str {
    SPIN[idx(born, FRAME_MS, SPIN.len())]
}

pub fn wait_line(born: Instant) -> &'static str {
    WAIT[idx(born, WAIT_MS, WAIT.len())]
}

fn idx(born: Instant, period_ms: u128, n: usize) -> usize {
    let n = n.max(1);
    ((born.elapsed().as_millis() / period_ms) as usize) % n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spin_rows_are_the_star() {
        for row in SPIN {
            assert!(
                SIGIL.contains(&row),
                "spinner row {row:?} is not a line of the house star"
            );
        }
    }

    #[test]
    fn splash_keeps_width() {
        for frame in &SPLASH {
            for line in frame {
                assert_eq!(line.chars().count(), 7, "{line:?}");
            }
        }
    }

    #[test]
    fn first_splash_is_the_sigil() {
        assert_eq!(SPLASH[0], SIGIL);
    }

    #[test]
    fn wait_holds_long_enough_to_read() {
        assert!(WAIT_MS >= 5000, "wait lines were flashing at {WAIT_MS} ms");
        assert!(
            SPLASH_FRAME_MS >= 300,
            "splash poses were {SPLASH_FRAME_MS} ms"
        );
    }
}
