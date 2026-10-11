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

type StorageHarness = Harness<'static, (Doc, StorageState)>;

fn pointer_button(
    harness: &mut StorageHarness,
    pos: egui::Pos2,
    button: egui::PointerButton,
    pressed: bool,
) {
    harness.event(egui::Event::PointerButton {
        pos,
        button,
        pressed,
        modifiers: egui::Modifiers::NONE,
    });
    harness.run();
}

/// A real pointer click, press and release on separate frames like any
/// physical click (an accesskit click would bypass the hit test).
fn pointer_click(harness: &mut StorageHarness, pos: egui::Pos2, button: egui::PointerButton) {
    harness.event(egui::Event::PointerMoved(pos));
    harness.run();
    pointer_button(harness, pos, button, true);
    pointer_button(harness, pos, button, false);
}

/// Center of party cell `i`: the cells sit in a row right under the
/// "Party n / 6" heading.
fn party_cell(harness: &StorageHarness, i: usize) -> egui::Pos2 {
    let heading = harness
        .query_all_by_value(&format!(
            "Party {} / 6",
            harness.state().0.save.party().len()
        ))
        .next()
        .expect("party heading")
        .rect();
    // 96pt is the minimum cell width, which the 1100pt harness yields.
    let pitch = 96.0 + harness.ctx.global_style().spacing.item_spacing.x;
    heading.left_bottom() + egui::vec2(48.0 + pitch * i as f32, 24.0)
}

/// A storage screen with two party mons; the second one is selected.
fn two_mon_harness() -> StorageHarness {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));
    for _ in 0..2 {
        harness.get_by_label("✚ party").click();
        harness.run();
    }
    assert_eq!(harness.state().1.selected, Some(SlotId::Party(1)));
    harness
}

#[test]
fn clicking_an_occupied_cell_selects_it() {
    let mut harness = two_mon_harness();
    let cell = party_cell(&harness, 0);
    pointer_click(&mut harness, cell, egui::PointerButton::Primary);
    assert_eq!(harness.state().1.selected, Some(SlotId::Party(0)));
}

#[test]
fn right_clicking_an_occupied_cell_opens_its_menu() {
    let mut harness = two_mon_harness();
    assert_eq!(harness.query_all_by_label("Move to daycare").count(), 0);
    let cell = party_cell(&harness, 0);
    pointer_click(&mut harness, cell, egui::PointerButton::Secondary);
    harness.run();
    assert_eq!(harness.query_all_by_label("Move to daycare").count(), 1);
}

#[test]
fn dragging_a_cell_onto_another_still_reorders_the_party() {
    let mut harness = two_mon_harness();
    harness.state_mut().0.save.party_mut().set_species(1, 0x16);
    let (from, to) = (party_cell(&harness, 1), party_cell(&harness, 0));
    harness.event(egui::Event::PointerMoved(from));
    harness.run();
    pointer_button(&mut harness, from, egui::PointerButton::Primary, true);
    for step in 1..=4 {
        harness.event(egui::Event::PointerMoved(from.lerp(to, step as f32 / 4.0)));
        harness.run();
    }
    pointer_button(&mut harness, to, egui::PointerButton::Primary, false);
    harness.run();
    use pksave::gen1::pokemon::MonView;
    assert_eq!(
        harness.state().0.save.party().mon(0).species(),
        0x16,
        "dragged mon lands in slot 1"
    );
}

#[test]
fn picking_a_move_in_the_editor_sets_it_with_full_pp() {
    let mut harness = storage_harness(egui::vec2(1100.0, 740.0));
    harness.get_by_label("✚ party").click();
    harness.run();

    let combo = harness
        .query_all_by_value("— none —")
        .next()
        .expect("first move slot")
        .rect()
        .center();
    pointer_click(&mut harness, combo, egui::PointerButton::Primary);
    harness.event(egui::Event::Text("hyper".to_owned()));
    harness.run();
    let option = harness
        .query_all_by_label("HYPER BEAM (5 PP)")
        .next()
        .expect("filtered option")
        .rect()
        .center();
    pointer_click(&mut harness, option, egui::PointerButton::Primary);

    use pksave::gen1::pokemon::MonView;
    let (doc, _) = harness.state();
    let party = doc.save.party();
    assert_eq!(party.mon(0).moves()[0], 0x3F);
    assert_eq!(party.mon(0).pp()[0], 5);
}
