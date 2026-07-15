//! One module per editor screen. Each screen is a free `ui` function
//! over `(&mut egui::Ui, &mut Doc, &mut <screen state>)`; persistent
//! per-screen UI state lives in [`ScreenState`] on the `App`.

pub mod boxes;
pub mod flags;
pub mod hex;
pub mod hof;
pub mod items;
pub mod map;
pub mod overview;
pub mod party;
pub mod pokedex;
pub mod trainer;

/// Per-screen UI state (selection, filters, scroll targets). Reset when
/// a new file is loaded.
#[derive(Default)]
pub struct ScreenState {
    pub party: party::PartyState,
    pub boxes: boxes::BoxesState,
    pub items: items::ItemsState,
    pub pokedex: pokedex::PokedexState,
    pub flags: flags::FlagsState,
    pub map: map::MapState,
    pub hof: hof::HofState,
    pub hex: hex::HexState,
}
