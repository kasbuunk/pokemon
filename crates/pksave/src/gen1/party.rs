//! The party block: count + species list + three parallel arrays
//! (mon records, OT names, nicknames).
//!
//! Layout per `docs/FORMAT.md` ("Party block", at [`offsets::PARTY`]).
//! Every mutating operation maintains the block invariants:
//!
//! - the count byte equals the number of species-list entries before the
//!   `0xFF` sentinel,
//! - species-list byte `i` equals `mon(i).species()`,
//! - the three parallel arrays stay aligned (a mon keeps its OT name and
//!   nickname through swaps and removals).

use thiserror::Error;

use super::offsets;
use super::pokemon::{PartyMonMut, PartyMonView};
use super::save::SaveFile;
use super::text::{self, TextError};

/// Terminator byte after the last species-list entry.
const SENTINEL: u8 = 0xFF;

// Party-block internal layout, relative to `offsets::PARTY` (FORMAT.md).
const COUNT: usize = 0x000;
const SPECIES_LIST: usize = 0x001;
const SPECIES_LIST_LEN: usize = offsets::PARTY_CAPACITY + 1;
const MONS: usize = SPECIES_LIST + SPECIES_LIST_LEN;
const OT_NAMES: usize = MONS + offsets::PARTY_CAPACITY * offsets::PARTY_MON_SIZE;
const NICKNAMES: usize = OT_NAMES + offsets::PARTY_CAPACITY * offsets::NAME_LEN;

// The layout must tile the whole party block exactly.
const _: () = assert!(MONS == 0x008);
const _: () = assert!(OT_NAMES == 0x110);
const _: () = assert!(NICKNAMES == 0x152);
const _: () =
    assert!(NICKNAMES + offsets::PARTY_CAPACITY * offsets::NAME_LEN == offsets::PARTY_LEN);

const fn mon_at(i: usize) -> usize {
    MONS + i * offsets::PARTY_MON_SIZE
}

const fn ot_name_at(i: usize) -> usize {
    OT_NAMES + i * offsets::NAME_LEN
}

const fn nickname_at(i: usize) -> usize {
    NICKNAMES + i * offsets::NAME_LEN
}

/// A party edit that cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PartyError {
    #[error("party is full ({capacity} mons)", capacity = offsets::PARTY_CAPACITY)]
    Full,
    #[error(transparent)]
    Text(#[from] TextError),
}

/// Read-only view of the party block.
#[derive(Debug, Clone, Copy)]
pub struct PartyView<'a> {
    data: &'a [u8],
}

/// Mutable view of the party block.
#[derive(Debug)]
pub struct PartyMut<'a> {
    data: &'a mut [u8],
}

impl SaveFile {
    /// Read-only access to the party block.
    pub fn party(&self) -> PartyView<'_> {
        PartyView {
            data: &self.buf()[offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN],
        }
    }

    /// Mutable access to the party block. Marks the file edited.
    pub fn party_mut(&mut self) -> PartyMut<'_> {
        PartyMut {
            data: &mut self.buf_mut()[offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN],
        }
    }
}

impl<'a> PartyView<'a> {
    /// Number of mons, clamped to [`offsets::PARTY_CAPACITY`] in case the
    /// stored count byte is corrupt.
    pub fn len(&self) -> usize {
        usize::from(self.data[COUNT]).min(offsets::PARTY_CAPACITY)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The species-list entries (internal indexes) before the sentinel.
    pub fn species_list(&self) -> &[u8] {
        &self.data[SPECIES_LIST..SPECIES_LIST + self.len()]
    }

    /// View of mon slot `i`.
    ///
    /// # Panics
    /// If `i >= PARTY_CAPACITY`.
    pub fn mon(&self, i: usize) -> PartyMonView<'a> {
        assert!(i < offsets::PARTY_CAPACITY, "party slot {i} out of range");
        PartyMonView::new(&self.data[mon_at(i)..mon_at(i) + offsets::PARTY_MON_SIZE])
    }

    /// Decoded OT name of slot `i`.
    pub fn ot_name(&self, i: usize) -> String {
        assert!(i < offsets::PARTY_CAPACITY, "party slot {i} out of range");
        text::decode(&self.data[ot_name_at(i)..ot_name_at(i) + offsets::NAME_LEN])
    }

    /// Decoded nickname of slot `i`.
    pub fn nickname(&self, i: usize) -> String {
        assert!(i < offsets::PARTY_CAPACITY, "party slot {i} out of range");
        text::decode(&self.data[nickname_at(i)..nickname_at(i) + offsets::NAME_LEN])
    }
}

impl<'a> PartyMut<'a> {
    /// Read-only view of the same block.
    pub fn as_view(&self) -> PartyView<'_> {
        PartyView { data: self.data }
    }

    /// Number of mons (see [`PartyView::len`]).
    pub fn len(&self) -> usize {
        self.as_view().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mutable view of mon slot `i`.
    ///
    /// Field edits through the returned view do not touch the species
    /// list — to change a species, use [`PartyMut::set_species`] so the
    /// list stays in sync.
    ///
    /// # Panics
    /// If `i >= PARTY_CAPACITY`.
    pub fn mon_mut(&mut self, i: usize) -> PartyMonMut<'_> {
        assert!(i < offsets::PARTY_CAPACITY, "party slot {i} out of range");
        PartyMonMut::new(&mut self.data[mon_at(i)..mon_at(i) + offsets::PARTY_MON_SIZE])
    }

    /// Set the species of slot `i` in both the mon record and the
    /// species list.
    ///
    /// # Panics
    /// If `i >= len()`.
    pub fn set_species(&mut self, i: usize, species: u8) {
        let len = self.len();
        assert!(i < len, "party slot {i} out of range (len {len})");
        self.data[SPECIES_LIST + i] = species;
        self.mon_mut(i).set_species(species);
    }

    /// Encode and store the OT name of slot `i`.
    ///
    /// # Panics
    /// If `i >= PARTY_CAPACITY`.
    pub fn set_ot_name(&mut self, i: usize, name: &str) -> Result<(), TextError> {
        assert!(i < offsets::PARTY_CAPACITY, "party slot {i} out of range");
        let encoded = text::encode(name, offsets::NAME_LEN)?;
        self.data[ot_name_at(i)..ot_name_at(i) + offsets::NAME_LEN].copy_from_slice(&encoded);
        Ok(())
    }

    /// Encode and store the nickname of slot `i`.
    ///
    /// # Panics
    /// If `i >= PARTY_CAPACITY`.
    pub fn set_nickname(&mut self, i: usize, name: &str) -> Result<(), TextError> {
        assert!(i < offsets::PARTY_CAPACITY, "party slot {i} out of range");
        let encoded = text::encode(name, offsets::NAME_LEN)?;
        self.data[nickname_at(i)..nickname_at(i) + offsets::NAME_LEN].copy_from_slice(&encoded);
        Ok(())
    }

    /// Append a mon. Returns the slot it landed in. Nothing is written
    /// if the party is full or a name fails to encode.
    pub fn add(
        &mut self,
        mon: &[u8; offsets::PARTY_MON_SIZE],
        ot_name: &str,
        nickname: &str,
    ) -> Result<usize, PartyError> {
        let i = self.len();
        if i >= offsets::PARTY_CAPACITY {
            return Err(PartyError::Full);
        }
        let ot_encoded = text::encode(ot_name, offsets::NAME_LEN)?;
        let nick_encoded = text::encode(nickname, offsets::NAME_LEN)?;
        self.data[mon_at(i)..mon_at(i) + offsets::PARTY_MON_SIZE].copy_from_slice(mon);
        self.data[ot_name_at(i)..ot_name_at(i) + offsets::NAME_LEN].copy_from_slice(&ot_encoded);
        self.data[nickname_at(i)..nickname_at(i) + offsets::NAME_LEN]
            .copy_from_slice(&nick_encoded);
        self.data[SPECIES_LIST + i] = PartyMonView::new(mon).species();
        self.write_count(i + 1);
        Ok(i)
    }

    /// Remove the mon in slot `i`, shifting later slots down. The
    /// vacated trailing slot (mon record, OT name, nickname) is
    /// zero-filled.
    ///
    /// # Panics
    /// If `i >= len()`.
    pub fn remove(&mut self, i: usize) {
        let len = self.len();
        assert!(i < len, "party slot {i} out of range (len {len})");
        self.data
            .copy_within(SPECIES_LIST + i + 1..SPECIES_LIST + len, SPECIES_LIST + i);
        self.data.copy_within(mon_at(i + 1)..mon_at(len), mon_at(i));
        self.data
            .copy_within(ot_name_at(i + 1)..ot_name_at(len), ot_name_at(i));
        self.data
            .copy_within(nickname_at(i + 1)..nickname_at(len), nickname_at(i));
        let new_len = len - 1;
        self.data[mon_at(new_len)..mon_at(new_len + 1)].fill(0);
        self.data[ot_name_at(new_len)..ot_name_at(new_len + 1)].fill(0);
        self.data[nickname_at(new_len)..nickname_at(new_len + 1)].fill(0);
        self.write_count(new_len);
    }

    /// Swap slots `i` and `j` (species-list entries, mon records, OT
    /// names and nicknames all move together).
    ///
    /// # Panics
    /// If `i >= len()` or `j >= len()`.
    pub fn swap(&mut self, i: usize, j: usize) {
        let len = self.len();
        assert!(i < len, "party slot {i} out of range (len {len})");
        assert!(j < len, "party slot {j} out of range (len {len})");
        if i == j {
            return;
        }
        self.data.swap(SPECIES_LIST + i, SPECIES_LIST + j);
        self.swap_range(mon_at(i), mon_at(j), offsets::PARTY_MON_SIZE);
        self.swap_range(ot_name_at(i), ot_name_at(j), offsets::NAME_LEN);
        self.swap_range(nickname_at(i), nickname_at(j), offsets::NAME_LEN);
    }

    /// Empty the party: count 0, sentinel in the first species-list
    /// byte, everything else in the block zeroed. Also suitable for
    /// initializing the party region of a blank buffer.
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.data[SPECIES_LIST] = SENTINEL;
    }

    /// Write the count byte and re-terminate the species list: sentinel
    /// at index `count`, zeros after it (deterministic content instead
    /// of stale bytes).
    fn write_count(&mut self, count: usize) {
        self.data[COUNT] = count as u8;
        self.data[SPECIES_LIST + count] = SENTINEL;
        self.data[SPECIES_LIST + count + 1..SPECIES_LIST + SPECIES_LIST_LEN].fill(0);
    }

    fn swap_range(&mut self, a: usize, b: usize, len: usize) {
        for k in 0..len {
            self.data.swap(a + k, b + k);
        }
    }
}
