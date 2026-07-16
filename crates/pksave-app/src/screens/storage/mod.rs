//! The unified Pokémon storage screen: party strip + daycare, box tabs,
//! a full-height box grid and a resizable detail panel. Pokémon move by
//! drag-and-drop (party ⇄ box, box ⇄ box, daycare), context menu or the
//! action-row buttons; all three funnel through the same validated
//! transfer semantics in [`transfer`].

pub mod detail;
pub mod editor;
pub mod slots;
pub mod transfer;

use pksave::gen1::offsets;
use pksave::gen1::pokemon::{MonView, PartyMon};

use crate::app::Doc;
use crate::widgets;
use slots::{Action, SlotId, SlotInfo};
use transfer::DropTarget;

/// How long a refused-drop notice stays visible.
const NOTICE_SECONDS: f64 = 4.0;

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

    // ---- right: detail editor (the screen's one scroll container) ----
    let mut touched = false;
    let mut touched_boxes: Vec<usize> = Vec::new();
    egui::Panel::right("storage_detail")
        .resizable(true)
        .default_size(460.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("storage_detail_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if detail::ui(ui, doc, state.selected) {
                        touched = true;
                        if let Some(SlotId::Box { box_n, .. }) = state.selected {
                            touched_boxes.push(box_n);
                        }
                    }
                });
        });

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
    ui.horizontal(|ui| {
        let party_full = doc.save.party().len() >= offsets::PARTY_CAPACITY;
        let box_full = doc.save.box_(state.tab).len() >= offsets::MONS_PER_BOX;

        match state.selected {
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
                ui.weak("Select a slot, or drag Pokémon between the party and boxes.");
            }
        }

        ui.separator();
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
