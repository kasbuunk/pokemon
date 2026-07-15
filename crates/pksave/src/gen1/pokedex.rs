//! Pokédex owned/seen bitfields.
//!
//! Two 19-byte LSB-first flag arrays (`wPokedexOwned` / `wPokedexSeen`),
//! indexed by **National Dex number − 1**: bit 0 = #001 Bulbasaur,
//! bit 150 = #151 Mew. Bit 151 (the last bit of byte 18) is unused and
//! never written.
//!
//! Dex number 0 or anything above 151 is out of range: getters return
//! `false` and setters are no-ops, so callers can pass untrusted values
//! without panicking.

use super::flags::{BitSlice, BitSliceMut};
use super::offsets;
use super::save::SaveFile;

/// Number of Pokédex entries (#001..=#151).
pub const DEX_COUNT: u8 = 151;

/// `dex` as a flag-array bit index, or `None` when out of range.
fn dex_bit(dex: u8) -> Option<usize> {
    (1..=DEX_COUNT).contains(&dex).then(|| usize::from(dex) - 1)
}

fn field(dex_offset: usize) -> core::ops::Range<usize> {
    dex_offset..dex_offset + offsets::POKEDEX_LEN
}

impl SaveFile {
    fn dex_get(&self, dex_offset: usize, dex: u8) -> bool {
        dex_bit(dex).is_some_and(|bit| BitSlice::new(&self.buf()[field(dex_offset)]).get(bit))
    }

    fn dex_set(&mut self, dex_offset: usize, dex: u8, value: bool) {
        if let Some(bit) = dex_bit(dex) {
            BitSliceMut::new(&mut self.buf_mut()[field(dex_offset)]).set(bit, value);
        }
    }

    fn dex_count(&self, dex_offset: usize) -> usize {
        // count_ones over all 152 bits is safe because bit 151 is never
        // set by this module, but filter anyway so a corrupt stray bit
        // can't produce an impossible 152.
        BitSlice::new(&self.buf()[field(dex_offset)])
            .iter_ones()
            .filter(|&bit| bit < usize::from(DEX_COUNT))
            .count()
    }

    /// Whether dex entry `dex` (1-151) is owned. Out-of-range reads
    /// `false`.
    pub fn dex_owned(&self, dex: u8) -> bool {
        self.dex_get(offsets::POKEDEX_OWNED, dex)
    }

    /// Mark dex entry `dex` (1-151) owned/unowned. Out-of-range is a
    /// no-op.
    pub fn set_dex_owned(&mut self, dex: u8, owned: bool) {
        self.dex_set(offsets::POKEDEX_OWNED, dex, owned);
    }

    /// Whether dex entry `dex` (1-151) is seen. Out-of-range reads
    /// `false`.
    pub fn dex_seen(&self, dex: u8) -> bool {
        self.dex_get(offsets::POKEDEX_SEEN, dex)
    }

    /// Mark dex entry `dex` (1-151) seen/unseen. Out-of-range is a
    /// no-op.
    pub fn set_dex_seen(&mut self, dex: u8, seen: bool) {
        self.dex_set(offsets::POKEDEX_SEEN, dex, seen);
    }

    /// Number of owned dex entries (0-151).
    pub fn owned_count(&self) -> usize {
        self.dex_count(offsets::POKEDEX_OWNED)
    }

    /// Number of seen dex entries (0-151).
    pub fn seen_count(&self) -> usize {
        self.dex_count(offsets::POKEDEX_SEEN)
    }

    /// Mark all 151 entries owned *and* seen (bit 151 stays clear).
    pub fn complete_dex(&mut self) {
        for dex_offset in [offsets::POKEDEX_OWNED, offsets::POKEDEX_SEEN] {
            let mut bits = BitSliceMut::new(&mut self.buf_mut()[field(dex_offset)]);
            for bit in 0..usize::from(DEX_COUNT) {
                bits.set(bit, true);
            }
        }
    }
}
