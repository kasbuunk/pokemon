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

use super::{checksum, offsets, text, validate};
use crate::{Diagnostic, SaveGame};

/// Which Gen 1 cartridge a save targets. The layout is identical
/// (see `docs/FORMAT.md` § Red/Blue vs Yellow); the variant only changes
/// which defaults make sense (Yellow gives meaning to the Pikachu
/// friendship byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameVariant {
    RedBlue,
    Yellow,
}

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
    /// Build a minimal, bootable blank save (exactly 32 KiB):
    ///
    /// - player name `"RED"`, rival name `"BLUE"`,
    /// - options: medium text speed (3), animations on, battle style
    ///   Shift; letter-delay flags = 1 as `InitOptions` sets them,
    /// - boxes-initialized flag set (bit 7 of [`offsets::CURRENT_BOX_NUM`]),
    ///   current box 0,
    /// - bag/PC item lists empty (count 0, `0xFF` terminator),
    /// - party count 0 with `0xFF` species sentinel; likewise all 12 bank
    ///   boxes and the current-box working copy,
    /// - Pokédex clear, money/coins zero (valid BCD), daycare empty,
    /// - all 15 checksums valid.
    ///
    /// For [`GameVariant::Yellow`] the Pikachu friendship byte is set to
    /// 90, the game's starting friendship; for `RedBlue` that byte is
    /// unused and left 0. The result is *not* marked edited, so it
    /// round-trips byte-identically.
    pub fn new_empty(variant: GameVariant) -> SaveFile {
        let mut raw = vec![0u8; offsets::SRAM_SIZE];

        let name = |s: &str| text::encode(s, offsets::NAME_LEN).expect("default name fits");
        raw[offsets::PLAYER_NAME..offsets::PLAYER_NAME + offsets::NAME_LEN]
            .copy_from_slice(&name("RED"));
        raw[offsets::RIVAL_NAME..offsets::RIVAL_NAME + offsets::NAME_LEN]
            .copy_from_slice(&name("BLUE"));

        // Medium text speed; InitOptions also sets the letter-delay flags
        // to 1 (BIT_FAST_TEXT_DELAY).
        raw[offsets::OPTIONS] = 3;
        raw[offsets::LETTER_DELAY] = 1;

        // Bit 7 = boxes initialized (must stay set), bits 0-6 = box 0.
        raw[offsets::CURRENT_BOX_NUM] = 0x80;

        // Empty terminated lists: count 0, 0xFF right after.
        raw[offsets::BAG_ITEM_COUNT] = 0;
        raw[offsets::BAG_ITEMS] = 0xFF;
        raw[offsets::PC_ITEM_COUNT] = 0;
        raw[offsets::PC_ITEMS] = 0xFF;

        // Party: count 0, species-list sentinel.
        raw[offsets::PARTY] = 0;
        raw[offsets::PARTY + 1] = 0xFF;

        // All 12 bank boxes plus the current-box working copy.
        for n in 0..offsets::NUM_BOXES {
            raw[offsets::box_offset(n)] = 0;
            raw[offsets::box_offset(n) + 1] = 0xFF;
        }
        raw[offsets::CURRENT_BOX] = 0;
        raw[offsets::CURRENT_BOX + 1] = 0xFF;

        if variant == GameVariant::Yellow {
            raw[offsets::PIKACHU_FRIENDSHIP] = 90;
        }

        // Pokédex, money, coins, play time, daycare-in-use are all-zero
        // already, which is exactly their empty encoding.
        checksum::fix_all(&mut raw);
        SaveFile { raw, edited: false }
    }

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

    /// Non-fatal findings: the full catalogue of [`validate::diagnose`]
    /// (checksums, counts/sentinels, species/level ranges, item lists,
    /// BCD, text terminators, box-initialization hazards, …).
    pub fn diagnostics(&self) -> Vec<Diagnostic> {
        validate::diagnose(self)
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
        let diags: Vec<Diagnostic> = save
            .diagnostics()
            .into_iter()
            .filter(|d| d.code.starts_with("W-CHECKSUM-"))
            .collect();
        assert_eq!(diags.len(), 15);
        assert!(diags.iter().all(|d| d.severity == crate::Severity::Warning));
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
        let checksum_diags = |save: &SaveFile| {
            save.diagnostics()
                .into_iter()
                .filter(|d| d.code.starts_with("W-CHECKSUM-"))
                .count()
        };
        let input = vec![0u8; 0x8000];
        let mut save = SaveFile::from_bytes(input.clone()).expect("length is valid");
        assert_eq!(checksum_diags(&save), 15);
        save.fix_checksums();
        assert_eq!(checksum_diags(&save), 0);
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
    fn new_empty_has_no_diagnostics() {
        // Against the *full* validate::diagnose catalogue, not just
        // checksums: counts/sentinels, BCD, terminators, box-init flag,
        // box staleness, map id — a blank save must be clean everywhere.
        for variant in [GameVariant::RedBlue, GameVariant::Yellow] {
            let save = SaveFile::new_empty(variant);
            assert_eq!(save.diagnostics(), Vec::new(), "{variant:?}");
            // Same through the game-agnostic trait object boundary.
            let as_trait: &dyn SaveGame = &save;
            assert_eq!(as_trait.diagnostics(), Vec::new(), "{variant:?} via trait");
        }
    }

    #[test]
    fn new_empty_round_trips_and_reloads() {
        let save = SaveFile::new_empty(GameVariant::RedBlue);
        assert!(!save.is_edited());
        let bytes = save.to_bytes();
        assert_eq!(bytes.len(), offsets::SRAM_SIZE);
        assert_eq!(bytes, save.as_bytes());
        let reloaded = SaveFile::from_bytes(bytes.clone()).expect("length is valid");
        assert_eq!(reloaded.to_bytes(), bytes);
        assert!(reloaded.diagnostics().is_empty());
    }

    #[test]
    fn new_empty_sets_the_documented_fields() {
        let save = SaveFile::new_empty(GameVariant::RedBlue);
        let b = save.as_bytes();
        assert_eq!(
            &b[offsets::PLAYER_NAME..offsets::PLAYER_NAME + 4],
            &[0x91, 0x84, 0x83, 0x50] // "RED" + terminator
        );
        assert_eq!(
            &b[offsets::RIVAL_NAME..offsets::RIVAL_NAME + 5],
            &[0x81, 0x8B, 0x94, 0x84, 0x50] // "BLUE" + terminator
        );
        assert_eq!(b[offsets::OPTIONS], 3);
        assert_eq!(b[offsets::LETTER_DELAY], 1);
        assert_eq!(b[offsets::CURRENT_BOX_NUM], 0x80);
        assert_eq!(b[offsets::BAG_ITEM_COUNT], 0);
        assert_eq!(b[offsets::BAG_ITEMS], 0xFF);
        assert_eq!(b[offsets::PC_ITEM_COUNT], 0);
        assert_eq!(b[offsets::PC_ITEMS], 0xFF);
        assert_eq!(b[offsets::PARTY], 0);
        assert_eq!(b[offsets::PARTY + 1], 0xFF);
        for n in 0..offsets::NUM_BOXES {
            assert_eq!(b[offsets::box_offset(n)], 0, "box {n} count");
            assert_eq!(b[offsets::box_offset(n) + 1], 0xFF, "box {n} sentinel");
        }
        assert_eq!(b[offsets::CURRENT_BOX], 0);
        assert_eq!(b[offsets::CURRENT_BOX + 1], 0xFF);
        assert_eq!(
            &b[offsets::POKEDEX_OWNED..offsets::POKEDEX_SEEN + offsets::POKEDEX_LEN],
            &[0u8; 2 * offsets::POKEDEX_LEN][..]
        );
        assert_eq!(&b[offsets::MONEY..offsets::MONEY + 3], &[0, 0, 0]);
        assert_eq!(&b[offsets::COINS..offsets::COINS + 2], &[0, 0]);
        assert_eq!(b[offsets::DAYCARE_IN_USE], 0);
        assert_eq!(b[offsets::PIKACHU_FRIENDSHIP], 0);
    }

    #[test]
    fn new_empty_yellow_sets_starting_friendship() {
        let save = SaveFile::new_empty(GameVariant::Yellow);
        assert_eq!(save.as_bytes()[offsets::PIKACHU_FRIENDSHIP], 90);
        assert!(save.diagnostics().is_empty());
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
