//! Human-readable labels for byte ranges of a Gen 1 save.
//!
//! Maps `changed_ranges` output to known field spans ("Money", "Party
//! slot 2", "Main checksum", …) for the history diff summary. This is a
//! deliberately app-side table built from `pksave::gen1::offsets`
//! constants — the core's public API gains nothing (issue #9: core
//! support is optional).

use std::ops::Range;
use std::sync::OnceLock;

use pksave::gen1::offsets;

/// The known field spans, in file order. Built once from the offsets
/// constants (the single source of truth for the layout).
fn table() -> &'static [(Range<usize>, String)] {
    static TABLE: OnceLock<Vec<(Range<usize>, String)>> = OnceLock::new();
    TABLE.get_or_init(build_table)
}

fn build_table() -> Vec<(Range<usize>, String)> {
    let mut spans: Vec<(Range<usize>, String)> = Vec::new();
    let mut add = |start: usize, len: usize, label: String| spans.push((start..start + len, label));

    // ---- bank 0 ----
    add(
        offsets::HALL_OF_FAME,
        offsets::HOF_TEAM_CAPACITY * offsets::HOF_TEAM_SIZE,
        "Hall of Fame".to_owned(),
    );

    // ---- bank 1: main data ----
    add(
        offsets::PLAYER_NAME,
        offsets::NAME_LEN,
        "Player name".to_owned(),
    );
    add(
        offsets::POKEDEX_OWNED,
        offsets::POKEDEX_LEN,
        "Pokédex owned".to_owned(),
    );
    add(
        offsets::POKEDEX_SEEN,
        offsets::POKEDEX_LEN,
        "Pokédex seen".to_owned(),
    );
    add(
        offsets::BAG_ITEM_COUNT,
        1 + 2 * offsets::BAG_CAPACITY + 1,
        "Bag items".to_owned(),
    );
    add(offsets::MONEY, 3, "Money".to_owned());
    add(
        offsets::RIVAL_NAME,
        offsets::NAME_LEN,
        "Rival name".to_owned(),
    );
    add(offsets::OPTIONS, 1, "Options".to_owned());
    add(offsets::BADGES, 1, "Badges".to_owned());
    add(offsets::PLAYER_ID, 2, "Trainer ID".to_owned());
    // wCurMap through wCurMapTileset: the location/warp block.
    add(
        offsets::CUR_MAP,
        offsets::CUR_MAP_TILESET - offsets::CUR_MAP + 1,
        "Map & position".to_owned(),
    );
    add(
        offsets::PIKACHU_FRIENDSHIP,
        1,
        "Pikachu friendship".to_owned(),
    );
    add(
        offsets::PC_ITEM_COUNT,
        1 + 2 * offsets::PC_ITEM_CAPACITY + 1,
        "PC items".to_owned(),
    );
    add(offsets::CURRENT_BOX_NUM, 1, "Current box number".to_owned());
    add(offsets::HOF_TEAM_COUNT, 1, "Hall of Fame count".to_owned());
    add(offsets::COINS, 2, "Coins".to_owned());
    add(
        offsets::MISSABLE_FLAGS,
        offsets::MISSABLE_FLAGS_LEN,
        "Missable objects".to_owned(),
    );
    add(
        offsets::GAME_PROGRESS_FLAGS,
        offsets::GAME_PROGRESS_FLAGS_LEN,
        "Game progress flags".to_owned(),
    );
    add(
        offsets::HIDDEN_ITEM_FLAGS,
        offsets::HIDDEN_ITEM_FLAGS_LEN,
        "Hidden items".to_owned(),
    );
    add(offsets::HIDDEN_COIN_FLAGS, 2, "Hidden coins".to_owned());
    add(
        offsets::TOWN_VISITED_FLAGS,
        2,
        "Fly-unlocked towns".to_owned(),
    );
    add(
        offsets::EVENT_FLAGS,
        offsets::EVENT_FLAGS_LEN,
        "Event flags".to_owned(),
    );
    add(
        offsets::PLAY_TIME_HOURS,
        offsets::PLAY_TIME_FRAMES - offsets::PLAY_TIME_HOURS + 1,
        "Play time".to_owned(),
    );
    add(
        offsets::DAYCARE_IN_USE,
        offsets::DAYCARE_MON + offsets::BOX_MON_SIZE - offsets::DAYCARE_IN_USE,
        "Daycare".to_owned(),
    );

    // ---- party: count, species list, then per-slot spans ----
    add(offsets::PARTY, 1, "Party count".to_owned());
    add(
        offsets::PARTY + 1,
        offsets::PARTY_CAPACITY + 1,
        "Party species list".to_owned(),
    );
    let mons = offsets::PARTY + 1 + offsets::PARTY_CAPACITY + 1;
    for slot in 0..offsets::PARTY_CAPACITY {
        add(
            mons + slot * offsets::PARTY_MON_SIZE,
            offsets::PARTY_MON_SIZE,
            format!("Party slot {}", slot + 1),
        );
    }
    let ots = mons + offsets::PARTY_CAPACITY * offsets::PARTY_MON_SIZE;
    for slot in 0..offsets::PARTY_CAPACITY {
        add(
            ots + slot * offsets::NAME_LEN,
            offsets::NAME_LEN,
            format!("Party slot {} OT", slot + 1),
        );
    }
    let nicknames = ots + offsets::PARTY_CAPACITY * offsets::NAME_LEN;
    for slot in 0..offsets::PARTY_CAPACITY {
        add(
            nicknames + slot * offsets::NAME_LEN,
            offsets::NAME_LEN,
            format!("Party slot {} nickname", slot + 1),
        );
    }

    add(
        offsets::CURRENT_BOX,
        offsets::BOX_LEN,
        "Current box (working copy)".to_owned(),
    );
    add(offsets::MAIN_CHECKSUM, 1, "Main checksum".to_owned());

    // ---- banks 2/3: boxes and their checksums ----
    for n in 0..offsets::NUM_BOXES {
        add(
            offsets::box_offset(n),
            offsets::BOX_LEN,
            format!("Box {}", n + 1),
        );
    }
    for (bank, base, first_box) in [
        (2, offsets::BANK2_ALL_BOXES_CHECKSUM, 1),
        (3, offsets::BANK3_ALL_BOXES_CHECKSUM, 7),
    ] {
        add(base, 1, format!("Bank {bank} all-boxes checksum"));
        for i in 0..6 {
            add(base + 1 + i, 1, format!("Box {} checksum", first_box + i));
        }
    }

    spans.sort_by_key(|(range, _)| range.start);
    spans
}

/// All labels of known field spans overlapping `range`, in file order,
/// deduplicated. Empty when the range touches no known field.
pub fn labels_for(range: &Range<usize>) -> Vec<String> {
    let mut labels: Vec<String> = Vec::new();
    for (span, label) in table() {
        if span.start < range.end && range.start < span.end && !labels.contains(label) {
            labels.push(label.clone());
        }
    }
    labels
}

/// One display line per changed range: `0xAAAA..0xBBBB (N bytes): L, L`
/// with `unlabeled` as the fallback for unknown spans.
pub fn describe(ranges: &[Range<usize>]) -> Vec<String> {
    ranges
        .iter()
        .map(|range| {
            let labels = labels_for(range);
            let what = if labels.is_empty() {
                "unlabeled".to_owned()
            } else {
                labels.join(", ")
            };
            let len = range.len();
            let plural = if len == 1 { "" } else { "s" };
            format!(
                "0x{:04X}..0x{:04X} ({len} byte{plural}): {what}",
                range.start, range.end
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pksave::gen1::offsets;

    fn single(range: Range<usize>) -> Vec<String> {
        labels_for(&range)
    }

    #[test]
    fn money_edit_is_labeled_money() {
        assert_eq!(
            single(offsets::MONEY..offsets::MONEY + 3),
            vec!["Money".to_owned()]
        );
        // A single-byte edit inside the field also matches.
        assert_eq!(
            single(offsets::MONEY + 1..offsets::MONEY + 2),
            vec!["Money".to_owned()]
        );
    }

    #[test]
    fn party_mon_edit_is_labeled_with_its_slot() {
        // Party mon records start after the count byte and the 7-byte
        // species list (6 + 0xFF sentinel).
        let mons = offsets::PARTY + 1 + offsets::PARTY_CAPACITY + 1;
        let slot1 = mons..mons + offsets::PARTY_MON_SIZE;
        assert_eq!(single(slot1), vec!["Party slot 1".to_owned()]);
        let in_slot3 = mons + 2 * offsets::PARTY_MON_SIZE + 5;
        assert_eq!(
            single(in_slot3..in_slot3 + 1),
            vec!["Party slot 3".to_owned()]
        );
    }

    #[test]
    fn party_nickname_edit_names_the_slot() {
        let mons = offsets::PARTY + 1 + offsets::PARTY_CAPACITY + 1;
        let nicknames = mons
            + offsets::PARTY_CAPACITY * offsets::PARTY_MON_SIZE
            + offsets::PARTY_CAPACITY * offsets::NAME_LEN;
        let nick2 = nicknames + offsets::NAME_LEN;
        assert_eq!(
            single(nick2..nick2 + 3),
            vec!["Party slot 2 nickname".to_owned()]
        );
    }

    #[test]
    fn checksum_bytes_are_labeled_with_their_names() {
        assert_eq!(
            single(offsets::MAIN_CHECKSUM..offsets::MAIN_CHECKSUM + 1),
            vec!["Main checksum".to_owned()]
        );
        assert_eq!(
            single(offsets::BANK2_ALL_BOXES_CHECKSUM..offsets::BANK2_ALL_BOXES_CHECKSUM + 1),
            vec!["Bank 2 all-boxes checksum".to_owned()]
        );
        // Per-box checksums follow the bank checksum byte.
        assert_eq!(
            single(offsets::BANK2_ALL_BOXES_CHECKSUM + 3..offsets::BANK2_ALL_BOXES_CHECKSUM + 4),
            vec!["Box 3 checksum".to_owned()]
        );
        assert_eq!(
            single(offsets::BANK3_ALL_BOXES_CHECKSUM + 6..offsets::BANK3_ALL_BOXES_CHECKSUM + 7),
            vec!["Box 12 checksum".to_owned()]
        );
    }

    #[test]
    fn boxes_and_current_box_are_labeled() {
        let b1 = offsets::box_offset(0);
        assert_eq!(single(b1 + 10..b1 + 12), vec!["Box 1".to_owned()]);
        let b12 = offsets::box_offset(11);
        assert_eq!(single(b12..b12 + 1), vec!["Box 12".to_owned()]);
        assert_eq!(
            single(offsets::CURRENT_BOX..offsets::CURRENT_BOX + 4),
            vec!["Current box (working copy)".to_owned()]
        );
    }

    #[test]
    fn a_range_spanning_fields_lists_all_of_them() {
        // Money (3 bytes) is directly followed by the rival name.
        let labels = single(offsets::MONEY..offsets::RIVAL_NAME + 2);
        assert_eq!(
            labels,
            vec!["Money".to_owned(), "Rival name".to_owned()],
            "in file order, deduplicated"
        );
    }

    #[test]
    fn unknown_ranges_yield_no_labels_and_an_unlabeled_description() {
        // The gap between the checksummed region and bank 2.
        assert_eq!(single(0x3600..0x3610), Vec::<String>::new());
        let range = 0x3600..0x3610;
        let lines = describe(std::slice::from_ref(&range));
        assert_eq!(lines.len(), 1);
        assert!(
            lines[0].contains("unlabeled"),
            "fallback expected: {}",
            lines[0]
        );
        assert!(lines[0].contains("0x3600"), "range shown: {}", lines[0]);
    }

    #[test]
    fn describe_prints_range_size_and_labels() {
        let range = offsets::MONEY..offsets::MONEY + 3;
        let lines = describe(std::slice::from_ref(&range));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("Money"), "line: {}", lines[0]);
        assert!(lines[0].contains("3 byte"), "line: {}", lines[0]);
    }

    #[test]
    fn more_fields_are_labeled() {
        assert_eq!(single(offsets::COINS..offsets::COINS + 2), vec!["Coins"]);
        assert_eq!(single(offsets::BADGES..offsets::BADGES + 1), vec!["Badges"]);
        assert_eq!(
            single(offsets::PLAYER_NAME..offsets::PLAYER_NAME + 2),
            vec!["Player name"]
        );
        assert_eq!(
            single(offsets::POKEDEX_OWNED..offsets::POKEDEX_OWNED + 1),
            vec!["Pokédex owned"]
        );
        assert_eq!(
            single(offsets::PLAY_TIME_HOURS..offsets::PLAY_TIME_HOURS + 1),
            vec!["Play time"]
        );
        assert_eq!(
            single(offsets::HALL_OF_FAME..offsets::HALL_OF_FAME + 96),
            vec!["Hall of Fame"]
        );
        assert_eq!(
            single(offsets::EVENT_FLAGS + 100..offsets::EVENT_FLAGS + 101),
            vec!["Event flags"]
        );
    }
}
