//! Native history backend: a `<full file name>.history/` directory
//! beside the save file.
//!
//! Layout:
//!
//! ```text
//! poke.srm
//! poke.srm.history/
//!   manifest.json           # Manifest (serde_json)
//!   versions/<sha256>.srm   # full snapshots, content-addressed
//! ```
//!
//! Corruption-safety write order (issue #9): blob (temp + fsync +
//! atomic rename) → manifest (temp + fsync + atomic rename) → only then
//! does the caller write the actual save file. A crash in between
//! leaves at worst an *orphan* blob, which is ignored and GC'd
//! opportunistically on the next mutation; the manifest can never
//! reference a blob that was not fully written. Should a manifest entry
//! nevertheless point at a missing blob (external tampering), the entry
//! surfaces as an error row instead of crashing.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;

use thiserror::Error;

use super::{
    sha256_hex, summarize, BlobPurpose, HistoryEvent, HistoryStore, Manifest, Origin, VersionEntry,
    VersionRow,
};

/// A history operation failed. Never blocks the save itself — the save
/// flow degrades to "saved, but history not recorded".
#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("history I/O failed at {}: {source}", path.display())]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("history manifest {} is corrupt: {source}", path.display())]
    Manifest {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("version {id} has no snapshot blob (sha256 {sha})")]
    MissingBlob { id: u64, sha: String },
    #[error("no version with id {id}")]
    UnknownVersion { id: u64 },
}

/// Everything [`record_version`] needs besides the bytes.
#[derive(Debug, Clone)]
pub struct RecordParams {
    /// Seconds since the Unix epoch.
    pub timestamp: u64,
    pub origin: Origin,
    pub parent_id: Option<u64>,
    pub label: Option<String>,
    /// Prune unnamed oldest-first past this count (`None` = keep all).
    pub max_versions: Option<usize>,
}

/// The history directory for `file`: `<full file name>.history` beside it.
pub fn history_dir(file: &Path) -> PathBuf {
    let name = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save".to_owned());
    file.with_file_name(format!("{name}.history"))
}

fn manifest_path(file: &Path) -> PathBuf {
    history_dir(file).join("manifest.json")
}

fn versions_dir(file: &Path) -> PathBuf {
    history_dir(file).join("versions")
}

fn blob_path_for(file: &Path, sha: &str) -> PathBuf {
    versions_dir(file).join(format!("{sha}.srm"))
}

fn io_err(path: &Path, source: std::io::Error) -> HistoryError {
    HistoryError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// Write the blob for `bytes` if it is not already stored (dedup by
/// content address). Atomic (temp + fsync + rename), so an existing
/// blob file is always complete. Returns the hex sha.
fn write_blob(file: &Path, bytes: &[u8]) -> Result<String, HistoryError> {
    let dir = versions_dir(file);
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    let sha = sha256_hex(bytes);
    let blob = blob_path_for(file, &sha);
    if !blob.exists() {
        write_atomic(&blob, bytes)?;
    }
    Ok(sha)
}

/// Persist the manifest atomically (temp + fsync + rename).
fn store_manifest(file: &Path, manifest: &Manifest) -> Result<(), HistoryError> {
    let dir = history_dir(file);
    std::fs::create_dir_all(&dir).map_err(|e| io_err(&dir, e))?;
    let path = manifest_path(file);
    let json = serde_json::to_vec_pretty(manifest).map_err(|source| HistoryError::Manifest {
        path: path.clone(),
        source,
    })?;
    write_atomic(&path, &json)
}

/// Best-effort GC: remove blobs (and stale temp files) in `versions/`
/// that no manifest entry references. Orphans appear when a crash hits
/// between blob write and manifest append; they are harmless and
/// silently collected here.
fn gc_orphan_blobs(file: &Path, manifest: &Manifest) {
    let Ok(entries) = std::fs::read_dir(versions_dir(file)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let keep = name
            .strip_suffix(".srm")
            .is_some_and(|sha| manifest.references(sha));
        if !keep {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Record a snapshot of `bytes` as a new version of `file` (which need
/// not exist yet — this runs *before* the save file is written).
pub fn record_version(
    file: &Path,
    bytes: &[u8],
    params: &RecordParams,
) -> Result<VersionEntry, HistoryError> {
    // Load the manifest first: a corrupt manifest must fail the whole
    // operation before anything is written.
    let mut manifest = load_manifest(file)?;
    let sha = write_blob(file, bytes)?;
    let entry = VersionEntry {
        id: manifest.next_id(),
        timestamp: params.timestamp,
        label: params.label.clone(),
        sha256: sha,
        size: bytes.len() as u64,
        parent_id: params.parent_id,
        origin: params.origin,
    };
    manifest.versions.push(entry.clone());
    if let Some(max) = params.max_versions {
        // At least the version just recorded is always kept.
        manifest.prune_unnamed_oldest(max.max(1));
    }
    store_manifest(file, &manifest)?;
    gc_orphan_blobs(file, &manifest);
    Ok(entry)
}

/// Load the manifest; a missing file is an empty manifest, a corrupt
/// one is an error (never a panic).
pub fn load_manifest(file: &Path) -> Result<Manifest, HistoryError> {
    let path = manifest_path(file);
    let json = match std::fs::read(&path) {
        Ok(json) => json,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Manifest::default()),
        Err(e) => return Err(io_err(&path, e)),
    };
    serde_json::from_slice(&json).map_err(|source| HistoryError::Manifest { path, source })
}

/// All versions as display rows (blob presence checked, trainer
/// summaries parsed lazily from the blobs).
pub fn list_versions(file: &Path) -> Result<Vec<VersionRow>, HistoryError> {
    let manifest = load_manifest(file)?;
    Ok(manifest
        .versions
        .into_iter()
        .map(|entry| {
            // A manifest entry pointing at a missing blob surfaces as
            // an error row instead of crashing or being dropped.
            let blob = std::fs::read(blob_path_for(file, &entry.sha256)).ok();
            VersionRow {
                blob_ok: blob.is_some(),
                summary: blob.as_deref().and_then(summarize),
                entry,
            }
        })
        .collect())
}

/// The snapshot bytes of version `id`.
pub fn load_blob(file: &Path, id: u64) -> Result<Vec<u8>, HistoryError> {
    let manifest = load_manifest(file)?;
    let entry = manifest
        .find(id)
        .ok_or(HistoryError::UnknownVersion { id })?;
    std::fs::read(blob_path_for(file, &entry.sha256)).map_err(|_| HistoryError::MissingBlob {
        id,
        sha: entry.sha256.clone(),
    })
}

/// Set or clear the name of version `id`.
pub fn set_label(file: &Path, id: u64, label: Option<String>) -> Result<(), HistoryError> {
    let mut manifest = load_manifest(file)?;
    let entry = manifest
        .versions
        .iter_mut()
        .find(|v| v.id == id)
        .ok_or(HistoryError::UnknownVersion { id })?;
    entry.label = label;
    store_manifest(file, &manifest)
}

/// Delete version `id` and GC its blob if unreferenced.
pub fn delete_version(file: &Path, id: u64) -> Result<(), HistoryError> {
    let mut manifest = load_manifest(file)?;
    if manifest.find(id).is_none() {
        return Err(HistoryError::UnknownVersion { id });
    }
    manifest.versions.retain(|v| v.id != id);
    store_manifest(file, &manifest)?;
    gc_orphan_blobs(file, &manifest);
    Ok(())
}

/// Prune unnamed versions oldest-first down to `max`. Returns how many
/// were removed.
pub fn prune(file: &Path, max: usize) -> Result<usize, HistoryError> {
    let mut manifest = load_manifest(file)?;
    let removed = manifest.prune_unnamed_oldest(max);
    if !removed.is_empty() {
        store_manifest(file, &manifest)?;
        gc_orphan_blobs(file, &manifest);
    }
    Ok(removed.len())
}

/// Parse the `YYYYMMDD-HHMMSS[-N]` timestamp portion of a legacy backup
/// name (the text after `.bak-`) into seconds since the Unix epoch.
/// The disambiguating `-N` counter (same-second backups) is accepted
/// and ignored.
pub fn parse_backup_timestamp(text: &str) -> Option<u64> {
    let (stamp, counter) = match text.split_at_checked(15) {
        Some((stamp, rest)) => (stamp, rest),
        None => return None,
    };
    if !counter.is_empty() {
        let digits = counter.strip_prefix('-')?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
    }
    let digits: &str = stamp;
    let num = |range: core::ops::Range<usize>| -> Option<u64> {
        let part = digits.get(range)?;
        if !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        part.parse().ok()
    };
    if digits.as_bytes().get(8) != Some(&b'-') {
        return None;
    }
    let year = num(0..4)?;
    let month = num(4..6)?;
    let day = num(6..8)?;
    let hour = num(9..11)?;
    let minute = num(11..13)?;
    let second = num(13..15)?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days = days_from_civil(year as i64, month as u32, day as u32);
    u64::try_from(days * 86_400 + (hour * 3_600 + minute * 60 + second) as i64).ok()
}

/// (year, month, day) to days since the Unix epoch — Howard Hinnant's
/// `days_from_civil`, the inverse of `io::civil_from_days`.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = year - i64::from(month <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i64::from(if month > 2 { month - 3 } else { month + 9 }); // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(day) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// Legacy `.bak-*` siblings of `file` with a parseable timestamp,
/// sorted oldest-first.
pub fn legacy_backups(file: &Path) -> Vec<(PathBuf, u64)> {
    let (Some(parent), Some(name)) = (file.parent(), file.file_name()) else {
        return Vec::new();
    };
    let prefix = format!("{}.bak-", name.to_string_lossy());
    let Ok(entries) = std::fs::read_dir(parent) else {
        return Vec::new();
    };
    let mut found: Vec<(PathBuf, u64)> = entries
        .flatten()
        .filter_map(|entry| {
            let file_name = entry.file_name().to_string_lossy().into_owned();
            let stamp = file_name.strip_prefix(&prefix)?;
            let secs = parse_backup_timestamp(stamp)?;
            entry.path().is_file().then(|| (entry.path(), secs))
        })
        .collect();
    found.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    found
}

/// Import every legacy `.bak-*` sibling into the history (origin
/// [`Origin::Import`], timestamp parsed from the filename). Backups
/// whose bytes are already in the history (same sha256) are skipped, so
/// importing is idempotent. Returns how many were imported.
pub fn import_legacy(file: &Path) -> Result<usize, HistoryError> {
    let mut manifest = load_manifest(file)?;
    let mut imported = 0;
    for (backup, timestamp) in legacy_backups(file) {
        let bytes = std::fs::read(&backup).map_err(|e| io_err(&backup, e))?;
        if manifest.references(&sha256_hex(&bytes)) {
            continue; // already in the history (idempotent re-import)
        }
        let sha = write_blob(file, &bytes)?;
        manifest.versions.push(VersionEntry {
            id: manifest.next_id(),
            timestamp,
            label: None,
            sha256: sha,
            size: bytes.len() as u64,
            parent_id: None,
            origin: Origin::Import,
        });
        imported += 1;
    }
    if imported > 0 {
        store_manifest(file, &manifest)?;
        gc_orphan_blobs(file, &manifest);
    }
    Ok(imported)
}

/// The native [`HistoryStore`]: synchronous filesystem calls, results
/// sent over the events channel before the method returns.
pub struct FsStore {
    file: PathBuf,
    tx: Sender<HistoryEvent>,
}

impl FsStore {
    pub fn new(file: PathBuf, tx: Sender<HistoryEvent>) -> FsStore {
        FsStore { file, tx }
    }

    fn send(&self, event: HistoryEvent) {
        let _ = self.tx.send(event);
    }

    /// Send the refreshed version list (after every mutation).
    fn refresh(&self) {
        match list_versions(&self.file) {
            Ok(rows) => self.send(HistoryEvent::Versions(rows)),
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }
}

impl HistoryStore for FsStore {
    fn record(
        &mut self,
        bytes: Vec<u8>,
        origin: Origin,
        parent_id: Option<u64>,
        max_versions: Option<usize>,
    ) {
        let params = RecordParams {
            timestamp: crate::io::now_secs(),
            origin,
            parent_id,
            label: None,
            max_versions,
        };
        match record_version(&self.file, &bytes, &params) {
            Ok(entry) => {
                self.send(HistoryEvent::Recorded(entry));
                self.refresh();
            }
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }

    fn list(&mut self) {
        self.refresh();
    }

    fn load_blob(&mut self, id: u64, purpose: BlobPurpose) {
        match load_blob(&self.file, id) {
            Ok(bytes) => self.send(HistoryEvent::BlobLoaded { id, purpose, bytes }),
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }

    fn set_label(&mut self, id: u64, label: Option<String>) {
        match set_label(&self.file, id, label) {
            Ok(()) => self.refresh(),
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }

    fn delete(&mut self, id: u64) {
        match delete_version(&self.file, id) {
            Ok(()) => self.refresh(),
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }

    fn prune(&mut self, max_versions: usize) {
        match prune(&self.file, max_versions) {
            Ok(_) => self.refresh(),
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }

    fn import_legacy(&mut self) {
        match import_legacy(&self.file) {
            Ok(count) => {
                self.send(HistoryEvent::LegacyImported { count });
                self.refresh();
            }
            Err(e) => self.send(HistoryEvent::Error(e.to_string())),
        }
    }
}

/// Write `bytes` to `target` atomically: temp file in the same
/// directory, fsync, rename over the target.
fn write_atomic(target: &Path, bytes: &[u8]) -> Result<(), HistoryError> {
    let io_err = |path: &Path, source| HistoryError::Io {
        path: path.to_path_buf(),
        source,
    };
    let name = target
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "history".to_owned());
    let temp = target.with_file_name(format!(".{name}.tmp"));
    let mut out = std::fs::File::create(&temp).map_err(|e| io_err(&temp, e))?;
    out.write_all(bytes)
        .and_then(|()| out.sync_all())
        .map_err(|e| io_err(&temp, e))?;
    drop(out);
    std::fs::rename(&temp, target).map_err(|e| io_err(target, e))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::sync::mpsc::channel;

    fn params(timestamp: u64) -> RecordParams {
        RecordParams {
            timestamp,
            origin: Origin::Save,
            parent_id: None,
            label: None,
            max_versions: None,
        }
    }

    /// A save-file path in a tempdir (the file itself need not exist —
    /// history is written before the save file).
    fn save_path(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("poke.srm")
    }

    fn blob_path(file: &Path, sha: &str) -> PathBuf {
        history_dir(file)
            .join("versions")
            .join(format!("{sha}.srm"))
    }

    fn valid_save_bytes() -> Vec<u8> {
        pksave::gen1::save::SaveFile::new_empty(pksave::gen1::save::GameVariant::RedBlue).to_bytes()
    }

    #[test]
    fn history_dir_appends_to_the_full_file_name() {
        assert_eq!(
            history_dir(Path::new("/saves/poke.srm")),
            PathBuf::from("/saves/poke.srm.history")
        );
        assert_eq!(
            history_dir(Path::new("/saves/poke.sav")),
            PathBuf::from("/saves/poke.sav.history")
        );
    }

    #[test]
    fn record_writes_blob_and_manifest_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        let bytes = valid_save_bytes();
        let entry = record_version(&file, &bytes, &params(42)).expect("record");
        assert_eq!(entry.id, 1);
        assert_eq!(entry.timestamp, 42);
        assert_eq!(entry.origin, Origin::Save);
        assert_eq!(entry.parent_id, None);
        assert_eq!(entry.label, None);
        assert_eq!(entry.size, bytes.len() as u64);
        assert_eq!(entry.sha256, sha256_hex(&bytes));

        // The blob holds the exact snapshot bytes.
        assert_eq!(
            fs::read(blob_path(&file, &entry.sha256)).expect("blob"),
            bytes
        );
        // The manifest round-trips with the entry.
        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(manifest.versions, vec![entry]);
        // No temp files were left behind.
        let versions_dir = history_dir(&file).join("versions");
        assert_eq!(fs::read_dir(&versions_dir).expect("readdir").count(), 1);
    }

    #[test]
    fn identical_bytes_share_one_blob_but_keep_the_timeline_complete() {
        // A no-change save appends a manifest entry pointing at the
        // existing blob (chosen over skipping — the timeline stays
        // complete; see the module docs).
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        let bytes = valid_save_bytes();
        let first = record_version(&file, &bytes, &params(1)).expect("first");
        let second = record_version(&file, &bytes, &params(2)).expect("second");
        assert_eq!(first.sha256, second.sha256, "content-addressed");
        assert_eq!(second.id, 2);
        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(manifest.versions.len(), 2);
        // Exactly one blob on disk.
        let versions_dir = history_dir(&file).join("versions");
        assert_eq!(fs::read_dir(&versions_dir).expect("readdir").count(), 1);
    }

    #[test]
    fn missing_history_is_an_empty_manifest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        assert_eq!(load_manifest(&file).expect("empty"), Manifest::default());
        assert_eq!(list_versions(&file).expect("empty"), Vec::new());
    }

    #[test]
    fn corrupt_manifest_is_an_error_not_a_panic() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        fs::create_dir_all(history_dir(&file)).expect("mkdir");
        fs::write(history_dir(&file).join("manifest.json"), b"{ not json").expect("seed");
        assert!(matches!(
            load_manifest(&file),
            Err(HistoryError::Manifest { .. })
        ));
        // Recording against a corrupt manifest fails cleanly too (the
        // save flow then degrades to "history not recorded").
        assert!(record_version(&file, b"x", &params(1)).is_err());
    }

    #[test]
    fn orphan_blob_from_a_simulated_crash_is_ignored_and_gcd() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"v1", &params(1)).expect("record");

        // Simulated crash between blob write and manifest append: the
        // blob exists, the manifest never mentions it.
        let orphan_sha = sha256_hex(b"crashed");
        let orphan = blob_path(&file, &orphan_sha);
        fs::write(&orphan, b"crashed").expect("orphan");

        // The manifest is still valid and the orphan is invisible.
        let rows = list_versions(&file).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(rows[0].blob_ok);

        // The next mutation GCs the orphan.
        record_version(&file, b"v2", &params(2)).expect("record");
        assert!(!orphan.exists(), "orphan blob was garbage-collected");
        // Both real blobs survive.
        assert!(blob_path(&file, &sha256_hex(b"v1")).exists());
        assert!(blob_path(&file, &sha256_hex(b"v2")).exists());
    }

    #[test]
    fn manifest_entry_with_missing_blob_surfaces_as_error_row() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        let entry = record_version(&file, b"gone", &params(1)).expect("record");
        fs::remove_file(blob_path(&file, &entry.sha256)).expect("sabotage");

        // No crash: the row is flagged, not dropped.
        let rows = list_versions(&file).expect("list");
        assert_eq!(rows.len(), 1);
        assert!(!rows[0].blob_ok);
        assert_eq!(rows[0].summary, None);

        // Loading it reports the missing blob.
        assert!(matches!(
            load_blob(&file, entry.id),
            Err(HistoryError::MissingBlob { id: 1, .. })
        ));
    }

    #[test]
    fn rows_carry_trainer_summaries_parsed_from_blobs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, &valid_save_bytes(), &params(1)).expect("record");
        let rows = list_versions(&file).expect("list");
        let summary = rows[0].summary.as_deref().expect("summary");
        assert!(summary.contains("RED"), "summary: {summary}");
    }

    #[test]
    fn restore_lineage_is_recorded() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"v1", &params(1)).expect("v1");
        let restored = record_version(
            &file,
            b"v1 edited",
            &RecordParams {
                timestamp: 9,
                origin: Origin::Restore,
                parent_id: Some(1),
                label: None,
                max_versions: None,
            },
        )
        .expect("restore save");
        assert_eq!(restored.origin, Origin::Restore);
        assert_eq!(restored.parent_id, Some(1));
        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(manifest.find(2).expect("entry 2").parent_id, Some(1));
    }

    #[test]
    fn record_prunes_unnamed_oldest_first_and_gcs_their_blobs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"v1", &params(1)).expect("v1");
        record_version(&file, b"v2", &params(2)).expect("v2");
        set_label(&file, 2, Some("named".to_owned())).expect("name v2");
        record_version(
            &file,
            b"v3",
            &RecordParams {
                max_versions: Some(2),
                ..params(3)
            },
        )
        .expect("v3");

        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(
            manifest.versions.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![2, 3],
            "v1 (unnamed, oldest) was pruned; the named v2 survived"
        );
        assert!(!blob_path(&file, &sha256_hex(b"v1")).exists());
        assert!(blob_path(&file, &sha256_hex(b"v2")).exists());
    }

    #[test]
    fn pruning_spares_shared_blobs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"same", &params(1)).expect("v1");
        record_version(&file, b"same", &params(2)).expect("v2");
        record_version(&file, b"new", &params(3)).expect("v3");
        // Prune to 2: v1 goes, but its blob is still referenced by v2.
        assert_eq!(prune(&file, 2).expect("prune"), 1);
        assert!(blob_path(&file, &sha256_hex(b"same")).exists());
        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(
            manifest.versions.iter().map(|v| v.id).collect::<Vec<_>>(),
            vec![2, 3]
        );
    }

    #[test]
    fn set_label_renames_and_clears() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"v1", &params(1)).expect("v1");
        set_label(&file, 1, Some("before badge edit".to_owned())).expect("name");
        assert_eq!(
            load_manifest(&file).expect("m").find(1).expect("e").label,
            Some("before badge edit".to_owned())
        );
        set_label(&file, 1, None).expect("clear");
        assert_eq!(
            load_manifest(&file).expect("m").find(1).expect("e").label,
            None
        );
        assert!(matches!(
            set_label(&file, 99, None),
            Err(HistoryError::UnknownVersion { id: 99 })
        ));
    }

    #[test]
    fn delete_removes_the_entry_and_gcs_the_blob() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"v1", &params(1)).expect("v1");
        record_version(&file, b"v2", &params(2)).expect("v2");
        delete_version(&file, 1).expect("delete");
        assert!(!blob_path(&file, &sha256_hex(b"v1")).exists());
        assert!(blob_path(&file, &sha256_hex(b"v2")).exists());
        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(manifest.versions.len(), 1);
        assert!(matches!(
            delete_version(&file, 1),
            Err(HistoryError::UnknownVersion { id: 1 })
        ));
    }

    // ---- legacy .bak import ---------------------------------------------

    #[test]
    fn backup_timestamps_parse_and_round_trip() {
        // Round-trips with the io::backup_timestamp formatter.
        for secs in [0u64, 951_868_800, 1_784_118_896, 1_709_251_199, 86_399] {
            let text = crate::io::backup_timestamp(secs);
            assert_eq!(parse_backup_timestamp(&text), Some(secs), "{text}");
        }
        // The same-second disambiguating counter is accepted.
        assert_eq!(
            parse_backup_timestamp("20260715-123456-2"),
            Some(1_784_118_896)
        );
        assert_eq!(
            parse_backup_timestamp("20260715-123456-13"),
            Some(1_784_118_896)
        );
    }

    #[test]
    fn garbage_timestamps_are_rejected() {
        for bad in [
            "",
            "2026",
            "20260715",
            "20260715-1234",
            "2026x715-123456",
            "20261315-123456", // month 13
            "20260732-123456", // day 32
            "20260715-243456", // hour 24
            "20260715-126056", // minute 60
            "20260715-123460", // second 60
            "20260715-123456-",
            "20260715-123456-x",
            "20260715-123456extra",
        ] {
            assert_eq!(parse_backup_timestamp(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn legacy_backups_finds_and_sorts_siblings() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        fs::write(&file, b"current").expect("save");
        fs::write(dir.path().join("poke.srm.bak-20260715-123456"), b"new").expect("bak");
        fs::write(dir.path().join("poke.srm.bak-20000301-000000"), b"old").expect("bak");
        // Non-matching siblings are ignored.
        fs::write(dir.path().join("poke.srm.bak-garbage"), b"x").expect("bak");
        fs::write(dir.path().join("other.srm.bak-20260715-123456"), b"x").expect("bak");

        let found = legacy_backups(&file);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].1, 951_868_800, "sorted oldest-first");
        assert_eq!(found[1].1, 1_784_118_896);
    }

    #[test]
    fn import_legacy_creates_import_entries_with_parsed_timestamps() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        fs::write(dir.path().join("poke.srm.bak-20260715-123456"), b"newer").expect("bak");
        fs::write(dir.path().join("poke.srm.bak-20000301-000000"), b"older").expect("bak");

        assert_eq!(import_legacy(&file).expect("import"), 2);
        let manifest = load_manifest(&file).expect("manifest");
        assert_eq!(manifest.versions.len(), 2);
        for v in &manifest.versions {
            assert_eq!(v.origin, Origin::Import);
            assert_eq!(v.label, None);
            assert_eq!(v.parent_id, None);
        }
        // Oldest-first, so ids follow the timeline.
        assert_eq!(manifest.versions[0].timestamp, 951_868_800);
        assert_eq!(manifest.versions[1].timestamp, 1_784_118_896);
        assert_eq!(load_blob(&file, 1).expect("blob"), b"older");

        // Idempotent: a second import adds nothing.
        assert_eq!(import_legacy(&file).expect("re-import"), 0);
        assert_eq!(load_manifest(&file).expect("m").versions.len(), 2);
    }

    // ---- the store adapter (events over the channel) ----------------------

    #[test]
    fn store_record_sends_recorded_and_versions_events() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        let (tx, rx) = channel();
        let mut store = FsStore::new(file, tx);
        store.record(b"v1".to_vec(), Origin::Save, None, None);
        match rx.try_recv().expect("recorded event") {
            HistoryEvent::Recorded(entry) => assert_eq!(entry.id, 1),
            other => panic!("expected Recorded, got {other:?}"),
        }
        match rx.try_recv().expect("versions event") {
            HistoryEvent::Versions(rows) => assert_eq!(rows.len(), 1),
            other => panic!("expected Versions, got {other:?}"),
        }
    }

    #[test]
    fn store_reports_errors_as_events_not_panics() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        let (tx, rx) = channel();
        let mut store = FsStore::new(file, tx);
        store.load_blob(42, BlobPurpose::Restore);
        assert!(matches!(
            rx.try_recv().expect("event"),
            HistoryEvent::Error(_)
        ));
    }

    #[test]
    fn store_load_blob_round_trips_with_purpose() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        record_version(&file, b"snapshot", &params(1)).expect("record");
        let (tx, rx) = channel();
        let mut store = FsStore::new(file, tx);
        store.load_blob(1, BlobPurpose::Diff);
        match rx.try_recv().expect("event") {
            HistoryEvent::BlobLoaded { id, purpose, bytes } => {
                assert_eq!(id, 1);
                assert_eq!(purpose, BlobPurpose::Diff);
                assert_eq!(bytes, b"snapshot");
            }
            other => panic!("expected BlobLoaded, got {other:?}"),
        }
    }

    #[test]
    fn store_import_legacy_reports_the_count() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = save_path(&dir);
        fs::write(dir.path().join("poke.srm.bak-20260715-123456"), b"old").expect("bak");
        let (tx, rx) = channel();
        let mut store = FsStore::new(file, tx);
        store.import_legacy();
        match rx.try_recv().expect("event") {
            HistoryEvent::LegacyImported { count } => assert_eq!(count, 1),
            other => panic!("expected LegacyImported, got {other:?}"),
        }
        match rx.try_recv().expect("refresh") {
            HistoryEvent::Versions(rows) => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].entry.origin, Origin::Import);
            }
            other => panic!("expected Versions, got {other:?}"),
        }
    }
}
