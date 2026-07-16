//! The per-slot detail editor of the storage screen, dispatched on
//! [`SlotId`]. Field classes are marked so nothing changes behind the
//! player's back in-game:
//!
//! - plain fields are stored and authoritative (exp, DVs, stat exp, …);
//! - ∑ fields are stored but *derived* — the game recomputes them from
//!   the authoritative ones on withdrawal/level-up (party level, the
//!   five calculated party stats). Drift is shown inline with a
//!   one-click fix, never silently "corrected";
//! - ↻ values are not stored at all for box/daycare records — the game
//!   computes them on withdrawal; the editor shows a live preview.

use pksave::gen1::data::{BASE_STATS, INDEX_TO_DEX, SPECIES_NAMES};
use pksave::gen1::pokemon::{BoxMonView, MonMut, MonView, PartyMon, PartyMonMut};
use pksave::gen1::stats;
use pksave::gen1::{offsets, text};

use super::editor::{apply_mon_edits, common_editor, expected_stats, MonSnapshot};
use super::slots::SlotId;
use crate::app::Doc;
use crate::widgets;

/// Render the editor for `slot`. Returns whether anything changed.
pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, slot: Option<SlotId>) -> bool {
    let Some(slot) = slot else {
        ui.weak("Select a Pokémon to edit it.");
        ui.weak("Drag slots to move; right-click for actions.");
        return false;
    };
    legend(ui);
    ui.add_space(4.0);
    match slot {
        SlotId::Party(i) if i < doc.save.party().len() => party_detail(ui, doc, i),
        SlotId::Box { box_n, index } if index < doc.save.box_(box_n).len() => {
            box_detail(ui, doc, box_n, index)
        }
        SlotId::Daycare if doc.save.daycare().is_some() => daycare_detail(ui, doc),
        _ => {
            ui.weak("Select a Pokémon to edit it.");
            false
        }
    }
}

/// One-line key for the field-class markers.
fn legend(ui: &mut egui::Ui) {
    ui.weak("∑ derived but stored (drift shown inline) · ↻ recomputed by the game")
        .on_hover_text(
            "Plain fields are authoritative — the game reads them as-is. ∑ fields are \
             stored in the save but recomputed by the game from the authoritative ones \
             (on withdrawal and at level-up); if they drift from the formula the editor \
             shows what the game will change them to. ↻ values are not stored for \
             box/daycare Pokémon at all — the game computes them on withdrawal.",
        );
}

/// A warn-colored inline drift note.
fn drift_label(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.colored_label(ui.visuals().warn_fg_color, text.into())
}

/// The party-slot editor.
fn party_detail(ui: &mut egui::Ui, doc: &mut Doc, i: usize) -> bool {
    let mut touched = false;
    let snap = MonSnapshot::read(&doc.save.party().mon(i));
    let (nickname, ot_name, level, stats_now) = {
        let party = doc.save.party();
        let mon = party.mon(i);
        (
            party.nickname(i),
            party.ot_name(i),
            mon.level(),
            [
                mon.max_hp(),
                mon.attack(),
                mon.defense(),
                mon.speed(),
                mon.special(),
            ],
        )
    };
    let base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(snap.species)])];

    // ---- identity ----
    ui.horizontal(|ui| {
        ui.label("Species:");
        if let Some(internal) = widgets::species_combo(ui, ("party_species", i), snap.species) {
            let new_dex = INDEX_TO_DEX[usize::from(internal)];
            let new_base = BASE_STATS[usize::from(new_dex)];
            let mut party = doc.save.party_mut();
            party.set_species(i, internal);
            let mut mon = party.mon_mut(i);
            mon.set_types(new_base.type1, new_base.type2);
            mon.set_catch_rate(new_base.catch_rate);
            touched = true;
        }
        ui.label("Nickname:");
        if let Some(name) = widgets::name_edit(ui, ("party_nick", i), &nickname) {
            if doc.save.party_mut().set_nickname(i, &name).is_ok() {
                touched = true;
            }
        }
        ui.label("OT:");
        if let Some(name) = widgets::name_edit(ui, ("party_ot", i), &ot_name) {
            if doc.save.party_mut().set_ot_name(i, &name).is_ok() {
                touched = true;
            }
        }
    });

    // ---- level & exp ----
    ui.horizontal(|ui| {
        ui.label("∑ Level:");
        let mut new_level = level;
        if widgets::byte_stepper(ui, &mut new_level, 1..=100) {
            doc.save
                .party_mut()
                .mon_mut(i)
                .set_level_coherent(new_level);
            touched = true;
        }
        ui.weak("(sets exp & stats coherently, full-heals)");
    });
    let level_from_exp = snap.level_from_exp();
    ui.horizontal(|ui| {
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
            doc.save.party_mut().mon_mut(i).set_exp(exp);
            touched = true;
        }
        if level == level_from_exp {
            ui.weak(format!("matches level {level}"));
        } else {
            drift_label(
                ui,
                format!("⚠ game will treat this as level {level_from_exp}"),
            )
            .on_hover_text(
                "In Gen 1 the level is derived from experience on withdrawal and at \
                 the next experience gain; a mismatched level byte silently changes \
                 in play.",
            );
            if ui
                .button(format!("Set level to {level_from_exp}"))
                .on_hover_text("Accept the exp-derived level; exp and current HP stay untouched")
                .clicked()
            {
                doc.save.party_mut().mon_mut(i).sync_level_from_exp();
                touched = true;
            }
        }
        if ui
            .button("Sync exp from level")
            .on_hover_text("exp := exp_for_level(growth curve, level)")
            .clicked()
        {
            let exp = stats::exp_for_level(base.growth_rate, level);
            doc.save.party_mut().mon_mut(i).set_exp(exp);
            touched = true;
        }
    });

    // ---- common record fields ----
    let mut edits = Vec::new();
    common_editor(ui, "party", &snap, &mut edits);
    if !edits.is_empty() {
        let mut party = doc.save.party_mut();
        let mut mon = party.mon_mut(i);
        apply_mon_edits(&mut mon, &edits);
        touched = true;
    }

    // ---- calculated stats ----
    ui.add_space(4.0);
    ui.strong("∑ Calculated stats");
    ui.horizontal(|ui| {
        let mut stored = stats_now;
        let mut changed = false;
        for (label, value) in ["Max HP", "Atk", "Def", "Spd", "Spc"]
            .into_iter()
            .zip(stored.iter_mut())
        {
            ui.label(label);
            changed |= widgets::word_stepper(ui, value, 0..=999);
        }
        if changed {
            let mut party = doc.save.party_mut();
            let mut mon = party.mon_mut(i);
            mon.set_max_hp(stored[0]);
            mon.set_attack(stored[1]);
            mon.set_defense(stored[2]);
            mon.set_speed(stored[3]);
            mon.set_special(stored[4]);
            touched = true;
        }
    });
    ui.horizontal(|ui| {
        let expected = expected_stats(&snap, level);
        if stats_now == expected {
            ui.weak("matches the formula");
        } else {
            drift_label(
                ui,
                format!(
                    "↻ on withdraw/level-up the game recalculates to {} / {} / {} / {} / {}",
                    expected[0], expected[1], expected[2], expected[3], expected[4]
                ),
            )
            .on_hover_text(
                "A difference here is a real in-game state — stat exp is only applied \
                 at level-up and on withdrawal (the Gen 1 'box trick'). Leave it if it \
                 is intentional.",
            );
        }
        if ui
            .button("Recalculate")
            .on_hover_text("Recompute the five stats from base stats, DVs and stat exp")
            .clicked()
        {
            doc.save.party_mut().mon_mut(i).recalculate_stats();
            touched = true;
        }
    });

    touched
}

/// The box-slot editor.
fn box_detail(ui: &mut egui::Ui, doc: &mut Doc, n: usize, i: usize) -> bool {
    let mut touched = false;
    let snap = MonSnapshot::read(&doc.save.box_(n).mon(i));
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

    // ---- level & exp (coherent) ----
    ui.horizontal(|ui| {
        ui.label("Level:");
        let mut level = snap.box_level;
        if widgets::byte_stepper(ui, &mut level, 1..=100) {
            doc.save.box_mut(n).mon_mut(i).set_level_coherent(level);
            touched = true;
        }
        ui.weak("(sets exp coherently, full-heals)").on_hover_text(
            "Sets experience to the start of this level — the game derives a box \
             Pokémon's level from experience on withdrawal, not from the shown level \
             byte. Current HP becomes the max HP it will have when withdrawn.",
        );
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
            let mut b = doc.save.box_mut(n);
            let mut mon = b.mon_mut(i);
            mon.set_exp(exp);
            // The shown level byte is cosmetic; keep it agreeing with
            // exp so the in-game box list shows the truth.
            let level = mon.level_from_exp();
            mon.set_box_level(level);
            touched = true;
        }
    });

    touched |= mismatch_repair_chip(ui, doc, &snap, RepairTarget::Box(n, i));
    withdraw_preview(ui, &snap, "On withdraw");

    let mut edits = Vec::new();
    common_editor(ui, "box", &snap, &mut edits);
    if !edits.is_empty() {
        let mut b = doc.save.box_mut(n);
        let mut mon = b.mon_mut(i);
        apply_mon_edits(&mut mon, &edits);
        touched = true;
    }

    touched
}

/// The daycare editor (a box-format record outside any box).
fn daycare_detail(ui: &mut egui::Ui, doc: &mut Doc) -> bool {
    let mut touched = false;
    let Some((record, nickname, ot_name)) = doc.save.daycare().map(|view| {
        let mut rec = [0u8; offsets::BOX_MON_SIZE];
        rec.copy_from_slice(view.mon().as_bytes());
        (rec, view.nickname(), view.ot_name())
    }) else {
        return false;
    };
    let snap = MonSnapshot::read(&BoxMonView::new(&record));

    ui.strong("Daycare");
    ui.horizontal(|ui| {
        ui.label("Species:");
        if let Some(internal) = widgets::species_combo(ui, "daycare_species", snap.species) {
            let new_base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(internal)])];
            if let Some(mut daycare) = doc.save.daycare_mut() {
                let mut mon = daycare.mon_mut();
                mon.set_species(internal);
                mon.set_types(new_base.type1, new_base.type2);
                mon.set_catch_rate(new_base.catch_rate);
                touched = true;
            }
        }
        ui.label("Nickname:");
        if let Some(name) = widgets::name_edit(ui, "daycare_nick", &nickname) {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                if daycare.set_nickname(&name).is_ok() {
                    touched = true;
                }
            }
        }
        ui.label("OT:");
        if let Some(name) = widgets::name_edit(ui, "daycare_ot", &ot_name) {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                if daycare.set_ot_name(&name).is_ok() {
                    touched = true;
                }
            }
        }
    });

    ui.horizontal(|ui| {
        ui.label("Level:");
        let mut level = snap.box_level;
        if widgets::byte_stepper(ui, &mut level, 1..=100) {
            if let Some(mut daycare) = doc.save.daycare_mut() {
                daycare.mon_mut().set_level_coherent(level);
                touched = true;
            }
        }
        ui.weak("(sets exp coherently, full-heals)").on_hover_text(
            "Sets experience to the start of this level — the game derives the level \
             from experience when the Pokémon returns to the party.",
        );
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
            if let Some(mut daycare) = doc.save.daycare_mut() {
                let mut mon = daycare.mon_mut();
                mon.set_exp(exp);
                let level = mon.level_from_exp();
                mon.set_box_level(level);
                touched = true;
            }
        }
    });

    touched |= mismatch_repair_chip(ui, doc, &snap, RepairTarget::Daycare);
    withdraw_preview(ui, &snap, "On return to party");

    let mut edits = Vec::new();
    common_editor(ui, "daycare", &snap, &mut edits);
    if !edits.is_empty() {
        if let Some(mut daycare) = doc.save.daycare_mut() {
            let mut mon = daycare.mon_mut();
            apply_mon_edits(&mut mon, &edits);
            touched = true;
        }
    }

    touched
}

enum RepairTarget {
    Box(usize, usize),
    Daycare,
}

/// When the (cosmetic) level byte disagrees with experience — e.g. a
/// save edited by an older tool — offer both one-click repairs, since
/// the user's intent is ambiguous.
fn mismatch_repair_chip(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    snap: &MonSnapshot,
    target: RepairTarget,
) -> bool {
    let from_exp = snap.level_from_exp();
    if snap.box_level == from_exp {
        return false;
    }
    let mut touched = false;
    ui.horizontal(|ui| {
        drift_label(
            ui,
            format!(
                "⚠ Shown level {} ≠ level from exp {from_exp} — the game will use {from_exp}",
                snap.box_level
            ),
        );
        let keep = ui
            .button(format!("Keep Lv.{} (set exp)", snap.box_level))
            .on_hover_text("Set experience to match the shown level")
            .clicked();
        let accept = ui
            .button(format!("Accept Lv.{from_exp} (fix byte)"))
            .on_hover_text("Fix the shown level to what experience says; exp stays untouched")
            .clicked();
        if keep || accept {
            let apply = |mon: &mut dyn FnMut(bool, u8)| {
                if keep {
                    mon(true, snap.box_level);
                } else {
                    mon(false, from_exp);
                }
            };
            match target {
                RepairTarget::Box(n, i) => {
                    let mut b = doc.save.box_mut(n);
                    let mut mon = b.mon_mut(i);
                    apply(&mut |coherent, level| {
                        if coherent {
                            mon.set_level_coherent(level);
                        } else {
                            mon.set_box_level(level);
                        }
                    });
                }
                RepairTarget::Daycare => {
                    if let Some(mut daycare) = doc.save.daycare_mut() {
                        let mut mon = daycare.mon_mut();
                        apply(&mut |coherent, level| {
                            if coherent {
                                mon.set_level_coherent(level);
                            } else {
                                mon.set_box_level(level);
                            }
                        });
                    }
                }
            }
            touched = true;
        }
    });
    touched
}

/// Read-only preview of what the game computes on withdrawal (level
/// from exp, five stats from base + DVs + stat exp).
fn withdraw_preview(ui: &mut egui::Ui, snap: &MonSnapshot, verb: &str) {
    let level = snap.level_from_exp();
    let s = expected_stats(snap, level);
    ui.weak(format!(
        "↻ {verb}: Lv.{level} — HP {} / Atk {} / Def {} / Spd {} / Spc {}",
        s[0], s[1], s[2], s[3], s[4]
    ))
    .on_hover_text(
        "Box and daycare records store no stats: the game recomputes level (from \
         experience) and all five stats when the Pokémon rejoins the party.",
    );
}

/// Build a minimal legal level-5 mon of `internal` species as a party
/// record (also the source for box adds, truncated via deposit logic).
fn build_level5(doc: &Doc, internal: u8) -> ([u8; offsets::PARTY_MON_SIZE], String, String) {
    let dex = INDEX_TO_DEX[usize::from(internal)];
    let base = BASE_STATS[usize::from(dex)];
    let mut record = [0u8; offsets::PARTY_MON_SIZE];
    {
        let mut mon = PartyMonMut::new(&mut record);
        mon.set_species(internal);
        mon.set_types(base.type1, base.type2);
        mon.set_catch_rate(base.catch_rate);
        mon.set_ot_id(doc.save.player_id());
        mon.set_level_coherent(5);
    }
    let nickname = if dex == 0 {
        "MISSINGNO.".to_owned()
    } else {
        SPECIES_NAMES[usize::from(dex)].to_owned()
    };
    (record, player_ot_name(doc), nickname)
}

/// Append a freshly built level-5 mon to the party.
pub fn add_mon_to_party(doc: &mut Doc, internal: u8) -> bool {
    let (record, ot, nickname) = build_level5(doc, internal);
    doc.save.party_mut().add(&record, &ot, &nickname).is_ok()
}

/// Append a freshly built level-5 mon to box `n`.
pub fn add_mon_to_box(doc: &mut Doc, internal: u8, n: usize) -> bool {
    let (record, ot, nickname) = build_level5(doc, internal);
    let boxed = pksave::gen1::pokemon::party_to_box(&record);
    doc.save.box_mut(n).add(&boxed, &ot, &nickname).is_ok()
}

/// The player's name if it encodes, else a safe fallback OT.
fn player_ot_name(doc: &Doc) -> String {
    let ot = doc.save.player_name();
    if text::encode(&ot, offsets::NAME_LEN).is_ok() {
        ot
    } else {
        "TRAINER".to_owned()
    }
}
