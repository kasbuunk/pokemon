//! Flags screen: named event flags (searchable), badges + fly
//! destinations, hidden-item pickups and the raw missable-object bits.

use egui_extras::{Column, TableBuilder};
use pksave::gen1::events::TOWN_NAMES;
use pksave::gen1::offsets;
use pksave::gen1::trainer::Badge;

use crate::app::Doc;

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum FlagsTab {
    #[default]
    Events,
    BadgesFly,
    HiddenItems,
    Missables,
}

#[derive(Default)]
pub struct FlagsState {
    pub tab: FlagsTab,
    pub filter: String,
}

const BADGE_NAMES: [&str; 8] = [
    "Boulder Badge",
    "Cascade Badge",
    "Thunder Badge",
    "Rainbow Badge",
    "Soul Badge",
    "Marsh Badge",
    "Volcano Badge",
    "Earth Badge",
];

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut FlagsState) {
    ui.heading("Flags");
    ui.add_space(4.0);

    ui.horizontal(|ui| {
        ui.selectable_value(&mut state.tab, FlagsTab::Events, "Events");
        ui.selectable_value(&mut state.tab, FlagsTab::BadgesFly, "Badges + Fly");
        ui.selectable_value(&mut state.tab, FlagsTab::HiddenItems, "Hidden items");
        ui.selectable_value(&mut state.tab, FlagsTab::Missables, "Missables");
    });
    ui.separator();

    match state.tab {
        FlagsTab::Events => events_tab(ui, doc, state),
        FlagsTab::BadgesFly => badges_fly_tab(ui, doc),
        FlagsTab::HiddenItems => hidden_items_tab(ui, doc),
        FlagsTab::Missables => missables_tab(ui, doc),
    }
}

fn events_tab(ui: &mut egui::Ui, doc: &mut Doc, state: &mut FlagsState) {
    let mut touched = false;
    ui.horizontal(|ui| {
        ui.label("Filter:");
        ui.add(
            egui::TextEdit::singleline(&mut state.filter)
                .hint_text("e.g. BADGE, RIVAL, SILPH…")
                .desired_width(260.0),
        );
    });
    ui.add_space(4.0);

    let needle = state.filter.to_uppercase();
    let flags: Vec<(usize, &'static str, bool)> = doc
        .save
        .named_event_flags()
        .filter(|(_, name, _)| needle.is_empty() || name.contains(&needle))
        .collect();
    ui.weak(format!("{} named flag(s)", flags.len()));

    TableBuilder::new(ui)
        .id_salt("flags_events")
        .striped(true)
        .column(Column::exact(60.0))
        .column(Column::exact(50.0))
        .column(Column::remainder())
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.strong("Bit");
            });
            header.col(|ui| {
                ui.strong("Set");
            });
            header.col(|ui| {
                ui.strong("Event");
            });
        })
        .body(|body| {
            body.rows(20.0, flags.len(), |mut row| {
                let (bit, name, value) = flags[row.index()];
                row.col(|ui| {
                    ui.monospace(format!("{bit}"));
                });
                row.col(|ui| {
                    let mut value = value;
                    if ui.checkbox(&mut value, "").changed() {
                        doc.save.set_event_flag(bit, value);
                        touched = true;
                    }
                });
                row.col(|ui| {
                    ui.monospace(name);
                });
            });
        });

    if touched {
        doc.touch();
    }
}

fn badges_fly_tab(ui: &mut egui::Ui, doc: &mut Doc) {
    let mut touched = false;
    ui.columns(2, |columns| {
        columns[0].group(|ui| {
            ui.strong("Badges");
            for (badge, name) in Badge::ALL.into_iter().zip(BADGE_NAMES) {
                let mut has = doc.save.has_badge(badge);
                if ui.checkbox(&mut has, name).changed() {
                    doc.save.set_badge(badge, has);
                    touched = true;
                }
            }
        });
        columns[1].group(|ui| {
            ui.strong("Fly destinations");
            for (index, town) in TOWN_NAMES.iter().enumerate() {
                let mut visited = doc.save.town_visited(index);
                if ui.checkbox(&mut visited, *town).changed() {
                    doc.save.set_town_visited(index, visited);
                    touched = true;
                }
            }
        });
    });
    if touched {
        doc.touch();
    }
}

fn hidden_items_tab(ui: &mut egui::Ui, doc: &mut Doc) {
    let mut touched = false;
    ui.colored_label(
        ui.visuals().warn_fg_color,
        "Advanced: raw hidden-item pickup bits (wObtainedHiddenItemsFlags). A set bit means \
         the hidden item was already collected; clear it to respawn the item. The bit -> \
         location mapping lives in the pokered disassembly (data/events/hidden_objects.asm); \
         no names are available here.",
    );
    ui.add_space(4.0);
    TableBuilder::new(ui)
        .id_salt("flags_hidden_items")
        .striped(true)
        .column(Column::exact(60.0))
        .column(Column::exact(70.0))
        .column(Column::remainder())
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.strong("Bit");
            });
            header.col(|ui| {
                ui.strong("Collected");
            });
            header.col(|_| {});
        })
        .body(|body| {
            body.rows(20.0, offsets::HIDDEN_ITEM_FLAGS_LEN * 8, |mut row| {
                let bit = row.index();
                row.col(|ui| {
                    ui.monospace(format!("{bit}"));
                });
                row.col(|ui| {
                    let mut value = doc.save.hidden_item_flag(bit);
                    if ui.checkbox(&mut value, "").changed() {
                        doc.save.set_hidden_item_flag(bit, value);
                        touched = true;
                    }
                });
                row.col(|_| {});
            });
        });
    if touched {
        doc.touch();
    }
}

fn missables_tab(ui: &mut egui::Ui, doc: &mut Doc) {
    let mut touched = false;
    ui.colored_label(
        ui.visuals().warn_fg_color,
        "Advanced: raw missable/toggleable overworld-object bits. The bit -> object mapping \
         lives in the pokered disassembly (data/maps/toggleable_objects.asm); no names are \
         available here.",
    );
    ui.add_space(4.0);
    TableBuilder::new(ui)
        .id_salt("flags_missables")
        .striped(true)
        .column(Column::exact(60.0))
        .column(Column::exact(50.0))
        .column(Column::remainder())
        .header(20.0, |mut header| {
            header.col(|ui| {
                ui.strong("Bit");
            });
            header.col(|ui| {
                ui.strong("Set");
            });
            header.col(|_| {});
        })
        .body(|body| {
            body.rows(20.0, 256, |mut row| {
                let bit = row.index();
                row.col(|ui| {
                    ui.monospace(format!("{bit}"));
                });
                row.col(|ui| {
                    let mut value = doc.save.missable_flag(bit);
                    if ui.checkbox(&mut value, "").changed() {
                        doc.save.set_missable_flag(bit, value);
                        touched = true;
                    }
                });
                row.col(|_| {});
            });
        });
    if touched {
        doc.touch();
    }
}
