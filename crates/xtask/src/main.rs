//! Development tasks: `cargo xtask <command>`.
//!
//! - `gen-tables --pokered <dir>`: regenerate static data tables in
//!   `crates/pksave/src/gen1/data/generated/` from a pokered checkout.
//! - `gen-offsets-check --sym <pokered.sym>`: verify `gen1::offsets` against
//!   a pokered symbol file.
//! - `make-e2e-fixtures --out <dir>`: write test save files built with the
//!   core crate for the PyBoy end-to-end suite.

mod pins;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pins") => {
            println!("pokered={}", pins::POKERED_SHA);
            println!("rom_sha1={}", pins::POKERED_ROM_SHA1);
            println!("rgbds={}", pins::RGBDS_VERSION);
        }
        Some(cmd) => {
            eprintln!("unknown or not yet implemented command: {cmd}");
            std::process::exit(2);
        }
        None => {
            eprintln!("usage: cargo xtask <pins|gen-tables|gen-offsets-check|make-e2e-fixtures>");
            std::process::exit(2);
        }
    }
}
