//! Integration tests for the Hall of Fame (`gen1::hof`).

use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::hof::{HOF_TEAM_LEN, HOF_TERMINATOR};
use pksave::gen1::offsets;
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::gen1::text::TextError;

#[test]
fn addressing_teams_and_records() {
    // Team t, slot s lives at HALL_OF_FAME + t*96 + s*16 in bank 0.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    for (t, slot) in [(0usize, 0usize), (0, 5), (7, 3), (49, 5)] {
        save.hof_team_mut(t)
            .set_mon(slot, DEX_TO_INDEX[151], 70, "MEW")
            .expect("fits");
        let at = offsets::HALL_OF_FAME + t * offsets::HOF_TEAM_SIZE + slot * offsets::HOF_MON_SIZE;
        let b = save.as_bytes();
        assert_eq!(b[at], DEX_TO_INDEX[151], "team {t} slot {slot} species");
        assert_eq!(b[at + 1], 70, "team {t} slot {slot} level");
        assert_eq!(
            &b[at + 2..at + 2 + 4],
            &[0x8C, 0x84, 0x96, 0x50], // "MEW" + terminator
            "team {t} slot {slot} nickname"
        );
        assert_eq!(&b[at + 13..at + 16], &[0, 0, 0], "padding zeroed");
        assert!(at + 16 <= offsets::BANK_SIZE, "HoF stays inside bank 0");
    }
    // The whole table fits in bank 0.
    const _: () = assert!(
        offsets::HALL_OF_FAME + offsets::HOF_TEAM_CAPACITY * offsets::HOF_TEAM_SIZE
            <= offsets::BANK_SIZE
    );
}

#[test]
fn record_reads_back_through_the_view() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mut team = save.hof_team_mut(3);
    team.set_mon(0, DEX_TO_INDEX[25], 81, "SPARKY").expect("ok");
    team.set_mon(1, DEX_TO_INDEX[6], 78, "ZARD").expect("ok");
    assert_eq!(team.len(), 2);

    let team = save.hof_team(3);
    assert_eq!(team.len(), 2);
    assert!(!team.is_empty());
    let first = team.mon(0).expect("occupied");
    assert_eq!(first.species(), DEX_TO_INDEX[25]);
    assert_eq!(first.level(), 81);
    assert_eq!(first.nickname(), "SPARKY");
    assert_eq!(team.mon(1).expect("occupied").nickname(), "ZARD");
    assert!(team.mon(2).is_none(), "slot 2 empty");
}

#[test]
fn empty_slot_convention_matches_the_game() {
    // AnimateHallOfFame zero-fills the team buffer, records the party,
    // then writes $FF into the next record's species byte; the League
    // PC reader stops at $FF. Both 0x00 and 0xFF species read as empty.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let team = save.hof_team(0);
    assert_eq!(team.len(), 0, "all-zero team is empty");
    assert!(team.mon(0).is_none());

    let mut team = save.hof_team_mut(0);
    team.set_mon(0, DEX_TO_INDEX[151], 70, "MEW").expect("ok");
    team.set_mon(1, DEX_TO_INDEX[150], 70, "MEWTWO")
        .expect("ok");
    team.clear_slot(1); // the game's terminator form
    let base = offsets::HALL_OF_FAME + offsets::HOF_MON_SIZE;
    let b = save.as_bytes();
    assert_eq!(b[base], HOF_TERMINATOR, "species byte is $FF");
    assert!(
        b[base + 1..base + offsets::HOF_MON_SIZE]
            .iter()
            .all(|&x| x == 0),
        "rest of the record zeroed"
    );
    assert_eq!(save.hof_team(0).len(), 1);

    // A terminator hides later slots, exactly like LeaguePCShowTeam.
    let mut team = save.hof_team_mut(0);
    team.set_mon(3, DEX_TO_INDEX[3], 60, "VENUS").expect("ok");
    assert_eq!(team.len(), 1, "slot 3 is unreachable past the terminator");
    assert!(team.mon(3).is_some(), "but direct access still sees it");
}

#[test]
fn team_count_lives_in_the_main_block() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert_eq!(save.hof_team_count(), 0);
    save.set_hof_team_count(51); // the game counts past storage capacity
    assert_eq!(save.hof_team_count(), 51);
    assert_eq!(save.as_bytes()[offsets::HOF_TEAM_COUNT], 51);
}

#[test]
#[should_panic(expected = "out of range")]
fn team_50_is_out_of_range() {
    let save = SaveFile::new_empty(GameVariant::RedBlue);
    save.hof_team(offsets::HOF_TEAM_CAPACITY);
}

#[test]
fn bad_nickname_is_rejected_without_writing() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let before = save.as_bytes().to_vec();
    let mut team = save.hof_team_mut(0);
    assert!(matches!(
        team.set_mon(0, DEX_TO_INDEX[151], 70, "~~~"),
        Err(TextError::Unencodable('~'))
    ));
    assert_eq!(save.as_bytes(), &before[..]);
}

#[test]
fn hof_only_edit_changes_only_hof_bytes_after_serialization() {
    // Bank 0 is not checksummed: even though the edit flag makes
    // to_bytes recompute all 15 checksums, they land on their old
    // values, so the serialized diff is exactly the HoF record.
    let pristine = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.hof_team_mut(10)
        .set_mon(2, DEX_TO_INDEX[149], 62, "DRAGON")
        .expect("ok");
    assert!(save.is_edited());

    let out = save.to_bytes();
    let rec = offsets::HALL_OF_FAME + 10 * offsets::HOF_TEAM_SIZE + 2 * offsets::HOF_MON_SIZE;
    for r in changed_ranges(&pristine, &out) {
        assert!(
            rec <= r.start && r.end <= rec + offsets::HOF_MON_SIZE,
            "changed 0x{:04X}..0x{:04X} outside the edited HoF record",
            r.start,
            r.end
        );
    }
    // ...and the count byte edit *does* move the main checksum.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_hof_team_count(1);
    let changed = changed_ranges(&pristine, &save.to_bytes());
    assert!(changed.contains(&(offsets::MAIN_CHECKSUM..offsets::MAIN_CHECKSUM + 1)));
}

#[test]
fn six_slots_per_team() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mut team = save.hof_team_mut(0);
    for slot in 0..HOF_TEAM_LEN {
        team.set_mon(slot, DEX_TO_INDEX[1 + slot], 50, "MON")
            .expect("ok");
    }
    assert_eq!(team.len(), 6);
}

#[test]
fn team_is_empty_reports_both_polarities_via_view_and_mut() {
    // Mutation hardening (issue #33).
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert!(save.hof_team(0).is_empty());
    assert!(save.hof_team_mut(0).is_empty());
    save.hof_team_mut(0)
        .set_mon(0, DEX_TO_INDEX[151], 70, "MEW")
        .expect("fits");
    assert!(!save.hof_team(0).is_empty());
    assert!(!save.hof_team_mut(0).is_empty());
}
