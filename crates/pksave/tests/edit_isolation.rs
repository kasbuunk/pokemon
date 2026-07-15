//! P2: every setter touches only its own bytes.
//!
//! For each registered edit we start from a fresh `new_empty(RedBlue)`
//! save, apply the edit and assert that:
//!
//! 1. the raw buffer (`as_bytes`) changed only inside the edit's allowed
//!    span, and
//! 2. `to_bytes()` differs from the pristine serialization only inside
//!    the allowed span plus the main checksum byte (all these fields
//!    live in the main checksummed region).

use core::ops::Range;

use pksave::gen1::items::{BAG_LIST, PC_LIST};
use pksave::gen1::offsets;
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::gen1::trainer::{Badge, PlayTime, TextSpeed};

type Edit = (&'static str, Box<dyn Fn(&mut SaveFile)>, Range<usize>);

fn registry() -> Vec<Edit> {
    let span = |start: usize, len: usize| start..start + len;
    vec![
        (
            "set_player_name",
            Box::new(|s: &mut SaveFile| s.set_player_name("BLAINE").expect("encodable")) as _,
            span(offsets::PLAYER_NAME, offsets::NAME_LEN),
        ),
        (
            "set_rival_name",
            Box::new(|s: &mut SaveFile| s.set_rival_name("GARY").expect("encodable")) as _,
            span(offsets::RIVAL_NAME, offsets::NAME_LEN),
        ),
        (
            "set_money",
            Box::new(|s: &mut SaveFile| s.set_money(987_654).expect("in range")) as _,
            span(offsets::MONEY, 3),
        ),
        (
            "set_coins",
            Box::new(|s: &mut SaveFile| s.set_coins(1234).expect("in range")) as _,
            span(offsets::COINS, 2),
        ),
        (
            "set_player_id",
            Box::new(|s: &mut SaveFile| s.set_player_id(0x5A5A)) as _,
            span(offsets::PLAYER_ID, 2),
        ),
        (
            "set_badges",
            Box::new(|s: &mut SaveFile| s.set_badges(0xFF)) as _,
            span(offsets::BADGES, 1),
        ),
        (
            "set_badge",
            Box::new(|s: &mut SaveFile| s.set_badge(Badge::Earth, true)) as _,
            span(offsets::BADGES, 1),
        ),
        (
            "set_options",
            Box::new(|s: &mut SaveFile| s.set_options(0xC5)) as _,
            span(offsets::OPTIONS, 1),
        ),
        (
            "set_text_speed",
            Box::new(|s: &mut SaveFile| s.set_text_speed(TextSpeed::Slow)) as _,
            span(offsets::OPTIONS, 1),
        ),
        (
            "set_battle_animations_off",
            Box::new(|s: &mut SaveFile| s.set_battle_animations_off(true)) as _,
            span(offsets::OPTIONS, 1),
        ),
        (
            "set_battle_style_set",
            Box::new(|s: &mut SaveFile| s.set_battle_style_set(true)) as _,
            span(offsets::OPTIONS, 1),
        ),
        (
            "set_pikachu_friendship",
            Box::new(|s: &mut SaveFile| s.set_pikachu_friendship(200)) as _,
            span(offsets::PIKACHU_FRIENDSHIP, 1),
        ),
        (
            "set_play_time",
            Box::new(|s: &mut SaveFile| {
                s.set_play_time(PlayTime {
                    hours: 12,
                    maxed: true,
                    minutes: 34,
                    seconds: 56,
                    frames: 58,
                })
            }) as _,
            span(offsets::PLAY_TIME_HOURS, 5),
        ),
        (
            "set_player_starter",
            Box::new(|s: &mut SaveFile| s.set_player_starter(0xB1)) as _,
            span(offsets::PLAYER_STARTER, 1),
        ),
        (
            "set_rival_starter",
            Box::new(|s: &mut SaveFile| s.set_rival_starter(0x99)) as _,
            span(offsets::RIVAL_STARTER, 1),
        ),
        (
            "set_safari_steps",
            Box::new(|s: &mut SaveFile| s.set_safari_steps(502)) as _,
            span(offsets::SAFARI_STEPS, 2),
        ),
        (
            "set_dex_owned",
            Box::new(|s: &mut SaveFile| s.set_dex_owned(151, true)) as _,
            span(offsets::POKEDEX_OWNED, offsets::POKEDEX_LEN),
        ),
        (
            "set_dex_seen",
            Box::new(|s: &mut SaveFile| s.set_dex_seen(1, true)) as _,
            span(offsets::POKEDEX_SEEN, offsets::POKEDEX_LEN),
        ),
        (
            "complete_dex",
            Box::new(|s: &mut SaveFile| s.complete_dex()) as _,
            offsets::POKEDEX_OWNED..offsets::POKEDEX_SEEN + offsets::POKEDEX_LEN,
        ),
        (
            "bag add",
            Box::new(|s: &mut SaveFile| s.bag_items_mut().add(0x14, 3).expect("has room")) as _,
            BAG_LIST.region(),
        ),
        (
            "bag add+remove+set_qty+set_id+swap",
            Box::new(|s: &mut SaveFile| {
                let mut bag = s.bag_items_mut();
                bag.add(0x14, 3).expect("has room");
                bag.add(0x01, 1).expect("has room");
                bag.add(0x06, 1).expect("has room");
                bag.set_qty(0, 99);
                bag.set_id(1, 0x02);
                bag.swap(0, 2);
                bag.remove(1);
            }) as _,
            BAG_LIST.region(),
        ),
        (
            "pc add+remove",
            Box::new(|s: &mut SaveFile| {
                let mut pc = s.pc_items_mut();
                pc.add(0x0B, 12).expect("has room");
                pc.add(0x0C, 1).expect("has room");
                pc.remove(0);
            }) as _,
            PC_LIST.region(),
        ),
    ]
}

/// Every changed range must lie inside one of the allowed ranges.
fn assert_within(what: &str, changed: &[Range<usize>], allowed: &[Range<usize>]) {
    for r in changed {
        assert!(
            allowed.iter().any(|a| a.start <= r.start && r.end <= a.end),
            "{what}: changed bytes 0x{:04X}..0x{:04X} outside allowed {allowed:X?}",
            r.start,
            r.end
        );
    }
}

#[test]
fn raw_buffer_changes_stay_inside_each_setters_span() {
    for (name, edit, allowed) in registry() {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        let before = save.as_bytes().to_vec();
        edit(&mut save);
        let changed = changed_ranges(&before, save.as_bytes());
        assert!(
            !changed.is_empty(),
            "{name}: registry edit must actually change bytes"
        );
        assert_within(name, &changed, std::slice::from_ref(&allowed));
    }
}

#[test]
fn serialized_changes_add_only_the_main_checksum() {
    let pristine = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
    for (name, edit, allowed) in registry() {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        edit(&mut save);
        let changed = changed_ranges(&pristine, &save.to_bytes());
        let allowed = [allowed, offsets::MAIN_CHECKSUM..offsets::MAIN_CHECKSUM + 1];
        assert_within(name, &changed, &allowed);
        // All these fields live in the main checksummed region, so the
        // main checksum itself must have moved.
        assert!(
            changed
                .iter()
                .any(|r| r.start <= offsets::MAIN_CHECKSUM && offsets::MAIN_CHECKSUM < r.end),
            "{name}: main checksum should be recomputed"
        );
    }
}

#[test]
fn spans_all_lie_in_the_main_checksummed_region() {
    // Sanity check on the registry itself: the second test's reasoning
    // (only MAIN_CHECKSUM moves besides the field) relies on it.
    for (name, _, allowed) in registry() {
        assert!(
            offsets::CHECKSUM_REGION_START <= allowed.start
                && allowed.end <= offsets::CHECKSUM_REGION_END + 1,
            "{name}: span {allowed:X?} outside the main region"
        );
    }
}
