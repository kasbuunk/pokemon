//! wasm history backend: IndexedDB via raw web-sys bindings.
//!
//! Thin async adapter over the same manifest semantics as the native
//! [`super::fs`] store (which carries the unit tests). Versions are
//! keyed by save identity = `sha256(original load bytes) + filename`:
//!
//! - object store `manifests`: `identity` → manifest JSON string
//! - object store `blobs`: `identity/<sha256>` → snapshot bytes
//!   (content-addressed within the save's namespace, like the native
//!   `versions/<sha256>.srm` files)
//!
//! Every operation runs in one IndexedDB transaction and reports back
//! over the shared [`HistoryEvent`] channel, requesting a repaint (the
//! app's mpsc→frame-loop pattern).
//!
//! Quota: when a record's transaction aborts (typically
//! `QuotaExceededError`), the oldest half of the *unnamed* versions is
//! evicted and the record retried once; a second failure surfaces as a
//! toast via [`HistoryEvent::Error`].
//!
//! Callback hygiene: handlers are one-shot [`Closure::once_into_js`]
//! closures, which free themselves when invoked. Of an
//! oncomplete/onabort pair only one ever fires, so the other leaks a
//! few bytes per operation — a deliberate, bounded trade-off to keep
//! this adapter free of future plumbing.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::mpsc::Sender;

use eframe::wasm_bindgen::closure::Closure;
use eframe::wasm_bindgen::{JsCast as _, JsValue};
use web_sys::{
    IdbDatabase, IdbKeyRange, IdbObjectStore, IdbRequest, IdbTransaction, IdbTransactionMode,
};

use super::{
    sha256_hex, summarize, BlobPurpose, HistoryEvent, HistoryStore, Manifest, Origin, VersionEntry,
    VersionRow,
};

// Stable on purpose: the app was renamed to "Pokémon SRM Editor" but
// this IndexedDB key must not change — renaming it would orphan every
// existing user's save-version history.
const DB_NAME: &str = "pksave-history";
const DB_VERSION: u32 = 1;
const MANIFESTS: &str = "manifests";
const BLOBS: &str = "blobs";

/// Everything the async callbacks need: the save identity, the events
/// channel and the egui context for repaints.
#[derive(Clone)]
struct Ops {
    identity: String,
    tx: Sender<HistoryEvent>,
    ctx: egui::Context,
}

impl Ops {
    fn emit(&self, event: HistoryEvent) {
        let _ = self.tx.send(event);
        self.ctx.request_repaint();
    }

    fn fail(&self, what: &str, err: JsValue) {
        self.emit(HistoryEvent::Error(format!("{what}: {err:?}")));
    }

    fn manifest_key(&self) -> JsValue {
        JsValue::from_str(&self.identity)
    }

    fn blob_key(&self, sha: &str) -> JsValue {
        JsValue::from_str(&format!("{}/{sha}", self.identity))
    }

    /// Key range covering every blob of this save's namespace.
    fn blob_range(&self) -> Result<IdbKeyRange, JsValue> {
        IdbKeyRange::bound(
            &JsValue::from_str(&format!("{}/", self.identity)),
            &JsValue::from_str(&format!("{}/\u{10FFFF}", self.identity)),
        )
    }
}

/// The wasm [`HistoryStore`]: a thin dispatcher; all logic lives in the
/// `run_*` free functions below.
pub struct IdbStore {
    ops: Ops,
}

impl IdbStore {
    pub fn new(identity: String, tx: Sender<HistoryEvent>, ctx: egui::Context) -> IdbStore {
        IdbStore {
            ops: Ops { identity, tx, ctx },
        }
    }
}

impl HistoryStore for IdbStore {
    fn record(
        &mut self,
        bytes: Vec<u8>,
        origin: Origin,
        parent_id: Option<u64>,
        max_versions: Option<usize>,
    ) {
        let request = RecordRequest {
            bytes,
            origin,
            parent_id,
            max_versions,
            allow_retry: true,
        };
        with_db(self.ops.clone(), move |db, ops| {
            run_record(db, ops, request);
        });
    }

    fn list(&mut self) {
        with_db(self.ops.clone(), run_list);
    }

    fn load_blob(&mut self, id: u64, purpose: BlobPurpose) {
        with_db(self.ops.clone(), move |db, ops| {
            run_load(&db, &ops, id, purpose);
        });
    }

    fn set_label(&mut self, id: u64, label: Option<String>) {
        with_db(self.ops.clone(), move |db, ops| {
            run_update(db, ops, move |manifest| {
                let entry = manifest
                    .versions
                    .iter_mut()
                    .find(|v| v.id == id)
                    .ok_or_else(|| format!("no version with id {id}"))?;
                entry.label = label;
                Ok(Vec::new())
            });
        });
    }

    fn delete(&mut self, id: u64) {
        with_db(self.ops.clone(), move |db, ops| {
            run_update(db, ops, move |manifest| {
                if manifest.find(id).is_none() {
                    return Err(format!("no version with id {id}"));
                }
                let removed = manifest
                    .versions
                    .iter()
                    .filter(|v| v.id == id)
                    .map(|v| v.sha256.clone())
                    .collect();
                manifest.versions.retain(|v| v.id != id);
                Ok(removed)
            });
        });
    }

    fn prune(&mut self, max_versions: usize) {
        with_db(self.ops.clone(), move |db, ops| {
            run_update(db, ops, move |manifest| {
                Ok(manifest
                    .prune_unnamed_oldest(max_versions)
                    .into_iter()
                    .map(|v| v.sha256)
                    .collect())
            });
        });
    }

    fn import_legacy(&mut self) {
        // No legacy `.bak-*` siblings exist in the browser; the app
        // never offers the import there. Nothing to do.
    }
}

/// One record operation, kept whole so a quota retry can rerun it.
#[derive(Clone)]
struct RecordRequest {
    bytes: Vec<u8>,
    origin: Origin,
    parent_id: Option<u64>,
    max_versions: Option<usize>,
    allow_retry: bool,
}

fn now_secs() -> u64 {
    (js_sys::Date::now() / 1000.0) as u64
}

/// Run `f(request.result())` when the request succeeds (one-shot).
fn on_success(request: &IdbRequest, f: impl FnOnce(Result<JsValue, JsValue>) + 'static) {
    let request2 = request.clone();
    let callback = Closure::once_into_js(move |_: web_sys::Event| f(request2.result()));
    request.set_onsuccess(Some(callback.unchecked_ref()));
}

/// Run `f` when the transaction commits (one-shot).
fn on_complete(tx: &IdbTransaction, f: impl FnOnce() + 'static) {
    let callback = Closure::once_into_js(move |_: web_sys::Event| f());
    tx.set_oncomplete(Some(callback.unchecked_ref()));
}

/// Run `f` when the transaction aborts — which is also what an
/// unhandled request error (e.g. `QuotaExceededError`) does (one-shot).
fn on_abort(tx: &IdbTransaction, f: impl FnOnce() + 'static) {
    let callback = Closure::once_into_js(move |_: web_sys::Event| f());
    tx.set_onabort(Some(callback.unchecked_ref()));
}

/// Open (and on first use create) the database, then hand it to `then`.
fn with_db(ops: Ops, then: impl FnOnce(IdbDatabase, Ops) + 'static) {
    let Some(factory) = web_sys::window().and_then(|w| w.indexed_db().ok().flatten()) else {
        return ops.fail("IndexedDB unavailable", JsValue::UNDEFINED);
    };
    let request = match factory.open_with_u32(DB_NAME, DB_VERSION) {
        Ok(request) => request,
        Err(e) => return ops.fail("could not open the history database", e),
    };

    let upgrade_request = request.clone();
    let onupgrade = Closure::once_into_js(move |_: web_sys::Event| {
        if let Ok(db) = upgrade_request
            .result()
            .and_then(|value| value.dyn_into::<IdbDatabase>())
        {
            // First run: create both object stores.
            let _ = db.create_object_store(MANIFESTS);
            let _ = db.create_object_store(BLOBS);
        }
    });
    request.set_onupgradeneeded(Some(onupgrade.unchecked_ref()));

    let error_ops = ops.clone();
    let onerror = Closure::once_into_js(move |_: web_sys::Event| {
        error_ops.fail("could not open the history database", JsValue::UNDEFINED);
    });
    request.set_onerror(Some(onerror.unchecked_ref()));

    let success_request = request.clone();
    let onsuccess = Closure::once_into_js(move |_: web_sys::Event| {
        match success_request
            .result()
            .and_then(|value| value.dyn_into::<IdbDatabase>())
        {
            Ok(db) => then(db, ops),
            Err(e) => ops.fail("could not open the history database", e),
        }
    });
    request.set_onsuccess(Some(onsuccess.unchecked_ref()));
}

/// A read-write or read-only transaction over both object stores.
fn transaction(db: &IdbDatabase, ops: &Ops, mode: IdbTransactionMode) -> Option<IdbTransaction> {
    let names = js_sys::Array::of2(&JsValue::from_str(MANIFESTS), &JsValue::from_str(BLOBS));
    match db.transaction_with_str_sequence_and_mode(&names, mode) {
        Ok(tx) => Some(tx),
        Err(e) => {
            ops.fail("could not open a history transaction", e);
            None
        }
    }
}

fn object_store(tx: &IdbTransaction, name: &str, ops: &Ops) -> Option<IdbObjectStore> {
    match tx.object_store(name) {
        Ok(store) => Some(store),
        Err(e) => {
            ops.fail("could not open a history object store", e);
            None
        }
    }
}

/// The stored manifest value (a JSON string; absent = empty manifest).
fn parse_manifest(result: Result<JsValue, JsValue>) -> Result<Manifest, String> {
    let value = result.map_err(|e| format!("could not read the history manifest: {e:?}"))?;
    if value.is_undefined() || value.is_null() {
        return Ok(Manifest::default());
    }
    let text = value
        .as_string()
        .ok_or_else(|| "history manifest is not a string".to_owned())?;
    serde_json::from_str(&text).map_err(|e| format!("history manifest is corrupt: {e}"))
}

/// Queue the manifest write on an open read-write transaction.
fn put_manifest(store: &IdbObjectStore, ops: &Ops, manifest: &Manifest) -> Result<(), String> {
    let json =
        serde_json::to_string(manifest).map_err(|e| format!("could not encode manifest: {e}"))?;
    store
        .put_with_key(&JsValue::from_str(&json), &ops.manifest_key())
        .map_err(|e| format!("could not write manifest: {e:?}"))?;
    Ok(())
}

/// Queue deletion of every blob in `shas` that `manifest` no longer
/// references (content-addressed blobs may be shared between versions).
fn delete_unreferenced(store: &IdbObjectStore, ops: &Ops, manifest: &Manifest, shas: &[String]) {
    for sha in shas {
        if !manifest.references(sha) {
            let _ = store.delete(&ops.blob_key(sha));
        }
    }
}

/// List every version with blob presence and lazily parsed summaries;
/// emits [`HistoryEvent::Versions`].
fn run_list(db: IdbDatabase, ops: Ops) {
    let Some(tx) = transaction(&db, &ops, IdbTransactionMode::Readonly) else {
        return;
    };
    let (Some(manifests), Some(blobs)) = (
        object_store(&tx, MANIFESTS, &ops),
        object_store(&tx, BLOBS, &ops),
    ) else {
        return;
    };
    let range = match ops.blob_range() {
        Ok(range) => range,
        Err(e) => return ops.fail("could not build the blob key range", e),
    };
    let manifest_get = match manifests.get(&ops.manifest_key()) {
        Ok(request) => request,
        Err(e) => return ops.fail("could not read the manifest", e),
    };
    let keys_get = match blobs.get_all_keys_with_key(&JsValue::from(range.clone())) {
        Ok(request) => request,
        Err(e) => return ops.fail("could not list snapshot keys", e),
    };
    let values_get = match blobs.get_all_with_key(&JsValue::from(range)) {
        Ok(request) => request,
        Err(e) => return ops.fail("could not list snapshots", e),
    };

    // All three results are available once the transaction completes;
    // getAllKeys and getAll return arrays in the same (key) order.
    on_complete(&tx, move || {
        let manifest = match parse_manifest(manifest_get.result()) {
            Ok(manifest) => manifest,
            Err(message) => return ops.emit(HistoryEvent::Error(message)),
        };
        let as_array = |request: &IdbRequest| -> js_sys::Array {
            request
                .result()
                .ok()
                .and_then(|value| value.dyn_into::<js_sys::Array>().ok())
                .unwrap_or_default()
        };
        let keys = as_array(&keys_get);
        let values = as_array(&values_get);
        let prefix = format!("{}/", ops.identity);
        let mut blobs_by_sha: HashMap<String, Vec<u8>> = HashMap::new();
        for (key, value) in keys.iter().zip(values.iter()) {
            let Some(sha) = key
                .as_string()
                .and_then(|k| k.strip_prefix(&prefix).map(str::to_owned))
            else {
                continue;
            };
            blobs_by_sha.insert(sha, js_sys::Uint8Array::new(&value).to_vec());
        }
        let rows = manifest
            .versions
            .into_iter()
            .map(|entry| {
                let blob = blobs_by_sha.get(&entry.sha256);
                VersionRow {
                    blob_ok: blob.is_some(),
                    summary: blob.and_then(|bytes| summarize(bytes)),
                    entry,
                }
            })
            .collect();
        ops.emit(HistoryEvent::Versions(rows));
    });
}

/// Record a snapshot: read manifest → append entry (+ prune) → put the
/// blob and manifest in one transaction. On abort (quota), evict the
/// oldest unnamed versions and retry once.
fn run_record(db: IdbDatabase, ops: Ops, request: RecordRequest) {
    let Some(tx) = transaction(&db, &ops, IdbTransactionMode::Readwrite) else {
        return;
    };
    let (Some(manifests), Some(blobs)) = (
        object_store(&tx, MANIFESTS, &ops),
        object_store(&tx, BLOBS, &ops),
    ) else {
        return;
    };
    let manifest_get = match manifests.get(&ops.manifest_key()) {
        Ok(get) => get,
        Err(e) => return ops.fail("could not read the manifest", e),
    };

    let recorded: Rc<RefCell<Option<VersionEntry>>> = Rc::new(RefCell::new(None));

    {
        // Quota handling: an aborted write evicts and retries once.
        let db = db.clone();
        let ops = ops.clone();
        let retry = RecordRequest {
            allow_retry: false,
            ..request.clone()
        };
        let allow_retry = request.allow_retry;
        on_abort(&tx, move || {
            if allow_retry {
                run_evict_then_record(db, ops, retry);
            } else {
                ops.emit(HistoryEvent::Error(
                    "could not store the version (storage quota?); \
                     evicting old unnamed versions did not help"
                        .to_owned(),
                ));
            }
        });
    }
    {
        let db = db.clone();
        let ops = ops.clone();
        let recorded = recorded.clone();
        on_complete(&tx, move || {
            if let Some(entry) = recorded.borrow_mut().take() {
                ops.emit(HistoryEvent::Recorded(entry));
                run_list(db, ops);
            }
        });
    }

    on_success(&manifest_get, move |result| {
        let mut manifest = match parse_manifest(result) {
            Ok(manifest) => manifest,
            Err(message) => return ops.emit(HistoryEvent::Error(message)),
        };
        let sha = sha256_hex(&request.bytes);
        let entry = VersionEntry {
            id: manifest.next_id(),
            timestamp: now_secs(),
            label: None,
            sha256: sha.clone(),
            size: request.bytes.len() as u64,
            parent_id: request.parent_id,
            origin: request.origin,
        };
        manifest.versions.push(entry.clone());
        let removed: Vec<String> = request
            .max_versions
            // At least the version just recorded is always kept.
            .map(|max| manifest.prune_unnamed_oldest(max.max(1)))
            .unwrap_or_default()
            .into_iter()
            .map(|v| v.sha256)
            .collect();
        *recorded.borrow_mut() = Some(entry);

        // Content-addressed put: rewriting an existing blob key with
        // identical bytes is an idempotent dedup.
        let bytes = js_sys::Uint8Array::from(request.bytes.as_slice());
        if let Err(e) = blobs.put_with_key(&JsValue::from(bytes), &ops.blob_key(&sha)) {
            return ops.fail("could not store the snapshot", e);
        }
        delete_unreferenced(&blobs, &ops, &manifest, &removed);
        if let Err(message) = put_manifest(&manifests, &ops, &manifest) {
            ops.emit(HistoryEvent::Error(message));
        }
    });
}

/// Evict the oldest half of the unnamed versions (at least one), then
/// retry the record. Named versions are never evicted; with nothing
/// evictable the quota failure is surfaced instead.
fn run_evict_then_record(db: IdbDatabase, ops: Ops, request: RecordRequest) {
    let Some(tx) = transaction(&db, &ops, IdbTransactionMode::Readwrite) else {
        return;
    };
    let (Some(manifests), Some(blobs)) = (
        object_store(&tx, MANIFESTS, &ops),
        object_store(&tx, BLOBS, &ops),
    ) else {
        return;
    };
    let manifest_get = match manifests.get(&ops.manifest_key()) {
        Ok(get) => get,
        Err(e) => return ops.fail("could not read the manifest", e),
    };
    {
        let ops = ops.clone();
        on_abort(&tx, move || {
            ops.emit(HistoryEvent::Error(
                "could not evict old versions to free storage".to_owned(),
            ));
        });
    }
    {
        let db = db.clone();
        let ops = ops.clone();
        on_complete(&tx, move || run_record(db, ops, request));
    }
    on_success(&manifest_get, move |result| {
        let mut manifest = match parse_manifest(result) {
            Ok(manifest) => manifest,
            Err(message) => return ops.emit(HistoryEvent::Error(message)),
        };
        let unnamed = manifest
            .versions
            .iter()
            .filter(|v| v.label.is_none())
            .count();
        if unnamed == 0 {
            return ops.emit(HistoryEvent::Error(
                "storage quota exceeded and every stored version is named — \
                 delete versions in the History screen to free space"
                    .to_owned(),
            ));
        }
        let keep = manifest.versions.len() - unnamed.div_ceil(2);
        let removed: Vec<String> = manifest
            .prune_unnamed_oldest(keep)
            .into_iter()
            .map(|v| v.sha256)
            .collect();
        delete_unreferenced(&blobs, &ops, &manifest, &removed);
        if let Err(message) = put_manifest(&manifests, &ops, &manifest) {
            ops.emit(HistoryEvent::Error(message));
        }
    });
}

/// Load one snapshot's bytes (manifest lookup → blob get) and emit
/// [`HistoryEvent::BlobLoaded`] with the echoed `purpose`.
fn run_load(db: &IdbDatabase, ops: &Ops, id: u64, purpose: BlobPurpose) {
    let Some(tx) = transaction(db, ops, IdbTransactionMode::Readonly) else {
        return;
    };
    let (Some(manifests), Some(blobs)) = (
        object_store(&tx, MANIFESTS, ops),
        object_store(&tx, BLOBS, ops),
    ) else {
        return;
    };
    let manifest_get = match manifests.get(&ops.manifest_key()) {
        Ok(get) => get,
        Err(e) => return ops.fail("could not read the manifest", e),
    };
    let ops = ops.clone();
    on_success(&manifest_get, move |result| {
        let manifest = match parse_manifest(result) {
            Ok(manifest) => manifest,
            Err(message) => return ops.emit(HistoryEvent::Error(message)),
        };
        let Some(entry) = manifest.find(id) else {
            return ops.emit(HistoryEvent::Error(format!("no version with id {id}")));
        };
        let blob_get = match blobs.get(&ops.blob_key(&entry.sha256)) {
            Ok(get) => get,
            Err(e) => return ops.fail("could not read the snapshot", e),
        };
        let sha = entry.sha256.clone();
        on_success(&blob_get, move |result| match result {
            Ok(value) if !value.is_undefined() && !value.is_null() => {
                ops.emit(HistoryEvent::BlobLoaded {
                    id,
                    purpose,
                    bytes: js_sys::Uint8Array::new(&value).to_vec(),
                });
            }
            Ok(_) => ops.emit(HistoryEvent::Error(format!(
                "version {id} has no snapshot blob (sha256 {sha})"
            ))),
            Err(e) => ops.fail("could not read the snapshot", e),
        });
    });
}

/// Read-modify-write the manifest; `mutate` returns the sha256 of every
/// removed version so unreferenced blobs can be GC'd in the same
/// transaction. On success the fresh list is emitted.
fn run_update(
    db: IdbDatabase,
    ops: Ops,
    mutate: impl FnOnce(&mut Manifest) -> Result<Vec<String>, String> + 'static,
) {
    let Some(tx) = transaction(&db, &ops, IdbTransactionMode::Readwrite) else {
        return;
    };
    let (Some(manifests), Some(blobs)) = (
        object_store(&tx, MANIFESTS, &ops),
        object_store(&tx, BLOBS, &ops),
    ) else {
        return;
    };
    let manifest_get = match manifests.get(&ops.manifest_key()) {
        Ok(get) => get,
        Err(e) => return ops.fail("could not read the manifest", e),
    };
    {
        let ops = ops.clone();
        on_abort(&tx, move || {
            ops.emit(HistoryEvent::Error(
                "could not update the history".to_owned(),
            ));
        });
    }
    {
        let db = db.clone();
        let ops = ops.clone();
        on_complete(&tx, move || run_list(db, ops));
    }
    on_success(&manifest_get, move |result| {
        let mut manifest = match parse_manifest(result) {
            Ok(manifest) => manifest,
            Err(message) => return ops.emit(HistoryEvent::Error(message)),
        };
        match mutate(&mut manifest) {
            Ok(removed) => {
                delete_unreferenced(&blobs, &ops, &manifest, &removed);
                if let Err(message) = put_manifest(&manifests, &ops, &manifest) {
                    ops.emit(HistoryEvent::Error(message));
                }
            }
            // The empty transaction still completes → the list refresh
            // runs, keeping the UI consistent after the error.
            Err(message) => ops.emit(HistoryEvent::Error(message)),
        }
    });
}
