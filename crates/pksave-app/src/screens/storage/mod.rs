//! The unified Pokémon storage screen: party strip + daycare, box tabs,
//! a full-height box grid and a resizable detail panel. Pokémon move by
//! drag-and-drop (party ⇄ box, box ⇄ box, daycare), context menu or the
//! action-row buttons; all three funnel through the same validated
//! transfer semantics in [`transfer`].

pub mod detail;
pub mod editor;
pub mod slots;
pub mod transfer;
#[cfg(test)]
mod widget_tests;

use pksave::gen1::offsets;
use pksave::gen1::pokemon::{MonView, PartyMon};

use crate::app::Doc;
use crate::widgets;
use slots::{Action, SlotId, SlotInfo};
use transfer::DropTarget;

/// How long a refused-drop notice stays visible.
const NOTICE_SECONDS: f64 = 4.0;

/// The center column (party strip + box grid) never gets squeezed below
/// this by the detail panel; the panel's width range is derived from it
/// every frame, so window shrinking takes space from the panel first.
const CENTER_MIN_WIDTH: f32 = 380.0;
/// Below this the side-by-side detail panel stops being useful; when
/// even that doesn't fit next to [`CENTER_MIN_WIDTH`], the detail
/// stacks under the grid instead.
const DETAIL_MIN_WIDTH: f32 = 240.0;
/// Preferred detail-panel width when there is room for it.
const DETAIL_DEFAULT_WIDTH: f32 = 460.0;

pub struct StorageState {
    pub selected: Option<SlotId>,
    /// The viewed box.
    pub tab: usize,
    /// Internal species index picked in the "add" row.
    pub add_species: u8,
    /// A transient refusal/notice banner: message + expiry time.
    pub notice: Option<(String, f64)>,
}

impl Default for StorageState {
    fn default() -> Self {
        StorageState {
            selected: None,
            tab: 0,
            add_species: 0x99, // Bulbasaur
            notice: None,
        }
    }
}

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut StorageState) {
    state.tab = state.tab.min(offsets::NUM_BOXES - 1);
    clamp_selection(doc, state);

    let mut queued: Vec<Action> = Vec::new();

    // ---- detail editor (the screen's one scroll container): beside the
    // grid when it fits, stacked under it at very narrow viewports ----
    let mut touched = false;
    let mut touched_boxes: Vec<usize> = Vec::new();
    let selected = state.selected;
    let detail_contents = |ui: &mut egui::Ui| {
        egui::ScrollArea::vertical()
            .id_salt("storage_detail_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if detail::ui(ui, doc, selected) {
                    touched = true;
                    if let Some(SlotId::Box { box_n, .. }) = selected {
                        touched_boxes.push(box_n);
                    }
                }
            });
    };
    let avail = ui.available_width();
    if avail < CENTER_MIN_WIDTH + DETAIL_MIN_WIDTH {
        let avail_h = ui.available_height();
        egui::Panel::bottom("storage_detail_stacked")
            .resizable(true)
            .max_size(avail_h * 0.6)
            .default_size(avail_h * 0.4)
            .show(ui, detail_contents);
    } else {
        let max_w = avail - CENTER_MIN_WIDTH;
        egui::Panel::right("storage_detail")
            .resizable(true)
            .min_size(DETAIL_MIN_WIDTH.min(max_w))
            .max_size(max_w)
            .default_size(DETAIL_DEFAULT_WIDTH.min(max_w))
            .show(ui, detail_contents);
    }

    // ---- center: header, party strip, tabs, box grid, action row ----
    egui::CentralPanel::default().show(ui, |ui| {
        header(ui, doc, state, &mut touched);
        ui.add_space(6.0);
        party_strip(ui, doc, state, &mut queued);
        ui.add_space(8.0);
        box_tabs(ui, doc, state, &mut queued);
        ui.add_space(4.0);

        // Reserve room for the action row below the grid.
        let action_row_reserve = 64.0;
        let grid_height = (ui.available_height() - action_row_reserve).max(120.0);
        box_grid(ui, doc, state, grid_height, &mut queued);
        ui.add_space(6.0);
        action_row(ui, doc, state, &mut queued);
    });

    // ---- apply queued mutations after everything is drawn ----
    for action in queued {
        apply_action(doc, state, action, &mut touched, &mut touched_boxes);
    }

    if touched {
        // A UI-initiated mutation of the current box changed the live
        // working copy: flush it to its bank slot right away (as an
        // in-game box switch would), so the app never raises
        // W-BOX-STALE about its own action. Files that were already
        // stale on load keep their warning (and the manual sync button)
        // until this box is edited or synced.
        if touched_boxes.iter().any(|&n| doc.save.box_is_live(n)) {
            doc.save.sync_current_box_to_bank();
        }
        doc.touch();
    }
}

/// Drop a selection that no longer points at an occupied slot.
fn clamp_selection(doc: &Doc, state: &mut StorageState) {
    let valid = match state.selected {
        Some(SlotId::Party(i)) => i < doc.save.party().len(),
        Some(SlotId::Box { box_n, index }) => {
            box_n < offsets::NUM_BOXES && index < doc.save.box_(box_n).len()
        }
        Some(SlotId::Daycare) => doc.save.daycare().is_some(),
        None => false,
    };
    if !valid {
        state.selected = None;
    }
}

fn header(ui: &mut egui::Ui, doc: &mut Doc, state: &mut StorageState, touched: &mut bool) {
    ui.heading("Pokémon");
    if !doc.save.boxes_initialized() {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "⚠ Boxes-initialized flag is clear: the game will wipe all boxes on load \
             (see W-BOX-INIT in Overview).",
        );
    }
    if doc.save.box_is_live(state.tab) && doc.diagnostics.iter().any(|d| d.code == "W-BOX-STALE") {
        ui.horizontal(|ui| {
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "⚠ The bank copy of the current box is stale (differs from the working copy).",
            );
            if ui
                .button("Sync working copy ➡ bank")
                .on_hover_text(
                    "Copy the live working copy into its bank slot, as a box switch does",
                )
                .clicked()
            {
                doc.save.sync_current_box_to_bank();
                *touched = true;
            }
        });
    }
    // Transient refused-drop notice. A fresh notice is stored with the
    // f64::MAX sentinel (the apply step has no clock); stamp its real
    // expiry on first display.
    if let Some((message, expires_at)) = &mut state.notice {
        let now = ui.input(|i| i.time);
        if *expires_at == f64::MAX {
            *expires_at = now + NOTICE_SECONDS;
        }
        if now < *expires_at {
            let message = message.clone();
            ui.colored_label(ui.visuals().warn_fg_color, message);
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(250));
        } else {
            state.notice = None;
        }
    }
}

/// Copy a slot's display info out of the save.
fn slot_info(doc: &Doc, slot: SlotId) -> Option<SlotInfo> {
    let (nickname, species, level, exp_level) = match slot {
        SlotId::Party(i) => {
            let party = doc.save.party();
            if i >= party.len() {
                return None;
            }
            let mon = party.mon(i);
            (
                party.nickname(i),
                mon.species(),
                mon.level(),
                mon.level_from_exp(),
            )
        }
        SlotId::Box { box_n, index } => {
            let view = doc.save.box_(box_n);
            if index >= view.len() {
                return None;
            }
            let mon = view.mon(index);
            (
                view.nickname(index),
                mon.species(),
                mon.box_level(),
                mon.level_from_exp(),
            )
        }
        SlotId::Daycare => {
            let view = doc.save.daycare()?;
            let mon = view.mon();
            (
                view.nickname(),
                mon.species(),
                mon.box_level(),
                mon.level_from_exp(),
            )
        }
    };
    Some(SlotInfo {
        nickname,
        species,
        level,
        level_from_exp: (exp_level != level).then_some(exp_level),
    })
}

/// The 6 party cells plus the daycare cell, sized to the full width.
fn party_strip(ui: &mut egui::Ui, doc: &Doc, state: &mut StorageState, queued: &mut Vec<Action>) {
    let party_len = doc.save.party().len();
    ui.horizontal(|ui| {
        ui.strong(format!("Party {party_len} / {}", offsets::PARTY_CAPACITY));
        ui.add_space(8.0);
        ui.weak("· drag to reorder or move · right-click for actions");
    });
    ui.add_space(2.0);

    let spacing = ui.spacing().item_spacing.x;
    // 6 party cells + a separator gap + the daycare cell.
    let cells = (offsets::PARTY_CAPACITY + 1) as f32;
    let extra_gap = 12.0;
    let slot_w =
        ((ui.available_width() - spacing * (cells - 1.0) - extra_gap) / cells).clamp(96.0, 190.0);
    let size = egui::vec2(slot_w, 44.0);

    ui.horizontal(|ui| {
        for i in 0..offsets::PARTY_CAPACITY {
            let slot = SlotId::Party(i);
            slots::slot_cell(
                ui,
                doc,
                slot,
                size,
                slot_info(doc, slot),
                &mut state.selected,
                queued,
            );
        }
        ui.add_space(extra_gap);
        ui.vertical(|ui| {
            ui.set_width(slot_w);
            slots::slot_cell(
                ui,
                doc,
                SlotId::Daycare,
                egui::vec2(
                    slot_w,
                    44.0 - ui.text_style_height(&egui::TextStyle::Small) - 2.0,
                ),
                slot_info(doc, SlotId::Daycare),
                &mut state.selected,
                queued,
            );
            ui.small("Daycare");
        });
    });
}

/// The 12 box tabs; every tab is also a drop target ("move to box N").
fn box_tabs(ui: &mut egui::Ui, doc: &Doc, state: &mut StorageState, queued: &mut Vec<Action>) {
    let current = usize::from(doc.save.current_box_number());
    ui.horizontal_wrapped(|ui| {
        for n in 0..offsets::NUM_BOXES {
            let star = if n == current { " ★" } else { "" };
            let count = doc.save.box_(n).len();
            let label = format!("Box {}{star} ({count})", n + 1);
            let response = ui.selectable_label(state.tab == n, label);
            if response.clicked() && state.tab != n {
                state.tab = n;
            }
            slots::handle_drop_target(ui, &response, doc, DropTarget::BoxTab(n), queued);
        }
    });
    if doc.save.box_is_live(state.tab) {
        ui.weak(
            "This is the current box: edits go to the live working copy, exactly as the \
             game reads it.",
        );
    }
}

/// The 4×5 grid of the viewed box, filling the available height. All 20
/// cells render; trailing empties accept drops as "append".
fn box_grid(
    ui: &mut egui::Ui,
    doc: &Doc,
    state: &mut StorageState,
    height: f32,
    queued: &mut Vec<Action>,
) {
    const COLS: usize = 4;
    const ROWS: usize = 5;
    debug_assert_eq!(COLS * ROWS, offsets::MONS_PER_BOX);

    let spacing = ui.spacing().item_spacing;
    let slot_w = (ui.available_width() - spacing.x * (COLS as f32 - 1.0)) / COLS as f32;
    let slot_h = ((height - spacing.y * (ROWS as f32 - 1.0)) / ROWS as f32).clamp(40.0, 84.0);
    let size = egui::vec2(slot_w, slot_h);

    let n = state.tab;
    for row in 0..ROWS {
        ui.horizontal(|ui| {
            for col in 0..COLS {
                let index = row * COLS + col;
                let slot = SlotId::Box { box_n: n, index };
                slots::slot_cell(
                    ui,
                    doc,
                    slot,
                    size,
                    slot_info(doc, slot),
                    &mut state.selected,
                    queued,
                );
            }
        });
    }
}

/// Button fallbacks for the selected slot plus the "add Pokémon" group —
/// everything the drags do, reachable without a mouse drag.
fn action_row(ui: &mut egui::Ui, doc: &Doc, state: &mut StorageState, queued: &mut Vec<Action>) {
    let party_full = doc.save.party().len() >= offsets::PARTY_CAPACITY;
    let box_full = doc.save.box_(state.tab).len() >= offsets::MONS_PER_BOX;

    // Two deliberate rows, each of which fits the center column at its
    // minimum width: a single row overflows under the detail panel at
    // narrow sizes (a widget that merely *starts* inside a wrapped row
    // still sticks out past the edge).
    ui.horizontal_wrapped(|ui| match state.selected {
        Some(slot @ SlotId::Party(_)) => {
            if ui
                .add_enabled(!box_full, egui::Button::new("Deposit ➡ this box"))
                .clicked()
            {
                queued.push(Action::Transfer(slot, DropTarget::BoxTab(state.tab)));
            }
            if ui
                .add_enabled(
                    doc.save.daycare().is_none(),
                    egui::Button::new("Move ➡ daycare"),
                )
                .clicked()
            {
                queued.push(Action::Transfer(slot, DropTarget::Slot(SlotId::Daycare)));
            }
            if ui.button("🗑 Delete").clicked() {
                queued.push(Action::Delete(slot));
            }
        }
        Some(slot @ SlotId::Box { .. }) => {
            if ui
                .add_enabled(!party_full, egui::Button::new("Withdraw ➡ party"))
                .clicked()
            {
                queued.push(Action::Transfer(slot, DropTarget::Party));
            }
            if ui.button("🗑 Delete").clicked() {
                queued.push(Action::Delete(slot));
            }
        }
        Some(SlotId::Daycare) => {
            if ui
                .add_enabled(!party_full, egui::Button::new("Take ➡ party"))
                .clicked()
            {
                queued.push(Action::Transfer(SlotId::Daycare, DropTarget::Party));
            }
            if ui.button("🗑 Clear daycare").clicked() {
                queued.push(Action::Delete(SlotId::Daycare));
            }
        }
        None => {
            ui.weak("Select a slot or drag to move Pokémon.");
        }
    });
    ui.horizontal_wrapped(|ui| {
        if let Some(picked) = widgets::species_combo(ui, "add_species", state.add_species) {
            state.add_species = picked;
        }
        if ui
            .add_enabled(!party_full, egui::Button::new("✚ party"))
            .on_hover_text("Add at level 5 with zero DVs and no moves — set moves in the editor")
            .clicked()
        {
            queued.push(Action::AddToParty(state.add_species));
        }
        if ui
            .add_enabled(!box_full, egui::Button::new("✚ this box"))
            .on_hover_text("Add at level 5 with zero DVs and no moves — set moves in the editor")
            .clicked()
        {
            queued.push(Action::AddToBox(state.add_species, state.tab));
        }
    });
}

/// Apply one queued action, updating selection, dirty state and the
/// notice banner.
fn apply_action(
    doc: &mut Doc,
    state: &mut StorageState,
    action: Action,
    touched: &mut bool,
    touched_boxes: &mut Vec<usize>,
) {
    let notice = |state: &mut StorageState, message: String| {
        // Expiry is set on the next frame's clock via egui time; store
        // f64::MAX-safe sentinel handled in header().
        state.notice = Some((message, f64::MAX));
    };
    match action {
        Action::Drop(from, to) | Action::Transfer(from, to) => {
            match transfer::validate_drop(&doc.save, from, to) {
                Ok(transfer::DropAction::NoOp) => {}
                Ok(action) => match transfer::perform_drop(&mut doc.save, action) {
                    Ok(new_slot) => {
                        if let Some(slot) = new_slot {
                            state.selected = Some(slot);
                        }
                        *touched = true;
                        touched_boxes.extend(transfer::boxes_touched(action));
                    }
                    Err(message) => notice(state, message),
                },
                Err(e) => notice(state, e.message()),
            }
        }
        Action::Delete(slot) => match slot {
            SlotId::Party(i) => {
                if i < doc.save.party().len() {
                    doc.save.party_mut().remove(i);
                    *touched = true;
                }
            }
            SlotId::Box { box_n, index } => {
                if index < doc.save.box_(box_n).len() {
                    doc.save.box_mut(box_n).remove(index);
                    *touched = true;
                    touched_boxes.push(box_n);
                }
            }
            SlotId::Daycare => {
                let _ = doc.save.set_daycare(None);
                *touched = true;
            }
        },
        Action::AddToParty(species) => {
            if detail::add_mon_to_party(doc, species) {
                state.selected = Some(SlotId::Party(doc.save.party().len().saturating_sub(1)));
                *touched = true;
            }
        }
        Action::AddToBox(species, n) => {
            if detail::add_mon_to_box(doc, species, n) {
                state.selected = Some(SlotId::Box {
                    box_n: n,
                    index: doc.save.box_(n).len().saturating_sub(1),
                });
                *touched = true;
                touched_boxes.push(n);
            }
        }
    }
}

#[cfg(test)]
mod layout_tests {
    //! Headless layout regressions (issue #38): the detail panel must
    //! never squeeze the center column below a working grid width.

    use pksave::gen1::save::GameVariant;

    use super::*;

    struct Layout {
        /// `available_width()` handed to the screen by its parent.
        avail: f32,
        /// Width of the side-by-side detail panel, if it was shown.
        side_panel_w: Option<f32>,
        /// Height of the stacked (bottom) detail panel, if it was shown.
        stacked_panel_h: Option<f32>,
    }

    /// Render the storage screen for a few frames at the given viewport
    /// size in a headless egui context and report the resulting layout.
    fn layout_at(size: egui::Vec2) -> Layout {
        let ctx = egui::Context::default();
        let mut doc = Doc::new_empty(GameVariant::RedBlue);
        let mut state = StorageState::default();
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            ..Default::default()
        };
        let mut avail = 0.0;
        for _ in 0..3 {
            let _ = ctx.run_ui(input.clone(), |ui| {
                avail = ui.available_width();
                super::ui(ui, &mut doc, &mut state);
            });
        }
        let panel_size =
            |id: &str| egui::PanelState::load(&ctx, egui::Id::new(id)).map(|s| s.outer_rect.size());
        Layout {
            avail,
            side_panel_w: panel_size("storage_detail").map(|s| s.x),
            stacked_panel_h: panel_size("storage_detail_stacked").map(|s| s.y),
        }
    }

    /// Whatever the mode, the center column keeps a usable width.
    fn assert_center_keeps_width(layout: &Layout, size: egui::Vec2) {
        match (layout.side_panel_w, layout.stacked_panel_h) {
            (Some(w), None) => {
                let center = layout.avail - w;
                assert!(
                    center >= CENTER_MIN_WIDTH - 0.5,
                    "detail panel ({w}px) squeezes the center to {center}px at {size:?}"
                );
            }
            (None, Some(h)) => {
                // Stacked: the grid gets the full width; the detail must
                // still leave the grid its minimum height.
                assert!(
                    h <= size.y * 0.65,
                    "stacked detail ({h}px) swallows the grid at {size:?}"
                );
            }
            other => panic!("expected exactly one detail panel, got {other:?} at {size:?}"),
        }
    }

    #[test]
    fn wide_viewport_keeps_default_panel_width() {
        let layout = layout_at(egui::vec2(1100.0, 740.0));
        let w = layout.side_panel_w.expect("side-by-side at 1100pt");
        assert!(
            (w - DETAIL_DEFAULT_WIDTH).abs() < 1.0,
            "default width regressed: {w}"
        );
        assert_center_keeps_width(&layout, egui::vec2(1100.0, 740.0));
    }

    #[test]
    fn narrow_640x400_keeps_grid_width() {
        // The issue #38 repro: at 640×400 the panel's old fixed 460pt
        // default collapsed the grid to a ~20pt sliver.
        let layout = layout_at(egui::vec2(640.0, 400.0));
        assert_center_keeps_width(&layout, egui::vec2(640.0, 400.0));
    }

    #[test]
    fn very_narrow_viewport_stacks_the_detail() {
        let layout = layout_at(egui::vec2(500.0, 400.0));
        assert!(
            layout.side_panel_w.is_none() && layout.stacked_panel_h.is_some(),
            "expected the stacked layout below {}pt",
            CENTER_MIN_WIDTH + DETAIL_MIN_WIDTH
        );
        assert_center_keeps_width(&layout, egui::vec2(500.0, 400.0));
    }
}
