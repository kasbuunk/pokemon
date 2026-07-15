//! Gen 1 save checksums.
//!
//! Every checksum in a Gen 1 save uses the same algorithm: the 8-bit
//! wrapping sum of the covered bytes, bitwise inverted (`chk = !sum`).
//! There are 15 checksummed regions (see `docs/FORMAT.md` § Checksums):
//! the main data block, one "all boxes" region per box bank, and one
//! region per individual box.

use core::ops::Range;

use super::offsets;

/// The Gen 1 checksum: 8-bit wrapping sum of `bytes`, bitwise NOT.
pub fn gen1_checksum(bytes: &[u8]) -> u8 {
    !bytes.iter().fold(0u8, |sum, &b| sum.wrapping_add(b))
}

/// One of the 15 checksummed regions of a Gen 1 save.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region {
    /// Main save data `0x2598..=0x3522`, checksum at `0x3523`.
    Main,
    /// All six box blocks of bank 2 (`0x4000..=0x5A4B`), checksum at `0x5A4C`.
    Bank2AllBoxes,
    /// All six box blocks of bank 3 (`0x6000..=0x7A4B`), checksum at `0x7A4C`.
    Bank3AllBoxes,
    /// Individual box `n` (0-based, `0..12`). Boxes 0-5 live in bank 2 with
    /// checksums at `0x5A4D + n`; boxes 6-11 live in bank 3 with checksums
    /// at `0x7A4D + (n - 6)`.
    Box(usize),
}

impl Region {
    /// Number of checksummed regions.
    pub const COUNT: usize = 15;

    /// All 15 regions.
    pub const ALL: [Region; Region::COUNT] = [
        Region::Main,
        Region::Bank2AllBoxes,
        Region::Bank3AllBoxes,
        Region::Box(0),
        Region::Box(1),
        Region::Box(2),
        Region::Box(3),
        Region::Box(4),
        Region::Box(5),
        Region::Box(6),
        Region::Box(7),
        Region::Box(8),
        Region::Box(9),
        Region::Box(10),
        Region::Box(11),
    ];

    /// Byte range covered by this region's checksum.
    pub fn data_range(self) -> Range<usize> {
        match self {
            Region::Main => offsets::CHECKSUM_REGION_START..offsets::MAIN_CHECKSUM,
            Region::Bank2AllBoxes => offsets::BANK2_BOXES..offsets::BANK2_ALL_BOXES_CHECKSUM,
            Region::Bank3AllBoxes => offsets::BANK3_BOXES..offsets::BANK3_ALL_BOXES_CHECKSUM,
            Region::Box(n) => {
                assert!(n < offsets::NUM_BOXES, "box index {n} out of range");
                let start = offsets::box_offset(n);
                start..start + offsets::BOX_LEN
            }
        }
    }

    /// Inverse of [`Region::checksum_offset`]: the region whose *stored
    /// checksum byte* lives at `offset`, or `None` if `offset` is not one
    /// of the 15 checksum bytes. Used by the raw-write path
    /// (`SaveFile::set_bytes`) to detect writes that pin a checksum.
    pub fn at_checksum_offset(offset: usize) -> Option<Region> {
        Region::ALL
            .into_iter()
            .find(|region| region.checksum_offset() == offset)
    }

    /// File offset of the checksum byte itself.
    pub fn checksum_offset(self) -> usize {
        match self {
            Region::Main => offsets::MAIN_CHECKSUM,
            Region::Bank2AllBoxes => offsets::BANK2_ALL_BOXES_CHECKSUM,
            Region::Bank3AllBoxes => offsets::BANK3_ALL_BOXES_CHECKSUM,
            Region::Box(n) => {
                assert!(n < offsets::NUM_BOXES, "box index {n} out of range");
                if n < offsets::NUM_BOXES / 2 {
                    offsets::BANK2_ALL_BOXES_CHECKSUM + 1 + n
                } else {
                    offsets::BANK3_ALL_BOXES_CHECKSUM + 1 + (n - offsets::NUM_BOXES / 2)
                }
            }
        }
    }
}

/// A stored checksum that does not match the bytes it covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecksumMismatch {
    pub region: Region,
    /// Checksum byte currently stored in the file.
    pub stored: u8,
    /// Checksum recomputed from the covered bytes.
    pub computed: u8,
}

/// Verify all 15 checksums. `buf` must be at least [`offsets::SRAM_SIZE`]
/// bytes long (callers hold buffers already validated by `SaveFile`);
/// panics otherwise.
pub fn verify(buf: &[u8]) -> Vec<ChecksumMismatch> {
    Region::ALL
        .iter()
        .filter_map(|&region| {
            let stored = buf[region.checksum_offset()];
            let computed = gen1_checksum(&buf[region.data_range()]);
            (stored != computed).then_some(ChecksumMismatch {
                region,
                stored,
                computed,
            })
        })
        .collect()
}

/// Recompute and store all 15 checksums. Same length requirement as
/// [`verify`].
pub fn fix_all(buf: &mut [u8]) {
    for region in Region::ALL {
        buf[region.checksum_offset()] = gen1_checksum(&buf[region.data_range()]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_of_empty_and_all_zero_is_ff() {
        // sum 0 -> !0 == 0xFF
        assert_eq!(gen1_checksum(&[]), 0xFF);
        assert_eq!(gen1_checksum(&[0u8; 0xF8B]), 0xFF);
    }

    #[test]
    fn checksum_wraps_at_8_bits() {
        // 0xFF + 0xFF = 0x1FE wraps to 0xFE -> !0xFE == 0x01
        assert_eq!(gen1_checksum(&[0xFF, 0xFF]), 0x01);
        // 0x01 -> !0x01 == 0xFE
        assert_eq!(gen1_checksum(&[0x01]), 0xFE);
    }

    #[test]
    fn region_metadata_matches_format_doc() {
        assert_eq!(Region::ALL.len(), 15);
        assert_eq!(Region::Main.data_range(), 0x2598..0x3523);
        assert_eq!(Region::Main.checksum_offset(), 0x3523);
        assert_eq!(Region::Bank2AllBoxes.data_range(), 0x4000..0x5A4C);
        assert_eq!(Region::Bank2AllBoxes.checksum_offset(), 0x5A4C);
        assert_eq!(Region::Bank3AllBoxes.data_range(), 0x6000..0x7A4C);
        assert_eq!(Region::Bank3AllBoxes.checksum_offset(), 0x7A4C);
        assert_eq!(Region::Box(0).data_range(), 0x4000..0x4462);
        assert_eq!(Region::Box(0).checksum_offset(), 0x5A4D);
        assert_eq!(Region::Box(5).checksum_offset(), 0x5A52);
        assert_eq!(Region::Box(6).data_range(), 0x6000..0x6462);
        assert_eq!(Region::Box(6).checksum_offset(), 0x7A4D);
        assert_eq!(Region::Box(11).checksum_offset(), 0x7A52);
        // Per-box regions tile their bank's all-boxes region exactly.
        assert_eq!(Region::Box(5).data_range().end, 0x5A4C);
        assert_eq!(Region::Box(11).data_range().end, 0x7A4C);
    }

    #[test]
    fn at_checksum_offset_inverts_checksum_offset() {
        for region in Region::ALL {
            assert_eq!(
                Region::at_checksum_offset(region.checksum_offset()),
                Some(region)
            );
        }
        // Immediate neighbors that are not themselves checksum bytes
        // (the per-box blocks are contiguous with their bank checksum,
        // so skip neighbors that land on another region's byte).
        let checksum_offsets: Vec<usize> =
            Region::ALL.iter().map(|r| r.checksum_offset()).collect();
        for region in Region::ALL {
            let at = region.checksum_offset();
            for neighbor in [at - 1, at + 1] {
                if !checksum_offsets.contains(&neighbor) {
                    assert_eq!(
                        Region::at_checksum_offset(neighbor),
                        None,
                        "0x{neighbor:04X}"
                    );
                }
            }
        }
        assert_eq!(Region::at_checksum_offset(0), None);
        assert_eq!(Region::at_checksum_offset(offsets::SRAM_SIZE), None);
    }

    #[test]
    fn all_zero_buffer_fails_all_15_then_fix_all_repairs() {
        let mut buf = vec![0u8; offsets::SRAM_SIZE];
        let mismatches = verify(&buf);
        assert_eq!(mismatches.len(), 15);
        for m in &mismatches {
            assert_eq!(m.stored, 0x00);
            assert_eq!(m.computed, 0xFF);
        }
        fix_all(&mut buf);
        assert!(verify(&buf).is_empty());
    }

    #[test]
    fn corrupting_main_region_is_detected() {
        let mut buf = vec![0u8; offsets::SRAM_SIZE];
        fix_all(&mut buf);
        buf[offsets::PLAYER_NAME] ^= 0x5A;
        let mismatches = verify(&buf);
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].region, Region::Main);
    }

    #[test]
    fn corrupting_a_box_flags_both_its_bank_and_the_box() {
        let mut buf = vec![0u8; offsets::SRAM_SIZE];
        fix_all(&mut buf);
        // A byte inside box 3 (0-based), which lives in bank 2.
        buf[offsets::box_offset(3) + 7] ^= 0x01;
        let regions: Vec<Region> = verify(&buf).iter().map(|m| m.region).collect();
        assert_eq!(regions, vec![Region::Bank2AllBoxes, Region::Box(3)]);
    }

    #[test]
    fn fix_all_is_idempotent() {
        let mut buf: Vec<u8> = (0..offsets::SRAM_SIZE).map(|i| (i % 253) as u8).collect();
        fix_all(&mut buf);
        let snapshot = buf.clone();
        fix_all(&mut buf);
        assert_eq!(buf, snapshot);
    }

    #[test]
    fn fix_all_touches_only_checksum_bytes() {
        let original: Vec<u8> = (0..offsets::SRAM_SIZE).map(|i| (i % 251) as u8).collect();
        let mut buf = original.clone();
        fix_all(&mut buf);
        let checksum_offsets: Vec<usize> =
            Region::ALL.iter().map(|r| r.checksum_offset()).collect();
        for (i, (&a, &b)) in original.iter().zip(&buf).enumerate() {
            if checksum_offsets.contains(&i) {
                continue;
            }
            assert_eq!(a, b, "non-checksum byte 0x{i:04X} changed");
        }
    }
}
