//! Integration tests for the Gen 1 stat math (`gen1::stats`) and mon
//! record views (`gen1::pokemon`).

use pksave::gen1::data::{BASE_STATS, DEX_TO_INDEX};
use pksave::gen1::offsets::{BOX_MON_SIZE, PARTY_MON_SIZE};
use pksave::gen1::pokemon::{
    box_to_party, party_to_box, BoxMonMut, BoxMonView, MonMut, MonView, PartyMon, PartyMonMut,
    PartyMonView,
};
use pksave::gen1::stats::{
    calc_stat, compose_pp, current_pp, exp_for_level, level_for_exp, pp_ups, Dvs,
};
use proptest::prelude::*;

// ---- stats: formula vectors ----

#[test]
fn calc_stat_formula_vectors() {
    // Max everything, base 45: E = floor(255/4) = 63;
    // ((45+15)*2 + 63) * 100 / 100 = 183.
    assert_eq!(calc_stat(45, 15, 65535, 100, false), 188);
    assert_eq!(calc_stat(45, 15, 65535, 100, true), 293);
    // Fresh level-5 mon: no stat exp.
    assert_eq!(calc_stat(49, 8, 0, 5, false), 10);
    // Well-known level 100 maximums (Mewtwo Special / HP).
    assert_eq!(calc_stat(154, 15, 65535, 100, false), 406);
    assert_eq!(calc_stat(106, 15, 65535, 100, true), 415);
    // Level 1 minimum: core = 0 -> other 0+5, hp 0+1+10.
    assert_eq!(calc_stat(0, 0, 0, 1, false), 5);
    assert_eq!(calc_stat(0, 0, 0, 1, true), 11);
}

// ---- stats: DV packing ----

#[test]
fn dv_pack_layout() {
    let dvs = Dvs {
        attack: 0x1,
        defense: 0x2,
        speed: 0x3,
        special: 0x4,
    };
    assert_eq!(dvs.pack(), [0x12, 0x34]);
    assert_eq!(Dvs::unpack([0x12, 0x34]), dvs);
    assert_eq!(dvs.hp_dv(), 0b1010); // odd, even, odd, even
}

proptest! {
    #[test]
    fn dv_pack_unpack_round_trips(atk in 0u8..16, def in 0u8..16, spd in 0u8..16, spc in 0u8..16) {
        let dvs = Dvs { attack: atk, defense: def, speed: spd, special: spc };
        prop_assert_eq!(Dvs::unpack(dvs.pack()), dvs);
    }

    #[test]
    fn dv_unpack_pack_round_trips(b0: u8, b1: u8) {
        prop_assert_eq!(Dvs::unpack([b0, b1]).pack(), [b0, b1]);
    }
}

// ---- stats: PP bits ----

#[test]
fn pp_bits() {
    for ups in 0..4u8 {
        for current in [0u8, 1, 33, 63] {
            let byte = compose_pp(current, ups);
            assert_eq!(current_pp(byte), current);
            assert_eq!(pp_ups(byte), ups);
        }
    }
}

// ---- stats: growth curves ----

/// The growth rates used by Gen 1 species: Medium Fast, Medium Slow,
/// Fast, Slow.
const GEN1_GROWTH_RATES: [u8; 4] = [0, 3, 4, 5];

#[test]
fn exp_curves_match_known_values() {
    assert_eq!(exp_for_level(0, 100), 1_000_000);
    assert_eq!(exp_for_level(3, 100), 1_059_860);
    assert_eq!(exp_for_level(4, 100), 800_000);
    assert_eq!(exp_for_level(5, 100), 1_250_000);
    // Level 5 Medium Slow starter.
    assert_eq!(exp_for_level(3, 5), 135);
    // Medium Slow level 1 is clamped to 0 (the true polynomial is -54).
    assert_eq!(exp_for_level(3, 1), 0);
}

#[test]
fn exp_curves_are_monotonic() {
    for g in GEN1_GROWTH_RATES {
        for n in 2..=100u8 {
            assert!(
                exp_for_level(g, n) > exp_for_level(g, n - 1),
                "growth {g} not strictly increasing at level {n}"
            );
        }
    }
}

#[test]
fn level_for_exp_inverts_exp_for_level() {
    for g in GEN1_GROWTH_RATES {
        for n in 1..=100u8 {
            assert_eq!(
                level_for_exp(g, exp_for_level(g, n)),
                n,
                "growth {g} level {n}"
            );
            // One exp point short of the next level stays at n.
            if n < 100 {
                assert_eq!(level_for_exp(g, exp_for_level(g, n + 1) - 1), n);
            }
        }
    }
}

// ---- pokemon: field accessors against a hand-built byte pattern ----

/// A 44-byte party record with a distinct value in every field,
/// laid out per docs/FORMAT.md.
fn patterned_party_mon() -> [u8; PARTY_MON_SIZE] {
    let mut b = [0u8; PARTY_MON_SIZE];
    b[0x00] = 0x99; // species (Bulbasaur internal index)
    b[0x01..0x03].copy_from_slice(&[0x01, 0x02]); // current HP = 258
    b[0x03] = 4; // box level
    b[0x04] = 0x5A; // status: sleep 2 + PSN + BRN + PAR
    b[0x05] = 22; // type 1
    b[0x06] = 3; // type 2
    b[0x07] = 45; // catch rate
    b[0x08..0x0C].copy_from_slice(&[33, 45, 22, 0]); // moves
    b[0x0C..0x0E].copy_from_slice(&[0xAB, 0xCD]); // OT id
    b[0x0E..0x11].copy_from_slice(&[0x01, 0x23, 0x45]); // exp = 0x012345
    b[0x11..0x1B].copy_from_slice(&[
        0x11, 0x11, // HP stat exp
        0x22, 0x22, // Attack
        0x33, 0x33, // Defense
        0x44, 0x44, // Speed
        0x55, 0x55, // Special
    ]);
    b[0x1B..0x1D].copy_from_slice(&[0xAB, 0xCD]); // DVs
    b[0x1D..0x21].copy_from_slice(&[0x41, 0x82, 0xC3, 0x04]); // PP
    b[0x21] = 56; // party level
    b[0x22..0x24].copy_from_slice(&[0x00, 0xC8]); // max HP = 200
    b[0x24..0x26].copy_from_slice(&[0x00, 0x64]); // attack = 100
    b[0x26..0x28].copy_from_slice(&[0x00, 0x65]); // defense = 101
    b[0x28..0x2A].copy_from_slice(&[0x00, 0x66]); // speed = 102
    b[0x2A..0x2C].copy_from_slice(&[0x00, 0x67]); // special = 103
    b
}

#[test]
fn party_mon_view_reads_every_field_at_its_offset() {
    let bytes = patterned_party_mon();
    let mon = PartyMonView::new(&bytes);
    assert_eq!(mon.species(), 0x99);
    assert_eq!(mon.current_hp(), 258);
    assert_eq!(mon.box_level(), 4);
    assert_eq!(mon.status(), 0x5A);
    assert_eq!(mon.types(), (22, 3));
    assert_eq!(mon.catch_rate(), 45);
    assert_eq!(mon.moves(), [33, 45, 22, 0]);
    assert_eq!(mon.ot_id(), 0xABCD);
    assert_eq!(mon.exp(), 0x012345);
    assert_eq!(mon.stat_exps(), [0x1111, 0x2222, 0x3333, 0x4444, 0x5555]);
    assert_eq!(
        mon.dvs(),
        Dvs {
            attack: 0xA,
            defense: 0xB,
            speed: 0xC,
            special: 0xD
        }
    );
    assert_eq!(mon.pp(), [0x41, 0x82, 0xC3, 0x04]);
    assert_eq!(current_pp(mon.pp()[0]), 1);
    assert_eq!(pp_ups(mon.pp()[0]), 1);
    assert_eq!(mon.level(), 56);
    assert_eq!(mon.max_hp(), 200);
    assert_eq!(mon.attack(), 100);
    assert_eq!(mon.defense(), 101);
    assert_eq!(mon.speed(), 102);
    assert_eq!(mon.special(), 103);
}

#[test]
fn box_mon_view_shares_the_first_33_bytes() {
    let party = patterned_party_mon();
    let mut bytes = [0u8; BOX_MON_SIZE];
    bytes.copy_from_slice(&party[..BOX_MON_SIZE]);
    let mon = BoxMonView::new(&bytes);
    assert_eq!(mon.species(), 0x99);
    assert_eq!(mon.current_hp(), 258);
    assert_eq!(mon.box_level(), 4);
    assert_eq!(mon.status(), 0x5A);
    assert_eq!(mon.types(), (22, 3));
    assert_eq!(mon.catch_rate(), 45);
    assert_eq!(mon.moves(), [33, 45, 22, 0]);
    assert_eq!(mon.ot_id(), 0xABCD);
    assert_eq!(mon.exp(), 0x012345);
    assert_eq!(mon.stat_exps(), [0x1111, 0x2222, 0x3333, 0x4444, 0x5555]);
    assert_eq!(mon.pp(), [0x41, 0x82, 0xC3, 0x04]);
}

#[test]
fn party_mon_setters_write_the_exact_bytes() {
    let expected = patterned_party_mon();
    let mut bytes = [0u8; PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(0x99);
    mon.set_current_hp(258);
    mon.set_box_level(4);
    mon.set_status(0x5A);
    mon.set_types(22, 3);
    mon.set_catch_rate(45);
    mon.set_moves([33, 45, 22, 0]);
    mon.set_ot_id(0xABCD);
    mon.set_exp(0x012345);
    mon.set_stat_exps([0x1111, 0x2222, 0x3333, 0x4444, 0x5555]);
    mon.set_dvs(Dvs {
        attack: 0xA,
        defense: 0xB,
        speed: 0xC,
        special: 0xD,
    });
    mon.set_pp([0x41, 0x82, 0xC3, 0x04]);
    mon.set_level(56);
    mon.set_max_hp(200);
    mon.set_attack(100);
    mon.set_defense(101);
    mon.set_speed(102);
    mon.set_special(103);
    assert_eq!(bytes, expected);
}

#[test]
fn box_mon_setters_write_the_exact_bytes() {
    let party = patterned_party_mon();
    let mut expected = [0u8; BOX_MON_SIZE];
    expected.copy_from_slice(&party[..BOX_MON_SIZE]);
    let mut bytes = [0u8; BOX_MON_SIZE];
    let mut mon = BoxMonMut::new(&mut bytes);
    mon.set_species(0x99);
    mon.set_current_hp(258);
    mon.set_box_level(4);
    mon.set_status(0x5A);
    mon.set_types(22, 3);
    mon.set_catch_rate(45);
    mon.set_moves([33, 45, 22, 0]);
    mon.set_ot_id(0xABCD);
    mon.set_exp(0x012345);
    mon.set_stat_exps([0x1111, 0x2222, 0x3333, 0x4444, 0x5555]);
    mon.set_dvs(Dvs {
        attack: 0xA,
        defense: 0xB,
        speed: 0xC,
        special: 0xD,
    });
    mon.set_pp([0x41, 0x82, 0xC3, 0x04]);
    assert_eq!(mon.as_view().as_bytes(), &expected);
    assert_eq!(bytes, expected);
}

#[test]
fn status_bitfield_helpers() {
    let mut bytes = [0u8; PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    assert_eq!(mon.sleep_turns(), 0);
    assert!(!mon.is_poisoned() && !mon.is_burned() && !mon.is_frozen() && !mon.is_paralyzed());

    mon.set_status(0b0000_0111); // sleep counter maxed
    assert_eq!(mon.sleep_turns(), 7);
    assert!(!mon.is_poisoned());

    mon.set_status(1 << 3);
    assert!(mon.is_poisoned());
    mon.set_status(1 << 4);
    assert!(mon.is_burned());
    mon.set_status(1 << 5);
    assert!(mon.is_frozen());
    mon.set_status(1 << 6);
    assert!(mon.is_paralyzed());

    mon.set_status(0x5A); // sleep 2 + PSN + BRN + PAR
    assert_eq!(mon.sleep_turns(), 2);
    assert!(mon.is_poisoned());
    assert!(mon.is_burned());
    assert!(!mon.is_frozen());
    assert!(mon.is_paralyzed());
}

#[test]
fn exp_setter_masks_to_24_bits() {
    let mut bytes = [0u8; PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_exp(0xFF_ABCDEF);
    assert_eq!(mon.exp(), 0xABCDEF);
}

// ---- pokemon: recalculation ----

#[test]
fn recalculate_stats_uses_base_stats_dvs_and_stat_exp() {
    // Mewtwo (dex 150) at level 100 with max DVs and max stat exp.
    let mewtwo = usize::from(DEX_TO_INDEX[150]);
    let base = BASE_STATS[150];
    let mut bytes = [0u8; PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(mewtwo as u8);
    mon.set_dvs(Dvs {
        attack: 15,
        defense: 15,
        speed: 15,
        special: 15,
    });
    mon.set_stat_exps([65535; 5]);
    mon.set_level(100);
    mon.recalculate_stats();
    assert_eq!(mon.max_hp(), 415);
    assert_eq!(mon.special(), 406);
    assert_eq!(mon.attack(), calc_stat(base.attack, 15, 65535, 100, false));
    assert_eq!(
        mon.defense(),
        calc_stat(base.defense, 15, 65535, 100, false)
    );
    assert_eq!(mon.speed(), calc_stat(base.speed, 15, 65535, 100, false));
}

#[test]
fn set_level_coherent_updates_level_exp_stats_and_heals() {
    // Bulbasaur (dex 1, Medium Slow).
    let bulbasaur = DEX_TO_INDEX[1];
    assert_eq!(BASE_STATS[1].growth_rate, 3);
    let mut bytes = [0u8; PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(bulbasaur);
    mon.set_dvs(Dvs {
        attack: 8,
        defense: 9,
        speed: 10,
        special: 11,
    });
    mon.set_level_coherent(50);
    assert_eq!(mon.level(), 50);
    assert_eq!(mon.box_level(), 50, "box level byte kept coherent");
    assert_eq!(mon.exp(), exp_for_level(3, 50));
    assert_eq!(level_for_exp(3, mon.exp()), 50);
    let dvs = mon.dvs();
    assert_eq!(
        mon.max_hp(),
        calc_stat(BASE_STATS[1].hp, dvs.hp_dv(), 0, 50, true)
    );
    assert_eq!(mon.current_hp(), mon.max_hp(), "full heal policy");

    // Lowering the level heals down too: current HP never exceeds max.
    mon.set_level_coherent(10);
    assert_eq!(mon.current_hp(), mon.max_hp());
    assert!(mon.exp() < exp_for_level(3, 50));
}

// ---- pokemon: party <-> box conversions ----

#[test]
fn party_to_box_truncates_and_carries_the_true_level() {
    let party = patterned_party_mon();
    let boxed = party_to_box(&party);
    // Every byte except the level byte is the party record's prefix.
    for (i, &b) in boxed.iter().enumerate() {
        if i == 0x03 {
            assert_eq!(b, 56, "box level := party level (0x21)");
        } else {
            assert_eq!(b, party[i], "byte {i:#04x}");
        }
    }
}

#[test]
fn box_to_party_restores_level_and_recalculates_stats() {
    // Build a coherent party mon, deposit, withdraw.
    let bulbasaur = DEX_TO_INDEX[1];
    let mut original = [0u8; PARTY_MON_SIZE];
    {
        let mut mon = PartyMonMut::new(&mut original);
        mon.set_species(bulbasaur);
        mon.set_types(22, 3);
        mon.set_catch_rate(45);
        mon.set_moves([33, 45, 0, 0]);
        mon.set_ot_id(0xBEEF);
        mon.set_dvs(Dvs {
            attack: 5,
            defense: 10,
            speed: 15,
            special: 2,
        });
        mon.set_stat_exps([120, 340, 560, 780, 900]);
        mon.set_pp([35, 40, 0, 0]);
        mon.set_level_coherent(31);
        mon.set_current_hp(17); // battle-worn, below max
    }
    let boxed = party_to_box(&original);
    assert_eq!(boxed[0x03], 31);
    let restored = box_to_party(&boxed);
    assert_eq!(restored, original, "party -> box -> party round-trips");
    let view = PartyMonView::new(&restored);
    assert_eq!(view.level(), 31);
    assert_eq!(view.current_hp(), 17, "current HP kept as stored");
}

#[test]
fn box_to_party_recomputes_stats_for_the_stored_level() {
    // A raw box mon that never had party stats: withdraw computes them.
    let pikachu = DEX_TO_INDEX[25];
    let mut boxed = [0u8; BOX_MON_SIZE];
    {
        let mut mon = BoxMonMut::new(&mut boxed);
        mon.set_species(pikachu);
        mon.set_box_level(42);
        mon.set_dvs(Dvs {
            attack: 7,
            defense: 7,
            speed: 7,
            special: 7,
        });
        mon.set_stat_exps([1000, 2000, 3000, 4000, 5000]);
    }
    let party = box_to_party(&boxed);
    let view = PartyMonView::new(&party);
    let base = BASE_STATS[25];
    assert_eq!(view.level(), 42);
    let dvs = view.dvs();
    assert_eq!(
        view.max_hp(),
        calc_stat(base.hp, dvs.hp_dv(), 1000, 42, true)
    );
    assert_eq!(view.attack(), calc_stat(base.attack, 7, 2000, 42, false));
    assert_eq!(view.defense(), calc_stat(base.defense, 7, 3000, 42, false));
    assert_eq!(view.speed(), calc_stat(base.speed, 7, 4000, 42, false));
    assert_eq!(view.special(), calc_stat(base.special, 7, 5000, 42, false));
}

// ---- traits: party/box parity over the shared 33 bytes ----

/// Every [`MonView`] getter except `as_bytes`, bundled for equality
/// comparison between views (`as_bytes` is compared as a prefix by the
/// caller since the record lengths differ).
#[derive(Debug, PartialEq)]
struct CommonFields {
    species: u8,
    current_hp: u16,
    box_level: u8,
    status: u8,
    sleep_turns: u8,
    is_poisoned: bool,
    is_burned: bool,
    is_frozen: bool,
    is_paralyzed: bool,
    types: (u8, u8),
    catch_rate: u8,
    moves: [u8; 4],
    ot_id: u16,
    exp: u32,
    stat_exps: [u16; 5],
    dvs: Dvs,
    pp: [u8; 4],
}

fn common_fields(mon: &dyn MonView) -> CommonFields {
    CommonFields {
        species: mon.species(),
        current_hp: mon.current_hp(),
        box_level: mon.box_level(),
        status: mon.status(),
        sleep_turns: mon.sleep_turns(),
        is_poisoned: mon.is_poisoned(),
        is_burned: mon.is_burned(),
        is_frozen: mon.is_frozen(),
        is_paralyzed: mon.is_paralyzed(),
        types: mon.types(),
        catch_rate: mon.catch_rate(),
        moves: mon.moves(),
        ot_id: mon.ot_id(),
        exp: mon.exp(),
        stat_exps: mon.stat_exps(),
        dvs: mon.dvs(),
        pp: mon.pp(),
    }
}

/// One arbitrary value per [`MonMut`] setter.
#[derive(Debug)]
struct CommonEdit {
    species: u8,
    current_hp: u16,
    box_level: u8,
    status: u8,
    types: (u8, u8),
    catch_rate: u8,
    moves: [u8; 4],
    ot_id: u16,
    exp: u32,
    stat_exps: [u16; 5],
    dvs: [u8; 2],
    pp: [u8; 4],
}

prop_compose! {
    fn arb_common_edit()(
        (species, current_hp, box_level, status, types, catch_rate) in any::<(u8, u16, u8, u8, (u8, u8), u8)>(),
        (moves, ot_id, exp, stat_exps, dvs, pp) in any::<([u8; 4], u16, u32, [u16; 5], [u8; 2], [u8; 4])>(),
    ) -> CommonEdit {
        CommonEdit {
            species, current_hp, box_level, status, types, catch_rate,
            moves, ot_id, exp, stat_exps, dvs, pp,
        }
    }
}

/// Drive every [`MonMut`] setter once.
fn apply_common_edit(mon: &mut dyn MonMut, e: &CommonEdit) {
    mon.set_species(e.species);
    mon.set_current_hp(e.current_hp);
    mon.set_box_level(e.box_level);
    mon.set_status(e.status);
    mon.set_types(e.types.0, e.types.1);
    mon.set_catch_rate(e.catch_rate);
    mon.set_moves(e.moves);
    mon.set_ot_id(e.ot_id);
    mon.set_exp(e.exp);
    mon.set_stat_exps(e.stat_exps);
    mon.set_dvs(Dvs::unpack(e.dvs));
    mon.set_pp(e.pp);
}

proptest! {
    #[test]
    fn mon_view_getters_agree_between_party_and_box(record in any::<[u8; PARTY_MON_SIZE]>()) {
        let party = PartyMonView::new(&record);
        let boxed = BoxMonView::new(&record[..BOX_MON_SIZE]);
        prop_assert_eq!(&party.as_bytes()[..BOX_MON_SIZE], boxed.as_bytes());
        prop_assert_eq!(common_fields(&party), common_fields(&boxed));
    }

    #[test]
    fn mon_mut_setters_agree_between_party_and_box(
        record in any::<[u8; PARTY_MON_SIZE]>(),
        edit in arb_common_edit(),
    ) {
        let mut party_bytes = record;
        let mut box_bytes = [0u8; BOX_MON_SIZE];
        box_bytes.copy_from_slice(&record[..BOX_MON_SIZE]);
        apply_common_edit(&mut PartyMonMut::new(&mut party_bytes), &edit);
        apply_common_edit(&mut BoxMonMut::new(&mut box_bytes), &edit);
        prop_assert_eq!(&party_bytes[..BOX_MON_SIZE], &box_bytes[..]);
        // The setters are common-field setters: the party-only tail is
        // untouched.
        prop_assert_eq!(&party_bytes[BOX_MON_SIZE..], &record[BOX_MON_SIZE..]);
    }
}

// ---- traits: dyn compatibility ----

#[test]
fn mon_traits_are_dyn_compatible() {
    fn read_species(mon: &dyn MonView) -> u8 {
        mon.species()
    }
    fn write_hp(mon: &mut dyn MonMut) {
        mon.set_current_hp(123);
    }

    let party = patterned_party_mon();
    let mut boxed = [0u8; BOX_MON_SIZE];
    boxed.copy_from_slice(&party[..BOX_MON_SIZE]);
    assert_eq!(read_species(&PartyMonView::new(&party)), 0x99);
    assert_eq!(read_species(&BoxMonView::new(&boxed)), 0x99);

    let mut party_bytes = party;
    let mut party_mon = PartyMonMut::new(&mut party_bytes);
    write_hp(&mut party_mon);
    assert_eq!(party_mon.current_hp(), 123);
    let mut box_mon = BoxMonMut::new(&mut boxed);
    write_hp(&mut box_mon);
    assert_eq!(box_mon.current_hp(), 123);
}

// ---- proptest: conversions preserve everything derivable ----

proptest! {
    #[test]
    fn party_box_party_round_trips_for_coherent_mons(
        dex in 1usize..=151,
        level in 2u8..=100,
        atk in 0u8..16, def in 0u8..16, spd in 0u8..16, spc in 0u8..16,
        hp_exp: u16, atk_exp: u16, def_exp: u16, spd_exp: u16, spc_exp: u16,
        ot_id: u16,
    ) {
        let mut original = [0u8; PARTY_MON_SIZE];
        {
            let mut mon = PartyMonMut::new(&mut original);
            mon.set_species(DEX_TO_INDEX[dex]);
            mon.set_ot_id(ot_id);
            mon.set_dvs(Dvs { attack: atk, defense: def, speed: spd, special: spc });
            mon.set_stat_exps([hp_exp, atk_exp, def_exp, spd_exp, spc_exp]);
            mon.set_level_coherent(level);
        }
        let restored = box_to_party(&party_to_box(&original));
        prop_assert_eq!(restored, original);
    }
}
