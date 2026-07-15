//! P3/P4: property tests for the text and BCD codecs.

use pksave::gen1::{bcd, text};
use proptest::prelude::*;

/// Strings built from the encodable alphabet, at most 10 glyphs (the
/// widest Gen 1 name field holds 10 characters + terminator).
fn encodable_string() -> impl Strategy<Value = String> {
    prop::collection::vec(prop::sample::select(text::CHARMAP), 0..=10)
        .prop_map(|entries| entries.iter().map(|(_, s)| *s).collect())
}

/// Canonical text buffers: sequences of printable bytes that the encoder
/// itself would produce. This excludes `0xE0` (`'`) directly followed by
/// a letter that forms an apostrophe ligature, because greedy encoding
/// canonicalizes that pair into the single ligature byte.
fn canonical_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop::sample::select(text::CHARMAP).prop_map(|(b, _)| b),
        0..=10,
    )
    .prop_filter(
        "no apostrophe+letter pair that a ligature canonicalizes",
        |bytes| {
            !bytes.windows(2).any(|w| {
                // d, l, s, t, v, r, m
                w[0] == 0xE0 && matches!(w[1], 0xA3 | 0xAB | 0xB2 | 0xB3 | 0xB5 | 0xB1 | 0xAC)
            })
        },
    )
}

proptest! {
    #[test]
    fn p3_decode_of_encode_is_identity(s in encodable_string()) {
        // Up to 10 glyphs -> at most 10 bytes -> always fits 11 + terminator.
        let field_len = 21; // roomy: ligature-splitting inputs still fit
        let encoded = text::encode(&s, field_len).expect("alphabet strings fit");
        prop_assert_eq!(encoded.len(), field_len);
        prop_assert_eq!(text::decode(&encoded), s);
    }

    #[test]
    fn p3_encode_of_decode_restores_canonical_buffers(buf in canonical_bytes()) {
        let s = text::decode(&buf);
        let encoded = text::encode(&s, 11).expect("canonical buffers fit");
        prop_assert_eq!(&encoded[..buf.len()], &buf[..]);
        prop_assert!(encoded[buf.len()..].iter().all(|&b| b == text::TERMINATOR));
    }

    #[test]
    fn p4_bcd_roundtrip_money_range(value in 0u32..=999_999) {
        let bytes = bcd::encode(value, 3).expect("6 digits fit 3 bytes");
        prop_assert_eq!(bytes.len(), 3);
        prop_assert_eq!(bcd::decode(&bytes).expect("encoder output is valid BCD"), value);
    }

    #[test]
    fn p4_bcd_roundtrip_general(
        (out_len, value) in (1usize..=4).prop_flat_map(|len| {
            (Just(len), 0u32..10u32.pow(2 * len as u32))
        })
    ) {
        let bytes = bcd::encode(value, out_len).expect("value fits by construction");
        prop_assert_eq!(bytes.len(), out_len);
        prop_assert_eq!(bcd::decode(&bytes).expect("encoder output is valid BCD"), value);
        prop_assert_eq!(bcd::decode_lossy(&bytes), value);
    }
}
