# Claude project notes

## Merge workflow (owner's standing instruction, 2026-07-16)

Changes go to `main` without waiting for human review, CI-gated:

1. **Preferred:** open a PR and **immediately enable auto-merge with
   the rebase method** — the branch's commits land on main individually
   the moment all required CI checks pass. **Never squash.** Craft the
   branch history accordingly: each commit self-contained (builds, tests
   green on its own), one logical change per commit, imperative message
   with a body explaining why. If enabling auto-merge fails (repo
   setting off / no required checks yet — see issue #21), fall back to
   polling checks and rebase-merging on green.
2. Direct pushes to `main` are also sanctioned for trivial changes
   (docs, comments) where CI adds nothing.

Never merge with failing or pending checks. Issues labelled **`human`**
are owner-only action items — do not attempt to resolve them
autonomously; everything else is fair game.

## Project facts

- Product name: **Pokémon SRM Editor** (user-facing). Crate names, the
  eframe app id, the IndexedDB key `pksave-history` and the macOS bundle
  identifier `com.kasbuunk.pksave` keep the old *pksave* name on
  purpose — renaming them breaks user persistence/TCC grants.
- Ground truth for save-format questions: `docs/FORMAT.md` and the
  pinned pret/pokered checkout (`crates/xtask/src/pins.rs`). Withdrawal
  level derives from **experience**, never the box level byte.
- Verification bar for any change: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, the same
  clippy for `--target wasm32-unknown-unknown` (app + core), and
  `cargo fmt --all --check`. CI additionally boots edited saves in a
  real ROM (`verify-against-pokered`).
- Releases are tag-triggered (`v*` → `.github/workflows/release.yml`);
  distribution channels and manual steps live in `docs/DISTRIBUTION.md`.
- `HANDOFF.md` has the deeper architecture orientation and design
  invariants (byte-identical round-trips, warn-never-refuse, live-box
  working copy at 0x30C0).
