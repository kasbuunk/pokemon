//! Coverage-guided sweep of the no-panic-from-file-contents guarantee:
//! parse arbitrary bytes and walk every view, accessor, diagnostic and
//! file-value-driven mutating path. The walk itself lives in
//! `pksave::fuzz_support` so it always compiles against the current API.
#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    pksave::fuzz_support::exercise(data);
});
