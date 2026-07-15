//! Fault-tolerant Pokémon save file editing.
//!
//! Currently supports Generation 1 (Red/Blue/Yellow) battery saves
//! (`.srm`/`.sav`, raw 32 KiB SRAM dumps) via [`gen1`]. The crate performs no
//! I/O, contains no unsafe code, and compiles unchanged to
//! `wasm32-unknown-unknown`.
//!
//! The anti-corruption model: the entire input buffer is held verbatim and
//! every edit mutates only the bytes of the field being changed. A single
//! global *edited* flag gates serialization: an untouched file always
//! round-trips byte-identically, while `to_bytes()` on an edited file
//! recomputes all 15 checksums — except regions pinned via a checksum
//! override, whose stored byte is kept verbatim. See `docs/FORMAT.md` in
//! the repository for the format reference.

#![forbid(unsafe_code)]

pub mod gen1;

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Info,
    Warning,
    Error,
}

/// A non-fatal finding about a save file. Diagnostics never block editing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable machine-readable code, e.g. `W-CHECKSUM-MAIN`.
    pub code: &'static str,
    pub message: String,
    /// Byte range in the file this concerns, if applicable.
    pub span: Option<core::ops::Range<usize>>,
}

/// Minimal game-agnostic boundary so future generations can slot in
/// alongside [`gen1`].
pub trait SaveGame {
    /// Human-readable label, e.g. "Pokémon Red/Blue".
    fn game_label(&self) -> &str;
    /// Current diagnostics for the buffer.
    fn diagnostics(&self) -> Vec<Diagnostic>;
    /// Serialize back to bytes (recomputing checksums once anything was
    /// edited, minus any pinned regions).
    fn to_bytes(&self) -> Vec<u8>;
}
