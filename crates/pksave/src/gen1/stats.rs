//! Gen 1 stat math: the stat formula, DV packing, PP bytes, and
//! experience growth curves.
//!
//! Formulas follow the pret/pokered disassembly (see `docs/FORMAT.md`).
//! Growth-curve coefficients come verbatim from pokered
//! `data/growth_rates.asm` (`GrowthRateTable`), indexed by the `GROWTH_*`
//! constants in `constants/pokemon_data_constants.asm`.

/// Integer ceiling square root: the smallest `r` with `r * r >= n`.
fn ceil_sqrt(n: u16) -> u32 {
    let n = u32::from(n);
    let mut r = 0u32;
    while r * r < n {
        r += 1;
    }
    r
}

/// The Gen 1 stat formula (all divisions floor):
///
/// ```text
/// E     = floor(min(255, ceil(sqrt(stat_exp))) / 4)
/// other = floor(((base + dv) * 2 + E) * level / 100) + 5
/// hp    = floor(((base + dv) * 2 + E) * level / 100) + level + 10
/// ```
pub fn calc_stat(base: u8, dv: u8, stat_exp: u16, level: u8, is_hp: bool) -> u16 {
    let e = ceil_sqrt(stat_exp).min(255) / 4;
    let core = ((u32::from(base) + u32::from(dv)) * 2 + e) * u32::from(level) / 100;
    let stat = if is_hp {
        core + u32::from(level) + 10
    } else {
        core + 5
    };
    // Max possible: ((255+15)*2 + 63) * 255 / 100 + 255 + 10 < 2000, fits.
    stat as u16
}

/// The four 4-bit determinant values ("DVs", Gen 1 IVs) stored in a mon
/// record. The HP DV is not stored; it derives from the low bit of each
/// (see [`Dvs::hp_dv`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Dvs {
    /// Attack DV (0-15), high nibble of the first DV byte.
    pub attack: u8,
    /// Defense DV (0-15), low nibble of the first DV byte.
    pub defense: u8,
    /// Speed DV (0-15), high nibble of the second DV byte.
    pub speed: u8,
    /// Special DV (0-15), low nibble of the second DV byte.
    pub special: u8,
}

impl Dvs {
    /// Pack into the two on-disk bytes:
    /// `[attack << 4 | defense, speed << 4 | special]`.
    /// Each field is masked to its low 4 bits.
    pub fn pack(self) -> [u8; 2] {
        [
            (self.attack & 0xF) << 4 | (self.defense & 0xF),
            (self.speed & 0xF) << 4 | (self.special & 0xF),
        ]
    }

    /// Inverse of [`Dvs::pack`].
    pub fn unpack(bytes: [u8; 2]) -> Dvs {
        Dvs {
            attack: bytes[0] >> 4,
            defense: bytes[0] & 0xF,
            speed: bytes[1] >> 4,
            special: bytes[1] & 0xF,
        }
    }

    /// The derived HP DV:
    /// `(atk&1)<<3 | (def&1)<<2 | (spd&1)<<1 | (spc&1)`.
    pub fn hp_dv(self) -> u8 {
        (self.attack & 1) << 3
            | (self.defense & 1) << 2
            | (self.speed & 1) << 1
            | (self.special & 1)
    }
}

/// Current PP from a PP byte (bits 0-5).
pub fn current_pp(byte: u8) -> u8 {
    byte & 0x3F
}

/// PP Ups applied, from a PP byte (bits 6-7).
pub fn pp_ups(byte: u8) -> u8 {
    byte >> 6
}

/// Compose a PP byte from current PP (masked to 6 bits) and PP Ups
/// (masked to 2 bits).
pub fn compose_pp(current: u8, ups: u8) -> u8 {
    (ups & 0x3) << 6 | (current & 0x3F)
}

/// `GrowthRateTable` rows from pokered `data/growth_rates.asm`, as
/// `(num, den, quad, lin, sub)` in
/// `exp(n) = floor(n^3 * num / den) + quad*n^2 + lin*n - sub`.
///
/// Row order matches the `GROWTH_*` constants. Gen 1 species only use
/// rows 0 (Medium Fast), 3 (Medium Slow), 4 (Fast) and 5 (Slow); rows
/// 1-2 exist in the ROM table but are unused.
const GROWTH_RATES: [(i64, i64, i64, i64, i64); 6] = [
    (1, 1, 0, 0, 0),       // 0 GROWTH_MEDIUM_FAST:  n^3
    (3, 4, 10, 0, 30),     // 1 GROWTH_SLIGHTLY_FAST (unused by any species)
    (3, 4, 20, 0, 70),     // 2 GROWTH_SLIGHTLY_SLOW (unused by any species)
    (6, 5, -15, 100, 140), // 3 GROWTH_MEDIUM_SLOW:  6n^3/5 - 15n^2 + 100n - 140
    (4, 5, 0, 0, 0),       // 4 GROWTH_FAST:         4n^3/5
    (5, 4, 0, 0, 0),       // 5 GROWTH_SLOW:         5n^3/4
];

/// Total experience at `level` for a growth rate (a `GROWTH_*` value —
/// the `growth_rate` field of [`crate::gen1::data::BaseStats`]).
///
/// Computed as the game's `CalcExperience` does — the cubed term is
/// floored *before* the polynomial sum — except that a negative result
/// is clamped to 0 instead of wrapping. The only negative case is Medium
/// Slow at level 1 (true value -54), where the game's 24-bit math wraps
/// to a garbage value it never consults: `CalcLevelFromExperience`
/// starts probing at level 2, and the formula is only evaluated for
/// levels >= 2 in practice. Clamping to 0 keeps
/// `level_for_exp(exp_for_level(g, 1)) == 1` intact.
///
/// An out-of-range growth rate falls back to Medium Fast.
pub fn exp_for_level(growth_rate: u8, level: u8) -> u32 {
    let (num, den, quad, lin, sub) = GROWTH_RATES
        .get(usize::from(growth_rate))
        .copied()
        .unwrap_or(GROWTH_RATES[0]);
    let n = i64::from(level);
    let cubed = n * n * n * num / den; // floor: all operands non-negative
    let total = cubed + quad * n * n + lin * n - sub;
    total.max(0) as u32
}

/// The level a mon with `exp` total experience sits at (1..=100): the
/// largest level whose [`exp_for_level`] does not exceed `exp`. Mirrors
/// the game's `CalcLevelFromExperience` loop, capped at 100.
pub fn level_for_exp(growth_rate: u8, exp: u32) -> u8 {
    for level in 2..=100u8 {
        if exp_for_level(growth_rate, level) > exp {
            return level - 1;
        }
    }
    100
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ceil_sqrt_vectors() {
        assert_eq!(ceil_sqrt(0), 0);
        assert_eq!(ceil_sqrt(1), 1);
        assert_eq!(ceil_sqrt(2), 2);
        assert_eq!(ceil_sqrt(4), 2);
        assert_eq!(ceil_sqrt(5), 3);
        assert_eq!(ceil_sqrt(25), 5);
        assert_eq!(ceil_sqrt(26), 6);
        assert_eq!(ceil_sqrt(65025), 255); // 255^2
        assert_eq!(ceil_sqrt(65026), 256);
        assert_eq!(ceil_sqrt(65535), 256); // then clamped to 255 in calc_stat
    }

    #[test]
    fn calc_stat_hand_vectors() {
        // E = floor(min(255, ceil_sqrt(65535)) / 4) = floor(255/4) = 63.
        // ((45+15)*2 + 63) * 100 / 100 + 5 = 183 + 5.
        assert_eq!(calc_stat(45, 15, 65535, 100, false), 188);
        // Same core, HP: 183 + 100 + 10.
        assert_eq!(calc_stat(45, 15, 65535, 100, true), 293);
        // Zero stat exp, low level: ((49+8)*2) * 5 / 100 = 5; +5.
        assert_eq!(calc_stat(49, 8, 0, 5, false), 10);
        // Level 5 HP, base 45, dv 7, no exp: (52*2)*5/100 = 5; +5+10.
        assert_eq!(calc_stat(45, 7, 0, 5, true), 20);
    }

    #[test]
    fn calc_stat_known_maximums() {
        // Level 100 Mewtwo: max Special 406, max HP 415 (well-known caps).
        assert_eq!(calc_stat(154, 15, 65535, 100, false), 406);
        assert_eq!(calc_stat(106, 15, 65535, 100, true), 415);
    }

    #[test]
    fn stat_exp_boundary_feeds_e_correctly() {
        // stat_exp 1 -> ceil_sqrt = 1 -> E = 0; stat_exp 16 -> 4 -> E = 1.
        assert_eq!(calc_stat(100, 0, 1, 100, false), 205);
        assert_eq!(calc_stat(100, 0, 16, 100, false), 206);
    }

    #[test]
    fn dvs_pack_unpack_and_hp_dv() {
        let dvs = Dvs {
            attack: 0xA,
            defense: 0xB,
            speed: 0xC,
            special: 0xD,
        };
        assert_eq!(dvs.pack(), [0xAB, 0xCD]);
        assert_eq!(Dvs::unpack([0xAB, 0xCD]), dvs);
        // atk even, def odd, spd even, spc odd -> 0b0101.
        assert_eq!(dvs.hp_dv(), 0b0101);
        assert_eq!(
            Dvs {
                attack: 15,
                defense: 15,
                speed: 15,
                special: 15
            }
            .hp_dv(),
            15
        );
        assert_eq!(Dvs::default().hp_dv(), 0);
        // Pack masks out-of-range values to their low nibble.
        assert_eq!(
            Dvs {
                attack: 0x1F,
                defense: 0xFF,
                speed: 0x10,
                special: 0x21
            }
            .pack(),
            [0xFF, 0x01]
        );
    }

    #[test]
    fn pp_byte_helpers() {
        assert_eq!(current_pp(0xC5), 5);
        assert_eq!(pp_ups(0xC5), 3);
        assert_eq!(compose_pp(5, 3), 0xC5);
        assert_eq!(current_pp(0x3F), 63);
        assert_eq!(pp_ups(0x3F), 0);
        assert_eq!(compose_pp(63, 0), 0x3F);
        // Out-of-range inputs are masked.
        assert_eq!(compose_pp(0xFF, 0xFF), 0xFF);
        assert_eq!(compose_pp(64, 4), 0x00);
    }

    #[test]
    fn exp_curve_hand_vectors() {
        // Medium Fast: n^3.
        assert_eq!(exp_for_level(0, 1), 1);
        assert_eq!(exp_for_level(0, 50), 125_000);
        assert_eq!(exp_for_level(0, 100), 1_000_000);
        // Medium Slow: 6n^3/5 - 15n^2 + 100n - 140 (cubed term floored).
        assert_eq!(exp_for_level(3, 1), 0); // true value -54, clamped
        assert_eq!(exp_for_level(3, 2), 9); // floor(48/5) - 60 + 200 - 140
        assert_eq!(exp_for_level(3, 5), 135); // fresh starter value
        assert_eq!(exp_for_level(3, 100), 1_059_860);
        // Fast: 4n^3/5.
        assert_eq!(exp_for_level(4, 1), 0);
        assert_eq!(exp_for_level(4, 50), 100_000);
        assert_eq!(exp_for_level(4, 100), 800_000);
        // Slow: 5n^3/4.
        assert_eq!(exp_for_level(5, 1), 1);
        assert_eq!(exp_for_level(5, 100), 1_250_000);
    }

    #[test]
    fn unknown_growth_rate_falls_back_to_medium_fast() {
        assert_eq!(exp_for_level(200, 10), 1000);
    }

    #[test]
    fn level_for_exp_boundaries() {
        assert_eq!(level_for_exp(0, 0), 1);
        assert_eq!(level_for_exp(0, 7), 1);
        assert_eq!(level_for_exp(0, 8), 2);
        assert_eq!(level_for_exp(0, 999_999), 99);
        assert_eq!(level_for_exp(0, 1_000_000), 100);
        assert_eq!(level_for_exp(0, u32::MAX), 100);
        assert_eq!(level_for_exp(3, 0), 1);
        assert_eq!(level_for_exp(3, 8), 1);
        assert_eq!(level_for_exp(3, 9), 2);
    }
}
