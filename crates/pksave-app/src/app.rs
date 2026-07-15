//! Application state: the loaded document, screen routing, menu/status
//! bars, modals and the unsaved-changes guards.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use pksave::gen1::detect::detect_variant;
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::{Diagnostic, Severity};

use crate::error::AppError;
use crate::history::{
    self, BlobPurpose, HistoryEvent, HistorySettings, HistoryStore, Origin, VersionRow,
};
use crate::io::{self, IoEvent, SaveHistory, SaveRequest};
use crate::screens::{self, history::HistoryAction};
#[cfg(not(target_arch = "wasm32"))]
use crate::sdcard;

/// Mirror of the current dirty state for code outside the frame loop
/// (the wasm `beforeunload` listener).
static DIRTY_PUBLISHED: AtomicBool = AtomicBool::new(false);

pub fn publish_dirty(dirty: bool) {
    DIRTY_PUBLISHED.store(dirty, Ordering::Relaxed);
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn is_dirty_published() -> bool {
    DIRTY_PUBLISHED.load(Ordering::Relaxed)
}

const SHORTCUT_OPEN: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O);
const SHORTCUT_SAVE: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
const SHORTCUT_NEW: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::N);

/// How long a save-confirmation toast stays in the status bar.
const TOAST_SECONDS: f64 = 8.0;

/// Preset when the user turns on the max-versions limit.
const DEFAULT_MAX_VERSIONS: usize = 50;

/// The navigable screens, in sidebar order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Overview,
    Trainer,
    Party,
    Boxes,
    Items,
    Pokedex,
    Flags,
    Map,
    HallOfFame,
    Hex,
    History,
}

impl Screen {
    pub const ALL: [Screen; 11] = [
        Screen::Overview,
        Screen::Trainer,
        Screen::Party,
        Screen::Boxes,
        Screen::Items,
        Screen::Pokedex,
        Screen::Flags,
        Screen::Map,
        Screen::HallOfFame,
        Screen::Hex,
        Screen::History,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Screen::Overview => "Overview",
            Screen::Trainer => "Trainer",
            Screen::Party => "Party",
            Screen::Boxes => "Boxes",
            Screen::Items => "Items",
            Screen::Pokedex => "Pokédex",
            Screen::Flags => "Flags",
            Screen::Map => "Map",
            Screen::HallOfFame => "Hall of Fame",
            Screen::Hex => "Hex",
            Screen::History => "History",
        }
    }
}

/// Which screen a diagnostic most concerns (drives the sidebar badges).
pub fn screen_for_diagnostic(diag: &Diagnostic) -> Screen {
    use pksave::gen1::offsets;
    let span_start = diag.span.as_ref().map(|s| s.start);
    match diag.code {
        c if c.starts_with("W-CHECKSUM") || c == "I-CHECKSUM-PINNED" => Screen::Hex,
        c if c.starts_with("W-ITEMS") => Screen::Items,
        "W-PARTY-COUNT" | "W-PARTY-SENTINEL" | "W-LEVEL-RANGE" => Screen::Party,
        "W-SPECIES-INVALID" => {
            // Party slot, daycare mon or box slot: route by span.
            match span_start {
                Some(at) if (offsets::PARTY..offsets::PARTY + offsets::PARTY_LEN).contains(&at) => {
                    Screen::Party
                }
                _ => Screen::Overview,
            }
        }
        c if c.starts_with("W-BOX") => Screen::Boxes,
        "W-BCD-MONEY" | "W-BCD-COINS" => Screen::Trainer,
        "W-TEXT-UNTERMINATED" => match span_start {
            Some(at) if at >= offsets::PARTY => Screen::Party,
            _ => Screen::Trainer,
        },
        "W-DEX-RANGE" => Screen::Pokedex,
        "W-MAP-UNKNOWN" => Screen::Map,
        _ => Screen::Overview,
    }
}

/// A loaded save plus everything derived from it.
pub struct Doc {
    /// The bytes as loaded (or as last saved) — the dirty baseline.
    pub original: Vec<u8>,
    pub save: SaveFile,
    pub file_name: String,
    /// Native: where the file was opened from (backup target on save).
    pub path: Option<std::path::PathBuf>,
    pub variant: GameVariant,
    pub diagnostics: Vec<Diagnostic>,
    /// `changed_ranges(original, serialized)`, cached.
    pub changed: Vec<Range<usize>>,
    pub dirty: bool,
    /// `save.to_bytes()`, cached — recomputed by [`Doc::end_frame`] after
    /// a [`Doc::touch`], so per-frame consumers (the hex view) don't
    /// serialize the whole buffer every frame.
    serialized: Vec<u8>,
    touched: bool,
}

impl Doc {
    pub fn from_bytes(
        bytes: Vec<u8>,
        file_name: String,
        path: Option<std::path::PathBuf>,
    ) -> Result<Doc, pksave::gen1::save::LoadError> {
        let save = SaveFile::from_bytes(bytes.clone())?;
        let variant = detect_variant(&save);
        let diagnostics = save.diagnostics();
        Ok(Doc {
            original: bytes.clone(),
            save,
            file_name,
            path,
            variant,
            diagnostics,
            changed: Vec::new(),
            dirty: false,
            serialized: bytes,
            touched: false,
        })
    }

    pub fn new_empty(variant: GameVariant) -> Doc {
        let save = SaveFile::new_empty(variant);
        let bytes = save.to_bytes();
        Doc {
            original: bytes.clone(),
            diagnostics: save.diagnostics(),
            save,
            file_name: "new.sav".to_owned(),
            path: None,
            variant,
            changed: Vec::new(),
            dirty: false,
            serialized: bytes,
            touched: false,
        }
    }

    /// Record that an edit happened this frame; the expensive dirty/
    /// diagnostics recomputation is deferred to [`Doc::end_frame`] so it
    /// runs at most once per frame.
    pub fn touch(&mut self) {
        self.touched = true;
    }

    /// Recompute derived state if (and only if) something was touched
    /// this frame. Returns whether a recomputation happened.
    pub fn end_frame(&mut self) -> bool {
        if !self.touched {
            return false;
        }
        self.touched = false;
        self.refresh();
        true
    }

    /// The bytes exactly as they would be saved (checksums included),
    /// cached — see [`Doc::end_frame`].
    pub fn serialized(&self) -> &[u8] {
        &self.serialized
    }

    fn refresh(&mut self) {
        self.serialized = self.save.to_bytes();
        self.changed = changed_ranges(&self.original, &self.serialized);
        self.dirty = !self.changed.is_empty();
        self.diagnostics = self.save.diagnostics();
    }

    /// The edited bytes become the new baseline (after a successful save).
    pub fn mark_saved(&mut self) {
        self.original = self.save.to_bytes();
        self.refresh();
    }

    /// Throw away all edits and reload from the baseline bytes.
    pub fn revert(&mut self) {
        if let Ok(save) = SaveFile::from_bytes(self.original.clone()) {
            self.save = save;
        }
        self.refresh();
    }

    pub fn warning_count(&self) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity >= Severity::Warning)
            .count()
    }

    pub fn badge_count(&self, screen: Screen) -> usize {
        self.diagnostics
            .iter()
            .filter(|d| d.severity >= Severity::Warning && screen_for_diagnostic(d) == screen)
            .count()
    }
}

/// A destructive action awaiting confirmation while the document is dirty.
#[derive(Clone, PartialEq, Eq)]
enum PendingAction {
    Open,
    New(GameVariant),
    Revert,
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Close,
    /// A file dropped onto the window, stashed until the unsaved-changes
    /// guard resolves.
    LoadDropped {
        bytes: Vec<u8>,
        file_name: String,
        path: Option<std::path::PathBuf>,
    },
    /// A save clicked in the SD-card panel: read from disk at perform
    /// time (the card may have been pulled since discovery), then warn
    /// about a shadowing save state.
    #[cfg(not(target_arch = "wasm32"))]
    OpenDiscovered {
        path: std::path::PathBuf,
        shadowing_state: Option<std::path::PathBuf>,
    },
    /// Load a history version into the editor as the current buffer
    /// (guarded: it replaces any unsaved edits). Nothing is written to
    /// disk until the user saves.
    RestoreVersion(u64),
}

/// A transient status-bar confirmation (e.g. after a save).
struct Toast {
    message: String,
    /// `egui` time (seconds) after which the toast disappears.
    expires_at: f64,
}

/// Version-history state: the events channel, the per-document store,
/// the cached rows, and the toast-side "name this version" input.
struct HistoryUi {
    tx: Sender<HistoryEvent>,
    rx: Receiver<HistoryEvent>,
    /// The store for the current document; `None` while there is no
    /// document (or, natively, no on-disk path yet).
    store: Option<Box<dyn HistoryStore>>,
    settings: HistorySettings,
    /// Cached rows for the History screen (refreshed by `Versions`).
    versions: Vec<VersionRow>,
    /// Version awaiting the optional name input in the toast area.
    naming: Option<u64>,
    name_text: String,
    /// Count of legacy `.bak-*` siblings offered for import.
    legacy_offer: Option<usize>,
    /// The user declined the import offer for this document.
    legacy_dismissed: bool,
    /// Version loaded into the editor by Restore; the next save records
    /// origin "restore" with this as `parent_id`.
    restore_parent: Option<u64>,
    /// Version awaiting delete confirmation.
    pending_delete: Option<u64>,
}

impl HistoryUi {
    fn new() -> HistoryUi {
        let (tx, rx) = std::sync::mpsc::channel();
        HistoryUi {
            tx,
            rx,
            store: None,
            settings: HistorySettings::default(),
            versions: Vec::new(),
            naming: None,
            name_text: String::new(),
            legacy_offer: None,
            legacy_dismissed: false,
            restore_parent: None,
            pending_delete: None,
        }
    }

    /// Forget everything tied to the previous document (the settings
    /// survive — they are app-wide).
    fn reset_for_new_doc(&mut self) {
        self.store = None;
        self.versions.clear();
        self.naming = None;
        self.name_text.clear();
        self.legacy_offer = None;
        self.legacy_dismissed = false;
        self.restore_parent = None;
        self.pending_delete = None;
    }
}

/// SD-card discovery state (native only): the poller channel, the
/// currently mounted cards and the shadow-state warning for the
/// last-opened card save.
#[cfg(not(target_arch = "wasm32"))]
struct SdState {
    /// Sender handed to the poller thread on the first frame (the
    /// constructor has no `egui::Context` to give it earlier).
    tx: Option<Sender<sdcard::SdEvent>>,
    rx: Receiver<sdcard::SdEvent>,
    cards: Vec<sdcard::OnionCard>,
    panel_open: bool,
    /// A save state that will override the just-opened card save on next
    /// launch; drives the prominent warning modal.
    shadow_warning: Option<std::path::PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
impl SdState {
    fn new() -> SdState {
        let (tx, rx) = std::sync::mpsc::channel();
        SdState {
            tx: Some(tx),
            rx,
            cards: Vec::new(),
            panel_open: false,
            shadow_warning: None,
        }
    }
}

pub struct App {
    pub doc: Option<Doc>,
    pub screen: Screen,
    pub ui: screens::ScreenState,
    io_tx: Sender<IoEvent>,
    io_rx: Receiver<IoEvent>,
    /// One dialog at a time; the flag stops double-spawning.
    dialog_open: bool,
    pending: Option<PendingAction>,
    error: Option<AppError>,
    toast: Option<Toast>,
    close_confirmed: bool,
    history: HistoryUi,
    #[cfg(not(target_arch = "wasm32"))]
    sd: SdState,
}

impl Default for App {
    fn default() -> Self {
        App::new()
    }
}

impl App {
    /// Plain constructor (no `eframe::CreationContext` needed), so tests
    /// can build an `App` without a windowing backend.
    pub fn new() -> App {
        let (io_tx, io_rx) = std::sync::mpsc::channel();
        App {
            doc: None,
            screen: Screen::Overview,
            ui: screens::ScreenState::default(),
            io_tx,
            io_rx,
            dialog_open: false,
            pending: None,
            error: None,
            toast: None,
            close_confirmed: false,
            history: HistoryUi::new(),
            #[cfg(not(target_arch = "wasm32"))]
            sd: SdState::new(),
        }
    }

    /// (Re)bind the history store to the current document: natively to
    /// its on-disk path (none until the first save of a new file), on
    /// wasm to its IndexedDB identity. Sends a fresh version list.
    fn attach_history_store(&mut self, ctx: &egui::Context) {
        #[cfg(not(target_arch = "wasm32"))]
        let _ = ctx;
        self.history.store = match &self.doc {
            #[cfg(not(target_arch = "wasm32"))]
            Some(doc) => doc.path.clone().map(|path| {
                Box::new(history::fs::FsStore::new(path, self.history.tx.clone()))
                    as Box<dyn HistoryStore>
            }),
            #[cfg(target_arch = "wasm32")]
            Some(doc) => {
                let identity = format!("{}:{}", history::sha256_hex(&doc.original), doc.file_name);
                Some(Box::new(history::idb::IdbStore::new(
                    identity,
                    self.history.tx.clone(),
                    ctx.clone(),
                )) as Box<dyn HistoryStore>)
            }
            None => None,
        };
        self.history.versions.clear();
        if let Some(store) = &mut self.history.store {
            store.list();
        }
    }

    /// The history parameters the *next* primary save should record
    /// with: origin "restore" + the restored version when the buffer
    /// came from Restore, a plain "save" otherwise. `None` while
    /// history is off.
    fn history_params(&self) -> Option<SaveHistory> {
        if !self.history.settings.enabled {
            return None;
        }
        Some(SaveHistory {
            origin: match self.history.restore_parent {
                Some(_) => Origin::Restore,
                None => Origin::Save,
            },
            parent_id: self.history.restore_parent,
            max_versions: self.history.settings.max_versions,
        })
    }

    /// Load bytes as the new document. Returns whether it succeeded.
    fn load_bytes(
        &mut self,
        ctx: &egui::Context,
        bytes: Vec<u8>,
        file_name: String,
        path: Option<std::path::PathBuf>,
    ) -> bool {
        match Doc::from_bytes(bytes, file_name, path) {
            Ok(doc) => {
                publish_dirty(false);
                self.doc = Some(doc);
                self.ui = screens::ScreenState::default();
                self.screen = Screen::Overview;
                self.history.reset_for_new_doc();
                self.attach_history_store(ctx);
                true
            }
            Err(e) => {
                self.error = Some(AppError::Load(e));
                false
            }
        }
    }

    fn show_toast(&mut self, now: f64, message: String) {
        self.toast = Some(Toast {
            message,
            expires_at: now + TOAST_SECONDS,
        });
    }

    fn poll_io(&mut self, ctx: &egui::Context, now: f64) {
        while let Ok(event) = self.io_rx.try_recv() {
            self.dialog_open = false;
            match event {
                IoEvent::Opened {
                    file_name,
                    bytes,
                    path,
                } => {
                    self.load_bytes(ctx, bytes, file_name, path);
                }
                IoEvent::Saved {
                    primary,
                    path,
                    backup,
                    file_name,
                    version,
                    legacy_backups,
                    history_error,
                } => {
                    let is_native = path.is_some();
                    #[cfg(not(target_arch = "wasm32"))]
                    let saved_to = path.clone();
                    if let Some(doc) = &mut self.doc {
                        if primary {
                            doc.mark_saved();
                            if let Some(path) = path {
                                if let Some(name) = path.file_name() {
                                    doc.file_name = name.to_string_lossy().into_owned();
                                }
                                doc.path = Some(path);
                            }
                            publish_dirty(doc.dirty);
                        }
                    }
                    let mut message = if !is_native {
                        format!("Download started: {file_name}")
                    } else if primary {
                        format!("Saved to {file_name}")
                    } else {
                        format!("Copy of original saved to {file_name}")
                    };
                    if let Some(backup) = backup {
                        if let Some(name) = backup.file_name() {
                            message.push_str(&format!(" — backup: {}", name.to_string_lossy()));
                        }
                    }
                    // Writing to a discovered card is fsynced by the save
                    // flow, so once reported it is safe to pull the card.
                    #[cfg(not(target_arch = "wasm32"))]
                    if saved_to
                        .as_deref()
                        .is_some_and(|p| sdcard::containing_card(&self.sd.cards, p).is_some())
                    {
                        message.push_str(" — safe to eject the SD card");
                    }
                    if primary {
                        // Judged on the pre-save rows: rebinding the
                        // store below clears the cache until the fresh
                        // list arrives.
                        let already_imported = self.has_imported_versions();
                        // Native: save-as may have moved the file —
                        // rebind the store to the (new) path; this also
                        // refreshes the version list. On wasm the store
                        // stays: its identity is pinned to the *load*
                        // bytes and must not be recomputed from the
                        // just-rebaselined buffer.
                        #[cfg(not(target_arch = "wasm32"))]
                        self.attach_history_store(ctx);
                        if let Some(entry) = &version {
                            // Native: the version was recorded inside
                            // the save flow (blob → manifest → file).
                            message.push_str(&format!(" · version {}", entry.id));
                            self.history.naming = Some(entry.id);
                            self.history.name_text.clear();
                            self.history.restore_parent = None;
                        }
                        if let Some(e) = &history_error {
                            message.push_str(&format!(" — history not recorded: {e}"));
                        }
                        if legacy_backups > 0 && !self.history.legacy_dismissed && !already_imported
                        {
                            self.history.legacy_offer = Some(legacy_backups);
                        }
                        // wasm: no io-side history — record through the
                        // IndexedDB store now that the download started.
                        #[cfg(target_arch = "wasm32")]
                        if !is_native {
                            if let (Some(params), Some(doc)) = (self.history_params(), &self.doc) {
                                let bytes = doc.original.clone();
                                if let Some(store) = &mut self.history.store {
                                    store.record(
                                        bytes,
                                        params.origin,
                                        params.parent_id,
                                        params.max_versions,
                                    );
                                    self.history.restore_parent = None;
                                }
                            }
                        }
                    }
                    self.show_toast(now, message);
                }
                IoEvent::Cancelled => {}
                IoEvent::Error(e) => self.error = Some(e),
            }
        }
    }

    /// Whether the cached version list already contains imported legacy
    /// backups (suppresses the "import .bak" offer).
    fn has_imported_versions(&self) -> bool {
        self.history
            .versions
            .iter()
            .any(|r| r.entry.origin == Origin::Import)
    }

    /// Drain history-store events (mirrors [`App::poll_io`]).
    fn poll_history(&mut self, ctx: &egui::Context, now: f64) {
        while let Ok(event) = self.history.rx.try_recv() {
            match event {
                HistoryEvent::Versions(rows) => {
                    self.history.versions = rows;
                    if self.has_imported_versions() {
                        self.history.legacy_offer = None;
                    }
                }
                HistoryEvent::Recorded(entry) => {
                    // wasm path: the record completed after the Saved
                    // toast was shown — extend it.
                    match &mut self.toast {
                        Some(toast) => {
                            toast.message.push_str(&format!(" · version {}", entry.id));
                            toast.expires_at = now + TOAST_SECONDS;
                        }
                        None => self.show_toast(now, format!("Saved · version {}", entry.id)),
                    }
                    self.history.naming = Some(entry.id);
                    self.history.name_text.clear();
                }
                HistoryEvent::BlobLoaded { id, purpose, bytes } => match purpose {
                    BlobPurpose::Restore => self.finish_restore(now, id, bytes),
                    BlobPurpose::Diff => self.finish_diff(id, bytes),
                    BlobPurpose::Export => self.start_export(ctx, id, bytes),
                },
                HistoryEvent::LegacyImported { count } => {
                    self.history.legacy_offer = None;
                    self.history.legacy_dismissed = true;
                    let plural = if count == 1 { "" } else { "s" };
                    self.show_toast(
                        now,
                        format!("Imported {count} legacy backup{plural} into the version history"),
                    );
                }
                HistoryEvent::Error(message) => {
                    self.show_toast(now, format!("History: {message}"));
                }
            }
        }
    }

    /// A Restore blob arrived: replace the editor buffer. The dirty
    /// flag comes on through the usual diff against the on-disk
    /// baseline; nothing is written until the user saves.
    fn finish_restore(&mut self, now: f64, id: u64, bytes: Vec<u8>) {
        let Some(doc) = &mut self.doc else { return };
        match SaveFile::from_bytes(bytes) {
            Ok(save) => {
                doc.save = save;
                doc.touch();
                doc.end_frame();
                publish_dirty(doc.dirty);
                self.history.restore_parent = Some(id);
                self.show_toast(
                    now,
                    format!("Version {id} loaded into the editor — save to write it to disk"),
                );
            }
            Err(e) => self.error = Some(AppError::Load(e)),
        }
    }

    /// A Diff blob arrived: show its changed ranges (version → current
    /// buffer) with human-readable field labels on the History screen.
    fn finish_diff(&mut self, id: u64, bytes: Vec<u8>) {
        let Some(doc) = &self.doc else { return };
        let ranges = changed_ranges(&bytes, doc.serialized());
        self.ui.history.diff = Some(screens::history::DiffView {
            id,
            byte_count: ranges.iter().map(|r| r.len()).sum(),
            lines: history::spans::describe(&ranges),
        });
        self.screen = Screen::History;
    }

    /// An Export blob arrived: hand it to the regular save-as flow
    /// (native dialog / wasm download) as a non-primary save.
    fn start_export(&mut self, ctx: &egui::Context, id: u64, bytes: Vec<u8>) {
        if self.dialog_open {
            return;
        }
        let file_name = self
            .doc
            .as_ref()
            .map(|d| d.file_name.as_str())
            .unwrap_or("save.srm");
        let request = SaveRequest {
            default_file_name: history::export_file_name(file_name, id),
            bytes,
            primary: false,
            original_path: None,
            history: None,
        };
        self.dialog_open = true;
        io::spawn_save(self.io_tx.clone(), ctx.clone(), request);
    }

    /// Start the SD-card poller thread once (the first frame is the
    /// earliest moment an `egui::Context` exists) and drain its events.
    #[cfg(not(target_arch = "wasm32"))]
    fn poll_sd(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(tx) = self.sd.tx.take() {
            sdcard::spawn_poller(tx, ctx.clone());
        }
        while let Ok(event) = self.sd.rx.try_recv() {
            match event {
                sdcard::SdEvent::CardDetected(card) => {
                    let count = card.saves.len();
                    let plural = if count == 1 { "" } else { "s" };
                    self.show_toast(
                        now,
                        format!(
                            "Miyoo SD card detected ({}) — {count} Pokémon save{plural}",
                            card.volume_name
                        ),
                    );
                    self.sd.cards.retain(|c| c.root != card.root);
                    if count > 0 {
                        self.sd.panel_open = true;
                    }
                    self.sd.cards.push(card);
                }
                sdcard::SdEvent::CardRemoved(root) => {
                    self.sd.cards.retain(|c| c.root != root);
                    if self.sd.cards.is_empty() {
                        self.sd.panel_open = false;
                    }
                    self.show_toast(
                        now,
                        format!(
                            "SD card removed ({})",
                            root.file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| root.display().to_string())
                        ),
                    );
                }
            }
        }
    }

    /// Route a dropped file through the same unsaved-changes guard as
    /// File → Open: with unsaved edits, the payload is stashed and a
    /// confirmation modal decides its fate.
    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let Some(file) = dropped.into_iter().next() else {
            return;
        };
        if let Some(bytes) = file.bytes {
            let name = if file.name.is_empty() {
                "dropped.sav".to_owned()
            } else {
                file.name.clone()
            };
            self.request(
                ctx,
                PendingAction::LoadDropped {
                    bytes: bytes.to_vec(),
                    file_name: name,
                    path: None,
                },
            );
        } else if let Some(path) = file.path {
            match std::fs::read(&path) {
                Ok(bytes) => {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "dropped.sav".to_owned());
                    self.request(
                        ctx,
                        PendingAction::LoadDropped {
                            bytes,
                            file_name: name,
                            path: Some(path),
                        },
                    );
                }
                Err(source) => self.error = Some(AppError::Read { path, source }),
            }
        }
    }

    fn is_dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty)
    }

    /// Run `action` now, or queue a confirmation modal if it would drop
    /// unsaved changes.
    fn request(&mut self, ctx: &egui::Context, action: PendingAction) {
        if self.is_dirty() {
            self.pending = Some(action);
        } else {
            self.perform(ctx, action);
        }
    }

    /// The confirm-discard modal was decided: perform the stashed action
    /// (`discard == true`) or drop it.
    fn resolve_pending(&mut self, ctx: &egui::Context, discard: bool) {
        if let Some(action) = self.pending.take() {
            if discard {
                self.perform(ctx, action);
            }
        }
    }

    fn perform(&mut self, ctx: &egui::Context, action: PendingAction) {
        match action {
            PendingAction::Open => {
                if !self.dialog_open {
                    self.dialog_open = true;
                    io::spawn_open(self.io_tx.clone(), ctx.clone());
                }
            }
            PendingAction::New(variant) => {
                publish_dirty(false);
                self.doc = Some(Doc::new_empty(variant));
                self.ui = screens::ScreenState::default();
                self.screen = Screen::Overview;
                self.history.reset_for_new_doc();
                self.attach_history_store(ctx);
            }
            PendingAction::Revert => {
                if let Some(doc) = &mut self.doc {
                    doc.revert();
                    publish_dirty(doc.dirty);
                }
            }
            PendingAction::Close => {
                self.close_confirmed = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
            PendingAction::LoadDropped {
                bytes,
                file_name,
                path,
            } => {
                self.load_bytes(ctx, bytes, file_name, path);
            }
            #[cfg(not(target_arch = "wasm32"))]
            PendingAction::OpenDiscovered {
                path,
                shadowing_state,
            } => match std::fs::read(&path) {
                Ok(bytes) => {
                    let file_name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "card.srm".to_owned());
                    if self.load_bytes(ctx, bytes, file_name, Some(path)) {
                        self.sd.shadow_warning = shadowing_state;
                    }
                }
                Err(source) => self.error = Some(AppError::Read { path, source }),
            },
            PendingAction::RestoreVersion(id) => {
                if let Some(store) = &mut self.history.store {
                    store.load_blob(id, BlobPurpose::Restore);
                }
            }
        }
    }

    fn start_save(&mut self, ctx: &egui::Context, primary: bool) {
        let Some(doc) = &self.doc else { return };
        if self.dialog_open {
            return;
        }
        let request = SaveRequest {
            default_file_name: doc.file_name.clone(),
            bytes: if primary {
                doc.save.to_bytes()
            } else {
                doc.original.clone()
            },
            primary,
            original_path: if primary { doc.path.clone() } else { None },
            history: if primary { self.history_params() } else { None },
        };
        self.dialog_open = true;
        io::spawn_save(self.io_tx.clone(), ctx.clone(), request);
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_OPEN)) {
            self.request(ctx, PendingAction::Open);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_SAVE)) && self.doc.is_some() {
            self.start_save(ctx, true);
        }
        if ctx.input_mut(|i| i.consume_shortcut(&SHORTCUT_NEW)) {
            self.request(ctx, PendingAction::New(GameVariant::RedBlue));
        }
    }

    fn menu_bar(&mut self, ui: &mut egui::Ui) {
        let ctx = &ui.ctx().clone();
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    ui.menu_button("New", |ui| {
                        if ui
                            .add(
                                egui::Button::new("Red / Blue")
                                    .shortcut_text(ctx.format_shortcut(&SHORTCUT_NEW)),
                            )
                            .clicked()
                        {
                            self.request(ctx, PendingAction::New(GameVariant::RedBlue));
                        }
                        if ui.button("Yellow").clicked() {
                            self.request(ctx, PendingAction::New(GameVariant::Yellow));
                        }
                    });
                    if ui
                        .add(
                            egui::Button::new("Open…")
                                .shortcut_text(ctx.format_shortcut(&SHORTCUT_OPEN)),
                        )
                        .clicked()
                    {
                        self.request(ctx, PendingAction::Open);
                    }
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui
                        .add_enabled(
                            !self.sd.cards.is_empty(),
                            egui::Button::new("SD card saves…"),
                        )
                        .on_disabled_hover_text("No OnionOS/Miyoo SD card detected")
                        .clicked()
                    {
                        self.sd.panel_open = true;
                    }
                    ui.separator();
                    let has_doc = self.doc.is_some();
                    if ui
                        .add_enabled(
                            has_doc,
                            egui::Button::new("Save…")
                                .shortcut_text(ctx.format_shortcut(&SHORTCUT_SAVE)),
                        )
                        .clicked()
                    {
                        self.start_save(ctx, true);
                    }
                    let copy_label = if cfg!(target_arch = "wasm32") {
                        "Download original"
                    } else {
                        "Save copy of original…"
                    };
                    if ui
                        .add_enabled(has_doc, egui::Button::new(copy_label))
                        .clicked()
                    {
                        self.start_save(ctx, false);
                    }
                    let dirty = self.is_dirty();
                    if ui.add_enabled(dirty, egui::Button::new("Revert")).clicked() {
                        self.request(ctx, PendingAction::Revert);
                    }
                    ui.separator();
                    ui.menu_button("History", |ui| self.history_settings_menu(ui));
                    #[cfg(not(target_arch = "wasm32"))]
                    {
                        ui.separator();
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    }
                });
                if let Some(doc) = &mut self.doc {
                    ui.menu_button("Repair", |ui| {
                        if ui
                            .button("Fix all checksums now")
                            .on_hover_text(
                                "Recompute and store all 15 checksums (also unpinning any \
                                 hand-edited checksum bytes), repairing a file that was \
                                 already corrupt on load",
                            )
                            .clicked()
                        {
                            doc.save.fix_checksums();
                            doc.touch();
                        }
                    });
                }
            });
        });
    }

    /// The File → History settings submenu: history on/off and the
    /// max-versions limit. (A history-location override is deliberately
    /// deferred — the history always lives beside the file for v1.)
    fn history_settings_menu(&mut self, ui: &mut egui::Ui) {
        ui.checkbox(&mut self.history.settings.enabled, "Record version history")
            .on_hover_text(
                "Every Save records a restorable snapshot beside the file \
             (in the browser: IndexedDB). Off: only the plain .bak backup is written.",
            );
        let mut limited = self.history.settings.max_versions.is_some();
        let mut changed_max: Option<usize> = None;
        if ui
            .checkbox(&mut limited, "Limit stored versions")
            .on_hover_text(
                "Past the limit, the oldest unnamed versions are pruned. \
                 Named versions are never auto-pruned; if only named versions \
                 remain, nothing more is pruned.",
            )
            .changed()
        {
            self.history.settings.max_versions = limited.then_some(DEFAULT_MAX_VERSIONS);
            if limited {
                changed_max = Some(DEFAULT_MAX_VERSIONS);
            }
        }
        if let Some(max) = &mut self.history.settings.max_versions {
            let response = ui.add(
                egui::DragValue::new(max)
                    .range(1..=100_000)
                    .prefix("keep ")
                    .suffix(" versions"),
            );
            if response.changed() {
                changed_max = Some(*max);
            }
        }
        // Apply a (new) limit to the existing history right away.
        if let (Some(max), Some(store)) = (changed_max, &mut self.history.store) {
            store.prune(max);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        if self.toast.as_ref().is_some_and(|t| t.expires_at <= now) {
            self.toast = None;
            // The optional naming field lives in the toast area and
            // goes with it (versions can still be renamed in History).
            self.history.naming = None;
        }
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                match &mut self.doc {
                    Some(doc) => {
                        if doc.dirty {
                            ui.colored_label(ui.visuals().warn_fg_color, "●")
                                .on_hover_text("Unsaved changes");
                        }
                        ui.label(&doc.file_name);
                        ui.separator();
                        ui.label("Variant:");
                        let variant_hover = "Label only — the save layout is identical";
                        ui.selectable_value(&mut doc.variant, GameVariant::RedBlue, "Red/Blue")
                            .on_hover_text(variant_hover);
                        ui.selectable_value(&mut doc.variant, GameVariant::Yellow, "Yellow")
                            .on_hover_text(variant_hover);
                        ui.separator();
                        let warnings = doc.warning_count();
                        if warnings == 0 {
                            ui.label("no warnings");
                        } else if ui.link(format!("⚠ {warnings} warning(s)")).clicked() {
                            self.screen = Screen::Overview;
                        }
                        ui.separator();
                        ui.label(doc.save.game_label());
                    }
                    None => {
                        ui.label("No file loaded — File → Open, or drop a .sav here");
                    }
                }
                if let Some(toast) = &self.toast {
                    ui.separator();
                    let message = toast.message.clone();
                    ui.colored_label(ui.visuals().hyperlink_color, message);
                    self.naming_field(ui, now);
                    if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                        self.toast = None;
                        self.history.naming = None;
                    } else {
                        // Wake up in time to expire the toast.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs(1));
                    }
                }
                self.legacy_offer_button(ui);
            });
        });
    }

    /// The unobtrusive, optional "name this version" input next to the
    /// save toast. Never required, never blocks: OK/Enter with text
    /// names the just-saved version, anything else just goes away.
    fn naming_field(&mut self, ui: &mut egui::Ui, now: f64) {
        let Some(id) = self.history.naming else {
            return;
        };
        let response = ui.add(
            egui::TextEdit::singleline(&mut self.history.name_text)
                .desired_width(150.0)
                .hint_text("name this version (optional)"),
        );
        if response.has_focus() {
            // Don't expire the toast mid-typing.
            if let Some(toast) = &mut self.toast {
                toast.expires_at = now + TOAST_SECONDS;
            }
        }
        let submit = ui.small_button("OK").clicked()
            || (response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
        if submit {
            let name = self.history.name_text.trim().to_owned();
            if !name.is_empty() {
                if let Some(store) = &mut self.history.store {
                    store.set_label(id, Some(name));
                }
            }
            self.history.naming = None;
            self.history.name_text.clear();
        }
    }

    /// Non-blocking offer to import legacy `.bak-*` siblings into the
    /// history (shown after a save discovered them).
    fn legacy_offer_button(&mut self, ui: &mut egui::Ui) {
        let Some(count) = self.history.legacy_offer else {
            return;
        };
        ui.separator();
        let plural = if count == 1 { "" } else { "s" };
        if ui
            .button(format!("Import {count} old .bak backup{plural}"))
            .on_hover_text(
                "Add the legacy .bak-<timestamp> files beside this save to the \
                 version history (timestamps are taken from the file names).",
            )
            .clicked()
        {
            self.history.legacy_offer = None;
            if let Some(store) = &mut self.history.store {
                store.import_legacy();
            }
        }
        if ui.small_button("✕").on_hover_text("Not now").clicked() {
            self.history.legacy_offer = None;
            self.history.legacy_dismissed = true;
        }
    }

    fn side_panel(&mut self, ui: &mut egui::Ui) {
        let Some(doc) = &self.doc else { return };
        let badges: Vec<(Screen, usize)> = Screen::ALL
            .iter()
            .map(|&s| (s, doc.badge_count(s)))
            .collect();
        egui::Panel::left("nav")
            .resizable(false)
            .default_size(150.0)
            .show(ui, |ui| {
                ui.add_space(4.0);
                for (screen, badge) in badges {
                    let label = if badge > 0 {
                        format!("{}  ⚠{badge}", screen.label())
                    } else {
                        screen.label().to_owned()
                    };
                    if ui.selectable_label(self.screen == screen, label).clicked() {
                        self.screen = screen;
                    }
                }
            });
    }

    fn central(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let mut history_actions: Vec<HistoryAction> = Vec::new();
        egui::CentralPanel::default().show(ui, |ui| {
            let Some(doc) = &mut self.doc else {
                self.empty_state(ui, &ctx);
                return;
            };
            let mut goto: Option<(Screen, usize)> = None;
            match self.screen {
                Screen::Overview => screens::overview::ui(ui, doc, &mut goto),
                Screen::Trainer => screens::trainer::ui(ui, doc),
                Screen::Party => screens::party::ui(ui, doc, &mut self.ui.party),
                Screen::Boxes => screens::boxes::ui(ui, doc, &mut self.ui.boxes),
                Screen::Items => screens::items::ui(ui, doc, &mut self.ui.items),
                Screen::Pokedex => screens::pokedex::ui(ui, doc, &mut self.ui.pokedex),
                Screen::Flags => screens::flags::ui(ui, doc, &mut self.ui.flags),
                Screen::Map => screens::map::ui(ui, doc, &mut self.ui.map),
                Screen::HallOfFame => screens::hof::ui(ui, doc, &mut self.ui.hof),
                Screen::Hex => screens::hex::ui(ui, doc, &mut self.ui.hex),
                Screen::History => screens::history::ui(
                    ui,
                    &mut self.ui.history,
                    &self.history.versions,
                    self.history.settings.enabled,
                    &mut history_actions,
                ),
            }
            if let Some((screen, offset)) = goto {
                self.screen = screen;
                self.ui.hex.scroll_to(offset);
            }
        });
        for action in history_actions {
            self.handle_history_action(&ctx, action);
        }
    }

    /// Execute a History-screen row action.
    fn handle_history_action(&mut self, ctx: &egui::Context, action: HistoryAction) {
        match action {
            // Guarded like Open: restoring replaces any unsaved edits.
            HistoryAction::Restore(id) => self.request(ctx, PendingAction::RestoreVersion(id)),
            HistoryAction::Diff(id) => {
                if let Some(store) = &mut self.history.store {
                    store.load_blob(id, BlobPurpose::Diff);
                }
            }
            HistoryAction::SetLabel(id, label) => {
                if let Some(store) = &mut self.history.store {
                    store.set_label(id, label);
                }
            }
            HistoryAction::Export(id) => {
                if let Some(store) = &mut self.history.store {
                    store.load_blob(id, BlobPurpose::Export);
                }
            }
            HistoryAction::Delete(id) => self.history.pending_delete = Some(id),
        }
    }

    /// The no-document landing screen: prominent Open / New actions plus
    /// a drag-and-drop hint.
    fn empty_state(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.25);
            ui.heading("pksave — Gen 1 save editor");
            ui.add_space(8.0);
            ui.label("Open a Pokémon Red/Blue/Yellow save file (.sav / .srm) to start editing.");
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                // Center the button row.
                let width = 330.0;
                ui.add_space((ui.available_width() - width).max(0.0) / 2.0);
                if ui.button("📂 Open a save file…").clicked() {
                    self.request(ctx, PendingAction::Open);
                }
                if ui.button("✚ New Red/Blue").clicked() {
                    self.request(ctx, PendingAction::New(GameVariant::RedBlue));
                }
                if ui.button("✚ New Yellow").clicked() {
                    self.request(ctx, PendingAction::New(GameVariant::Yellow));
                }
            });
            ui.add_space(12.0);
            ui.weak("…or drag and drop a .sav file anywhere in this window.");
        });
    }

    /// The SD-card panel: every discovered card with its saves. Valid
    /// saves open through the standard (guarded) open path; unparsable
    /// ones are greyed with their diagnostic.
    #[cfg(not(target_arch = "wasm32"))]
    fn sd_panel(&mut self, ctx: &egui::Context) {
        if self.sd.cards.is_empty() || !self.sd.panel_open {
            return;
        }
        let mut open = true;
        let mut clicked: Option<PendingAction> = None;
        egui::Window::new("SD card saves")
            .open(&mut open)
            .default_width(440.0)
            .show(ctx, |ui| {
                for (i, card) in self.sd.cards.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.heading(format!("💾 {}", card.volume_name));
                    ui.weak(card.root.display().to_string());
                    ui.add_space(4.0);
                    if card.saves.is_empty() {
                        ui.label("No Gen 1 saves found on this card.");
                        continue;
                    }
                    for save in &card.saves {
                        let origin = if save.legacy {
                            format!("{} (legacy path)", save.profile)
                        } else {
                            save.profile.clone()
                        };
                        match &save.preview {
                            Some(preview) => {
                                ui.horizontal(|ui| {
                                    if ui
                                        .button(format!("📂 {}", save.rom_name))
                                        .on_hover_text(save.path.display().to_string())
                                        .clicked()
                                    {
                                        clicked = Some(PendingAction::OpenDiscovered {
                                            path: save.path.clone(),
                                            shadowing_state: save.shadowing_state.clone(),
                                        });
                                    }
                                    ui.weak(origin);
                                    if save.shadowing_state.is_some() {
                                        ui.colored_label(ui.visuals().warn_fg_color, "⚠ state")
                                            .on_hover_text(
                                                "A save state exists and will override this \
                                                 battery save on next launch",
                                            );
                                    }
                                });
                                ui.weak(format!(
                                    "    {} · {} badge(s) · {} · {}",
                                    preview.trainer_name,
                                    preview.badges,
                                    preview.play_time,
                                    preview.party_summary
                                ));
                            }
                            None => {
                                ui.add_enabled(
                                    false,
                                    egui::Button::new(format!("📂 {}", save.rom_name)),
                                );
                                ui.weak(format!(
                                    "    {} · {}",
                                    origin,
                                    save.diagnostic.as_deref().unwrap_or("could not parse")
                                ));
                            }
                        }
                        ui.add_space(2.0);
                    }
                }
            });
        self.sd.panel_open = open;
        if let Some(action) = clicked {
            self.request(ctx, action);
        }
    }

    /// Prominent warning when the just-opened card save has a save state
    /// that OnionOS will auto-load over it, discarding any edits.
    #[cfg(not(target_arch = "wasm32"))]
    fn shadow_warning_modal(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let Some(state_path) = self.sd.shadow_warning.clone() else {
            return;
        };
        egui::Modal::new(egui::Id::new("shadow_state_modal")).show(ctx, |ui| {
            ui.heading("⚠ A save state will override your edits");
            ui.add_space(4.0);
            ui.label(
                "OnionOS auto-loads save states (savestate_auto_load). This save state \
                 will override the battery save the next time the game is launched, \
                 discarding any edits you write back:",
            );
            ui.monospace(state_path.display().to_string());
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Rename state (recommended)").clicked() {
                    match sdcard::neutralize_state(&state_path) {
                        Ok(renamed) => {
                            let name = renamed
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_else(|| renamed.display().to_string());
                            self.show_toast(now, format!("Save state renamed to {name}"));
                        }
                        Err(source) => {
                            self.error = Some(AppError::RenameState {
                                path: state_path.clone(),
                                source,
                            });
                        }
                    }
                    self.sd.shadow_warning = None;
                }
                if ui.button("Ignore").clicked() {
                    self.sd.shadow_warning = None;
                }
            });
        });
    }

    fn modals(&mut self, ctx: &egui::Context) {
        if self.pending.is_some() {
            let mut decided: Option<bool> = None;
            egui::Modal::new(egui::Id::new("confirm_discard")).show(ctx, |ui| {
                ui.heading("Unsaved changes");
                ui.label("The current file has unsaved changes that will be lost. Continue?");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Discard changes").clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
            if let Some(discard) = decided {
                self.resolve_pending(ctx, discard);
            }
        }

        if let Some(id) = self.history.pending_delete {
            let mut decided: Option<bool> = None;
            egui::Modal::new(egui::Id::new("confirm_delete_version")).show(ctx, |ui| {
                ui.heading(format!("Delete version {id}?"));
                ui.label("The snapshot is removed from the history. This cannot be undone.");
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        decided = Some(true);
                    }
                    if ui.button("Cancel").clicked() {
                        decided = Some(false);
                    }
                });
            });
            if let Some(delete) = decided {
                self.history.pending_delete = None;
                if delete {
                    if let Some(store) = &mut self.history.store {
                        store.delete(id);
                    }
                }
            }
        }

        if let Some(message) = self.error.as_ref().map(|e| e.to_string()) {
            let mut close = false;
            egui::Modal::new(egui::Id::new("error_modal")).show(ctx, |ui| {
                ui.heading("Error");
                ui.label(&message);
                ui.add_space(8.0);
                if ui.button("OK").clicked() {
                    close = true;
                }
            });
            if close {
                self.error = None;
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn handle_close_request(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.is_dirty() && !self.close_confirmed
        {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(PendingAction::Close);
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = ctx.input(|i| i.time);
        self.poll_io(&ctx, now);
        self.poll_history(&ctx, now);
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_sd(&ctx, now);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.handle_close_request(&ctx);

        self.menu_bar(ui);
        self.status_bar(ui);
        self.side_panel(ui);
        self.central(ui);
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.sd_panel(&ctx);
            self.shadow_warning_modal(&ctx);
        }
        self.modals(&ctx);

        if let Some(doc) = &mut self.doc {
            if doc.end_frame() {
                publish_dirty(doc.dirty);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_doc() -> Doc {
        Doc::new_empty(GameVariant::RedBlue)
    }

    #[test]
    fn end_frame_is_a_no_op_without_a_touch() {
        let mut doc = empty_doc();
        assert!(!doc.end_frame());
        assert!(!doc.dirty);
    }

    #[test]
    fn touch_then_end_frame_recomputes_dirty_once() {
        let mut doc = empty_doc();
        doc.save.set_player_id(0x1234);
        doc.touch();
        assert!(!doc.dirty, "dirty is deferred until end_frame");
        assert!(doc.end_frame());
        assert!(doc.dirty);
        assert!(!doc.changed.is_empty());
        // The throttle resets: no touch, no recompute.
        assert!(!doc.end_frame());
    }

    #[test]
    fn serialized_cache_tracks_touches() {
        let mut doc = empty_doc();
        assert_eq!(doc.serialized(), doc.original.as_slice());
        doc.save.set_player_id(0xBEEF);
        // Stale until end_frame.
        assert_eq!(doc.serialized(), doc.original.as_slice());
        doc.touch();
        doc.end_frame();
        assert_eq!(doc.serialized(), doc.save.to_bytes().as_slice());
        assert_ne!(doc.serialized(), doc.original.as_slice());
    }

    #[test]
    fn mark_saved_clears_dirty() {
        let mut doc = empty_doc();
        doc.save.set_player_id(0x1234);
        doc.touch();
        doc.end_frame();
        assert!(doc.dirty);
        doc.mark_saved();
        assert!(!doc.dirty);
        assert!(doc.changed.is_empty());
    }

    #[test]
    fn revert_restores_the_baseline() {
        let mut doc = empty_doc();
        let before = doc.save.to_bytes();
        doc.save.set_badges(0xFF);
        doc.touch();
        doc.end_frame();
        assert!(doc.dirty);
        doc.revert();
        assert!(!doc.dirty);
        assert_eq!(doc.save.to_bytes(), before);
    }

    #[test]
    fn diagnostics_route_to_screens() {
        let zeroed = SaveFile::from_bytes(vec![0u8; 0x8000]).expect("valid length");
        let diags = zeroed.diagnostics();
        assert!(!diags.is_empty());
        for d in &diags {
            // Every diagnostic maps to some screen without panicking.
            let _ = screen_for_diagnostic(d);
        }
        let checksum = diags
            .iter()
            .find(|d| d.code.starts_with("W-CHECKSUM"))
            .expect("zeroed save has checksum warnings");
        assert_eq!(screen_for_diagnostic(checksum), Screen::Hex);
    }

    #[test]
    fn badge_counts_sum_to_warning_count() {
        let bytes = vec![0u8; 0x8000];
        let doc = Doc::from_bytes(bytes, "z.sav".into(), None).expect("valid length");
        let sum: usize = Screen::ALL.iter().map(|&s| doc.badge_count(s)).sum();
        assert_eq!(sum, doc.warning_count());
    }

    // ---- io-event flow (J1) ----

    fn valid_save_bytes() -> Vec<u8> {
        SaveFile::new_empty(GameVariant::RedBlue).to_bytes()
    }

    /// An `IoEvent::Saved` without any history payload.
    fn saved_event(
        primary: bool,
        path: Option<std::path::PathBuf>,
        backup: Option<std::path::PathBuf>,
        file_name: &str,
    ) -> IoEvent {
        IoEvent::Saved {
            primary,
            path,
            backup,
            file_name: file_name.to_owned(),
            version: None,
            legacy_backups: 0,
            history_error: None,
        }
    }

    #[test]
    fn opened_event_loads_a_doc_and_clears_dialog_flag() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.dialog_open = true;
        app.io_tx
            .send(IoEvent::Opened {
                file_name: "poke.sav".into(),
                bytes: valid_save_bytes(),
                path: Some(std::path::PathBuf::from("/tmp/poke.sav")),
            })
            .expect("send");
        app.poll_io(&ctx, 0.0);
        assert!(!app.dialog_open);
        let doc = app.doc.as_ref().expect("doc loaded");
        assert_eq!(doc.file_name, "poke.sav");
        assert_eq!(
            doc.path.as_deref(),
            Some(std::path::Path::new("/tmp/poke.sav"))
        );
        assert!(!doc.dirty);
        assert!(app.error.is_none());
    }

    #[test]
    fn opened_event_with_short_bytes_sets_load_error() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.io_tx
            .send(IoEvent::Opened {
                file_name: "tiny.sav".into(),
                bytes: vec![0u8; 16],
                path: None,
            })
            .expect("send");
        app.poll_io(&ctx, 0.0);
        assert!(app.doc.is_none());
        assert!(matches!(app.error, Some(AppError::Load(_))));
    }

    #[test]
    fn saved_event_rebaselines_updates_path_and_toasts() {
        let mut app = App::new();
        let mut doc = empty_doc();
        doc.save.set_player_id(0xABCD);
        doc.touch();
        doc.end_frame();
        assert!(doc.dirty);
        app.doc = Some(doc);
        app.dialog_open = true;
        app.io_tx
            .send(saved_event(
                true,
                Some(std::path::PathBuf::from("/tmp/renamed.sav")),
                Some(std::path::PathBuf::from("/tmp/renamed.sav.bak-x")),
                "renamed.sav",
            ))
            .expect("send");
        let ctx = egui::Context::default();
        app.poll_io(&ctx, 1.0);
        assert!(!app.dialog_open);
        let doc = app.doc.as_ref().expect("doc");
        assert!(!doc.dirty, "saved: edits became the new baseline");
        assert_eq!(doc.file_name, "renamed.sav");
        assert_eq!(
            doc.path.as_deref(),
            Some(std::path::Path::new("/tmp/renamed.sav"))
        );
        let toast = app.toast.as_ref().expect("toast");
        assert!(toast.message.contains("Saved to renamed.sav"));
        assert!(toast.message.contains("backup: renamed.sav.bak-x"));
        assert!(toast.expires_at > 1.0);
    }

    #[test]
    fn non_primary_save_keeps_baseline_and_name() {
        let mut app = App::new();
        let mut doc = empty_doc();
        doc.save.set_player_id(0xABCD);
        doc.touch();
        doc.end_frame();
        app.doc = Some(doc);
        app.io_tx
            .send(saved_event(
                false,
                Some(std::path::PathBuf::from("/tmp/copy.sav")),
                None,
                "copy.sav",
            ))
            .expect("send");
        let ctx = egui::Context::default();
        app.poll_io(&ctx, 0.0);
        let doc = app.doc.as_ref().expect("doc");
        assert!(doc.dirty, "copy of original does not rebaseline");
        assert_eq!(doc.file_name, "new.sav");
        let toast = app.toast.as_ref().expect("toast");
        assert!(toast.message.contains("Copy of original saved to copy.sav"));
    }

    #[test]
    fn wasm_style_saved_event_toasts_download() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.io_tx
            .send(saved_event(true, None, None, "new.sav"))
            .expect("send");
        app.poll_io(&ctx, 0.0);
        let toast = app.toast.as_ref().expect("toast");
        assert_eq!(toast.message, "Download started: new.sav");
    }

    #[test]
    fn error_and_cancelled_events() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.dialog_open = true;
        app.io_tx.send(IoEvent::Cancelled).expect("send");
        app.poll_io(&ctx, 0.0);
        assert!(!app.dialog_open);
        assert!(app.error.is_none());

        app.io_tx
            .send(IoEvent::Error(AppError::WasmSave("boom".into())))
            .expect("send");
        app.poll_io(&ctx, 0.0);
        assert!(matches!(app.error, Some(AppError::WasmSave(_))));
    }

    // ---- SD-card discovery wiring ----

    #[cfg(not(target_arch = "wasm32"))]
    fn fake_card(root: &std::path::Path) -> sdcard::OnionCard {
        sdcard::OnionCard {
            root: root.to_path_buf(),
            volume_name: "ONION".to_owned(),
            saves: Vec::new(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_under_a_card_root_gets_the_eject_note() {
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.sd
            .cards
            .push(fake_card(std::path::Path::new("/Volumes/ONION")));
        app.io_tx
            .send(saved_event(
                true,
                Some(std::path::PathBuf::from(
                    "/Volumes/ONION/Saves/CurrentProfile/saves/Gambatte/RED.srm",
                )),
                None,
                "RED.srm",
            ))
            .expect("send");
        let ctx = egui::Context::default();
        app.poll_io(&ctx, 0.0);
        let toast = app.toast.as_ref().expect("toast");
        assert!(
            toast.message.contains("safe to eject the SD card"),
            "eject note expected: {}",
            toast.message
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn save_outside_any_card_gets_no_eject_note() {
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.sd
            .cards
            .push(fake_card(std::path::Path::new("/Volumes/ONION")));
        app.io_tx
            .send(saved_event(
                true,
                Some(std::path::PathBuf::from("/home/user/RED.srm")),
                None,
                "RED.srm",
            ))
            .expect("send");
        let ctx = egui::Context::default();
        app.poll_io(&ctx, 0.0);
        let toast = app.toast.as_ref().expect("toast");
        assert!(!toast.message.contains("safe to eject"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn card_events_update_cards_panel_and_toast() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        // Drop the poller sender so poll_sd does not spawn a real poller.
        let tx = app.sd.tx.take().expect("unspawned poller sender");
        tx.send(sdcard::SdEvent::CardDetected(sdcard::OnionCard {
            root: std::path::PathBuf::from("/Volumes/ONION"),
            volume_name: "ONION".to_owned(),
            saves: vec![sdcard::DiscoveredSave {
                path: std::path::PathBuf::from(
                    "/Volumes/ONION/Saves/CurrentProfile/saves/Gambatte/RED.srm",
                ),
                rom_name: "RED".to_owned(),
                profile: "CurrentProfile".to_owned(),
                legacy: false,
                preview: None,
                diagnostic: Some("x".to_owned()),
                shadowing_state: None,
            }],
        }))
        .expect("send");
        app.poll_sd(&ctx, 0.0);
        assert_eq!(app.sd.cards.len(), 1);
        assert!(app.sd.panel_open, "panel opens when a card has saves");
        let toast = app.toast.as_ref().expect("toast");
        assert!(
            toast
                .message
                .contains("Miyoo SD card detected (ONION) — 1 Pokémon save"),
            "detection toast: {}",
            toast.message
        );

        tx.send(sdcard::SdEvent::CardRemoved(std::path::PathBuf::from(
            "/Volumes/ONION",
        )))
        .expect("send");
        app.poll_sd(&ctx, 1.0);
        assert!(app.sd.cards.is_empty());
        assert!(!app.sd.panel_open);
        let toast = app.toast.as_ref().expect("toast");
        assert!(toast.message.contains("SD card removed"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn open_discovered_loads_the_doc_and_arms_the_shadow_warning() {
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("RED.srm");
        std::fs::write(&path, valid_save_bytes()).expect("seed save");
        let state = dir.path().join("RED.state");

        let mut app = App::new();
        app.request(
            &ctx,
            PendingAction::OpenDiscovered {
                path: path.clone(),
                shadowing_state: Some(state.clone()),
            },
        );
        let doc = app.doc.as_ref().expect("doc");
        assert_eq!(doc.file_name, "RED.srm");
        assert_eq!(doc.path.as_deref(), Some(path.as_path()));
        assert_eq!(app.sd.shadow_warning.as_deref(), Some(state.as_path()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn open_discovered_from_a_pulled_card_reports_a_read_error() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.request(
            &ctx,
            PendingAction::OpenDiscovered {
                path: std::path::PathBuf::from("/nonexistent/RED.srm"),
                shadowing_state: None,
            },
        );
        assert!(app.doc.is_none());
        assert!(matches!(app.error, Some(AppError::Read { .. })));
        assert_eq!(app.sd.shadow_warning, None);
    }

    // ---- dirty / unsaved-changes guard (J2) ----

    fn dropped_action(name: &str) -> PendingAction {
        PendingAction::LoadDropped {
            bytes: valid_save_bytes(),
            file_name: name.to_owned(),
            path: None,
        }
    }

    fn make_dirty(app: &mut App) {
        let doc = app.doc.as_mut().expect("doc");
        doc.save.set_player_id(0x7777);
        doc.touch();
        doc.end_frame();
        assert!(doc.dirty);
    }

    #[test]
    fn clean_request_performs_immediately() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.request(&ctx, dropped_action("dropped.sav"));
        assert!(app.pending.is_none());
        assert_eq!(
            app.doc.as_ref().map(|d| d.file_name.as_str()),
            Some("dropped.sav")
        );
    }

    #[test]
    fn dirty_request_stashes_a_pending_action() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        make_dirty(&mut app);
        app.request(&ctx, dropped_action("dropped.sav"));
        assert!(app.pending.is_some(), "guarded: not performed yet");
        assert_eq!(
            app.doc.as_ref().map(|d| d.file_name.as_str()),
            Some("new.sav"),
            "old doc still loaded"
        );
    }

    #[test]
    fn discard_performs_the_pending_drop() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        make_dirty(&mut app);
        app.request(&ctx, dropped_action("dropped.sav"));
        app.resolve_pending(&ctx, true);
        assert!(app.pending.is_none());
        let doc = app.doc.as_ref().expect("doc");
        assert_eq!(doc.file_name, "dropped.sav");
        assert!(!doc.dirty);
    }

    #[test]
    fn cancel_clears_the_pending_action_and_keeps_edits() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        make_dirty(&mut app);
        app.request(&ctx, dropped_action("dropped.sav"));
        app.resolve_pending(&ctx, false);
        assert!(app.pending.is_none());
        let doc = app.doc.as_ref().expect("doc");
        assert_eq!(doc.file_name, "new.sav");
        assert!(doc.dirty, "edits survive a cancelled discard");
    }

    #[test]
    fn guarded_revert_restores_the_baseline() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        let baseline = app.doc.as_ref().expect("doc").original.clone();
        make_dirty(&mut app);
        app.request(&ctx, PendingAction::Revert);
        assert!(app.pending.is_some());
        app.resolve_pending(&ctx, true);
        let doc = app.doc.as_ref().expect("doc");
        assert!(!doc.dirty);
        assert_eq!(doc.save.to_bytes(), baseline);
    }

    #[test]
    fn guarded_new_replaces_the_doc_only_on_discard() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        make_dirty(&mut app);
        app.request(&ctx, PendingAction::New(GameVariant::Yellow));
        app.resolve_pending(&ctx, true);
        let doc = app.doc.as_ref().expect("doc");
        assert_eq!(doc.variant, GameVariant::Yellow);
        assert!(!doc.dirty);
    }

    // ---- version history (issue #9) ----

    fn version_entry(id: u64) -> history::VersionEntry {
        history::VersionEntry {
            id,
            timestamp: 42,
            label: None,
            sha256: "00".repeat(32),
            size: 0x8000,
            parent_id: None,
            origin: Origin::Save,
        }
    }

    fn version_row(id: u64, origin: Origin) -> VersionRow {
        VersionRow {
            entry: history::VersionEntry {
                origin,
                ..version_entry(id)
            },
            blob_ok: true,
            summary: None,
        }
    }

    #[test]
    fn saved_event_with_version_extends_toast_and_arms_naming() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.history.restore_parent = Some(7); // cleared by the save
        app.io_tx
            .send(IoEvent::Saved {
                primary: true,
                path: Some(std::path::PathBuf::from("/tmp/poke.srm")),
                backup: None,
                file_name: "poke.srm".into(),
                version: Some(version_entry(14)),
                legacy_backups: 0,
                history_error: None,
            })
            .expect("send");
        app.poll_io(&ctx, 0.0);
        let toast = app.toast.as_ref().expect("toast");
        assert!(
            toast.message.contains("· version 14"),
            "toast: {}",
            toast.message
        );
        assert_eq!(app.history.naming, Some(14), "naming field armed");
        assert_eq!(
            app.history.restore_parent, None,
            "restore lineage consumed by the save"
        );
    }

    #[test]
    fn saved_event_history_failure_is_reported_but_save_succeeds() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.io_tx
            .send(IoEvent::Saved {
                primary: true,
                path: Some(std::path::PathBuf::from("/tmp/poke.srm")),
                backup: None,
                file_name: "poke.srm".into(),
                version: None,
                legacy_backups: 0,
                history_error: Some("disk full".into()),
            })
            .expect("send");
        app.poll_io(&ctx, 0.0);
        let toast = app.toast.as_ref().expect("toast");
        assert!(toast.message.contains("Saved to poke.srm"));
        assert!(
            toast.message.contains("history not recorded: disk full"),
            "toast: {}",
            toast.message
        );
        assert_eq!(app.history.naming, None);
    }

    #[test]
    fn saved_event_offers_legacy_import_until_imported_or_dismissed() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        let saved_with_baks = || IoEvent::Saved {
            primary: true,
            path: Some(std::path::PathBuf::from("/tmp/poke.srm")),
            backup: None,
            file_name: "poke.srm".into(),
            version: Some(version_entry(1)),
            legacy_backups: 3,
            history_error: None,
        };
        app.io_tx.send(saved_with_baks()).expect("send");
        app.poll_io(&ctx, 0.0);
        assert_eq!(app.history.legacy_offer, Some(3));

        // Already-imported history suppresses the offer.
        app.history.legacy_offer = None;
        app.history.versions = vec![version_row(1, Origin::Import)];
        app.io_tx.send(saved_with_baks()).expect("send");
        app.poll_io(&ctx, 0.0);
        assert_eq!(app.history.legacy_offer, None);

        // A dismissal sticks for the rest of the document's session.
        app.history.versions.clear();
        app.history.legacy_dismissed = true;
        app.io_tx.send(saved_with_baks()).expect("send");
        app.poll_io(&ctx, 0.0);
        assert_eq!(app.history.legacy_offer, None);
    }

    #[test]
    fn versions_event_updates_rows_and_import_clears_the_offer() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.history.legacy_offer = Some(2);
        app.history
            .tx
            .send(HistoryEvent::Versions(vec![version_row(1, Origin::Save)]))
            .expect("send");
        app.poll_history(&ctx, 0.0);
        assert_eq!(app.history.versions.len(), 1);
        assert_eq!(app.history.legacy_offer, Some(2), "no import yet");

        app.history
            .tx
            .send(HistoryEvent::Versions(vec![
                version_row(1, Origin::Save),
                version_row(2, Origin::Import),
            ]))
            .expect("send");
        app.poll_history(&ctx, 0.0);
        assert_eq!(app.history.versions.len(), 2);
        assert_eq!(
            app.history.legacy_offer, None,
            "imported versions retire the offer"
        );
    }

    #[test]
    fn restore_blob_sets_dirty_without_touching_disk() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        assert_eq!(
            app.doc.as_ref().expect("doc").path,
            None,
            "nowhere to write"
        );

        // A restored snapshot that differs from the current buffer.
        let mut restored = SaveFile::new_empty(GameVariant::RedBlue);
        restored.set_badges(0xFF);
        let blob = restored.to_bytes();

        app.history
            .tx
            .send(HistoryEvent::BlobLoaded {
                id: 5,
                purpose: BlobPurpose::Restore,
                bytes: blob.clone(),
            })
            .expect("send");
        app.poll_history(&ctx, 0.0);

        let doc = app.doc.as_ref().expect("doc");
        assert!(doc.dirty, "restore turns the dirty flag on");
        assert_eq!(doc.save.to_bytes(), blob, "buffer is the restored version");
        assert_eq!(app.history.restore_parent, Some(5));
        let toast = app.toast.as_ref().expect("toast");
        assert!(toast.message.contains("Version 5 loaded"));
    }

    #[test]
    fn history_params_track_restore_lineage_and_settings() {
        let mut app = App::new();
        let params = app.history_params().expect("history on by default");
        assert_eq!(params.origin, Origin::Save);
        assert_eq!(params.parent_id, None);
        assert_eq!(params.max_versions, None);

        app.history.restore_parent = Some(3);
        app.history.settings.max_versions = Some(10);
        let params = app.history_params().expect("params");
        assert_eq!(params.origin, Origin::Restore);
        assert_eq!(params.parent_id, Some(3));
        assert_eq!(params.max_versions, Some(10));

        app.history.settings.enabled = false;
        assert!(
            app.history_params().is_none(),
            "history off: nothing recorded"
        );
    }

    #[test]
    fn recorded_event_extends_an_existing_toast() {
        // The wasm flow: the Saved toast appears first, the async
        // record completes after.
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.show_toast(0.0, "Download started: new.sav".to_owned());
        app.history
            .tx
            .send(HistoryEvent::Recorded(version_entry(2)))
            .expect("send");
        app.poll_history(&ctx, 1.0);
        let toast = app.toast.as_ref().expect("toast");
        assert_eq!(toast.message, "Download started: new.sav · version 2");
        assert_eq!(app.history.naming, Some(2));
    }

    #[test]
    fn diff_blob_builds_the_labeled_diff_view() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        let mut doc = empty_doc();
        // Give the current buffer more money than the snapshot.
        doc.save.set_money(123_456).expect("money in range");
        doc.touch();
        doc.end_frame();
        let snapshot = SaveFile::new_empty(GameVariant::RedBlue).to_bytes();
        app.doc = Some(doc);

        app.history
            .tx
            .send(HistoryEvent::BlobLoaded {
                id: 1,
                purpose: BlobPurpose::Diff,
                bytes: snapshot,
            })
            .expect("send");
        app.poll_history(&ctx, 0.0);

        let diff = app.ui.history.diff.as_ref().expect("diff view");
        assert_eq!(diff.id, 1);
        assert!(diff.byte_count > 0);
        assert!(
            diff.lines.iter().any(|l| l.contains("Money")),
            "money edit labeled: {:?}",
            diff.lines
        );
        assert!(
            diff.lines.iter().any(|l| l.contains("Main checksum")),
            "checksum change labeled: {:?}",
            diff.lines
        );
        assert_eq!(app.screen, Screen::History, "diff navigates to History");
    }

    #[test]
    fn restore_is_guarded_by_the_dirty_check() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        make_dirty(&mut app);
        app.handle_history_action(&ctx, HistoryAction::Restore(1));
        assert!(
            matches!(app.pending, Some(PendingAction::RestoreVersion(1))),
            "dirty: restore waits for the discard confirmation"
        );
    }

    #[test]
    fn delete_action_asks_for_confirmation_first() {
        let ctx = egui::Context::default();
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.handle_history_action(&ctx, HistoryAction::Delete(4));
        assert_eq!(app.history.pending_delete, Some(4));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn end_to_end_native_history_flow_through_the_stores() {
        // Open a real file, attach the store, record via the io flow,
        // then rename and restore through events — the full loop.
        let ctx = egui::Context::default();
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("poke.srm");
        std::fs::write(&path, valid_save_bytes()).expect("seed");

        let mut app = App::new();
        app.io_tx
            .send(IoEvent::Opened {
                file_name: "poke.srm".into(),
                bytes: valid_save_bytes(),
                path: Some(path.clone()),
            })
            .expect("send");
        app.poll_io(&ctx, 0.0);
        app.poll_history(&ctx, 0.0);
        assert!(app.history.store.is_some(), "store attached to the path");
        assert!(app.history.versions.is_empty(), "no versions yet");

        // Save through the (post-dialog) io flow, as spawn_save would.
        let mut doc = app.doc.take().expect("doc");
        doc.save.set_badges(0x01);
        doc.touch();
        doc.end_frame();
        let bytes = doc.save.to_bytes();
        app.doc = Some(doc);
        let event = io::write_picked(
            &path,
            &bytes,
            Some(&path),
            true,
            app.history_params().as_ref(),
        );
        app.io_tx.send(event).expect("send");
        app.poll_io(&ctx, 1.0);
        app.poll_history(&ctx, 1.0);
        assert_eq!(app.history.versions.len(), 1);
        assert_eq!(app.history.naming, Some(1));

        // Name it through the store, as the toast field would.
        if let Some(store) = &mut app.history.store {
            store.set_label(1, Some("first badge".into()));
        }
        app.poll_history(&ctx, 2.0);
        assert_eq!(
            app.history.versions[0].entry.label.as_deref(),
            Some("first badge")
        );

        // Restore it (clean doc: performs immediately) and check the
        // buffer matches the snapshot while the file is untouched.
        let before_on_disk = std::fs::read(&path).expect("on disk");
        app.request(&ctx, PendingAction::RestoreVersion(1));
        app.poll_history(&ctx, 3.0);
        assert_eq!(app.history.restore_parent, Some(1));
        assert_eq!(
            app.doc.as_ref().expect("doc").save.to_bytes(),
            bytes,
            "buffer holds the restored snapshot"
        );
        assert_eq!(
            std::fs::read(&path).expect("on disk"),
            before_on_disk,
            "nothing written to disk by the restore"
        );
    }
}
