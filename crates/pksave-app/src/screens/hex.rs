//! Hex screen: a virtualized 16-bytes-per-row view of the serialized
//! save with region coloring, changed-byte highlighting and optional
//! click-to-edit.

use egui_extras::{Column, TableBuilder};
use pksave::gen1::offsets;

use crate::app::Doc;

const BYTES_PER_ROW: usize = 16;

#[derive(Default)]
pub struct HexState {
    pub edit_enabled: bool,
    pub jump_text: String,
    pub pending_scroll: Option<usize>,
    pub selected: Option<usize>,
    pub edit_text: String,
}

impl HexState {
    /// Queue a scroll to `offset` for the next time the screen shows.
    pub fn scroll_to(&mut self, offset: usize) {
        self.pending_scroll = Some(offset);
        self.selected = Some(offset);
    }
}

/// The save-file regions the viewer color-codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    Bank0,
    MainData,
    Party,
    CurrentBox,
    Checksum,
    BoxBank,
    Other,
}

impl Region {
    pub fn name(self) -> &'static str {
        match self {
            Region::Bank0 => "bank 0 (HoF, unchecksummed)",
            Region::MainData => "main data (checksummed)",
            Region::Party => "party",
            Region::CurrentBox => "current box (working copy)",
            Region::Checksum => "checksum bytes",
            Region::BoxBank => "box banks 2-3",
            Region::Other => "tail / padding",
        }
    }

    fn color(self, dark: bool) -> egui::Color32 {
        // One hue family per region, readable on both themes.
        match (self, dark) {
            (Region::Bank0, true) => egui::Color32::from_gray(150),
            (Region::Bank0, false) => egui::Color32::from_gray(110),
            (Region::MainData, true) => egui::Color32::from_rgb(130, 180, 255),
            (Region::MainData, false) => egui::Color32::from_rgb(30, 90, 200),
            (Region::Party, true) => egui::Color32::from_rgb(140, 220, 140),
            (Region::Party, false) => egui::Color32::from_rgb(20, 130, 20),
            (Region::CurrentBox, true) => egui::Color32::from_rgb(110, 210, 210),
            (Region::CurrentBox, false) => egui::Color32::from_rgb(0, 130, 130),
            (Region::Checksum, true) => egui::Color32::from_rgb(240, 140, 240),
            (Region::Checksum, false) => egui::Color32::from_rgb(160, 30, 160),
            (Region::BoxBank, true) => egui::Color32::from_rgb(230, 200, 120),
            (Region::BoxBank, false) => egui::Color32::from_rgb(150, 110, 10),
            (Region::Other, true) => egui::Color32::from_gray(120),
            (Region::Other, false) => egui::Color32::from_gray(140),
        }
    }
}

/// Region of a file offset. Most specific wins (party and the current
/// box live inside the main checksummed region).
pub fn region_of(offset: usize) -> Region {
    let in_ = |start: usize, len: usize| (start..start + len).contains(&offset);
    if offset == offsets::MAIN_CHECKSUM
        || in_(offsets::BANK2_ALL_BOXES_CHECKSUM, 7)
        || in_(offsets::BANK3_ALL_BOXES_CHECKSUM, 7)
    {
        Region::Checksum
    } else if in_(offsets::PARTY, offsets::PARTY_LEN) {
        Region::Party
    } else if in_(offsets::CURRENT_BOX, offsets::BOX_LEN) {
        Region::CurrentBox
    } else if (offsets::CHECKSUM_REGION_START..=offsets::CHECKSUM_REGION_END).contains(&offset) {
        Region::MainData
    } else if offset < offsets::BANK_SIZE {
        Region::Bank0
    } else if in_(offsets::BANK2_BOXES, offsets::BANK_SIZE * 2) && offset < offsets::SRAM_SIZE {
        Region::BoxBank
    } else {
        Region::Other
    }
}

/// Whether `offset` falls in any of the (sorted, disjoint) ranges.
pub fn in_ranges(ranges: &[std::ops::Range<usize>], offset: usize) -> bool {
    let i = ranges.partition_point(|r| r.end <= offset);
    ranges.get(i).is_some_and(|r| r.contains(&offset))
}

/// The classic printable-ASCII gutter for one row of bytes.
pub fn ascii_gutter(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                '·'
            }
        })
        .collect()
}

/// Parse the jump-to-offset text: hex digits with an optional `0x`/`0X`
/// prefix and surrounding whitespace. `None` when unparsable or `>= len`.
pub fn parse_jump(text: &str, len: usize) -> Option<usize> {
    let t = text.trim();
    let t = t
        .strip_prefix("0x")
        .or_else(|| t.strip_prefix("0X"))
        .unwrap_or(t);
    let offset = usize::from_str_radix(t, 16).ok()?;
    (offset < len).then_some(offset)
}

pub fn ui(ui: &mut egui::Ui, doc: &mut Doc, state: &mut HexState) {
    ui.heading("Hex");
    ui.add_space(4.0);

    // The bytes exactly as they would be saved (checksums included) —
    // cached on the Doc and refreshed after each touched frame, so this
    // does not re-serialize the buffer every frame.
    let bytes: &[u8] = doc.serialized();
    let rows = bytes.len().div_ceil(BYTES_PER_ROW);
    let dark = ui.visuals().dark_mode;
    let mut write: Option<(usize, u8)> = None;

    // ---- toolbar ----
    ui.horizontal(|ui| {
        ui.checkbox(&mut state.edit_enabled, "Enable editing")
            .on_hover_text(
                "Click a byte, then type a hex value. A hand-edited checksum byte is \
                 PINNED: it is kept verbatim on save (W-CHECKSUM warns if it mismatches \
                 the data); Repair → Fix all checksums unpins and repairs it.",
            );
        ui.separator();
        ui.label("Jump to offset (hex):");
        let response = ui.add(
            egui::TextEdit::singleline(&mut state.jump_text)
                .desired_width(70.0)
                .font(egui::TextStyle::Monospace),
        );
        let go = ui.button("Go").clicked()
            || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if go {
            if let Some(offset) = parse_jump(&state.jump_text, bytes.len()) {
                state.scroll_to(offset);
            }
        }
        ui.separator();
        ui.label(format!("{} byte(s) differ from the loaded file", {
            doc.changed.iter().map(|r| r.len()).sum::<usize>()
        }));
    });

    // ---- legend ----
    ui.horizontal_wrapped(|ui| {
        for region in [
            Region::Bank0,
            Region::MainData,
            Region::Party,
            Region::CurrentBox,
            Region::BoxBank,
            Region::Checksum,
        ] {
            ui.colored_label(region.color(dark), "■");
            ui.weak(region.name());
            ui.add_space(8.0);
        }
        ui.colored_label(changed_color(dark), "■");
        ui.weak("changed");
    });

    // ---- edit bar ----
    if state.edit_enabled {
        if let Some(offset) = state.selected.filter(|&o| o < bytes.len()) {
            ui.horizontal(|ui| {
                ui.monospace(format!(
                    "0x{offset:04X} = 0x{:02X} ({})",
                    bytes[offset],
                    region_of(offset).name()
                ));
                ui.label("new hex value:");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.edit_text)
                        .desired_width(36.0)
                        .char_limit(2)
                        .font(egui::TextStyle::Monospace),
                );
                let parsed = u8::from_str_radix(state.edit_text.trim(), 16);
                let apply = ui.button("Apply").clicked()
                    || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
                match parsed {
                    Ok(value) if apply => {
                        write = Some((offset, value));
                        state.edit_text.clear();
                    }
                    Err(_) if !state.edit_text.is_empty() => {
                        ui.colored_label(ui.visuals().error_fg_color, "not hex");
                    }
                    _ => {}
                }
            });
        } else {
            ui.weak("Click a byte to edit it.");
        }
    }
    ui.add_space(4.0);

    // ---- table ----
    let text_height = egui::TextStyle::Monospace.resolve(ui.style()).size + 4.0;
    let mut table = TableBuilder::new(ui)
        .striped(true)
        .column(Column::exact(60.0));
    for _ in 0..BYTES_PER_ROW {
        table = table.column(Column::exact(22.0));
    }
    table = table.column(Column::exact(140.0));

    if let Some(offset) = state.pending_scroll.take() {
        table = table.scroll_to_row(offset / BYTES_PER_ROW, Some(egui::Align::Center));
    }

    let mut clicked: Option<usize> = None;
    let selected = state.selected;
    let edit_enabled = state.edit_enabled;
    table
        .header(text_height, |mut header| {
            header.col(|ui| {
                ui.monospace("offset");
            });
            for i in 0..BYTES_PER_ROW {
                header.col(|ui| {
                    ui.monospace(format!("{i:X}"));
                });
            }
            header.col(|ui| {
                ui.monospace("ascii");
            });
        })
        .body(|body| {
            body.rows(text_height, rows, |mut row| {
                let base = row.index() * BYTES_PER_ROW;
                row.col(|ui| {
                    ui.monospace(format!("0x{base:04X}"));
                });
                for i in 0..BYTES_PER_ROW {
                    let offset = base + i;
                    row.col(|ui| {
                        let Some(&byte) = bytes.get(offset) else {
                            return;
                        };
                        let changed = in_ranges(&doc.changed, offset);
                        let color = if changed {
                            changed_color(dark)
                        } else {
                            region_of(offset).color(dark)
                        };
                        let mut text = egui::RichText::new(format!("{byte:02X}"))
                            .monospace()
                            .color(color);
                        if changed {
                            text = text.strong();
                        }
                        if selected == Some(offset) {
                            text = text.underline();
                        }
                        let label = if edit_enabled {
                            ui.add(egui::Label::new(text).sense(egui::Sense::click()))
                        } else {
                            ui.add(egui::Label::new(text))
                        };
                        if label.clicked() {
                            clicked = Some(offset);
                        }
                    });
                }
                row.col(|ui| {
                    let end = (base + BYTES_PER_ROW).min(bytes.len());
                    ui.monospace(ascii_gutter(&bytes[base..end]));
                });
            });
        });

    if let Some(offset) = clicked {
        state.selected = Some(offset);
        state.edit_text = format!("{:02X}", bytes.get(offset).copied().unwrap_or(0));
    }

    // Deferred past the table so the render borrows the cached bytes.
    if let Some((offset, value)) = write {
        // In-place raw edit; a write to a stored checksum byte pins it
        // (see the toolbar tooltip). Out-of-range cannot happen here —
        // `offset` was validated against the rendered buffer.
        if doc.save.set_byte(offset, value).is_ok() {
            doc.touch();
        }
    }
}

fn changed_color(dark: bool) -> egui::Color32 {
    if dark {
        egui::Color32::from_rgb(255, 120, 100)
    } else {
        egui::Color32::from_rgb(200, 40, 20)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ascii_gutter_prints_printables_and_dots() {
        assert_eq!(ascii_gutter(b"AZaz09 ~"), "AZaz09 ~");
        assert_eq!(ascii_gutter(&[0x00, 0x1F, 0x7F, 0xFF]), "····");
        assert_eq!(ascii_gutter(&[]), "");
        // A short trailing row formats without panicking.
        assert_eq!(ascii_gutter(&[0x41, 0x00]), "A·");
    }

    #[test]
    fn in_ranges_uses_binary_search_correctly() {
        let ranges = vec![2..4, 10..11, 20..32];
        for (offset, expected) in [
            (0, false),
            (1, false),
            (2, true),
            (3, true),
            (4, false),
            (9, false),
            (10, true),
            (11, false),
            (19, false),
            (20, true),
            (31, true),
            (32, false),
            (100, false),
        ] {
            assert_eq!(in_ranges(&ranges, offset), expected, "offset {offset}");
        }
        assert!(!in_ranges(&[], 5));
    }

    #[test]
    fn regions_are_classified() {
        assert_eq!(region_of(0x0000), Region::Bank0);
        assert_eq!(region_of(offsets::HALL_OF_FAME), Region::Bank0);
        assert_eq!(region_of(offsets::PLAYER_NAME), Region::MainData);
        assert_eq!(region_of(offsets::PARTY), Region::Party);
        assert_eq!(region_of(offsets::CURRENT_BOX), Region::CurrentBox);
        assert_eq!(region_of(offsets::MAIN_CHECKSUM), Region::Checksum);
        assert_eq!(
            region_of(offsets::BANK2_ALL_BOXES_CHECKSUM),
            Region::Checksum
        );
        assert_eq!(
            region_of(offsets::BANK2_ALL_BOXES_CHECKSUM + 6),
            Region::Checksum
        );
        assert_eq!(
            region_of(offsets::BANK3_ALL_BOXES_CHECKSUM + 6),
            Region::Checksum
        );
        assert_eq!(region_of(offsets::BANK2_BOXES), Region::BoxBank);
        assert_eq!(region_of(offsets::BANK3_BOXES), Region::BoxBank);
        assert_eq!(region_of(offsets::SRAM_SIZE), Region::Other);
        // The gap between the main region and bank 2.
        assert_eq!(region_of(0x3600), Region::Other);
    }

    #[test]
    fn parse_jump_accepts_plain_and_prefixed_hex() {
        assert_eq!(parse_jump("2f2c", 0x8000), Some(0x2F2C));
        assert_eq!(parse_jump("2F2C", 0x8000), Some(0x2F2C));
        assert_eq!(parse_jump("0x2f2c", 0x8000), Some(0x2F2C));
        assert_eq!(parse_jump("0X2F2C", 0x8000), Some(0x2F2C));
        assert_eq!(parse_jump("0", 0x8000), Some(0));
    }

    #[test]
    fn parse_jump_trims_whitespace() {
        assert_eq!(parse_jump("  1a2b  ", 0x8000), Some(0x1A2B));
        assert_eq!(parse_jump("\t0x10\n", 0x8000), Some(0x10));
    }

    #[test]
    fn parse_jump_rejects_out_of_range() {
        assert_eq!(parse_jump("8000", 0x8000), None);
        assert_eq!(parse_jump("7fff", 0x8000), Some(0x7FFF));
        assert_eq!(parse_jump("0", 0), None);
    }

    #[test]
    fn parse_jump_rejects_garbage() {
        assert_eq!(parse_jump("", 0x8000), None);
        assert_eq!(parse_jump("0x", 0x8000), None);
        assert_eq!(parse_jump("wxyz", 0x8000), None);
        assert_eq!(parse_jump("12 34", 0x8000), None);
        assert_eq!(parse_jump("-5", 0x8000), None);
    }

    #[test]
    fn hex_row_count_covers_partial_rows() {
        assert_eq!(100usize.div_ceil(BYTES_PER_ROW), 7);
        assert_eq!(0x8000usize.div_ceil(BYTES_PER_ROW), 2048);
    }
}
