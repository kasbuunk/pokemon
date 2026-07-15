//! Integration tests for the party editor (`gen1::party`).

use pksave::gen1::data::DEX_TO_INDEX;
use pksave::gen1::offsets;
use pksave::gen1::party::PartyError;
use pksave::gen1::pokemon::PartyMonMut;
use pksave::gen1::save::SaveFile;

const PARTY_END: usize = offsets::PARTY + offsets::PARTY_LEN;

/// A blank save with an initialized (empty) party block.
fn blank_save() -> SaveFile {
    let mut save = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("length is valid");
    save.party_mut().clear();
    save
}

/// A coherent party mon record for the given National Dex number.
fn make_mon(dex: usize, level: u8) -> [u8; offsets::PARTY_MON_SIZE] {
    let mut bytes = [0u8; offsets::PARTY_MON_SIZE];
    let mut mon = PartyMonMut::new(&mut bytes);
    mon.set_species(DEX_TO_INDEX[dex]);
    mon.set_ot_id(0x1234);
    mon.set_level_coherent(level);
    bytes
}

/// Raw party-block bytes of a save (for byte-exact assertions).
fn party_bytes(save: &SaveFile) -> &[u8] {
    &save.as_bytes()[offsets::PARTY..PARTY_END]
}

#[test]
fn clear_produces_an_empty_terminated_party() {
    let save = blank_save();
    let party = save.party();
    assert_eq!(party.len(), 0);
    assert!(party.is_empty());
    assert_eq!(party.species_list(), &[] as &[u8]);
    let raw = party_bytes(&save);
    assert_eq!(raw[0], 0, "count");
    assert_eq!(raw[1], 0xFF, "sentinel");
    assert!(raw[2..].iter().all(|&b| b == 0), "rest zeroed");
}

#[test]
fn add_to_empty_party() {
    let mut save = blank_save();
    let mon = make_mon(25, 42); // Pikachu
    let slot = save
        .party_mut()
        .add(&mon, "ASH", "SPARKY")
        .expect("party has room");
    assert_eq!(slot, 0);

    let party = save.party();
    assert_eq!(party.len(), 1);
    assert_eq!(party.species_list(), &[DEX_TO_INDEX[25]]);
    assert_eq!(party.mon(0).species(), DEX_TO_INDEX[25]);
    assert_eq!(party.mon(0).level(), 42);
    assert_eq!(party.ot_name(0), "ASH");
    assert_eq!(party.nickname(0), "SPARKY");

    // Sentinel sits right after the single species entry.
    let raw = party_bytes(&save);
    assert_eq!(raw[1], DEX_TO_INDEX[25]);
    assert_eq!(raw[2], 0xFF);
}

#[test]
fn add_to_full_party_fails_and_changes_nothing() {
    let mut save = blank_save();
    for i in 0..offsets::PARTY_CAPACITY {
        let mon = make_mon(i + 1, 10 + i as u8);
        let slot = save
            .party_mut()
            .add(&mon, "RED", "MON")
            .expect("party has room");
        assert_eq!(slot, i);
    }
    assert_eq!(save.party().len(), 6);
    let before = save.as_bytes().to_vec();

    let extra = make_mon(150, 70);
    assert_eq!(
        save.party_mut().add(&extra, "RED", "MEWTWO"),
        Err(PartyError::Full)
    );
    assert_eq!(save.as_bytes(), &before[..], "failed add writes nothing");
}

#[test]
fn add_rejects_unencodable_names_without_writing() {
    let mut save = blank_save();
    let mon = make_mon(1, 5);
    let before = save.as_bytes().to_vec();
    assert!(matches!(
        save.party_mut().add(&mon, "RED", "~~~"),
        Err(PartyError::Text(_))
    ));
    assert!(matches!(
        save.party_mut().add(&mon, "WAYTOOLONGNAME", "BULBY"),
        Err(PartyError::Text(_))
    ));
    assert_eq!(save.as_bytes(), &before[..]);
}

#[test]
fn remove_first_middle_last() {
    for &(remove_at, expect_dexes) in &[
        (0usize, [2usize, 3, 4, 5].as_slice()),
        (2, [1, 2, 4, 5].as_slice()),
        (4, [1, 2, 3, 4].as_slice()),
    ] {
        let mut save = blank_save();
        for dex in 1..=5usize {
            save.party_mut()
                .add(&make_mon(dex, 20), "RED", &format!("MON{dex}"))
                .expect("party has room");
        }
        save.party_mut().remove(remove_at);

        let party = save.party();
        assert_eq!(party.len(), 4, "remove at {remove_at}");
        let expected_species: Vec<u8> = expect_dexes.iter().map(|&d| DEX_TO_INDEX[d]).collect();
        assert_eq!(party.species_list(), &expected_species[..]);
        for (i, &dex) in expect_dexes.iter().enumerate() {
            assert_eq!(party.mon(i).species(), DEX_TO_INDEX[dex]);
            assert_eq!(
                party.nickname(i),
                format!("MON{dex}"),
                "nickname follows mon"
            );
        }

        // Sentinel after 4 entries; vacated trailing slot zero-filled.
        let raw = party_bytes(&save);
        assert_eq!(raw[0], 4);
        assert_eq!(raw[1 + 4], 0xFF);
        assert_eq!(raw[1 + 5], 0, "species bytes past sentinel zeroed");
        let mon5 = 0x008 + 4 * offsets::PARTY_MON_SIZE;
        assert!(
            raw[mon5..mon5 + offsets::PARTY_MON_SIZE]
                .iter()
                .all(|&b| b == 0),
            "vacated mon slot zero-filled"
        );
        let ot5 = 0x110 + 4 * offsets::NAME_LEN;
        assert!(raw[ot5..ot5 + offsets::NAME_LEN].iter().all(|&b| b == 0));
        let nick5 = 0x152 + 4 * offsets::NAME_LEN;
        assert!(raw[nick5..nick5 + offsets::NAME_LEN]
            .iter()
            .all(|&b| b == 0));
    }
}

#[test]
fn remove_only_mon_leaves_a_clean_empty_party() {
    let mut save = blank_save();
    save.party_mut()
        .add(&make_mon(7, 12), "GARY", "SQUIRT")
        .expect("party has room");
    save.party_mut().remove(0);
    assert!(save.party().is_empty());
    let raw = party_bytes(&save);
    assert_eq!(raw[0], 0);
    assert_eq!(raw[1], 0xFF);
    assert!(raw[2..].iter().all(|&b| b == 0));
}

#[test]
fn swap_keeps_names_attached_to_the_right_mon() {
    let mut save = blank_save();
    save.party_mut()
        .add(&make_mon(6, 60), "RED", "ZARD")
        .expect("room");
    save.party_mut()
        .add(&make_mon(9, 55), "BLUE", "TOISE")
        .expect("room");
    save.party_mut()
        .add(&make_mon(3, 50), "GREEN", "SAUR")
        .expect("room");

    save.party_mut().swap(0, 2);

    let party = save.party();
    assert_eq!(party.len(), 3);
    assert_eq!(
        party.species_list(),
        &[DEX_TO_INDEX[3], DEX_TO_INDEX[9], DEX_TO_INDEX[6]]
    );
    assert_eq!(party.mon(0).species(), DEX_TO_INDEX[3]);
    assert_eq!(party.ot_name(0), "GREEN");
    assert_eq!(party.nickname(0), "SAUR");
    assert_eq!(party.mon(1).species(), DEX_TO_INDEX[9]);
    assert_eq!(party.ot_name(1), "BLUE");
    assert_eq!(party.nickname(1), "TOISE");
    assert_eq!(party.mon(2).species(), DEX_TO_INDEX[6]);
    assert_eq!(party.ot_name(2), "RED");
    assert_eq!(party.nickname(2), "ZARD");
    assert_eq!(party.mon(0).level(), 50);
    assert_eq!(party.mon(2).level(), 60);
}

#[test]
fn swap_with_self_is_a_no_op() {
    let mut save = blank_save();
    save.party_mut()
        .add(&make_mon(4, 20), "RED", "CHAR")
        .expect("room");
    let before = save.as_bytes().to_vec();
    save.party_mut().swap(0, 0);
    assert_eq!(save.as_bytes(), &before[..]);
}

#[test]
fn set_names_and_species_stay_in_sync() {
    let mut save = blank_save();
    save.party_mut()
        .add(&make_mon(1, 5), "OLD", "OLDNICK")
        .expect("room");
    {
        let mut party = save.party_mut();
        party.set_ot_name(0, "NEWOT").expect("encodes");
        party.set_nickname(0, "NEWNICK").expect("encodes");
        party.set_species(0, DEX_TO_INDEX[151]);
    }
    let party = save.party();
    assert_eq!(party.ot_name(0), "NEWOT");
    assert_eq!(party.nickname(0), "NEWNICK");
    assert_eq!(party.species_list(), &[DEX_TO_INDEX[151]]);
    assert_eq!(party.mon(0).species(), DEX_TO_INDEX[151]);
}

#[test]
fn mon_mut_edits_are_visible_through_the_view() {
    let mut save = blank_save();
    save.party_mut()
        .add(&make_mon(25, 10), "RED", "PIKA")
        .expect("room");
    save.party_mut().mon_mut(0).set_level_coherent(88);
    let party = save.party();
    assert_eq!(party.mon(0).level(), 88);
    assert_eq!(party.mon(0).current_hp(), party.mon(0).max_hp());
}

#[test]
fn party_ops_touch_only_the_party_region() {
    // Run every op against a patterned background and diff whole buffers.
    let mut base = vec![0u8; 0x8000];
    for (i, b) in base.iter_mut().enumerate() {
        *b = (i % 251) as u8;
    }
    let mut save = SaveFile::from_bytes(base.clone()).expect("length is valid");

    save.party_mut().clear();
    let mut party = save.party_mut();
    party.add(&make_mon(1, 5), "RED", "A").expect("room");
    party.add(&make_mon(4, 5), "RED", "B").expect("room");
    party.add(&make_mon(7, 5), "RED", "C").expect("room");
    party.swap(0, 2);
    party.set_nickname(1, "D").expect("encodes");
    party.set_ot_name(1, "BLUE").expect("encodes");
    party.set_species(1, DEX_TO_INDEX[150]);
    party.mon_mut(1).set_level_coherent(99);
    party.remove(0);

    let after = save.as_bytes();
    assert_eq!(&after[..offsets::PARTY], &base[..offsets::PARTY]);
    assert_eq!(&after[PARTY_END..], &base[PARTY_END..]);
    assert_ne!(
        &after[offsets::PARTY..PARTY_END],
        &base[offsets::PARTY..PARTY_END]
    );
}

#[test]
fn edits_mark_the_save_edited_so_checksums_get_fixed() {
    let mut save = blank_save();
    assert!(save.is_edited());
    save.party_mut()
        .add(&make_mon(1, 5), "RED", "BULBY")
        .expect("room");
    let out = save.to_bytes();
    let reloaded = SaveFile::from_bytes(out).expect("length is valid");
    assert!(
        reloaded
            .diagnostics()
            .iter()
            .all(|d| d.code != "W-CHECKSUM-MAIN"),
        "main checksum repaired on serialize"
    );
}

#[test]
fn party_view_and_party_mut_agree_on_is_empty() {
    let mut save = blank_save();
    assert!(save.party().is_empty(), "PartyView::is_empty on empty save");
    assert!(
        save.party_mut().is_empty(),
        "PartyMut::is_empty on empty save"
    );

    save.party_mut()
        .add(&make_mon(25, 42), "ASH", "SPARKY")
        .expect("room");
    assert!(!save.party().is_empty(), "PartyView::is_empty after an add");
    assert!(
        !save.party_mut().is_empty(),
        "PartyMut::is_empty after an add"
    );
}

#[test]
fn corrupt_count_byte_is_clamped_on_read() {
    let mut save = blank_save();
    save.party_mut()
        .add(&make_mon(1, 5), "RED", "BULBY")
        .expect("room");
    // Corrupt the count byte and reload.
    let mut bytes = save.to_bytes();
    bytes[offsets::PARTY] = 200;
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    assert_eq!(save.party().len(), offsets::PARTY_CAPACITY);
}
