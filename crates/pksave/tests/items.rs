//! M5: terminated item lists (bag and PC).

use pksave::gen1::items::{diagnose_item_list, ItemError, BAG_LIST, PC_LIST};
use pksave::gen1::offsets;
use pksave::gen1::save::{GameVariant, SaveFile};
use pksave::Severity;

fn blank() -> SaveFile {
    SaveFile::new_empty(GameVariant::RedBlue)
}

const POTION: u8 = 0x14;
const MASTER_BALL: u8 = 0x01;
const BICYCLE: u8 = 0x06;

#[test]
fn empty_lists_read_as_empty() {
    let save = blank();
    assert_eq!(save.bag_items().len(), 0);
    assert_eq!(save.bag_items().get(0), None);
    assert_eq!(save.bag_items().iter().count(), 0);
    assert_eq!(save.pc_items().len(), 0);
}

#[test]
fn add_to_empty_writes_count_pair_and_terminator() {
    let mut save = blank();
    save.bag_items_mut().add(POTION, 5).expect("has room");
    let view = save.bag_items();
    assert_eq!(view.len(), 1);
    assert_eq!(view.get(0), Some((POTION, 5)));
    let b = save.as_bytes();
    assert_eq!(b[offsets::BAG_ITEM_COUNT], 1);
    assert_eq!(b[offsets::BAG_ITEMS], POTION);
    assert_eq!(b[offsets::BAG_ITEMS + 1], 5);
    assert_eq!(b[offsets::BAG_ITEMS + 2], 0xFF);
    assert!(save.is_edited());
}

#[test]
fn add_clamps_quantity_to_1_through_99() {
    let mut save = blank();
    save.bag_items_mut().add(POTION, 0).expect("has room");
    save.bag_items_mut().add(BICYCLE, 200).expect("has room");
    assert_eq!(save.bag_items().get(0), Some((POTION, 1)));
    assert_eq!(save.bag_items().get(1), Some((BICYCLE, 99)));
}

#[test]
fn add_to_full_bag_fails_without_touching_bytes() {
    let mut save = blank();
    for i in 0..offsets::BAG_CAPACITY as u8 {
        save.bag_items_mut().add(i + 1, 1).expect("has room");
    }
    assert_eq!(save.bag_items().len(), 20);
    // Terminator sits at the last byte of the 41-byte field.
    assert_eq!(save.as_bytes()[offsets::BAG_ITEMS + 40], 0xFF);
    let before = save.as_bytes().to_vec();
    assert_eq!(
        save.bag_items_mut().add(POTION, 1),
        Err(ItemError::CapacityFull { capacity: 20 })
    );
    assert_eq!(save.as_bytes(), &before[..]);
}

#[test]
fn pc_holds_50_items() {
    let mut save = blank();
    for i in 0..offsets::PC_ITEM_CAPACITY as u8 {
        save.pc_items_mut().add(i + 1, 2).expect("has room");
    }
    assert_eq!(save.pc_items().len(), 50);
    assert_eq!(save.as_bytes()[offsets::PC_ITEMS + 100], 0xFF);
    assert_eq!(
        save.pc_items_mut().add(POTION, 1),
        Err(ItemError::CapacityFull { capacity: 50 })
    );
}

fn bag_with_three() -> SaveFile {
    let mut save = blank();
    save.bag_items_mut().add(MASTER_BALL, 1).expect("room");
    save.bag_items_mut().add(POTION, 42).expect("room");
    save.bag_items_mut().add(BICYCLE, 1).expect("room");
    save
}

#[test]
fn remove_first_shifts_pairs_down() {
    let mut save = bag_with_three();
    save.bag_items_mut().remove(0);
    let view = save.bag_items();
    assert_eq!(view.len(), 2);
    assert_eq!(view.get(0), Some((POTION, 42)));
    assert_eq!(view.get(1), Some((BICYCLE, 1)));
    // Terminator right after the new last pair.
    assert_eq!(save.as_bytes()[offsets::BAG_ITEMS + 4], 0xFF);
}

#[test]
fn remove_middle_and_last() {
    let mut save = bag_with_three();
    save.bag_items_mut().remove(1);
    assert_eq!(
        save.bag_items().iter().collect::<Vec<_>>(),
        vec![(MASTER_BALL, 1), (BICYCLE, 1)]
    );
    save.bag_items_mut().remove(1);
    assert_eq!(
        save.bag_items().iter().collect::<Vec<_>>(),
        vec![(MASTER_BALL, 1)]
    );
    assert_eq!(save.as_bytes()[offsets::BAG_ITEM_COUNT], 1);
    assert_eq!(save.as_bytes()[offsets::BAG_ITEMS + 2], 0xFF);
}

#[test]
fn remove_leaves_bytes_past_the_terminator_untouched() {
    let mut save = bag_with_three();
    let before = save.as_bytes().to_vec();
    save.bag_items_mut().remove(0);
    let after = save.as_bytes();
    // Canonical policy: pairs shift down, terminator is written at the
    // new end (offset 2*len), and everything past it stays as-is — here
    // the stale qty byte of the old last pair and the old terminator.
    assert_eq!(
        after[offsets::BAG_ITEMS + 5],
        before[offsets::BAG_ITEMS + 5]
    );
    assert_eq!(
        after[offsets::BAG_ITEMS + 6],
        before[offsets::BAG_ITEMS + 6]
    );
}

#[test]
#[should_panic(expected = "out of range")]
fn remove_out_of_range_panics() {
    let mut save = bag_with_three();
    save.bag_items_mut().remove(3);
}

#[test]
fn set_qty_clamps_and_set_id_overwrites() {
    let mut save = bag_with_three();
    save.bag_items_mut().set_qty(1, 0);
    assert_eq!(save.bag_items().get(1), Some((POTION, 1)));
    save.bag_items_mut().set_qty(1, 150);
    assert_eq!(save.bag_items().get(1), Some((POTION, 99)));
    save.bag_items_mut().set_qty(1, 7);
    assert_eq!(save.bag_items().get(1), Some((POTION, 7)));
    save.bag_items_mut().set_id(1, MASTER_BALL);
    assert_eq!(save.bag_items().get(1), Some((MASTER_BALL, 7)));
}

// The documented panic contract of set_qty/set_id/swap: index >= len()
// panics via pair_offset ("item index {i} out of range (len {n})").

#[test]
#[should_panic(expected = "out of range")]
fn set_qty_out_of_range_panics() {
    let mut save = bag_with_three();
    save.bag_items_mut().set_qty(3, 1);
}

#[test]
#[should_panic(expected = "out of range")]
fn set_id_out_of_range_panics() {
    let mut save = bag_with_three();
    save.bag_items_mut().set_id(3, POTION);
}

#[test]
#[should_panic(expected = "out of range")]
fn swap_out_of_range_panics() {
    let mut save = bag_with_three();
    save.bag_items_mut().swap(0, 3);
}

#[test]
fn swap_exchanges_pairs() {
    let mut save = bag_with_three();
    save.bag_items_mut().swap(0, 2);
    assert_eq!(
        save.bag_items().iter().collect::<Vec<_>>(),
        vec![(BICYCLE, 1), (POTION, 42), (MASTER_BALL, 1)]
    );
    save.bag_items_mut().swap(1, 1); // self-swap is a no-op
    assert_eq!(save.bag_items().get(1), Some((POTION, 42)));
}

#[test]
fn structural_edits_touch_only_the_list_region() {
    // Compare whole buffers around every mutating op; anything outside
    // count byte + items field must be byte-identical.
    let bag_region = offsets::BAG_ITEM_COUNT..offsets::BAG_ITEMS + 2 * offsets::BAG_CAPACITY + 1;
    type Op = (&'static str, Box<dyn Fn(&mut SaveFile)>);
    let ops: Vec<Op> = vec![
        (
            "add",
            Box::new(|s| s.bag_items_mut().add(POTION, 9).expect("has room")),
        ),
        ("remove", Box::new(|s| s.bag_items_mut().remove(1))),
        ("set_qty", Box::new(|s| s.bag_items_mut().set_qty(0, 50))),
        ("set_id", Box::new(|s| s.bag_items_mut().set_id(2, POTION))),
        ("swap", Box::new(|s| s.bag_items_mut().swap(0, 2))),
    ];
    for (label, op) in ops {
        let mut save = bag_with_three();
        let before = save.as_bytes().to_vec();
        op(&mut save);
        let after = save.as_bytes();
        for (i, (&a, &b)) in before.iter().zip(after).enumerate() {
            if !bag_region.contains(&i) {
                assert_eq!(a, b, "op {label} changed byte 0x{i:04X} outside the bag");
            }
        }
    }
}

#[test]
fn count_clamps_to_capacity_when_corrupt() {
    let mut bytes = blank().to_bytes();
    bytes[offsets::BAG_ITEM_COUNT] = 200;
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    assert_eq!(save.bag_items().len(), offsets::BAG_CAPACITY);
}

// ---- diagnostics ----

#[test]
fn clean_lists_have_no_diagnostics() {
    let save = bag_with_three();
    assert_eq!(diagnose_item_list(save.as_bytes(), &BAG_LIST), Vec::new());
    assert_eq!(diagnose_item_list(save.as_bytes(), &PC_LIST), Vec::new());
}

#[test]
fn diagnostics_flag_count_terminator_unknown_id_and_zero_qty() {
    let mut bytes = bag_with_three().to_bytes();
    bytes[offsets::BAG_ITEM_COUNT] = 21; // > capacity
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    let diags = diagnose_item_list(save.as_bytes(), &BAG_LIST);
    assert!(
        diags.iter().any(|d| d.code == "W-ITEMS-COUNT"),
        "count out of range must be flagged: {diags:?}"
    );

    let mut bytes = bag_with_three().to_bytes();
    bytes[offsets::BAG_ITEMS + 6] = 0x00; // clobber terminator
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    let diags = diagnose_item_list(save.as_bytes(), &BAG_LIST);
    assert!(
        diags.iter().any(|d| d.code == "W-ITEMS-TERMINATOR"),
        "{diags:?}"
    );
    assert!(diags.iter().all(|d| d.severity == Severity::Warning));

    let mut bytes = bag_with_three().to_bytes();
    bytes[offsets::BAG_ITEMS + 2] = 0x00; // id 0 has no name
    bytes[offsets::BAG_ITEMS + 5] = 0; // qty 0
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    let diags = diagnose_item_list(save.as_bytes(), &BAG_LIST);
    assert!(
        diags.iter().any(|d| d.code == "W-ITEMS-UNKNOWN-ID"),
        "{diags:?}"
    );
    assert!(
        diags.iter().any(|d| d.code == "W-ITEMS-QTY-ZERO"),
        "{diags:?}"
    );
}

#[test]
fn diagnostics_spans_point_into_the_list_region() {
    let mut bytes = bag_with_three().to_bytes();
    bytes[offsets::BAG_ITEMS + 2] = 0x00;
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    let diags = diagnose_item_list(save.as_bytes(), &BAG_LIST);
    let unknown = diags
        .iter()
        .find(|d| d.code == "W-ITEMS-UNKNOWN-ID")
        .expect("present");
    assert_eq!(
        unknown.span,
        Some(offsets::BAG_ITEMS + 2..offsets::BAG_ITEMS + 3)
    );
}

// ---- mutation hardening (issue #33) ----

#[test]
fn is_empty_and_get_report_through_the_mut_view_too() {
    let mut save = blank();
    assert!(save.bag_items().is_empty());
    assert!(save.bag_items_mut().is_empty());
    save.bag_items_mut().add(POTION, 2).expect("room");
    save.bag_items_mut().add(0x0A, 95).expect("room");
    assert!(!save.bag_items().is_empty());
    let list = save.bag_items_mut();
    assert!(!list.is_empty());
    assert_eq!(list.get(0), Some((POTION, 2)));
    assert_eq!(list.get(1), Some((0x0A, 95)));
    assert_eq!(list.get(2), None, "past the end");
}

#[test]
fn spec_regions_span_count_pairs_and_terminator() {
    // count byte + 2*capacity pair bytes + terminator, as literal
    // offsets so the arithmetic itself is pinned.
    assert_eq!(BAG_LIST.region(), 0x25C9..0x25C9 + 1 + 2 * 20 + 1);
    assert_eq!(PC_LIST.region(), 0x27E6..0x27E6 + 1 + 2 * 50 + 1);
}

#[test]
fn count_equal_to_capacity_is_not_flagged() {
    let mut save = blank();
    for _ in 0..offsets::BAG_CAPACITY {
        save.bag_items_mut().add(POTION, 1).expect("room");
    }
    assert!(
        save.diagnostics().iter().all(|d| d.code != "W-ITEMS-COUNT"),
        "a full list is legal"
    );
}

#[test]
fn unknown_item_id_span_points_at_that_entry() {
    let mut save = blank();
    save.bag_items_mut().add(POTION, 1).expect("room");
    save.bag_items_mut().add(POTION, 1).expect("room");
    // Corrupt entry 1's id to 0x00 (no item name) through the raw path.
    save.set_byte(offsets::BAG_ITEMS + 2, 0x00)
        .expect("in range");
    let diags = save.diagnostics();
    let diag = diags
        .iter()
        .find(|d| d.code == "W-ITEMS-UNKNOWN-ID")
        .expect("unknown id flagged");
    assert_eq!(
        diag.span,
        Some(offsets::BAG_ITEMS + 2..offsets::BAG_ITEMS + 3),
        "span names entry 1's id byte"
    );
}

#[test]
fn zero_quantity_reads_the_qty_byte_not_the_id() {
    // Entry 1: valid id, quantity 0 — W-ITEMS-QTY-ZERO must fire (the
    // check reads the qty byte at `at + 1`, not the id byte).
    let mut save = blank();
    save.bag_items_mut().add(POTION, 1).expect("room");
    save.bag_items_mut().add(POTION, 1).expect("room");
    save.set_byte(offsets::BAG_ITEMS + 3, 0).expect("in range");
    let diags = save.diagnostics();
    let diag = diags
        .iter()
        .find(|d| d.code == "W-ITEMS-QTY-ZERO")
        .expect("zero quantity flagged");
    assert_eq!(
        diag.span,
        Some(offsets::BAG_ITEMS + 3..offsets::BAG_ITEMS + 4),
        "span names entry 1's quantity byte"
    );
}
