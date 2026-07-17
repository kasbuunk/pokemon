//! Shared harness for coverage-guided fuzzing (issue #34): parse an
//! arbitrary buffer and walk every public view, accessor and diagnostic,
//! then drive the mutating paths that consume file-derived values.
//!
//! Nothing here may panic for any input: this is the module-shaped form
//! of the crate's core guarantee that **no panic is reachable from file
//! contents** (a panic aborts the whole app on wasm — DoS via a crafted
//! save). The fuzz target (`fuzz/fuzz_targets/save_walk.rs`) is a thin
//! wrapper around [`exercise`]; keeping the walk inside the crate keeps
//! it compiling against every API change.

use crate::gen1::offsets;
use crate::gen1::pokemon::{MonView, PartyMon};
use crate::gen1::save::SaveFile;

/// Read every getter of one mon record.
fn walk_mon(mon: &dyn MonView) {
    let _ = mon.species();
    let _ = mon.current_hp();
    let _ = mon.box_level();
    let _ = mon.status();
    let _ = mon.sleep_turns();
    let _ = mon.is_poisoned();
    let _ = mon.is_burned();
    let _ = mon.is_frozen();
    let _ = mon.is_paralyzed();
    let _ = mon.types();
    let _ = mon.catch_rate();
    let _ = mon.moves();
    let _ = mon.ot_id();
    let _ = mon.exp();
    let _ = mon.stat_exps();
    let _ = mon.dvs();
    let _ = mon.pp();
    let _ = mon.level_from_exp();
}

/// Read every view/accessor/diagnostic of the save.
fn walk(save: &SaveFile) {
    let _ = save.diagnostics();
    let _ = save.game_label();

    let party = save.party();
    for i in 0..party.len() {
        let mon = party.mon(i);
        walk_mon(&mon);
        let _ = mon.level();
        let _ = mon.max_hp();
        let _ = mon.attack();
        let _ = mon.defense();
        let _ = mon.speed();
        let _ = mon.special();
        let _ = party.nickname(i);
        let _ = party.ot_name(i);
    }

    let _ = save.current_box_number();
    let _ = save.boxes_initialized();
    for n in 0..offsets::NUM_BOXES {
        let _ = save.box_is_live(n);
        let view = save.box_(n);
        for i in 0..view.len() {
            walk_mon(&view.mon(i));
            let _ = view.nickname(i);
            let _ = view.ot_name(i);
        }
    }

    if let Some(daycare) = save.daycare() {
        walk_mon(&daycare.mon());
        let _ = daycare.nickname();
        let _ = daycare.ot_name();
    }

    let _ = save.hof_team_count();
    for t in 0..offsets::HOF_TEAM_CAPACITY {
        let team = save.hof_team(t);
        for slot in 0..team.len() {
            if let Some(mon) = team.mon(slot) {
                let _ = mon.species();
                let _ = mon.level();
                let _ = mon.nickname();
            }
        }
    }

    for (bag, list) in [(true, save.bag_items()), (false, save.pc_items())] {
        let _ = bag;
        let _ = list.is_empty();
        for (id, qty) in list.iter() {
            let _ = (id, qty);
        }
    }

    let _ = save.player_name();
    let _ = save.rival_name();
    let _ = save.money();
    let _ = save.money_lossy();
    let _ = save.coins();
    let _ = save.coins_lossy();
    let _ = save.player_id();
    let _ = save.badges();
    let _ = save.options();
    let _ = save.text_speed();
    let _ = save.battle_animations_off();
    let _ = save.battle_style_set();

    for dex in 1..=151u8 {
        let _ = save.dex_owned(dex);
        let _ = save.dex_seen(dex);
    }
    let _ = save.owned_count();
    let _ = save.seen_count();

    let _ = save.cur_map();
    let _ = save.cur_map_name();
    let _ = save.x_coord();
    let _ = save.y_coord();
    let _ = save.x_block_coord();
    let _ = save.y_block_coord();
    let _ = save.last_map();
    let _ = save.tileset();
    let _ = save.map_view_pointer();

    for (_, _, value) in save.named_event_flags() {
        let _ = value;
    }
    let _ = save.game_progress_flags();
}

/// The fuzz entry point: parse `data`, walk everything, then exercise
/// the mutating paths driven by file-derived values on clones.
pub fn exercise(data: &[u8]) {
    let Ok(save) = SaveFile::from_bytes(data.to_vec()) else {
        // Too short: the only rejection; nothing further to check.
        return;
    };

    walk(&save);

    // Untouched files round-trip byte-identically — even hostile ones.
    assert_eq!(save.to_bytes(), data, "untouched round-trip");

    // fix_checksums must produce a buffer that reloads warning-free at
    // the checksum level and never panics on re-walk.
    let mut fixed = SaveFile::from_bytes(data.to_vec()).expect("length already accepted");
    fixed.fix_checksums();
    let reloaded = SaveFile::from_bytes(fixed.to_bytes()).expect("serialized length is valid");
    walk(&reloaded);

    // Transfers consume file-derived counts and species; drive one
    // deposit and one withdraw per box on a clone (failures are fine,
    // panics are not).
    let mut edited = SaveFile::from_bytes(data.to_vec()).expect("length already accepted");
    for n in 0..offsets::NUM_BOXES {
        let _ = edited.deposit(0, n);
        let _ = edited.withdraw(n, 0);
    }
    let mut party = edited.party_mut();
    for i in 0..party.as_view().len() {
        party.mon_mut(i).recalculate_stats();
    }
    walk(&edited);
    let _ = edited.to_bytes();
}
