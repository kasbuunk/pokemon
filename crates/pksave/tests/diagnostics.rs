//! Integration tests for the diagnostic catalogue (`gen1::validate`)
//! and variant detection (`gen1::detect`).
//!
//! Pattern: start from a clean `new_empty` save, break one specific
//! thing through the raw bytes, and assert the exact diagnostic code and
//! span. Where a poke lands inside a checksummed region the checksums
//! are re-fixed first so only the targeted finding remains.

use pksave::gen1::checksum;
use pksave::gen1::data::{DEX_TO_INDEX, INDEX_TO_DEX, MAP_NAMES};
use pksave::gen1::detect::detect_variant;
use pksave::gen1::offsets;
use pksave::gen1::pokemon::PartyMonMut;
use pksave::gen1::save::{GameVariant, SaveFile};
use pksave::{Diagnostic, Severity};

/// A clean save whose bytes were mangled by `f`. `fix` re-fixes all
/// checksums after mangling (so checksum warnings don't drown the
/// finding under test).
fn broken(fix: bool, f: impl FnOnce(&mut Vec<u8>)) -> SaveFile {
    let mut bytes = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
    f(&mut bytes);
    if fix {
        checksum::fix_all(&mut bytes);
    }
    SaveFile::from_bytes(bytes).expect("length is valid")
}

/// The diagnostics of `save` that carry `code`.
fn with_code(save: &SaveFile, code: &str) -> Vec<Diagnostic> {
    save.diagnostics()
        .into_iter()
        .filter(|d| d.code == code)
        .collect()
}

/// Assert exactly one diagnostic with `code`, at `span`.
fn assert_single(save: &SaveFile, code: &str, span: core::ops::Range<usize>) -> Diagnostic {
    let found = with_code(save, code);
    assert_eq!(found.len(), 1, "expected exactly one {code}: {found:?}");
    assert_eq!(found[0].span, Some(span), "{code} span");
    found[0].clone()
}

/// An invalid internal species index (maps to no dex number).
fn invalid_species() -> u8 {
    (0..=255u8)
        .find(|&i| i != 0 && INDEX_TO_DEX[usize::from(i)] == 0)
        .expect("glitch indexes exist")
}

/// A coherent party with one valid mon, written into `bytes`.
fn install_party_mon(bytes: &mut [u8], dex: usize, level: u8) {
    let mut mon = [0u8; offsets::PARTY_MON_SIZE];
    let mut m = PartyMonMut::new(&mut mon);
    m.set_species(DEX_TO_INDEX[dex]);
    m.set_level_coherent(level);
    bytes[offsets::PARTY] = 1;
    bytes[offsets::PARTY + 1] = DEX_TO_INDEX[dex];
    bytes[offsets::PARTY + 2] = 0xFF;
    bytes[offsets::PARTY + 8..offsets::PARTY + 8 + offsets::PARTY_MON_SIZE].copy_from_slice(&mon);
    // OT name + nickname for slot 0: "RED" / "MON".
    let ot = [
        0x91, 0x84, 0x83, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50,
    ];
    let nick = [
        0x8C, 0x8E, 0x8D, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50, 0x50,
    ];
    bytes[offsets::PARTY + 0x110..offsets::PARTY + 0x110 + 11].copy_from_slice(&ot);
    bytes[offsets::PARTY + 0x152..offsets::PARTY + 0x152 + 11].copy_from_slice(&nick);
}

#[test]
fn new_empty_is_fully_clean() {
    for variant in [GameVariant::RedBlue, GameVariant::Yellow] {
        assert_eq!(
            SaveFile::new_empty(variant).diagnostics(),
            Vec::new(),
            "{variant:?}"
        );
    }
}

#[test]
fn corrupt_main_checksum_byte() {
    let save = broken(false, |b| b[offsets::MAIN_CHECKSUM] ^= 0x5A);
    let d = assert_single(
        &save,
        "W-CHECKSUM-MAIN",
        offsets::MAIN_CHECKSUM..offsets::MAIN_CHECKSUM + 1,
    );
    assert_eq!(d.severity, Severity::Warning);
    assert_eq!(save.diagnostics().len(), 1, "nothing else is wrong");
}

#[test]
fn invalid_party_species_via_raw_buf() {
    let bad = invalid_species();
    let save = broken(true, |b| {
        install_party_mon(b, 25, 42);
        b[offsets::PARTY + 8] = bad; // mon record species byte
        b[offsets::PARTY + 1] = bad; // keep the species list in sync
    });
    let d = assert_single(
        &save,
        "W-SPECIES-INVALID",
        offsets::PARTY + 8..offsets::PARTY + 9,
    );
    assert!(d.message.contains("party slot 0"));
}

#[test]
fn party_count_and_sentinel_mismatches() {
    let save = broken(true, |b| b[offsets::PARTY] = 7);
    assert_single(&save, "W-PARTY-COUNT", offsets::PARTY..offsets::PARTY + 1);

    // Count 0 but the byte after the last entry is not the sentinel.
    let save = broken(true, |b| b[offsets::PARTY + 1] = 0x00);
    assert_single(
        &save,
        "W-PARTY-SENTINEL",
        offsets::PARTY + 1..offsets::PARTY + 2,
    );
}

#[test]
fn level_above_100() {
    let save = broken(true, |b| {
        install_party_mon(b, 25, 42);
        b[offsets::PARTY + 8 + 0x21] = 101;
    });
    assert_single(
        &save,
        "W-LEVEL-RANGE",
        offsets::PARTY + 8 + 0x21..offsets::PARTY + 8 + 0x22,
    );
    // Level 100 is fine.
    let save = broken(true, |b| {
        install_party_mon(b, 25, 42);
        b[offsets::PARTY + 8 + 0x21] = 100;
    });
    assert!(with_code(&save, "W-LEVEL-RANGE").is_empty());
}

#[test]
fn box_count_and_sentinel_mismatches() {
    let base = offsets::box_offset(7);
    let save = broken(true, |b| b[base] = 21);
    assert_single(&save, "W-BOX-COUNT", base..base + 1);

    let save = broken(true, |b| b[base + 1] = 0x12);
    assert_single(&save, "W-BOX-SENTINEL", base + 1..base + 2);

    // The current-box working copy is checked too.
    let save = broken(true, |b| b[offsets::CURRENT_BOX + 1] = 0x12);
    let d = assert_single(
        &save,
        "W-BOX-SENTINEL",
        offsets::CURRENT_BOX + 1..offsets::CURRENT_BOX + 2,
    );
    assert!(d.message.contains("current box"));
}

#[test]
fn money_with_invalid_bcd_nibble() {
    let save = broken(true, |b| b[offsets::MONEY] = 0xFA);
    assert_single(&save, "W-BCD-MONEY", offsets::MONEY..offsets::MONEY + 3);

    let save = broken(true, |b| b[offsets::COINS + 1] = 0x0B);
    assert_single(&save, "W-BCD-COINS", offsets::COINS..offsets::COINS + 2);
}

#[test]
fn unterminated_text_fields() {
    let save = broken(true, |b| {
        b[offsets::PLAYER_NAME..offsets::PLAYER_NAME + offsets::NAME_LEN].fill(0x80);
    });
    let d = assert_single(
        &save,
        "W-TEXT-UNTERMINATED",
        offsets::PLAYER_NAME..offsets::PLAYER_NAME + offsets::NAME_LEN,
    );
    assert!(d.message.contains("player name"));

    let save = broken(true, |b| {
        install_party_mon(b, 25, 42);
        b[offsets::PARTY + 0x152..offsets::PARTY + 0x152 + offsets::NAME_LEN].fill(0x81);
    });
    let d = assert_single(
        &save,
        "W-TEXT-UNTERMINATED",
        offsets::PARTY + 0x152..offsets::PARTY + 0x152 + offsets::NAME_LEN,
    );
    assert!(d.message.contains("nickname"));
}

#[test]
fn cleared_box_init_bit() {
    let save = broken(true, |b| b[offsets::CURRENT_BOX_NUM] &= 0x7F);
    let d = assert_single(
        &save,
        "W-BOX-INIT",
        offsets::CURRENT_BOX_NUM..offsets::CURRENT_BOX_NUM + 1,
    );
    assert_eq!(d.severity, Severity::Warning);
    assert!(
        d.message.contains("wipe"),
        "message must explain the game will wipe boxes: {}",
        d.message
    );
}

#[test]
fn desynced_current_box_copies() {
    // Current box is 0; poke its bank copy so it differs from 0x30C0.
    let bank = offsets::box_offset(0);
    let save = broken(true, |b| b[bank + 0x20] = 0x33);
    assert_single(&save, "W-BOX-STALE", bank..bank + offsets::BOX_LEN);

    // Editing the working copy through the API also desyncs...
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.box_mut(0)
        .add(&[1u8; offsets::BOX_MON_SIZE], "RED", "MON")
        .expect("room");
    assert_eq!(with_code(&save, "W-BOX-STALE").len(), 1);
    // ...until an explicit sync reconciles the copies.
    save.sync_current_box_to_bank();
    assert!(with_code(&save, "W-BOX-STALE").is_empty());
}

#[test]
fn daycare_with_invalid_species() {
    let bad = invalid_species();
    let save = broken(true, |b| {
        b[offsets::DAYCARE_IN_USE] = 1;
        b[offsets::DAYCARE_MON] = bad;
    });
    let d = assert_single(
        &save,
        "W-SPECIES-INVALID",
        offsets::DAYCARE_MON..offsets::DAYCARE_MON + 1,
    );
    assert!(d.message.contains("daycare"));

    // Not in use -> the stale species byte is ignored.
    let save = broken(true, |b| b[offsets::DAYCARE_MON] = bad);
    assert!(with_code(&save, "W-SPECIES-INVALID").is_empty());
}

#[test]
fn dex_bit_above_151() {
    let owned_last = offsets::POKEDEX_OWNED + offsets::POKEDEX_LEN - 1;
    let save = broken(true, |b| b[owned_last] |= 0x80);
    assert_single(&save, "W-DEX-RANGE", owned_last..owned_last + 1);

    let seen_last = offsets::POKEDEX_SEEN + offsets::POKEDEX_LEN - 1;
    let save = broken(true, |b| b[seen_last] |= 0x80);
    assert_single(&save, "W-DEX-RANGE", seen_last..seen_last + 1);
}

#[test]
fn unknown_current_map_id() {
    let unused = MAP_NAMES
        .iter()
        .position(|n| n.is_empty())
        .expect("unused map ids exist") as u8;
    let save = broken(true, |b| b[offsets::CUR_MAP] = unused);
    assert_single(
        &save,
        "W-MAP-UNKNOWN",
        offsets::CUR_MAP..offsets::CUR_MAP + 1,
    );
}

#[test]
fn oversize_file_is_an_info_note() {
    let mut bytes = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
    bytes.extend_from_slice(&[0xAB; 9]);
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    let d = assert_single(
        &save,
        "I-FILE-SIZE",
        offsets::SRAM_SIZE..offsets::SRAM_SIZE + 9,
    );
    assert_eq!(d.severity, Severity::Info);
}

#[test]
fn item_list_diagnostics_flow_through() {
    let save = broken(true, |b| {
        b[offsets::BAG_ITEM_COUNT] = 21; // over capacity
        b[offsets::PC_ITEMS] = 0x00; // PC terminator gone (count 0)
    });
    assert_eq!(with_code(&save, "W-ITEMS-COUNT").len(), 1);
    assert!(with_code(&save, "W-ITEMS-TERMINATOR")
        .iter()
        .any(|d| d.message.contains("item PC")));
}

#[test]
fn detect_variant_uses_pikachu_friendship() {
    assert_eq!(
        detect_variant(&SaveFile::new_empty(GameVariant::RedBlue)),
        GameVariant::RedBlue
    );
    assert_eq!(
        detect_variant(&SaveFile::new_empty(GameVariant::Yellow)),
        GameVariant::Yellow
    );
    let save = broken(true, |b| b[offsets::PIKACHU_FRIENDSHIP] = 1);
    assert_eq!(detect_variant(&save), GameVariant::Yellow);
}

#[test]
fn multi_broken_save_snapshot() {
    // One deliberately wrecked save exercising most of the catalogue at
    // once; the sorted rendering is snapshot-tested for stability.
    let bad = invalid_species();
    let save = broken(false, |b| {
        checksum::fix_all(b);
        b[offsets::MAIN_CHECKSUM] ^= 0xFF; // main checksum corrupt
        install_party_mon(b, 25, 42);
        b[offsets::PARTY + 8] = bad; // invalid species (record only)
        b[offsets::PARTY + 8 + 0x21] = 120; // level out of range
        b[offsets::MONEY + 1] = 0xC3; // two bad BCD nibbles
        b[offsets::CURRENT_BOX_NUM] = 0x00; // init bit clear, box 0
        b[offsets::box_offset(0) + 3] = 0x44; // desync bank copy of box 0
        b[offsets::box_offset(11) + 1] = 0x07; // box 12 sentinel gone
        b[offsets::POKEDEX_OWNED + 18] = 0x80; // dex bit 151
        b[offsets::CUR_MAP] = 0x0B; // UNUSED_MAP_0B... still named
        b[offsets::CUR_MAP] = 0xFF; // actually unused id
        b[offsets::RIVAL_NAME..offsets::RIVAL_NAME + offsets::NAME_LEN].fill(0x99);
        b[offsets::DAYCARE_IN_USE] = 1;
        b[offsets::DAYCARE_MON] = bad;
        b[offsets::BAG_ITEM_COUNT] = 1;
        b[offsets::BAG_ITEMS] = 0x00; // unknown item id 0x00, qty 0...
        b[offsets::BAG_ITEMS + 1] = 0x00;
        b[offsets::BAG_ITEMS + 2] = 0xFF; // ...with a proper terminator
    });

    let mut diags = save.diagnostics();
    diags.sort_by_key(|d| (d.span.clone().map(|s| (s.start, s.end)), d.code));
    let rendered: Vec<String> = diags
        .iter()
        .map(|d| {
            let span = match &d.span {
                Some(s) => format!("0x{:04X}..0x{:04X}", s.start, s.end),
                None => "-".to_string(),
            };
            format!("{:?} {} @ {} : {}", d.severity, d.code, span, d.message)
        })
        .collect();
    insta::assert_snapshot!(rendered.join("\n"));
}
