//! History screen: the version table (time, name, size, trainer
//! summary) with per-row Restore / Diff / rename / Export / Delete
//! actions, plus the minimal diff view against the current buffer.
//!
//! The screen is pure presentation: row actions are emitted as
//! [`HistoryAction`]s and executed by the `App` (which owns the
//! [`crate::history::HistoryStore`] and the unsaved-changes guard).

use egui_extras::{Column, TableBuilder};

use crate::history::{self, VersionRow};

/// A row action requested by the user, executed by the `App`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryAction {
    /// Load the version into the editor (guarded by the dirty check;
    /// nothing is written to disk until the user saves).
    Restore(u64),
    /// Diff the version against the current buffer.
    Diff(u64),
    /// Set (`Some`) or clear (`None`) the version's name.
    SetLabel(u64, Option<String>),
    /// Save-as / download a copy of the version.
    Export(u64),
    /// Delete the version (the `App` confirms first).
    Delete(u64),
}

/// The computed diff of one version against the current buffer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffView {
    /// The version diffed against the current buffer.
    pub id: u64,
    /// Total differing bytes.
    pub byte_count: usize,
    /// Human-readable lines from [`history::spans::describe`].
    pub lines: Vec<String>,
}

#[derive(Default)]
pub struct HistoryState {
    /// Row being renamed: (version id, text being edited).
    pub editing: Option<(u64, String)>,
    pub diff: Option<DiffView>,
}

/// `1234 B` / `32 KiB` for the size column.
pub fn format_size(bytes: u64) -> String {
    if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KiB", bytes / 1024)
    } else {
        format!("{bytes} B")
    }
}

pub fn ui(
    ui: &mut egui::Ui,
    state: &mut HistoryState,
    versions: &[VersionRow],
    enabled: bool,
    actions: &mut Vec<HistoryAction>,
) {
    ui.heading("History");
    ui.add_space(4.0);
    if !enabled {
        ui.colored_label(
            ui.visuals().warn_fg_color,
            "Version history is off — new saves are not recorded (File -> History).",
        );
        ui.add_space(4.0);
    }
    if versions.is_empty() {
        ui.label("No versions yet. Every Save records a restorable snapshot here.");
        return;
    }

    let text_height = egui::TextStyle::Body.resolve(ui.style()).size + 8.0;
    let table = TableBuilder::new(ui)
        .id_salt("history_table")
        .striped(true)
        .column(Column::exact(28.0)) // version id
        .column(Column::exact(140.0)) // time
        .column(Column::exact(52.0)) // origin
        .column(Column::exact(56.0)) // size
        .column(Column::initial(160.0).at_least(80.0)) // name
        .column(Column::remainder()) // summary
        .column(Column::exact(230.0)); // actions

    table
        .header(text_height, |mut header| {
            for title in ["#", "Time", "Origin", "Size", "Name", "Save", "Actions"] {
                header.col(|ui| {
                    ui.strong(title);
                });
            }
        })
        .body(|body| {
            // Newest first.
            body.rows(text_height, versions.len(), |mut row| {
                let version = &versions[versions.len() - 1 - row.index()];
                let entry = &version.entry;
                let weak = !version.blob_ok;
                let label = |ui: &mut egui::Ui, text: &str| {
                    if weak {
                        ui.weak(text);
                    } else {
                        ui.label(text);
                    }
                };
                row.col(|ui| label(ui, &entry.id.to_string()));
                row.col(|ui| label(ui, &history::format_timestamp(entry.timestamp)));
                row.col(|ui| label(ui, entry.origin.label()));
                row.col(|ui| label(ui, &format_size(entry.size)));
                row.col(|ui| {
                    name_cell(ui, state, entry.id, entry.label.as_deref(), actions);
                });
                row.col(|ui| {
                    if version.blob_ok {
                        label(ui, version.summary.as_deref().unwrap_or("—"));
                    } else {
                        ui.colored_label(ui.visuals().error_fg_color, "snapshot missing")
                            .on_hover_text(
                                "The manifest references a snapshot blob that is gone \
                                 from the history directory; only Delete is possible.",
                            );
                    }
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        let ok = version.blob_ok;
                        if ui
                            .add_enabled(ok, egui::Button::new("Restore").small())
                            .on_hover_text(
                                "Load this version into the editor. Nothing is written \
                                 to disk until you save.",
                            )
                            .clicked()
                        {
                            actions.push(HistoryAction::Restore(entry.id));
                        }
                        if ui
                            .add_enabled(ok, egui::Button::new("Diff").small())
                            .on_hover_text("Compare with the current buffer")
                            .clicked()
                        {
                            actions.push(HistoryAction::Diff(entry.id));
                        }
                        if ui
                            .add_enabled(ok, egui::Button::new("Export").small())
                            .on_hover_text("Save a copy of this version elsewhere")
                            .clicked()
                        {
                            actions.push(HistoryAction::Export(entry.id));
                        }
                        if ui.add(egui::Button::new("Delete").small()).clicked() {
                            actions.push(HistoryAction::Delete(entry.id));
                        }
                    });
                });
            });
        });

    let mut close_diff = false;
    if let Some(diff) = &state.diff {
        ui.add_space(8.0);
        ui.separator();
        ui.horizontal(|ui| {
            ui.strong(format!("Diff: version {} vs current buffer", diff.id));
            if ui.small_button("×").on_hover_text("Close diff").clicked() {
                close_diff = true;
            }
        });
        if diff.byte_count == 0 {
            ui.label("No differences — the version matches the current buffer.");
        } else {
            let plural = if diff.byte_count == 1 { "" } else { "s" };
            ui.label(format!("{} byte{plural} differ:", diff.byte_count));
            egui::ScrollArea::vertical()
                .max_height(160.0)
                .show(ui, |ui| {
                    for line in &diff.lines {
                        ui.monospace(line);
                    }
                });
        }
    }
    if close_diff {
        state.diff = None;
    }
}

/// The name column: the label with a rename button, or an inline edit.
fn name_cell(
    ui: &mut egui::Ui,
    state: &mut HistoryState,
    id: u64,
    label: Option<&str>,
    actions: &mut Vec<HistoryAction>,
) {
    if let Some((editing_id, text)) = &mut state.editing {
        if *editing_id == id {
            let response = ui.add(
                egui::TextEdit::singleline(text)
                    .desired_width(110.0)
                    .hint_text("version name"),
            );
            let submit = ui.small_button("OK").clicked()
                || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if submit {
                let trimmed = text.trim();
                let new_label = (!trimmed.is_empty()).then(|| trimmed.to_owned());
                actions.push(HistoryAction::SetLabel(id, new_label));
                state.editing = None;
            } else if ui.small_button("×").on_hover_text("Cancel").clicked() {
                state.editing = None;
            }
            return;
        }
    }
    match label {
        Some(name) => {
            ui.label(name)
                .on_hover_text("Named versions are never auto-pruned");
        }
        None => {
            ui.weak("—");
        }
    }
    if ui
        .small_button("✏")
        .on_hover_text("Name / rename this version")
        .clicked()
    {
        state.editing = Some((id, label.unwrap_or_default().to_owned()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_format_in_bytes_and_kib() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1000), "1000 B");
        assert_eq!(format_size(32_768), "32 KiB");
        assert_eq!(format_size(32_769), "32769 B");
    }
}
