//! Hall of Fame screen: induction count plus the 50 stored teams of six
//! 16-byte records each.

use pksave::gen1::data::{INDEX_TO_DEX, SPECIES_NAMES};
use pksave::gen1::hof::HOF_TEAM_LEN;
use pksave::gen1::offsets;

use crate::app::Doc;
use crate::widgets;

#[derive(Default)]
pub struct HofState {
    pub team: usize,
}

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut HofState) {
    ui.heading("Hall of Fame");
    ui.add_space(4.0);
    let mut touched = false;

    ui.horizontal(|ui| {
        ui.label("Inductions (wNumHoFTeams):");
        let mut count = doc.save.hof_team_count();
        if widgets::byte_stepper(ui, &mut count, 0..=255) {
            doc.save.set_hof_team_count(count);
            touched = true;
        }
        ui.weak(format!(
            "(storage holds the most recent {} teams)",
            offsets::HOF_TEAM_CAPACITY
        ));
    });

    let stored = usize::from(doc.save.hof_team_count()).min(offsets::HOF_TEAM_CAPACITY);
    if stored == 0 {
        ui.weak("No stored teams. Raise the induction count to edit team slots.");
        return finish(doc, touched);
    }
    state.team = state.team.min(stored - 1);

    ui.horizontal(|ui| {
        ui.label("Team:");
        let mut team = state.team;
        if ui
            .add(egui::DragValue::new(&mut team).range(0..=stored - 1))
            .changed()
        {
            state.team = team;
        }
        ui.weak("(0 = oldest stored)");
        if ui.button("Clear team").clicked() {
            let mut t = doc.save.hof_team_mut(state.team);
            for slot in 0..HOF_TEAM_LEN {
                t.clear_slot(slot);
            }
            touched = true;
        }
    });
    ui.add_space(6.0);

    let t = state.team;
    for slot in 0..HOF_TEAM_LEN {
        let record = {
            let team = doc.save.hof_team(t);
            team.mon(slot)
                .map(|m| (m.species(), m.level(), m.nickname()))
        };
        ui.horizontal(|ui| {
            ui.monospace(format!("{}.", slot + 1));
            match record {
                Some((species, level, nickname)) => {
                    if let Some(internal) = widgets::species_combo(ui, ("hof", t, slot), species) {
                        if write_slot(doc, t, slot, internal, level, &nickname) {
                            touched = true;
                        }
                    }
                    ui.label("Lv.");
                    let mut new_level = level;
                    if widgets::byte_stepper(ui, &mut new_level, 1..=100)
                        && write_slot(doc, t, slot, species, new_level, &nickname)
                    {
                        touched = true;
                    }
                    if let Some(name) = widgets::name_edit(ui, ("hof_nick", t, slot), &nickname) {
                        if write_slot(doc, t, slot, species, level, &name) {
                            touched = true;
                        }
                    }
                    if ui
                        .small_button("🗑")
                        .on_hover_text("Empty this slot (hides later slots, as in the game)")
                        .clicked()
                    {
                        doc.save.hof_team_mut(t).clear_slot(slot);
                        touched = true;
                    }
                }
                None => {
                    ui.weak("(empty)");
                    // Only offer to fill the first empty slot: the game's
                    // reader stops at the first empty record.
                    let is_first_empty = doc.save.hof_team(t).len() == slot;
                    if is_first_empty && ui.button("Add").clicked() {
                        // Internal 0x99 = Bulbasaur.
                        let internal = 0x99;
                        let dex = INDEX_TO_DEX[usize::from(internal)];
                        let name = SPECIES_NAMES[usize::from(dex)];
                        if write_slot(doc, t, slot, internal, 50, name) {
                            touched = true;
                        }
                    }
                }
            }
        });
    }

    finish(doc, touched)
}

fn write_slot(doc: &mut Doc, team: usize, slot: usize, species: u8, level: u8, nick: &str) -> bool {
    doc.save
        .hof_team_mut(team)
        .set_mon(slot, species, level, nick)
        .is_ok()
}

fn finish(doc: &mut Doc, touched: bool) {
    if touched {
        doc.touch();
    }
}
