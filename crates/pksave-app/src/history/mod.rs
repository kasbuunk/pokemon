//! Automatic backups + save version history (issue #9).
//!
//! Every successful Save records a full 32 KiB snapshot in a per-file
//! history. Snapshots are content-addressed by SHA-256 so identical
//! bytes are stored once; a *no-change* save still appends a manifest
//! entry pointing at the existing blob (deliberately chosen over
//! skipping, so the timeline of saves stays complete).
//!
//! Two backends implement the same [`HistoryStore`] trait, adapted to
//! the app's mpsc→frame-loop pattern (results are delivered as
//! [`HistoryEvent`]s over a channel, like `crate::io::IoEvent`):
//!
//! - native ([`fs`]): a `<full file name>.history/` directory beside the
//!   save with `manifest.json` + `versions/<sha256>.srm` blobs. The
//!   store methods run synchronously; all logic is unit-tested here.
//! - wasm ([`idb`]): IndexedDB, keyed by save identity =
//!   `sha256(original load bytes) + filename`. A thin async adapter over
//!   the same manifest semantics.
//!
//! Git mode and a history-location override are explicitly out of scope
//! for v1 (the history always lives beside the file / in the browser's
//! IndexedDB).

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub mod spans;

#[cfg(not(target_arch = "wasm32"))]
pub mod fs;
#[cfg(target_arch = "wasm32")]
pub mod idb;

/// How a version came to exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// A regular Save of the edited buffer.
    Save,
    /// Imported from a legacy `.bak-<timestamp>` sibling file.
    Import,
    /// A Save of a buffer that was restored from an earlier version;
    /// `parent_id` points at the restored version.
    Restore,
}

impl Origin {
    pub fn label(self) -> &'static str {
        match self {
            Origin::Save => "save",
            Origin::Import => "import",
            Origin::Restore => "restore",
        }
    }
}

/// One version in the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionEntry {
    /// Monotonically increasing, 1-based; never reused after pruning,
    /// so "version N" is stable ("the Nth version ever recorded").
    pub id: u64,
    /// Seconds since the Unix epoch.
    pub timestamp: u64,
    /// Optional user-provided name; a named version is never auto-pruned.
    pub label: Option<String>,
    /// Hex SHA-256 of the snapshot bytes (the blob key).
    pub sha256: String,
    /// Snapshot size in bytes.
    pub size: u64,
    /// For [`Origin::Restore`]: the version that was restored.
    pub parent_id: Option<u64>,
    pub origin: Origin,
}

/// The manifest: an append-only list of versions (append-only in the
/// save flow; the explicit Delete action and pruning remove entries).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Manifest {
    pub versions: Vec<VersionEntry>,
}

impl Manifest {
    /// The id for the next version: max existing id + 1 (1 for empty).
    pub fn next_id(&self) -> u64 {
        self.versions.iter().map(|v| v.id).max().unwrap_or(0) + 1
    }

    pub fn find(&self, id: u64) -> Option<&VersionEntry> {
        self.versions.iter().find(|v| v.id == id)
    }

    /// Whether any entry references the blob `sha` (hex).
    pub fn references(&self, sha: &str) -> bool {
        self.versions.iter().any(|v| v.sha256 == sha)
    }

    /// Prune oldest-first down to `max` entries, skipping named
    /// versions (a named version is never auto-pruned). If everything
    /// that could be pruned is named, nothing more is pruned — the
    /// manifest is then allowed to stay over `max`. Returns the removed
    /// entries.
    pub fn prune_unnamed_oldest(&mut self, max: usize) -> Vec<VersionEntry> {
        let mut removed = Vec::new();
        while self.versions.len() > max {
            // Manifest order is append order, so the first unnamed
            // entry is the oldest prunable one.
            let Some(index) = self.versions.iter().position(|v| v.label.is_none()) else {
                break; // everything left is named: prune nothing more
            };
            removed.push(self.versions.remove(index));
        }
        removed
    }
}

/// Hex SHA-256 of `bytes` (blob content address / wasm save identity).
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        use core::fmt::Write as _;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// `YYYY-MM-DD HH:MM:SS` (UTC) for the history table.
pub fn format_timestamp(secs_since_epoch: u64) -> String {
    let days = (secs_since_epoch / 86_400) as i64;
    let rem = secs_since_epoch % 86_400;
    let (year, month, day) = crate::io::civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// One-line trainer summary of a snapshot for the history table
/// (parsed lazily from the blob); `None` when the blob does not parse.
pub fn summarize(bytes: &[u8]) -> Option<String> {
    let save = pksave::gen1::save::SaveFile::from_bytes(bytes.to_vec()).ok()?;
    let time = save.play_time();
    let badges = save.badges().count_ones();
    let party = save.party().len();
    let badge_plural = if badges == 1 { "" } else { "s" };
    let mon_plural = if party == 1 { "" } else { "s" };
    Some(format!(
        "{} · {badges} badge{badge_plural} · {}:{:02} · {party} mon{mon_plural}",
        save.player_name(),
        time.hours,
        time.minutes
    ))
}

/// Default file name for an exported copy of version `id`:
/// `poke.srm` → `poke-v3.srm`.
pub fn export_file_name(file_name: &str, id: u64) -> String {
    match file_name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem}-v{id}.{ext}"),
        _ => format!("{file_name}-v{id}"),
    }
}

/// A version plus its display state for the History screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VersionRow {
    pub entry: VersionEntry,
    /// `false` when the manifest references a missing/unreadable blob;
    /// the row is shown greyed with an error instead of crashing.
    pub blob_ok: bool,
    /// Trainer summary parsed from the blob (`None`: unavailable).
    pub summary: Option<String>,
}

/// Why a blob was requested — echoed back in
/// [`HistoryEvent::BlobLoaded`] so the app knows what to do with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlobPurpose {
    /// Load the version into the editor as the current buffer.
    Restore,
    /// Diff the version against the current buffer.
    Diff,
    /// Save-as / download a copy of the version.
    Export,
}

/// Result of a history-store operation, delivered to the UI thread
/// (same pattern as [`crate::io::IoEvent`]).
#[derive(Debug)]
pub enum HistoryEvent {
    /// The freshly listed versions (sent after every mutation too).
    Versions(Vec<VersionRow>),
    /// A snapshot was recorded via [`HistoryStore::record`]. Natively
    /// this is only constructed by the store's test-covered `record`
    /// path — the save flow records inside `io::write_picked` and
    /// reports through `IoEvent::Saved` instead.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    Recorded(VersionEntry),
    /// A blob was loaded for `purpose`.
    BlobLoaded {
        id: u64,
        purpose: BlobPurpose,
        bytes: Vec<u8>,
    },
    /// Legacy `.bak-*` siblings were imported (never sent on wasm — no
    /// `.bak` files exist in the browser).
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    LegacyImported { count: usize },
    /// A history operation failed (shown as a toast; never blocks the
    /// save itself).
    Error(String),
}

/// Version-history backend. Every operation reports through the
/// [`HistoryEvent`] channel handed to the store at construction: the
/// native store runs synchronously and sends before returning; the wasm
/// store completes asynchronously (IndexedDB) and requests a repaint.
pub trait HistoryStore {
    /// Record a snapshot of `bytes`. Used by the wasm app flow after a
    /// download-save; the native save flow records inside
    /// `io::write_picked` instead (blob → manifest → save file order),
    /// so natively this is only exercised by the unit tests.
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    fn record(
        &mut self,
        bytes: Vec<u8>,
        origin: Origin,
        parent_id: Option<u64>,
        max_versions: Option<usize>,
    );
    /// Send a fresh [`HistoryEvent::Versions`].
    fn list(&mut self);
    /// Load version `id`'s snapshot bytes for `purpose`.
    fn load_blob(&mut self, id: u64, purpose: BlobPurpose);
    /// Set or clear version `id`'s name.
    fn set_label(&mut self, id: u64, label: Option<String>);
    /// Delete version `id` (the app confirms first).
    fn delete(&mut self, id: u64);
    /// Prune unnamed versions oldest-first down to `max_versions`.
    fn prune(&mut self, max_versions: usize);
    /// Import legacy `.bak-*` sibling files into the history (native
    /// only; a no-op on wasm, where no siblings exist).
    fn import_legacy(&mut self);
}

/// History settings (in-memory for v1; defaults per issue #9).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HistorySettings {
    /// Record a version on every save (default on).
    pub enabled: bool,
    /// Keep at most this many versions, pruning unnamed oldest-first
    /// (`None` = keep all, the default).
    pub max_versions: Option<usize>,
}

impl Default for HistorySettings {
    fn default() -> Self {
        HistorySettings {
            enabled: true,
            max_versions: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, label: Option<&str>) -> VersionEntry {
        VersionEntry {
            id,
            timestamp: 1_000 + id,
            label: label.map(str::to_owned),
            sha256: format!("{id:064x}"),
            size: 0x8000,
            parent_id: None,
            origin: Origin::Save,
        }
    }

    #[test]
    fn origin_serializes_as_lowercase_strings() {
        for (origin, expected) in [
            (Origin::Save, "\"save\""),
            (Origin::Import, "\"import\""),
            (Origin::Restore, "\"restore\""),
        ] {
            assert_eq!(serde_json::to_string(&origin).expect("serialize"), expected);
        }
        let back: Origin = serde_json::from_str("\"restore\"").expect("deserialize");
        assert_eq!(back, Origin::Restore);
    }

    #[test]
    fn manifest_round_trips_through_json() {
        let manifest = Manifest {
            versions: vec![
                entry(1, None),
                VersionEntry {
                    id: 2,
                    timestamp: 1_784_118_896,
                    label: Some("traded Alakazam".to_owned()),
                    sha256: "ab".repeat(32),
                    size: 32_768,
                    parent_id: Some(1),
                    origin: Origin::Restore,
                },
            ],
        };
        let json = serde_json::to_string_pretty(&manifest).expect("serialize");
        let back: Manifest = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, manifest);
        assert!(json.contains("\"origin\": \"restore\""));
    }

    #[test]
    fn sha256_hex_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn next_id_is_max_plus_one_and_survives_pruned_gaps() {
        assert_eq!(Manifest::default().next_id(), 1);
        let manifest = Manifest {
            // Entry 1 was pruned; ids are never reused.
            versions: vec![entry(2, None), entry(7, None)],
        };
        assert_eq!(manifest.next_id(), 8);
    }

    #[test]
    fn prune_removes_unnamed_oldest_first() {
        let mut manifest = Manifest {
            versions: vec![
                entry(1, None),
                entry(2, Some("keep me")),
                entry(3, None),
                entry(4, None),
            ],
        };
        let removed = manifest.prune_unnamed_oldest(2);
        assert_eq!(
            removed.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![1, 3],
            "unnamed pruned oldest-first"
        );
        assert_eq!(
            manifest.versions.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![2, 4]
        );
    }

    #[test]
    fn prune_never_removes_named_versions() {
        // Everything prunable is named: prune nothing, even over max.
        let mut manifest = Manifest {
            versions: vec![
                entry(1, Some("a")),
                entry(2, Some("b")),
                entry(3, Some("c")),
            ],
        };
        assert_eq!(manifest.prune_unnamed_oldest(1), Vec::new());
        assert_eq!(manifest.versions.len(), 3);

        // Mixed: only the unnamed one goes, the result stays over max.
        let mut manifest = Manifest {
            versions: vec![entry(1, Some("a")), entry(2, None), entry(3, Some("c"))],
        };
        let removed = manifest.prune_unnamed_oldest(1);
        assert_eq!(removed.iter().map(|v| v.id).collect::<Vec<_>>(), vec![2]);
        assert_eq!(manifest.versions.len(), 2, "named survivors exceed max");
    }

    #[test]
    fn prune_is_a_no_op_at_or_under_max() {
        let mut manifest = Manifest {
            versions: vec![entry(1, None), entry(2, None)],
        };
        assert_eq!(manifest.prune_unnamed_oldest(2), Vec::new());
        assert_eq!(manifest.prune_unnamed_oldest(5), Vec::new());
        assert_eq!(manifest.versions.len(), 2);
    }

    #[test]
    fn format_timestamp_known_moments() {
        assert_eq!(format_timestamp(0), "1970-01-01 00:00:00");
        assert_eq!(format_timestamp(1_784_118_896), "2026-07-15 12:34:56");
        assert_eq!(format_timestamp(1_709_251_199), "2024-02-29 23:59:59");
    }

    #[test]
    fn summarize_reads_trainer_facts_from_a_blob() {
        use pksave::gen1::save::{GameVariant, SaveFile};
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        save.set_player_name("ASH").expect("name fits");
        save.set_badges(0b0000_0111);
        let summary = summarize(&save.to_bytes()).expect("valid save summarizes");
        assert!(summary.contains("ASH"), "summary: {summary}");
        assert!(summary.contains("3 badge"), "summary: {summary}");
    }

    #[test]
    fn summarize_rejects_unparsable_bytes_without_panicking() {
        assert_eq!(summarize(&[0u8; 16]), None);
    }

    #[test]
    fn export_names_keep_the_extension() {
        assert_eq!(export_file_name("poke.srm", 3), "poke-v3.srm");
        assert_eq!(export_file_name("new.sav", 14), "new-v14.sav");
        assert_eq!(export_file_name("noext", 1), "noext-v1");
        assert_eq!(export_file_name(".hidden", 2), ".hidden-v2");
    }
}
