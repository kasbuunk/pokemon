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
            Box::new(|s| s.bag_items_mut().add(POTION, 9).unwrap()),
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
