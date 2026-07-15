//! P6: property test — an arbitrary sequence of valid party operations
//! keeps the party-block invariants intact after every step, and never
//! writes a byte outside the party region.

use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::offsets;
use pksave::gen1::party::PartyError;
use pksave::gen1::pokemon::{MonMut, MonView, PartyMonMut};
use pksave::gen1::save::SaveFile;
use pksave::gen1::text;
use proptest::collection::vec;
use proptest::prelude::*;

const PARTY_END: usize = offsets::PARTY + offsets::PARTY_LEN;

/// Proptest case count: the `PROPTEST_CASES` env var when set (e.g. to
/// raise coverage in CI), otherwise `default`.
fn env_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

#[derive(Debug, Clone)]
enum Op {
    Add {
        dex: u8,
        level: u8,
        ot: String,
        nick: String,
    },
    Remove(usize),
    Swap(usize, usize),
    SetOtName(usize, String),
    SetNickname(usize, String),
    SetSpecies(usize, u8),
    Clear,
}

fn name_strategy() -> impl Strategy<Value = String> {
    // Up to 10 uppercase letters: always encodable within NAME_LEN.
    proptest::string::string_regex("[A-Z]{0,10}").expect("valid regex")
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        4 => (1u8..=151, 2u8..=100, name_strategy(), name_strategy())
            .prop_map(|(dex, level, ot, nick)| Op::Add { dex, level, ot, nick }),
        2 => (0usize..offsets::PARTY_CAPACITY).prop_map(Op::Remove),
        2 => (0usize..offsets::PARTY_CAPACITY, 0usize..offsets::PARTY_CAPACITY)
            .prop_map(|(i, j)| Op::Swap(i, j)),
        1 => (0usize..offsets::PARTY_CAPACITY, name_strategy())
            .prop_map(|(i, s)| Op::SetOtName(i, s)),
        1 => (0usize..offsets::PARTY_CAPACITY, name_strategy())
            .prop_map(|(i, s)| Op::SetNickname(i, s)),
        1 => (0usize..offsets::PARTY_CAPACITY, 1u8..=151)
            .prop_map(|(i, dex)| Op::SetSpecies(i, dex)),
        1 => Just(Op::Clear),
    ]
}

/// Shadow model of one party slot.
#[derive(Debug, Clone, PartialEq)]
struct Slot {
    mon: [u8; offsets::PARTY_MON_SIZE],
    ot: String,
    nick: String,
}

fn make_mon(dex: u8, level: u8) -> [u8; offsets::PARTY_MON_SIZE] {
    let mut bytes = [0u8; offsets::PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[usize::from(dex)]);
    mon.set_ot_id(0xC0DE);
    mon.set_level_coherent(level);
    bytes
}

/// Assert every party-block invariant (P6) plus region isolation.
fn check_invariants(
    save: &SaveFile,
    model: &[Slot],
    background: &[u8],
) -> Result<(), TestCaseError> {
    let raw = &save.as_bytes()[offsets::PARTY..PARTY_END];

    // Count matches the model and the species list is sentinel-terminated.
    prop_assert_eq!(usize::from(raw[0]), model.len(), "count byte");
    prop_assert_eq!(raw[1 + model.len()], 0xFF, "sentinel after last entry");

    for (i, slot) in model.iter().enumerate() {
        // Species list entry i == mon(i).species() == model species.
        prop_assert_eq!(raw[1 + i], slot.mon[0], "species list entry {}", i);
        let party = save.party();
        prop_assert_eq!(party.mon(i).species(), slot.mon[0], "mon {} species", i);
        // Parallel arrays stay aligned with their mon.
        let mon = party.mon(i);
        prop_assert_eq!(mon.as_bytes(), &slot.mon[..], "mon {} bytes", i);
        prop_assert_eq!(party.ot_name(i), slot.ot.clone(), "OT name {}", i);
        prop_assert_eq!(party.nickname(i), slot.nick.clone(), "nickname {}", i);
    }

    // No byte outside the party region ever changes.
    let after = save.as_bytes();
    prop_assert_eq!(
        &after[..offsets::PARTY],
        &background[..offsets::PARTY],
        "bytes before the party region"
    );
    prop_assert_eq!(
        &after[PARTY_END..],
        &background[PARTY_END..],
        "bytes after the party region"
    );
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: env_cases(64), ..ProptestConfig::default() })]

    #[test]
    fn party_ops_maintain_invariants_and_isolation(ops in vec(op_strategy(), 1..40)) {
        // Patterned background so silent zero-writes outside the party
        // region cannot hide.
        let mut background = vec![0u8; 0x8000];
        for (i, b) in background.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let mut save = SaveFile::from_bytes(background.clone()).expect("length is valid");
        save.party_mut().clear();
        let mut model: Vec<Slot> = Vec::new();
        check_invariants(&save, &model, &background)?;

        for op in ops {
            match op {
                Op::Add { dex, level, ot, nick } => {
                    let result = save.party_mut().add(&make_mon(dex, level), &ot, &nick);
                    if model.len() < offsets::PARTY_CAPACITY {
                        prop_assert_eq!(result, Ok(model.len()));
                        model.push(Slot { mon: make_mon(dex, level), ot, nick });
                    } else {
                        prop_assert_eq!(result, Err(PartyError::Full));
                    }
                }
                Op::Remove(i) => {
                    if i < model.len() {
                        save.party_mut().remove(i);
                        model.remove(i);
                    }
                }
                Op::Swap(i, j) => {
                    if i < model.len() && j < model.len() {
                        save.party_mut().swap(i, j);
                        model.swap(i, j);
                    }
                }
                Op::SetOtName(i, s) => {
                    if i < model.len() {
                        save.party_mut().set_ot_name(i, &s).expect("name encodes");
                        model[i].ot = s;
                    }
                }
                Op::SetNickname(i, s) => {
                    if i < model.len() {
                        save.party_mut().set_nickname(i, &s).expect("name encodes");
                        model[i].nick = s;
                    }
                }
                Op::SetSpecies(i, dex) => {
                    if i < model.len() {
                        let species = DEX_TO_INDEX[usize::from(dex)];
                        save.party_mut().set_species(i, species);
                        model[i].mon[0] = species;
                    }
                }
                Op::Clear => {
                    save.party_mut().clear();
                    model.clear();
                }
            }
            check_invariants(&save, &model, &background)?;
        }

        // Sanity: names in the model really round-trip the text codec
        // (guards the model itself against vacuous equality).
        for slot in &model {
            let encoded = text::encode(&slot.ot, offsets::NAME_LEN).expect("model name encodes");
            prop_assert_eq!(text::decode(&encoded), slot.ot.clone());
        }
    }
}
