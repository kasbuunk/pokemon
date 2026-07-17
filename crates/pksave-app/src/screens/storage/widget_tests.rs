//! Widget-level tests (issue #32): drive the storage screen with real
//! pointer events through `egui_kittest` and assert on the resulting
//! save state — the click-target wiring the logic-level tests never
//! exercise.

use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use pksave::gen1::save::GameVariant;

use super::*;

fn storage_harness(size: egui::Vec2) -> Harness<'static, (Doc, StorageState)> {
    Harness::builder().with_size(size).build_ui_state(
        |ui, state: &mut (Doc, StorageState)| super::ui(ui, &mut state.0, &mut state.1),
        (
            Doc::new_empty(GameVariant::RedBlue),
            StorageState::default(),
        ),
    )
}

#[test]
fn action_row_add_deposit_withdraw_click_through() {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));

    // Add a level-5 mon to the party via the action row.
    harness.get_by_label("✚ party").click();
    harness.run();
    {
        let (doc, state) = harness.state();
        assert_eq!(doc.save.party().len(), 1);
        assert_eq!(
            state.selected,
            Some(SlotId::Party(0)),
            "add selects the new mon"
        );
    }

    // Deposit it into the viewed box; the selection follows.
    harness.get_by_label("Deposit ➡ this box").click();
    harness.run();
    {
        let (doc, state) = harness.state();
        assert_eq!(doc.save.party().len(), 0);
        assert_eq!(doc.save.box_(0).len(), 1);
        assert_eq!(state.selected, Some(SlotId::Box { box_n: 0, index: 0 }));
    }

    // And withdraw it back.
    harness.get_by_label("Withdraw ➡ party").click();
    harness.run();
    {
        let (doc, state) = harness.state();
        assert_eq!(doc.save.party().len(), 1);
        assert_eq!(doc.save.box_(0).len(), 0);
        assert_eq!(state.selected, Some(SlotId::Party(0)));
    }
}

#[test]
fn box_tab_click_switches_the_viewed_box() {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));

    harness.get_by_label("Box 2 (0)").click();
    harness.run();
    assert_eq!(harness.state().1.tab, 1);

    harness.get_by_label("✚ this box").click();
    harness.run();
    let (doc, state) = harness.state();
    assert_eq!(doc.save.box_(1).len(), 1, "add lands in the viewed box");
    assert_eq!(state.selected, Some(SlotId::Box { box_n: 1, index: 0 }));
}

#[test]
fn narrow_viewport_keeps_the_grid_width() {
    // The issue #38 repro size: the detail panel must leave the grid
    // column its minimum width.
    let harness = storage_harness(egui::vec2(640.0, 400.0));
    let panel = egui::PanelState::load(&harness.ctx, egui::Id::new("storage_detail"))
        .expect("side-by-side detail panel at 640pt");
    assert!(
        640.0 - panel.outer_rect.width() >= CENTER_MIN_WIDTH - 0.5,
        "detail panel must leave the grid its minimum width"
    );
}

#[test]
fn narrow_viewport_keeps_the_action_row_reachable() {
    // At the minimum center width the two action rows must stay inside
    // the center column as real click targets (a single unwrapped row
    // used to overflow under the detail panel). 480pt of height: below
    // ~440 the center column overflows vertically (tracked separately).
    let mut harness = storage_harness(egui::vec2(640.0, 480.0));
    harness.get_by_label("✚ party").click();
    harness.run();
    assert_eq!(harness.state().0.save.party().len(), 1);
    harness.get_by_label("✚ this box").click();
    harness.run();
    assert_eq!(harness.state().0.save.box_(0).len(), 1);
}
#[test]
fn dbg_box_tab() {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));
    harness.get_by_label("Box 2 (0)").click();
    harness.run();
    let node = harness.get_by_label("✚ this box");
    println!("button rect: {:?}", node.rect());
    let node2 = harness.get_by_label("✚ party");
    println!("party button rect: {:?}", node2.rect());
}
