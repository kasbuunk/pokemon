//! Overworld flag arrays: event flags, missables, hidden items/coins,
//! fly-unlocked towns and the raw game-progress region.
//!
//! All of these are pokered `flag_array`s — LSB-first bit fields
//! addressed through [`flags::BitSlice`](super::flags::BitSlice). Bit
//! indexes match the
//! `EVENT_*` / object constants in the disassembly; event flag *names*
//! come from the generated [`EVENT_FLAG_NAMES`] table (2560 entries,
//! `""` for the unnamed gaps).

use super::data::EVENT_FLAG_NAMES;
use super::flags::{BitSlice, BitSliceMut};
use super::offsets;
use super::save::SaveFile;

/// The eleven fly-destination towns, in `wTownVisitedFlag` bit order
/// (bit 0 = Pallet Town). This is the map-id order of pokered
/// `constants/map_constants.asm`, whose first eleven entries are the
/// towns/cities — note Saffron City is *last* (map id 0x0A), after
/// Indigo Plateau.
pub const TOWN_NAMES: [&str; 11] = [
    "Pallet Town",
    "Viridian City",
    "Pewter City",
    "Cerulean City",
    "Lavender Town",
    "Vermilion City",
    "Celadon City",
    "Fuchsia City",
    "Cinnabar Island",
    "Indigo Plateau",
    "Saffron City",
];

/// Number of hidden-coin flag bits (2 bytes).
pub const HIDDEN_COIN_FLAGS_LEN: usize = 2;
/// Number of town-visited flag bytes.
const TOWN_FLAGS_LEN: usize = 2;

impl SaveFile {
    fn bits(&self, at: usize, len: usize) -> BitSlice<'_> {
        BitSlice::new(&self.buf()[at..at + len])
    }

    fn bits_mut(&mut self, at: usize, len: usize) -> BitSliceMut<'_> {
        BitSliceMut::new(&mut self.buf_mut()[at..at + len])
    }

    // ---- event flags (wEventFlags, 2560 bits) ----

    /// Read event flag `bit` (0..2560). Story milestones and one flag
    /// per battled trainer; named bits are in [`EVENT_FLAG_NAMES`].
    ///
    /// # Panics
    /// If `bit >= NUM_EVENTS`.
    pub fn event_flag(&self, bit: usize) -> bool {
        self.bits(offsets::EVENT_FLAGS, offsets::EVENT_FLAGS_LEN)
            .get(bit)
    }

    /// Write event flag `bit`.
    ///
    /// # Panics
    /// If `bit >= NUM_EVENTS`.
    pub fn set_event_flag(&mut self, bit: usize, value: bool) {
        self.bits_mut(offsets::EVENT_FLAGS, offsets::EVENT_FLAGS_LEN)
            .set(bit, value);
    }

    /// Read an event flag by its pokered `EVENT_*` name, or `None` if no
    /// bit carries that name. Linear scan of the 2560-entry table.
    pub fn event_flag_by_name(&self, name: &str) -> Option<bool> {
        let bit = event_bit_by_name(name)?;
        Some(self.event_flag(bit))
    }

    /// Write an event flag by name. Returns `false` (writing nothing) if
    /// no bit carries that name.
    pub fn set_event_flag_by_name(&mut self, name: &str, value: bool) -> bool {
        match event_bit_by_name(name) {
            Some(bit) => {
                self.set_event_flag(bit, value);
                true
            }
            None => false,
        }
    }

    /// All *named* event flags as `(bit index, name, value)`, ascending
    /// by bit; the unnamed `""` gaps are skipped.
    pub fn named_event_flags(&self) -> impl Iterator<Item = (usize, &'static str, bool)> + '_ {
        let bits = self.bits(offsets::EVENT_FLAGS, offsets::EVENT_FLAGS_LEN);
        EVENT_FLAG_NAMES
            .iter()
            .enumerate()
            .filter(|(_, name)| !name.is_empty())
            .map(move |(bit, &name)| (bit, name, bits.get(bit)))
    }

    // ---- missable object flags (wToggleableObjectFlags, 256 bits) ----

    /// Read missable/toggleable overworld-object flag `bit` (0..256).
    /// Index-based only: the bit-to-object mapping lives in pokered
    /// `data/maps/toggleable_objects.asm`.
    ///
    /// # Panics
    /// If `bit >= 256`.
    pub fn missable_flag(&self, bit: usize) -> bool {
        self.bits(offsets::MISSABLE_FLAGS, offsets::MISSABLE_FLAGS_LEN)
            .get(bit)
    }

    /// Write missable flag `bit`.
    ///
    /// # Panics
    /// If `bit >= 256`.
    pub fn set_missable_flag(&mut self, bit: usize, value: bool) {
        self.bits_mut(offsets::MISSABLE_FLAGS, offsets::MISSABLE_FLAGS_LEN)
            .set(bit, value);
    }

    // ---- hidden item / hidden coin pickup flags ----

    /// Read hidden-item pickup flag `bit` (0..112, 14 bytes).
    ///
    /// # Panics
    /// If `bit >= 112`.
    pub fn hidden_item_flag(&self, bit: usize) -> bool {
        self.bits(offsets::HIDDEN_ITEM_FLAGS, offsets::HIDDEN_ITEM_FLAGS_LEN)
            .get(bit)
    }

    /// Write hidden-item pickup flag `bit`.
    ///
    /// # Panics
    /// If `bit >= 112`.
    pub fn set_hidden_item_flag(&mut self, bit: usize, value: bool) {
        self.bits_mut(offsets::HIDDEN_ITEM_FLAGS, offsets::HIDDEN_ITEM_FLAGS_LEN)
            .set(bit, value);
    }

    /// Read hidden-coin pickup flag `bit` (0..16, 2 bytes).
    ///
    /// # Panics
    /// If `bit >= 16`.
    pub fn hidden_coin_flag(&self, bit: usize) -> bool {
        self.bits(offsets::HIDDEN_COIN_FLAGS, HIDDEN_COIN_FLAGS_LEN)
            .get(bit)
    }

    /// Write hidden-coin pickup flag `bit`.
    ///
    /// # Panics
    /// If `bit >= 16`.
    pub fn set_hidden_coin_flag(&mut self, bit: usize, value: bool) {
        self.bits_mut(offsets::HIDDEN_COIN_FLAGS, HIDDEN_COIN_FLAGS_LEN)
            .set(bit, value);
    }

    // ---- town visited / fly flags (wTownVisitedFlag) ----

    /// Whether town `index` (see [`TOWN_NAMES`], bit 0 = Pallet Town) is
    /// fly-unlocked.
    ///
    /// # Panics
    /// If `index >= 11`.
    pub fn town_visited(&self, index: usize) -> bool {
        assert!(index < TOWN_NAMES.len(), "town index {index} out of range");
        self.bits(offsets::TOWN_VISITED_FLAGS, TOWN_FLAGS_LEN)
            .get(index)
    }

    /// Set whether town `index` is fly-unlocked.
    ///
    /// # Panics
    /// If `index >= 11`.
    pub fn set_town_visited(&mut self, index: usize, visited: bool) {
        assert!(index < TOWN_NAMES.len(), "town index {index} out of range");
        self.bits_mut(offsets::TOWN_VISITED_FLAGS, TOWN_FLAGS_LEN)
            .set(index, visited);
    }

    // ---- game progress flags (wGameProgressFlags, raw) ----

    /// The raw `wGameProgressFlags` region (0xC8 bytes). Its internal
    /// structure is engine state; exposed raw for inspection.
    pub fn game_progress_flags(&self) -> &[u8] {
        &self.buf()[offsets::GAME_PROGRESS_FLAGS
            ..offsets::GAME_PROGRESS_FLAGS + offsets::GAME_PROGRESS_FLAGS_LEN]
    }

    /// Mutable access to the raw game-progress region. Marks the file
    /// edited.
    pub fn game_progress_flags_mut(&mut self) -> &mut [u8] {
        &mut self.buf_mut()[offsets::GAME_PROGRESS_FLAGS
            ..offsets::GAME_PROGRESS_FLAGS + offsets::GAME_PROGRESS_FLAGS_LEN]
    }
}

/// Bit index of the event flag named `name`, if any. Linear scan — 2560
/// entries is small enough that an index structure isn't worth carrying.
fn event_bit_by_name(name: &str) -> Option<usize> {
    if name.is_empty() {
        return None;
    }
    EVENT_FLAG_NAMES.iter().position(|&n| n == name)
}
