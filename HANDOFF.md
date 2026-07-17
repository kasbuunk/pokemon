# Handoff: architecture orientation & invariants

Written for a fresh session (or human reviewer) with zero prior context.
Section A orients you in the codebase, section B lists the design invariants
that must not regress, section C has every verification command. Process
conventions (merge workflow, owner interaction, session constraints) live in
`CLAUDE.md`; open work lives in the issue tracker, not here.

---

## A. Orientation

This repo is a fault-tolerant Pokémon Gen 1 (Red/Blue/Yellow) `.srm`/`.sav`
save editor. The product is named **Pokémon SRM Editor**; crate names, the
eframe app id, the IndexedDB key and the macOS bundle id keep the old
*pksave* name on purpose (renaming breaks user persistence/TCC grants).

| Where | What |
|---|---|
| `crates/pksave` | Core library. Pure Rust, `#![forbid(unsafe_code)]`, no I/O, builds for wasm32. All parsing/editing/validation. ~255 tests: proptest invariants (round-trip, per-setter edit isolation, list coherence, no-panic-from-file-contents sweep), insta snapshots, mutation-hardened checksum/bcd/party. |
| `crates/pksave-app` | egui/eframe GUI. One codebase → native desktop + browser WASM (trunk). ~148 tests at the logic level (io-event flows, guards, validation). |
| `crates/pksave-app/src/screens/` | One module per screen: overview, trainer, items, pokedex, flags, map, hof, hex, history, and `storage/`. |
| `crates/pksave-app/src/screens/storage/` | The unified party/box UI: `slots.rs` (grid), `detail.rs` (read view), `editor.rs` (edit commands), `transfer.rs` (deposit/withdraw/move). |
| `crates/pksave-app/src/sdcard.rs` | OnionOS/Miyoo SD-card save discovery (native-only): volume poller, marker detection, save preview, `.state`-shadowing warning. |
| `crates/pksave-app/src/history/` | Save version history: `fs.rs` (native), `idb.rs` (wasm IndexedDB, key `pksave-history`), `spans.rs` (human-readable labels for changed byte ranges). |
| `crates/xtask` | Dev tool: `gen-tables` (regenerates static data from a pret/pokered checkout), `gen-offsets-check` (re-derives every offset constant from the ROM symbol file), `make-e2e-fixtures`. |
| `e2e/` | PyBoy harness: boots editor-produced saves in a ROM built from pret/pokered (byte-identical to retail), asserts WRAM values + overworld survival, parametrized over the fixture manifest. |
| `docs/FORMAT.md` | Format ground truth (sym-verified offsets, structures, checksums, encoding). |
| `docs/DISTRIBUTION.md` | Release channels: GitHub Releases (mac + web zip), GitHub Pages, Homebrew tap (`kasbuunk/homebrew-tap`, cask auto-bumped by `release.yml`). |
| `crates/pksave/src/gen1/offsets.rs` | Single source of truth for layout in code; every const doc-comments its pokered WRAM/SRAM label. |
| `crates/xtask/src/pins.rs` | Pinned pokered SHA, retail ROM SHA-1, RGBDS version. CI enforces tables/offsets match these pins. |

Two model decisions worth internalizing before touching storage code:

- **Withdrawal is exp-authoritative.** The box level byte is cosmetic; the
  game's `_MoveMon` derives the party level from experience
  (`CalcLevelFromExperience`) and recalculates stats from base+DV+statExp at
  that level. The editor mirrors this exactly: withdraw derives level from
  exp, deposit copies party level into the box level byte. Never read the
  box level byte as truth.
- **Releases are cut by commit, not by tag.** Sessions cannot push tags
  (proxy-blocked); bump `.release-version` on `main` and
  `cut-release.yml` → `release.yml` creates the tag, builds mac/web
  artifacts, and bumps the Homebrew cask in `kasbuunk/homebrew-tap`.

## B. Design invariants (do not regress)

- Untouched files round-trip **byte-identically** (even with bad checksums).
- A setter touches only its declared byte span; checksums are recomputed only
  in `to_bytes()` and only when edited.
- Diagnostics warn, never refuse.
- The current box lives at 0x30C0 (bank-1 working copy); bank copies of the
  current box are typically stale on real saves (`W-BOX-STALE`) — by design.
- No panic is reachable from file *contents* (hostile-buffer sweep in core
  tests gates this); documented index panics are fine because every
  file-sourced count is clamped.
- pokered symbol names drift across revisions — when referencing WRAM labels
  (e2e manifests, docs), read the pinned checkout's `ram/wram.asm`, never
  quote from memory.

## C. Verification commands

```sh
# Core + app
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p pksave -p pksave-app --target wasm32-unknown-unknown -- -D warnings
cargo fmt --all --check

# Ground-truth checks (needs a pokered checkout at the pinned SHA + RGBDS 1.0.1)
git clone https://github.com/pret/pokered /tmp/pokered && git -C /tmp/pokered checkout $(sed -n 's/.*POKERED_SHA.*"\(.*\)".*/\1/p' crates/xtask/src/pins.rs)
make -C /tmp/pokered red        # needs rgbasm/rgblink/rgbfix 1.0.1 in PATH
cargo run -p xtask -- gen-tables --pokered /tmp/pokered && git diff --exit-code
cargo run -p xtask -- gen-offsets-check --sym /tmp/pokered/pokered.sym

# E2E (boots edited saves in the real game)
cargo run -p xtask -- make-e2e-fixtures --out e2e/fixtures
pip install -r e2e/requirements.txt
POKERED_ROM=/tmp/pokered/pokered.gbc POKERED_SYM=/tmp/pokered/pokered.sym python -m pytest e2e -v

# Web app
cd crates/pksave-app && trunk serve     # or trunk build --release

# Supply chain / coverage / mutation (see issues #33, #34)
cargo audit && cargo deny check
cargo llvm-cov -p pksave
cargo mutants -p pksave -f checksum.rs -f bcd.rs -f party.rs
```

CI on main runs fmt/clippy/test/wasm-build/mac-build/cargo-deny/
verify-against-pokered (tables diff + offsets check + PyBoy e2e) and deploys
the web app to GitHub Pages. All of this must stay green.

## History

The post-merge review packages and the SD-card feature spec that used to
live here shipped: review angles in issues #2–#7 (fixes merged in PR #11),
SD-card discovery in PR #12, save history in PR #13, the storage-UX/rename/
withdrawal work in PR #15. Still-open review angles are tracked as issues:
egui_kittest widget tests (#32), full-crate mutation pass (#33),
coverage-guided fuzzing (#34).
