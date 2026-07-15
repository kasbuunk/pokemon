//! Trainer-level fields: names, money, coins, id, badges, options, play
//! time, starters, safari steps.
//!
//! All accessors are inherent methods on [`SaveFile`]. Setters write only
//! the bytes of the field being changed and mark the file edited (via
//! `buf_mut`), so checksums are recomputed on the next
//! [`SaveFile::to_bytes`]. Fallible setters validate *before* touching
//! the buffer.

use super::offsets::{self, NAME_LEN};
use super::save::SaveFile;
use super::{bcd, text};

/// The eight Kanto badges, in `wObtainedBadges` bit order
/// (bit 0 = Boulder … bit 7 = Earth).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Badge {
    Boulder,
    Cascade,
    Thunder,
    Rainbow,
    Soul,
    Marsh,
    Volcano,
    Earth,
}

impl Badge {
    /// All badges, ascending bit order.
    pub const ALL: [Badge; 8] = [
        Badge::Boulder,
        Badge::Cascade,
        Badge::Thunder,
        Badge::Rainbow,
        Badge::Soul,
        Badge::Marsh,
        Badge::Volcano,
        Badge::Earth,
    ];

    /// Bit position in the badges byte.
    pub fn bit(self) -> u8 {
        self as u8
    }
}

/// Text speed nibble of the options byte (frames per letter).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextSpeed {
    Fast = 1,
    Medium = 3,
    Slow = 5,
}

/// Play time as stored: five separate bytes. `maxed` is the
/// `wPlayTimeMaxed` flag (set once the clock hits 255:59:59).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayTime {
    pub hours: u8,
    pub maxed: bool,
    /// 0-59; clamped on write.
    pub minutes: u8,
    /// 0-59; clamped on write.
    pub seconds: u8,
    /// 0-59 (the game counts 60 frames per second); clamped on write.
    pub frames: u8,
}

/// Maximum money the game can hold (3 bytes of BCD).
pub const MAX_MONEY: u32 = 999_999;
/// Maximum casino coins (2 bytes of BCD).
pub const MAX_COINS: u32 = 9_999;

impl SaveFile {
    // ---- names ----

    /// Player name, decoded with the Gen 1 charset (damaged bytes read
    /// as U+FFFD).
    pub fn player_name(&self) -> String {
        text::decode(&self.buf()[offsets::PLAYER_NAME..offsets::PLAYER_NAME + NAME_LEN])
    }

    /// Set the player name (max 10 characters + terminator). The buffer
    /// is untouched on error.
    pub fn set_player_name(&mut self, name: &str) -> Result<(), text::TextError> {
        let encoded = text::encode(name, NAME_LEN)?;
        self.buf_mut()[offsets::PLAYER_NAME..offsets::PLAYER_NAME + NAME_LEN]
            .copy_from_slice(&encoded);
        Ok(())
    }

    /// Rival name.
    pub fn rival_name(&self) -> String {
        text::decode(&self.buf()[offsets::RIVAL_NAME..offsets::RIVAL_NAME + NAME_LEN])
    }

    /// Set the rival name. The buffer is untouched on error.
    pub fn set_rival_name(&mut self, name: &str) -> Result<(), text::TextError> {
        let encoded = text::encode(name, NAME_LEN)?;
        self.buf_mut()[offsets::RIVAL_NAME..offsets::RIVAL_NAME + NAME_LEN]
            .copy_from_slice(&encoded);
        Ok(())
    }

    // ---- money & coins ----

    /// Money, strictly decoded. Errors if the stored bytes are not valid
    /// BCD (corrupt save); use [`SaveFile::money_lossy`] to read a
    /// best-effort value anyway.
    pub fn money(&self) -> Result<u32, bcd::BcdError> {
        bcd::decode(&self.buf()[offsets::MONEY..offsets::MONEY + 3])
    }

    /// Money via [`bcd::decode_lossy`]: invalid nibbles clamp to 9, so a
    /// corrupt field reads as the largest value it resembles instead of
    /// failing.
    pub fn money_lossy(&self) -> u32 {
        bcd::decode_lossy(&self.buf()[offsets::MONEY..offsets::MONEY + 3])
    }

    /// Set money. Errors with [`bcd::BcdError::Overflow`] above
    /// [`MAX_MONEY`] (999 999); the buffer is untouched on error.
    pub fn set_money(&mut self, value: u32) -> Result<(), bcd::BcdError> {
        let encoded = bcd::encode(value, 3)?;
        self.buf_mut()[offsets::MONEY..offsets::MONEY + 3].copy_from_slice(&encoded);
        Ok(())
    }

    /// Casino coins, strictly decoded (see [`SaveFile::money`]).
    pub fn coins(&self) -> Result<u32, bcd::BcdError> {
        bcd::decode(&self.buf()[offsets::COINS..offsets::COINS + 2])
    }

    /// Casino coins via lossy BCD decode (see [`SaveFile::money_lossy`]).
    pub fn coins_lossy(&self) -> u32 {
        bcd::decode_lossy(&self.buf()[offsets::COINS..offsets::COINS + 2])
    }

    /// Set casino coins. Errors with [`bcd::BcdError::Overflow`] above
    /// [`MAX_COINS`] (9 999); the buffer is untouched on error.
    pub fn set_coins(&mut self, value: u32) -> Result<(), bcd::BcdError> {
        let encoded = bcd::encode(value, 2)?;
        self.buf_mut()[offsets::COINS..offsets::COINS + 2].copy_from_slice(&encoded);
        Ok(())
    }

    // ---- trainer id ----

    /// Trainer ID (`wPlayerID`, big-endian).
    pub fn player_id(&self) -> u16 {
        u16::from_be_bytes([
            self.buf()[offsets::PLAYER_ID],
            self.buf()[offsets::PLAYER_ID + 1],
        ])
    }

    /// Set the trainer ID.
    pub fn set_player_id(&mut self, id: u16) {
        self.buf_mut()[offsets::PLAYER_ID..offsets::PLAYER_ID + 2]
            .copy_from_slice(&id.to_be_bytes());
    }

    // ---- badges ----

    /// The full badges bitfield (bit 0 = Boulder … bit 7 = Earth).
    pub fn badges(&self) -> u8 {
        self.buf()[offsets::BADGES]
    }

    /// Overwrite the full badges bitfield.
    pub fn set_badges(&mut self, badges: u8) {
        self.buf_mut()[offsets::BADGES] = badges;
    }

    /// Whether one badge is obtained.
    pub fn has_badge(&self, badge: Badge) -> bool {
        self.badges() & (1 << badge.bit()) != 0
    }

    /// Grant or revoke one badge.
    pub fn set_badge(&mut self, badge: Badge, obtained: bool) {
        let byte = &mut self.buf_mut()[offsets::BADGES];
        if obtained {
            *byte |= 1 << badge.bit();
        } else {
            *byte &= !(1 << badge.bit());
        }
    }

    // ---- options ----

    /// The raw options byte (`wOptions`).
    pub fn options(&self) -> u8 {
        self.buf()[offsets::OPTIONS]
    }

    /// Overwrite the raw options byte.
    pub fn set_options(&mut self, options: u8) {
        self.buf_mut()[offsets::OPTIONS] = options;
    }

    /// Text speed from the low nibble; `None` if it holds none of the
    /// three values the options menu can write (1/3/5).
    pub fn text_speed(&self) -> Option<TextSpeed> {
        match self.options() & 0x0F {
            1 => Some(TextSpeed::Fast),
            3 => Some(TextSpeed::Medium),
            5 => Some(TextSpeed::Slow),
            _ => None,
        }
    }

    /// Set the text speed, preserving the other option bits.
    pub fn set_text_speed(&mut self, speed: TextSpeed) {
        let byte = &mut self.buf_mut()[offsets::OPTIONS];
        *byte = (*byte & 0xF0) | speed as u8;
    }

    /// Bit 7: battle animations disabled.
    pub fn battle_animations_off(&self) -> bool {
        self.options() & 0x80 != 0
    }

    /// Set bit 7 (true = animations off).
    pub fn set_battle_animations_off(&mut self, off: bool) {
        let byte = &mut self.buf_mut()[offsets::OPTIONS];
        if off {
            *byte |= 0x80;
        } else {
            *byte &= !0x80;
        }
    }

    /// Bit 6: battle style Set (true) vs Shift (false).
    pub fn battle_style_set(&self) -> bool {
        self.options() & 0x40 != 0
    }

    /// Set bit 6 (true = Set style).
    pub fn set_battle_style_set(&mut self, set_style: bool) {
        let byte = &mut self.buf_mut()[offsets::OPTIONS];
        if set_style {
            *byte |= 0x40;
        } else {
            *byte &= !0x40;
        }
    }

    // ---- pikachu friendship (Yellow) ----

    /// Pikachu friendship byte (meaningful in Yellow only; an unused byte
    /// in Red/Blue).
    pub fn pikachu_friendship(&self) -> u8 {
        self.buf()[offsets::PIKACHU_FRIENDSHIP]
    }

    /// Set the Pikachu friendship byte.
    pub fn set_pikachu_friendship(&mut self, value: u8) {
        self.buf_mut()[offsets::PIKACHU_FRIENDSHIP] = value;
    }

    // ---- play time ----

    /// Play time (five raw bytes; see [`PlayTime`]).
    pub fn play_time(&self) -> PlayTime {
        let b = self.buf();
        PlayTime {
            hours: b[offsets::PLAY_TIME_HOURS],
            maxed: b[offsets::PLAY_TIME_MAXED] != 0,
            minutes: b[offsets::PLAY_TIME_MINUTES],
            seconds: b[offsets::PLAY_TIME_SECONDS],
            frames: b[offsets::PLAY_TIME_FRAMES],
        }
    }

    /// Set play time. Minutes, seconds and frames clamp to 0-59; the
    /// maxed flag is stored as 0/1.
    pub fn set_play_time(&mut self, t: PlayTime) {
        let b = self.buf_mut();
        b[offsets::PLAY_TIME_HOURS] = t.hours;
        b[offsets::PLAY_TIME_MAXED] = u8::from(t.maxed);
        b[offsets::PLAY_TIME_MINUTES] = t.minutes.min(59);
        b[offsets::PLAY_TIME_SECONDS] = t.seconds.min(59);
        b[offsets::PLAY_TIME_FRAMES] = t.frames.min(59);
    }

    // ---- starters ----

    /// Player's starter, as the raw internal species index.
    pub fn player_starter(&self) -> u8 {
        self.buf()[offsets::PLAYER_STARTER]
    }

    /// Set the player's starter (raw internal species index).
    pub fn set_player_starter(&mut self, species: u8) {
        self.buf_mut()[offsets::PLAYER_STARTER] = species;
    }

    /// Rival's starter, as the raw internal species index.
    pub fn rival_starter(&self) -> u8 {
        self.buf()[offsets::RIVAL_STARTER]
    }

    /// Set the rival's starter (raw internal species index).
    pub fn set_rival_starter(&mut self, species: u8) {
        self.buf_mut()[offsets::RIVAL_STARTER] = species;
    }

    // ---- safari steps ----

    /// Safari steps remaining. Stored **big-endian**: the game writes
    /// `HIGH(502)` to `wSafariSteps` and `LOW(502)` to `wSafariSteps + 1`
    /// (pokered `scripts/SafariZoneGate.asm`,
    /// `engine/events/hidden_events/safari_game.asm`).
    pub fn safari_steps(&self) -> u16 {
        u16::from_be_bytes([
            self.buf()[offsets::SAFARI_STEPS],
            self.buf()[offsets::SAFARI_STEPS + 1],
        ])
    }

    /// Set safari steps remaining (big-endian; see
    /// [`SaveFile::safari_steps`]).
    pub fn set_safari_steps(&mut self, steps: u16) {
        self.buf_mut()[offsets::SAFARI_STEPS..offsets::SAFARI_STEPS + 2]
            .copy_from_slice(&steps.to_be_bytes());
    }
}
