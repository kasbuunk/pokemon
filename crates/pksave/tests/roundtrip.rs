//! P1: an untouched save round-trips byte-identically, whatever its
//! content (including garbage checksums) and whatever its length class
//! (exact 32 KiB, RTC-footer tail, 64 KiB pad).

use pksave::gen1::save::SaveFile;
use proptest::prelude::*;

/// Proptest case count: the `PROPTEST_CASES` env var when set (e.g. to
/// raise coverage in CI), otherwise `default`.
fn env_cases(default: u32) -> u32 {
    std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn roundtrips(bytes: Vec<u8>) -> Result<(), TestCaseError> {
    let save = SaveFile::from_bytes(bytes.clone()).expect("length is valid");
    prop_assert_eq!(save.to_bytes(), bytes);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: env_cases(32), ..ProptestConfig::default() })]

    #[test]
    fn p1_untouched_roundtrip_exact_sram(bytes in prop::collection::vec(any::<u8>(), 0x8000)) {
        roundtrips(bytes)?;
    }

    #[test]
    fn p1_untouched_roundtrip_rtc_footer(bytes in prop::collection::vec(any::<u8>(), 0x8009)) {
        roundtrips(bytes)?;
    }

    #[test]
    fn p1_untouched_roundtrip_64k_pad(bytes in prop::collection::vec(any::<u8>(), 0x10000)) {
        roundtrips(bytes)?;
    }
}
