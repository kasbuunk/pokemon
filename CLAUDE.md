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

Never merge with failing or pending checks — and don't assume auto-merge
enforces that: on 2026-07-16 armed PRs merged within a minute, before
any CI job finished, meaning required status checks were not active on
the `main` ruleset. Until a PR is observed *waiting* for checks, verify
green yourself before or right after the merge.

## Working with the owner

- Issues labelled **`human`** are owner-only action items — never
  resolve them autonomously. The owner delegates one by commenting and
  *removing* the label; hand it back by commenting what remains and
  re-adding the label. Close only when nothing remains.
- Issue format the owner expects: 1–2 sentences of impact, then the
  instructions, then two lines of alternatives considered. Concise
  everywhere; no narration.

## Session environment constraints (Claude Code remote sessions)

- **Tag pushes are blocked** on every channel: the git proxy 403s
  tag refs, the API proxy denies ref-creation writes and workflow
  dispatch, and the GitHub MCP app lacks the dispatch permission.
  Branch pushes and PR/issue/auto-merge MCP operations work. Releases
  from a session therefore go through **release-by-commit**: bump
  `.release-version` on `main` (cut-release.yml → release.yml creates
  the tag in CI). Don't spend time rediscovering this.
- GitHub MCP `actions_list`-style responses can exceed the token limit;
  they get saved to a file — filter with python/jq instead of rereading.

## Project facts

- Product name: **Pokémon SRM Editor** (user-facing). Crate names, the
  eframe app id, the IndexedDB key `pksave-history` and the macOS bundle
  identifier `com.kasbuunk.pksave` keep the old *pksave* name on
  purpose — renaming them breaks user persistence/TCC grants.
- Ground truth for save-format questions: `docs/FORMAT.md` and the
  pinned pret/pokered checkout (`crates/xtask/src/pins.rs`). Withdrawal
  level derives from **experience**, never the box level byte.
- **pokered symbol names drift across revisions** (e.g. the box count
  is `wBoxCount` at the pinned SHA, not the older `wNumInBox` — that
  mismatch broke CI once). When referencing WRAM labels (e2e manifests,
  docs), read the pinned checkout's `ram/wram.asm`; never quote symbol
  names from memory.
- Verification bar for any change: `cargo test --workspace`,
  `cargo clippy --workspace --all-targets -- -D warnings`, the same
  clippy for `--target wasm32-unknown-unknown` (app + core), and
  `cargo fmt --all --check`. CI additionally boots edited saves in a
  real ROM (`verify-against-pokered`).
- Releases: bump `.release-version` on main (preferred from sessions)
  or push a `v*` tag; see `docs/DISTRIBUTION.md` for channels and
  manual steps.
- `HANDOFF.md` has the deeper architecture orientation and design
  invariants (byte-identical round-trips, warn-never-refuse, live-box
  working copy at 0x30C0).
- Search Console: "Couldn't fetch" right after a sitemap submission on
  a fresh property is a placeholder, not an error — recheck after
  24–48h before debugging the serving side.
