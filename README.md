# pksave — Pokémon Red/Blue/Yellow save editor

A comprehensive, fault-tolerant editor for Generation 1 Pokémon battery saves
(`.srm`/`.sav` — raw 32 KiB SRAM dumps), written in Rust. One codebase runs as
a **native desktop app** (macOS/Linux/Windows) and **in the browser** (WASM).

It edits everything the game saves: trainer identity, money, coins, badges,
options, play time, location (map/coordinates with coherent warping), bag and
PC items, the full Pokédex, party and all 12 PC boxes (every Pokémon field:
species, level/EXP, moves, PP + PP Ups, DVs, stat experience, status, types,
catch rate, OT, nicknames, calculated stats), daycare, event flags — including
every battled-trainer flag, by their pokered names — missable objects, hidden
items, fly-unlocked towns, Hall of Fame, and a raw hex view for anything else.

## Safety model (why it won't corrupt your save)

- **In-place editing.** The entire input file is held verbatim; every edit
  mutates only the bytes of the field being changed. An untouched file always
  serializes back **byte-identical** — enforced by property-based tests
  (round-trip on arbitrary buffers, per-setter edit-isolation with declared
  byte spans, list-repacking coherence).
- **Checksums** (main + per-box + per-bank) are recomputed only when the file
  was actually edited, so even a corrupt file round-trips unchanged until you
  choose to modify or repair it.
- **Warn, never refuse.** A diagnostics catalogue (stable codes, byte spans)
  reports checksum mismatches, invalid species/levels/BCD/terminators, stale
  box copies, and more — without blocking you from opening or fixing a file.
- **Never overwrites silently.** The desktop app writes a timestamped `.bak`
  beside the original before saving over it; the web app keeps the original
  bytes downloadable at all times.
- **Verified against the real game.** CI builds Pokémon Red from the
  [pret/pokered](https://github.com/pret/pokered) disassembly (byte-identical
  to retail, pinned by SHA), boots editor-produced saves in a headless
  emulator (PyBoy), and asserts the game engine accepts them and reads the
  exact values back from memory — including surviving in the overworld, not
  just passing the checksum screen.

## Format ground truth

Offsets and structures are documented in [docs/FORMAT.md](docs/FORMAT.md) and
derived from the pokered disassembly. `crates/pksave/src/gen1/offsets.rs` is
the single source of truth in code; `cargo xtask gen-offsets-check` re-derives
every constant from the ROM build's symbol file, and `cargo xtask gen-tables`
regenerates all static data tables (species/base stats/index↔dex mapping,
moves, items, types, event-flag names, maps) from the pinned checkout — CI
fails if either drifts.

## Building

```sh
# Native desktop app
cargo run --release -p pksave-app

# Web app (requires trunk: cargo install trunk)
cd crates/pksave-app && trunk serve

# Tests (core library: unit + property + snapshot tests)
cargo test --workspace
```

Pushes to `main` deploy the web app to GitHub Pages automatically.

### OnionOS/Miyoo SD-card discovery (native only)

The desktop app watches for removable volumes (macOS `/Volumes`,
elsewhere via `sysinfo`). When an OnionOS/Miyoo Mini SD card appears —
recognized by its root markers (`.tmp_update/`, `Saves/CurrentProfile/`,
`miyoo/`, …), never by volume name — it scans every profile's
`Saves/<profile>/saves/Gambatte/*.srm` (plus the legacy
`RetroArch/.retroarch/saves/` path), previews the Gen 1 saves it finds
and offers them in an "SD card saves" panel. Opening and saving use the
regular flows (timestamped `.bak` beside the file, fsync before the
"safe to eject" note). If a save state shadows the battery save (OnionOS
auto-loads states), the app warns and offers to rename it out of the way.

On macOS 13+, reading a removable volume triggers a one-time
"Files and Folders → Removable Volumes" consent prompt. For a stable
prompt (remembered across runs), ship the app as a bundle: install
[cargo-bundle](https://github.com/burtonageo/cargo-bundle), then

```sh
cd crates/pksave-app && cargo bundle --release
codesign --force --deep -s - target/release/bundle/osx/pksave.app  # ad-hoc
```

The bundle metadata in `crates/pksave-app/Cargo.toml` injects
`NSRemovableVolumesUsageDescription` (from `assets/InfoPlist.ext.plist`)
into the generated `Info.plist`. A bare `cargo run` binary still works —
macOS then attributes the TCC grant per invocation, which can re-prompt.

### End-to-end verification (optional, used by CI)

Requires RGBDS 1.0.1 and Python 3.11+:

```sh
git clone https://github.com/pret/pokered && git -C pokered checkout <pinned SHA>  # see crates/xtask/src/pins.rs
make -C pokered red                                    # builds pokered.gbc + pokered.sym
cargo run -p xtask -- make-e2e-fixtures --out e2e/fixtures
pip install -r e2e/requirements.txt
POKERED_ROM=$PWD/pokered/pokered.gbc POKERED_SYM=$PWD/pokered/pokered.sym \
  python -m pytest e2e -v
```

## Workspace layout

| Crate | Purpose |
|---|---|
| `crates/pksave` | Core library: pure, no I/O, `#![forbid(unsafe_code)]`, compiles to wasm32. All parsing/editing/validation. |
| `crates/pksave-app` | egui/eframe GUI, native + browser from one codebase. |
| `crates/xtask` | Dev tooling: table generation, offset verification, e2e fixture generation. |
| `e2e/` | PyBoy harness booting fixtures in the real game. |

The core is namespaced under `pksave::gen1` behind a minimal `SaveGame` trait,
so later generations can be added alongside without touching Gen 1 code.

## Sources

- [pret/pokered](https://github.com/pret/pokered) — the authoritative
  disassembly (pinned commit in `crates/xtask/src/pins.rs`)
- Bulbapedia: *Save data structure (Generation I)*, *Pokémon data structure
  (Generation I)*, *Character encoding (Generation I)*
- PKHeX's Gen 1 handling informed the in-place-editing safety model
