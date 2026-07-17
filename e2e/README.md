# End-to-end: boot editor-produced saves in the real engine

This suite proves that saves written by the `pksave` core crate are accepted
by real Pokémon Red: each fixture is injected as cartridge RAM into a
headless PyBoy running a pokered-built ROM (byte-identical to retail,
SHA-1 pinned in `crates/xtask/src/pins.rs`), the intro is scripted through
`CONTINUE`, and WRAM is compared against expectations the fixture generator
emitted. A save the engine rejects never offers `CONTINUE`, so the boot
times out and the test fails — that is the corruption signal.

## Running locally

```sh
# 1. Generate the save fixtures + fixtures.json manifest with the core crate
cargo run -p xtask -- make-e2e-fixtures --out e2e/fixtures

# 2. Point the suite at a pokered build (ROM + rgbds symbol file)
export POKERED_ROM=/path/to/pokered.gbc
export POKERED_SYM=/path/to/pokered.sym

# 3. Run
pip install -r e2e/requirements.txt
cd e2e && python -m pytest -v
```

Failure screenshots land in `e2e/artifacts/`.

## How it works

- **RAM injection**: PyBoy's `ram_file` constructor argument loads a raw,
  headerless dump of the cartridge RAM banks before the first frame. For
  Pokémon Red (MBC3, 32 KiB SRAM) that format is byte-identical to a
  `.sav`, so fixtures are passed through unmodified.
- **Boot scripting**: alternate START/A presses while a PyBoy hook waits
  for the game to execute `EnterMap` (resolved from the `.sym` file);
  after it fires, `wSaveFileStatus == 2` is required (the value the menu
  sets only when the save checksum matched).
- **Assertions**: `fixtures.json` carries `expected_wram` entries of
  `{label, offset, bytes}` — raw bytes read back from the serialized save
  by the Rust generator — so the Python side needs no Gen 1 charset/BCD
  code. Labels are resolved through the pokered symbol file at runtime.

## Overworld survival

`test_overworld_survives_after_continue` runs each fixture 600+ frames past
`EnterMap` and requires the LCD to stay on and the play-time clock to keep
ticking. This guards a subtle engine contract: on CONTINUE the game trusts
the save's cached map-header block (music id/bank, map view pointer, map
dimensions, map data/text/script pointers, connections, warps, signs,
sprite objects, tileset pointers — WRAM `$D35B..$D53A`) because `LoadSAV`
sets `BIT_NO_PREVIOUS_MAP` and `LoadMapHeader` then early-returns instead
of reloading it from ROM. A zeroed block boots but crashes (`rst $38` wild
jump) about one second in. `SaveFile::new_empty` now bakes in the genuine
NEW GAME spawn block (`crates/pksave/src/gen1/engine_state.rs`), captured
from a scripted fresh game in this very harness.

## Scripted PC withdrawal

`test_pc_withdrawal.py` goes beyond booting: it plays the game. Using the
`boxmon.sav` fixture (CHARMANDER stored with a stale box level byte of 80
but experience for level 50), it redirects the bedroom staircase's cached
warp entry to a Pokémon Center, walks the player to the PC through real
collision/NPC traffic (coordinate-feedback steps, not blind input replay),
drives BILL'S PC → WITHDRAW MON → slot 2 synchronized on symbol hooks, and
asserts the withdrawn party mon comes out at level 50 — proving in-engine
that withdrawal levels derive from experience, never the box level byte
(the editor's exp-authoritative model, issue #29).
