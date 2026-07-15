//! Development tasks: `cargo xtask <command>`.
//!
//! - `gen-tables --pokered <dir>`: regenerate static data tables in
//!   `crates/pksave/src/gen1/data/generated/` from a pokered checkout.
//! - `gen-offsets-check --sym <pokered.sym>`: verify `gen1::offsets` against
//!   a pokered symbol file.
//! - `make-e2e-fixtures --out <dir>`: write test save files built with the
//!   core crate for the PyBoy end-to-end suite.

mod charmap;
mod e2e_fixtures;
mod gen_tables;
mod offsets_check;
mod pins;

use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("pins") => {
            println!("pokered={}", pins::POKERED_SHA);
            println!("rom_sha1={}", pins::POKERED_ROM_SHA1);
            println!("rgbds={}", pins::RGBDS_VERSION);
        }
        Some("gen-tables") => {
            let pokered = flag_value(&args[1..], "--pokered");
            gen_tables::run(&pokered);
        }
        Some("gen-offsets-check") => {
            let sym = flag_value(&args[1..], "--sym");
            std::process::exit(offsets_check::run(&sym));
        }
        Some("make-e2e-fixtures") => {
            let out = flag_value(&args[1..], "--out");
            e2e_fixtures::run(&out);
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

fn flag_value(args: &[String], flag: &str) -> PathBuf {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if arg == flag {
            if let Some(value) = iter.next() {
                return PathBuf::from(value);
            }
            break;
        }
    }
    eprintln!("missing required argument: {flag} <path>");
    std::process::exit(2);
}
