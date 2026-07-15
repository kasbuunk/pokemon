//! Raw-mutation (`set_byte`/`set_bytes`) and checksum-override API.
//!
//! The contract under test (see `save.rs` docs):
//!
//! - a raw data write inside the SRAM image marks the file edited only
//!   when the byte actually changes;
//! - a raw write to one of the 15 stored checksum bytes *pins* that
//!   region, so `to_bytes()` keeps it verbatim while recomputing the
//!   rest;
//! - writes past the SRAM image touch only the tail and never trigger
//!   checksum repair;
//! - out-of-range writes are all-or-nothing errors.

use std::collections::HashSet;

use pksave::gen1::checksum::{self, Region};
use pksave::gen1::offsets;
use pksave::gen1::save::{changed_ranges, GameVariant, RawOffsetError, SaveFile};
use pksave::Severity;
use proptest::prelude::*;

fn blank() -> SaveFile {
    SaveFile::new_empty(GameVariant::RedBlue)
}

/// A data byte that is inside the SRAM image but outside every
/// checksummed region (bank 0 before the main block), so writing it
/// marks the file edited without changing any *computed* checksum.
const UNCHECKSUMMED_DATA: usize = 0x0100;

#[test]
fn set_byte_data_edit_recomputes_all_checksums() {
    let mut save = blank();
    save.set_byte(offsets::PLAYER_NAME, 0x81).unwrap(); // "B"
    assert!(save.is_edited());
    let out = save.to_bytes();
    assert_eq!(out[offsets::PLAYER_NAME], 0x81);
    assert!(checksum::verify(&out).is_empty());
}

#[test]
fn set_bytes_writes_a_run_and_recomputes_checksums() {
    let mut save = blank();
    save.set_bytes(offsets::PLAYER_NAME, &[0x80, 0x92, 0x87, 0x50]) // "ASH"
        .unwrap();
    assert!(save.is_edited());
    let out = save.to_bytes();
    assert_eq!(
        &out[offsets::PLAYER_NAME..offsets::PLAYER_NAME + 4],
        &[0x80, 0x92, 0x87, 0x50]
    );
    assert!(checksum::verify(&out).is_empty());
}

#[test]
fn noop_write_keeps_byte_identical_roundtrip() {
    let mut save = blank();
    let original = save.as_bytes().to_vec();
    let stored = original[offsets::PLAYER_NAME];
    save.set_byte(offsets::PLAYER_NAME, stored).unwrap();
    assert!(!save.is_edited());
    assert_eq!(save.to_bytes(), original);
}

#[test]
fn out_of_range_write_is_all_or_nothing() {
    let mut save = blank();
    let original = save.as_bytes().to_vec();
    assert_eq!(
        save.set_byte(0x8000, 0xAA),
        Err(RawOffsetError {
            offset: 0x8000,
            len: 0x8000
        })
    );
    // Straddling the end: the in-range byte at 0x7FFF must stay
    // untouched too.
    assert_eq!(
        save.set_bytes(0x7FFF, &[1, 2]),
        Err(RawOffsetError {
            offset: 0x8000,
            len: 0x8000
        })
    );
    assert_eq!(
        save.set_byte(usize::MAX, 0),
        Err(RawOffsetError {
            offset: usize::MAX,
            len: 0x8000
        })
    );
    assert!(!save.is_edited());
    assert_eq!(save.as_bytes(), &original[..]);
}

#[test]
fn tail_write_preserves_corrupt_checksums_verbatim() {
    // All-zero SRAM: every stored checksum (0x00) disagrees with the
    // computed one (0xFF); the main byte is extra-wrong on purpose.
    let mut input = vec![0u8; 0x8009];
    input[offsets::MAIN_CHECKSUM] = 0x12;
    let mut save = SaveFile::from_bytes(input.clone()).expect("length is valid");
    save.set_byte(0x8003, 0xAB).unwrap();
    assert!(!save.is_edited());
    let out = save.to_bytes();
    let mut expected = input.clone();
    expected[0x8003] = 0xAB;
    assert_eq!(out, expected, "only the tail byte may change");
    for region in Region::ALL {
        let at = region.checksum_offset();
        assert_eq!(out[at], input[at], "{region:?} stored byte not repaired");
    }
}

#[test]
fn pinning_with_the_stored_value_still_marks_edited() {
    let mut save = blank();
    let stored = save.as_bytes()[offsets::MAIN_CHECKSUM];
    save.set_byte(offsets::MAIN_CHECKSUM, stored).unwrap();
    assert!(save.is_edited(), "pinning is an explicit act");
    assert_eq!(save.checksum_override(Region::Main), Some(stored));
}

/// The full pin life cycle, for each of the 15 checksum bytes:
/// `set_byte` pins; a later data edit leaves the pinned byte verbatim in
/// `to_bytes()` while every other region verifies; diagnostics report
/// both the mismatch and the pin; `Clone` carries the pin;
/// `clear_checksum_override` restores recompute; `fix_checksums` unpins
/// and repairs.
#[test]
fn set_byte_on_each_checksum_offset_pins_that_region() {
    for region in Region::ALL {
        let at = region.checksum_offset();
        let mut save = blank();
        // new_empty checksums are valid, so flipping every bit of the
        // stored byte guarantees a mismatch.
        let pinned_value = save.as_bytes()[at] ^ 0xFF;
        save.set_byte(at, pinned_value).unwrap();
        assert!(save.is_edited(), "{region:?}");
        assert_eq!(
            save.checksum_override(region),
            Some(pinned_value),
            "{region:?}"
        );

        // A subsequent data edit elsewhere (outside every checksummed
        // region, so computed values stay valid) must not disturb the pin.
        save.set_byte(UNCHECKSUMMED_DATA, 0xAB).unwrap();

        let out = save.to_bytes();
        assert_eq!(out[UNCHECKSUMMED_DATA], 0xAB, "{region:?}");
        assert_eq!(out[at], pinned_value, "{region:?}: pinned byte survives");
        let mismatched: Vec<Region> = checksum::verify(&out).iter().map(|m| m.region).collect();
        assert_eq!(
            mismatched,
            vec![region],
            "{region:?}: all unpinned regions must verify"
        );

        // Diagnostics: the mismatch warning fires on the pinned byte,
        // and exactly one I-CHECKSUM-PINNED info points at it.
        let diags = save.diagnostics();
        assert!(
            diags
                .iter()
                .any(|d| d.code.starts_with("W-CHECKSUM-") && d.span == Some(at..at + 1)),
            "{region:?}: mismatching pinned value must still warn"
        );
        let pinned_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.code == "I-CHECKSUM-PINNED")
            .collect();
        assert_eq!(pinned_diags.len(), 1, "{region:?}");
        assert_eq!(pinned_diags[0].severity, Severity::Info, "{region:?}");
        assert_eq!(pinned_diags[0].span, Some(at..at + 1), "{region:?}");

        // Clone preserves the pin (and its effect on serialization).
        let cloned = save.clone();
        assert_eq!(
            cloned.checksum_override(region),
            Some(pinned_value),
            "{region:?}"
        );
        assert_eq!(cloned.to_bytes()[at], pinned_value, "{region:?}");

        // clear_checksum_override alone restores recompute.
        let mut cleared = save.clone();
        cleared.clear_checksum_override(region);
        assert_eq!(cleared.checksum_override(region), None, "{region:?}");
        assert!(
            checksum::verify(&cleared.to_bytes()).is_empty(),
            "{region:?}: unpinned region is recomputed again"
        );
        assert!(
            cleared
                .diagnostics()
                .iter()
                .all(|d| d.code != "I-CHECKSUM-PINNED"),
            "{region:?}"
        );

        // fix_checksums unpins and repairs the stored bytes in place.
        save.fix_checksums();
        assert_eq!(save.checksum_override(region), None, "{region:?}");
        assert!(checksum::verify(save.as_bytes()).is_empty(), "{region:?}");
        assert!(checksum::verify(&save.to_bytes()).is_empty(), "{region:?}");
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    /// Raw writes touch nothing but the written offsets plus (possibly)
    /// the 15 stored checksum bytes; no writes at all means a
    /// byte-identical round-trip.
    #[test]
    fn p_raw_writes_change_only_written_and_checksum_bytes(
        bytes in prop::collection::vec(any::<u8>(), 0x8000..=0x8100),
        writes in prop::collection::vec((0usize..0x8200, any::<u8>()), 0..=16),
    ) {
        let mut save = SaveFile::from_bytes(bytes.clone()).expect("length is valid");
        for &(offset, value) in &writes {
            // Out-of-range writes error and must leave the buffer alone.
            let _ = save.set_byte(offset, value);
        }
        let out = save.to_bytes();
        prop_assert_eq!(out.len(), bytes.len());
        if writes.is_empty() {
            prop_assert_eq!(&out, &bytes);
        }
        let allowed: HashSet<usize> = writes
            .iter()
            .map(|&(offset, _)| offset)
            .chain(Region::ALL.iter().map(|r| r.checksum_offset()))
            .collect();
        for range in changed_ranges(&bytes, &out) {
            for i in range {
                prop_assert!(
                    allowed.contains(&i),
                    "byte 0x{:04X} changed without being written",
                    i
                );
            }
        }
    }

    /// A pinned region survives unrelated structured edits: the
    /// serialized pinned byte is exactly the override value while every
    /// other region verifies.
    #[test]
    fn p_pinned_region_survives_structured_edits(
        region_index in 0usize..Region::COUNT,
        value in any::<u8>(),
        money in 0u32..=999_999,
    ) {
        let region = Region::ALL[region_index];
        let mut save = blank();
        save.set_checksum_override(region, value);
        prop_assert_eq!(save.checksum_override(region), Some(value));
        save.set_money(money).expect("within MAX_MONEY");
        save.set_player_name("ASH").expect("valid name");
        let out = save.to_bytes();
        prop_assert_eq!(out[region.checksum_offset()], value);
        for other in Region::ALL {
            if other == region {
                continue;
            }
            prop_assert_eq!(
                out[other.checksum_offset()],
                checksum::gen1_checksum(&out[other.data_range()]),
                "region {:?} must verify",
                other
            );
        }
    }
}
