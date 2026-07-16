//! Shared widgets: Gen 1 name editor, species/move/item selectors, BCD
//! money editor and small numeric steppers.

use pksave::gen1::data::{
    DEX_TO_INDEX, INDEX_TO_DEX, ITEM_NAMES, MOVES, SPECIES_NAMES, TYPE_NAMES,
};
use pksave::gen1::offsets::NAME_LEN;
use pksave::gen1::text::{self, TextError};

/// Validate a name for a standard 11-byte field (10 chars + terminator).
pub fn validate_name(s: &str) -> Result<(), TextError> {
    text::encode(s, NAME_LEN).map(|_| ())
}

/// What [`name_edit`] should do with the in-progress buffer this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NameOutcome {
    /// Apply the buffer to the save: it just changed, encodes, and
    /// differs from the stored name.
    Apply,
    /// Overwrite the buffer with the stored name (external change, e.g.
    /// Revert, while the field is not being edited).
    Resync,
    /// Leave both alone (unchanged, invalid, or mid-edit).
    Keep,
}

/// The pure decision step of [`name_edit`]: `Apply` only on
/// changed + valid + different.
pub fn name_outcome(buf: &str, current: &str, changed: bool, has_focus: bool) -> NameOutcome {
    if changed {
        if validate_name(buf).is_ok() && buf != current {
            NameOutcome::Apply
        } else {
            NameOutcome::Keep
        }
    } else if !has_focus && buf != current {
        NameOutcome::Resync
    } else {
        NameOutcome::Keep
    }
}

/// Text editor for a Gen 1 name field. Keeps the in-progress string in
/// egui temp memory; returns `Some(new_name)` only when the content is
/// encodable and differs from `current`. Invalid text gets a red outline
/// and an explanatory hover text and is never applied.
pub fn name_edit(ui: &mut egui::Ui, id_salt: impl egui::AsIdSalt, current: &str) -> Option<String> {
    let id = ui.make_persistent_id(id_salt);
    let mut buf: String = ui
        .data_mut(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| current.to_owned());

    let error = validate_name(&buf).err();
    let response = ui.add(egui::TextEdit::singleline(&mut buf).desired_width(120.0));

    let mut result = None;
    match name_outcome(&buf, current, response.changed(), response.has_focus()) {
        NameOutcome::Apply => result = Some(buf.clone()),
        NameOutcome::Resync => buf = current.to_owned(),
        NameOutcome::Keep => {}
    }

    if let Some(error) = error {
        ui.painter().rect_stroke(
            response.rect,
            2.0,
            egui::Stroke::new(1.5, ui.visuals().error_fg_color),
            egui::StrokeKind::Outside,
        );
        response.on_hover_text(format!("Not writable to the save: {error}"));
    } else {
        response.on_hover_text("Gen 1 charset, up to 10 characters");
    }

    ui.data_mut(|d| d.insert_temp(id, buf));
    result
}

/// Label for an internal species index: the dex name, `(none)` for the
/// empty byte 0x00 (e.g. an unset starter on a fresh save), or a hex
/// marker for MissingNo/glitch indexes.
pub fn species_label(internal: u8) -> String {
    if internal == 0 {
        return "(none)".to_owned();
    }
    let dex = INDEX_TO_DEX[usize::from(internal)];
    if dex == 0 {
        format!("glitch 0x{internal:02X}")
    } else {
        format!("#{dex:03} {}", SPECIES_NAMES[usize::from(dex)])
    }
}

/// Species picker over the 151 real Pokémon in dex order. Returns the
/// newly selected *internal* index, if changed.
pub fn species_combo(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    current_internal: u8,
) -> Option<u8> {
    let mut picked = None;
    egui::ComboBox::from_id_salt(ui.id().with(id_salt))
        .selected_text(species_label(current_internal))
        .width(170.0)
        .show_ui(ui, |ui| {
            for dex in 1..=151u8 {
                let internal = DEX_TO_INDEX[usize::from(dex)];
                let selected = internal == current_internal;
                let label = format!("#{dex:03} {}", SPECIES_NAMES[usize::from(dex)]);
                if ui.selectable_label(selected, label).clicked() && !selected {
                    picked = Some(internal);
                }
            }
        });
    picked
}

/// Searchable selector over `(id, label)` options: a combo whose popup
/// starts with a filter box. Returns the newly picked id, if any.
pub fn search_combo(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    selected_text: &str,
    options: &[(u16, String)],
) -> Option<u16> {
    let combo_id = ui.id().with(id_salt);
    let filter_id = combo_id.with("filter");
    let open_id = combo_id.with("was_open");
    // Whether the popup was already open last frame: on the first open
    // frame the filter starts fresh and grabs keyboard focus.
    let was_open = ui
        .data_mut(|d| d.get_temp::<bool>(open_id))
        .unwrap_or(false);
    let mut is_open = false;
    let mut picked = None;
    egui::ComboBox::from_id_salt(combo_id)
        .selected_text(selected_text)
        .width(170.0)
        // The default CloseOnClick would close the popup when the filter
        // box itself is clicked; keep it open until a selection (closed
        // manually below) or a click outside.
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show_ui(ui, |ui| {
            is_open = true;
            let mut filter: String = if was_open {
                ui.data_mut(|d| d.get_temp::<String>(filter_id))
                    .unwrap_or_default()
            } else {
                String::new()
            };
            let response = ui.add(egui::TextEdit::singleline(&mut filter).hint_text("filter…"));
            if !was_open {
                // Freshly opened: type into the filter right away.
                response.request_focus();
            }
            let needle = filter.to_lowercase();
            egui::ScrollArea::vertical()
                .max_height(240.0)
                .show(ui, |ui| {
                    for (id, label) in options {
                        if !needle.is_empty() && !label.to_lowercase().contains(&needle) {
                            continue;
                        }
                        if ui.selectable_label(false, label.as_str()).clicked() {
                            picked = Some(*id);
                        }
                    }
                });
            ui.data_mut(|d| d.insert_temp(filter_id, filter));
            if picked.is_some() {
                // CloseOnClickOutside keeps the popup open on inside
                // clicks — close it explicitly after a selection.
                ui.close();
            }
        });
    ui.data_mut(|d| d.insert_temp(open_id, is_open));
    if picked.is_some() {
        ui.data_mut(|d| d.remove_temp::<String>(filter_id));
    }
    picked
}

/// All valid item ids with display labels, for [`search_combo`].
pub fn item_options() -> Vec<(u16, String)> {
    ITEM_NAMES
        .iter()
        .enumerate()
        .filter(|(_, name)| !name.is_empty())
        .map(|(id, name)| (id as u16, format!("0x{id:02X} {name}")))
        .collect()
}

/// Display label for an item id.
pub fn item_label(id: u8) -> String {
    let name = ITEM_NAMES[usize::from(id)];
    if name.is_empty() {
        format!("unknown 0x{id:02X}")
    } else {
        name.to_owned()
    }
}

/// Move ids 0 ("—") plus 1..=165 with names, for [`search_combo`].
pub fn move_options() -> Vec<(u16, String)> {
    let mut out = vec![(0u16, "— none —".to_owned())];
    out.extend(
        MOVES
            .iter()
            .enumerate()
            .skip(1)
            .map(|(id, info)| (id as u16, format!("{} ({} PP)", info.name, info.pp))),
    );
    out
}

/// Display label for a move id.
pub fn move_label(id: u8) -> String {
    match usize::from(id) {
        0 => "— none —".to_owned(),
        i if i < MOVES.len() => MOVES[i].name.to_owned(),
        _ => format!("glitch 0x{id:02X}"),
    }
}

/// Display name for a Gen 1 type id.
pub fn type_label(type_id: u8) -> &'static str {
    TYPE_NAMES
        .get(usize::from(type_id))
        .copied()
        .filter(|n| !n.is_empty())
        .unwrap_or("?")
}

/// A `DragValue` over a `u8` with a closed range. Returns `true` when
/// the value changed.
pub fn byte_stepper(
    ui: &mut egui::Ui,
    value: &mut u8,
    range: std::ops::RangeInclusive<u8>,
) -> bool {
    ui.add(egui::DragValue::new(value).range(range)).changed()
}

/// A `DragValue` over a `u16`. Returns `true` when the value changed.
pub fn word_stepper(
    ui: &mut egui::Ui,
    value: &mut u16,
    range: std::ops::RangeInclusive<u16>,
) -> bool {
    ui.add(egui::DragValue::new(value).range(range)).changed()
}

/// BCD money/coins editor: a clamped `DragValue` in decimal. Returns the
/// new value when changed.
pub fn bcd_editor(ui: &mut egui::Ui, current: u32, max: u32) -> Option<u32> {
    let mut value = current.min(max);
    if ui
        .add(egui::DragValue::new(&mut value).range(0..=max).speed(50))
        .changed()
    {
        Some(value)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_name_accepts_gen1_text() {
        assert!(validate_name("RED").is_ok());
        assert!(validate_name("Weepinbell").is_ok());
        assert!(validate_name("MR.MIME").is_ok());
        assert!(validate_name("'d'l's").is_ok());
        assert!(validate_name("").is_ok());
        // Exactly 10 characters fits (field is 11 with terminator).
        assert!(validate_name("ABCDEFGHIJ").is_ok());
    }

    #[test]
    fn validate_name_rejects_unencodable_and_too_long() {
        assert!(matches!(
            validate_name("naïve"),
            Err(TextError::Unencodable('ï'))
        ));
        assert!(matches!(
            validate_name("ABCDEFGHIJK"),
            Err(TextError::TooLong { .. })
        ));
        // '@' is the terminator glyph and deliberately unencodable.
        assert!(validate_name("A@B").is_err());
    }

    #[test]
    fn name_outcome_applies_only_changed_valid_different() {
        // Valid, changed, different: apply.
        assert_eq!(name_outcome("RED", "BLUE", true, true), NameOutcome::Apply);
        // Invalid text is never applied.
        assert_eq!(name_outcome("naïve", "BLUE", true, true), NameOutcome::Keep);
        assert_eq!(
            name_outcome("ABCDEFGHIJK", "BLUE", true, true),
            NameOutcome::Keep
        );
        // Changed back to the stored value: nothing to apply.
        assert_eq!(name_outcome("BLUE", "BLUE", true, true), NameOutcome::Keep);
    }

    #[test]
    fn name_outcome_resyncs_external_changes_when_unfocused() {
        // Not editing, buffer differs (e.g. after Revert): resync.
        assert_eq!(
            name_outcome("OLD", "REVERTED", false, false),
            NameOutcome::Resync
        );
        // Mid-edit (focused): leave the user's buffer alone.
        assert_eq!(name_outcome("OL", "OLD", false, true), NameOutcome::Keep);
        // In sync: nothing to do.
        assert_eq!(name_outcome("RED", "RED", false, false), NameOutcome::Keep);
    }

    #[test]
    fn species_labels() {
        // Internal 0x99 is Bulbasaur (#001).
        assert_eq!(species_label(0x99), "#001 BULBASAUR");
        // Internal 0x1F maps to no dex entry (MissingNo family).
        assert_eq!(species_label(0x1F), "glitch 0x1F");
        // 0x00 is the empty byte (unset starter on a fresh save), not a
        // glitch species.
        assert_eq!(species_label(0x00), "(none)");
    }

    /// Every non-ASCII symbol the UI renders must have a glyph in egui's
    /// default fonts — anything missing shows up as a tofu box (□).
    #[test]
    fn ui_symbols_have_glyphs_in_default_fonts() {
        use egui::text::{FontDefinitions, Fonts};

        // Symbols used in proportional text (labels, buttons, headings).
        const PROPORTIONAL: &str = "—·…é⚠🗑📂✚⬆⬇■💾★✏ℹ⛔➡•×∑↻≠";
        // Symbols used in monospace text (hex ASCII gutter, diff lines).
        const MONOSPACE: &str = "·é—";

        let mut fonts = Fonts::new(
            egui::epaint::text::TextOptions::default(),
            FontDefinitions::default(),
        );
        for (family, symbols) in [
            (egui::FontFamily::Proportional, PROPORTIONAL),
            (egui::FontFamily::Monospace, MONOSPACE),
        ] {
            let coverage = fonts.fonts.font(&family).characters().clone();
            for c in symbols.chars() {
                assert!(
                    coverage.contains_key(&c),
                    "U+{:04X} {c:?} has no glyph in the default {family:?} fonts \
                     and would render as tofu",
                    c as u32
                );
            }
        }
    }

    #[test]
    fn option_tables_are_populated() {
        assert!(item_options().len() > 90);
        assert_eq!(move_options().len(), 166);
        assert_eq!(move_options()[1].1, "POUND (35 PP)");
        assert_eq!(type_label(0), "NORMAL");
        assert_eq!(type_label(0x14), "FIRE");
        assert_eq!(type_label(0x09), "?");
        assert_eq!(type_label(0xFF), "?");
    }
}
