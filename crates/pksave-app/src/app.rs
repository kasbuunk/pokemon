//! Application state: the loaded document, screen routing, menu/status
//! bars, modals and the unsaved-changes guards.

use std::ops::Range;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};

use pksave::gen1::detect::detect_variant;
use pksave::gen1::save::{changed_ranges, GameVariant, SaveFile};
use pksave::{Diagnostic, Severity};

use crate::error::AppError;
use crate::io::{self, IoEvent, SaveRequest};
use crate::screens;

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
}

impl Screen {
    pub const ALL: [Screen; 10] = [
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
}

/// A transient status-bar confirmation (e.g. after a save).
struct Toast {
    message: String,
    /// `egui` time (seconds) after which the toast disappears.
    expires_at: f64,
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
        }
    }

    fn load_bytes(&mut self, bytes: Vec<u8>, file_name: String, path: Option<std::path::PathBuf>) {
        match Doc::from_bytes(bytes, file_name, path) {
            Ok(doc) => {
                publish_dirty(false);
                self.doc = Some(doc);
                self.ui = screens::ScreenState::default();
                self.screen = Screen::Overview;
            }
            Err(e) => self.error = Some(AppError::Load(e)),
        }
    }

    fn poll_io(&mut self, now: f64) {
        while let Ok(event) = self.io_rx.try_recv() {
            self.dialog_open = false;
            match event {
                IoEvent::Opened {
                    file_name,
                    bytes,
                    path,
                } => self.load_bytes(bytes, file_name, path),
                IoEvent::Saved {
                    primary,
                    path,
                    backup,
                    file_name,
                } => {
                    let is_native = path.is_some();
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
                    self.toast = Some(Toast {
                        message,
                        expires_at: now + TOAST_SECONDS,
                    });
                }
                IoEvent::Cancelled => {}
                IoEvent::Error(e) => self.error = Some(e),
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
            } => self.load_bytes(bytes, file_name, path),
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

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let now = ui.ctx().input(|i| i.time);
        if self.toast.as_ref().is_some_and(|t| t.expires_at <= now) {
            self.toast = None;
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
                    if ui.small_button("✕").on_hover_text("Dismiss").clicked() {
                        self.toast = None;
                    } else {
                        // Wake up in time to expire the toast.
                        ui.ctx()
                            .request_repaint_after(std::time::Duration::from_secs(1));
                    }
                }
            });
        });
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
            }
            if let Some((screen, offset)) = goto {
                self.screen = screen;
                self.ui.hex.scroll_to(offset);
            }
        });
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
        self.poll_io(now);
        self.handle_dropped_files(&ctx);
        self.handle_shortcuts(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.handle_close_request(&ctx);

        self.menu_bar(ui);
        self.status_bar(ui);
        self.side_panel(ui);
        self.central(ui);
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

    #[test]
    fn opened_event_loads_a_doc_and_clears_dialog_flag() {
        let mut app = App::new();
        app.dialog_open = true;
        app.io_tx
            .send(IoEvent::Opened {
                file_name: "poke.sav".into(),
                bytes: valid_save_bytes(),
                path: Some(std::path::PathBuf::from("/tmp/poke.sav")),
            })
            .expect("send");
        app.poll_io(0.0);
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
        let mut app = App::new();
        app.io_tx
            .send(IoEvent::Opened {
                file_name: "tiny.sav".into(),
                bytes: vec![0u8; 16],
                path: None,
            })
            .expect("send");
        app.poll_io(0.0);
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
            .send(IoEvent::Saved {
                primary: true,
                path: Some(std::path::PathBuf::from("/tmp/renamed.sav")),
                backup: Some(std::path::PathBuf::from("/tmp/renamed.sav.bak-x")),
                file_name: "renamed.sav".into(),
            })
            .expect("send");
        app.poll_io(1.0);
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
            .send(IoEvent::Saved {
                primary: false,
                path: Some(std::path::PathBuf::from("/tmp/copy.sav")),
                backup: None,
                file_name: "copy.sav".into(),
            })
            .expect("send");
        app.poll_io(0.0);
        let doc = app.doc.as_ref().expect("doc");
        assert!(doc.dirty, "copy of original does not rebaseline");
        assert_eq!(doc.file_name, "new.sav");
        let toast = app.toast.as_ref().expect("toast");
        assert!(toast.message.contains("Copy of original saved to copy.sav"));
    }

    #[test]
    fn wasm_style_saved_event_toasts_download() {
        let mut app = App::new();
        app.doc = Some(empty_doc());
        app.io_tx
            .send(IoEvent::Saved {
                primary: true,
                path: None,
                backup: None,
                file_name: "new.sav".into(),
            })
            .expect("send");
        app.poll_io(0.0);
        let toast = app.toast.as_ref().expect("toast");
        assert_eq!(toast.message, "Download started: new.sav");
    }

    #[test]
    fn error_and_cancelled_events() {
        let mut app = App::new();
        app.dialog_open = true;
        app.io_tx.send(IoEvent::Cancelled).expect("send");
        app.poll_io(0.0);
        assert!(!app.dialog_open);
        assert!(app.error.is_none());

        app.io_tx
            .send(IoEvent::Error(AppError::WasmSave("boom".into())))
            .expect("send");
        app.poll_io(0.0);
        assert!(matches!(app.error, Some(AppError::WasmSave(_))));
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
}
