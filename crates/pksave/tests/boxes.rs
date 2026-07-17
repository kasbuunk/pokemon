//! Integration tests for PC boxes (`gen1::boxes`): addressing math,
//! current-box routing, deposit/withdraw and edit isolation.

use pksave::gen1::boxes::{BoxError, TransferError};
use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::offsets;
use pksave::gen1::pokemon::{BoxMonMut, MonMut, MonView, PartyMon, PartyMonMut};
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::gen1::stats::Dvs;

// Box-block internal layout (docs/FORMAT.md + pokered.sym: wBoxMonOT is
// 0x2AA from wNumInBox, wBoxMonNicks 0x386).
const SPECIES_LIST: usize = 0x001;
const MONS: usize = 0x016;
const OT_NAMES: usize = 0x2AA;
const NICKNAMES: usize = 0x386;

/// A coherent 33-byte box mon for the given National Dex number:
/// level byte and exp agree (`set_level_coherent`), then a battle-worn
/// current HP of 23.
fn make_box_mon(dex: usize, level: u8) -> [u8; offsets::BOX_MON_SIZE] {
    let mut bytes = [0u8; offsets::BOX_MON_SIZE];
    let mut mon = BoxMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_ot_id(0x1234);
    mon.set_dvs(Dvs {
        attack: 10,
        defense: 11,
        speed: 12,
        special: 13,
    });
    mon.set_stat_exps([100, 200, 300, 400, 500]);
    mon.set_level_coherent(level);
    mon.set_current_hp(23);
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
    assert_eq!(back.level(), 42, "level derived from experience");
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
fn withdraw_uses_experience_not_the_box_level_byte() {
    // Regression test for the level-drop bug: the game derives the
    // withdrawal level from experience (`CalcLevelFromExperience`); a
    // stale/edited box level byte must be ignored.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let boxed = make_box_mon(25, 50); // Pikachu, exp coherent for 50
    save.box_mut(0).add(&boxed, "RED", "PIKA").expect("room");
    save.box_mut(0).mon_mut(0).set_box_level(80); // stale byte

    save.withdraw(0, 0).expect("valid withdraw");
    let party = save.party();
    let got = party.mon(0);
    assert_eq!(
        got.level(),
        50,
        "withdrawal level derives from exp, not the box level byte"
    );
    assert_eq!(
        got.box_level(),
        80,
        "the stale box byte is copied verbatim, like the game"
    );
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
fn move_box_to_box_moves_bytes_verbatim() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.box_mut(2)
        .add(&make_box_mon(6, 55), "RED", "ZARD")
        .expect("room");
    save.box_mut(2)
        .add(&make_box_mon(9, 40), "RED", "BLASTY")
        .expect("room");
    let record = save.box_(2).mon(0).as_bytes().to_vec();

    save.move_box_to_box(2, 0, 5).expect("valid move");

    let dst = save.box_(5);
    assert_eq!(dst.len(), 1);
    assert_eq!(dst.mon(0).as_bytes(), &record[..], "record moves verbatim");
    assert_eq!(dst.nickname(0), "ZARD");
    assert_eq!(dst.ot_name(0), "RED");
    let src = save.box_(2);
    assert_eq!(src.len(), 1, "source repacked");
    assert_eq!(src.nickname(0), "BLASTY");
}

#[test]
fn move_box_to_box_routes_through_the_live_working_copy() {
    // Box 0 is the current box on a fresh save: moves out of and into it
    // must hit the working copy at 0x30C0, not its bank slot.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.box_mut(0)
        .add(&make_box_mon(25, 42), "ASH", "SPARKY")
        .expect("room");
    save.sync_current_box_to_bank();
    let before = save.as_bytes().to_vec();

    save.move_box_to_box(0, 0, 7).expect("valid move");
    let changed = changed_ranges(&before, save.as_bytes());
    assert_within(
        "move out of the live box",
        &changed,
        &[
            offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN,
            offsets::box_offset(7)..offsets::box_offset(7) + offsets::BOX_LEN,
        ],
    );
    assert_eq!(save.box_(0).len(), 0);
    assert_eq!(save.box_(7).len(), 1);
}

#[test]
fn move_box_to_box_errors() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.box_mut(2)
        .add(&make_box_mon(1, 5), "RED", "MON")
        .expect("room");
    let before = save.as_bytes().to_vec();

    assert_eq!(
        save.move_box_to_box(2, 0, 2).unwrap_err(),
        TransferError::BadIndex,
        "same-box move is refused (use swap to reorder)"
    );
    assert_eq!(
        save.move_box_to_box(2, 1, 5).unwrap_err(),
        TransferError::BadIndex
    );
    assert_eq!(
        save.move_box_to_box(12, 0, 5).unwrap_err(),
        TransferError::BadIndex
    );
    assert_eq!(
        save.move_box_to_box(2, 0, 12).unwrap_err(),
        TransferError::BadIndex
    );
    assert_eq!(save.as_bytes(), &before[..], "failed moves write nothing");

    for _ in 0..offsets::MONS_PER_BOX {
        save.box_mut(5)
            .add(&make_box_mon(1, 5), "RED", "MON")
            .expect("room");
    }
    let before = save.as_bytes().to_vec();
    assert_eq!(
        save.move_box_to_box(2, 0, 5).unwrap_err(),
        TransferError::TargetFull
    );
    assert_eq!(
        save.as_bytes(),
        &before[..],
        "full-target move writes nothing"
    );
}

#[test]
fn swap_party_box_swaps_positionally() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.party_mut()
        .add(&make_party_mon(25, 42), "ASH", "SPARKY")
        .expect("room");
    save.party_mut()
        .add(&make_party_mon(1, 10), "ASH", "BULBA")
        .expect("room");
    save.box_mut(3)
        .add(&make_box_mon(4, 20), "RED", "CHAR")
        .expect("room");
    save.box_mut(3)
        .add(&make_box_mon(151, 70), "RED", "MEW")
        .expect("room");

    // Swap party slot 0 (Pikachu 42) with box 3 slot 1 (Mew 70).
    save.swap_party_box(0, 3, 1).expect("valid swap");

    let party = save.party();
    assert_eq!(party.len(), 2, "capacity-neutral");
    let got = party.mon(0);
    assert_eq!(got.species(), DEX_TO_INDEX[151], "Mew took party slot 0");
    assert_eq!(got.level(), 70, "level derived from experience");
    assert_eq!(party.nickname(0), "MEW");
    assert_eq!(party.ot_name(0), "RED");
    assert_eq!(party.nickname(1), "BULBA", "other party slot untouched");

    let bx = save.box_(3);
    assert_eq!(bx.len(), 2, "capacity-neutral");
    let deposited = bx.mon(1);
    assert_eq!(
        deposited.species(),
        DEX_TO_INDEX[25],
        "Pikachu took box slot 1"
    );
    assert_eq!(deposited.box_level(), 42, "party level copied to the byte");
    assert_eq!(bx.nickname(1), "SPARKY");
    assert_eq!(bx.nickname(0), "CHAR", "other box slot untouched");
}

#[test]
fn swap_party_box_touches_only_party_and_that_bank_block() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.party_mut()
        .add(&make_party_mon(25, 42), "ASH", "SPARKY")
        .expect("room");
    save.box_mut(9)
        .add(&make_box_mon(4, 20), "RED", "CHAR")
        .expect("room");
    let before = save.as_bytes().to_vec();

    save.swap_party_box(0, 9, 0).expect("valid swap");
    let changed = changed_ranges(&before, save.as_bytes());
    assert_within(
        "swap_party_box",
        &changed,
        &[
            offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN,
            offsets::box_offset(9)..offsets::box_offset(9) + offsets::BOX_LEN,
        ],
    );
}

#[test]
fn swap_party_box_errors() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.party_mut()
        .add(&make_party_mon(25, 42), "ASH", "SPARKY")
        .expect("room");
    save.box_mut(3)
        .add(&make_box_mon(4, 20), "RED", "CHAR")
        .expect("room");
    let before = save.as_bytes().to_vec();

    assert_eq!(save.swap_party_box(1, 3, 0), Err(TransferError::BadIndex));
    assert_eq!(save.swap_party_box(0, 3, 1), Err(TransferError::BadIndex));
    assert_eq!(save.swap_party_box(0, 12, 0), Err(TransferError::BadIndex));
    assert_eq!(save.as_bytes(), &before[..], "failed swaps write nothing");
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

// ---- mutation hardening (issue #33): pin the layout arithmetic against
// the raw byte image, so offset math cannot silently self-cancel through
// symmetric read/write paths.

#[test]
fn box_edits_at_high_slots_land_at_the_documented_offsets() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1); // box 0 routes to its bank slot
    {
        let mut b = save.box_mut(0);
        for dex in [1, 4, 7] {
            b.add(&make_box_mon(dex, 12), "RED", "NICK").expect("room");
        }
        b.set_ot_name(2, "BLUE").expect("encodes");
        b.set_nickname(2, "SQUIRT").expect("encodes");
        b.set_species(1, 0x99);
    }
    let bytes = save.to_bytes();
    let base = offsets::box_offset(0);
    assert_eq!(bytes[base + SPECIES_LIST + 1], 0x99, "species list entry 1");
    assert_eq!(
        &bytes[base + OT_NAMES + 2 * offsets::NAME_LEN..][..offsets::NAME_LEN],
        &pksave::gen1::text::encode("BLUE", offsets::NAME_LEN).expect("encodes")[..],
        "slot 2 OT name bytes"
    );
    assert_eq!(
        &bytes[base + NICKNAMES + 2 * offsets::NAME_LEN..][..offsets::NAME_LEN],
        &pksave::gen1::text::encode("SQUIRT", offsets::NAME_LEN).expect("encodes")[..],
        "slot 2 nickname bytes"
    );
    assert_eq!(
        bytes[base + MONS + 2 * offsets::BOX_MON_SIZE],
        DEX_TO_INDEX[7],
        "slot 2 mon record species byte"
    );
}

#[test]
fn box_remove_middle_slot_shifts_every_array() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    {
        let mut b = save.box_mut(0);
        for (dex, nick) in [(1, "N1"), (4, "N2"), (7, "N3"), (25, "N4")] {
            b.add(&make_box_mon(dex, 9), &format!("O{nick}"), nick)
                .expect("room");
        }
        b.remove(1);
    }
    let view = save.box_(0);
    assert_eq!(view.len(), 3);
    assert_eq!(
        view.species_list(),
        &[DEX_TO_INDEX[1], DEX_TO_INDEX[7], DEX_TO_INDEX[25]]
    );
    assert_eq!(
        [view.nickname(0), view.nickname(1), view.nickname(2)],
        ["N1".to_owned(), "N3".to_owned(), "N4".to_owned()]
    );
    assert_eq!(view.ot_name(1), "ON3");
    assert_eq!(view.mon(1).species(), DEX_TO_INDEX[7]);
    // The vacated trailing slot is zero-filled in all three arrays.
    let bytes = save.to_bytes();
    let base = offsets::box_offset(0);
    assert!(
        bytes[base + MONS + 3 * offsets::BOX_MON_SIZE..][..offsets::BOX_MON_SIZE]
            .iter()
            .all(|&b| b == 0)
    );
    assert!(
        bytes[base + OT_NAMES + 3 * offsets::NAME_LEN..][..offsets::NAME_LEN]
            .iter()
            .all(|&b| b == 0)
    );
    assert!(
        bytes[base + NICKNAMES + 3 * offsets::NAME_LEN..][..offsets::NAME_LEN]
            .iter()
            .all(|&b| b == 0)
    );
}

#[test]
fn box_swap_moves_all_four_arrays_together() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    {
        let mut b = save.box_mut(0);
        for (dex, nick) in [(1, "A"), (4, "B"), (7, "C")] {
            b.add(&make_box_mon(dex, 9), &format!("O{nick}"), nick)
                .expect("room");
        }
        b.swap(0, 2);
    }
    let view = save.box_(0);
    assert_eq!(
        view.species_list(),
        &[DEX_TO_INDEX[7], DEX_TO_INDEX[4], DEX_TO_INDEX[1]]
    );
    assert_eq!(view.nickname(0), "C");
    assert_eq!(view.nickname(2), "A");
    assert_eq!(view.ot_name(0), "OC");
    assert_eq!(view.mon(0).species(), DEX_TO_INDEX[7]);
    assert_eq!(view.mon(2).species(), DEX_TO_INDEX[1]);
}

#[test]
fn withdraw_appends_at_the_documented_party_offsets() {
    // Party-block layout (docs/FORMAT.md): count 0x000, species list
    // 0x001, mons 0x008, OT names 0x110, nicknames 0x152.
    const P_MONS: usize = 0x008;
    const P_OT_NAMES: usize = 0x110;
    const P_NICKNAMES: usize = 0x152;

    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    for (dex, nick) in [(1, "P1"), (4, "P2")] {
        save.party_mut()
            .add(&make_party_mon(dex, 8), "ASH", nick)
            .expect("room");
    }
    save.box_mut(0)
        .add(&make_box_mon(7, 31), "OTIS", "BOXY")
        .expect("room");
    save.withdraw(0, 0).expect("party has room");

    let party = save.party();
    assert_eq!(party.len(), 3);
    assert_eq!(party.nickname(2), "BOXY");
    assert_eq!(party.ot_name(2), "OTIS");
    assert_eq!(party.mon(2).species(), DEX_TO_INDEX[7]);
    assert_eq!(party.mon(2).level(), 31, "level derives from exp");

    let bytes = save.to_bytes();
    let base = offsets::PARTY;
    assert_eq!(bytes[base], 3, "party count byte");
    assert_eq!(bytes[base + 1 + 2], DEX_TO_INDEX[7], "species list entry 2");
    assert_eq!(bytes[base + 1 + 3], 0xFF, "species list sentinel");
    assert_eq!(
        bytes[base + P_MONS + 2 * offsets::PARTY_MON_SIZE],
        DEX_TO_INDEX[7],
        "mon record 2 species byte"
    );
    assert_eq!(
        &bytes[base + P_OT_NAMES + 2 * offsets::NAME_LEN..][..offsets::NAME_LEN],
        &pksave::gen1::text::encode("OTIS", offsets::NAME_LEN).expect("encodes")[..],
        "OT name 2 bytes"
    );
    assert_eq!(
        &bytes[base + P_NICKNAMES + 2 * offsets::NAME_LEN..][..offsets::NAME_LEN],
        &pksave::gen1::text::encode("BOXY", offsets::NAME_LEN).expect("encodes")[..],
        "nickname 2 bytes"
    );
}

#[test]
fn swap_party_box_exchanges_records_names_and_nicknames() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    for (dex, nick) in [(1, "P1"), (4, "P2"), (7, "P3")] {
        save.party_mut()
            .add(&make_party_mon(dex, 8), "ASH", nick)
            .expect("room");
    }
    for (dex, nick) in [(25, "B1"), (39, "B2"), (52, "B3")] {
        save.box_mut(0)
            .add(&make_box_mon(dex, 21), "GARY", nick)
            .expect("room");
    }
    save.swap_party_box(2, 0, 2).expect("both occupied");

    let party = save.party();
    assert_eq!(party.mon(2).species(), DEX_TO_INDEX[52]);
    assert_eq!(party.nickname(2), "B3");
    assert_eq!(party.ot_name(2), "GARY");
    assert_eq!(party.mon(2).level(), 21, "withdrawn level derives from exp");
    let boxv = save.box_(0);
    assert_eq!(boxv.mon(2).species(), DEX_TO_INDEX[7]);
    assert_eq!(boxv.nickname(2), "P3");
    assert_eq!(boxv.ot_name(2), "ASH");
}

#[test]
fn box_is_empty_reports_both_polarities_via_view_and_mut() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    assert!(save.box_(0).is_empty());
    assert!(save.box_mut(0).is_empty());
    save.box_mut(0)
        .add(&make_box_mon(1, 5), "RED", "BULBA")
        .expect("room");
    assert!(!save.box_(0).is_empty());
    assert!(!save.box_mut(0).is_empty());
}

#[test]
fn box_swap_between_nonzero_slots_moves_the_species_list() {
    // swap(1, 2) rather than swap(0, _): at slot 0 the species-list
    // index arithmetic is a fixed point of +/- mutations.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    {
        let mut b = save.box_mut(0);
        for dex in [1, 4, 7] {
            b.add(&make_box_mon(dex, 9), "RED", "X").expect("room");
        }
        b.swap(1, 2);
    }
    assert_eq!(
        save.box_(0).species_list(),
        &[DEX_TO_INDEX[1], DEX_TO_INDEX[7], DEX_TO_INDEX[4]]
    );
}

#[test]
fn swap_party_box_updates_both_species_lists() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    for dex in [1, 4, 7] {
        save.party_mut()
            .add(&make_party_mon(dex, 8), "ASH", "P")
            .expect("room");
    }
    for dex in [25, 39, 52] {
        save.box_mut(0)
            .add(&make_box_mon(dex, 21), "GARY", "B")
            .expect("room");
    }
    save.swap_party_box(2, 0, 2).expect("both occupied");
    assert_eq!(
        save.party().species_list(),
        &[DEX_TO_INDEX[1], DEX_TO_INDEX[4], DEX_TO_INDEX[52]],
        "party species list entry 2 follows the swap"
    );
    assert_eq!(
        save.box_(0).species_list(),
        &[DEX_TO_INDEX[25], DEX_TO_INDEX[39], DEX_TO_INDEX[7]],
        "box species list entry 2 follows the swap"
    );
}

#[test]
fn withdraw_into_a_one_mon_party_repacks_the_species_list() {
    // At party index 2 the tail-fill start `list + i + 2` is a fixed
    // point of the +/* mutation (2+2 == 2*2); index 1 is not.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_current_box_number(1);
    save.party_mut()
        .add(&make_party_mon(1, 8), "ASH", "P1")
        .expect("room");
    save.box_mut(0)
        .add(&make_box_mon(7, 31), "OTIS", "BOXY")
        .expect("room");
    save.withdraw(0, 0).expect("party has room");

    let bytes = save.to_bytes();
    let list = offsets::PARTY + 1;
    assert_eq!(bytes[offsets::PARTY], 2, "party count byte");
    assert_eq!(bytes[list + 1], DEX_TO_INDEX[7], "species list entry 1");
    assert_eq!(bytes[list + 2], 0xFF, "species list sentinel");
    assert!(
        bytes[list + 3..list + 7].iter().all(|&b| b == 0),
        "species list tail is zero-filled"
    );
}
