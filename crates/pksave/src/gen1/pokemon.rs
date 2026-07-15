//! Zero-copy views over the Gen 1 Pokémon record formats.
//!
//! A *party mon* is 44 bytes ([`offsets::PARTY_MON_SIZE`]); a *box mon*
//! is its first 33 bytes ([`offsets::BOX_MON_SIZE`]). Field layout per
//! `docs/FORMAT.md` ("Pokémon structures"); all multi-byte integers are
//! big-endian. The level byte at `+0x03` is the *box* level — stale for
//! party mons, whose authoritative level lives at `+0x21`.

use super::data::{BASE_STATS, INDEX_TO_DEX};
use super::offsets;
use super::stats::{self, Dvs};

/// Field offsets within a mon record (relative to the record start).
/// See `docs/FORMAT.md`.
mod off {
    pub const SPECIES: usize = 0x00;
    pub const CURRENT_HP: usize = 0x01;
    /// Box copy of the level — stale in party records (see `LEVEL`).
    pub const BOX_LEVEL: usize = 0x03;
    pub const STATUS: usize = 0x04;
    pub const TYPE1: usize = 0x05;
    pub const TYPE2: usize = 0x06;
    pub const CATCH_RATE: usize = 0x07;
    pub const MOVES: usize = 0x08;
    pub const OT_ID: usize = 0x0C;
    /// 3 bytes, big-endian.
    pub const EXP: usize = 0x0E;
    /// 5 × u16 BE: HP, Attack, Defense, Speed, Special.
    pub const STAT_EXP: usize = 0x11;
    pub const DVS: usize = 0x1B;
    pub const PP: usize = 0x1D;
    // Party-only fields:
    pub const LEVEL: usize = 0x21;
    pub const MAX_HP: usize = 0x22;
    pub const ATTACK: usize = 0x24;
    pub const DEFENSE: usize = 0x26;
    pub const SPEED: usize = 0x28;
    pub const SPECIAL: usize = 0x2A;
}

/// Status-condition bit positions in the status byte (`+0x04`).
/// Bits 0-2 hold the sleep-turn counter.
pub const STATUS_SLEEP_MASK: u8 = 0b0000_0111;
pub const STATUS_POISONED: u8 = 1 << 3;
pub const STATUS_BURNED: u8 = 1 << 4;
pub const STATUS_FROZEN: u8 = 1 << 5;
pub const STATUS_PARALYZED: u8 = 1 << 6;

fn get_u16(bytes: &[u8], at: usize) -> u16 {
    u16::from_be_bytes([bytes[at], bytes[at + 1]])
}

fn set_u16(bytes: &mut [u8], at: usize, value: u16) {
    bytes[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

/// Read-only view of a 44-byte party mon record.
#[derive(Debug, Clone, Copy)]
pub struct PartyMonView<'a>(&'a [u8]);

/// Mutable view of a 44-byte party mon record.
#[derive(Debug)]
pub struct PartyMonMut<'a>(&'a mut [u8]);

/// Read-only view of a 33-byte box mon record.
#[derive(Debug, Clone, Copy)]
pub struct BoxMonView<'a>(&'a [u8]);

/// Mutable view of a 33-byte box mon record.
#[derive(Debug)]
pub struct BoxMonMut<'a>(&'a mut [u8]);

impl<'a> PartyMonView<'a> {
    /// # Panics
    /// If `bytes` is not exactly [`offsets::PARTY_MON_SIZE`] long.
    pub fn new(bytes: &'a [u8]) -> Self {
        assert_eq!(bytes.len(), offsets::PARTY_MON_SIZE, "party mon record");
        PartyMonView(bytes)
    }
}

impl<'a> PartyMonMut<'a> {
    /// # Panics
    /// If `bytes` is not exactly [`offsets::PARTY_MON_SIZE`] long.
    pub fn new(bytes: &'a mut [u8]) -> Self {
        assert_eq!(bytes.len(), offsets::PARTY_MON_SIZE, "party mon record");
        PartyMonMut(bytes)
    }
}

impl<'a> BoxMonView<'a> {
    /// # Panics
    /// If `bytes` is not exactly [`offsets::BOX_MON_SIZE`] long.
    pub fn new(bytes: &'a [u8]) -> Self {
        assert_eq!(bytes.len(), offsets::BOX_MON_SIZE, "box mon record");
        BoxMonView(bytes)
    }
}

impl<'a> BoxMonMut<'a> {
    /// # Panics
    /// If `bytes` is not exactly [`offsets::BOX_MON_SIZE`] long.
    pub fn new(bytes: &'a mut [u8]) -> Self {
        assert_eq!(bytes.len(), offsets::BOX_MON_SIZE, "box mon record");
        BoxMonMut(bytes)
    }
}

/// Getters for the fields shared by party and box records (the first 33
/// bytes).
macro_rules! common_getters {
    ($ty:ident) => {
        impl<'a> $ty<'a> {
            /// The raw record bytes.
            pub fn as_bytes(&self) -> &[u8] {
                &self.0
            }

            /// Species, as the *internal index* (not National Dex).
            pub fn species(&self) -> u8 {
                self.0[off::SPECIES]
            }

            pub fn current_hp(&self) -> u16 {
                get_u16(&self.0, off::CURRENT_HP)
            }

            /// The level byte at `+0x03`. For party mons this is the
            /// stale box copy; the authoritative level is `level()`.
            pub fn box_level(&self) -> u8 {
                self.0[off::BOX_LEVEL]
            }

            /// Raw status byte (see the `STATUS_*` constants).
            pub fn status(&self) -> u8 {
                self.0[off::STATUS]
            }

            /// Remaining sleep turns (0 = awake), status bits 0-2.
            pub fn sleep_turns(&self) -> u8 {
                self.status() & STATUS_SLEEP_MASK
            }

            pub fn is_poisoned(&self) -> bool {
                self.status() & STATUS_POISONED != 0
            }

            pub fn is_burned(&self) -> bool {
                self.status() & STATUS_BURNED != 0
            }

            pub fn is_frozen(&self) -> bool {
                self.status() & STATUS_FROZEN != 0
            }

            pub fn is_paralyzed(&self) -> bool {
                self.status() & STATUS_PARALYZED != 0
            }

            /// `(type1, type2)`; equal for monotype species.
            pub fn types(&self) -> (u8, u8) {
                (self.0[off::TYPE1], self.0[off::TYPE2])
            }

            pub fn catch_rate(&self) -> u8 {
                self.0[off::CATCH_RATE]
            }

            /// The four move indexes (0 = empty slot).
            pub fn moves(&self) -> [u8; 4] {
                [
                    self.0[off::MOVES],
                    self.0[off::MOVES + 1],
                    self.0[off::MOVES + 2],
                    self.0[off::MOVES + 3],
                ]
            }

            pub fn ot_id(&self) -> u16 {
                get_u16(&self.0, off::OT_ID)
            }

            /// Total experience (3 bytes big-endian).
            pub fn exp(&self) -> u32 {
                u32::from(self.0[off::EXP]) << 16
                    | u32::from(self.0[off::EXP + 1]) << 8
                    | u32::from(self.0[off::EXP + 2])
            }

            /// Stat experience in record order: HP, Attack, Defense,
            /// Speed, Special.
            pub fn stat_exps(&self) -> [u16; 5] {
                [
                    get_u16(&self.0, off::STAT_EXP),
                    get_u16(&self.0, off::STAT_EXP + 2),
                    get_u16(&self.0, off::STAT_EXP + 4),
                    get_u16(&self.0, off::STAT_EXP + 6),
                    get_u16(&self.0, off::STAT_EXP + 8),
                ]
            }

            pub fn dvs(&self) -> Dvs {
                Dvs::unpack([self.0[off::DVS], self.0[off::DVS + 1]])
            }

            /// The four raw PP bytes (decode with [`stats::current_pp`] /
            /// [`stats::pp_ups`]).
            pub fn pp(&self) -> [u8; 4] {
                [
                    self.0[off::PP],
                    self.0[off::PP + 1],
                    self.0[off::PP + 2],
                    self.0[off::PP + 3],
                ]
            }
        }
    };
}

common_getters!(PartyMonView);
common_getters!(PartyMonMut);
common_getters!(BoxMonView);
common_getters!(BoxMonMut);

/// Setters for the fields shared by party and box records.
macro_rules! common_setters {
    ($ty:ident) => {
        impl<'a> $ty<'a> {
            /// Set the species byte (internal index). Note: a mon inside
            /// a party/box list must keep the block's species list in
            /// sync — prefer `PartyMut::set_species` there.
            pub fn set_species(&mut self, species: u8) {
                self.0[off::SPECIES] = species;
            }

            pub fn set_current_hp(&mut self, hp: u16) {
                set_u16(&mut self.0, off::CURRENT_HP, hp);
            }

            pub fn set_box_level(&mut self, level: u8) {
                self.0[off::BOX_LEVEL] = level;
            }

            pub fn set_status(&mut self, status: u8) {
                self.0[off::STATUS] = status;
            }

            pub fn set_types(&mut self, type1: u8, type2: u8) {
                self.0[off::TYPE1] = type1;
                self.0[off::TYPE2] = type2;
            }

            pub fn set_catch_rate(&mut self, catch_rate: u8) {
                self.0[off::CATCH_RATE] = catch_rate;
            }

            pub fn set_moves(&mut self, moves: [u8; 4]) {
                self.0[off::MOVES..off::MOVES + 4].copy_from_slice(&moves);
            }

            pub fn set_ot_id(&mut self, id: u16) {
                set_u16(&mut self.0, off::OT_ID, id);
            }

            /// Set total experience (masked to 24 bits).
            pub fn set_exp(&mut self, exp: u32) {
                self.0[off::EXP] = (exp >> 16) as u8;
                self.0[off::EXP + 1] = (exp >> 8) as u8;
                self.0[off::EXP + 2] = exp as u8;
            }

            /// Set stat experience in record order: HP, Attack, Defense,
            /// Speed, Special.
            pub fn set_stat_exps(&mut self, stat_exps: [u16; 5]) {
                for (i, se) in stat_exps.into_iter().enumerate() {
                    set_u16(&mut self.0, off::STAT_EXP + 2 * i, se);
                }
            }

            pub fn set_dvs(&mut self, dvs: Dvs) {
                let packed = dvs.pack();
                self.0[off::DVS] = packed[0];
                self.0[off::DVS + 1] = packed[1];
            }

            /// Set the four raw PP bytes (compose with
            /// [`stats::compose_pp`]).
            pub fn set_pp(&mut self, pp: [u8; 4]) {
                self.0[off::PP..off::PP + 4].copy_from_slice(&pp);
            }
        }
    };
}

common_setters!(PartyMonMut);
common_setters!(BoxMonMut);

/// Party-only getters (bytes 0x21..0x2C).
macro_rules! party_getters {
    ($ty:ident) => {
        impl<'a> $ty<'a> {
            /// The authoritative party level (`+0x21`).
            pub fn level(&self) -> u8 {
                self.0[off::LEVEL]
            }

            pub fn max_hp(&self) -> u16 {
                get_u16(&self.0, off::MAX_HP)
            }

            pub fn attack(&self) -> u16 {
                get_u16(&self.0, off::ATTACK)
            }

            pub fn defense(&self) -> u16 {
                get_u16(&self.0, off::DEFENSE)
            }

            pub fn speed(&self) -> u16 {
                get_u16(&self.0, off::SPEED)
            }

            pub fn special(&self) -> u16 {
                get_u16(&self.0, off::SPECIAL)
            }
        }
    };
}

party_getters!(PartyMonView);
party_getters!(PartyMonMut);

impl<'a> PartyMonMut<'a> {
    /// Read-only view of the same record.
    pub fn as_view(&self) -> PartyMonView<'_> {
        PartyMonView(self.0)
    }

    /// Set the authoritative party level byte (`+0x21`) only. Use
    /// [`PartyMonMut::set_level_coherent`] to keep exp and stats in step.
    pub fn set_level(&mut self, level: u8) {
        self.0[off::LEVEL] = level;
    }

    pub fn set_max_hp(&mut self, value: u16) {
        set_u16(self.0, off::MAX_HP, value);
    }

    pub fn set_attack(&mut self, value: u16) {
        set_u16(self.0, off::ATTACK, value);
    }

    pub fn set_defense(&mut self, value: u16) {
        set_u16(self.0, off::DEFENSE, value);
    }

    pub fn set_speed(&mut self, value: u16) {
        set_u16(self.0, off::SPEED, value);
    }

    pub fn set_special(&mut self, value: u16) {
        set_u16(self.0, off::SPECIAL, value);
    }

    /// Recompute the five calculated stats (max HP, Attack, Defense,
    /// Speed, Special) from species base stats, DVs, stat exp and the
    /// party level, exactly as a box withdrawal does. An invalid species
    /// (MissingNo/glitch) maps to all-zero base stats, yielding the
    /// formula's level-scaled minimums.
    pub fn recalculate_stats(&mut self) {
        let base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(self.species())])];
        let dvs = self.dvs();
        let level = self.level();
        let se = self.stat_exps();
        self.set_max_hp(stats::calc_stat(base.hp, dvs.hp_dv(), se[0], level, true));
        self.set_attack(stats::calc_stat(
            base.attack,
            dvs.attack,
            se[1],
            level,
            false,
        ));
        self.set_defense(stats::calc_stat(
            base.defense,
            dvs.defense,
            se[2],
            level,
            false,
        ));
        self.set_speed(stats::calc_stat(base.speed, dvs.speed, se[3], level, false));
        self.set_special(stats::calc_stat(
            base.special,
            dvs.special,
            se[4],
            level,
            false,
        ));
    }

    /// Set the level and bring every level-derived field in step: both
    /// level bytes (`+0x21` and the box copy at `+0x03`), exp :=
    /// `exp_for_level(growth_rate, level)`, recalculated stats, and —
    /// deliberate policy — current HP := new max HP (full heal). This
    /// mirrors what the player sees after a Pokémon Center visit and
    /// avoids leaving current HP above max after a level decrease.
    pub fn set_level_coherent(&mut self, level: u8) {
        let base = BASE_STATS[usize::from(INDEX_TO_DEX[usize::from(self.species())])];
        self.set_level(level);
        self.set_box_level(level);
        self.set_exp(stats::exp_for_level(base.growth_rate, level));
        self.recalculate_stats();
        let max_hp = self.max_hp();
        self.set_current_hp(max_hp);
    }
}

impl<'a> BoxMonMut<'a> {
    /// Read-only view of the same record.
    pub fn as_view(&self) -> BoxMonView<'_> {
        BoxMonView(self.0)
    }
}

/// Convert a party record to a box record: truncate to 33 bytes, then
/// copy the authoritative party level (`+0x21`) over the box level byte
/// (`+0x03`) so the box record carries the true level (as the game's
/// deposit path does).
pub fn party_to_box(party: &[u8; offsets::PARTY_MON_SIZE]) -> [u8; offsets::BOX_MON_SIZE] {
    let mut out = [0u8; offsets::BOX_MON_SIZE];
    out.copy_from_slice(&party[..offsets::BOX_MON_SIZE]);
    out[off::BOX_LEVEL] = party[off::LEVEL];
    out
}

/// Convert a box record to a party record, as a withdrawal does: the
/// party level (`+0x21`) := box level (`+0x03`) and the five calculated
/// stats are recomputed from base stats + DVs + stat exp. Current HP is
/// kept as stored (it lives within the first 33 bytes).
pub fn box_to_party(box_: &[u8; offsets::BOX_MON_SIZE]) -> [u8; offsets::PARTY_MON_SIZE] {
    let mut out = [0u8; offsets::PARTY_MON_SIZE];
    out[..offsets::BOX_MON_SIZE].copy_from_slice(box_);
    out[off::LEVEL] = box_[off::BOX_LEVEL];
    PartyMonMut::new(&mut out).recalculate_stats();
    out
}
