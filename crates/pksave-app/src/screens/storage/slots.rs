//! Slot widgets for the storage screen: a party/box/daycare cell that
//! is a drag source, a drop target, click-selectable and carries a
//! context menu. Drawing is manual (painter) so every cell has the
//! exact size the grid math hands it.

use pksave::gen1::offsets;

use super::transfer::{self, DropTarget};
use crate::app::Doc;
use crate::widgets;

/// Identity of one storage slot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SlotId {
    Party(usize),
    Box { box_n: usize, index: usize },
    Daycare,
}

/// The type-keyed egui drag-and-drop payload: which slot the drag
/// started from. A newtype so no other payload type can collide.
#[derive(Clone, Copy)]
pub struct MonDragPayload(pub SlotId);

/// Everything a cell shows, copied out so drawing borrows nothing.
pub struct SlotInfo {
    pub nickname: String,
    pub species: u8,
    pub level: u8,
    /// The level byte disagrees with experience (W-LEVEL-EXP-MISMATCH):
    /// the game will use `level_from_exp` instead.
    pub level_from_exp: Option<u8>,
}

/// A queued mutation, applied by `mod.rs` after the whole frame is
/// drawn so no in-flight `SlotId` is invalidated mid-render.
pub enum Action {
    /// A drag released on a target (validated again at apply time).
    Drop(SlotId, DropTarget),
    /// Context-menu / action-row transfer, same semantics as a drop.
    Transfer(SlotId, DropTarget),
    /// Remove the mon in a slot (daycare: clear it).
    Delete(SlotId),
    /// Add a freshly built level-5 mon of this species to the party.
    AddToParty(u8),
    /// Add a freshly built level-5 mon of this species to box `.1`.
    AddToBox(u8, usize),
}

/// Paint one cell: selection-aware frame plus up to two lines of text.
/// Returns the click response for the whole cell. Occupied cells sense
/// clicks *and* drags on the same widget: egui then defers the
/// click-vs-drag decision until the pointer actually moves, so plain
/// clicks and right-clicks (select, context menu) are never swallowed
/// by an instantly starting drag. A separate drag-only handle (as
/// `dnd_drag_source` adds) would grab the pointer on the press frame
/// and break both.
fn draw_cell(
    ui: &mut egui::Ui,
    size: egui::Vec2,
    info: Option<&SlotInfo>,
    slot_no: usize,
    selected: bool,
) -> egui::Response {
    let sense = if info.is_some() {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(size, sense);
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let visuals = ui.style().interact_selectable(&response, selected);
    let painter = ui.painter();

    let (fill, stroke) = if info.is_some() || selected || response.hovered() {
        (visuals.bg_fill, visuals.bg_stroke)
    } else {
        // Empty cell: a faint outline and slot number only.
        (
            egui::Color32::TRANSPARENT,
            egui::Stroke::new(1.0, ui.visuals().weak_text_color().linear_multiply(0.35)),
        )
    };
    painter.rect(rect, 3.0, fill, stroke, egui::StrokeKind::Inside);
    if selected {
        painter.rect_stroke(
            rect,
            3.0,
            egui::Stroke::new(1.5, ui.visuals().selection.stroke.color),
            egui::StrokeKind::Inside,
        );
    }

    let inner = rect.shrink(4.0);
    match info {
        Some(info) => {
            let text_color = if selected {
                visuals.text_color()
            } else {
                ui.visuals().text_color()
            };
            let nick = truncated(ui, &info.nickname, inner.width(), egui::TextStyle::Body);
            painter.text(
                inner.left_top(),
                egui::Align2::LEFT_TOP,
                nick,
                egui::TextStyle::Body.resolve(ui.style()),
                text_color,
            );
            let level = match info.level_from_exp {
                Some(real) => format!("Lv.{}➡{} ⚠", info.level, real),
                None => format!("Lv.{}", info.level),
            };
            let line2 = truncated(
                ui,
                &format!("{level} {}", short_species(info.species)),
                inner.width(),
                egui::TextStyle::Small,
            );
            let color = if info.level_from_exp.is_some() {
                ui.visuals().warn_fg_color
            } else {
                ui.visuals().weak_text_color()
            };
            painter.text(
                inner.left_bottom(),
                egui::Align2::LEFT_BOTTOM,
                line2,
                egui::TextStyle::Small.resolve(ui.style()),
                color,
            );
        }
        None => {
            painter.text(
                inner.left_top(),
                egui::Align2::LEFT_TOP,
                format!("{slot_no}"),
                egui::TextStyle::Small.resolve(ui.style()),
                ui.visuals().weak_text_color().linear_multiply(0.6),
            );
        }
    }
    response
}

/// Species label without the `#NNN ` dex prefix, for the tight cells.
fn short_species(internal: u8) -> String {
    let label = widgets::species_label(internal);
    match label.split_once(' ') {
        Some((dex, name)) if dex.starts_with('#') => name.to_owned(),
        _ => label,
    }
}

/// Elide `text` with `…` to fit `width` in the given style.
fn truncated(ui: &egui::Ui, text: &str, width: f32, style: egui::TextStyle) -> String {
    let font = style.resolve(ui.style());
    let fits = |s: &str| {
        ui.painter()
            .layout_no_wrap(s.to_owned(), font.clone(), egui::Color32::WHITE)
            .rect
            .width()
            <= width
    };
    if fits(text) {
        return text.to_owned();
    }
    let mut out = text.to_owned();
    while !out.is_empty() {
        out.pop();
        let candidate = format!("{out}…");
        if fits(&candidate) {
            return candidate;
        }
    }
    "…".to_owned()
}

/// One occupied or empty cell: drag source (occupied only), drop
/// target, selection and context menu.
#[allow(clippy::too_many_arguments)]
pub fn slot_cell(
    ui: &mut egui::Ui,
    doc: &Doc,
    slot: SlotId,
    size: egui::Vec2,
    info: Option<SlotInfo>,
    selected: &mut Option<SlotId>,
    queued: &mut Vec<Action>,
) {
    let id = ui.id().with((
        "mon_slot",
        match slot {
            SlotId::Party(i) => (0usize, 0usize, i),
            SlotId::Box { box_n, index } => (1, box_n, index),
            SlotId::Daycare => (2, 0, 0),
        },
    ));

    let slot_no = match slot {
        SlotId::Party(i) => i + 1,
        SlotId::Box { index, .. } => index + 1,
        SlotId::Daycare => 1,
    };

    let response = draw_cell(
        ui,
        size,
        info.as_ref(),
        slot_no,
        info.is_some() && *selected == Some(slot),
    );

    // Expose the cell to accessibility (and the widget tests).
    let place = match slot {
        SlotId::Party(i) => format!("party slot {}", i + 1),
        SlotId::Box { box_n, index } => format!("box {} slot {}", box_n + 1, index + 1),
        SlotId::Daycare => "daycare".to_owned(),
    };
    let label = match &info {
        Some(info) => format!("{}, {place}", info.nickname),
        None => format!("Empty {place}"),
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));

    if let Some(info) = &info {
        response.dnd_set_drag_payload(MonDragPayload(slot));
        if response.dragged() {
            drag_preview(ui, id, size, info);
        } else if response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
        }
    }

    // Both click kinds select: the detail editor follows the slot the
    // user is acting on, so right-click alone is enough to edit.
    if (response.clicked() || response.secondary_clicked()) && info.is_some() {
        *selected = Some(slot);
    }

    if info.is_some() {
        response.context_menu(|ui| context_menu(ui, doc, slot, selected, queued));
    }

    handle_drop_target(ui, &response, doc, DropTarget::Slot(slot), queued);
}

/// A ghost of the dragged cell following the pointer, painted on its
/// own tooltip-order layer; the in-place cell keeps rendering.
fn drag_preview(ui: &egui::Ui, id: egui::Id, size: egui::Vec2, info: &SlotInfo) {
    let Some(pos) = ui.ctx().pointer_interact_pos() else {
        return;
    };
    let painter = ui
        .ctx()
        .layer_painter(egui::LayerId::new(egui::Order::Tooltip, id));
    let rect = egui::Rect::from_center_size(pos, size);
    let visuals = &ui.visuals().widgets.active;
    painter.rect(
        rect,
        3.0,
        visuals.bg_fill.gamma_multiply(0.85),
        visuals.bg_stroke,
        egui::StrokeKind::Inside,
    );
    painter.text(
        rect.shrink(4.0).left_top(),
        egui::Align2::LEFT_TOP,
        &info.nickname,
        egui::TextStyle::Body.resolve(ui.style()),
        ui.visuals().text_color(),
    );
}

/// Shared drop-target behavior for cells and tabs: highlight a valid
/// target while a mon hovers it (tooltip with the refusal otherwise)
/// and queue the action on release.
pub fn handle_drop_target(
    ui: &egui::Ui,
    response: &egui::Response,
    doc: &Doc,
    target: DropTarget,
    queued: &mut Vec<Action>,
) {
    if let Some(payload) = response.dnd_hover_payload::<MonDragPayload>() {
        match transfer::validate_drop(&doc.save, payload.0, target) {
            Ok(_) => {
                ui.painter().rect_stroke(
                    response.rect,
                    3.0,
                    egui::Stroke::new(2.0, ui.visuals().hyperlink_color),
                    egui::StrokeKind::Inside,
                );
            }
            Err(e) => {
                response.show_tooltip_text(e.message());
            }
        }
    }
    if let Some(payload) = response.dnd_release_payload::<MonDragPayload>() {
        queued.push(Action::Drop(payload.0, target));
    }
}

/// Right-click actions for an occupied slot.
fn context_menu(
    ui: &mut egui::Ui,
    doc: &Doc,
    slot: SlotId,
    selected: &mut Option<SlotId>,
    queued: &mut Vec<Action>,
) {
    if ui.button("✏ Edit").clicked() {
        *selected = Some(slot);
        ui.close();
    }
    let party_full = doc.save.party().len() >= offsets::PARTY_CAPACITY;
    match slot {
        SlotId::Party(_) => {
            ui.menu_button("Move to box", |ui| {
                for n in 0..offsets::NUM_BOXES {
                    let full = doc.save.box_(n).len() >= offsets::MONS_PER_BOX;
                    let star = if doc.save.box_is_live(n) { " ★" } else { "" };
                    if ui
                        .add_enabled(!full, egui::Button::new(format!("Box {}{star}", n + 1)))
                        .clicked()
                    {
                        queued.push(Action::Transfer(slot, DropTarget::BoxTab(n)));
                        ui.close();
                    }
                }
            });
            if ui
                .add_enabled(
                    doc.save.daycare().is_none(),
                    egui::Button::new("Move to daycare"),
                )
                .clicked()
            {
                queued.push(Action::Transfer(slot, DropTarget::Slot(SlotId::Daycare)));
                ui.close();
            }
            if ui.button("🗑 Delete").clicked() {
                queued.push(Action::Delete(slot));
                ui.close();
            }
        }
        SlotId::Box { box_n, .. } => {
            if ui
                .add_enabled(!party_full, egui::Button::new("Withdraw ➡ party"))
                .clicked()
            {
                queued.push(Action::Transfer(slot, DropTarget::Party));
                ui.close();
            }
            ui.menu_button("Move to box", |ui| {
                for n in 0..offsets::NUM_BOXES {
                    if n == box_n {
                        continue;
                    }
                    let full = doc.save.box_(n).len() >= offsets::MONS_PER_BOX;
                    let star = if doc.save.box_is_live(n) { " ★" } else { "" };
                    if ui
                        .add_enabled(!full, egui::Button::new(format!("Box {}{star}", n + 1)))
                        .clicked()
                    {
                        queued.push(Action::Transfer(slot, DropTarget::BoxTab(n)));
                        ui.close();
                    }
                }
            });
            if ui.button("🗑 Delete").clicked() {
                queued.push(Action::Delete(slot));
                ui.close();
            }
        }
        SlotId::Daycare => {
            if ui
                .add_enabled(!party_full, egui::Button::new("Take ➡ party"))
                .clicked()
            {
                queued.push(Action::Transfer(slot, DropTarget::Party));
                ui.close();
            }
            if ui.button("🗑 Clear daycare").clicked() {
                queued.push(Action::Delete(slot));
                ui.close();
            }
        }
    }
}
