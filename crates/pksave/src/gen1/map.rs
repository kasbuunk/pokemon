//! Player position: current map, coordinates, block coordinates, last
//! map, tileset and the tile-block map view pointer.
//!
//! These fields are a raw copy of live engine WRAM, so they must stay
//! mutually coherent: `wCurrentTileBlockMapViewPointer` points into the
//! WRAM tile-block buffer for the player's current block, the block
//! coordinates hold the player's parity within a 2×2-tile block, and the
//! tileset must match the map. Setting the map id alone produces a save
//! the game will render as garbage or walk-through-walls glitch until
//! the next warp. [`SaveFile::warp_to`] is a best-effort helper; for
//! full coherence the tileset and view pointer must also be set to
//! values captured from a real save on that map.

use super::data::MAP_NAMES;
use super::offsets;
use super::save::SaveFile;

impl SaveFile {
    /// Current map id (`wCurMap`).
    pub fn cur_map(&self) -> u8 {
        self.buf()[offsets::CUR_MAP]
    }

    /// Set the current map id **only**. See the module docs — coords,
    /// block coords, tileset and view pointer must be made coherent or
    /// the game may glitch; prefer [`SaveFile::warp_to`].
    pub fn set_cur_map(&mut self, map_id: u8) {
        self.buf_mut()[offsets::CUR_MAP] = map_id;
    }

    /// Name of the current map (pokered map constant), or `None` for an
    /// id with no map.
    pub fn cur_map_name(&self) -> Option<&'static str> {
        let name = MAP_NAMES[usize::from(self.cur_map())];
        (!name.is_empty()).then_some(name)
    }

    /// Player X coordinate (`wXCoord`, in 1×1 tiles).
    pub fn x_coord(&self) -> u8 {
        self.buf()[offsets::X_COORD]
    }

    pub fn set_x_coord(&mut self, x: u8) {
        self.buf_mut()[offsets::X_COORD] = x;
    }

    /// Player Y coordinate (`wYCoord`).
    pub fn y_coord(&self) -> u8 {
        self.buf()[offsets::Y_COORD]
    }

    pub fn set_y_coord(&mut self, y: u8) {
        self.buf_mut()[offsets::Y_COORD] = y;
    }

    /// X block coordinate (`wXBlockCoord`): the player's parity within
    /// the current 2×2-tile block (0 or 1).
    pub fn x_block_coord(&self) -> u8 {
        self.buf()[offsets::X_BLOCK_COORD]
    }

    pub fn set_x_block_coord(&mut self, v: u8) {
        self.buf_mut()[offsets::X_BLOCK_COORD] = v;
    }

    /// Y block coordinate (`wYBlockCoord`).
    pub fn y_block_coord(&self) -> u8 {
        self.buf()[offsets::Y_BLOCK_COORD]
    }

    pub fn set_y_block_coord(&mut self, v: u8) {
        self.buf_mut()[offsets::Y_BLOCK_COORD] = v;
    }

    /// Last outdoor map (`wLastMap`, used by dungeon warps / Dig /
    /// Escape Rope).
    pub fn last_map(&self) -> u8 {
        self.buf()[offsets::LAST_MAP]
    }

    pub fn set_last_map(&mut self, map_id: u8) {
        self.buf_mut()[offsets::LAST_MAP] = map_id;
    }

    /// Current tileset id (`wCurMapTileset`).
    pub fn tileset(&self) -> u8 {
        self.buf()[offsets::CUR_MAP_TILESET]
    }

    pub fn set_tileset(&mut self, tileset: u8) {
        self.buf_mut()[offsets::CUR_MAP_TILESET] = tileset;
    }

    /// Raw `wCurrentTileBlockMapViewPointer`: a little-endian pointer
    /// into WRAM's tile-block buffer for the block the player stands in.
    pub fn map_view_pointer(&self) -> u16 {
        u16::from_le_bytes([
            self.buf()[offsets::MAP_VIEW_POINTER],
            self.buf()[offsets::MAP_VIEW_POINTER + 1],
        ])
    }

    /// Set the raw map view pointer (little-endian).
    pub fn set_map_view_pointer(&mut self, pointer: u16) {
        self.buf_mut()[offsets::MAP_VIEW_POINTER..offsets::MAP_VIEW_POINTER + 2]
            .copy_from_slice(&pointer.to_le_bytes());
    }

    /// Best-effort teleport: set the map id, X/Y coordinates and the
    /// block coordinates derived from them.
    ///
    /// The block-coordinate derivation is exactly the game's own: after
    /// a warp, `LoadTilesetHeader` (pokered
    /// `engine/overworld/tilesets.asm`, past
    /// `LoadDestinationWarpPosition`) computes
    /// `wYBlockCoord := wYCoord & 1` and `wXBlockCoord := wXCoord & 1` —
    /// the parity of the player inside a 2×2-tile block.
    ///
    /// **Not** updated (see the module docs): `wLastMap`, the tileset
    /// and the tile-block view pointer. The save stays loadable (all
    /// fields are inside the main checksummed region and the checksum is
    /// recomputed on serialize), but rendering may glitch until those
    /// are set to values coherent with `map_id`.
    pub fn warp_to(&mut self, map_id: u8, x: u8, y: u8) {
        let buf = self.buf_mut();
        buf[offsets::CUR_MAP] = map_id;
        buf[offsets::X_COORD] = x;
        buf[offsets::Y_COORD] = y;
        buf[offsets::X_BLOCK_COORD] = x & 1;
        buf[offsets::Y_BLOCK_COORD] = y & 1;
    }
}
