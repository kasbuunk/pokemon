//! The full diagnostic catalogue for a Gen 1 save.
//!
//! [`diagnose`] inspects a [`SaveFile`] and reports every non-fatal
//! finding: checksum mismatches, incoherent counts/sentinels, invalid
//! species, out-of-range levels, item-list problems, invalid BCD,
//! unterminated text, box-initialization hazards and more. Philosophy
//! (see `lib.rs`): warnings never block — a corrupt save loads, edits
//! and serializes fine; diagnostics only *describe* it.
//!
//! Every diagnostic carries a stable machine-readable code, a
//! human-readable message and, where sensible, the byte span it
//! concerns. The order of the returned vector is deterministic (fixed
//! check order, ascending indexes within each check).

use super::boxes::{layout as box_layout, party_layout};
use super::data::{INDEX_TO_DEX, MAP_NAMES};
use super::items::{diagnose_item_list, BAG_LIST, PC_LIST};
use super::pokemon::{BoxMonView, MonView, PartyMon, PartyMonView};
use super::save::SaveFile;
use super::text::TERMINATOR;
use super::{bcd, checksum, offsets};
use crate::{Diagnostic, Severity};

/// Highest level the game can legitimately produce.
const MAX_LEVEL: u8 = 100;

fn warn(code: &'static str, message: String, span: core::ops::Range<usize>) -> Diagnostic {
    Diagnostic {
        severity: Severity::Warning,
        code,
        message,
        span: Some(span),
    }
}

/// Run every check and collect the findings. An untouched
/// [`SaveFile::new_empty`] save yields an empty vector.
pub fn diagnose(save: &SaveFile) -> Vec<Diagnostic> {
    let buf = save.as_bytes();
    let mut diags = Vec::new();

    file_size(buf, &mut diags);
    checksums(buf, &mut diags);
    pinned_checksums(save, &mut diags);
    party(buf, &mut diags);
    boxes(buf, &mut diags);
    level_exp_coherence(buf, &mut diags);
    diags.extend(diagnose_item_list(buf, &BAG_LIST));
    diags.extend(diagnose_item_list(buf, &PC_LIST));
    money_and_coins(buf, &mut diags);
    text_terminators(buf, &mut diags);
    box_flags(buf, &mut diags);
    daycare(buf, &mut diags);
    dex_range(buf, &mut diags);
    map(buf, &mut diags);
    diags
}

/// `I-FILE-SIZE`: informational note when the file is longer than the
/// bare 32 KiB SRAM image (emulator padding, RTC footer). The tail is
/// preserved verbatim; nothing is wrong.
fn file_size(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    if buf.len() != offsets::SRAM_SIZE {
        diags.push(Diagnostic {
            severity: Severity::Info,
            code: "I-FILE-SIZE",
            message: format!(
                "file is {} bytes, not the bare 0x8000 SRAM image; the extra bytes are preserved verbatim",
                buf.len()
            ),
            span: Some(offsets::SRAM_SIZE..buf.len()),
        });
    }
}

/// Human-readable label for a checksummed region, shared by the
/// `W-CHECKSUM-*` and `I-CHECKSUM-PINNED` messages.
fn region_label(region: checksum::Region) -> String {
    match region {
        checksum::Region::Main => "main data".to_string(),
        checksum::Region::Bank2AllBoxes => "bank 2 all-boxes".to_string(),
        checksum::Region::Bank3AllBoxes => "bank 3 all-boxes".to_string(),
        checksum::Region::Box(n) => format!("box {}", n + 1),
    }
}

/// `W-CHECKSUM-*`: one warning per stored checksum that disagrees with
/// the bytes it covers.
fn checksums(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    const BOX_CODES: [&str; offsets::NUM_BOXES] = [
        "W-CHECKSUM-BOX1",
        "W-CHECKSUM-BOX2",
        "W-CHECKSUM-BOX3",
        "W-CHECKSUM-BOX4",
        "W-CHECKSUM-BOX5",
        "W-CHECKSUM-BOX6",
        "W-CHECKSUM-BOX7",
        "W-CHECKSUM-BOX8",
        "W-CHECKSUM-BOX9",
        "W-CHECKSUM-BOX10",
        "W-CHECKSUM-BOX11",
        "W-CHECKSUM-BOX12",
    ];
    for m in checksum::verify(buf) {
        let code = match m.region {
            checksum::Region::Main => "W-CHECKSUM-MAIN",
            checksum::Region::Bank2AllBoxes => "W-CHECKSUM-BOXBANK2",
            checksum::Region::Bank3AllBoxes => "W-CHECKSUM-BOXBANK3",
            checksum::Region::Box(n) => BOX_CODES[n],
        };
        let at = m.region.checksum_offset();
        diags.push(warn(
            code,
            format!(
                "{} checksum mismatch: stored 0x{:02X}, computed 0x{:02X}",
                region_label(m.region),
                m.stored,
                m.computed
            ),
            at..at + 1,
        ));
    }
}

/// `I-CHECKSUM-PINNED`: informational note per region whose stored
/// checksum byte is pinned by an override
/// ([`SaveFile::set_checksum_override`] or a raw `set_byte` on the
/// checksum byte) and therefore kept verbatim by `to_bytes()`.
fn pinned_checksums(save: &SaveFile, diags: &mut Vec<Diagnostic>) {
    for region in checksum::Region::ALL {
        if save.checksum_override(region).is_some() {
            let at = region.checksum_offset();
            diags.push(Diagnostic {
                severity: Severity::Info,
                code: "I-CHECKSUM-PINNED",
                message: format!(
                    "{} stored checksum is pinned by an override and will not be recomputed on \
                     save",
                    region_label(region)
                ),
                span: Some(at..at + 1),
            });
        }
    }
}

/// Party checks: `W-PARTY-COUNT` (count byte above 6),
/// `W-PARTY-SENTINEL` (species list not `0xFF`-terminated at the count),
/// `W-SPECIES-INVALID` and `W-LEVEL-RANGE` per mon.
fn party(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    let count = usize::from(buf[offsets::PARTY + party_layout::COUNT]);
    if count > offsets::PARTY_CAPACITY {
        diags.push(warn(
            "W-PARTY-COUNT",
            format!(
                "party count is {count} but the party holds at most {}",
                offsets::PARTY_CAPACITY
            ),
            offsets::PARTY..offsets::PARTY + 1,
        ));
    }
    let len = count.min(offsets::PARTY_CAPACITY);

    let sentinel_at = offsets::PARTY + party_layout::SPECIES_LIST + len;
    if buf[sentinel_at] != 0xFF {
        diags.push(warn(
            "W-PARTY-SENTINEL",
            format!(
                "party species list is not 0xFF-terminated after entry {len} (found 0x{:02X})",
                buf[sentinel_at]
            ),
            sentinel_at..sentinel_at + 1,
        ));
    }

    for i in 0..len {
        let species_at = party_layout::mon_at(i);
        let species = buf[species_at];
        if INDEX_TO_DEX[usize::from(species)] == 0 {
            diags.push(warn(
                "W-SPECIES-INVALID",
                format!("party slot {i} species 0x{species:02X} is not a valid Pokémon"),
                species_at..species_at + 1,
            ));
        }
        // +0x21 is the authoritative party level (FORMAT.md).
        let level_at = party_layout::mon_at(i) + 0x21;
        let level = buf[level_at];
        if level > MAX_LEVEL {
            diags.push(warn(
                "W-LEVEL-RANGE",
                format!("party slot {i} level {level} is above the legitimate maximum 100"),
                level_at..level_at + 1,
            ));
        }
    }
}

/// Box checks for all 12 bank blocks plus the current-box working copy:
/// `W-BOX-COUNT` (count above 20) and `W-BOX-SENTINEL` (species list not
/// terminated at the count).
fn boxes(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    let blocks = (0..offsets::NUM_BOXES)
        .map(|n| (offsets::box_offset(n), format!("box {}", n + 1)))
        .chain([(offsets::CURRENT_BOX, "current box".to_string())]);
    for (base, label) in blocks {
        let count = usize::from(buf[base + box_layout::COUNT]);
        if count > offsets::MONS_PER_BOX {
            diags.push(warn(
                "W-BOX-COUNT",
                format!(
                    "{label} count is {count} but a box holds at most {}",
                    offsets::MONS_PER_BOX
                ),
                base..base + 1,
            ));
        }
        let len = count.min(offsets::MONS_PER_BOX);
        let sentinel_at = base + box_layout::SPECIES_LIST + len;
        if buf[sentinel_at] != 0xFF {
            diags.push(warn(
                "W-BOX-SENTINEL",
                format!(
                    "{label} species list is not 0xFF-terminated after entry {len} (found 0x{:02X})",
                    buf[sentinel_at]
                ),
                sentinel_at..sentinel_at + 1,
            ));
        }
    }
}

/// `W-LEVEL-EXP-MISMATCH`: a stored level byte that disagrees with the
/// level the game derives from experience. The game trusts experience —
/// on withdrawal (and at the next experience gain) it recomputes the
/// level via `CalcLevelFromExperience`, so a mismatched mon silently
/// changes level in play. Checked for occupied party slots (level byte
/// `+0x21`), every slot of the 12 bank boxes and the current-box
/// working copy (level byte `+0x03`), and the daycare. Glitch species
/// (no growth curve; `W-SPECIES-INVALID` already fires) and levels
/// above 100 (`W-LEVEL-RANGE`; the editor's exp→level lookup caps at
/// 100 and would misfire) are skipped.
fn level_exp_coherence(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    let skip = |species: u8, level: u8| {
        INDEX_TO_DEX[usize::from(species)] == 0 || level > MAX_LEVEL || level == 0
    };

    let party_len =
        usize::from(buf[offsets::PARTY + party_layout::COUNT]).min(offsets::PARTY_CAPACITY);
    for i in 0..party_len {
        let at = party_layout::mon_at(i);
        let mon = PartyMonView::new(&buf[at..at + offsets::PARTY_MON_SIZE]);
        if skip(mon.species(), mon.level()) {
            continue;
        }
        let from_exp = mon.level_from_exp();
        if mon.level() != from_exp {
            let level_at = at + 0x21;
            diags.push(warn(
                "W-LEVEL-EXP-MISMATCH",
                format!(
                    "party slot {i} level {} does not match its experience: the game computes \
                     level {from_exp} on withdrawal and at the next experience gain",
                    mon.level()
                ),
                level_at..level_at + 1,
            ));
        }
    }

    let blocks = (0..offsets::NUM_BOXES)
        .map(|n| (offsets::box_offset(n), format!("box {}", n + 1)))
        .chain([(offsets::CURRENT_BOX, "current box".to_string())]);
    for (base, label) in blocks {
        let len = usize::from(buf[base + box_layout::COUNT]).min(offsets::MONS_PER_BOX);
        for i in 0..len {
            let at = base + box_layout::mon_at(i);
            let mon = BoxMonView::new(&buf[at..at + offsets::BOX_MON_SIZE]);
            if skip(mon.species(), mon.box_level()) {
                continue;
            }
            let from_exp = mon.level_from_exp();
            if mon.box_level() != from_exp {
                let level_at = at + 0x03;
                diags.push(warn(
                    "W-LEVEL-EXP-MISMATCH",
                    format!(
                        "{label} slot {i} level byte is {} but its experience gives level \
                         {from_exp}; the game derives level from experience on withdrawal",
                        mon.box_level()
                    ),
                    level_at..level_at + 1,
                ));
            }
        }
    }

    if buf[offsets::DAYCARE_IN_USE] != 0 {
        let at = offsets::DAYCARE_MON;
        let mon = BoxMonView::new(&buf[at..at + offsets::BOX_MON_SIZE]);
        if !skip(mon.species(), mon.box_level()) {
            let from_exp = mon.level_from_exp();
            if mon.box_level() != from_exp {
                let level_at = at + 0x03;
                diags.push(warn(
                    "W-LEVEL-EXP-MISMATCH",
                    format!(
                        "daycare mon level byte is {} but its experience gives level {from_exp}; \
                         the game derives level from experience when it returns to the party",
                        mon.box_level()
                    ),
                    level_at..level_at + 1,
                ));
            }
        }
    }
}

/// `W-BCD-MONEY` / `W-BCD-COINS`: a nibble above 9 in the packed-BCD
/// money or coin fields.
fn money_and_coins(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    if bcd::decode(&buf[offsets::MONEY..offsets::MONEY + 3]).is_err() {
        diags.push(warn(
            "W-BCD-MONEY",
            "money field holds invalid BCD (a nibble above 9)".to_string(),
            offsets::MONEY..offsets::MONEY + 3,
        ));
    }
    if bcd::decode(&buf[offsets::COINS..offsets::COINS + 2]).is_err() {
        diags.push(warn(
            "W-BCD-COINS",
            "coins field holds invalid BCD (a nibble above 9)".to_string(),
            offsets::COINS..offsets::COINS + 2,
        ));
    }
}

/// `W-TEXT-UNTERMINATED`: a name field with no `0x50` terminator (the
/// game would print into the following bytes). Checked for the player
/// name, rival name and the nicknames of occupied party slots.
fn text_terminators(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    let check = |what: String, at: usize, diags: &mut Vec<Diagnostic>| {
        if !buf[at..at + offsets::NAME_LEN].contains(&TERMINATOR) {
            diags.push(warn(
                "W-TEXT-UNTERMINATED",
                format!("{what} has no 0x50 text terminator"),
                at..at + offsets::NAME_LEN,
            ));
        }
    };
    check("player name".to_string(), offsets::PLAYER_NAME, diags);
    check("rival name".to_string(), offsets::RIVAL_NAME, diags);
    let party_len =
        usize::from(buf[offsets::PARTY + party_layout::COUNT]).min(offsets::PARTY_CAPACITY);
    for i in 0..party_len {
        check(
            format!("party slot {i} nickname"),
            party_layout::nickname_at(i),
            diags,
        );
    }
}

/// `W-BOX-INIT` and `W-BOX-STALE`: the boxes-initialized bit and the
/// bank copy of the current box.
fn box_flags(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    let box_num_byte = buf[offsets::CURRENT_BOX_NUM];
    if box_num_byte & 0x80 == 0 {
        diags.push(warn(
            "W-BOX-INIT",
            "boxes-initialized flag (bit 7 of wCurrentBoxNum) is clear: the game will treat the \
             box banks as uninitialized and wipe all boxes on load"
                .to_string(),
            offsets::CURRENT_BOX_NUM..offsets::CURRENT_BOX_NUM + 1,
        ));
    }

    let n = usize::from(box_num_byte & 0x7F);
    if n < offsets::NUM_BOXES {
        let bank = offsets::box_offset(n);
        if buf[bank..bank + offsets::BOX_LEN]
            != buf[offsets::CURRENT_BOX..offsets::CURRENT_BOX + offsets::BOX_LEN]
        {
            diags.push(warn(
                "W-BOX-STALE",
                format!(
                    "bank copy of current box {} differs from the working copy at 0x30C0; the \
                     working copy is authoritative (common in real saves — the game only flushes \
                     on box switch); sync_current_box_to_bank() reconciles them",
                    n + 1
                ),
                bank..bank + offsets::BOX_LEN,
            ));
        }
    }
}

/// `W-SPECIES-INVALID` for an occupied daycare whose mon species maps to
/// no Pokémon.
fn daycare(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    if buf[offsets::DAYCARE_IN_USE] == 0 {
        return;
    }
    let species = buf[offsets::DAYCARE_MON];
    if INDEX_TO_DEX[usize::from(species)] == 0 {
        diags.push(warn(
            "W-SPECIES-INVALID",
            format!("daycare is in use but its mon species 0x{species:02X} is not a valid Pokémon"),
            offsets::DAYCARE_MON..offsets::DAYCARE_MON + 1,
        ));
    }
}

/// `W-DEX-RANGE`: a set bit above dex #151 in the owned/seen bitfields
/// (only bit 151, the last bit of byte 18, exists to be wrong).
fn dex_range(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    for (field, what) in [
        (offsets::POKEDEX_OWNED, "owned"),
        (offsets::POKEDEX_SEEN, "seen"),
    ] {
        let last = field + offsets::POKEDEX_LEN - 1;
        if buf[last] & 0x80 != 0 {
            diags.push(warn(
                "W-DEX-RANGE",
                format!("Pokédex {what} bitfield has bit 151 set; only entries 1-151 exist"),
                last..last + 1,
            ));
        }
    }
}

/// `W-MAP-UNKNOWN`: the current map id has no map in the game.
fn map(buf: &[u8], diags: &mut Vec<Diagnostic>) {
    let id = buf[offsets::CUR_MAP];
    if MAP_NAMES[usize::from(id)].is_empty() {
        diags.push(warn(
            "W-MAP-UNKNOWN",
            format!("current map id 0x{id:02X} is not a map; the game may crash on load"),
            offsets::CUR_MAP..offsets::CUR_MAP + 1,
        ));
    }
}
