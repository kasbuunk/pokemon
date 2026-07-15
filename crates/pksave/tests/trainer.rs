//! M4c: trainer field accessors on [`SaveFile`].

use pksave::gen1::bcd::BcdError;
use pksave::gen1::save::{GameVariant, SaveFile};
use pksave::gen1::text::TextError;
use pksave::gen1::trainer::{Badge, PlayTime, TextSpeed};

fn blank() -> SaveFile {
    SaveFile::new_empty(GameVariant::RedBlue)
}

// Public re-derivations of the offsets used for raw-byte assertions
// (the offsets module is public; using it keeps magic numbers out).
use pksave::gen1::offsets;

// ---- names ----

#[test]
fn player_name_defaults_to_red_and_round_trips() {
    let mut save = blank();
    assert_eq!(save.player_name(), "RED");
    save.set_player_name("ASH").expect("encodable");
    assert_eq!(save.player_name(), "ASH");
    assert!(save.is_edited());
    // Raw bytes go through the real charset: A=0x80, S=0x92, H=0x87.
    let b = save.as_bytes();
    assert_eq!(
        &b[offsets::PLAYER_NAME..offsets::PLAYER_NAME + 4],
        &[0x80, 0x92, 0x87, 0x50]
    );
}

#[test]
fn rival_name_round_trips_with_ligatures() {
    let mut save = blank();
    assert_eq!(save.rival_name(), "BLUE");
    save.set_rival_name("GARY's").expect("encodable");
    assert_eq!(save.rival_name(), "GARY's");
    // 's is the single ligature byte 0xBD.
    assert_eq!(save.as_bytes()[offsets::RIVAL_NAME + 4], 0xBD);
}

#[test]
fn name_setters_reject_bad_input_without_touching_the_buffer() {
    let mut save = blank();
    let before = save.as_bytes().to_vec();
    assert_eq!(
        save.set_player_name("~~~"),
        Err(TextError::Unencodable('~'))
    );
    // 10 chars fit an 11-byte field, 11 do not.
    assert!(save.set_player_name("ABCDEFGHIJ").is_ok());
    save.set_player_name("RED").expect("encodable");
    assert!(matches!(
        save.set_rival_name("ABCDEFGHIJK"),
        Err(TextError::TooLong { .. })
    ));
    assert_eq!(
        save.as_bytes()[offsets::RIVAL_NAME..],
        before[offsets::RIVAL_NAME..]
    );
}

// ---- money & coins (BCD) ----

#[test]
fn money_round_trips_bcd_edge_cases() {
    let mut save = blank();
    assert_eq!(save.money(), Ok(0));
    for value in [0u32, 1, 9, 10, 99999, 100000, 123456, 999_999] {
        save.set_money(value).expect("in range");
        assert_eq!(save.money(), Ok(value), "value {value}");
    }
    // 999999 is stored as three 0x99 bytes, big-endian BCD.
    assert_eq!(
        &save.as_bytes()[offsets::MONEY..offsets::MONEY + 3],
        &[0x99, 0x99, 0x99]
    );
    save.set_money(3005).expect("in range");
    assert_eq!(
        &save.as_bytes()[offsets::MONEY..offsets::MONEY + 3],
        &[0x00, 0x30, 0x05]
    );
}

#[test]
fn money_setter_rejects_over_max_and_leaves_bytes_alone() {
    let mut save = blank();
    save.set_money(1234).expect("in range");
    assert_eq!(save.set_money(1_000_000), Err(BcdError::Overflow));
    assert_eq!(save.money(), Ok(1234));
}

#[test]
fn money_getter_reports_invalid_bcd_and_lossy_reads_it() {
    let mut bytes = blank().to_bytes();
    bytes[offsets::MONEY] = 0xAB;
    let save = SaveFile::from_bytes(bytes).expect("length is valid");
    assert_eq!(
        save.money(),
        Err(BcdError::InvalidNibble {
            byte_index: 0,
            nibble: 0xA
        })
    );
    // decode_lossy clamps invalid nibbles to 9.
    assert_eq!(save.money_lossy(), 990000);
}

#[test]
fn coins_round_trip_and_reject_over_9999() {
    let mut save = blank();
    assert_eq!(save.coins(), Ok(0));
    save.set_coins(9999).expect("in range");
    assert_eq!(save.coins(), Ok(9999));
    assert_eq!(
        &save.as_bytes()[offsets::COINS..offsets::COINS + 2],
        &[0x99, 0x99]
    );
    assert_eq!(save.set_coins(10_000), Err(BcdError::Overflow));
    assert_eq!(save.coins(), Ok(9999));
    save.set_coins(50).expect("in range");
    assert_eq!(
        &save.as_bytes()[offsets::COINS..offsets::COINS + 2],
        &[0x00, 0x50]
    );
}

// ---- trainer id ----

#[test]
fn player_id_is_big_endian_u16() {
    let mut save = blank();
    assert_eq!(save.player_id(), 0);
    save.set_player_id(0xABCD);
    assert_eq!(save.player_id(), 0xABCD);
    assert_eq!(
        &save.as_bytes()[offsets::PLAYER_ID..offsets::PLAYER_ID + 2],
        &[0xAB, 0xCD]
    );
}

// ---- badges ----

#[test]
fn badge_bits_match_the_format_doc() {
    // bit 0 = Boulder … bit 7 = Earth.
    let expected: [(Badge, u8); 8] = [
        (Badge::Boulder, 0x01),
        (Badge::Cascade, 0x02),
        (Badge::Thunder, 0x04),
        (Badge::Rainbow, 0x08),
        (Badge::Soul, 0x10),
        (Badge::Marsh, 0x20),
        (Badge::Volcano, 0x40),
        (Badge::Earth, 0x80),
    ];
    for (badge, mask) in expected {
        let mut save = blank();
        assert!(!save.has_badge(badge));
        save.set_badge(badge, true);
        assert!(save.has_badge(badge));
        assert_eq!(save.badges(), mask, "{badge:?}");
        assert_eq!(save.as_bytes()[offsets::BADGES], mask);
        save.set_badge(badge, false);
        assert_eq!(save.badges(), 0);
    }
}

#[test]
fn badges_full_bitfield_round_trips() {
    let mut save = blank();
    save.set_badges(0xFF);
    for badge in Badge::ALL {
        assert!(save.has_badge(badge), "{badge:?}");
    }
    save.set_badge(Badge::Volcano, false);
    assert_eq!(save.badges(), 0xBF);
}

// ---- options ----

#[test]
fn options_typed_helpers() {
    let mut save = blank();
    assert_eq!(save.options(), 3);
    assert_eq!(save.text_speed(), Some(TextSpeed::Medium));
    assert!(!save.battle_animations_off());
    assert!(!save.battle_style_set());

    save.set_text_speed(TextSpeed::Fast);
    assert_eq!(save.text_speed(), Some(TextSpeed::Fast));
    save.set_battle_animations_off(true);
    save.set_battle_style_set(true);
    assert!(save.battle_animations_off());
    assert!(save.battle_style_set());
    assert_eq!(save.options(), 0x80 | 0x40 | 1);
    save.set_text_speed(TextSpeed::Slow);
    assert_eq!(save.options(), 0x80 | 0x40 | 5);

    // Unknown speed nibble reads as None; raw byte is still exposed.
    save.set_options(0x02);
    assert_eq!(save.text_speed(), None);
    assert_eq!(save.options(), 0x02);
}

// ---- pikachu friendship ----

#[test]
fn pikachu_friendship_round_trips() {
    let mut save = blank();
    assert_eq!(save.pikachu_friendship(), 0);
    save.set_pikachu_friendship(255);
    assert_eq!(save.pikachu_friendship(), 255);
    assert_eq!(save.as_bytes()[offsets::PIKACHU_FRIENDSHIP], 255);
}

// ---- play time ----

#[test]
fn play_time_round_trips_and_clamps() {
    let mut save = blank();
    assert_eq!(
        save.play_time(),
        PlayTime {
            hours: 0,
            maxed: false,
            minutes: 0,
            seconds: 0,
            frames: 0
        }
    );
    save.set_play_time(PlayTime {
        hours: 255,
        maxed: true,
        minutes: 59,
        seconds: 59,
        frames: 59,
    });
    assert_eq!(
        save.play_time(),
        PlayTime {
            hours: 255,
            maxed: true,
            minutes: 59,
            seconds: 59,
            frames: 59
        }
    );
    // Out-of-range minutes/seconds/frames clamp to 59.
    save.set_play_time(PlayTime {
        hours: 1,
        maxed: false,
        minutes: 200,
        seconds: 61,
        frames: 99,
    });
    let t = save.play_time();
    assert_eq!((t.minutes, t.seconds, t.frames), (59, 59, 59));
    assert!(!t.maxed);
    assert_eq!(save.as_bytes()[offsets::PLAY_TIME_MAXED], 0);
}

// ---- starters ----

#[test]
fn starters_round_trip_raw_internal_indexes() {
    let mut save = blank();
    assert_eq!(save.player_starter(), 0);
    assert_eq!(save.rival_starter(), 0);
    save.set_player_starter(0xB1); // Squirtle
    save.set_rival_starter(0x99); // Bulbasaur
    assert_eq!(save.player_starter(), 0xB1);
    assert_eq!(save.rival_starter(), 0x99);
    assert_eq!(save.as_bytes()[offsets::PLAYER_STARTER], 0xB1);
    assert_eq!(save.as_bytes()[offsets::RIVAL_STARTER], 0x99);
}

// ---- safari steps ----

#[test]
fn safari_steps_are_big_endian() {
    let mut save = blank();
    assert_eq!(save.safari_steps(), 0);
    // The game writes HIGH(502) to wSafariSteps and LOW(502) to +1
    // (pokered scripts/SafariZoneGate.asm), i.e. big-endian.
    save.set_safari_steps(502);
    assert_eq!(save.safari_steps(), 502);
    assert_eq!(
        &save.as_bytes()[offsets::SAFARI_STEPS..offsets::SAFARI_STEPS + 2],
        &[0x01, 0xF6]
    );
}

// ---- setters mark the file edited ----

#[test]
fn every_setter_marks_the_file_edited() {
    type Setter = (&'static str, Box<dyn Fn(&mut SaveFile)>);
    let setters: Vec<Setter> = vec![
        (
            "player_name",
            Box::new(|s| s.set_player_name("A").map(|_| ()).unwrap()),
        ),
        (
            "rival_name",
            Box::new(|s| s.set_rival_name("B").map(|_| ()).unwrap()),
        ),
        ("money", Box::new(|s| s.set_money(1).unwrap())),
        ("coins", Box::new(|s| s.set_coins(1).unwrap())),
        ("player_id", Box::new(|s| s.set_player_id(1))),
        ("badges", Box::new(|s| s.set_badges(1))),
        ("badge", Box::new(|s| s.set_badge(Badge::Earth, true))),
        ("options", Box::new(|s| s.set_options(1))),
        (
            "text_speed",
            Box::new(|s| s.set_text_speed(TextSpeed::Fast)),
        ),
        ("anims", Box::new(|s| s.set_battle_animations_off(true))),
        ("style", Box::new(|s| s.set_battle_style_set(true))),
        ("friendship", Box::new(|s| s.set_pikachu_friendship(1))),
        (
            "play_time",
            Box::new(|s| {
                s.set_play_time(PlayTime {
                    hours: 1,
                    maxed: false,
                    minutes: 2,
                    seconds: 3,
                    frames: 4,
                })
            }),
        ),
        ("player_starter", Box::new(|s| s.set_player_starter(1))),
        ("rival_starter", Box::new(|s| s.set_rival_starter(1))),
        ("safari_steps", Box::new(|s| s.set_safari_steps(1))),
    ];
    for (label, setter) in setters {
        let mut save = blank();
        assert!(!save.is_edited());
        setter(&mut save);
        assert!(save.is_edited(), "setter {label} must mark_edited");
    }
}
