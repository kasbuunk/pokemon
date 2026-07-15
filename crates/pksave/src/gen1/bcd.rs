//! Big-endian packed binary-coded decimal, as used for money (3 bytes,
//! max 999999) and casino coins (2 bytes, max 9999).
//!
//! Each byte holds two decimal digits, high nibble first; the first byte
//! is the most significant. Nibbles above 9 are invalid BCD — the strict
//! [`decode`] rejects them, while [`decode_lossy`] clamps each invalid
//! nibble to 9 (documented policy: a garbage nibble reads as the largest
//! digit rather than wrapping into a bogus small value).

use thiserror::Error;

/// BCD conversion failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum BcdError {
    /// A nibble was not a decimal digit. `nibble` is the offending 4-bit
    /// value (0xA..=0xF); `byte_index` the byte it came from.
    #[error("invalid BCD nibble 0x{nibble:X} in byte {byte_index}")]
    InvalidNibble {
        /// Index of the offending byte within the decoded field.
        byte_index: usize,
        /// The offending 4-bit value (0xA..=0xF).
        nibble: u8,
    },
    /// The value needs more decimal digits than the output (or `u32`)
    /// can hold.
    #[error("value needs more decimal digits than the field can hold")]
    Overflow,
}

/// Strictly decode big-endian packed BCD.
pub fn decode(bytes: &[u8]) -> Result<u32, BcdError> {
    let mut value: u64 = 0;
    for (byte_index, &b) in bytes.iter().enumerate() {
        for nibble in [b >> 4, b & 0x0F] {
            if nibble > 9 {
                return Err(BcdError::InvalidNibble { byte_index, nibble });
            }
            value = value * 10 + u64::from(nibble);
        }
        if value > u64::from(u32::MAX) {
            return Err(BcdError::Overflow);
        }
    }
    Ok(value as u32)
}

/// Lossy decode: each invalid nibble is clamped to 9, and the total
/// saturates at `u32::MAX`. Never fails.
pub fn decode_lossy(bytes: &[u8]) -> u32 {
    let mut value: u64 = 0;
    for &b in bytes {
        for nibble in [b >> 4, b & 0x0F] {
            value = value * 10 + u64::from(nibble.min(9));
        }
        value = value.min(u64::from(u32::MAX));
    }
    value as u32
}

/// Encode `value` as exactly `out_len` bytes of big-endian packed BCD,
/// zero-padded on the left. Errors with [`BcdError::Overflow`] if `value`
/// has more than `2 * out_len` decimal digits.
pub fn encode(value: u32, out_len: usize) -> Result<Vec<u8>, BcdError> {
    let mut out = vec![0u8; out_len];
    let mut v = value;
    for byte in out.iter_mut().rev() {
        let lo = (v % 10) as u8;
        v /= 10;
        let hi = (v % 10) as u8;
        v /= 10;
        *byte = (hi << 4) | lo;
    }
    if v != 0 {
        return Err(BcdError::Overflow);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_known_values() {
        assert_eq!(decode(&[0x00, 0x00, 0x00]).expect("valid"), 0);
        assert_eq!(decode(&[0x01, 0x23, 0x45]).expect("valid"), 12345);
        assert_eq!(decode(&[0x99, 0x99, 0x99]).expect("valid"), 999_999);
        assert_eq!(decode(&[0x09, 0x99, 0x99]).expect("valid"), 99_999);
        assert_eq!(decode(&[0x42]).expect("valid"), 42);
        assert_eq!(decode(&[]).expect("valid"), 0);
    }

    #[test]
    fn rejects_invalid_nibbles() {
        assert_eq!(
            decode(&[0xFA]).expect_err("invalid nibble"),
            BcdError::InvalidNibble {
                byte_index: 0,
                nibble: 0xF
            }
        );
        assert_eq!(
            decode(&[0x12, 0x3A]).expect_err("invalid nibble"),
            BcdError::InvalidNibble {
                byte_index: 1,
                nibble: 0xA
            }
        );
    }

    #[test]
    fn decode_lossy_clamps_invalid_nibbles_to_9() {
        assert_eq!(decode_lossy(&[0xFA]), 99);
        assert_eq!(decode_lossy(&[0x1A, 0x23]), 1923);
        assert_eq!(decode_lossy(&[0x01, 0x23, 0x45]), 12345);
    }

    #[test]
    fn encodes_known_values() {
        assert_eq!(encode(0, 3).expect("fits"), vec![0x00, 0x00, 0x00]);
        assert_eq!(encode(999_999, 3).expect("fits"), vec![0x99, 0x99, 0x99]);
        assert_eq!(encode(12345, 3).expect("fits"), vec![0x01, 0x23, 0x45]);
        assert_eq!(encode(9999, 2).expect("fits"), vec![0x99, 0x99]);
        assert_eq!(encode(0, 0).expect("fits"), Vec::<u8>::new());
    }

    #[test]
    fn encode_rejects_overflow() {
        assert_eq!(
            encode(1_000_000, 3).expect_err("overflows"),
            BcdError::Overflow
        );
        assert_eq!(encode(100, 1).expect_err("overflows"), BcdError::Overflow);
        assert_eq!(encode(1, 0).expect_err("overflows"), BcdError::Overflow);
    }

    #[test]
    fn decode_rejects_u32_overflow() {
        // 10 digits of 9s exceed u32::MAX (4294967295).
        assert_eq!(
            decode(&[0x99, 0x99, 0x99, 0x99, 0x99]).expect_err("overflows"),
            BcdError::Overflow
        );
        // But a 5-byte value within range is fine.
        assert_eq!(
            decode(&[0x42, 0x94, 0x96, 0x72, 0x95]).expect("valid"),
            4_294_967_295
        );
    }
}
