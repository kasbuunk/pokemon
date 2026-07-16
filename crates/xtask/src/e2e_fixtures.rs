//! `cargo xtask make-e2e-fixtures --out <dir>`: write save files built with
//! the `pksave` core crate, plus a `fixtures.json` manifest, for the PyBoy
//! end-to-end suite in `e2e/`.
//!
//! Each manifest entry carries:
//! - `file` / `description`,
//! - `expected`: human-readable values (names, money, party levels, …) the
//!   Python tests can assert against labeled WRAM fields,
//! - `expected_wram`: raw byte expectations as `{label, offset, bytes}`
//!   triples, where `label` is a pokered WRAM symbol and `bytes` is hex.
//!   These are read *back* from the serialized save, so the manifest is the
//!   generator's ground truth and the Python side needs no Gen 1 codecs
//!   (charset, BCD) of its own.
//!
//! The mapping save-offset -> WRAM label relies on the game's `LoadSAV`,
//! which copies `sPlayerName`/`sMainData`/`sPartyData` verbatim into WRAM on
//! CONTINUE (see `gen1/offsets.rs` and `docs/FORMAT.md`).

use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use pksave::gen1::data::{BASE_STATS, DEX_TO_INDEX, MOVES, SPECIES_NAMES};
use pksave::gen1::offsets;
use pksave::gen1::pokemon::{MonMut, MonView, PartyMon, PartyMonMut};
use pksave::gen1::save::{GameVariant, SaveFile};
use pksave::gen1::stats::Dvs;
use pksave::gen1::trainer::Badge;

pub fn run(out: &Path) {
    fs::create_dir_all(out).unwrap_or_else(|e| panic!("create {}: {e}", out.display()));

    let fixtures = [
        (
            "baseline.sav",
            "SaveFile::new_empty(RedBlue) exactly as constructed",
            baseline(),
        ),
        (
            "renamed.sav",
            "player renamed to ASH, rival to GARY",
            renamed(),
        ),
        (
            "rich.sav",
            "money 999999 and coins 9999 (both BCD maximums)",
            rich(),
        ),
        ("badges.sav", "all 8 badges obtained", badges()),
        (
            "party.sav",
            "party of 3 legal mons: PIKACHU L25, MEWTWO L70, BULBASAUR L5",
            party(),
        ),
        (
            "pokedex.sav",
            "complete_dex(): all 151 entries owned and seen",
            pokedex(),
        ),
        (
            "boxmon.sav",
            "current box holds PIKACHU with level/exp coherent at 50 and \
             CHARMANDER with a deliberately stale level byte (80, exp for 50)",
            boxmon(),
        ),
    ];

    let mut entries = Vec::new();
    for (file, description, save) in &fixtures {
        let bytes = save.to_bytes();
        let path = out.join(file);
        fs::write(&path, &bytes).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
        entries.push(manifest_entry(file, description, save, &bytes));
        println!("wrote {}", path.display());
    }

    let manifest = format!(
        "{{\n  \"generated_by\": \"cargo xtask make-e2e-fixtures\",\n  \"fixtures\": [\n{}\n  ]\n}}\n",
        entries.join(",\n")
    );
    let manifest_path = out.join("fixtures.json");
    fs::write(&manifest_path, manifest)
        .unwrap_or_else(|e| panic!("write {}: {e}", manifest_path.display()));
    println!("wrote {}", manifest_path.display());
}

// ---------------------------------------------------------------------------
// Fixture construction
// ---------------------------------------------------------------------------

fn baseline() -> SaveFile {
    SaveFile::new_empty(GameVariant::RedBlue)
}

fn renamed() -> SaveFile {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_player_name("ASH").expect("name encodes");
    save.set_rival_name("GARY").expect("name encodes");
    save
}

fn rich() -> SaveFile {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_money(999_999).expect("max money encodes");
    save.set_coins(9_999).expect("max coins encodes");
    save
}

fn badges() -> SaveFile {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    for badge in Badge::ALL {
        save.set_badge(badge, true);
    }
    save
}

fn party() -> SaveFile {
    const TRAINER_ID: u16 = 12345;
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_player_id(TRAINER_ID);
    let player = save.player_name();

    let team: [(u8, u8, &[&str]); 3] = [
        (
            25, // PIKACHU
            25,
            &["THUNDERSHOCK", "GROWL", "THUNDER WAVE", "QUICK ATTACK"],
        ),
        (150, 70, &["PSYCHIC", "SWIFT", "BARRIER", "RECOVER"]), // MEWTWO
        (1, 5, &["TACKLE", "GROWL"]),                           // BULBASAUR
    ];

    for (dex, level, moves) in team {
        let record = make_mon(dex, level, moves, TRAINER_ID);
        save.party_mut()
            .add(&record, &player, SPECIES_NAMES[usize::from(dex)])
            .expect("party has room");
        save.set_dex_owned(dex, true);
        save.set_dex_seen(dex, true);
    }
    save
}

fn pokedex() -> SaveFile {
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.complete_dex();
    save
}

/// Two mons in the current box (the 0x30C0 working copy `LoadSAV`
/// copies into WRAM): one fully coherent, one with a stale level byte —
/// the game must load both byte-identically (a box mon's level derives
/// from exp only on *withdrawal*, never on load).
fn boxmon() -> SaveFile {
    const TRAINER_ID: u16 = 12345;
    let mut save = SaveFile::new_empty(GameVariant::RedBlue);
    save.set_player_id(TRAINER_ID);
    let player = save.player_name();

    for (dex, level, moves) in [
        (25u8, 50u8, &["THUNDERSHOCK", "GROWL"][..]), // PIKACHU
        (4, 50, &["SCRATCH", "GROWL"][..]),           // CHARMANDER
    ] {
        let record = make_mon(dex, level, moves, TRAINER_ID);
        let boxed = pksave::gen1::pokemon::party_to_box(&record);
        save.box_mut(0)
            .add(&boxed, &player, SPECIES_NAMES[usize::from(dex)])
            .expect("box has room");
        save.set_dex_owned(dex, true);
        save.set_dex_seen(dex, true);
    }
    // Slot 1: stale level byte (exp still says 50) — the regression
    // shape behind the level-drop bug.
    save.box_mut(0).mon_mut(1).set_box_level(80);
    save.sync_current_box_to_bank();
    save
}

/// Craft a legal 44-byte party record the way the game itself would:
/// species/types/catch rate from base stats, real move ids with full PP,
/// perfect DVs, zero stat exp, and exp/stats/current HP made coherent with
/// the level via `set_level_coherent`.
fn make_mon(dex: u8, level: u8, move_names: &[&str], ot_id: u16) -> [u8; offsets::PARTY_MON_SIZE] {
    assert!(move_names.len() <= 4, "at most 4 moves");
    let base = BASE_STATS[usize::from(dex)];
    let mut record = [0u8; offsets::PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut record);

    mon.set_species(DEX_TO_INDEX[usize::from(dex)]);
    mon.set_types(base.type1, base.type2);
    mon.set_catch_rate(base.catch_rate);

    let mut ids = [0u8; 4];
    let mut pp = [0u8; 4];
    for (slot, name) in move_names.iter().enumerate() {
        let id = move_id(name);
        ids[slot] = id;
        pp[slot] = MOVES[usize::from(id)].pp;
    }
    mon.set_moves(ids);
    mon.set_pp(pp);

    mon.set_ot_id(ot_id);
    mon.set_dvs(Dvs {
        attack: 15,
        defense: 15,
        speed: 15,
        special: 15,
    });
    // Stat exp stays 0 (zero-initialized). Level last: it derives exp,
    // the five calculated stats, and current HP = max HP.
    mon.set_level_coherent(level);
    record
}

/// Move id (index into `MOVES`) by exact table name.
fn move_id(name: &str) -> u8 {
    let id = MOVES
        .iter()
        .position(|m| m.name == name)
        .unwrap_or_else(|| panic!("no move named {name:?}"));
    u8::try_from(id).expect("move table has < 256 entries")
}

// ---------------------------------------------------------------------------
// Manifest emission
// ---------------------------------------------------------------------------

/// One `{label, offset, bytes}` WRAM expectation.
struct WramExpect {
    label: &'static str,
    offset: usize,
    bytes: Vec<u8>,
}

/// The WRAM contents `LoadSAV` should produce from `bytes`, read back from
/// the serialized save itself.
fn wram_expectations(save: &SaveFile, bytes: &[u8]) -> Vec<WramExpect> {
    let grab = |label, save_offset: usize, offset: usize, len: usize| WramExpect {
        label,
        offset,
        bytes: bytes[save_offset..save_offset + len].to_vec(),
    };

    let mut out = vec![
        grab("wPlayerName", offsets::PLAYER_NAME, 0, offsets::NAME_LEN),
        grab("wRivalName", offsets::RIVAL_NAME, 0, offsets::NAME_LEN),
        grab("wPlayerMoney", offsets::MONEY, 0, 3),
        grab("wPlayerCoins", offsets::COINS, 0, 2),
        grab("wObtainedBadges", offsets::BADGES, 0, 1),
        grab("wPlayerID", offsets::PLAYER_ID, 0, 2),
        grab("wCurMap", offsets::CUR_MAP, 0, 1),
        grab(
            "wPokedexOwned",
            offsets::POKEDEX_OWNED,
            0,
            offsets::POKEDEX_LEN,
        ),
        grab(
            "wPokedexSeen",
            offsets::POKEDEX_SEEN,
            0,
            offsets::POKEDEX_LEN,
        ),
        // Count byte plus the species list including its 0xFF terminator.
        grab("wPartyCount", offsets::PARTY, 0, 1),
        grab(
            "wPartySpecies",
            offsets::PARTY + 1,
            0,
            save.party().len() + 1,
        ),
    ];

    // Per-mon: the full 44-byte record and its OT name / nickname. The mon
    // records sit at party block +0x008 (wPartyMons), OT names at +0x110
    // (wPartyMonOT), nicknames at +0x152 (wPartyMonNicks); see party.rs.
    for i in 0..save.party().len() {
        out.push(grab(
            "wPartyMons",
            offsets::PARTY + 0x008 + i * offsets::PARTY_MON_SIZE,
            i * offsets::PARTY_MON_SIZE,
            offsets::PARTY_MON_SIZE,
        ));
        out.push(grab(
            "wPartyMonOT",
            offsets::PARTY + 0x110 + i * offsets::NAME_LEN,
            i * offsets::NAME_LEN,
            offsets::NAME_LEN,
        ));
        out.push(grab(
            "wPartyMonNicks",
            offsets::PARTY + 0x152 + i * offsets::NAME_LEN,
            i * offsets::NAME_LEN,
            offsets::NAME_LEN,
        ));
    }

    // Current-box working copy: LoadSAV copies sCurBoxData (0x30C0)
    // verbatim into the WRAM box block starting at wBoxCount (the first
    // byte of wBoxDataStart in pokered). Offsets within the block:
    // species list +0x001, mon records +0x016 (33 bytes each), OT names
    // +0x2AA, nicknames +0x386 (docs/FORMAT.md).
    let current_box = save.box_(usize::from(save.current_box_number()).min(11));
    let box_len = current_box.len();
    if box_len > 0 {
        out.push(grab("wBoxCount", offsets::CURRENT_BOX, 0, 1));
        out.push(grab(
            "wBoxCount",
            offsets::CURRENT_BOX + 0x001,
            0x001,
            box_len + 1,
        ));
        for i in 0..box_len {
            out.push(grab(
                "wBoxCount",
                offsets::CURRENT_BOX + 0x016 + i * offsets::BOX_MON_SIZE,
                0x016 + i * offsets::BOX_MON_SIZE,
                offsets::BOX_MON_SIZE,
            ));
            out.push(grab(
                "wBoxCount",
                offsets::CURRENT_BOX + 0x2AA + i * offsets::NAME_LEN,
                0x2AA + i * offsets::NAME_LEN,
                offsets::NAME_LEN,
            ));
            out.push(grab(
                "wBoxCount",
                offsets::CURRENT_BOX + 0x386 + i * offsets::NAME_LEN,
                0x386 + i * offsets::NAME_LEN,
                offsets::NAME_LEN,
            ));
        }
    }
    out
}

fn manifest_entry(file: &str, description: &str, save: &SaveFile, bytes: &[u8]) -> String {
    let party = save.party();
    let species: Vec<u8> = party.species_list().to_vec();
    let dex: Vec<u8> = species
        .iter()
        .map(|&s| pksave::gen1::data::INDEX_TO_DEX[usize::from(s)])
        .collect();
    let levels: Vec<u8> = (0..party.len()).map(|i| party.mon(i).level()).collect();
    let nicknames: Vec<String> = (0..party.len()).map(|i| party.nickname(i)).collect();
    let first_mon = (!party.is_empty()).then(|| party.mon(0));

    let mut expected = String::new();
    let e = &mut expected;
    push_kv(e, "player_name", &json_string(&save.player_name()));
    push_kv(e, "rival_name", &json_string(&save.rival_name()));
    push_kv(e, "money", &save.money().expect("valid BCD").to_string());
    push_kv(e, "coins", &save.coins().expect("valid BCD").to_string());
    push_kv(e, "badges_byte", &save.badges().to_string());
    push_kv(e, "player_id", &save.player_id().to_string());
    push_kv(e, "party_count", &party.len().to_string());
    push_kv(e, "party_species_internal", &json_numbers(&species));
    push_kv(e, "party_dex", &json_numbers(&dex));
    push_kv(e, "party_levels", &json_numbers(&levels));
    push_kv(e, "party_nicknames", &json_strings(&nicknames));
    push_kv(e, "owned_count", &save.owned_count().to_string());
    push_kv(e, "seen_count", &save.seen_count().to_string());
    if let Some(mon) = first_mon {
        push_kv(e, "first_mon_current_hp", &mon.current_hp().to_string());
        push_kv(e, "first_mon_max_hp", &mon.max_hp().to_string());
        push_kv(e, "first_mon_attack", &mon.attack().to_string());
        push_kv(e, "first_mon_defense", &mon.defense().to_string());
        push_kv(e, "first_mon_speed", &mon.speed().to_string());
        push_kv(e, "first_mon_special", &mon.special().to_string());
    }

    let wram: Vec<String> = wram_expectations(save, bytes)
        .iter()
        .map(|w| {
            format!(
                "        {{ \"label\": {}, \"offset\": {}, \"bytes\": {} }}",
                json_string(w.label),
                w.offset,
                json_string(&hex(&w.bytes))
            )
        })
        .collect();

    format!
    (
        "    {{\n      \"file\": {},\n      \"description\": {},\n      \"expected\": {{\n{}\n      }},\n      \"expected_wram\": [\n{}\n      ]\n    }}",
        json_string(file),
        json_string(description),
        expected,
        wram.join(",\n")
    )
}

/// Append one `"key": value` line to an `expected` object body.
fn push_kv(out: &mut String, key: &str, value: &str) {
    if !out.is_empty() {
        out.push_str(",\n");
    }
    write!(out, "        \"{key}\": {value}").expect("write to String");
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => {
                write!(out, "\\u{:04x}", c as u32).expect("write to String");
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn json_strings(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| json_string(s)).collect();
    format!("[{}]", inner.join(", "))
}

fn json_numbers(items: &[u8]) -> String {
    let inner: Vec<String> = items.iter().map(u8::to_string).collect();
    format!("[{}]", inner.join(", "))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(out, "{b:02x}").expect("write to String");
    }
    out
}
