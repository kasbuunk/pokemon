//! LSB-first bit slices over byte buffers — the Gen 1 `flag_array`
//! convention.
//!
//! pokered's `FlagAction` (engine/flag_action.asm) addresses flag `i` as
//! byte `i >> 3`, mask `1 << (i & 7)`: bit 0 is the least significant bit
//! of the first byte. Every Gen 1 flag array (Pokédex owned/seen, event
//! flags, hidden items, …) uses this layout, so [`BitSlice`] /
//! [`BitSliceMut`] are the single bit-addressing primitives for all of
//! them.

/// Read-only LSB-first bit view of a byte slice.
#[derive(Debug, Clone, Copy)]
pub struct BitSlice<'a> {
    bytes: &'a [u8],
}

/// Mutable LSB-first bit view of a byte slice.
#[derive(Debug)]
pub struct BitSliceMut<'a> {
    bytes: &'a mut [u8],
}

#[inline]
fn locate(bit: usize, len_bits: usize) -> (usize, u8) {
    assert!(
        bit < len_bits,
        "bit index {bit} out of range ({len_bits} bits)"
    );
    (bit >> 3, 1u8 << (bit & 7))
}

impl<'a> BitSlice<'a> {
    /// Wrap a byte slice for bit-level reads.
    pub fn new(bytes: &'a [u8]) -> Self {
        BitSlice { bytes }
    }

    /// Number of addressable bits (8 per byte).
    pub fn len_bits(&self) -> usize {
        self.bytes.len() * 8
    }

    /// Read bit `bit`. Panics if `bit >= len_bits()`.
    pub fn get(&self, bit: usize) -> bool {
        let (byte, mask) = locate(bit, self.len_bits());
        self.bytes[byte] & mask != 0
    }

    /// Number of set bits in the whole slice.
    pub fn count_ones(&self) -> usize {
        self.bytes.iter().map(|b| b.count_ones() as usize).sum()
    }

    /// Iterator over the indexes of set bits, ascending.
    pub fn iter_ones(&self) -> impl Iterator<Item = usize> + 'a {
        self.bytes.iter().enumerate().flat_map(|(byte_index, &b)| {
            (0..8).filter_map(move |bit_in_byte| {
                (b & (1 << bit_in_byte) != 0).then_some(byte_index * 8 + bit_in_byte)
            })
        })
    }
}

impl<'a> BitSliceMut<'a> {
    /// Wrap a byte slice for bit-level reads and writes.
    pub fn new(bytes: &'a mut [u8]) -> Self {
        BitSliceMut { bytes }
    }

    /// Number of addressable bits (8 per byte).
    pub fn len_bits(&self) -> usize {
        self.bytes.len() * 8
    }

    /// Read bit `bit`. Panics if `bit >= len_bits()`.
    pub fn get(&self, bit: usize) -> bool {
        BitSlice::new(self.bytes).get(bit)
    }

    /// Write bit `bit`. Panics if `bit >= len_bits()`.
    pub fn set(&mut self, bit: usize, value: bool) {
        let (byte, mask) = locate(bit, self.len_bits());
        if value {
            self.bytes[byte] |= mask;
        } else {
            self.bytes[byte] &= !mask;
        }
    }

    /// Number of set bits in the whole slice.
    pub fn count_ones(&self) -> usize {
        BitSlice::new(self.bytes).count_ones()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bit_addressing_is_lsb_first() {
        // 0x01 = bit 0 of byte 0; 0x80 = bit 7; byte 1 starts at bit 8.
        let bytes = [0b0000_0001u8, 0b1000_0000];
        let bits = BitSlice::new(&bytes);
        assert!(bits.get(0));
        assert!(!bits.get(1));
        assert!(!bits.get(7));
        assert!(!bits.get(8));
        assert!(bits.get(15));
    }

    #[test]
    fn len_bits_is_eight_per_byte() {
        assert_eq!(BitSlice::new(&[]).len_bits(), 0);
        assert_eq!(BitSlice::new(&[0; 19]).len_bits(), 152);
        let mut buf = [0u8; 3];
        assert_eq!(BitSliceMut::new(&mut buf).len_bits(), 24);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn get_out_of_range_panics() {
        BitSlice::new(&[0u8; 2]).get(16);
    }

    #[test]
    #[should_panic(expected = "out of range")]
    fn set_out_of_range_panics() {
        let mut buf = [0u8; 2];
        BitSliceMut::new(&mut buf).set(16, true);
    }

    #[test]
    fn set_writes_the_pokered_mask() {
        let mut buf = [0u8; 3];
        let mut bits = BitSliceMut::new(&mut buf);
        // FlagAction: byte = bit >> 3, mask = 1 << (bit & 7).
        bits.set(0, true);
        bits.set(7, true);
        bits.set(10, true);
        assert_eq!(buf, [0b1000_0001, 0b0000_0100, 0]);
    }

    #[test]
    fn set_false_clears_only_its_bit() {
        let mut buf = [0xFFu8; 2];
        let mut bits = BitSliceMut::new(&mut buf);
        bits.set(3, false);
        assert!(!bits.get(3));
        assert_eq!(buf, [0b1111_0111, 0xFF]);
    }

    #[test]
    fn set_is_idempotent() {
        let mut buf = [0u8; 1];
        let mut bits = BitSliceMut::new(&mut buf);
        bits.set(5, true);
        bits.set(5, true);
        assert_eq!(buf, [0b0010_0000]);
        let mut bits = BitSliceMut::new(&mut buf);
        bits.set(5, false);
        bits.set(5, false);
        assert_eq!(buf, [0]);
    }

    #[test]
    fn count_ones_counts_all_bytes() {
        assert_eq!(BitSlice::new(&[]).count_ones(), 0);
        assert_eq!(BitSlice::new(&[0xFF, 0x0F, 0x01]).count_ones(), 13);
        let mut buf = [0b101u8];
        assert_eq!(BitSliceMut::new(&mut buf).count_ones(), 2);
    }

    #[test]
    fn iter_ones_yields_ascending_indexes() {
        let bytes = [0b1000_0001u8, 0b0000_0100];
        let ones: Vec<usize> = BitSlice::new(&bytes).iter_ones().collect();
        assert_eq!(ones, vec![0, 7, 10]);
        assert_eq!(BitSlice::new(&[0u8; 4]).iter_ones().count(), 0);
    }
}
