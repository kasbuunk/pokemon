//! Generation 1 (Red/Blue/Yellow) save file support.
//!
//! Offsets and structure come from the pret/pokered disassembly; see
//! `docs/FORMAT.md` at the repository root. `offsets.rs` is the single
//! source of truth for the layout and is cross-checked in CI against the
//! symbol file of a pokered build (`cargo xtask gen-offsets-check`).

pub mod bcd;
pub mod boxes;
pub mod checksum;
pub mod data;
pub mod daycare;
pub mod detect;
mod engine_state;
pub mod events;
pub mod flags;
pub mod hof;
pub mod items;
pub mod map;
pub mod offsets;
pub mod party;
pub mod pokedex;
pub mod pokemon;
pub mod save;
pub mod stats;
pub mod text;
pub mod trainer;
pub mod validate;
