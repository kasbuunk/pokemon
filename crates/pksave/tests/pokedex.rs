//! M6: Pokédex owned/seen bitfields.

use pksave::gen1::offsets;
use pksave::gen1::save::{GameVariant, SaveFile};

fn blank() -> SaveFile {
    SaveFile::new_empty(GameVariant::RedBlue)
}

#[test]
fn dex_1_maps_to_bit_0_of_the_first_byte() {
    let mut save = blank();
    assert!(!save.dex_owned(1));
    save.set_dex_owned(1, true);
    assert!(save.dex_owned(1));
    assert_eq!(save.as_bytes()[offsets::POKEDEX_OWNED], 0b0000_0001);
    assert!(!save.dex_seen(1));
    save.set_dex_seen(1, true);
    assert_eq!(save.as_bytes()[offsets::POKEDEX_SEEN], 0b0000_0001);
}

#[test]
fn dex_151_maps_to_bit_6_of_the_last_byte() {
    let mut save = blank();
    save.set_dex_owned(151, true); // Mew = bit 150 = byte 18, bit 6
    assert!(save.dex_owned(151));
    assert_eq!(
        save.as_bytes()[offsets::POKEDEX_OWNED + offsets::POKEDEX_LEN - 1],
        0b0100_0000
    );
    save.set_dex_seen(151, true);
    assert_eq!(
        save.as_bytes()[offsets::POKEDEX_SEEN + offsets::POKEDEX_LEN - 1],
        0b0100_0000
    );
    save.set_dex_owned(151, false);
    assert!(!save.dex_owned(151));
    assert_eq!(
        save.as_bytes()[offsets::POKEDEX_OWNED + offsets::POKEDEX_LEN - 1],
        0
    );
}

#[test]
fn dex_9_straddles_the_byte_boundary_correctly() {
    let mut save = blank();
    save.set_dex_owned(8, true); // bit 7 of byte 0
    save.set_dex_owned(9, true); // bit 0 of byte 1
    assert_eq!(save.as_bytes()[offsets::POKEDEX_OWNED], 0b1000_0000);
    assert_eq!(save.as_bytes()[offsets::POKEDEX_OWNED + 1], 0b0000_0001);
}

#[test]
fn owned_and_seen_are_independent() {
    let mut save = blank();
    save.set_dex_owned(25, true);
    assert!(save.dex_owned(25));
    assert!(!save.dex_seen(25));
    save.set_dex_seen(150, true);
    assert!(save.dex_seen(150));
    assert!(!save.dex_owned(150));
}

#[test]
fn counts_track_set_bits() {
    let mut save = blank();
    assert_eq!(save.owned_count(), 0);
    assert_eq!(save.seen_count(), 0);
    for dex in [1u8, 4, 7, 25, 151] {
        save.set_dex_owned(dex, true);
        save.set_dex_seen(dex, true);
    }
    save.set_dex_seen(133, true);
    assert_eq!(save.owned_count(), 5);
    assert_eq!(save.seen_count(), 6);
    save.set_dex_owned(4, false);
    assert_eq!(save.owned_count(), 4);
}

#[test]
fn invalid_dex_numbers_read_false_and_ignore_writes() {
    let mut save = blank();
    let before = save.as_bytes().to_vec();
    assert!(!save.dex_owned(0));
    assert!(!save.dex_seen(0));
    assert!(!save.dex_owned(152));
    assert!(!save.dex_seen(255));
    save.set_dex_owned(0, true);
    save.set_dex_seen(0, true);
    save.set_dex_owned(152, true);
    save.set_dex_seen(255, true);
    assert_eq!(save.as_bytes(), &before[..]);
    assert_eq!(save.owned_count(), 0);
}

#[test]
fn complete_dex_sets_all_151_owned_and_seen() {
    let mut save = blank();
    save.complete_dex();
    assert_eq!(save.owned_count(), 151);
    assert_eq!(save.seen_count(), 151);
    for dex in 1..=151u8 {
        assert!(save.dex_owned(dex), "dex {dex} owned");
        assert!(save.dex_seen(dex), "dex {dex} seen");
    }
    // Bit 151 (past Mew) stays clear: last byte is 0x7F, not 0xFF.
    let b = save.as_bytes();
    assert_eq!(b[offsets::POKEDEX_OWNED + offsets::POKEDEX_LEN - 1], 0x7F);
    assert_eq!(b[offsets::POKEDEX_SEEN + offsets::POKEDEX_LEN - 1], 0x7F);
    assert!(save.is_edited());
}
