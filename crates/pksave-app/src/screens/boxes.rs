//! Boxes screen: tab strip over the 12 PC boxes, slot table, a detail
//! editor (shared with the party screen) and party ⇄ box transfers.

use pksave::gen1::data::{BASE_STATS, INDEX_TO_DEX};
use pksave::gen1::offsets;

use crate::app::Doc;
use crate::screens::party::{apply_edits, common_editor, snapshot, MonSnapshot};
use crate::widgets;

#[derive(Default)]
pub struct BoxesState {
    pub tab: usize,
    pub selected: Option<usize>,
}

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut BoxesState) {
    ui.heading("Boxes");
    ui.add_space(4.0);
    let mut touched = false;

    let current = usize::from(doc.save.current_box_number());

    // ---- tab strip ----
    ui.horizontal_wrapped(|ui| {
        for n in 0..offsets::NUM_BOXES {
            let star = if n == current { " ★" } else { "" };
            let label = format!("Box {}{star}", n + 1);
            if ui.selectable_label(state.tab == n, label).clicked() && state.tab != n {
                state.tab = n;
                state.selected = None;
            }
        }
    });
    if !doc.save.boxes_initialized() {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "⚠ Boxes-initialized flag is clear: the game will wipe all boxes on load \
             (see W-BOX-INIT in Overview).",
        );
    }

    let n = state.tab.min(offsets::NUM_BOXES - 1);
    let is_live = doc.save.box_is_live(n);
    if is_live {
        ui.weak(
            "This is the current box: edits go to the live working copy, exactly as the \
             game reads it.",
        );
        if doc.diagnostics.iter().any(|d| d.code == "W-BOX-STALE") {
            ui.horizontal(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    "⚠ The bank copy of this box is stale (differs from the working copy).",
                );
                if ui
                    .button("Sync working copy → bank")
                    .on_hover_text(
                        "Copy the live working copy into its bank slot, as a box \
                                    switch does",
                    )
                    .clicked()
                {
                    doc.save.sync_current_box_to_bank();
                    touched = true;
                }
            });
        }
    }

    ui.add_space(6.0);
    let box_len = doc.save.box_(n).len();
    if let Some(sel) = state.selected {
        if sel >= box_len {
            state.selected = None;
        }
    }

    ui.horizontal(|ui| {
        // ---- slot table ----
        ui.vertical(|ui| {
            ui.set_width(260.0);
            ui.strong(format!("{box_len} / {} mons", offsets::MONS_PER_BOX));
            egui::ScrollArea::vertical()
                .id_salt("box_slots")
                .max_height(360.0)
                .show(ui, |ui| {
                    for i in 0..box_len {
                        let (nick, label) = {
                            let view = doc.save.box_(n);
                            let mon = view.mon(i);
                            (
                                view.nickname(i),
                                format!(
                                    "Lv.{} {}",
                                    mon.box_level(),
                                    widgets::species_label(mon.species())
                                ),
                            )
                        };
                        if ui
                            .selectable_label(
                                state.selected == Some(i),
                                format!("{}. {nick} — {label}", i + 1),
                            )
                            .clicked()
                        {
                            state.selected = Some(i);
                        }
                    }
                    if box_len == 0 {
                        ui.weak("(empty box)");
                    }
                });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let has_sel = state.selected.is_some();
                let party_full = doc.save.party().len() >= offsets::PARTY_CAPACITY;
                if ui
                    .add_enabled(
                        has_sel && !party_full,
                        egui::Button::new("Withdraw → party"),
                    )
                    .clicked()
                {
                    if let Some(i) = state.selected {
                        if doc.save.withdraw(n, i).is_ok() {
                            state.selected = None;
                            touched = true;
                        }
                    }
                }
                if ui
                    .add_enabled(has_sel, egui::Button::new("🗑 Remove"))
                    .clicked()
                {
                    if let Some(i) = state.selected {
                        doc.save.box_mut(n).remove(i);
                        state.selected = None;
                        touched = true;
                    }
                }
            });

            // ---- deposit from party ----
            ui.add_space(8.0);
            ui.group(|ui| {
                ui.strong("Deposit from party");
                let party_len = doc.save.party().len();
                if party_len == 0 {
                    ui.weak("(party is empty)");
                }
                for p in 0..party_len {
                    let label = {
                        let party = doc.save.party();
                        format!("{} (Lv.{})", party.nickname(p), party.mon(p).level())
                    };
                    ui.horizontal(|ui| {
                        ui.label(&label);
                        let box_full = doc.save.box_(n).len() >= offsets::MONS_PER_BOX;
                        if ui
                            .add_enabled(!box_full, egui::Button::new("Deposit"))
                            .clicked()
                            && doc.save.deposit(p, n).is_ok()
                        {
                            touched = true;
                        }
                    });
                }
            });
        });

        ui.separator();

        // ---- detail editor ----
        ui.vertical(|ui| {
            let Some(i) = state.selected else {
                ui.weak("Select a mon to edit it.");
                return;
            };
            egui::ScrollArea::vertical()
                .id_salt("box_detail")
                .show(ui, |ui| {
                    touched |= detail_editor(ui, doc, n, i);
                });
        });
    });

    if touched {
        doc.touch();
    }
}

fn detail_editor(ui: &mut egui::Ui, doc: &mut Doc, n: usize, i: usize) -> bool {
    let mut touched = false;
    let snap: MonSnapshot = {
        let view = doc.save.box_(n);
        let mon = view.mon(i);
        snapshot!(mon)
    };
    let (nickname, ot_name) = {
        let view = doc.save.box_(n);
        (view.nickname(i), view.ot_name(i))
    };

    ui.horizontal(|ui| {
        ui.label("Species:");
        if let Some(internal) = widgets::species_combo(ui, ("box_species", n, i), snap.species) {
            let new_dex = INDEX_TO_DEX[usize::from(internal)];
            let new_base = BASE_STATS[usize::from(new_dex)];
            let mut b = doc.save.box_mut(n);
            b.set_species(i, internal);
            let mut mon = b.mon_mut(i);
            mon.set_types(new_base.type1, new_base.type2);
            mon.set_catch_rate(new_base.catch_rate);
            touched = true;
        }
        ui.label("Nickname:");
        if let Some(name) = widgets::name_edit(ui, ("box_nick", n, i), &nickname) {
            if doc.save.box_mut(n).set_nickname(i, &name).is_ok() {
                touched = true;
            }
        }
        ui.label("OT:");
        if let Some(name) = widgets::name_edit(ui, ("box_ot", n, i), &ot_name) {
            if doc.save.box_mut(n).set_ot_name(i, &name).is_ok() {
                touched = true;
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Level (box):");
        let mut level = snap.box_level;
        if widgets::byte_stepper(ui, &mut level, 1..=100) {
            doc.save.box_mut(n).mon_mut(i).set_box_level(level);
            touched = true;
        }
        ui.separator();
        ui.label("Exp:");
        let mut exp = snap.exp;
        if ui
            .add(
                egui::DragValue::new(&mut exp)
                    .range(0..=0x00FF_FFFFu32)
                    .speed(100),
            )
            .changed()
        {
            doc.save.box_mut(n).mon_mut(i).set_exp(exp);
            touched = true;
        }
        ui.weak("Stats are recomputed by the game on withdrawal.");
    });

    let mut edits = Vec::new();
    common_editor(ui, "box", &snap, &mut edits);
    if !edits.is_empty() {
        let mut b = doc.save.box_mut(n);
        let mut mon = b.mon_mut(i);
        apply_edits!(mon, edits);
        touched = true;
    }

    touched
}
