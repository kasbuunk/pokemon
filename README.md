# Pokémon SRM Editor — Red/Blue/Yellow save editor (web + desktop)

**[Open the web app →](https://kasbuunk.github.io/pokemon/)** — free, runs
entirely in your browser; your save file never leaves your device.

Pokémon SRM Editor is a save editor for the Generation 1 Pokémon games —
**Red, Blue and Yellow**. It opens the battery save files (`.srm` or `.sav`,
raw 32 KiB SRAM dumps) written by emulators and flash carts, including
RetroArch/Gambatte, the Miyoo Mini and other OnionOS handhelds, the Analogue
Pocket, and real cartridges dumped to a file. Use it online in any browser, or
download the native desktop app for macOS, Windows or Linux. Written in Rust;
formerly known as *pksave*.

## Features

- **Everything the game saves**: trainer identity, money, coins, badges,
  options, play time, location (map/coordinates with coherent warping), bag
  and PC items, the full Pokédex, daycare, Hall of Fame, event flags —
  including every battled-trainer flag, by their pokered names — missable
  objects, hidden items, fly-unlocked towns, and a raw hex view for the rest.
- **Party and all 12 PC boxes on one screen**: drag and drop Pokémon between
  party and boxes (or right-click / use buttons), reorder, move across boxes,
  swap in place, daycare in and out. Every Pokémon field is editable: species,
  level/EXP, moves, PP + PP Ups, DVs, stat experience, status, types, catch
  rate, OT, nicknames, calculated stats.
- **No in-game surprises**: fields the game recomputes (a box Pokémon's level
  and stats derive from *experience* on withdrawal) are marked, previewed
  ("on withdraw: Lv.50 — …") and kept coherent — editing a box Pokémon's
  level also sets its experience, so it stays that level after you withdraw it.
- **Save version history** with diffs and one-click restore.
- **OnionOS/Miyoo SD-card discovery** (desktop): insert the card and your
  Gen 1 saves are found and offered automatically.

## Screenshots

<!-- TODO: add screenshots (welcome screen, Pokémon storage screen, hex view). -->

## FAQ

### What is an .srm file?

The battery-backed save RAM of a Game Boy cartridge, dumped to a file. For
Gen 1 Pokémon games it is exactly 32,768 bytes. Emulators (RetroArch/Gambatte,
SameBoy, mGBA, …) write it next to the ROM as `<game>.srm` or `<game>.sav`;
handhelds like the Miyoo Mini keep it on the SD card. Both extensions are the
same format and this editor opens either.

### Is it safe? Will it corrupt my save?

The editor is built around not corrupting files:

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
  box copies, level/experience mismatches, and more — without blocking you
  from opening or fixing a file.
- **Never overwrites silently.** The desktop app writes a timestamped `.bak`
  beside the original before saving over it; the web app keeps the original
  bytes downloadable at all times.
- **Verified against the real game.** CI builds Pokémon Red from the
  [pret/pokered](https://github.com/pret/pokered) disassembly (byte-identical
  to retail, pinned by SHA), boots editor-produced saves in a headless
  emulator (PyBoy), and asserts the game engine accepts them and reads the
  exact values back from memory — including surviving in the overworld, not
  just passing the checksum screen.

### Does it work with Pokémon Yellow? With the Miyoo Mini?

Yes — Red, Blue and Yellow share one save layout (Yellow is auto-detected via
its Pikachu friendship byte). Miyoo Mini / OnionOS cards are scanned
automatically by the desktop app (see below), or open the `.srm` from
`Saves/CurrentProfile/saves/Gambatte/` by hand in the web app. One caveat on
OnionOS: an auto-loaded **save state** shadows the battery save — the app
warns about this and offers to rename the state out of the way.

### Why does my box Pokémon change level when I withdraw it?

In Gen 1 the game derives a box Pokémon's level from its **experience** when
you withdraw it (`CalcLevelFromExperience`); the level shown in the box list
is cosmetic. Editors that change only the level byte cause exactly this
surprise. This editor sets experience together with the level and shows an
"on withdraw" preview, so what you see is what you get in-game.

## Download the desktop app

Grab the latest build from
[Releases](https://github.com/kasbuunk/pokemon/releases):

- **macOS** (universal): download `Pokemon-SRM-Editor-macos-universal.zip`,
  unzip, drag `Pokémon SRM Editor.app` to Applications. The app is open source
  but **not notarized by Apple** (that requires a paid developer account), so
  the first launch needs one extra step: **right-click the app → Open → Open**.
  Alternatively: `xattr -dc "/Applications/Pokémon SRM Editor.app"`.
- **Windows**: `Pokemon-SRM-Editor-windows-x86_64.zip`.
- **Linux**: `Pokemon-SRM-Editor-linux-x86_64.tar.gz`.

Verify a download against the release's `SHA256SUMS.txt`:
`shasum -a 256 -c SHA256SUMS.txt` (macOS/Linux). Or build from source in one
command — see below. Signing/notarization details and the release process are
documented in [docs/DISTRIBUTION.md](docs/DISTRIBUTION.md).

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
codesign --force --deep -s - "target/release/bundle/osx/Pokémon SRM Editor.app"  # ad-hoc
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

## Format ground truth

Offsets and structures are documented in [docs/FORMAT.md](docs/FORMAT.md) and
derived from the pokered disassembly. `crates/pksave/src/gen1/offsets.rs` is
the single source of truth in code; `cargo xtask gen-offsets-check` re-derives
every constant from the ROM build's symbol file, and `cargo xtask gen-tables`
regenerates all static data tables (species/base stats/index↔dex mapping,
moves, items, types, event-flag names, maps) from the pinned checkout — CI
fails if either drifts.

## Workspace layout

| Crate | Purpose |
|---|---|
| `crates/pksave` | Core library: pure, no I/O, `#![forbid(unsafe_code)]`, compiles to wasm32. All parsing/editing/validation. |
| `crates/pksave-app` | egui/eframe GUI, native + browser from one codebase. |
| `crates/xtask` | Dev tooling: table generation, offset verification, e2e fixture generation. |
| `e2e/` | PyBoy harness booting fixtures in the real game. |

The core is namespaced under `pksave::gen1` behind a minimal `SaveGame` trait,
so later generations can be added alongside without touching Gen 1 code.
(The crates keep the original *pksave* name; only the product was renamed.)

## Sources

- [pret/pokered](https://github.com/pret/pokered) — the authoritative
  disassembly (pinned commit in `crates/xtask/src/pins.rs`)
- Bulbapedia: *Save data structure (Generation I)*, *Pokémon data structure
  (Generation I)*, *Character encoding (Generation I)*
- PKHeX's Gen 1 handling informed the in-place-editing safety model

---

Pokémon is a trademark of Nintendo / Creatures Inc. / GAME FREAK inc. This
project is fan-made, open source ([MIT](LICENSE)), and not affiliated with or
endorsed by Nintendo, Game Freak or The Pokémon Company.
