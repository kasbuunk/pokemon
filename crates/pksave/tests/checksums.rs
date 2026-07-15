//! P5: `fix_all` produces a buffer that verifies clean, and running it
//! again changes nothing (idempotence), for arbitrary SRAM images.

use pksave::gen1::checksum::{fix_all, verify};
use proptest::prelude::*;

/// Proptest case count: the `PROPTEST_CASES` env var when set (e.g. to
/// raise coverage in CI), otherwise `default`.
fn env_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: env_cases(32), ..ProptestConfig::default() })]

    #[test]
    fn p5_fix_all_verifies_clean_and_is_idempotent(
        bytes in prop::collection::vec(any::<u8>(), 0x8000)
    ) {
        let mut buf = bytes;
        fix_all(&mut buf);
        prop_assert!(verify(&buf).is_empty());
        let snapshot = buf.clone();
        fix_all(&mut buf);
        prop_assert_eq!(buf, snapshot);
    }
}
