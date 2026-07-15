//! P5: `fix_all` produces a buffer that verifies clean, and running it
//! again changes nothing (idempotence), for arbitrary SRAM images.

use pksave::gen1::checksum::{fix_all, verify};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

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
