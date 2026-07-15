//! `cargo xtask gen-offsets-check --sym <pokered.sym>`: verify the constants
//! in `pksave::gen1::offsets` against the label addresses in a pokered
//! symbol file.
//!
//! Mapping rules (docs/FORMAT.md):
//! - SRAM label (banks 0-3, $A000-$BFFF): `bank * 0x2000 + (addr - 0xA000)`
//! - main-block WRAM label ($D2F7-$DA7F): `0x25A3 + (addr - 0xD2F7)`
//! - party WRAM label ($D163-$D2F6): `0x2F2C + (addr - 0xD163)`

use std::collections::HashMap;
use std::path::Path;

use pksave::gen1::offsets as o;

#[derive(Clone, Copy)]
enum Rule {
    /// SRAM label -> file offset.
    Sram,
    /// WRAM label inside the `sMainData` copy.
    MainWram,
    /// WRAM label inside the `sPartyData` copy.
    PartyWram,
    /// Raw WRAM address (sanity-checks the mapping anchors themselves).
    WramAddr,
}

const MAIN_WRAM_START: u32 = 0xD2F7;
const MAIN_WRAM_END: u32 = 0xDA80;
const PARTY_WRAM_START: u32 = 0xD163;

/// Every labeled constant in `gen1/offsets.rs`, as
/// `(constant name, expected value, pokered label, mapping rule)`.
/// `PIKACHU_FRIENDSHIP` has no pokered label (unused byte in R/B) and cannot
/// be checked. Length constants are checked separately below.
const CHECKS: &[(&str, usize, &str, Rule)] = &[
    // Bank 0
    ("HALL_OF_FAME", o::HALL_OF_FAME, "sHallOfFame", Rule::Sram),
    // Bank 1 SRAM labels
    ("PLAYER_NAME", o::PLAYER_NAME, "sPlayerName", Rule::Sram),
    ("MAIN_DATA", o::MAIN_DATA, "sMainData", Rule::Sram),
    (
        "CHECKSUM_REGION_START",
        o::CHECKSUM_REGION_START,
        "sGameData",
        Rule::Sram,
    ),
    (
        "CHECKSUM_REGION_END",
        o::CHECKSUM_REGION_END,
        "sTileAnimations",
        Rule::Sram,
    ),
    (
        "MAIN_CHECKSUM",
        o::MAIN_CHECKSUM,
        "sMainDataCheckSum",
        Rule::Sram,
    ),
    ("SPRITE_DATA", o::SPRITE_DATA, "sSpriteData", Rule::Sram),
    ("PARTY", o::PARTY, "sPartyData", Rule::Sram),
    ("CURRENT_BOX", o::CURRENT_BOX, "sCurBoxData", Rule::Sram),
    // Main-block WRAM labels
    (
        "POKEDEX_OWNED",
        o::POKEDEX_OWNED,
        "wPokedexOwned",
        Rule::MainWram,
    ),
    (
        "POKEDEX_SEEN",
        o::POKEDEX_SEEN,
        "wPokedexSeen",
        Rule::MainWram,
    ),
    (
        "BAG_ITEM_COUNT",
        o::BAG_ITEM_COUNT,
        "wNumBagItems",
        Rule::MainWram,
    ),
    ("BAG_ITEMS", o::BAG_ITEMS, "wBagItems", Rule::MainWram),
    ("MONEY", o::MONEY, "wPlayerMoney", Rule::MainWram),
    ("RIVAL_NAME", o::RIVAL_NAME, "wRivalName", Rule::MainWram),
    ("OPTIONS", o::OPTIONS, "wOptions", Rule::MainWram),
    ("BADGES", o::BADGES, "wObtainedBadges", Rule::MainWram),
    (
        "LETTER_DELAY",
        o::LETTER_DELAY,
        "wLetterPrintingDelayFlags",
        Rule::MainWram,
    ),
    ("PLAYER_ID", o::PLAYER_ID, "wPlayerID", Rule::MainWram),
    ("CUR_MAP", o::CUR_MAP, "wCurMap", Rule::MainWram),
    (
        "MAP_VIEW_POINTER",
        o::MAP_VIEW_POINTER,
        "wCurrentTileBlockMapViewPointer",
        Rule::MainWram,
    ),
    ("Y_COORD", o::Y_COORD, "wYCoord", Rule::MainWram),
    ("X_COORD", o::X_COORD, "wXCoord", Rule::MainWram),
    (
        "Y_BLOCK_COORD",
        o::Y_BLOCK_COORD,
        "wYBlockCoord",
        Rule::MainWram,
    ),
    (
        "X_BLOCK_COORD",
        o::X_BLOCK_COORD,
        "wXBlockCoord",
        Rule::MainWram,
    ),
    ("LAST_MAP", o::LAST_MAP, "wLastMap", Rule::MainWram),
    (
        "CUR_MAP_TILESET",
        o::CUR_MAP_TILESET,
        "wCurMapTileset",
        Rule::MainWram,
    ),
    (
        "PC_ITEM_COUNT",
        o::PC_ITEM_COUNT,
        "wNumBoxItems",
        Rule::MainWram,
    ),
    ("PC_ITEMS", o::PC_ITEMS, "wBoxItems", Rule::MainWram),
    (
        "CURRENT_BOX_NUM",
        o::CURRENT_BOX_NUM,
        "wCurrentBoxNum",
        Rule::MainWram,
    ),
    (
        "HOF_TEAM_COUNT",
        o::HOF_TEAM_COUNT,
        "wNumHoFTeams",
        Rule::MainWram,
    ),
    ("COINS", o::COINS, "wPlayerCoins", Rule::MainWram),
    (
        "MISSABLE_FLAGS",
        o::MISSABLE_FLAGS,
        "wToggleableObjectFlags",
        Rule::MainWram,
    ),
    (
        "GAME_PROGRESS_FLAGS",
        o::GAME_PROGRESS_FLAGS,
        "wGameProgressFlags",
        Rule::MainWram,
    ),
    (
        "HIDDEN_ITEM_FLAGS",
        o::HIDDEN_ITEM_FLAGS,
        "wObtainedHiddenItemsFlags",
        Rule::MainWram,
    ),
    (
        "HIDDEN_COIN_FLAGS",
        o::HIDDEN_COIN_FLAGS,
        "wObtainedHiddenCoinsFlags",
        Rule::MainWram,
    ),
    (
        "TOWN_VISITED_FLAGS",
        o::TOWN_VISITED_FLAGS,
        "wTownVisitedFlag",
        Rule::MainWram,
    ),
    (
        "SAFARI_STEPS",
        o::SAFARI_STEPS,
        "wSafariSteps",
        Rule::MainWram,
    ),
    (
        "RIVAL_STARTER",
        o::RIVAL_STARTER,
        "wRivalStarter",
        Rule::MainWram,
    ),
    (
        "PLAYER_STARTER",
        o::PLAYER_STARTER,
        "wPlayerStarter",
        Rule::MainWram,
    ),
    ("EVENT_FLAGS", o::EVENT_FLAGS, "wEventFlags", Rule::MainWram),
    (
        "PLAY_TIME_HOURS",
        o::PLAY_TIME_HOURS,
        "wPlayTimeHours",
        Rule::MainWram,
    ),
    (
        "PLAY_TIME_MAXED",
        o::PLAY_TIME_MAXED,
        "wPlayTimeMaxed",
        Rule::MainWram,
    ),
    (
        "PLAY_TIME_MINUTES",
        o::PLAY_TIME_MINUTES,
        "wPlayTimeMinutes",
        Rule::MainWram,
    ),
    (
        "PLAY_TIME_SECONDS",
        o::PLAY_TIME_SECONDS,
        "wPlayTimeSeconds",
        Rule::MainWram,
    ),
    (
        "PLAY_TIME_FRAMES",
        o::PLAY_TIME_FRAMES,
        "wPlayTimeFrames",
        Rule::MainWram,
    ),
    (
        "DAYCARE_IN_USE",
        o::DAYCARE_IN_USE,
        "wDayCareInUse",
        Rule::MainWram,
    ),
    (
        "DAYCARE_NICKNAME",
        o::DAYCARE_NICKNAME,
        "wDayCareMonName",
        Rule::MainWram,
    ),
    ("DAYCARE_OT", o::DAYCARE_OT, "wDayCareMonOT", Rule::MainWram),
    ("DAYCARE_MON", o::DAYCARE_MON, "wDayCareMon", Rule::MainWram),
    // Party rule cross-check (same constant, independent derivation)
    ("PARTY", o::PARTY, "wPartyDataStart", Rule::PartyWram),
    // Banks 2/3
    ("BANK2_BOXES", o::BANK2_BOXES, "sBox1", Rule::Sram),
    ("BANK3_BOXES", o::BANK3_BOXES, "sBox7", Rule::Sram),
    (
        "BANK2_ALL_BOXES_CHECKSUM",
        o::BANK2_ALL_BOXES_CHECKSUM,
        "sBank2AllBoxesChecksum",
        Rule::Sram,
    ),
    (
        "BANK3_ALL_BOXES_CHECKSUM",
        o::BANK3_ALL_BOXES_CHECKSUM,
        "sBank3AllBoxesChecksum",
        Rule::Sram,
    ),
    // Mapping anchors (raw WRAM addresses the rules above are built on)
    (
        "(main data anchor)",
        MAIN_WRAM_START as usize,
        "wMainDataStart",
        Rule::WramAddr,
    ),
    (
        "(main data end)",
        MAIN_WRAM_END as usize,
        "wMainDataEnd",
        Rule::WramAddr,
    ),
    (
        "(party anchor)",
        PARTY_WRAM_START as usize,
        "wPartyDataStart",
        Rule::WramAddr,
    ),
    (
        "(party end)",
        MAIN_WRAM_START as usize,
        "wPartyDataEnd",
        Rule::WramAddr,
    ),
];

pub fn run(sym_path: &Path) -> i32 {
    let symbols = parse_sym(sym_path);
    let mut failures = 0usize;

    let mut check = |name: &str, expected: usize, actual: Result<usize, String>| match actual {
        Ok(actual) if actual == expected => {
            println!("ok   {name}: {expected:#06X} == {actual:#06X}");
        }
        Ok(actual) => {
            eprintln!("FAIL {name}: offsets.rs has {expected:#06X}, sym file gives {actual:#06X}");
            failures += 1;
        }
        Err(e) => {
            eprintln!("FAIL {name}: {e}");
            failures += 1;
        }
    };

    for &(name, expected, label, rule) in CHECKS {
        let derived = symbols
            .get(label)
            .ok_or_else(|| format!("label {label} not found in sym file"))
            .and_then(|&(bank, addr)| derive(bank, addr, rule, label));
        check(&format!("{name} <- {label}"), expected, derived);
    }

    // Length constants derivable from label pairs.
    let span = |start: &str, end: &str| -> Result<usize, String> {
        let &(b1, a1) = symbols
            .get(start)
            .ok_or(format!("label {start} not found"))?;
        let &(b2, a2) = symbols.get(end).ok_or(format!("label {end} not found"))?;
        if b1 != b2 {
            return Err(format!("{start} and {end} are in different banks"));
        }
        Ok((a2 - a1) as usize)
    };
    check(
        "PARTY_LEN <- wPartyDataEnd - wPartyDataStart",
        o::PARTY_LEN,
        span("wPartyDataStart", "wPartyDataEnd"),
    );
    check(
        "POKEDEX_LEN <- wPokedexOwnedEnd - wPokedexOwned",
        o::POKEDEX_LEN,
        span("wPokedexOwned", "wPokedexOwnedEnd"),
    );

    if failures == 0 {
        println!("all offset checks passed");
        0
    } else {
        eprintln!("{failures} offset check(s) FAILED");
        1
    }
}

fn derive(bank: u32, addr: u32, rule: Rule, label: &str) -> Result<usize, String> {
    match rule {
        Rule::Sram => {
            if bank > 3 || !(0xA000..0xC000).contains(&addr) {
                return Err(format!(
                    "{label} ({bank:02X}:{addr:04X}) is not an SRAM label"
                ));
            }
            Ok((bank as usize) * 0x2000 + (addr as usize - 0xA000))
        }
        Rule::MainWram => {
            if bank != 0 || !(MAIN_WRAM_START..MAIN_WRAM_END).contains(&addr) {
                return Err(format!(
                    "{label} ({bank:02X}:{addr:04X}) is outside the main data WRAM copy"
                ));
            }
            Ok(o::MAIN_DATA + (addr - MAIN_WRAM_START) as usize)
        }
        Rule::PartyWram => {
            if bank != 0 || !(PARTY_WRAM_START..MAIN_WRAM_START).contains(&addr) {
                return Err(format!(
                    "{label} ({bank:02X}:{addr:04X}) is outside the party WRAM copy"
                ));
            }
            Ok(o::PARTY + (addr - PARTY_WRAM_START) as usize)
        }
        Rule::WramAddr => {
            if bank != 0 {
                return Err(format!("{label} is not in WRAM bank 0"));
            }
            Ok(addr as usize)
        }
    }
}

/// Parse `bb:aaaa label` lines. Duplicate labels must agree on the address.
fn parse_sym(path: &Path) -> HashMap<String, (u32, u32)> {
    let text =
        std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let mut map: HashMap<String, (u32, u32)> = HashMap::new();
    for (lineno, line) in text.lines().enumerate() {
        let line = line.split(';').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        let ctx = || format!("{}:{}", path.display(), lineno + 1);
        let mut parts = line.split_whitespace();
        let (Some(loc), Some(label)) = (parts.next(), parts.next()) else {
            panic!("{}: malformed sym line {line:?}", ctx());
        };
        let Some((bank, addr)) = loc.split_once(':') else {
            // RGBDS also emits bare numeric constants (`01 SOME_CONSTANT`);
            // only banked `bank:addr label` symbols matter here.
            continue;
        };
        let bank = u32::from_str_radix(bank, 16)
            .unwrap_or_else(|_| panic!("{}: bad bank {bank:?}", ctx()));
        let addr = u32::from_str_radix(addr, 16)
            .unwrap_or_else(|_| panic!("{}: bad address {addr:?}", ctx()));
        if let Some(&prev) = map.get(label) {
            assert_eq!(
                prev,
                (bank, addr),
                "{}: label {label} redefined with a different address",
                ctx()
            );
        }
        map.insert(label.to_string(), (bank, addr));
    }
    assert!(!map.is_empty(), "{}: no symbols parsed", path.display());
    map
}
