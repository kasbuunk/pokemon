//! Map screen: current position with a coherent warp helper, plus the
//! advanced raw fields (last map, tileset, view pointer).

use pksave::gen1::data::MAP_NAMES;

use crate::app::Doc;
use crate::widgets;

#[derive(Default)]
pub struct MapState {
    pub map_id: u8,
    pub x: u8,
    pub y: u8,
    pub initialized: bool,
    pub pointer_text: String,
}

fn map_label(id: u8) -> String {
    let name = MAP_NAMES[usize::from(id)];
    if name.is_empty() {
        format!("0x{id:02X} (unknown)")
    } else {
        format!("0x{id:02X} {name}")
    }
}

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut MapState) {
    ui.heading("Map");
    ui.add_space(4.0);
    let mut touched = false;

    if !state.initialized {
        state.map_id = doc.save.cur_map();
        state.x = doc.save.x_coord();
        state.y = doc.save.y_coord();
        state.pointer_text = format!("{:04X}", doc.save.map_view_pointer());
        state.initialized = true;
    }

    ui.group(|ui| {
        ui.strong("Current position");
        ui.horizontal(|ui| {
            ui.label(format!(
                "Now: {} at ({}, {})",
                map_label(doc.save.cur_map()),
                doc.save.x_coord(),
                doc.save.y_coord()
            ));
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("Warp to:");
            let options: Vec<(u16, String)> = MAP_NAMES
                .iter()
                .enumerate()
                .filter(|(_, name)| !name.is_empty())
                .map(|(id, name)| (id as u16, format!("0x{id:02X} {name}")))
                .collect();
            if let Some(id) =
                widgets::search_combo(ui, "warp_map", &map_label(state.map_id), &options)
            {
                state.map_id = id as u8;
            }
            ui.label("X");
            widgets::byte_stepper(ui, &mut state.x, 0..=255);
            ui.label("Y");
            widgets::byte_stepper(ui, &mut state.y, 0..=255);
            if ui
                .button("Warp")
                .on_hover_text(
                    "Sets map id, coordinates and the derived block coordinates together",
                )
                .clicked()
            {
                doc.save.warp_to(state.map_id, state.x, state.y);
                touched = true;
            }
        });
        ui.weak(
            "warp_to keeps map/coords/block-coords coherent, but does NOT update the tileset \
             or view pointer — rendering may glitch until the next in-game warp.",
        );
    });

    ui.add_space(8.0);
    ui.group(|ui| {
        ui.strong("Advanced");
        ui.horizontal(|ui| {
            ui.label("Last outdoor map:");
            let options: Vec<(u16, String)> = MAP_NAMES
                .iter()
                .enumerate()
                .filter(|(_, name)| !name.is_empty())
                .map(|(id, name)| (id as u16, format!("0x{id:02X} {name}")))
                .collect();
            if let Some(id) =
                widgets::search_combo(ui, "last_map", &map_label(doc.save.last_map()), &options)
            {
                doc.save.set_last_map(id as u8);
                touched = true;
            }
        });
        ui.horizontal(|ui| {
            ui.label("Tileset:");
            let mut tileset = doc.save.tileset();
            if widgets::byte_stepper(ui, &mut tileset, 0..=255) {
                doc.save.set_tileset(tileset);
                touched = true;
            }
            ui.colored_label(
                ui.visuals().warn_fg_color,
                "⚠ A tileset that doesn't match the map can soft-lock or glitch the game.",
            );
        });
        ui.horizontal(|ui| {
            ui.label("Tile-block view pointer (hex):");
            let response = ui.add(
                egui::TextEdit::singleline(&mut state.pointer_text)
                    .desired_width(60.0)
                    .font(egui::TextStyle::Monospace),
            );
            let parsed = u16::from_str_radix(state.pointer_text.trim(), 16);
            if response.changed() {
                if let Ok(pointer) = parsed {
                    doc.save.set_map_view_pointer(pointer);
                    touched = true;
                }
            }
            if parsed.is_err() {
                ui.colored_label(ui.visuals().error_fg_color, "not hex");
            } else if !response.has_focus() {
                let stored = format!("{:04X}", doc.save.map_view_pointer());
                if !state.pointer_text.eq_ignore_ascii_case(&stored) {
                    state.pointer_text = stored;
                }
            }
            ui.weak("raw WRAM pointer — advanced");
        });
    });

    if touched {
        doc.touch();
    }
}
