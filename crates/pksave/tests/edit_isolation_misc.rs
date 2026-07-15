//! P2 for the M9-M13 setters: every operation touches only its own
//! bytes, and serialization adds only the checksum bytes of the regions
//! it dirtied.
//!
//! Same registry approach as `edit_isolation.rs`, extended with
//! per-edit *checksum* expectations: ops on the main block add the main
//! checksum, ops on a bank box add that bank's all-boxes checksum plus
//! the per-box checksum, and Hall of Fame edits (bank 0, unchecksummed)
//! add nothing at all.

use core::ops::Range;

use pksave::gen1::checksum::Region;
use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::offsets;
use pksave::gen1::pokemon::{BoxMonMut, MonMut, PartyMonMut};
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};

fn make_box_mon(dex: usize, level: u8) -> [u8; offsets::BOX_MON_SIZE] {
    let mut bytes = [0u8; offsets::BOX_MON_SIZE];
    let mut mon = BoxMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_box_level(level);
    bytes
}

fn make_party_mon(dex: usize, level: u8) -> [u8; offsets::PARTY_MON_SIZE] {
    let mut bytes = [0u8; offsets::PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_level_coherent(level);
    bytes
}

struct Edit {
    name: &'static str,
    /// Preparation applied before the baseline snapshot is taken (so
    /// its bytes don't count as the edit's changes).
    setup: Box<dyn Fn(&mut SaveFile)>,
    apply: Box<dyn Fn(&mut SaveFile)>,
    /// Where raw-buffer changes may land.
    allowed: Vec<Range<usize>>,
    /// Checksum bytes `to_bytes()` may additionally rewrite.
    checksums: Vec<usize>,
}

fn span(start: usize, len: usize) -> Range<usize> {
    start..start + len
}

fn box_block(n: usize) -> Range<usize> {
    span(offsets::box_offset(n), offsets::BOX_LEN)
}

const CURRENT_BOX_BLOCK: Range<usize> =
    offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN;
const PARTY_BLOCK: Range<usize> = offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN;
const MAIN: usize = offsets::MAIN_CHECKSUM;

/// Checksum bytes for bank box `n`: the bank's all-boxes checksum and
/// the box's own.
fn bank_checksums(n: usize) -> Vec<usize> {
    vec![
        if n < 6 {
            Region::Bank2AllBoxes.checksum_offset()
        } else {
            Region::Bank3AllBoxes.checksum_offset()
        },
        Region::Box(n).checksum_offset(),
    ]
}

fn registry() -> Vec<Edit> {
    vec![
        Edit {
            name: "box_mut(5) add (non-current box)",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                s.box_mut(5)
                    .add(&make_box_mon(151, 30), "RED", "MEW")
                    .expect("room");
            }),
            allowed: vec![box_block(5)],
            checksums: bank_checksums(5),
        },
        Edit {
            name: "box_mut(9) add+swap+remove (bank 3 box)",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                let mut bx = s.box_mut(9);
                bx.add(&make_box_mon(1, 5), "A", "X").expect("room");
                bx.add(&make_box_mon(4, 6), "B", "Y").expect("room");
                bx.swap(0, 1);
                bx.remove(0);
            }),
            allowed: vec![box_block(9)],
            checksums: bank_checksums(9),
        },
        Edit {
            name: "box_mut(0) add (current box -> working copy)",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                s.box_mut(0)
                    .add(&make_box_mon(151, 30), "RED", "MEW")
                    .expect("room");
            }),
            allowed: vec![CURRENT_BOX_BLOCK],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_current_box_number",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_current_box_number(7)),
            allowed: vec![span(offsets::CURRENT_BOX_NUM, 1)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "edit working copy + sync_current_box_to_bank",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                s.box_mut(0)
                    .add(&make_box_mon(151, 30), "RED", "MEW")
                    .expect("room");
                s.sync_current_box_to_bank();
            }),
            allowed: vec![CURRENT_BOX_BLOCK, box_block(0)],
            checksums: [vec![MAIN], bank_checksums(0)].concat(),
        },
        Edit {
            name: "deposit party slot 0 into box 3",
            setup: Box::new(|s| {
                s.party_mut()
                    .add(&make_party_mon(25, 42), "ASH", "SPARKY")
                    .expect("room");
            }),
            apply: Box::new(|s| s.deposit(0, 3).expect("valid deposit")),
            allowed: vec![PARTY_BLOCK, box_block(3)],
            checksums: [vec![MAIN], bank_checksums(3)].concat(),
        },
        Edit {
            name: "withdraw from box 8 into the party",
            setup: Box::new(|s| {
                s.box_mut(8)
                    .add(&make_box_mon(151, 30), "RED", "MEW")
                    .expect("room");
            }),
            apply: Box::new(|s| s.withdraw(8, 0).expect("valid withdraw")),
            allowed: vec![PARTY_BLOCK, box_block(8)],
            checksums: [vec![MAIN], bank_checksums(8)].concat(),
        },
        Edit {
            name: "set_daycare deposit",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                s.set_daycare(Some((&make_box_mon(113, 40), "JOY", "LUCKY")))
                    .expect("names encode");
            }),
            allowed: vec![span(
                offsets::DAYCARE_IN_USE,
                offsets::SPRITE_DATA - offsets::DAYCARE_IN_USE,
            )],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_event_flag",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_event_flag(0x77, true)),
            allowed: vec![span(offsets::EVENT_FLAGS, offsets::EVENT_FLAGS_LEN)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_event_flag_by_name",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                assert!(s.set_event_flag_by_name("EVENT_GOT_TOWN_MAP", true));
            }),
            allowed: vec![span(offsets::EVENT_FLAGS, offsets::EVENT_FLAGS_LEN)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_missable_flag",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_missable_flag(200, true)),
            allowed: vec![span(offsets::MISSABLE_FLAGS, offsets::MISSABLE_FLAGS_LEN)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_hidden_item_flag",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_hidden_item_flag(50, true)),
            allowed: vec![span(
                offsets::HIDDEN_ITEM_FLAGS,
                offsets::HIDDEN_ITEM_FLAGS_LEN,
            )],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_hidden_coin_flag",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_hidden_coin_flag(9, true)),
            allowed: vec![span(offsets::HIDDEN_COIN_FLAGS, 2)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_town_visited",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_town_visited(10, true)),
            allowed: vec![span(offsets::TOWN_VISITED_FLAGS, 2)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "game_progress_flags_mut",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.game_progress_flags_mut()[3] = 0x5A),
            allowed: vec![span(
                offsets::GAME_PROGRESS_FLAGS,
                offsets::GAME_PROGRESS_FLAGS_LEN,
            )],
            checksums: vec![MAIN],
        },
        Edit {
            name: "warp_to",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.warp_to(0x0A, 13, 9)),
            allowed: vec![
                span(offsets::CUR_MAP, 1),
                offsets::Y_COORD..offsets::X_BLOCK_COORD + 1,
            ],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_map_view_pointer",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_map_view_pointer(0xC7A2)),
            allowed: vec![span(offsets::MAP_VIEW_POINTER, 2)],
            checksums: vec![MAIN],
        },
        Edit {
            name: "set_last_map + set_tileset",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                s.set_last_map(0x0C);
                s.set_tileset(0x03);
            }),
            allowed: vec![
                span(offsets::LAST_MAP, 1),
                span(offsets::CUR_MAP_TILESET, 1),
            ],
            checksums: vec![MAIN],
        },
        Edit {
            name: "hof_team_mut set_mon (bank 0, unchecksummed)",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| {
                s.hof_team_mut(4)
                    .set_mon(1, DEX_TO_INDEX[6], 78, "ZARD")
                    .expect("nickname encodes");
            }),
            allowed: vec![span(
                offsets::HALL_OF_FAME + 4 * offsets::HOF_TEAM_SIZE + offsets::HOF_MON_SIZE,
                offsets::HOF_MON_SIZE,
            )],
            checksums: vec![], // bank 0 is not checksummed
        },
        Edit {
            name: "hof clear_slot (bank 0, unchecksummed)",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.hof_team_mut(0).clear_slot(0)),
            allowed: vec![span(offsets::HALL_OF_FAME, offsets::HOF_MON_SIZE)],
            checksums: vec![],
        },
        Edit {
            name: "set_hof_team_count (main block)",
            setup: Box::new(|_| {}),
            apply: Box::new(|s| s.set_hof_team_count(3)),
            allowed: vec![span(offsets::HOF_TEAM_COUNT, 1)],
            checksums: vec![MAIN],
        },
    ]
}

/// Every changed range must lie inside one of the allowed ranges.
fn assert_within(what: &str, changed: &[Range<usize>], allowed: &[Range<usize>]) {
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
fn raw_buffer_changes_stay_inside_each_ops_spans() {
    for edit in registry() {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        (edit.setup)(&mut save);
        let before = save.as_bytes().to_vec();
        (edit.apply)(&mut save);
        let changed = changed_ranges(&before, save.as_bytes());
        assert!(
            !changed.is_empty(),
            "{}: registry edit must actually change bytes",
            edit.name
        );
        assert_within(edit.name, &changed, &edit.allowed);
    }
}

#[test]
fn serialization_adds_only_the_regions_own_checksum_bytes() {
    for edit in registry() {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        (edit.setup)(&mut save);
        let pristine = save.to_bytes();
        (edit.apply)(&mut save);
        let changed = changed_ranges(&pristine, &save.to_bytes());

        let mut allowed = edit.allowed.clone();
        // Coalesce adjacent checksum bytes (e.g. a bank's all-boxes
        // checksum at 0x5A4C and box 1's at 0x5A4D form one changed run).
        let mut sums = edit.checksums.clone();
        sums.sort_unstable();
        for &at in &sums {
            match allowed.last_mut() {
                Some(last) if last.end == at => last.end = at + 1,
                _ => allowed.push(at..at + 1),
            }
        }
        assert_within(edit.name, &changed, &allowed);

        // Each dirtied region's checksum byte must actually have moved
        // (the registry's edits all change their regions' sums), and
        // HoF-only edits must leave every checksum byte alone.
        for &at in &edit.checksums {
            assert!(
                changed.iter().any(|r| r.start <= at && at < r.end),
                "{}: checksum byte 0x{at:04X} should have been rewritten",
                edit.name
            );
        }
        if edit.checksums.is_empty() {
            for region in Region::ALL {
                let at = region.checksum_offset();
                assert!(
                    !changed.iter().any(|r| r.start <= at && at < r.end),
                    "{}: checksum byte 0x{at:04X} moved for an unchecksummed edit",
                    edit.name
                );
            }
        }
    }
}
