//! P1: an untouched save round-trips byte-identically, whatever its
//! content (including garbage checksums) and whatever its length class
//! (exact 32 KiB, RTC-footer tail, 64 KiB pad).

use pksave::gen1::save::SaveFile;
use proptest::prelude::*;

fn roundtrips(bytes: Vec<u8>) -> Result<(), TestCaseError> {
    let save = SaveFile::from_bytes(bytes.clone()).expect("length is valid");
    prop_assert_eq!(save.to_bytes(), bytes);
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]

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
