//! Integration tests for the flag arrays (`gen1::events`) and player
//! position (`gen1::map`).

use pksave::gen1::data::{EVENT_FLAG_NAMES, MAP_NAMES};
use pksave::gen1::events::TOWN_NAMES;
use pksave::gen1::offsets;
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};

#[test]
fn event_flag_bit_positions_match_raw_bytes() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    // LSB-first flag_array addressing: byte = bit >> 3, mask = 1 << (bit & 7).
    save.set_event_flag(0, true);
    save.set_event_flag(9, true);
    save.set_event_flag(2522, true); // last named event bit
    let b = save.as_bytes();
    assert_eq!(b[offsets::EVENT_FLAGS], 0b0000_0001);
    assert_eq!(b[offsets::EVENT_FLAGS + 1], 0b0000_0010);
    assert_eq!(b[offsets::EVENT_FLAGS + 2522 / 8], 1 << (2522 % 8));
    assert!(save.event_flag(0));
    assert!(save.event_flag(9));
    assert!(save.event_flag(2522));
    assert!(!save.event_flag(1));

    save.set_event_flag(9, false);
    assert!(!save.event_flag(9));
    assert_eq!(save.as_bytes()[offsets::EVENT_FLAGS + 1], 0);
}

#[test]
#[should_panic(expected = "out of range")]
fn event_flag_2560_is_out_of_range() {
    SaveFile::new_empty(GameVariant::RedBlue).event_flag(2560);
}

#[test]
fn event_flags_by_name() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    // Real names from the generated table, at known bits.
    assert_eq!(EVENT_FLAG_NAMES[0], "EVENT_FOLLOWED_OAK_INTO_LAB");
    assert_eq!(EVENT_FLAG_NAMES[0x18], "EVENT_GOT_TOWN_MAP");
    assert_eq!(EVENT_FLAG_NAMES[0x77], "EVENT_BEAT_BROCK");

    assert_eq!(save.event_flag_by_name("EVENT_BEAT_BROCK"), Some(false));
    assert!(save.set_event_flag_by_name("EVENT_BEAT_BROCK", true));
    assert_eq!(save.event_flag_by_name("EVENT_BEAT_BROCK"), Some(true));
    assert!(save.event_flag(0x77), "name mapped to bit 0x77");
    assert_eq!(
        save.as_bytes()[offsets::EVENT_FLAGS + 0x77 / 8],
        1 << (0x77 % 8)
    );

    assert!(save.set_event_flag_by_name("EVENT_GOT_TOWN_MAP", true));
    assert!(save.event_flag(0x18));

    // Unknown names read None and write nothing.
    assert_eq!(save.event_flag_by_name("EVENT_NO_SUCH_THING"), None);
    assert!(!save.set_event_flag_by_name("EVENT_NO_SUCH_THING", true));
    assert_eq!(save.event_flag_by_name(""), None, "gaps are not a name");
}

#[test]
fn named_event_flags_iterates_names_only() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_event_flag(0, true);
    save.set_event_flag(0x77, true);

    let named: Vec<(usize, &str, bool)> = save.named_event_flags().collect();
    let expected_count = EVENT_FLAG_NAMES.iter().filter(|n| !n.is_empty()).count();
    assert_eq!(named.len(), expected_count);
    assert!(named.iter().all(|(_, name, _)| !name.is_empty()));
    // Ascending bit order, and values reflect the buffer.
    assert!(named.windows(2).all(|w| w[0].0 < w[1].0));
    assert_eq!(named[0], (0, "EVENT_FOLLOWED_OAK_INTO_LAB", true));
    assert!(named
        .iter()
        .any(|&(bit, name, v)| bit == 0x77 && name == "EVENT_BEAT_BROCK" && v));
    assert!(named
        .iter()
        .any(|&(bit, name, v)| bit == 0x18 && name == "EVENT_GOT_TOWN_MAP" && !v));
}

#[test]
fn missable_hidden_item_and_coin_flags_hit_their_bytes() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_missable_flag(0, true);
    save.set_missable_flag(255, true);
    save.set_hidden_item_flag(3, true);
    save.set_hidden_item_flag(111, true);
    save.set_hidden_coin_flag(15, true);

    let b = save.as_bytes();
    assert_eq!(b[offsets::MISSABLE_FLAGS], 0x01);
    assert_eq!(b[offsets::MISSABLE_FLAGS + 31], 0x80);
    assert_eq!(b[offsets::HIDDEN_ITEM_FLAGS], 0x08);
    assert_eq!(b[offsets::HIDDEN_ITEM_FLAGS + 13], 0x80);
    assert_eq!(b[offsets::HIDDEN_COIN_FLAGS + 1], 0x80);

    assert!(save.missable_flag(255));
    assert!(save.hidden_item_flag(111));
    assert!(save.hidden_coin_flag(15));
    assert!(!save.hidden_coin_flag(0));
}

#[test]
fn town_visited_flags_use_map_id_order() {
    // TOWN_NAMES mirrors the first 11 map ids of map_constants.asm
    // (Saffron City last, id 0x0A).
    assert_eq!(TOWN_NAMES[0], "Pallet Town");
    assert_eq!(TOWN_NAMES[9], "Indigo Plateau");
    assert_eq!(TOWN_NAMES[10], "Saffron City");
    assert_eq!(MAP_NAMES[0], "PALLET_TOWN");
    assert_eq!(MAP_NAMES[10], "SAFFRON_CITY");

    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_town_visited(0, true); // Pallet = bit 0
    save.set_town_visited(10, true); // Saffron = bit 10
    let b = save.as_bytes();
    assert_eq!(b[offsets::TOWN_VISITED_FLAGS], 0b0000_0001);
    assert_eq!(b[offsets::TOWN_VISITED_FLAGS + 1], 0b0000_0100);
    assert!(save.town_visited(0));
    assert!(save.town_visited(10));
    assert!(!save.town_visited(1));

    save.set_town_visited(0, false);
    assert!(!save.town_visited(0));
}

#[test]
#[should_panic(expected = "out of range")]
fn town_index_11_is_rejected() {
    SaveFile::new_empty(GameVariant::RedBlue).town_visited(11);
}

#[test]
fn game_progress_flags_expose_the_raw_region() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    assert_eq!(
        save.game_progress_flags().len(),
        offsets::GAME_PROGRESS_FLAGS_LEN
    );
    save.game_progress_flags_mut()[0] = 0x5A;
    save.game_progress_flags_mut()[offsets::GAME_PROGRESS_FLAGS_LEN - 1] = 0xA5;
    let b = save.as_bytes();
    assert_eq!(b[offsets::GAME_PROGRESS_FLAGS], 0x5A);
    assert_eq!(
        b[offsets::GAME_PROGRESS_FLAGS + offsets::GAME_PROGRESS_FLAGS_LEN - 1],
        0xA5
    );
}

#[test]
fn map_fields_round_trip_raw_bytes() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_cur_map(0x0A);
    save.set_x_coord(19);
    save.set_y_coord(18);
    save.set_x_block_coord(1);
    save.set_y_block_coord(0);
    save.set_last_map(0x01);
    save.set_tileset(0x17);

    let b = save.as_bytes();
    assert_eq!(b[offsets::CUR_MAP], 0x0A);
    assert_eq!(b[offsets::X_COORD], 19);
    assert_eq!(b[offsets::Y_COORD], 18);
    assert_eq!(b[offsets::X_BLOCK_COORD], 1);
    assert_eq!(b[offsets::Y_BLOCK_COORD], 0);
    assert_eq!(b[offsets::LAST_MAP], 0x01);
    assert_eq!(b[offsets::CUR_MAP_TILESET], 0x17);

    assert_eq!(save.cur_map(), 0x0A);
    assert_eq!(save.cur_map_name(), Some("SAFFRON_CITY"));
    assert_eq!(save.x_coord(), 19);
    assert_eq!(save.y_coord(), 18);
    assert_eq!(save.last_map(), 0x01);
    assert_eq!(save.tileset(), 0x17);
}

#[test]
fn unknown_map_ids_have_no_name() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let unused = MAP_NAMES
        .iter()
        .position(|n| n.is_empty())
        .expect("the map table has unused ids") as u8;
    save.set_cur_map(unused);
    assert_eq!(save.cur_map_name(), None);
}

#[test]
fn map_view_pointer_is_little_endian() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_map_view_pointer(0xC6E8);
    let b = save.as_bytes();
    assert_eq!(b[offsets::MAP_VIEW_POINTER], 0xE8, "low byte first");
    assert_eq!(b[offsets::MAP_VIEW_POINTER + 1], 0xC6);
    assert_eq!(save.map_view_pointer(), 0xC6E8);
}

#[test]
fn warp_to_derives_block_coords_as_the_game_does() {
    // LoadTilesetHeader (pokered engine/overworld/tilesets.asm), after a
    // warp: wYBlockCoord := wYCoord & 1, wXBlockCoord := wXCoord & 1.
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.warp_to(0x02, 5, 7); // Pewter City, both coords odd
    assert_eq!(save.cur_map(), 0x02);
    assert_eq!((save.x_coord(), save.y_coord()), (5, 7));
    assert_eq!((save.x_block_coord(), save.y_block_coord()), (1, 1));

    save.warp_to(0x00, 4, 6); // both even
    assert_eq!((save.x_block_coord(), save.y_block_coord()), (0, 0));

    save.warp_to(0x01, 10, 3); // mixed
    assert_eq!((save.x_block_coord(), save.y_block_coord()), (0, 1));
}

#[test]
fn warp_to_touches_only_map_and_coord_bytes() {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    let before = save.as_bytes().to_vec();
    save.warp_to(0x05, 13, 9);
    // wCurMap (0x260A) and wYCoord..wXBlockCoord (0x260D..0x2611) — the
    // view pointer between them (0x260B-0x260C) must not move.
    for r in changed_ranges(&before, save.as_bytes()) {
        let in_map_byte = r == (offsets::CUR_MAP..offsets::CUR_MAP + 1);
        let in_coords = offsets::Y_COORD <= r.start && r.end <= offsets::X_BLOCK_COORD + 1;
        assert!(
            in_map_byte || in_coords,
            "changed 0x{:04X}..0x{:04X} outside warp_to's fields",
            r.start,
            r.end
        );
    }
    assert_eq!(save.map_view_pointer(), 0, "pointer left untouched");
}
