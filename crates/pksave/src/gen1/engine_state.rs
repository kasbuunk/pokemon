//! The cached map-header / overworld engine state a bootable save must
//! carry, baked for the NEW GAME spawn (player's bedroom, `REDS_HOUSE_2F`).
//!
//! Why this exists: on CONTINUE the game trusts these WRAM fields *from the
//! save* instead of rebuilding them from ROM. `LoadSAV` (pokered
//! `engine/menus/save.asm`) sets `BIT_NO_PREVIOUS_MAP` (bit 7) in
//! `wCurMapTileset` right after restoring `sMainData`, which makes
//! `LoadMapHeader` (pokered `home/overworld.asm`) return early on the first
//! `EnterMap` — skipping the reload of the map header, connections, warps,
//! signs, sprite objects and tileset pointers. A save that leaves this block
//! zeroed passes every checksum and menu, but the overworld then runs the
//! map script through `wCurMapScriptPtr = $0000` and the audio engine
//! through music ROM bank 0, taking a wild jump into the classic `rst $38`
//! crash loop about one second after the overworld loads. (Root-caused by
//! byte-bisecting a crashing blank save against a genuine one in a PyBoy
//! harness; see `e2e/README.md`.)
//!
//! The values below are the full non-zero content of the engine-state
//! region `wMapMusicSoundID..wNumBoxItems` (WRAM `$D35B..$D53A`), captured
//! from a genuine save written immediately at the NEW GAME spawn by a
//! retail-identical pokered build (pret/pokered @ the SHA pinned in
//! `crates/xtask/src/pins.rs`). The capture is deterministic — two
//! independent scripted runs produced byte-identical blocks — and every
//! byte not listed here is zero in that genuine save, matching the
//! zero-filled buffer `new_empty` starts from.

use super::offsets;

/// First WRAM address of the `sMainData` region (`wMainDataStart`); the
/// save-file image of WRAM address `a` is `MAIN_DATA + (a - WRAM_MAIN_DATA)`.
const WRAM_MAIN_DATA: usize = 0xD2F7;

/// Non-zero engine-state bytes of a genuine fresh save at the NEW GAME
/// spawn, as `(WRAM address, bytes)`. Everything else in
/// `$D35B..$D53A` is zero.
const SPAWN_STATE: &[(usize, &[u8])] = &[
    // wMapMusicSoundID, wMapMusicROMBank: MUSIC_PALLET_TOWN ($BA), played
    // by the audio engine in ROM bank 2 (Music_PalletTown lives at 02:422e).
    // Bank 0 here is what sends the sound engine off the rails.
    (0xD35B, &[0xBA, 0x02]),
    // wCurMap: REDS_HOUSE_2F ($26) — where a fresh game starts.
    (0xD35E, &[0x26]),
    // wCurrentTileBlockMapViewPointer: $C712 (little-endian), the visible
    // screen's top-left block inside the wOverworldMap buffer for the
    // spawn position. LoadCurrentMapView draws from this pointer verbatim.
    (0xD35F, &[0x12, 0xC7]),
    // wYCoord, wXCoord, wYBlockCoord, wXBlockCoord: the spawn tile (3, 6)
    // and the in-block parity the game derives as x&1 / y&1.
    (0xD361, &[0x06, 0x03, 0x00, 0x01]),
    // wCurMapTileset (REDS_HOUSE, without BIT_NO_PREVIOUS_MAP — LoadSAV
    // sets that bit itself on CONTINUE), wCurMapHeight, wCurMapWidth
    // (map size in 2x2-tile blocks).
    (0xD367, &[0x04, 0x04, 0x04]),
    // wCurMapDataPtr, wCurMapTextPtr, wCurMapScriptPtr (little-endian,
    // within the map's switchable ROM bank window): RedsHouse2F blocks,
    // text pointers and map script. The script pointer is called every
    // overworld frame; $0000 here is the direct crash vector.
    (0xD36A, &[0x10, 0x40, 0xCF, 0x40, 0xB0, 0x40]),
    // wNorth/South/West/EastConnectedMap: $FF = no connection on that
    // edge (0 would claim a connection to map 0 / Pallet Town).
    (0xD371, &[0xFF]),
    (0xD37C, &[0xFF]),
    (0xD387, &[0xFF]),
    (0xD392, &[0xFF]),
    // wObjectDataPointerTemp: $40D0, scratch left by the map loader;
    // copied verbatim from the genuine save.
    (0xD3A9, &[0xD0, 0x40]),
    // wMapBackgroundTile: $0A, the tile drawn past the map edge.
    (0xD3AD, &[0x0A]),
    // wNumberOfWarps = 1; warp 0 = (y 1, x 7) -> warp 2 of
    // REDS_HOUSE_1F ($25): the staircase down.
    (0xD3AE, &[0x01, 0x01, 0x07, 0x02, 0x25]),
    // wDestinationWarpID: $FF = none pending.
    (0xD42F, &[0xFF]),
    // wCurrentMapHeight2/Width2 (size in tiles, 8x8) and
    // wMapViewVRAMPointer ($9800 = vBGMap0, little-endian).
    (0xD524, &[0x08, 0x08, 0x00, 0x98]),
    // wPlayerLastStopDirection: facing the camera, as captured.
    (0xD529, &[0x08]),
    // wTilesetBank ($19) and wTilesetBlocksPtr/GfxPtr/CollisionPtr
    // ($5270/$4DE0/$1749, little-endian) for the REDS_HOUSE tileset;
    // LoadCurrentMapView switches to wTilesetBank and reads blocks
    // through these pointers.
    (0xD52B, &[0x19, 0x70, 0x52, 0xE0, 0x4D, 0x49, 0x17]),
    // wTilesetTalkingOverTiles: $FF terminator (no counter tiles).
    (0xD532, &[0xFF, 0xFF, 0xFF]),
    // wGrassTile: $FF = this tileset has no grass tile.
    (0xD535, &[0xFF]),
];

/// Write the spawn engine state into a raw (zero-filled) save buffer.
/// Used by [`super::save::SaveFile::new_empty`]; does not touch checksums.
pub(crate) fn write_spawn_state(raw: &mut [u8]) {
    for &(wram, bytes) in SPAWN_STATE {
        let at = offsets::MAIN_DATA + (wram - WRAM_MAIN_DATA);
        raw[at..at + bytes.len()].copy_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spawn_state_stays_inside_the_engine_state_region() {
        // wMapMusicSoundID ($D35B) .. wNumBoxItems ($D53A): the region the
        // continue path trusts from the save. PC items start right after.
        for &(wram, bytes) in SPAWN_STATE {
            assert!(wram >= 0xD35B, "entry below the region: {wram:#06X}");
            assert!(
                wram + bytes.len() <= 0xD53A,
                "entry crosses into PC items: {wram:#06X}"
            );
        }
        // The region maps to file offsets [0x2607, 0x27E6): directly before
        // PC_ITEM_COUNT.
        assert_eq!(
            offsets::MAIN_DATA + (0xD53A - WRAM_MAIN_DATA),
            offsets::PC_ITEM_COUNT
        );
    }

    #[test]
    fn spawn_state_matches_the_map_field_offsets() {
        // Cross-check the WRAM-address encoding against the offsets module
        // for the fields it also names.
        let at = |wram: usize| offsets::MAIN_DATA + (wram - WRAM_MAIN_DATA);
        assert_eq!(at(0xD35E), offsets::CUR_MAP);
        assert_eq!(at(0xD35F), offsets::MAP_VIEW_POINTER);
        assert_eq!(at(0xD361), offsets::Y_COORD);
        assert_eq!(at(0xD362), offsets::X_COORD);
        assert_eq!(at(0xD365), offsets::LAST_MAP);
        assert_eq!(at(0xD367), offsets::CUR_MAP_TILESET);
    }

    #[test]
    fn spawn_state_entries_do_not_overlap_and_are_sorted() {
        let mut prev_end = 0usize;
        for &(wram, bytes) in SPAWN_STATE {
            assert!(wram >= prev_end, "unsorted/overlapping at {wram:#06X}");
            prev_end = wram + bytes.len();
        }
    }
}
