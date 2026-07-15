//! Static Gen 1 data tables (generated from pret/pokered by `cargo xtask gen-tables`).

/// Base stats record for one species, as stored in pokered
/// `data/pokemon/base_stats/*.asm`.
///
/// `type1`/`type2` are Gen 1 type ids (see [`generated::types::TYPE_NAMES`]);
/// `growth_rate` is a `GROWTH_*` constant value
/// (pokered `constants/pokemon_data_constants.asm`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseStats {
    pub hp: u8,
    pub attack: u8,
    pub defense: u8,
    pub speed: u8,
    pub special: u8,
    pub type1: u8,
    pub type2: u8,
    pub catch_rate: u8,
    pub exp_yield: u8,
    pub growth_rate: u8,
}

/// One entry of the Gen 1 move table (pokered `data/moves/moves.asm`).
///
/// `type_` is a Gen 1 type id; `accuracy` is the raw ROM byte
/// (out of 255, i.e. `percent * 255 / 100`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveInfo {
    pub name: &'static str,
    pub power: u8,
    pub type_: u8,
    pub accuracy: u8,
    pub pp: u8,
}

pub mod generated {
    pub mod events;
    pub mod items;
    pub mod maps;
    pub mod moves;
    pub mod species;
    pub mod types;
}

pub use generated::events::EVENT_FLAG_NAMES;
pub use generated::items::ITEM_NAMES;
pub use generated::maps::MAP_NAMES;
pub use generated::moves::MOVES;
pub use generated::species::{BASE_STATS, DEX_TO_INDEX, INDEX_TO_DEX, SPECIES_NAMES};
pub use generated::types::TYPE_NAMES;
