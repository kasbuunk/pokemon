//! Pinned versions of external verification inputs. CI reads these values;
//! bumping them is a deliberate PR that re-runs table generation and e2e.

/// pret/pokered commit the data tables and offsets are generated from.
/// A build of this commit reproduces retail Pokémon Red byte-for-byte.
pub const POKERED_SHA: &str = "1e96034092686d006e863cace09e87273051a3d8";

/// SHA-1 of the ROM that commit builds (must match pokered's roms.sha1).
pub const POKERED_ROM_SHA1: &str = "ea9bcae617fdf159b045185467ae58b2e4a48b9a";

/// RGBDS release used to assemble it.
pub const RGBDS_VERSION: &str = "1.0.1";
