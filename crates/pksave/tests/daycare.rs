//! Integration tests for the daycare (`gen1::daycare`).

use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::offsets;
use pksave::gen1::pokemon::{BoxMonMut, MonMut, MonView};
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::gen1::text::TextError;

/// The whole daycare region: in-use byte, nickname, OT, mon record
/// (contiguous per FORMAT.md, ending right at sSpriteData).
const DAYCARE_REGION: core::ops::Range<usize> = offsets::DAYCARE_IN_USE..offsets::SPRITE_DATA;

fn make_box_mon(dex: usize, level: u8) -> [u8; offsets::BOX_MON_SIZE] {
    let mut bytes = [0u8; offsets::BOX_MON_SIZE];
    let mut mon = BoxMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_box_level(level);
    mon.set_ot_id(0xBEEF);
    bytes
}

#[test]
fn empty_save_has_no_daycare_occupant() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert!(save.daycare().is_none());
    assert!(save.daycare_mut().is_none());
}

#[test]
fn set_daycare_deposits_and_views_read_back() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let mon = make_box_mon(113, 40); // Chansey
    save.set_daycare(Some((&mon, "JOY", "LUCKY"))).expect("ok");

    assert_eq!(save.as_bytes()[offsets::DAYCARE_IN_USE], 1);
    let dc = save.daycare().expect("occupied");
    assert_eq!(dc.mon().species(), DEX_TO_INDEX[113]);
    assert_eq!(dc.mon().box_level(), 40);
    assert_eq!(dc.mon().ot_id(), 0xBEEF);
    assert_eq!(dc.ot_name(), "JOY");
    assert_eq!(dc.nickname(), "LUCKY");
}

#[test]
fn daycare_mut_edits_fields_in_place() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_daycare(Some((&make_box_mon(113, 40), "JOY", "LUCKY")))
        .expect("ok");

    let mut dc = save.daycare_mut().expect("occupied");
    dc.mon_mut().set_box_level(41);
    dc.set_nickname("EGGSY").expect("fits");
    dc.set_ot_name("JENNY").expect("fits");
    assert_eq!(dc.mon().box_level(), 41);
    assert_eq!(dc.as_view().nickname(), "EGGSY");

    let dc = save.daycare().expect("still occupied");
    assert_eq!(dc.nickname(), "EGGSY");
    assert_eq!(dc.ot_name(), "JENNY");
    assert_eq!(dc.mon().box_level(), 41);
}

#[test]
fn clearing_writes_only_the_in_use_byte_and_leaves_stale_bytes() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_daycare(Some((&make_box_mon(113, 40), "JOY", "LUCKY")))
        .expect("ok");
    let before = save.as_bytes().to_vec();

    save.set_daycare(None).expect("ok");
    assert!(save.daycare().is_none());
    assert_eq!(
        changed_ranges(&before, save.as_bytes()),
        vec![offsets::DAYCARE_IN_USE..offsets::DAYCARE_IN_USE + 1],
        "None writes exactly the in-use byte"
    );
    // The stale mon record is still there, just ignored.
    assert_eq!(save.as_bytes()[offsets::DAYCARE_MON], DEX_TO_INDEX[113]);
}

#[test]
fn deposit_touches_only_the_daycare_region() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let before = save.as_bytes().to_vec();
    save.set_daycare(Some((&make_box_mon(1, 12), "RED", "BULBA")))
        .expect("ok");
    for r in changed_ranges(&before, save.as_bytes()) {
        assert!(
            DAYCARE_REGION.start <= r.start && r.end <= DAYCARE_REGION.end,
            "changed 0x{:04X}..0x{:04X} outside the daycare region",
            r.start,
            r.end
        );
    }
}

#[test]
fn bad_names_are_rejected_before_anything_is_written() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let before = save.as_bytes().to_vec();
    let mon = make_box_mon(1, 12);
    assert!(matches!(
        save.set_daycare(Some((&mon, "RED", "~~~"))),
        Err(TextError::Unencodable('~'))
    ));
    assert!(matches!(
        save.set_daycare(Some((&mon, "WAYTOOLONGNAME", "OK"))),
        Err(TextError::TooLong { .. })
    ));
    assert_eq!(save.as_bytes(), &before[..]);
    assert!(save.daycare().is_none());
}

#[test]
fn nonzero_in_use_byte_counts_as_occupied() {
    // The game writes 0/1, but a corrupt save may hold anything nonzero.
    let mut bytes = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
    bytes[offsets::DAYCARE_IN_USE] = 0x2A;
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    assert!(save.daycare().is_some());
}
