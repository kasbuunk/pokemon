# Handoff: post-merge review + SD-card discovery feature

Written for a fresh session (or human reviewer) with zero prior context. Two
work packages: **B** — a multi-angle review of the merged editor; **C** — the
OnionOS/Miyoo SD-card save-discovery feature (fully specified, not yet built).
Section A orients you; section D has every verification command.

---

## A. Orientation

This repo is a fault-tolerant Pokémon Gen 1 (Red/Blue/Yellow) `.srm`/`.sav`
save editor. Everything merged in PR #1. The product is named **Pokémon SRM
Editor** (formerly *pksave* — the crate names keep the old name on purpose).

| Where | What |
|---|---|
| `crates/pksave` | Core library. Pure Rust, `#![forbid(unsafe_code)]`, no I/O, builds for wasm32. All parsing/editing/validation. ~235 tests incl. proptest invariants (round-trip, per-setter edit isolation, list coherence) and insta snapshots. |
| `crates/pksave-app` | egui/eframe GUI (10 screens incl. hex viewer). One codebase → native desktop + browser WASM (trunk). |
| `crates/xtask` | Dev tool: `gen-tables` (regenerates static data from a pret/pokered checkout), `gen-offsets-check` (re-derives every offset constant from the ROM symbol file), `make-e2e-fixtures`. |
| `e2e/` | PyBoy harness: boots editor-produced saves in a ROM built from pret/pokered (byte-identical to retail), asserts WRAM values + overworld survival. 12 tests. |
| `docs/FORMAT.md` | Format ground truth (sym-verified offsets, structures, checksums, encoding). |
| `crates/pksave/src/gen1/offsets.rs` | Single source of truth for layout in code; every const doc-comments its pokered WRAM/SRAM label. |
| `crates/xtask/src/pins.rs` | Pinned pokered SHA, retail ROM SHA-1, RGBDS version. CI enforces tables/offsets match these pins. |

Key design invariants (do not regress):
- Untouched files round-trip **byte-identically** (even with bad checksums).
- A setter touches only its declared byte span; checksums are recomputed only
  in `to_bytes()` and only when edited.
- Diagnostics warn, never refuse.
- The current box lives at 0x30C0 (bank-1 working copy); bank copies of the
  current box are typically stale on real saves (`W-BOX-STALE`) — by design.

## B. Post-merge review brief

Run each angle with a dedicated subagent to keep context clean; verify claims
adversarially (reproduce, don't trust). File findings as GitHub issues (one
per finding, labeled by angle) or a single review report — your call.

### B1. Functional correctness
Entry points: `gen1/{boxes,party,items,pokemon,stats,bcd,text,checksum}.rs`.
Probe specifically:
- Current-box routing and `sync_current_box_to_bank` (edits to box N==current
  must hit 0x30C0 + main checksum only; N≠current only its bank + 2 checksums).
- Repacking edge cases: remove-at-count-boundary, add-after-remove, swap with
  daycare/HoF untouched; parallel-array alignment.
- Box↔party conversion: withdraw derives the party level from experience
  (`CalcLevelFromExperience` — the box level byte is cosmetic) and
  recalculates stats from base+DV+statExp at that level; deposit copies
  party level into box level byte.
- BCD lossy path (`decode_lossy`) and money/coins clamps.
- Text codec multi-char glyphs (`'r`, `<PK>`) encode/decode round-trip.
- Hex-editor write path in the app (round-trips whole buffer through
  `to_bytes`/`from_bytes` per byte edit — check it can't corrupt tail bytes).
- Spot-check ~10 offsets in `offsets.rs` against `docs/FORMAT.md` and the
  pokered symbol file independently (`cargo xtask gen-offsets-check` exists,
  but re-derive a sample by hand).
Pass: no confirmed correctness finding above Low severity.

### B2. Ergonomic usage (core API)
Known pain points reported by the GUI build — evaluate and propose/implement:
- No `raw_mut()`/`set_byte(offset, value)` on `SaveFile`; the hex editor
  round-trips the entire buffer per edit as a workaround.
- Hex edits to checksum bytes are silently undone by `to_bytes()` (documented
  only in a UI tooltip). Consider an explicit `set_checksum_override` or a
  documented API-level note + diagnostic.
- Party/box mon accessors are macro-generated, not a shared trait — GUI needed
  snapshot/edit-command indirection to share the editor between party and box.
Pass: a written API-evolution proposal (or merged improvements) covering all three.

### B3. Security
- Fuzz `SaveFile::from_bytes` + every view accessor over malformed 32 KiB
  buffers (cargo-fuzz or a proptest sweep): **no panics reachable from file
  contents**. Documented panics on out-of-range *indexes* are fine only if the
  UI can't trigger them from a hostile file (counts are clamped — verify).
- wasm: a panic aborts the app = DoS via crafted file; same sweep applies.
- Verify `#![forbid(unsafe_code)]` transitively (cargo-geiger optional).
- Supply chain: `cargo audit` / `cargo deny check`; review GitHub Actions for
  unpinned third-party actions (Swatinem/rust-cache etc. — consider SHA-pinning).
- Native save/backup path handling: backup writes `<path>.bak-<ts>` next to a
  user-chosen path; confirm no traversal/symlink surprise on overwrite.
Pass: fuzz corpus runs clean; audit clean or triaged.

### B4. Test coverage
- `cargo llvm-cov -p pksave` — report line/branch coverage; target the gaps
  (validate.rs branches, lossy paths, error arms).
- GUI: 17 unit tests only; egui_kittest was skipped. Identify the 5 highest-
  value app tests (open→edit→save flow with a fake io channel; dirty guard;
  hex jump-to-span; name-validation widget; backup naming).
- Mutation-test the crown jewels: `cargo mutants -p pksave -f checksum.rs -f
  bcd.rs -f party.rs` — surviving mutants = missing assertions.
- Consider raising proptest cases in CI (PROPTEST_CASES env) on main.
Pass: coverage numbers reported + top gaps closed or ticketed.

### B5. Coding standards & quality
- `clippy::unwrap_used` is warn — decide: deny in core, allow-with-comment in
  app? Enforce.
- `missing_docs` on `pksave` public API; module-level docs exist — check item level.
- Duplication across `screens/*.rs` (the party/box mon editors, list widgets)
  — extract if it reduces total complexity, don't force it.
- Error types: thiserror in core, ad-hoc strings in app io — unify app-side.
- `data/generated/*` headers say do-not-edit + record the pokered SHA — verify
  `git diff` cleanliness is CI-enforced (it is: `verify-against-pokered`), and
  that no one hand-edited them.
Pass: lints decided + enforced in CI, findings fixed or ticketed.

### B6. UX
The app has **never been run by a human** (built and tested headless). Do a
manual pass on macOS and in a browser (Pages URL below):
- First-run flow: empty state, New/Open affordances, drag-and-drop a `.sav`.
- Editing flows: party mon editor completeness, box deposit/withdraw, dex
  bulk actions, flags search, hex viewer legend/jump.
- Safety UX: dirty indicator, unsaved-changes guards (close/open/revert),
  backup message after save, "Download original" discoverability on web,
  diagnostics banner → hex-span navigation.
- Basics: resize, dark/light, keyboard navigation, wasm file-picker fallback
  when the File System Access API is absent (Safari/Firefox).
Pass: screenshot-annotated findings list, prioritized.

## C. SD-card discovery feature (spec — build next)

**Goal:** insert a Miyoo Mini Plus (OnionOS) SD card into a MacBook → the
running app notices within seconds, finds Gen 1 saves, and offers them —
open, edit, write back safely.

### C1. Verified facts (sources inline — re-verify only if OnionOS pin changes)
- OnionOS ships RetroArch with `savefile_directory = "/mnt/SDCARD/Saves/CurrentProfile/saves"`,
  `sort_savefiles_enable = "true"`, `savefiles_in_content_dir = "false"`
  (OnionUI/Onion repo, `static/configs/RetroArch/.retroarch/retroarch.cfg`).
  → battery saves on the card: `<CARD_ROOT>/Saves/CurrentProfile/saves/Gambatte/<ROM basename>.srm`
  (core dir is literally `Gambatte`, capital G).
- Legacy/stock path (older Onion, stock Miyoo fw): `<CARD_ROOT>/RetroArch/.retroarch/saves/<CORE>/` — scan as fallback (OnionOS FAQ).
- Profiles: `Saves/` can contain a secondary/guest profile — enumerate every
  `Saves/*/saves/` child, not just `CurrentProfile` (OnionOS docs/features).
- Save states: `Saves/CurrentProfile/states/Gambatte/*.state`, and OnionOS
  sets `savestate_auto_load = "true"` — **an existing state shadows an edited
  .srm**; the OnionOS FAQ explicitly tells users to delete the state. The app
  MUST surface this.
- Card markers at volume root: `.tmp_update/` (OnionOS boot hook, strongest),
  plus `Saves/CurrentProfile/`, `miyoo/`, `RetroArch/`, `Roms/GB/`.
- Gambatte Gen 1 `.srm` = raw 32,768-byte SRAM, no header/RTC/sidecars (Gen 2
  appends RTC → ≥32768; out of scope).
- macOS mounts cards at `/Volumes/<arbitrary label>`; scan entries, test markers.
- macOS TCC (13+): first access to a removable volume without implied consent
  prompts the user ("Files and Folders → Removable Volumes"). Ship the Mac
  build as a signed `.app` with `NSRemovableVolumesUsageDescription` in
  Info.plist (add a `cargo-bundle`/bundle step); a bare cargo binary gets
  flaky per-path TCC attribution. rfd-dialog/drag-drop flows carry implied
  consent and stay as fallback.

### C2. Design
- New `crates/pksave-app/src/sdcard.rs`, native-only
  (`#[cfg(not(target_arch = "wasm32"))]`); panel hidden on wasm.
- Pure, testable core: `fn scan_volume(root: &Path) -> Option<OnionCard>`
  where `OnionCard { volume_name, saves: Vec<DiscoveredSave> }` and
  `DiscoveredSave { path, rom_name, profile, legacy, size_ok, preview: Option<SavePreview>, shadowing_state: Option<PathBuf> }`.
  Preview = parse with `pksave` (trainer name, badges count, play time, party
  summary) — only list files that are 32,768+ bytes AND parse with checksum
  valid-or-repairable; show invalid ones greyed with the diagnostic.
- Poller thread (reuse the mpsc→frame-loop channel pattern in `io.rs`):
  every ~3 s enumerate candidate roots — macOS `/Volumes/*` via `std::fs`;
  Linux/Windows via the `sysinfo` crate `Disks` (`mount_point()`,
  `is_removable()`) — diff against the last set, `scan_volume` on new mounts,
  send results to the UI. Stop/steady-state cost ≈ one readdir per poll.
- UX: unobtrusive toast/banner "Miyoo SD card detected — N Pokémon saves" →
  SD panel listing saves with previews → click opens (standard open path, so
  all guards/backups apply). Saving back to the card: existing timestamped
  `.bak` beside the file + `File::sync_all` before reporting success, then a
  "safe to eject" note. If `shadowing_state` is Some, show a prominent
  warning with buttons: Rename state (`.state` → `.state.bak-<ts>`) / Ignore.
- Card disappearing mid-edit (user pulls it): the buffer is in memory —
  detect the vanished mount on save, fall back to save-as dialog instead of
  erroring out.

### C3. Tests
Unit tests build fake card trees in tempdirs: marker detection (positive;
negatives: plain camera card, empty card, `Saves/` without profile dirs);
CurrentProfile + secondary profile enumeration; legacy path fallback;
Gambatte-only filtering (ignore `Saves/*/saves/gpSP/`); 32 KiB size gate
(reject 512 B, accept 32768 and 32777? no — Gen 1 exactly 32768; a 65536
Gambatte file should be rejected here even though `from_bytes` tolerates it,
because on-card Gen 1 Gambatte saves are exactly 32 KiB — document this
deliberate strictness); shadowing `.state` detection; preview extraction from
a `new_empty`-derived fixture. Poller: unit-test the diff logic with an
injected `fn list_roots() -> Vec<PathBuf>`.

### C4. Acceptance criteria
1. Fake-tree unit suite green in CI (runs on ubuntu; pure fs logic).
2. Manual on a Mac: card detected < 5 s after insert; save listed with correct
   preview; open→edit→save writes back with `.bak` on card; state-shadow
   warning appears when a matching `.state` exists; second run of the bundled
   `.app` does not re-prompt TCC.
3. No regression: wasm build unaffected; all existing tests green.

## D. Verification commands

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

# Review tooling (B3/B4)
cargo audit && cargo deny check
cargo llvm-cov -p pksave
cargo mutants -p pksave -f checksum.rs -f bcd.rs -f party.rs
```

CI on main runs fmt/clippy/test/wasm-build/mac-build/verify-against-pokered
(tables diff + offsets check + PyBoy e2e) and deploys the web app to GitHub
Pages. All of this must stay green.
