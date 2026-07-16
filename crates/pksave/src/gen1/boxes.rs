//! PC box editing: the 12 bank boxes plus the current-box working copy.
//!
//! Each box block is 0x462 bytes (`docs/FORMAT.md` "Box block"):
//! count(1) + species list(21) + 20 × 33-byte box mons + 20 × 11-byte OT
//! names + 20 × 11-byte nicknames. The parallel-array offsets derive
//! from `pokered.sym` (`wBoxMonOT` − `wNumInBox` = 0x2AA,
//! `wBoxMonNicks` − `wNumInBox` = 0x386), which is what the field sizes
//! in the FORMAT.md table add up to.
//!
//! Boxes 1-6 are stored in bank 2 and 7-12 in bank 3
//! ([`offsets::box_offset`]), but the *current* box has a live working
//! copy at [`offsets::CURRENT_BOX`] inside the main checksummed region —
//! that copy is what the game reads and writes until the player switches
//! boxes, at which point it is flushed back to its bank slot. Accessors
//! here route the same way (see [`SaveFile::box_is_live`]); nothing is
//! flushed implicitly — call [`SaveFile::sync_current_box_to_bank`]
//! explicitly if you want the bank slot to match.
//!
//! Every mutating operation maintains the block invariants of `party.rs`:
//! the count byte matches the species list, the species list terminates
//! with `0xFF`, and the three parallel arrays stay aligned.

use thiserror::Error;

use super::offsets;
use super::pokemon::{box_to_party, party_to_box, BoxMonMut, BoxMonView, MonMut, MonView};
use super::save::SaveFile;
use super::text::{self, TextError};

/// Terminator byte after the last species-list entry.
const SENTINEL: u8 = 0xFF;

/// Box-block internal layout, relative to the block start.
pub(crate) mod layout {
    use super::offsets;

    pub const COUNT: usize = 0x000;
    pub const SPECIES_LIST: usize = 0x001;
    pub const SPECIES_LIST_LEN: usize = offsets::MONS_PER_BOX + 1;
    pub const MONS: usize = SPECIES_LIST + SPECIES_LIST_LEN;
    pub const OT_NAMES: usize = MONS + offsets::MONS_PER_BOX * offsets::BOX_MON_SIZE;
    pub const NICKNAMES: usize = OT_NAMES + offsets::MONS_PER_BOX * offsets::NAME_LEN;

    // The layout must tile the whole 0x462-byte block exactly
    // (offsets per pokered.sym; see the module docs).
    const _: () = assert!(MONS == 0x016);
    const _: () = assert!(OT_NAMES == 0x2AA);
    const _: () = assert!(NICKNAMES == 0x386);
    const _: () =
        assert!(NICKNAMES + offsets::MONS_PER_BOX * offsets::NAME_LEN == offsets::BOX_LEN);

    pub const fn mon_at(i: usize) -> usize {
        MONS + i * offsets::BOX_MON_SIZE
    }

    pub const fn ot_name_at(i: usize) -> usize {
        OT_NAMES + i * offsets::NAME_LEN
    }

    pub const fn nickname_at(i: usize) -> usize {
        NICKNAMES + i * offsets::NAME_LEN
    }
}

/// Party-block internal layout as *file* offsets. Mirrors the constants
/// in `party.rs` (FORMAT.md "Party block"); duplicated here for the raw
/// byte moves of [`SaveFile::deposit`] / [`SaveFile::withdraw`] and the
/// diagnostic spans of `validate.rs`.
pub(crate) mod party_layout {
    use super::offsets;

    pub const COUNT: usize = 0x000;
    pub const SPECIES_LIST: usize = 0x001;
    pub const SPECIES_LIST_LEN: usize = offsets::PARTY_CAPACITY + 1;
    pub const MONS: usize = SPECIES_LIST + SPECIES_LIST_LEN;
    pub const OT_NAMES: usize = MONS + offsets::PARTY_CAPACITY * offsets::PARTY_MON_SIZE;
    pub const NICKNAMES: usize = OT_NAMES + offsets::PARTY_CAPACITY * offsets::NAME_LEN;

    const _: () = assert!(MONS == 0x008);
    const _: () = assert!(OT_NAMES == 0x110);
    const _: () = assert!(NICKNAMES == 0x152);
    const _: () =
        assert!(NICKNAMES + offsets::PARTY_CAPACITY * offsets::NAME_LEN == offsets::PARTY_LEN);

    pub const fn mon_at(i: usize) -> usize {
        offsets::PARTY + MONS + i * offsets::PARTY_MON_SIZE
    }

    pub const fn ot_name_at(i: usize) -> usize {
        offsets::PARTY + OT_NAMES + i * offsets::NAME_LEN
    }

    pub const fn nickname_at(i: usize) -> usize {
        offsets::PARTY + NICKNAMES + i * offsets::NAME_LEN
    }
}

/// A box edit that cannot be applied.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum BoxError {
    /// The box already holds [`offsets::MONS_PER_BOX`] (20) mons.
    #[error("box is full ({capacity} mons)", capacity = offsets::MONS_PER_BOX)]
    Full,
    /// A nickname or OT name failed to encode.
    #[error(transparent)]
    Text(#[from] TextError),
}

/// A party ⇄ box transfer that cannot be applied. Nothing is written
/// when one of these is returned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum TransferError {
    /// A source index (party slot, box slot or box number) is out of
    /// range.
    #[error("index out of range")]
    BadIndex,
    /// The destination container (box on deposit, party on withdraw) is
    /// already full.
    #[error("target container is full")]
    TargetFull,
}

/// Read-only view of one box block.
#[derive(Debug, Clone, Copy)]
pub struct BoxView<'a> {
    data: &'a [u8],
}

/// Mutable view of one box block.
#[derive(Debug)]
pub struct BoxMut<'a> {
    data: &'a mut [u8],
}

impl SaveFile {
    /// The current box number (0-11), bits 0-6 of `wCurrentBoxNum`. A
    /// corrupt save may store a value ≥ 12.
    pub fn current_box_number(&self) -> u8 {
        self.buf()[offsets::CURRENT_BOX_NUM] & 0x7F
    }

    /// Bit 7 of `wCurrentBoxNum`: whether the box banks have been
    /// initialized. If clear, the game wipes all boxes on load.
    pub fn boxes_initialized(&self) -> bool {
        self.buf()[offsets::CURRENT_BOX_NUM] & 0x80 != 0
    }

    /// Set the current box number, preserving the boxes-initialized bit.
    ///
    /// This only rewrites the number: unlike an in-game box switch it
    /// neither flushes the working copy to the old box's bank slot nor
    /// loads the new box into the working copy. Call
    /// [`SaveFile::sync_current_box_to_bank`] *before* switching if the
    /// working copy holds edits you want to keep.
    ///
    /// # Panics
    /// If `n >= NUM_BOXES` (12).
    pub fn set_current_box_number(&mut self, n: u8) {
        assert!(
            usize::from(n) < offsets::NUM_BOXES,
            "box number {n} out of range"
        );
        let byte = &mut self.buf_mut()[offsets::CURRENT_BOX_NUM];
        *byte = (*byte & 0x80) | n;
    }

    /// Whether box `n` routes to the live working copy at
    /// [`offsets::CURRENT_BOX`] (i.e. `n` is the current box) rather
    /// than to its bank slot.
    ///
    /// # Panics
    /// If `n >= NUM_BOXES`.
    pub fn box_is_live(&self, n: usize) -> bool {
        assert!(n < offsets::NUM_BOXES, "box index {n} out of range");
        usize::from(self.current_box_number()) == n
    }

    fn box_base(&self, n: usize) -> usize {
        if self.box_is_live(n) {
            offsets::CURRENT_BOX
        } else {
            offsets::box_offset(n)
        }
    }

    /// Read-only access to box `n` (0-based), routed per
    /// [`SaveFile::box_is_live`].
    ///
    /// # Panics
    /// If `n >= NUM_BOXES`.
    pub fn box_(&self, n: usize) -> BoxView<'_> {
        let base = self.box_base(n);
        BoxView {
            data: &self.buf()[base..base + offsets::BOX_LEN],
        }
    }

    /// Mutable access to box `n`, routed per [`SaveFile::box_is_live`].
    /// Marks the file edited.
    ///
    /// # Panics
    /// If `n >= NUM_BOXES`.
    pub fn box_mut(&mut self, n: usize) -> BoxMut<'_> {
        let base = self.box_base(n);
        BoxMut {
            data: &mut self.buf_mut()[base..base + offsets::BOX_LEN],
        }
    }

    /// Copy the working copy at [`offsets::CURRENT_BOX`] into the
    /// current box's bank slot, as the game does when switching boxes.
    /// Never called implicitly. A no-op if the stored current box
    /// number is out of range (corrupt save).
    pub fn sync_current_box_to_bank(&mut self) {
        let n = usize::from(self.current_box_number());
        if n >= offsets::NUM_BOXES {
            return;
        }
        let bank = offsets::box_offset(n);
        self.buf_mut().copy_within(
            offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN,
            bank,
        );
    }

    /// Move party slot `party_index` into box `box_n`: the game's
    /// deposit — the 44-byte party record is truncated to 33 bytes with
    /// the authoritative party level copied over the box level byte
    /// ([`party_to_box`]), then removed from the party. OT name and
    /// nickname bytes move verbatim. Nothing is written on error.
    pub fn deposit(&mut self, party_index: usize, box_n: usize) -> Result<(), TransferError> {
        if box_n >= offsets::NUM_BOXES || party_index >= self.party().len() {
            return Err(TransferError::BadIndex);
        }
        if self.box_(box_n).len() >= offsets::MONS_PER_BOX {
            return Err(TransferError::TargetFull);
        }

        let buf = self.buf();
        let mut mon = [0u8; offsets::PARTY_MON_SIZE];
        mon.copy_from_slice(
            &buf[party_layout::mon_at(party_index)
                ..party_layout::mon_at(party_index) + offsets::PARTY_MON_SIZE],
        );
        let mut ot = [0u8; offsets::NAME_LEN];
        ot.copy_from_slice(
            &buf[party_layout::ot_name_at(party_index)
                ..party_layout::ot_name_at(party_index) + offsets::NAME_LEN],
        );
        let mut nick = [0u8; offsets::NAME_LEN];
        nick.copy_from_slice(
            &buf[party_layout::nickname_at(party_index)
                ..party_layout::nickname_at(party_index) + offsets::NAME_LEN],
        );

        let box_mon = party_to_box(&mon);
        self.party_mut().remove(party_index);
        self.box_mut(box_n)
            .add_raw(&box_mon, &ot, &nick)
            .expect("box capacity was checked");
        Ok(())
    }

    /// Move box slot `box_index` of box `box_n` into the party: the
    /// game's withdrawal — the party level is computed from experience
    /// (`CalcLevelFromExperience`; the box level byte is cosmetic and
    /// not consulted) and the five calculated stats are recomputed from
    /// base stats + DVs + stat exp ([`box_to_party`]). OT name and
    /// nickname bytes move verbatim. Nothing is written on error.
    pub fn withdraw(&mut self, box_n: usize, box_index: usize) -> Result<(), TransferError> {
        if box_n >= offsets::NUM_BOXES || box_index >= self.box_(box_n).len() {
            return Err(TransferError::BadIndex);
        }
        let party_len = self.party().len();
        if party_len >= offsets::PARTY_CAPACITY {
            return Err(TransferError::TargetFull);
        }

        let src = self.box_(box_n);
        let mut mon = [0u8; offsets::BOX_MON_SIZE];
        mon.copy_from_slice(src.mon(box_index).as_bytes());
        let mut ot = [0u8; offsets::NAME_LEN];
        ot.copy_from_slice(src.raw_ot_name(box_index));
        let mut nick = [0u8; offsets::NAME_LEN];
        nick.copy_from_slice(src.raw_nickname(box_index));

        let party_mon = box_to_party(&mon);
        self.box_mut(box_n).remove(box_index);
        self.party_append_raw(&party_mon, &ot, &nick);
        Ok(())
    }

    /// Raw party append, mirroring `PartyMut::add` (which takes decoded
    /// names; the raw bytes here must round-trip verbatim). The caller
    /// has checked there is room.
    fn party_append_raw(
        &mut self,
        mon: &[u8; offsets::PARTY_MON_SIZE],
        ot: &[u8; offsets::NAME_LEN],
        nick: &[u8; offsets::NAME_LEN],
    ) {
        let i = self.party().len();
        debug_assert!(i < offsets::PARTY_CAPACITY, "party capacity was checked");
        let buf = self.buf_mut();
        buf[party_layout::mon_at(i)..party_layout::mon_at(i) + offsets::PARTY_MON_SIZE]
            .copy_from_slice(mon);
        buf[party_layout::ot_name_at(i)..party_layout::ot_name_at(i) + offsets::NAME_LEN]
            .copy_from_slice(ot);
        buf[party_layout::nickname_at(i)..party_layout::nickname_at(i) + offsets::NAME_LEN]
            .copy_from_slice(nick);
        let list = offsets::PARTY + party_layout::SPECIES_LIST;
        buf[list + i] = mon[0];
        buf[offsets::PARTY + party_layout::COUNT] = (i + 1) as u8;
        buf[list + i + 1] = SENTINEL;
        buf[list + i + 2..list + party_layout::SPECIES_LIST_LEN].fill(0);
    }

    /// Move box slot `from_index` of box `from_box` into box `to_box`:
    /// the 33-byte record, OT name and nickname bytes move verbatim (a
    /// box→box move transforms nothing). Errors on out-of-range indexes
    /// (including `from_box == to_box` — use [`BoxMut::swap`] to reorder
    /// within a box) and on a full target box; nothing is written on
    /// error.
    pub fn move_box_to_box(
        &mut self,
        from_box: usize,
        from_index: usize,
        to_box: usize,
    ) -> Result<(), TransferError> {
        if from_box >= offsets::NUM_BOXES
            || to_box >= offsets::NUM_BOXES
            || from_box == to_box
            || from_index >= self.box_(from_box).len()
        {
            return Err(TransferError::BadIndex);
        }
        if self.box_(to_box).len() >= offsets::MONS_PER_BOX {
            return Err(TransferError::TargetFull);
        }

        let src = self.box_(from_box);
        let mut mon = [0u8; offsets::BOX_MON_SIZE];
        mon.copy_from_slice(src.mon(from_index).as_bytes());
        let mut ot = [0u8; offsets::NAME_LEN];
        ot.copy_from_slice(src.raw_ot_name(from_index));
        let mut nick = [0u8; offsets::NAME_LEN];
        nick.copy_from_slice(src.raw_nickname(from_index));

        self.box_mut(from_box).remove(from_index);
        self.box_mut(to_box)
            .add_raw(&mon, &ot, &nick)
            .expect("box capacity was checked");
        Ok(())
    }

    /// Swap party slot `party_index` with box slot `box_index` of box
    /// `box_n`, in place: the party mon is deposited into that exact box
    /// slot ([`party_to_box`]) and the box mon withdrawn into that exact
    /// party slot ([`box_to_party`], exp-derived level + recomputed
    /// stats). OT name and nickname bytes swap verbatim. Capacity-
    /// neutral, so it cannot fail on a full container; errors only on
    /// out-of-range indexes, writing nothing.
    pub fn swap_party_box(
        &mut self,
        party_index: usize,
        box_n: usize,
        box_index: usize,
    ) -> Result<(), TransferError> {
        if box_n >= offsets::NUM_BOXES
            || party_index >= self.party().len()
            || box_index >= self.box_(box_n).len()
        {
            return Err(TransferError::BadIndex);
        }

        // Copy the box-side trio out.
        let src = self.box_(box_n);
        let mut box_mon = [0u8; offsets::BOX_MON_SIZE];
        box_mon.copy_from_slice(src.mon(box_index).as_bytes());
        let mut box_ot = [0u8; offsets::NAME_LEN];
        box_ot.copy_from_slice(src.raw_ot_name(box_index));
        let mut box_nick = [0u8; offsets::NAME_LEN];
        box_nick.copy_from_slice(src.raw_nickname(box_index));

        // Copy the party-side trio out.
        let buf = self.buf();
        let mut party_mon = [0u8; offsets::PARTY_MON_SIZE];
        party_mon.copy_from_slice(
            &buf[party_layout::mon_at(party_index)
                ..party_layout::mon_at(party_index) + offsets::PARTY_MON_SIZE],
        );
        let mut party_ot = [0u8; offsets::NAME_LEN];
        party_ot.copy_from_slice(
            &buf[party_layout::ot_name_at(party_index)
                ..party_layout::ot_name_at(party_index) + offsets::NAME_LEN],
        );
        let mut party_nick = [0u8; offsets::NAME_LEN];
        party_nick.copy_from_slice(
            &buf[party_layout::nickname_at(party_index)
                ..party_layout::nickname_at(party_index) + offsets::NAME_LEN],
        );

        // Deposit the party mon into the box slot, in place.
        let deposited = party_to_box(&party_mon);
        self.box_mut(box_n)
            .replace_raw(box_index, &deposited, &party_ot, &party_nick);

        // Withdraw the box mon into the party slot, in place.
        let withdrawn = box_to_party(&box_mon);
        let buf = self.buf_mut();
        buf[party_layout::mon_at(party_index)
            ..party_layout::mon_at(party_index) + offsets::PARTY_MON_SIZE]
            .copy_from_slice(&withdrawn);
        buf[party_layout::ot_name_at(party_index)
            ..party_layout::ot_name_at(party_index) + offsets::NAME_LEN]
            .copy_from_slice(&box_ot);
        buf[party_layout::nickname_at(party_index)
            ..party_layout::nickname_at(party_index) + offsets::NAME_LEN]
            .copy_from_slice(&box_nick);
        buf[offsets::PARTY + party_layout::SPECIES_LIST + party_index] = withdrawn[0];
        Ok(())
    }
}

impl<'a> BoxView<'a> {
    /// Number of mons, clamped to [`offsets::MONS_PER_BOX`] in case the
    /// stored count byte is corrupt.
    pub fn len(&self) -> usize {
        usize::from(self.data[layout::COUNT]).min(offsets::MONS_PER_BOX)
    }

    /// Whether the box holds no mons.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The species-list entries (internal indexes) before the sentinel.
    pub fn species_list(&self) -> &[u8] {
        &self.data[layout::SPECIES_LIST..layout::SPECIES_LIST + self.len()]
    }

    /// View of mon slot `i`.
    ///
    /// # Panics
    /// If `i >= MONS_PER_BOX`.
    pub fn mon(&self, i: usize) -> BoxMonView<'a> {
        assert!(i < offsets::MONS_PER_BOX, "box slot {i} out of range");
        BoxMonView::new(&self.data[layout::mon_at(i)..layout::mon_at(i) + offsets::BOX_MON_SIZE])
    }

    /// Decoded OT name of slot `i`.
    pub fn ot_name(&self, i: usize) -> String {
        text::decode(self.raw_ot_name(i))
    }

    /// Decoded nickname of slot `i`.
    pub fn nickname(&self, i: usize) -> String {
        text::decode(self.raw_nickname(i))
    }

    fn raw_ot_name(&self, i: usize) -> &'a [u8] {
        assert!(i < offsets::MONS_PER_BOX, "box slot {i} out of range");
        &self.data[layout::ot_name_at(i)..layout::ot_name_at(i) + offsets::NAME_LEN]
    }

    fn raw_nickname(&self, i: usize) -> &'a [u8] {
        assert!(i < offsets::MONS_PER_BOX, "box slot {i} out of range");
        &self.data[layout::nickname_at(i)..layout::nickname_at(i) + offsets::NAME_LEN]
    }
}

impl<'a> BoxMut<'a> {
    /// Read-only view of the same block.
    pub fn as_view(&self) -> BoxView<'_> {
        BoxView { data: self.data }
    }

    /// Number of mons (see [`BoxView::len`]).
    pub fn len(&self) -> usize {
        self.as_view().len()
    }

    /// Whether the box holds no mons.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Mutable view of mon slot `i`.
    ///
    /// Field edits through the returned view do not touch the species
    /// list — to change a species, use [`BoxMut::set_species`] so the
    /// list stays in sync.
    ///
    /// # Panics
    /// If `i >= MONS_PER_BOX`.
    pub fn mon_mut(&mut self, i: usize) -> BoxMonMut<'_> {
        assert!(i < offsets::MONS_PER_BOX, "box slot {i} out of range");
        BoxMonMut::new(&mut self.data[layout::mon_at(i)..layout::mon_at(i) + offsets::BOX_MON_SIZE])
    }

    /// Set the species of slot `i` in both the mon record and the
    /// species list.
    ///
    /// # Panics
    /// If `i >= len()`.
    pub fn set_species(&mut self, i: usize, species: u8) {
        let len = self.len();
        assert!(i < len, "box slot {i} out of range (len {len})");
        self.data[layout::SPECIES_LIST + i] = species;
        self.mon_mut(i).set_species(species);
    }

    /// Decoded OT name of slot `i`.
    pub fn ot_name(&self, i: usize) -> String {
        self.as_view().ot_name(i)
    }

    /// Decoded nickname of slot `i`.
    pub fn nickname(&self, i: usize) -> String {
        self.as_view().nickname(i)
    }

    /// Encode and store the OT name of slot `i`.
    ///
    /// # Panics
    /// If `i >= MONS_PER_BOX`.
    pub fn set_ot_name(&mut self, i: usize, name: &str) -> Result<(), TextError> {
        assert!(i < offsets::MONS_PER_BOX, "box slot {i} out of range");
        let encoded = text::encode(name, offsets::NAME_LEN)?;
        self.data[layout::ot_name_at(i)..layout::ot_name_at(i) + offsets::NAME_LEN]
            .copy_from_slice(&encoded);
        Ok(())
    }

    /// Encode and store the nickname of slot `i`.
    ///
    /// # Panics
    /// If `i >= MONS_PER_BOX`.
    pub fn set_nickname(&mut self, i: usize, name: &str) -> Result<(), TextError> {
        assert!(i < offsets::MONS_PER_BOX, "box slot {i} out of range");
        let encoded = text::encode(name, offsets::NAME_LEN)?;
        self.data[layout::nickname_at(i)..layout::nickname_at(i) + offsets::NAME_LEN]
            .copy_from_slice(&encoded);
        Ok(())
    }

    /// Append a mon. Returns the slot it landed in. Nothing is written
    /// if the box is full or a name fails to encode.
    pub fn add(
        &mut self,
        mon: &[u8; offsets::BOX_MON_SIZE],
        ot_name: &str,
        nickname: &str,
    ) -> Result<usize, BoxError> {
        if self.len() >= offsets::MONS_PER_BOX {
            return Err(BoxError::Full);
        }
        let ot = text::encode(ot_name, offsets::NAME_LEN)?;
        let nick = text::encode(nickname, offsets::NAME_LEN)?;
        self.add_raw(
            mon,
            ot.as_slice().try_into().expect("encode returns NAME_LEN"),
            nick.as_slice().try_into().expect("encode returns NAME_LEN"),
        )
    }

    /// Append a mon with already-encoded name bytes (used by
    /// [`SaveFile::deposit`] so undecodable name bytes survive
    /// verbatim). Returns the slot it landed in.
    pub fn add_raw(
        &mut self,
        mon: &[u8; offsets::BOX_MON_SIZE],
        ot_name: &[u8; offsets::NAME_LEN],
        nickname: &[u8; offsets::NAME_LEN],
    ) -> Result<usize, BoxError> {
        let i = self.len();
        if i >= offsets::MONS_PER_BOX {
            return Err(BoxError::Full);
        }
        self.data[layout::mon_at(i)..layout::mon_at(i) + offsets::BOX_MON_SIZE]
            .copy_from_slice(mon);
        self.data[layout::ot_name_at(i)..layout::ot_name_at(i) + offsets::NAME_LEN]
            .copy_from_slice(ot_name);
        self.data[layout::nickname_at(i)..layout::nickname_at(i) + offsets::NAME_LEN]
            .copy_from_slice(nickname);
        self.data[layout::SPECIES_LIST + i] = mon[0];
        self.write_count(i + 1);
        Ok(i)
    }

    /// Overwrite occupied slot `i` in place with already-encoded bytes
    /// (mon record, OT name, nickname), keeping the species list in
    /// sync. Count is unchanged. Used by [`SaveFile::swap_party_box`]
    /// for positional swaps.
    ///
    /// # Panics
    /// If `i >= len()`.
    pub fn replace_raw(
        &mut self,
        i: usize,
        mon: &[u8; offsets::BOX_MON_SIZE],
        ot_name: &[u8; offsets::NAME_LEN],
        nickname: &[u8; offsets::NAME_LEN],
    ) {
        let len = self.len();
        assert!(i < len, "box slot {i} out of range (len {len})");
        self.data[layout::mon_at(i)..layout::mon_at(i) + offsets::BOX_MON_SIZE]
            .copy_from_slice(mon);
        self.data[layout::ot_name_at(i)..layout::ot_name_at(i) + offsets::NAME_LEN]
            .copy_from_slice(ot_name);
        self.data[layout::nickname_at(i)..layout::nickname_at(i) + offsets::NAME_LEN]
            .copy_from_slice(nickname);
        self.data[layout::SPECIES_LIST + i] = mon[0];
    }

    /// Remove the mon in slot `i`, shifting later slots down. The
    /// vacated trailing slot (mon record, OT name, nickname) is
    /// zero-filled.
    ///
    /// # Panics
    /// If `i >= len()`.
    pub fn remove(&mut self, i: usize) {
        let len = self.len();
        assert!(i < len, "box slot {i} out of range (len {len})");
        self.data.copy_within(
            layout::SPECIES_LIST + i + 1..layout::SPECIES_LIST + len,
            layout::SPECIES_LIST + i,
        );
        self.data.copy_within(
            layout::mon_at(i + 1)..layout::mon_at(len),
            layout::mon_at(i),
        );
        self.data.copy_within(
            layout::ot_name_at(i + 1)..layout::ot_name_at(len),
            layout::ot_name_at(i),
        );
        self.data.copy_within(
            layout::nickname_at(i + 1)..layout::nickname_at(len),
            layout::nickname_at(i),
        );
        let new_len = len - 1;
        self.data[layout::mon_at(new_len)..layout::mon_at(new_len + 1)].fill(0);
        self.data[layout::ot_name_at(new_len)..layout::ot_name_at(new_len + 1)].fill(0);
        self.data[layout::nickname_at(new_len)..layout::nickname_at(new_len + 1)].fill(0);
        self.write_count(new_len);
    }

    /// Swap slots `i` and `j` (species-list entries, mon records, OT
    /// names and nicknames all move together).
    ///
    /// # Panics
    /// If `i >= len()` or `j >= len()`.
    pub fn swap(&mut self, i: usize, j: usize) {
        let len = self.len();
        assert!(i < len, "box slot {i} out of range (len {len})");
        assert!(j < len, "box slot {j} out of range (len {len})");
        if i == j {
            return;
        }
        self.data
            .swap(layout::SPECIES_LIST + i, layout::SPECIES_LIST + j);
        self.swap_range(layout::mon_at(i), layout::mon_at(j), offsets::BOX_MON_SIZE);
        self.swap_range(
            layout::ot_name_at(i),
            layout::ot_name_at(j),
            offsets::NAME_LEN,
        );
        self.swap_range(
            layout::nickname_at(i),
            layout::nickname_at(j),
            offsets::NAME_LEN,
        );
    }

    /// Empty the box: count 0, sentinel in the first species-list byte,
    /// everything else in the block zeroed.
    pub fn clear(&mut self) {
        self.data.fill(0);
        self.data[layout::SPECIES_LIST] = SENTINEL;
    }

    /// Write the count byte and re-terminate the species list: sentinel
    /// at index `count`, zeros after it (deterministic content instead
    /// of stale bytes).
    fn write_count(&mut self, count: usize) {
        self.data[layout::COUNT] = count as u8;
        self.data[layout::SPECIES_LIST + count] = SENTINEL;
        self.data
            [layout::SPECIES_LIST + count + 1..layout::SPECIES_LIST + layout::SPECIES_LIST_LEN]
            .fill(0);
    }

    fn swap_range(&mut self, a: usize, b: usize, len: usize) {
        for k in 0..len {
            self.data.swap(a + k, b + k);
        }
    }
}
