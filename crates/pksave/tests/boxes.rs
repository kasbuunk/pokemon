//! Integration tests for PC boxes (`gen1::boxes`): addressing math,
//! current-box routing, deposit/withdraw and edit isolation.

use pksave::gen1::boxes::{BoxError, TransferError};
use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::offsets;
use pksave::gen1::pokemon::{BoxMonMut, PartyMonMut};
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::gen1::stats::Dvs;

// Box-block internal layout (docs/FORMAT.md + pokered.sym: wBoxMonOT is
// 0x2AA from wNumInBox, wBoxMonNicks 0x386).
const SPECIES_LIST: usize = 0x001;
const MONS: usize = 0x016;
const OT_NAMES: usize = 0x2AA;
const NICKNAMES: usize = 0x386;

/// A coherent 33-byte box mon for the given National Dex number.
fn make_box_mon(dex: usize, level: u8) -> [u8; offsets::BOX_MON_SIZE] {
    let mut bytes = [0u8; offsets::BOX_MON_SIZE];
    let mut mon = BoxMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_box_level(level);
    mon.set_ot_id(0x1234);
    mon.set_current_hp(23);
    mon.set_dvs(Dvs {
        attack: 10,
        defense: 11,
        speed: 12,
        special: 13,
    });
    mon.set_stat_exps([100, 200, 300, 400, 500]);
    bytes
}

/// A coherent 44-byte party mon for the given National Dex number.
fn make_party_mon(dex: usize, level: u8) -> [u8; offsets::PARTY_MON_SIZE] {
    let mut bytes = [0u8; offsets::PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_ot_id(0x1234);
    mon.set_dvs(Dvs {
        attack: 5,
        defense: 6,
        speed: 7,
        special: 8,
    });
    mon.set_stat_exps([11, 22, 33, 44, 55]);
    mon.set_level_coherent(level);
    bytes
}

/// Every changed range must lie inside one of the allowed ranges.
fn assert_within(
    what: &str,
    changed: &[core::ops::Range<usize>],
    allowed: &[core::ops::Range<usize>],
) {
    for r in changed {
        assert!(
            allowed.iter().any(|a| a.start <= r.start && r.end <= a.end),
            "{what}: changed bytes 0x{:04X}..0x{:04X} outside allowed {allowed:X?}",
            r.start,
            r.end
        );
    }
}

#[test]
fn addressing_all_twelve_bank_boxes() {
    for n in 0..offsets::NUM_BOXES {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        // Make sure box n is NOT the current box so it routes to its bank.
        save.set_current_box_number(((n + 1) % offsets::NUM_BOXES) as u8);
        assert!(!save.box_is_live(n));

        let mon = make_box_mon(151, 30); // Mew
        save.box_mut(n).add(&mon, "RED", "MEWTWO").expect("empty");

        let base = offsets::box_offset(n);
        let b = save.as_bytes();
        assert_eq!(b[base], 1, "box {n} count byte");
        assert_eq!(b[base + SPECIES_LIST], DEX_TO_INDEX[151], "box {n} species");
        assert_eq!(b[base + SPECIES_LIST + 1], 0xFF, "box {n} sentinel");
        assert_eq!(&b[base + MONS..base + MONS + 33], &mon, "box {n} record");
        assert_eq!(
            &b[base + OT_NAMES..base + OT_NAMES + 4],
            &[0x91, 0x84, 0x83, 0x50], // "RED" + terminator
            "box {n} OT"
        );
        assert_eq!(
            &b[base + NICKNAMES..base + NICKNAMES + 3],
            &[0x8C, 0x84, 0x96], // "MEW"...
            "box {n} nickname"
        );

        // Expected bank: 2 for boxes 0-5, 3 for boxes 6-11.
        let bank = base / offsets::BANK_SIZE;
        assert_eq!(bank, if n < 6 { 2 } else { 3 }, "box {n} bank");
    }
}

#[test]
fn current_box_routes_to_working_copy_not_bank() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(4);
    assert!(save.box_is_live(4));
    assert!(!save.box_is_live(3));

    let bank_before: Vec<u8> =
        save.as_bytes()[offsets::box_offset(4)..offsets::box_offset(4) + offsets::BOX_LEN].to_vec();
    let mon = make_box_mon(6, 55); // Charizard
    save.box_mut(4).add(&mon, "BLUE", "ZARD").expect("empty");

    let b = save.as_bytes();
    // The edit landed in the working copy...
    assert_eq!(b[offsets::CURRENT_BOX], 1);
    assert_eq!(b[offsets::CURRENT_BOX + SPECIES_LIST], DEX_TO_INDEX[6]);
    // ...and the bank slot did not move.
    assert_eq!(
        &b[offsets::box_offset(4)..offsets::box_offset(4) + offsets::BOX_LEN],
        &bank_before[..],
        "bank copy must stay untouched"
    );
    // Reading box 4 sees the working copy.
    assert_eq!(save.box_(4).len(), 1);
    assert_eq!(save.box_(4).mon(0).species(), DEX_TO_INDEX[6]);
}

#[test]
fn set_current_box_number_preserves_bit7() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert!(save.boxes_initialized());
    save.set_current_box_number(11);
    assert_eq!(save.as_bytes()[offsets::CURRENT_BOX_NUM], 0x80 | 11);
    assert_eq!(save.current_box_number(), 11);
    assert!(save.boxes_initialized());

    // With bit 7 clear it must stay clear.
    let mut save = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("length is valid");
    save.set_current_box_number(5);
    assert_eq!(save.as_bytes()[offsets::CURRENT_BOX_NUM], 5);
    assert!(!save.boxes_initialized());
}

#[test]
#[should_panic(expected = "out of range")]
fn set_current_box_number_rejects_12() {
    SaveFile::new_empty(GameVariant::RedBlue).set_current_box_number(12);
}

#[test]
fn box_add_remove_swap_clear_maintain_invariants() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mut bx = save.box_mut(2);
    bx.add(&make_box_mon(1, 5), "A", "BULBA").expect("room");
    bx.add(&make_box_mon(4, 6), "B", "CHAR").expect("room");
    bx.add(&make_box_mon(7, 7), "C", "SQUIRT").expect("room");
    assert_eq!(bx.len(), 3);
    assert_eq!(
        bx.as_view().species_list(),
        &[DEX_TO_INDEX[1], DEX_TO_INDEX[4], DEX_TO_INDEX[7]]
    );

    bx.swap(0, 2);
    assert_eq!(bx.as_view().mon(0).species(), DEX_TO_INDEX[7]);
    assert_eq!(bx.nickname(0), "SQUIRT");
    assert_eq!(bx.ot_name(0), "C");
    assert_eq!(bx.as_view().mon(2).species(), DEX_TO_INDEX[1]);

    bx.remove(1);
    assert_eq!(bx.len(), 2);
    assert_eq!(
        bx.as_view().species_list(),
        &[DEX_TO_INDEX[7], DEX_TO_INDEX[1]]
    );
    assert_eq!(bx.nickname(1), "BULBA");

    bx.set_species(0, DEX_TO_INDEX[150]);
    assert_eq!(bx.as_view().species_list()[0], DEX_TO_INDEX[150]);
    assert_eq!(bx.as_view().mon(0).species(), DEX_TO_INDEX[150]);

    bx.set_nickname(0, "MEWTWO").expect("fits");
    bx.set_ot_name(0, "GIOVANNI").expect("fits");
    assert_eq!(bx.nickname(0), "MEWTWO");
    assert_eq!(bx.ot_name(0), "GIOVANNI");

    bx.clear();
    assert_eq!(bx.len(), 0);
    assert!(bx.is_empty());
    let b = save.as_bytes();
    let base = offsets::box_offset(2);
    assert_eq!(b[base], 0);
    assert_eq!(b[base + 1], 0xFF);
    assert!(b[base + 2..base + offsets::BOX_LEN].iter().all(|&x| x == 0));
}

#[test]
fn box_add_rejects_full_and_bad_text() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mut bx = save.box_mut(7);
    for i in 0..offsets::MONS_PER_BOX {
        bx.add(&make_box_mon(1 + (i % 151), 9), "RED", "MON")
            .expect("room");
    }
    assert_eq!(bx.len(), 20);
    assert_eq!(
        bx.add(&make_box_mon(1, 9), "RED", "MON").unwrap_err(),
        BoxError::Full
    );
    let before = save.as_bytes().to_vec();
    let mut bx = save.box_mut(8);
    assert!(matches!(
        bx.add(&make_box_mon(1, 9), "RED", "~~~"),
        Err(BoxError::Text(_))
    ));
    assert_eq!(save.as_bytes(), &before[..], "failed add writes nothing");
}

#[test]
fn deposit_and_withdraw_round_trip_preserves_identity_and_recalculates_stats() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mon = make_party_mon(25, 42); // Pikachu
    save.party_mut()
        .add(&mon, "ASH", "SPARKY")
        .expect("party has room");
    let before = save.party().mon(0);
    let dvs = before.dvs();
    let stat_exps = before.stat_exps();
    let expected_stats = [
        before.max_hp(),
        before.attack(),
        before.defense(),
        before.speed(),
        before.special(),
    ];

    save.deposit(0, 3).expect("valid deposit");
    assert_eq!(save.party().len(), 0);
    let bx = save.box_(3);
    assert_eq!(bx.len(), 1);
    let boxed = bx.mon(0);
    assert_eq!(boxed.species(), DEX_TO_INDEX[25]);
    // The authoritative party level was copied over the box level byte.
    assert_eq!(boxed.box_level(), 42);
    assert_eq!(boxed.dvs(), dvs);
    assert_eq!(boxed.stat_exps(), stat_exps);
    assert_eq!(bx.nickname(0), "SPARKY");
    assert_eq!(bx.ot_name(0), "ASH");

    save.withdraw(3, 0).expect("valid withdraw");
    assert_eq!(save.box_(3).len(), 0);
    let party = save.party();
    assert_eq!(party.len(), 1);
    let back = party.mon(0);
    assert_eq!(back.species(), DEX_TO_INDEX[25]);
    assert_eq!(back.level(), 42, "level comes from the box level byte");
    assert_eq!(back.dvs(), dvs, "DVs preserved");
    assert_eq!(back.stat_exps(), stat_exps, "stat exp preserved");
    assert_eq!(party.nickname(0), "SPARKY");
    assert_eq!(party.ot_name(0), "ASH");
    assert_eq!(
        [
            back.max_hp(),
            back.attack(),
            back.defense(),
            back.speed(),
            back.special()
        ],
        expected_stats,
        "stats recalculated from base + DVs + stat exp at the same level"
    );
}

#[test]
fn withdraw_recalculates_stats_from_scratch() {
    // A raw box mon has no stat fields at all; the withdrawal must
    // compute them. Cross-check against a fresh set_level_coherent mon.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let boxed = make_box_mon(151, 70); // Mew, level 70, custom DVs/stat exp
    save.box_mut(0).add(&boxed, "RED", "MEW").expect("room");
    save.withdraw(0, 0).expect("valid withdraw");

    let mut reference = [0u8; offsets::PARTY_MON_SIZE];
    reference[..offsets::BOX_MON_SIZE].copy_from_slice(&boxed);
    let mut refmon = PartyMonMut::new(&mut reference);
    refmon.set_level(70);
    refmon.recalculate_stats();
    let party = save.party();
    let got = party.mon(0);
    assert_eq!(got.level(), 70);
    assert_eq!(got.max_hp(), refmon.max_hp());
    assert_eq!(got.attack(), refmon.attack());
    assert_eq!(got.defense(), refmon.defense());
    assert_eq!(got.speed(), refmon.speed());
    assert_eq!(got.special(), refmon.special());
    // Current HP is carried over verbatim from the box record.
    assert_eq!(got.current_hp(), 23);
}

#[test]
fn deposit_errors() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert_eq!(save.deposit(0, 3).unwrap_err(), TransferError::BadIndex);

    save.party_mut()
        .add(&make_party_mon(1, 5), "RED", "MON")
        .expect("room");
    assert_eq!(save.deposit(1, 3).unwrap_err(), TransferError::BadIndex);
    assert_eq!(save.deposit(0, 12).unwrap_err(), TransferError::BadIndex);

    for _ in 0..offsets::MONS_PER_BOX {
        save.box_mut(3)
            .add(&make_box_mon(1, 5), "RED", "MON")
            .expect("room");
    }
    let before = save.as_bytes().to_vec();
    assert_eq!(save.deposit(0, 3).unwrap_err(), TransferError::TargetFull);
    assert_eq!(
        save.as_bytes(),
        &before[..],
        "failed deposit writes nothing"
    );
}

#[test]
fn withdraw_errors() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert_eq!(save.withdraw(12, 0).unwrap_err(), TransferError::BadIndex);
    assert_eq!(save.withdraw(3, 0).unwrap_err(), TransferError::BadIndex);

    save.box_mut(3)
        .add(&make_box_mon(4, 20), "RED", "CHAR")
        .expect("room");
    for i in 0..offsets::PARTY_CAPACITY {
        save.party_mut()
            .add(&make_party_mon(1 + i, 10), "RED", "MON")
            .expect("room");
    }
    let before = save.as_bytes().to_vec();
    assert_eq!(save.withdraw(3, 0).unwrap_err(), TransferError::TargetFull);
    assert_eq!(
        save.as_bytes(),
        &before[..],
        "failed withdraw writes nothing"
    );
}

#[test]
fn deposit_to_non_current_box_touches_only_party_and_that_bank_block() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.party_mut()
        .add(&make_party_mon(25, 42), "ASH", "SPARKY")
        .expect("room");
    let before = save.as_bytes().to_vec();

    save.deposit(0, 9).expect("valid deposit"); // box 10, bank 3
    let changed = changed_ranges(&before, save.as_bytes());
    assert_within(
        "deposit",
        &changed,
        &[
            offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN,
            offsets::box_offset(9)..offsets::box_offset(9) + offsets::BOX_LEN,
        ],
    );
}

#[test]
fn withdraw_from_current_box_touches_only_party_and_working_copy() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(2);
    save.box_mut(2)
        .add(&make_box_mon(151, 30), "RED", "MEW")
        .expect("room");
    let before = save.as_bytes().to_vec();

    save.withdraw(2, 0).expect("valid withdraw");
    let changed = changed_ranges(&before, save.as_bytes());
    assert_within(
        "withdraw",
        &changed,
        &[
            offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN,
            offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN,
        ],
    );
    // The bank slot of box 2 must not have moved.
    let base = offsets::box_offset(2);
    assert_eq!(
        &save.as_bytes()[base..base + offsets::BOX_LEN],
        &before[base..base + offsets::BOX_LEN]
    );
}

#[test]
fn sync_current_box_to_bank_copies_the_working_copy() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(6); // bank 3
    save.box_mut(6)
        .add(&make_box_mon(9, 36), "RED", "BLASTY")
        .expect("room");

    let bank = offsets::box_offset(6);
    assert_ne!(
        &save.as_bytes()[bank..bank + offsets::BOX_LEN],
        &save.as_bytes()[offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN],
        "working copy diverged from the bank slot"
    );

    let before = save.as_bytes().to_vec();
    save.sync_current_box_to_bank();
    assert_eq!(
        &save.as_bytes()[bank..bank + offsets::BOX_LEN],
        &save.as_bytes()[offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN]
    );
    let changed = changed_ranges(&before, save.as_bytes());
    let allowed = bank..bank + offsets::BOX_LEN;
    assert_within(
        "sync_current_box_to_bank",
        &changed,
        std::slice::from_ref(&allowed),
    );
}

#[test]
fn sync_current_box_to_bank_is_a_no_op_when_box_number_is_corrupt() {
    let mut bytes = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
    // Bit 7 (boxes initialized) + box number 12, one past the last box.
    bytes[offsets::CURRENT_BOX_NUM] = 0x8C;
    // Make the working copy distinctive so an unguarded copy_within
    // could not silently write identical bytes.
    bytes[offsets::CURRENT_BOX + 0x20] = 0xAB;
    let mut save = SaveFile::from_bytes(bytes.clone()).expect("length is valid");

    save.sync_current_box_to_bank();
    assert!(!save.is_edited(), "the no-op guard must not mark_edited");
    assert_eq!(
        save.to_bytes(),
        bytes,
        "corrupt current-box number: sync must be a data no-op"
    );
}

#[test]
fn add_raw_to_a_full_box_fails_without_touching_bytes() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mut bx = save.box_mut(4);
    for i in 0..offsets::MONS_PER_BOX {
        bx.add(&make_box_mon(1 + (i % 151), 9), "RED", "MON")
            .expect("room");
    }
    assert_eq!(bx.len(), offsets::MONS_PER_BOX);

    let raw_before = save.as_bytes().to_vec();
    let serialized_before = save.to_bytes();
    let mut bx = save.box_mut(4);
    assert_eq!(
        bx.add_raw(
            &make_box_mon(151, 30),
            &[0x50; offsets::NAME_LEN],
            &[0x50; offsets::NAME_LEN],
        ),
        Err(BoxError::Full)
    );
    assert_eq!(
        save.as_bytes(),
        &raw_before[..],
        "failed add_raw writes nothing"
    );
    assert_eq!(
        save.to_bytes(),
        serialized_before,
        "failed add_raw must serialize byte-identically"
    );
}

#[test]
fn box_swap_of_a_slot_with_itself_is_byte_identical() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mut bx = save.box_mut(1);
    bx.add(&make_box_mon(1, 5), "A", "BULBA").expect("room");
    bx.add(&make_box_mon(4, 6), "B", "CHAR").expect("room");

    let before = save.as_bytes().to_vec();
    save.box_mut(1).swap(1, 1);
    assert_eq!(
        save.as_bytes(),
        &before[..],
        "swap(i, i) must be a byte-identical no-op"
    );
}

#[test]
fn box_len_clamps_a_corrupt_count_byte() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.buf_edit_for_tests(offsets::box_offset(1), 200);
    assert_eq!(save.box_(1).len(), offsets::MONS_PER_BOX);
}

// `SaveFile` has no public raw-byte poke; go through from_bytes instead.
trait TestPoke {
    fn buf_edit_for_tests(&mut self, at: usize, value: u8);
}

impl TestPoke for SaveFile {
    fn buf_edit_for_tests(&mut self, at: usize, value: u8) {
        let mut bytes = self.as_bytes().to_vec();
        bytes[at] = value;
        *self = SaveFile::from_bytes(bytes).expect("length is valid");
        self.mark_edited();
    }
}
