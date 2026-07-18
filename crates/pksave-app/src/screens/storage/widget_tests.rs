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

/// Press and release a pointer button on separate frames, the way real
/// pointer input arrives (a same-frame press+release is never treated
/// as a drag by egui, so it cannot regress the drag-source wiring).
fn click_across_frames(
    harness: &mut Harness<'_, (Doc, StorageState)>,
    pos: egui::Pos2,
    button: egui::PointerButton,
) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.step();
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: egui::Modifiers::default(),
        });
        harness.step();
    }
    harness.run();
}

#[test]
fn left_click_selects_a_mon_for_editing() {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));
    harness.get_by_label("✚ party").click();
    harness.run();
    harness.state_mut().1.selected = None;
    harness.run();

    let pos = harness.get_by_label_contains("BULBASAUR").rect().center();
    click_across_frames(&mut harness, pos, egui::PointerButton::Primary);

    assert_eq!(
        harness.state().1.selected,
        Some(SlotId::Party(0)),
        "clicking an occupied slot must select it for the detail editor"
    );
}

#[test]
fn right_click_opens_the_context_menu_and_selects() {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));
    harness.get_by_label("✚ party").click();
    harness.run();
    harness.state_mut().1.selected = None;
    harness.run();

    let pos = harness.get_by_label_contains("BULBASAUR").rect().center();
    click_across_frames(&mut harness, pos, egui::PointerButton::Secondary);

    assert!(
        harness.query_by_label("✏ Edit").is_some(),
        "right-clicking an occupied slot must open its context menu"
    );
    assert_eq!(
        harness.state().1.selected,
        Some(SlotId::Party(0)),
        "right-clicking a slot must also select it, so the editor opens"
    );
}

#[test]
fn drag_and_drop_still_deposits_a_mon() {
    // The cells now sense click+drag themselves (no separate drag-only
    // handle); a real multi-frame drag must still move the mon.
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));
    harness.get_by_label("✚ party").click();
    harness.run();

    // Target the first box cell: at this viewport the grid overflows
    // under the detail panel and clipped cells never see the pointer
    // (a pre-existing layout bug, tracked separately), so pick a cell
    // that is fully visible.
    let from = harness.get_by_label_contains("BULBASAUR").rect().center();
    let to = harness.get_by_label("Empty box 1 slot 1").rect().center();

    harness.event(egui::Event::PointerMoved(from));
    harness.step();
    harness.event(egui::Event::PointerButton {
        pos: from,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    for t in [0.25, 0.5, 0.75, 1.0] {
        harness.event(egui::Event::PointerMoved(from.lerp(to, t)));
        harness.step();
    }
    harness.event(egui::Event::PointerButton {
        pos: to,
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    });
    harness.step();
    harness.run();

    let (doc, _) = harness.state();
    assert_eq!(doc.save.party().len(), 0, "drag must leave the party");
    assert_eq!(doc.save.box_(0).len(), 1, "drop must land in the box");
}
