//! Static Gen 1 data tables (generated from pret/pokered by `cargo xtask gen-tables`).

/// Base stats record for one species, as stored in pokered
/// `data/pokemon/base_stats/*.asm`.
///
/// `type1`/`type2` are Gen 1 type ids (see [`generated::types::TYPE_NAMES`]);
/// `growth_rate` is a `GROWTH_*` constant value
/// (pokered `constants/pokemon_data_constants.asm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    /// Base HP.
    pub hp: u8,
    /// Base Attack.
    pub attack: u8,
    /// Base Defense.
    pub defense: u8,
    /// Base Speed.
    pub speed: u8,
    /// Base Special (one stat in Gen 1).
    pub special: u8,
    /// Primary type id.
    pub type1: u8,
    /// Secondary type id (equal to `type1` for mono-typed species).
    pub type2: u8,
    /// Catch rate (higher = easier to catch).
    pub catch_rate: u8,
    /// Base experience yield when defeated.
    pub exp_yield: u8,
    /// Experience growth curve: a `GROWTH_*` constant value.
    pub growth_rate: u8,
}

/// One entry of the Gen 1 move table (pokered `data/moves/moves.asm`).
///
/// `type_` is a Gen 1 type id; `accuracy` is the raw ROM byte
/// (out of 255, i.e. `percent * 255 / 100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveInfo {
    /// Display name, e.g. `"Thunderbolt"`.
    pub name: &'static str,
    /// Base power (0 for status moves).
    pub power: u8,
    /// Gen 1 type id of the move.
    pub type_: u8,
    /// Accuracy as a raw ROM byte out of 255.
    pub accuracy: u8,
    /// Maximum PP before PP Ups.
    pub pp: u8,
}

/// Tables generated from pret/pokered; do not edit by hand
/// (regenerate with `cargo xtask gen-tables`).
pub mod generated {
    /// Event flag names (`wEventFlags` bit index → `EVENT_*` label).
    pub mod events;
    /// Item id → name table.
    pub mod items;
    /// Map id → name table.
    pub mod maps;
    /// Move id → [`MoveInfo`](super::MoveInfo) table.
    pub mod moves;
    /// Species tables: names, base stats, dex ↔ internal index mapping.
    pub mod species;
    /// Gen 1 type id → name table.
    pub mod types;
}

pub use generated::events::EVENT_FLAG_NAMES;
pub use generated::items::ITEM_NAMES;
pub use generated::maps::MAP_NAMES;
pub use generated::moves::MOVES;
pub use generated::species::{BASE_STATS, DEX_TO_INDEX, INDEX_TO_DEX, SPECIES_NAMES};
pub use generated::types::TYPE_NAMES;
