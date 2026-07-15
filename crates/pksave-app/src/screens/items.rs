//! Items screen: the bag (20 slots) and the item PC (50 slots), each an
//! editable list with a searchable add row.

use pksave::gen1::items::MAX_QTY;
use pksave::gen1::offsets;

use crate::app::Doc;
use crate::widgets;

pub struct ItemsState {
    pub bag_add_id: u8,
    pub pc_add_id: u8,
}

impl Default for ItemsState {
    fn default() -> Self {
        // 0x14 = POTION, a sensible default.
        ItemsState {
            bag_add_id: 0x14,
            pc_add_id: 0x14,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Which {
    Bag,
    Pc,
}

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut ItemsState) {
    ui.heading("Items");
    ui.add_space(4.0);
    let mut touched = false;
    let item_options = widgets::item_options();

    ui.columns(2, |columns| {
        touched |= list_panel(
            &mut columns[0],
            doc,
            Which::Bag,
            "Bag",
            offsets::BAG_CAPACITY,
            &mut state.bag_add_id,
            &item_options,
        );
        touched |= list_panel(
            &mut columns[1],
            doc,
            Which::Pc,
            "Item PC",
            offsets::PC_ITEM_CAPACITY,
            &mut state.pc_add_id,
            &item_options,
        );
    });

    if touched {
        doc.touch();
    }
}

fn list_panel(
    ui: &mut egui::Ui,
    doc: &mut Doc,
    which: Which,
    title: &str,
    capacity: usize,
    add_id: &mut u8,
    item_options: &[(u16, String)],
) -> bool {
    let mut touched = false;
    let len = view_len(doc, which);

    ui.group(|ui| {
        ui.strong(format!("{title} — {len} / {capacity}"));
        ui.add_space(2.0);

        egui::ScrollArea::vertical()
            .id_salt((title, "list"))
            .max_height(420.0)
            .show(ui, |ui| {
                for index in 0..len {
                    let Some((id, qty)) = get_entry(doc, which, index) else {
                        continue;
                    };
                    ui.horizontal(|ui| {
                        ui.monospace(format!("{:2}.", index + 1));
                        if let Some(new_id) = widgets::search_combo(
                            ui,
                            (title, "item", index),
                            &widgets::item_label(id),
                            item_options,
                        ) {
                            with_list(doc, which, |list| list.set_id(index, new_id as u8));
                            touched = true;
                        }
                        let mut qty = qty;
                        if ui
                            .add(egui::DragValue::new(&mut qty).range(1..=MAX_QTY))
                            .changed()
                        {
                            with_list(doc, which, |list| list.set_qty(index, qty));
                            touched = true;
                        }
                        if ui.small_button("⬆").clicked() && index > 0 {
                            with_list(doc, which, |list| list.swap(index, index - 1));
                            touched = true;
                        }
                        if ui.small_button("⬇").clicked() && index + 1 < len {
                            with_list(doc, which, |list| list.swap(index, index + 1));
                            touched = true;
                        }
                        if ui.small_button("🗑").clicked() {
                            with_list(doc, which, |list| list.remove(index));
                            touched = true;
                        }
                    });
                }
                if len == 0 {
                    ui.weak("(empty)");
                }
            });

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Add:");
            if let Some(new_id) = widgets::search_combo(
                ui,
                (title, "add"),
                &widgets::item_label(*add_id),
                item_options,
            ) {
                *add_id = new_id as u8;
            }
            let full = len >= capacity;
            if ui.add_enabled(!full, egui::Button::new("+ Add")).clicked() {
                let id = *add_id;
                let mut ok = false;
                with_list(doc, which, |list| ok = list.add(id, 1).is_ok());
                touched |= ok;
            }
        });
    });

    touched
}

fn view_len(doc: &Doc, which: Which) -> usize {
    match which {
        Which::Bag => doc.save.bag_items().len(),
        Which::Pc => doc.save.pc_items().len(),
    }
}

fn get_entry(doc: &Doc, which: Which, index: usize) -> Option<(u8, u8)> {
    match which {
        Which::Bag => doc.save.bag_items().get(index),
        Which::Pc => doc.save.pc_items().get(index),
    }
}

fn with_list(
    doc: &mut Doc,
    which: Which,
    f: impl FnOnce(&mut pksave::gen1::items::ItemListMut<'_>),
) {
    match which {
        Which::Bag => f(&mut doc.save.bag_items_mut()),
        Which::Pc => f(&mut doc.save.pc_items_mut()),
    }
}
