//! The save-file buffer: verbatim byte retention plus lazy checksum repair.
//!
//! [`SaveFile`] wraps the raw `.sav`/`.srm` bytes. The anti-corruption
//! contract (see `lib.rs` and `docs/FORMAT.md`):
//!
//! - Loading only checks the length; corrupt content loads fine and is
//!   reported through [`SaveGame::diagnostics`] instead.
//! - An *untouched* file serializes back byte-identically, even if its
//!   stored checksums are wrong.
//! - Once anything was edited (any mutable buffer access), [`to_bytes`]
//!   recomputes all 15 checksums so the game accepts the file.
//! - Bytes past the 32 KiB SRAM image (emulator padding, RTC footers) are
//!   always preserved verbatim.
//!
//! [`to_bytes`]: SaveFile::to_bytes

use core::ops::Range;

use thiserror::Error;

use super::{checksum, offsets};
use crate::{Diagnostic, SaveGame, Severity};

/// Failure to load a save image. The only fatal condition is a buffer too
/// short to be a 32 KiB SRAM dump.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoadError {
    #[error("save file is {len} bytes; a Gen 1 save is at least 0x8000 (32768) bytes")]
    TooShort { len: usize },
}

/// A loaded Gen 1 save file. All input bytes are retained verbatim.
#[derive(Debug, Clone)]
pub struct SaveFile {
    raw: Vec<u8>,
    /// Set on any mutable access; gates checksum recomputation in
    /// [`SaveFile::to_bytes`].
    edited: bool,
}

impl SaveFile {
    /// Wrap raw save bytes. Errors only if `bytes` is shorter than
    /// [`offsets::SRAM_SIZE`]; longer files (64 KiB pads, RTC footers)
    /// load fine and keep their tail.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<SaveFile, LoadError> {
        if bytes.len() < offsets::SRAM_SIZE {
            return Err(LoadError::TooShort { len: bytes.len() });
        }
        Ok(SaveFile {
            raw: bytes,
            edited: false,
        })
    }

    /// The current buffer, including any tail past the SRAM image.
    pub fn as_bytes(&self) -> &[u8] {
        &self.raw
    }

    /// Read access for field-editor modules (items, trainer, party, …).
    #[allow(dead_code)] // consumed by the field-editor milestones
    pub(crate) fn buf(&self) -> &[u8] {
        &self.raw
    }

    /// Write access for field-editor modules. Marks the file edited.
    #[allow(dead_code)] // consumed by the field-editor milestones
    pub(crate) fn buf_mut(&mut self) -> &mut [u8] {
        self.edited = true;
        &mut self.raw
    }

    /// Explicitly mark the file edited, so [`SaveFile::to_bytes`]
    /// recomputes checksums.
    pub fn mark_edited(&mut self) {
        self.edited = true;
    }

    /// Whether any mutable access has happened since loading.
    pub fn is_edited(&self) -> bool {
        self.edited
    }

    /// Recompute and store all 15 checksums now — the explicit opt-in for
    /// repairing a file that was already corrupt on load. Marks the file
    /// edited.
    pub fn fix_checksums(&mut self) {
        self.edited = true;
        checksum::fix_all(&mut self.raw);
    }

    /// Serialize. Returns the buffer verbatim if nothing was edited;
    /// otherwise a copy with all 15 checksums recomputed. The tail past
    /// the SRAM image is always preserved.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = self.raw.clone();
        if self.edited {
            checksum::fix_all(&mut out);
        }
        out
    }

    /// Human-readable label for this save.
    pub fn game_label(&self) -> &'static str {
        "Pokémon Red/Blue/Yellow (Gen 1)"
    }

    /// Non-fatal findings: one warning per checksum mismatch.
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        const BOX_CODES: [&str; offsets::NUM_BOXES] = [
            "W-CHECKSUM-BOX1",
            "W-CHECKSUM-BOX2",
            "W-CHECKSUM-BOX3",
            "W-CHECKSUM-BOX4",
            "W-CHECKSUM-BOX5",
            "W-CHECKSUM-BOX6",
            "W-CHECKSUM-BOX7",
            "W-CHECKSUM-BOX8",
            "W-CHECKSUM-BOX9",
            "W-CHECKSUM-BOX10",
            "W-CHECKSUM-BOX11",
            "W-CHECKSUM-BOX12",
        ];
        checksum::verify(&self.raw)
            .into_iter()
            .map(|m| {
                let (code, what) = match m.region {
                    checksum::Region::Main => ("W-CHECKSUM-MAIN", "main data".to_string()),
                    checksum::Region::Bank2AllBoxes => {
                        ("W-CHECKSUM-BOXBANK2", "bank 2 all-boxes".to_string())
                    }
                    checksum::Region::Bank3AllBoxes => {
                        ("W-CHECKSUM-BOXBANK3", "bank 3 all-boxes".to_string())
                    }
                    checksum::Region::Box(n) => (BOX_CODES[n], format!("box {}", n + 1)),
                };
                let at = m.region.checksum_offset();
                Diagnostic {
                    severity: Severity::Warning,
                    code,
                    message: format!(
                        "{what} checksum mismatch: stored 0x{:02X}, computed 0x{:02X}",
                        m.stored, m.computed
                    ),
                    span: Some(at..at + 1),
                }
            })
            .collect()
    }
}

impl SaveGame for SaveFile {
    fn game_label(&self) -> &str {
        SaveFile::game_label(self)
    }

    fn diagnostics(&self) -> Vec<Diagnostic> {
        SaveFile::diagnostics(self)
    }

    fn to_bytes(&self) -> Vec<u8> {
        SaveFile::to_bytes(self)
    }
}

/// Maximal ranges where `original` and `current` differ (for UI dirty
/// highlighting). If the lengths differ, the trailing
/// `min(len)..max(len)` region counts as changed (merged with a
/// preceding range when adjacent).
pub fn changed_ranges(original: &[u8], current: &[u8]) -> Vec<Range<usize>> {
    let common = original.len().min(current.len());
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut run_start: Option<usize> = None;
    for (i, (&a, &b)) in original.iter().zip(current).enumerate() {
        if a != b {
            run_start.get_or_insert(i);
        } else if let Some(start) = run_start.take() {
            ranges.push(start..i);
        }
    }
    if let Some(start) = run_start {
        ranges.push(start..common);
    }
    let longest = original.len().max(current.len());
    if longest > common {
        match ranges.last_mut() {
            Some(last) if last.end == common => last.end = longest,
            _ => ranges.push(common..longest),
        }
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gen1::checksum;
    use crate::gen1::offsets;

    /// Deliberately non-trivial content whose stored checksum bytes are
    /// garbage (they are just part of the pattern).
    fn patterned(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i % 251) as u8).collect()
    }

    #[test]
    fn rejects_too_short() {
        assert_eq!(
            SaveFile::from_bytes(Vec::new()).unwrap_err(),
            LoadError::TooShort { len: 0 }
        );
        assert_eq!(
            SaveFile::from_bytes(vec![0u8; 0x7FFF]).unwrap_err(),
            LoadError::TooShort { len: 0x7FFF }
        );
    }

    #[test]
    fn accepts_exact_padded_and_double_size() {
        for len in [0x8000usize, 0x8009, 0x10000] {
            let save = SaveFile::from_bytes(vec![0u8; len]).expect("length is valid");
            assert_eq!(save.as_bytes().len(), len);
        }
    }

    #[test]
    fn untouched_file_round_trips_byte_identically() {
        for len in [0x8000usize, 0x8009, 0x10000] {
            let input = patterned(len);
            let save = SaveFile::from_bytes(input.clone()).expect("length is valid");
            assert!(!save.is_edited());
            assert_eq!(save.to_bytes(), input, "len 0x{len:X}");
        }
    }

    #[test]
    fn untouched_file_with_wrong_checksums_round_trips() {
        // All-zero: every stored checksum (0x00) disagrees with the
        // computed one (0xFF). Untouched -> still byte-identical.
        let mut input = vec![0u8; 0x8000];
        input[offsets::MAIN_CHECKSUM] = 0x12; // extra-wrong on purpose
        let save = SaveFile::from_bytes(input.clone()).expect("length is valid");
        assert!(!save.diagnostics().is_empty());
        assert_eq!(save.to_bytes(), input);
    }

    #[test]
    fn edit_recomputes_all_checksums() {
        let mut save = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("length is valid");
        save.buf_mut()[offsets::PLAYER_NAME] = 0x80; // "A"
        assert!(save.is_edited());
        let out = save.to_bytes();
        assert_eq!(out[offsets::PLAYER_NAME], 0x80);
        assert!(checksum::verify(&out).is_empty());
    }

    #[test]
    fn tail_past_sram_survives_an_edit() {
        let input = patterned(0x8009);
        let mut save = SaveFile::from_bytes(input.clone()).expect("length is valid");
        save.buf_mut()[offsets::PLAYER_NAME] = 0x91;
        let out = save.to_bytes();
        assert_eq!(out.len(), input.len());
        assert_eq!(&out[0x8000..], &input[0x8000..]);
    }

    #[test]
    fn mark_edited_alone_triggers_checksum_repair() {
        let mut save = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("length is valid");
        save.mark_edited();
        assert!(checksum::verify(&save.to_bytes()).is_empty());
    }

    #[test]
    fn diagnostics_report_checksum_mismatches_with_codes_and_spans() {
        let save = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("length is valid");
        let diags = save.diagnostics();
        assert_eq!(diags.len(), 15);
        assert!(diags.iter().all(|d| d.severity == Severity::Warning));
        let find = |code: &str| {
            diags
                .iter()
                .find(|d| d.code == code)
                .unwrap_or_else(|| panic!("missing diagnostic {code}"))
        };
        assert_eq!(
            find("W-CHECKSUM-MAIN").span,
            Some(offsets::MAIN_CHECKSUM..offsets::MAIN_CHECKSUM + 1)
        );
        assert_eq!(
            find("W-CHECKSUM-BOXBANK2").span,
            Some(offsets::BANK2_ALL_BOXES_CHECKSUM..offsets::BANK2_ALL_BOXES_CHECKSUM + 1)
        );
        assert_eq!(
            find("W-CHECKSUM-BOXBANK3").span,
            Some(offsets::BANK3_ALL_BOXES_CHECKSUM..offsets::BANK3_ALL_BOXES_CHECKSUM + 1)
        );
        assert_eq!(
            find("W-CHECKSUM-BOX1").span,
            Some(offsets::BANK2_ALL_BOXES_CHECKSUM + 1..offsets::BANK2_ALL_BOXES_CHECKSUM + 2)
        );
        assert_eq!(
            find("W-CHECKSUM-BOX12").span,
            Some(offsets::BANK3_ALL_BOXES_CHECKSUM + 6..offsets::BANK3_ALL_BOXES_CHECKSUM + 7)
        );
    }

    #[test]
    fn fix_checksums_clears_diagnostics_and_touches_only_checksum_bytes() {
        let input = vec![0u8; 0x8000];
        let mut save = SaveFile::from_bytes(input.clone()).expect("length is valid");
        assert_eq!(save.diagnostics().len(), 15);
        save.fix_checksums();
        assert!(save.diagnostics().is_empty());
        let out = save.to_bytes();
        // Main checksum byte, then the two contiguous 7-byte checksum
        // blocks (all-boxes + 6 per-box) in banks 2 and 3.
        assert_eq!(
            changed_ranges(&input, &out),
            vec![
                offsets::MAIN_CHECKSUM..offsets::MAIN_CHECKSUM + 1,
                offsets::BANK2_ALL_BOXES_CHECKSUM..offsets::BANK2_ALL_BOXES_CHECKSUM + 7,
                offsets::BANK3_ALL_BOXES_CHECKSUM..offsets::BANK3_ALL_BOXES_CHECKSUM + 7,
            ]
        );
    }

    #[test]
    fn game_label_names_gen1() {
        let save = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("length is valid");
        assert_eq!(save.game_label(), "Pokémon Red/Blue/Yellow (Gen 1)");
    }

    #[test]
    fn changed_ranges_equal_slices() {
        assert_eq!(
            changed_ranges(&[1, 2, 3], &[1, 2, 3]),
            Vec::<Range<usize>>::new()
        );
        assert_eq!(changed_ranges(&[], &[]), Vec::<Range<usize>>::new());
    }

    #[test]
    fn changed_ranges_finds_maximal_runs() {
        assert_eq!(changed_ranges(&[1, 2, 3], &[1, 9, 3]), vec![1..2]);
        assert_eq!(
            changed_ranges(&[1, 2, 3, 4, 5, 6], &[9, 9, 3, 4, 9, 6]),
            vec![0..2, 4..5]
        );
        // Run extending to the end.
        assert_eq!(changed_ranges(&[1, 2, 3], &[1, 9, 9]), vec![1..3]);
    }

    #[test]
    fn changed_ranges_length_difference_counts_as_changed() {
        assert_eq!(changed_ranges(&[1, 2, 3], &[1, 2, 3, 4, 5]), vec![3..5]);
        assert_eq!(changed_ranges(&[1, 2, 3, 4, 5], &[1, 2, 3]), vec![3..5]);
        // Adjacent to a differing run: merged into one range.
        assert_eq!(changed_ranges(&[1, 2, 9], &[1, 2, 3, 4]), vec![2..4]);
    }
}
