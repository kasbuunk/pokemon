//! Heuristic Red/Blue vs Yellow detection.
//!
//! The two layouts are byte-identical (FORMAT.md "Red/Blue vs Yellow"),
//! so there is no authoritative marker; detection only affects UI
//! labeling, never parsing.

use super::offsets;
use super::save::{GameVariant, SaveFile};

/// Guess which cartridge wrote this save: [`GameVariant::Yellow`] iff
/// the Pikachu-friendship byte (0x271C, meaningful only in Yellow) is
/// nonzero, else [`GameVariant::RedBlue`].
///
/// Known weaknesses of the heuristic:
///
/// - a Yellow save whose Pikachu friendship happens to be exactly 0
///   (possible after trading the starter away and letting friendship
///   drain) reads as Red/Blue;
/// - a Red/Blue save with garbage in that unused byte reads as Yellow.
///
/// In practice Red/Blue leaves the byte 0 and Yellow starts it at 90,
/// so the guess is almost always right — but treat it as a label, not a
/// fact.
pub fn detect_variant(save: &SaveFile) -> GameVariant {
    if save.as_bytes()[offsets::PIKACHU_FRIENDSHIP] != 0 {
        GameVariant::Yellow
    } else {
        GameVariant::RedBlue
    }
}
