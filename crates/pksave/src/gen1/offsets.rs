//! Byte offsets into a Gen 1 save file (raw 32 KiB SRAM dump).
//!
//! Every constant records the pret/pokered label it derives from. Mapping
//! rules (see docs/FORMAT.md):
//! - SRAM label: `bank * 0x2000 + (addr - 0xA000)`
//! - main-block WRAM label: `MAIN_DATA + (wLabel - 0xD2F7)`
//! - party WRAM label: `PARTY + (wLabel - 0xD163)`
//!
//! Verified against `pokered.sym` (pokered @ 1e96034) by
//! `cargo xtask gen-offsets-check`.

/// Size of the SRAM image; files may be longer (padding/RTC) but never shorter.
pub const SRAM_SIZE: usize = 0x8000;
/// Size of one SRAM bank (the MBC pages 8 KiB at a time).
pub const BANK_SIZE: usize = 0x2000;

// ---- Bank 0 ----
/// `sHallOfFame` (00:A598). 50 teams × 96 bytes.
pub const HALL_OF_FAME: usize = 0x0598;
/// Maximum Hall of Fame teams stored (`HOF_TEAM_CAPACITY` in pokered).
pub const HOF_TEAM_CAPACITY: usize = 50;
/// Bytes per Hall of Fame team: 6 mons × 16 bytes.
pub const HOF_TEAM_SIZE: usize = 96;
/// Bytes per Hall of Fame mon record (`HOF_MON` struct in pokered).
pub const HOF_MON_SIZE: usize = 16;

// ---- Bank 1: sGameData (checksummed region) ----
/// `sPlayerName` (01:A598), 11 bytes.
pub const PLAYER_NAME: usize = 0x2598;
/// `sMainData` == `wPokedexOwned` $D2F7. Anchor for all main-block fields.
pub const MAIN_DATA: usize = 0x25A3;
/// First byte covered by the main checksum (== PLAYER_NAME).
pub const CHECKSUM_REGION_START: usize = 0x2598;
/// Last byte covered by the main checksum (`sTileAnimations`, 01:B522).
pub const CHECKSUM_REGION_END: usize = 0x3522;
/// `sMainDataCheckSum` (01:B523).
pub const MAIN_CHECKSUM: usize = 0x3523;

/// `wPokedexOwned` $D2F7, 19 bytes (151 bits, bit 0 = Bulbasaur).
pub const POKEDEX_OWNED: usize = 0x25A3;
/// `wPokedexSeen` $D30A, 19 bytes.
pub const POKEDEX_SEEN: usize = 0x25B6;
/// Bytes per Pokédex bitfield: ceil(151 species / 8).
pub const POKEDEX_LEN: usize = 19;
/// `wNumBagItems` $D31D.
pub const BAG_ITEM_COUNT: usize = 0x25C9;
/// `wBagItems` $D31E, 20 × [id, qty] + 0xFF.
pub const BAG_ITEMS: usize = 0x25CA;
/// Maximum bag item slots (`BAG_ITEM_CAPACITY` in pokered).
pub const BAG_CAPACITY: usize = 20;
/// `wPlayerMoney` $D347, 3 bytes big-endian BCD.
pub const MONEY: usize = 0x25F3;
/// `wRivalName` $D34A, 11 bytes.
pub const RIVAL_NAME: usize = 0x25F6;
/// `wOptions` $D355.
pub const OPTIONS: usize = 0x2601;
/// `wObtainedBadges` $D356, bit 0 = Boulder.
pub const BADGES: usize = 0x2602;
/// `wLetterPrintingDelayFlags` $D358.
pub const LETTER_DELAY: usize = 0x2604;
/// `wPlayerID` $D359, 2 bytes big-endian.
pub const PLAYER_ID: usize = 0x2605;
/// `wCurMap` $D35E.
pub const CUR_MAP: usize = 0x260A;
/// `wCurrentTileBlockMapViewPointer` $D35F, 2 bytes (LE WRAM pointer).
pub const MAP_VIEW_POINTER: usize = 0x260B;
/// `wYCoord` $D361.
pub const Y_COORD: usize = 0x260D;
/// `wXCoord` $D362.
pub const X_COORD: usize = 0x260E;
/// `wYBlockCoord` $D363.
pub const Y_BLOCK_COORD: usize = 0x260F;
/// `wXBlockCoord` $D364.
pub const X_BLOCK_COORD: usize = 0x2610;
/// `wLastMap` $D365.
pub const LAST_MAP: usize = 0x2611;
/// `wCurMapTileset` $D367.
pub const CUR_MAP_TILESET: usize = 0x2613;
/// Pikachu friendship (Yellow only; unused byte in R/B).
pub const PIKACHU_FRIENDSHIP: usize = 0x271C;
/// `wNumBoxItems` $D53A.
pub const PC_ITEM_COUNT: usize = 0x27E6;
/// `wBoxItems` $D53B, 50 × [id, qty] + 0xFF.
pub const PC_ITEMS: usize = 0x27E7;
/// Maximum PC item slots (`PC_ITEM_CAPACITY` in pokered).
pub const PC_ITEM_CAPACITY: usize = 50;
/// `wCurrentBoxNum` $D5A0: bits 0-6 current box (0-11), bit 7 = boxes initialized.
pub const CURRENT_BOX_NUM: usize = 0x284C;
/// `wNumHoFTeams` $D5A2.
pub const HOF_TEAM_COUNT: usize = 0x284E;
/// `wPlayerCoins` $D5A4, 2 bytes big-endian BCD.
pub const COINS: usize = 0x2850;
/// `wToggleableObjectFlags` $D5A6, 32 bytes (missable overworld objects).
pub const MISSABLE_FLAGS: usize = 0x2852;
/// Length in bytes of `wToggleableObjectFlags`.
pub const MISSABLE_FLAGS_LEN: usize = 32;
/// `wGameProgressFlags` $D5F0, 0xC8 bytes.
pub const GAME_PROGRESS_FLAGS: usize = 0x289C;
/// Length in bytes of `wGameProgressFlags`.
pub const GAME_PROGRESS_FLAGS_LEN: usize = 0xC8;
/// `wObtainedHiddenItemsFlags` $D6F0, 14 bytes.
pub const HIDDEN_ITEM_FLAGS: usize = 0x299C;
/// Length in bytes of `wObtainedHiddenItemsFlags`.
pub const HIDDEN_ITEM_FLAGS_LEN: usize = 14;
/// `wObtainedHiddenCoinsFlags` $D6FE, 2 bytes.
pub const HIDDEN_COIN_FLAGS: usize = 0x29AA;
/// `wTownVisitedFlag` $D70B, 2 bytes (fly-unlocked towns, bit 0 = Pallet).
pub const TOWN_VISITED_FLAGS: usize = 0x29B7;
/// `wSafariSteps` $D70D, 2 bytes.
pub const SAFARI_STEPS: usize = 0x29B9;
/// `wRivalStarter` $D715 (internal species index).
pub const RIVAL_STARTER: usize = 0x29C1;
/// `wPlayerStarter` $D717 (internal species index).
pub const PLAYER_STARTER: usize = 0x29C3;
/// `wEventFlags` $D747, 320 bytes (`flag_array NUM_EVENTS`, NUM_EVENTS = $A00 =
/// 2560 bits; 507 named events allocated sparsely per map, last used bit 2522;
/// includes trainer-battled flags). Names: `data::generated::events`.
pub const EVENT_FLAGS: usize = 0x29F3;
/// Length in bytes of `wEventFlags` (NUM_EVENTS / 8).
pub const EVENT_FLAGS_LEN: usize = 320;
/// Total event flag bits reserved (`NUM_EVENTS` = $A00 in pokered).
pub const NUM_EVENTS: usize = 2560;
/// `wPlayTimeHours` $DA41.
pub const PLAY_TIME_HOURS: usize = 0x2CED;
/// `wPlayTimeMaxed` $DA42.
pub const PLAY_TIME_MAXED: usize = 0x2CEE;
/// `wPlayTimeMinutes` $DA43.
pub const PLAY_TIME_MINUTES: usize = 0x2CEF;
/// `wPlayTimeSeconds` $DA44.
pub const PLAY_TIME_SECONDS: usize = 0x2CF0;
/// `wPlayTimeFrames` $DA45.
pub const PLAY_TIME_FRAMES: usize = 0x2CF1;
/// `wDayCareInUse` $DA48.
pub const DAYCARE_IN_USE: usize = 0x2CF4;
/// `wDayCareMonName` $DA49, 11 bytes.
pub const DAYCARE_NICKNAME: usize = 0x2CF5;
/// `wDayCareMonOT` $DA54, 11 bytes.
pub const DAYCARE_OT: usize = 0x2D00;
/// `wDayCareMon` $DA5F, 33 bytes (box format).
pub const DAYCARE_MON: usize = 0x2D0B;
/// `sSpriteData` (01:AD2C), 0x200 bytes of overworld sprite state (opaque).
pub const SPRITE_DATA: usize = 0x2D2C;
/// `sPartyData` (01:AF2C) == `wPartyDataStart` $D163, 0x194 bytes.
pub const PARTY: usize = 0x2F2C;
/// Length in bytes of the party block (count + species list + 6×44 mons +
/// OT names + nicknames).
pub const PARTY_LEN: usize = 0x194;
/// `sCurBoxData` (01:B0C0): working copy of the current box, 0x462 bytes.
pub const CURRENT_BOX: usize = 0x30C0;

// ---- Banks 2/3: PC boxes ----
/// Number of PC boxes (`NUM_BOXES` in pokered): 6 in bank 2, 6 in bank 3.
pub const NUM_BOXES: usize = 12;
/// Maximum mons per PC box (`MONS_PER_BOX` in pokered).
pub const MONS_PER_BOX: usize = 20;
/// Maximum party size (`PARTY_LENGTH` in pokered).
pub const PARTY_CAPACITY: usize = 6;
/// Size of one box block.
pub const BOX_LEN: usize = 0x462;
/// `sBox1` (02:A000).
pub const BANK2_BOXES: usize = 0x4000;
/// `sBox7` (03:A000).
pub const BANK3_BOXES: usize = 0x6000;
/// `sBank2AllBoxesChecksum` (02:BA4C); per-box checksums follow (6 bytes).
pub const BANK2_ALL_BOXES_CHECKSUM: usize = 0x5A4C;
/// `sBank3AllBoxesChecksum` (03:BA4C).
pub const BANK3_ALL_BOXES_CHECKSUM: usize = 0x7A4C;

// ---- Record sizes ----
/// Name fields: 10 characters + 0x50 terminator.
pub const NAME_LEN: usize = 11;
/// Bytes per party mon record (`wPartyMon1` struct: box data + level +
/// current stats).
pub const PARTY_MON_SIZE: usize = 44;
/// Bytes per box mon record (`wBoxMon1` struct, no computed stats).
pub const BOX_MON_SIZE: usize = 33;

/// File offset of box `n` (0-based, 0..12) as stored in its bank.
/// Note: the *current* box's live data is at [`CURRENT_BOX`] instead.
pub const fn box_offset(n: usize) -> usize {
    if n < 6 {
        BANK2_BOXES + n * BOX_LEN
    } else {
        BANK3_BOXES + (n - 6) * BOX_LEN
    }
}
