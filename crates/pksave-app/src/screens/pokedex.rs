//! Pokédex screen: seen/owned checkboxes for all 151 entries.

use egui_extras::{Column, TableBuilder};
use pksave::gen1::data::SPECIES_NAMES;
use pksave::gen1::pokedex::DEX_COUNT;

use crate::app::Doc;

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc) {
    ui.heading("Pokédex");
    ui.add_space(4.0);
    let mut touched = false;

    ui.horizontal(|ui| {
        ui.strong(format!(
            "Owned: {} / 151   Seen: {} / 151",
            doc.save.owned_count(),
            doc.save.seen_count()
        ));
        ui.separator();
        if ui.button("Complete Dex").clicked() {
            doc.save.complete_dex();
            touched = true;
        }
        if ui.button("Clear all").clicked() {
            for dex in 1..=DEX_COUNT {
                doc.save.set_dex_owned(dex, false);
                doc.save.set_dex_seen(dex, false);
            }
            touched = true;
        }
    });
    ui.add_space(4.0);

    TableBuilder::new(ui)
        .striped(true)
        .column(Column::exact(44.0))
        .column(Column::exact(130.0))
        .column(Column::exact(60.0))
        .column(Column::exact(60.0))
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.strong("#");
            });
            header.col(|ui| {
                ui.strong("Species");
            });
            header.col(|ui| {
                ui.strong("Seen");
            });
            header.col(|ui| {
                ui.strong("Owned");
            });
        })
        .body(|body| {
            body.rows(20.0, usize::from(DEX_COUNT), |mut row| {
                let dex = (row.index() + 1) as u8;
                row.col(|ui| {
                    ui.monospace(format!("{dex:03}"));
                });
                row.col(|ui| {
                    ui.label(SPECIES_NAMES[usize::from(dex)]);
                });
                row.col(|ui| {
                    let mut seen = doc.save.dex_seen(dex);
                    if ui.checkbox(&mut seen, "").changed() {
                        doc.save.set_dex_seen(dex, seen);
                        touched = true;
                    }
                });
                row.col(|ui| {
                    let mut owned = doc.save.dex_owned(dex);
                    if ui.checkbox(&mut owned, "").changed() {
                        doc.save.set_dex_owned(dex, owned);
                        // Owning implies having seen it, as in the game.
                        if owned {
                            doc.save.set_dex_seen(dex, true);
                        }
                        touched = true;
                    }
                });
            });
        });

    if touched {
        doc.touch();
    }
}
