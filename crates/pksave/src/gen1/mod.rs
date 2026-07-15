//! Generation 1 (Red/Blue/Yellow) save file support.
//!
//! Offsets and structure come from the pret/pokered disassembly; see
//! `docs/FORMAT.md` at the repository root. `offsets.rs` is the single
//! source of truth for the layout and is cross-checked in CI against the
//! symbol file of a pokered build (`cargo xtask gen-offsets-check`).

pub mod offsets;
