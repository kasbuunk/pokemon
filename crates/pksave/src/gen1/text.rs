//! The Gen 1 English text codec.
//!
//! Byte-to-glyph mapping follows the pret/pokered `constants/charmap.asm`
//! (English "actual characters" set). Mapping choices, documented for
//! round-trip stability:
//!
//! - `0x50` is the string terminator. [`decode`] stops at it (nothing is
//!   emitted for it) and [`encode`] appends it and pads with it, so `'@'`
//!   (pokered's name for `0x50`) is intentionally *not* encodable text.
//! - Apostrophe ligatures `0xBB..=0xBF`, `0xE4`, `0xE5` map to two-char
//!   strings `'d 'l 's 't 'v 'r 'm` (ASCII apostrophe), and `0xE0` to a
//!   lone `'`. Encoding is greedy longest-match, so `"'d"` encodes to the
//!   single byte `0xBB`; the two-byte sequence `[0xE0, 0xA3]` decodes to
//!   the same string and is therefore non-canonical.
//! - The `PK`/`MN` glyphs `0xE1`/`0xE2` map to `"<PK>"`/`"<MN>"`, the
//!   decimal point `0xF2` to `"<DOT>"` (distinct from `.` = `0xE8`, per
//!   charmap.asm), and the `POKé` macro `0x54` to `"#"` exactly as in
//!   charmap.asm.
//! - Unknown bytes decode to U+FFFD (`�`) and `�` is not encodable, so
//!   damaged names are visible but cannot be written back accidentally.

use thiserror::Error;

/// String terminator and canonical padding byte.
pub const TERMINATOR: u8 = 0x50;

/// The printable byte ⇄ string map (a bijection). Multi-char entries are
/// documented in the module docs. Sorted by byte for readability.
pub const CHARMAP: &[(u8, &str)] = &[
    (0x54, "#"), // the "POKé" macro glyph
    (0x70, "‘"),
    (0x71, "’"),
    (0x72, "“"),
    (0x73, "”"),
    (0x74, "·"),
    (0x75, "…"),
    (0x79, "┌"),
    (0x7A, "─"),
    (0x7B, "┐"),
    (0x7C, "│"),
    (0x7D, "└"),
    (0x7E, "┘"),
    (0x7F, " "),
    (0x80, "A"),
    (0x81, "B"),
    (0x82, "C"),
    (0x83, "D"),
    (0x84, "E"),
    (0x85, "F"),
    (0x86, "G"),
    (0x87, "H"),
    (0x88, "I"),
    (0x89, "J"),
    (0x8A, "K"),
    (0x8B, "L"),
    (0x8C, "M"),
    (0x8D, "N"),
    (0x8E, "O"),
    (0x8F, "P"),
    (0x90, "Q"),
    (0x91, "R"),
    (0x92, "S"),
    (0x93, "T"),
    (0x94, "U"),
    (0x95, "V"),
    (0x96, "W"),
    (0x97, "X"),
    (0x98, "Y"),
    (0x99, "Z"),
    (0x9A, "("),
    (0x9B, ")"),
    (0x9C, ":"),
    (0x9D, ";"),
    (0x9E, "["),
    (0x9F, "]"),
    (0xA0, "a"),
    (0xA1, "b"),
    (0xA2, "c"),
    (0xA3, "d"),
    (0xA4, "e"),
    (0xA5, "f"),
    (0xA6, "g"),
    (0xA7, "h"),
    (0xA8, "i"),
    (0xA9, "j"),
    (0xAA, "k"),
    (0xAB, "l"),
    (0xAC, "m"),
    (0xAD, "n"),
    (0xAE, "o"),
    (0xAF, "p"),
    (0xB0, "q"),
    (0xB1, "r"),
    (0xB2, "s"),
    (0xB3, "t"),
    (0xB4, "u"),
    (0xB5, "v"),
    (0xB6, "w"),
    (0xB7, "x"),
    (0xB8, "y"),
    (0xB9, "z"),
    (0xBA, "é"),
    (0xBB, "'d"),
    (0xBC, "'l"),
    (0xBD, "'s"),
    (0xBE, "'t"),
    (0xBF, "'v"),
    (0xE0, "'"),
    (0xE1, "<PK>"),
    (0xE2, "<MN>"),
    (0xE3, "-"),
    (0xE4, "'r"),
    (0xE5, "'m"),
    (0xE6, "?"),
    (0xE7, "!"),
    (0xE8, "."),
    (0xEC, "▷"),
    (0xED, "▶"),
    (0xEE, "▼"),
    (0xEF, "♂"),
    (0xF0, "¥"),
    (0xF1, "×"),
    (0xF2, "<DOT>"),
    (0xF3, "/"),
    (0xF4, ","),
    (0xF5, "♀"),
    (0xF6, "0"),
    (0xF7, "1"),
    (0xF8, "2"),
    (0xF9, "3"),
    (0xFA, "4"),
    (0xFB, "5"),
    (0xFC, "6"),
    (0xFD, "7"),
    (0xFE, "8"),
    (0xFF, "9"),
];

/// Text encoding failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TextError {
    /// A character (reported as the first char of the unmatched input)
    /// has no Gen 1 encoding.
    #[error("character {0:?} has no Gen 1 encoding")]
    Unencodable(char),
    /// The encoded text plus its terminator does not fit the field.
    #[error(
        "encoded text needs {needed} bytes (terminator included) but the field is {field_len}"
    )]
    TooLong {
        /// Bytes the encoded text needs, terminator included.
        needed: usize,
        /// Size of the destination field in bytes.
        field_len: usize,
    },
}

/// Decode a Gen 1 text field. Stops at the `0x50` terminator; bytes with
/// no printable mapping become U+FFFD.
pub fn decode(bytes: &[u8]) -> String {
    let mut out = String::new();
    for &b in bytes {
        if b == TERMINATOR {
            break;
        }
        match CHARMAP.iter().find(|&&(byte, _)| byte == b) {
            Some(&(_, s)) => out.push_str(s),
            None => out.push('\u{FFFD}'),
        }
    }
    out
}

/// Encode `s` into exactly `field_len` bytes: the encoded characters, a
/// `0x50` terminator, then `0x50` padding. Matching is greedy
/// longest-first, so ligatures like `"'d"` win over `'` + `d`.
pub fn encode(s: &str, field_len: usize) -> Result<Vec<u8>, TextError> {
    let mut bytes = Vec::new();
    let mut rest = s;
    while !rest.is_empty() {
        let longest_match = CHARMAP
            .iter()
            .filter(|&&(_, text)| rest.starts_with(text))
            .max_by_key(|&&(_, text)| text.len());
        match longest_match {
            Some(&(byte, text)) => {
                bytes.push(byte);
                rest = &rest[text.len()..];
            }
            None => {
                let c = rest.chars().next().unwrap_or('\u{FFFD}');
                return Err(TextError::Unencodable(c));
            }
        }
    }
    if bytes.len() + 1 > field_len {
        return Err(TextError::TooLong {
            needed: bytes.len() + 1,
            field_len,
        });
    }
    bytes.resize(field_len, TERMINATOR);
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_red() {
        assert_eq!(decode(&[0x91, 0x84, 0x83, 0x50, 0x50, 0x50]), "RED");
    }

    #[test]
    fn encodes_red_with_terminator_padding() {
        assert_eq!(
            encode("RED", 11).expect("fits"),
            vec![0x91, 0x84, 0x83, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50]
        );
    }

    #[test]
    fn pikachu_round_trips() {
        let bytes = [0x8F, 0x88, 0x8A, 0x80, 0x82, 0x87, 0x94];
        assert_eq!(decode(&bytes), "PIKACHU");
        assert_eq!(
            encode("PIKACHU", 8).expect("fits"),
            [&bytes[..], &[0x50]].concat()
        );
    }

    #[test]
    fn decode_stops_at_terminator() {
        assert_eq!(decode(&[0x80, 0x50, 0x81]), "A");
        assert_eq!(decode(&[0x50]), "");
        assert_eq!(decode(&[]), "");
    }

    #[test]
    fn unknown_bytes_decode_to_replacement_char() {
        // 0x00 (<NULL>) and 0x60 (unused bold glyph) are not printable text.
        assert_eq!(decode(&[0x00, 0x80, 0x60]), "\u{FFFD}A\u{FFFD}");
    }

    #[test]
    fn apostrophe_ligatures_use_single_bytes() {
        assert_eq!(decode(&[0xBD]), "'s");
        assert_eq!(encode("'s", 3).expect("fits"), vec![0xBD, 0x50, 0x50]);
        assert_eq!(encode("'r", 3).expect("fits"), vec![0xE4, 0x50, 0x50]);
        // Lone apostrophe stays 0xE0.
        assert_eq!(encode("'", 2).expect("fits"), vec![0xE0, 0x50]);
        // Non-canonical [0xE0, 'd'] decodes to the same text as 0xBB.
        assert_eq!(decode(&[0xE0, 0xA3]), "'d");
        assert_eq!(encode("'d", 3).expect("fits"), vec![0xBB, 0x50, 0x50]);
    }

    #[test]
    fn pk_mn_glyphs_round_trip() {
        assert_eq!(decode(&[0xE1, 0xE2]), "<PK><MN>");
        assert_eq!(encode("<PK><MN>", 3).expect("fits"), vec![0xE1, 0xE2, 0x50]);
    }

    #[test]
    fn decimal_point_is_distinct_from_period() {
        assert_eq!(decode(&[0xF2]), "<DOT>");
        assert_eq!(decode(&[0xE8]), ".");
        assert_eq!(encode("<DOT>", 2).expect("fits"), vec![0xF2, 0x50]);
        assert_eq!(encode(".", 2).expect("fits"), vec![0xE8, 0x50]);
    }

    #[test]
    fn too_long_is_rejected() {
        // 10 chars + terminator == 11 fits an 11-byte name field...
        assert!(encode("ABCDEFGHIJ", 11).is_ok());
        // ...but 11 chars do not.
        assert_eq!(
            encode("ABCDEFGHIJK", 11).expect_err("too long"),
            TextError::TooLong {
                needed: 12,
                field_len: 11
            }
        );
        assert_eq!(
            encode("", 0).expect_err("too long"),
            TextError::TooLong {
                needed: 1,
                field_len: 0
            }
        );
    }

    #[test]
    fn unencodable_chars_are_rejected() {
        assert_eq!(
            encode("~", 11).expect_err("unencodable"),
            TextError::Unencodable('~')
        );
        assert_eq!(
            encode("A@B", 11).expect_err("unencodable"),
            TextError::Unencodable('@')
        );
        assert_eq!(
            encode("\u{FFFD}", 11).expect_err("unencodable"),
            TextError::Unencodable('\u{FFFD}')
        );
    }

    #[test]
    fn empty_string_is_all_terminator() {
        assert_eq!(encode("", 4).expect("fits"), vec![0x50; 4]);
    }

    #[test]
    fn charmap_is_a_bijection_and_every_entry_round_trips() {
        for (i, &(byte, s)) in CHARMAP.iter().enumerate() {
            for &(other_byte, other_s) in &CHARMAP[i + 1..] {
                assert_ne!(byte, other_byte, "duplicate byte 0x{byte:02X}");
                assert_ne!(s, other_s, "duplicate string {s:?}");
            }
            assert_eq!(decode(&[byte]), s, "decode 0x{byte:02X}");
            let encoded = encode(s, 6).expect("single entry fits");
            assert_eq!(encoded[0], byte, "encode {s:?}");
            assert!(encoded[1..].iter().all(|&b| b == TERMINATOR));
        }
    }
}
