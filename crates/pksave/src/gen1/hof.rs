//! Hall of Fame: 50 teams × 96 bytes in bank 0 (not checksummed).
//!
//! Each team is 6 × 16-byte records: species(1), level(1), nickname(11),
//! 3 padding bytes. The team count lives in the main block
//! ([`offsets::HOF_TEAM_COUNT`], `wNumHoFTeams`).
//!
//! Empty-slot convention, from pokered `engine/movie/hall_of_fame.asm`
//! (`AnimateHallOfFame`): the game zero-fills the whole team buffer
//! (`FillMemory` of `HOF_TEAM` bytes with 0), records each party mon,
//! then writes `$FF` into the species byte of the record *after* the
//! last mon. The reader (`engine/menus/league_pc.asm`,
//! `LeaguePCShowTeam`) stops at a `$FF` species. So in a stored team the
//! first empty slot has species `0xFF` and any slots after it are all
//! zero; this module treats species `0` or `0xFF` as an empty slot and
//! [`HofTeamMut::clear_slot`] writes the game's terminator form
//! (`0xFF` species, rest of the record zeroed).
//!
//! Bank 0 is **not** covered by any checksum, so an HoF-only edit
//! changes no other byte of the serialized file (checksums are
//! recomputed but land on their old values).

use super::offsets;
use super::save::SaveFile;
use super::text::{self, TextError};

/// Species byte the game writes as the end-of-team terminator.
pub const HOF_TERMINATOR: u8 = 0xFF;
/// Mons per Hall of Fame team.
pub const HOF_TEAM_LEN: usize = offsets::HOF_TEAM_SIZE / offsets::HOF_MON_SIZE;

// Record-internal layout: species(1) + level(1) + nickname(11) + pad(3).
const REC_SPECIES: usize = 0;
const REC_LEVEL: usize = 1;
const REC_NICKNAME: usize = 2;
const _: () = assert!(REC_NICKNAME + offsets::NAME_LEN + 3 == offsets::HOF_MON_SIZE);
const _: () = assert!(HOF_TEAM_LEN == 6);

const fn team_at(t: usize) -> usize {
    offsets::HALL_OF_FAME + t * offsets::HOF_TEAM_SIZE
}

const fn mon_at(slot: usize) -> usize {
    slot * offsets::HOF_MON_SIZE
}

/// Read-only view of one 96-byte Hall of Fame team.
#[derive(Debug, Clone, Copy)]
pub struct HofTeamView<'a> {
    data: &'a [u8],
}

/// Mutable view of one Hall of Fame team.
#[derive(Debug)]
pub struct HofTeamMut<'a> {
    data: &'a mut [u8],
}

/// Read-only view of one 16-byte Hall of Fame record.
#[derive(Debug, Clone, Copy)]
pub struct HofMonView<'a> {
    data: &'a [u8],
}

impl SaveFile {
    /// Number of Hall of Fame inductions (`wNumHoFTeams`). May exceed
    /// the 50-team storage: the game keeps counting (saturating at 255)
    /// and shifts the oldest stored team out.
    pub fn hof_team_count(&self) -> u8 {
        self.buf()[offsets::HOF_TEAM_COUNT]
    }

    /// Set the Hall of Fame induction count. Lives in the main
    /// checksummed block, unlike the team data itself.
    pub fn set_hof_team_count(&mut self, count: u8) {
        self.buf_mut()[offsets::HOF_TEAM_COUNT] = count;
    }

    /// Read-only access to stored team `t` (0-based storage slot,
    /// oldest first).
    ///
    /// # Panics
    /// If `t >= HOF_TEAM_CAPACITY` (50).
    pub fn hof_team(&self, t: usize) -> HofTeamView<'_> {
        assert!(t < offsets::HOF_TEAM_CAPACITY, "HoF team {t} out of range");
        HofTeamView {
            data: &self.buf()[team_at(t)..team_at(t) + offsets::HOF_TEAM_SIZE],
        }
    }

    /// Mutable access to stored team `t`. Marks the file edited (which
    /// is harmless here: bank 0 is not checksummed, so serialization
    /// changes no byte outside the team).
    ///
    /// # Panics
    /// If `t >= HOF_TEAM_CAPACITY`.
    pub fn hof_team_mut(&mut self, t: usize) -> HofTeamMut<'_> {
        assert!(t < offsets::HOF_TEAM_CAPACITY, "HoF team {t} out of range");
        HofTeamMut {
            data: &mut self.buf_mut()[team_at(t)..team_at(t) + offsets::HOF_TEAM_SIZE],
        }
    }
}

impl<'a> HofTeamView<'a> {
    /// Record `slot` (0-5), or `None` if the slot is empty (species 0
    /// or the `0xFF` terminator).
    ///
    /// # Panics
    /// If `slot >= 6`.
    pub fn mon(&self, slot: usize) -> Option<HofMonView<'a>> {
        assert!(slot < HOF_TEAM_LEN, "HoF slot {slot} out of range");
        let data = &self.data[mon_at(slot)..mon_at(slot) + offsets::HOF_MON_SIZE];
        match data[REC_SPECIES] {
            0 | HOF_TERMINATOR => None,
            _ => Some(HofMonView { data }),
        }
    }

    /// Number of leading occupied slots (the game's reader stops at the
    /// first empty one).
    pub fn len(&self) -> usize {
        (0..HOF_TEAM_LEN)
            .take_while(|&slot| self.mon(slot).is_some())
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl HofTeamMut<'_> {
    /// Read-only view of the same team.
    pub fn as_view(&self) -> HofTeamView<'_> {
        HofTeamView { data: self.data }
    }

    /// See [`HofTeamView::mon`].
    pub fn mon(&self, slot: usize) -> Option<HofMonView<'_>> {
        self.as_view().mon(slot)
    }

    /// See [`HofTeamView::len`].
    pub fn len(&self) -> usize {
        self.as_view().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Write record `slot`: species, level, encoded nickname, zeroed
    /// padding. Nothing is written if the nickname fails to encode.
    ///
    /// # Panics
    /// If `slot >= 6`.
    pub fn set_mon(
        &mut self,
        slot: usize,
        species: u8,
        level: u8,
        nickname: &str,
    ) -> Result<(), TextError> {
        assert!(slot < HOF_TEAM_LEN, "HoF slot {slot} out of range");
        let nick = text::encode(nickname, offsets::NAME_LEN)?;
        let rec = &mut self.data[mon_at(slot)..mon_at(slot) + offsets::HOF_MON_SIZE];
        rec[REC_SPECIES] = species;
        rec[REC_LEVEL] = level;
        rec[REC_NICKNAME..REC_NICKNAME + offsets::NAME_LEN].copy_from_slice(&nick);
        rec[REC_NICKNAME + offsets::NAME_LEN..].fill(0);
        Ok(())
    }

    /// Empty record `slot` the way the game terminates a team: species
    /// `0xFF`, everything else zero. Note that (as in the game) an empty
    /// slot hides any occupied slots after it from readers.
    ///
    /// # Panics
    /// If `slot >= 6`.
    pub fn clear_slot(&mut self, slot: usize) {
        assert!(slot < HOF_TEAM_LEN, "HoF slot {slot} out of range");
        let rec = &mut self.data[mon_at(slot)..mon_at(slot) + offsets::HOF_MON_SIZE];
        rec.fill(0);
        rec[REC_SPECIES] = HOF_TERMINATOR;
    }
}

impl HofMonView<'_> {
    /// Species (internal index).
    pub fn species(&self) -> u8 {
        self.data[REC_SPECIES]
    }

    /// Level at induction.
    pub fn level(&self) -> u8 {
        self.data[REC_LEVEL]
    }

    /// Decoded nickname.
    pub fn nickname(&self) -> String {
        text::decode(&self.data[REC_NICKNAME..REC_NICKNAME + offsets::NAME_LEN])
    }
}
