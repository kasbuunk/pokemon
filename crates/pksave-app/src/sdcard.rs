//! OnionOS/Miyoo SD-card save discovery (native only).
//!
//! When a Miyoo Mini (Plus) SD card is inserted, a background poller
//! notices the new mount, scans it for Gen 1 Gambatte battery saves and
//! offers them to the UI. The scanning core ([`scan_volume`]) and the
//! poller's diff logic ([`RootTracker`]) are pure functions over the
//! filesystem so they can be unit-tested against fake card trees in
//! tempdirs.
//!
//! On-card layout (verified against OnionUI/Onion's shipped
//! `retroarch.cfg`; see HANDOFF.md §C1):
//!
//! - Battery saves: `<root>/Saves/<profile>/saves/Gambatte/<rom>.srm`.
//!   Every `Saves/*/saves/` profile child is enumerated (OnionOS supports
//!   guest/secondary profiles besides `CurrentProfile`). The core dir is
//!   literally `Gambatte` (capital G); other cores' dirs (e.g. `gpSP`)
//!   are ignored.
//! - Legacy/stock path (older Onion, stock Miyoo firmware):
//!   `<root>/RetroArch/.retroarch/saves/<CORE>/` — scanned as a fallback
//!   and marked [`DiscoveredSave::legacy`].
//! - Save states: `<root>/Saves/<profile>/states/Gambatte/<rom>.state*`.
//!   OnionOS sets `savestate_auto_load = "true"`, so an existing state
//!   **overrides an edited `.srm`** on next launch; a matching state is
//!   recorded in [`DiscoveredSave::shadowing_state`] so the UI can warn.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Sender;
use std::time::Duration;

use pksave::gen1::checksum::{self, Region};
use pksave::gen1::pokemon::PartyMon as _;
use pksave::gen1::save::SaveFile;

/// The libretro core directory holding Gen 1 battery saves — literally
/// `Gambatte`, capital G (other cores' dirs, e.g. `gpSP`, are ignored).
const GAMBATTE_DIR: &str = "Gambatte";

/// Exact size in bytes of an on-card Gen 1 Gambatte battery save.
///
/// Deliberately stricter than `SaveFile::from_bytes` (which tolerates
/// ≥ 32 KiB to accept emulator padding and RTC footers): Gambatte writes
/// Gen 1 `.srm` files as raw 32 KiB SRAM dumps with no header or sidecar,
/// so anything else in the core dir (Gen 2 saves with RTC footers, GBA
/// saves, junk) is *not* a Gen 1 card save and is rejected outright.
pub const GEN1_SRM_SIZE: u64 = 32_768;

/// How often the background poller re-enumerates candidate mount roots.
const POLL_INTERVAL: Duration = Duration::from_secs(3);

/// Preview of a parsed, trustworthy save (main checksum valid).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavePreview {
    /// Decoded trainer name.
    pub trainer_name: String,
    /// Number of gym badges obtained (0..=8).
    pub badges: u32,
    /// Play time as `H:MM`.
    pub play_time: String,
    /// E.g. "3 mons, lv 42 max" or "empty party".
    pub party_summary: String,
}

/// One `.srm` found on the card.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredSave {
    /// Absolute path of the `.srm` on the card.
    pub path: PathBuf,
    /// ROM basename (file stem of the `.srm`).
    pub rom_name: String,
    /// Profile dir under `Saves/` (e.g. `CurrentProfile`); `"stock"` for
    /// legacy entries (the stock path has no profiles).
    pub profile: String,
    /// Found under the legacy/stock `RetroArch/.retroarch/saves/` path.
    pub legacy: bool,
    /// `Some` iff the save parses and its main checksum is valid or
    /// repairable (see [`scan_volume`]); `None` entries are shown greyed.
    pub preview: Option<SavePreview>,
    /// Why there is no preview (shown on greyed entries).
    pub diagnostic: Option<String>,
    /// A save state that will shadow an edited `.srm` on next launch
    /// (OnionOS auto-loads states), if one exists.
    pub shadowing_state: Option<PathBuf>,
}

/// An OnionOS/Miyoo SD card and the Gen 1 saves found on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OnionCard {
    /// Mount root of the card.
    pub root: PathBuf,
    /// Volume label (the mount root's file name).
    pub volume_name: String,
    /// Discovered saves, in a stable (sorted) order.
    pub saves: Vec<DiscoveredSave>,
}

/// Whether `root` looks like an OnionOS/Miyoo card, judged **only** by
/// marker entries at the volume root (never by volume name):
///
/// - `.tmp_update/` (the OnionOS boot hook) alone is conclusive;
/// - otherwise at least two of `Saves/CurrentProfile/`, `miyoo/`,
///   `RetroArch/`, `Roms/GB/` must be present (any single one is too
///   weak — e.g. any desktop RetroArch install has a `RetroArch/` dir).
pub fn is_onion_card(root: &Path) -> bool {
    if root.join(".tmp_update").is_dir() {
        return true;
    }
    let secondary = [
        root.join("Saves").join("CurrentProfile"),
        root.join("miyoo"),
        root.join("RetroArch"),
        root.join("Roms").join("GB"),
    ];
    secondary.iter().filter(|m| m.is_dir()).count() >= 2
}

/// Scan a mounted volume. Returns `None` if the markers say it is not an
/// OnionOS/Miyoo card; otherwise the card with every discovered Gen 1
/// save (possibly zero).
///
/// A file is listed only if it is exactly [`GEN1_SRM_SIZE`] bytes. It
/// gets a [`SavePreview`] only if it parses with checksums
/// valid-or-repairable: the *main* data checksum must match (stale
/// per-box/bank checksums are common on real saves and always
/// mechanically repairable, so they don't block a preview). A file whose
/// main checksum is wrong is included greyed (`preview: None`) with its
/// diagnostic string.
pub fn scan_volume(root: &Path) -> Option<OnionCard> {
    if !is_onion_card(root) {
        return None;
    }
    let volume_name = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string());

    let mut saves = Vec::new();

    // Modern OnionOS layout: every profile under Saves/ (CurrentProfile
    // plus any guest/secondary profiles).
    for profile_dir in sorted_children(&root.join("Saves")) {
        if !profile_dir.is_dir() {
            continue;
        }
        let profile = profile_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        collect_saves(
            &profile_dir.join("saves").join(GAMBATTE_DIR),
            &profile_dir.join("states").join(GAMBATTE_DIR),
            &profile,
            false,
            &mut saves,
        );
    }

    // Legacy/stock fallback (older Onion, stock Miyoo firmware).
    let legacy_root = root.join("RetroArch").join(".retroarch");
    collect_saves(
        &legacy_root.join("saves").join(GAMBATTE_DIR),
        &legacy_root.join("states").join(GAMBATTE_DIR),
        "stock",
        true,
        &mut saves,
    );

    saves.sort_by(|a, b| a.path.cmp(&b.path));
    Some(OnionCard {
        root: root.to_path_buf(),
        volume_name,
        saves,
    })
}

/// Children of `dir`, sorted for a stable scan order; empty when the dir
/// is missing or unreadable.
fn sorted_children(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut children: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    children.sort();
    children
}

/// Scan one core save dir for Gen 1 `.srm` files, pairing each with a
/// shadowing state from the sibling states dir.
fn collect_saves(
    saves_dir: &Path,
    states_dir: &Path,
    profile: &str,
    legacy: bool,
    out: &mut Vec<DiscoveredSave>,
) {
    for path in sorted_children(saves_dir) {
        if !path.is_file()
            || !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("srm"))
        {
            continue;
        }
        let Some(rom_name) = path.file_stem().map(|s| s.to_string_lossy().into_owned()) else {
            continue;
        };
        // Gen 1 gate: exactly 32 KiB (see GEN1_SRM_SIZE) — wrong-size
        // files are not Gen 1 card saves and are rejected outright.
        let Ok(metadata) = std::fs::metadata(&path) else {
            continue;
        };
        if metadata.len() != GEN1_SRM_SIZE {
            continue;
        }
        let (preview, diagnostic) = match std::fs::read(&path) {
            Ok(bytes) => preview_or_diagnostic(bytes),
            Err(e) => (None, Some(format!("could not read the file: {e}"))),
        };
        let shadowing_state = find_shadowing_state(states_dir, &rom_name);
        out.push(DiscoveredSave {
            path,
            rom_name,
            profile: profile.to_owned(),
            legacy,
            preview,
            diagnostic,
            shadowing_state,
        });
    }
}

/// Parse a size-gated save image: a preview when the *main* data
/// checksum is valid, otherwise the diagnostic for the greyed entry.
/// Stale per-box/bank checksums don't block a preview — they are common
/// on real saves and always mechanically repairable, whereas a wrong
/// main checksum means the very bytes the preview reads are untrustworthy.
fn preview_or_diagnostic(bytes: Vec<u8>) -> (Option<SavePreview>, Option<String>) {
    let save = match SaveFile::from_bytes(bytes) {
        Ok(save) => save,
        Err(e) => return (None, Some(e.to_string())),
    };
    let main_mismatch = checksum::verify(save.as_bytes())
        .into_iter()
        .find(|m| m.region == Region::Main);
    if let Some(m) = main_mismatch {
        return (
            None,
            Some(format!(
                "main checksum invalid (stored 0x{:02X}, computed 0x{:02X}) — not a valid Gen 1 save",
                m.stored, m.computed
            )),
        );
    }
    (Some(build_preview(&save)), None)
}

fn build_preview(save: &SaveFile) -> SavePreview {
    let time = save.play_time();
    let party = save.party();
    let party_summary = if party.is_empty() {
        "empty party".to_owned()
    } else {
        let count = party.len();
        let max_level = (0..count).map(|i| party.mon(i).level()).max().unwrap_or(0);
        let plural = if count == 1 { "" } else { "s" };
        format!("{count} mon{plural}, lv {max_level} max")
    };
    SavePreview {
        trainer_name: save.player_name(),
        badges: save.badges().count_ones(),
        play_time: format!("{}:{:02}", time.hours, time.minutes),
        party_summary,
    }
}

/// The first (sorted) save state in `states_dir` that shadows `rom`.
fn find_shadowing_state(states_dir: &Path, rom: &str) -> Option<PathBuf> {
    sorted_children(states_dir).into_iter().find(|p| {
        p.is_file()
            && p.file_name()
                .is_some_and(|n| is_shadowing_state_name(rom, &n.to_string_lossy()))
    })
}

/// Whether `file_name` is a RetroArch save state that shadows the ROM
/// named `rom`: `<rom>.state`, a numbered slot (`<rom>.state1`, …) or
/// the auto-save state (`<rom>.state.auto`). Our own neutralized backups
/// (`<rom>.state.bak-<ts>`) do **not** match.
pub fn is_shadowing_state_name(rom: &str, file_name: &str) -> bool {
    let Some(rest) = file_name.strip_prefix(rom) else {
        return false;
    };
    let Some(suffix) = rest.strip_prefix(".state") else {
        return false;
    };
    suffix.is_empty() || suffix == ".auto" || suffix.chars().all(|c| c.is_ascii_digit())
}

/// Rename a shadowing save state to `<name>.state.bak-<ts>` (appending
/// `-2`, `-3`, … on a name collision) so OnionOS no longer auto-loads it
/// over the edited battery save. Returns the new path.
pub fn neutralize_state(state: &Path) -> std::io::Result<PathBuf> {
    let name = state
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save.state".to_owned());
    let base = state.with_file_name(format!(
        "{name}.bak-{}",
        crate::io::backup_timestamp(crate::io::now_secs())
    ));
    let mut candidate = base.clone();
    let mut counter = 2u32;
    while candidate.exists() && counter <= 1000 {
        let base_name = base
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "save.state.bak".to_owned());
        candidate = base.with_file_name(format!("{base_name}-{counter}"));
        counter += 1;
    }
    std::fs::rename(state, &candidate)?;
    Ok(candidate)
}

/// The discovered card whose root contains `path`, if any (drives the
/// "safe to eject" note after saving back to a card).
pub fn containing_card<'a>(cards: &'a [OnionCard], path: &Path) -> Option<&'a OnionCard> {
    // `starts_with` compares whole components, so a sibling volume
    // sharing a name prefix (ONION vs ONION2) does not match.
    cards.iter().find(|card| path.starts_with(&card.root))
}

/// Added/removed mount roots between two polls.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RootsDiff {
    /// Roots present now that were absent last poll.
    pub added: Vec<PathBuf>,
    /// Roots absent now that were present last poll.
    pub removed: Vec<PathBuf>,
}

impl RootsDiff {
    /// No roots appeared or vanished.
    #[cfg_attr(not(test), allow(dead_code))] // steady-state assertion helper
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// The poller's pure diff logic: feed it each enumeration of candidate
/// roots and it reports every appearance/disappearance exactly once.
#[derive(Debug, Default)]
pub struct RootTracker {
    known: BTreeSet<PathBuf>,
}

impl RootTracker {
    /// Diff `roots` (the current enumeration) against the previous one.
    pub fn poll(&mut self, roots: Vec<PathBuf>) -> RootsDiff {
        let current: BTreeSet<PathBuf> = roots.into_iter().collect();
        let added = current.difference(&self.known).cloned().collect();
        let removed = self.known.difference(&current).cloned().collect();
        self.known = current;
        RootsDiff { added, removed }
    }
}

/// Event sent from the poller thread to the UI thread.
#[derive(Debug)]
pub enum SdEvent {
    /// A newly mounted OnionOS card (also fired for cards already
    /// mounted when the app started).
    CardDetected(OnionCard),
    /// A previously detected card's mount root vanished.
    CardRemoved(PathBuf),
}

/// Spawn the background poller thread: every [`POLL_INTERVAL`] it
/// enumerates candidate mount roots, diffs them against the previous
/// set, scans newly appeared mounts with [`scan_volume`] and reports
/// detected/removed cards over `tx` (same mpsc→frame-loop pattern as
/// `crate::io`). Exits when the receiver is dropped.
pub fn spawn_poller(tx: Sender<SdEvent>, ctx: egui::Context) {
    std::thread::spawn(move || {
        let mut tracker = RootTracker::default();
        let mut card_roots: BTreeSet<PathBuf> = BTreeSet::new();
        loop {
            let diff = tracker.poll(list_roots());
            for root in diff.added {
                if let Some(card) = scan_volume(&root) {
                    card_roots.insert(root);
                    if tx.send(SdEvent::CardDetected(card)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
            for root in diff.removed {
                if card_roots.remove(&root) {
                    if tx.send(SdEvent::CardRemoved(root)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

/// Candidate mount roots on macOS: every directory under `/Volumes`.
#[cfg(target_os = "macos")]
fn list_roots() -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir("/Volumes") else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

/// Candidate mount roots on Linux/Windows: removable disks per `sysinfo`.
#[cfg(not(target_os = "macos"))]
fn list_roots() -> Vec<PathBuf> {
    sysinfo::Disks::new_with_refreshed_list()
        .iter()
        .filter(|d| d.is_removable())
        .map(|d| d.mount_point().to_path_buf())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pksave::gen1::save::GameVariant;
    use std::fs;

    /// A tempdir posing as a mounted volume root.
    fn volume(name: &str) -> tempfile::TempDir {
        tempfile::Builder::new()
            .prefix(name)
            .tempdir()
            .expect("create tempdir")
    }

    fn mkdirs(root: &Path, rel: &str) -> PathBuf {
        let dir = root.join(rel);
        fs::create_dir_all(&dir).expect("create fake card dirs");
        dir
    }

    fn write_file(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, bytes).expect("write fake save");
        path
    }

    /// A valid, freshly initialized Gen 1 save image (all 15 checksums
    /// good, trainer name "RED").
    fn valid_save_bytes() -> Vec<u8> {
        SaveFile::new_empty(GameVariant::RedBlue).to_bytes()
    }

    /// Markers for a canonical OnionOS card.
    fn make_onion_markers(root: &Path) {
        mkdirs(root, ".tmp_update");
        mkdirs(root, "Saves/CurrentProfile");
        mkdirs(root, "miyoo");
        mkdirs(root, "RetroArch");
        mkdirs(root, "Roms/GB");
    }

    // ---- marker detection -------------------------------------------------

    #[test]
    fn tmp_update_alone_marks_a_card() {
        let vol = volume("ONION");
        mkdirs(vol.path(), ".tmp_update");
        assert!(is_onion_card(vol.path()));
    }

    #[test]
    fn secondary_markers_mark_a_card_without_tmp_update() {
        let vol = volume("ONION");
        mkdirs(vol.path(), "miyoo");
        mkdirs(vol.path(), "Roms/GB");
        assert!(is_onion_card(vol.path()));
    }

    #[test]
    fn camera_card_is_not_a_card() {
        let vol = volume("CANON");
        mkdirs(vol.path(), "DCIM/100CANON");
        assert!(!is_onion_card(vol.path()));
        assert!(scan_volume(vol.path()).is_none());
    }

    #[test]
    fn empty_volume_is_not_a_card() {
        let vol = volume("UNTITLED");
        assert!(!is_onion_card(vol.path()));
        assert!(scan_volume(vol.path()).is_none());
    }

    #[test]
    fn saves_dir_without_profile_dirs_is_not_a_card() {
        let vol = volume("NAS");
        mkdirs(vol.path(), "Saves");
        assert!(!is_onion_card(vol.path()));
        assert!(scan_volume(vol.path()).is_none());
    }

    #[test]
    fn a_single_secondary_marker_is_not_enough() {
        // Any desktop RetroArch install has a RetroArch/ dir; volume
        // names never matter.
        let vol = volume("ONION"); // deceptive label
        mkdirs(vol.path(), "RetroArch");
        assert!(!is_onion_card(vol.path()));
    }

    // ---- save enumeration -------------------------------------------------

    #[test]
    fn enumerates_every_profile_not_just_current() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let cur = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        let guest = mkdirs(vol.path(), "Saves/GuestProfile/saves/Gambatte");
        write_file(&cur, "POKEMON RED.srm", &valid_save_bytes());
        write_file(&guest, "POKEMON BLUE.srm", &valid_save_bytes());

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves.len(), 2);
        let mut profiles: Vec<(&str, &str)> = card
            .saves
            .iter()
            .map(|s| (s.profile.as_str(), s.rom_name.as_str()))
            .collect();
        profiles.sort_unstable();
        assert_eq!(
            profiles,
            vec![
                ("CurrentProfile", "POKEMON RED"),
                ("GuestProfile", "POKEMON BLUE"),
            ]
        );
        assert!(card.saves.iter().all(|s| !s.legacy));
    }

    #[test]
    fn legacy_retroarch_path_is_scanned_and_flagged() {
        let vol = volume("MIYOO");
        // A stock-firmware card: no .tmp_update, no Saves/CurrentProfile.
        mkdirs(vol.path(), "miyoo");
        mkdirs(vol.path(), "Roms/GB");
        let legacy = mkdirs(vol.path(), "RetroArch/.retroarch/saves/Gambatte");
        write_file(&legacy, "POKEMON YELLOW.srm", &valid_save_bytes());

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves.len(), 1);
        assert!(card.saves[0].legacy);
        assert_eq!(card.saves[0].rom_name, "POKEMON YELLOW");
        assert!(card.saves[0].preview.is_some());
    }

    #[test]
    fn other_core_dirs_are_ignored() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let gambatte = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        let gpsp = mkdirs(vol.path(), "Saves/CurrentProfile/saves/gpSP");
        let legacy_gpsp = mkdirs(vol.path(), "RetroArch/.retroarch/saves/gpSP");
        write_file(&gambatte, "POKEMON RED.srm", &valid_save_bytes());
        // Valid Gen 1 bytes in the wrong core dir must still be ignored.
        write_file(&gpsp, "GBA GAME.srm", &valid_save_bytes());
        write_file(&legacy_gpsp, "GBA GAME.srm", &valid_save_bytes());

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves.len(), 1);
        assert_eq!(card.saves[0].rom_name, "POKEMON RED");
    }

    #[test]
    fn volume_name_is_the_mount_dir_name() {
        let vol = volume("MYCARD");
        make_onion_markers(vol.path());
        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.root, vol.path());
        assert!(card.volume_name.starts_with("MYCARD"));
        assert!(card.saves.is_empty());
    }

    // ---- size gate ----------------------------------------------------

    #[test]
    fn size_gate_rejects_non_32768_byte_files() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let dir = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        write_file(&dir, "TINY.srm", &[0u8; 512]);
        // Deliberately stricter than SaveFile::from_bytes, which
        // tolerates 64 KiB: on-card Gen 1 Gambatte saves are exactly
        // 32 KiB, so a 64 KiB file is not a Gen 1 card save.
        write_file(&dir, "PADDED.srm", &[0u8; 65_536]);

        let card = scan_volume(vol.path()).expect("is a card");
        assert!(
            card.saves.is_empty(),
            "wrong-size files must be rejected, got {:?}",
            card.saves
        );
    }

    #[test]
    fn size_gate_accepts_exactly_32768() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let dir = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        write_file(&dir, "RED.srm", &valid_save_bytes());
        assert_eq!(valid_save_bytes().len() as u64, GEN1_SRM_SIZE);

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves.len(), 1);
    }

    // ---- preview / greyed entries ---------------------------------------

    #[test]
    fn checksum_invalid_file_is_listed_greyed_with_diagnostic() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let dir = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        // Right size, but a zeroed image has an invalid main checksum.
        write_file(&dir, "CORRUPT.srm", &[0u8; GEN1_SRM_SIZE as usize]);

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves.len(), 1);
        let save = &card.saves[0];
        assert!(save.preview.is_none(), "corrupt file must have no preview");
        let diag = save.diagnostic.as_deref().expect("diagnostic present");
        assert!(
            diag.to_ascii_lowercase().contains("checksum"),
            "diagnostic should mention the checksum: {diag}"
        );
    }

    #[test]
    fn valid_file_yields_a_correct_preview() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let dir = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        write_file(&dir, "POKEMON RED.srm", &valid_save_bytes());

        let card = scan_volume(vol.path()).expect("is a card");
        let save = &card.saves[0];
        assert!(save.diagnostic.is_none());
        let preview = save.preview.as_ref().expect("valid save has a preview");
        assert_eq!(preview.trainer_name, "RED");
        assert_eq!(preview.badges, 0);
        assert_eq!(preview.play_time, "0:00");
        assert_eq!(preview.party_summary, "empty party");
    }

    #[test]
    fn preview_reflects_edited_trainer_and_party() {
        let mut save = SaveFile::new_empty(GameVariant::RedBlue);
        save.set_player_name("ASH").expect("name fits");
        save.set_badges(0b0000_0111);
        save.set_play_time(pksave::gen1::trainer::PlayTime {
            hours: 12,
            minutes: 5,
            seconds: 0,
            frames: 0,
            maxed: false,
        });
        // Raw party mon record: species (internal index) at +0x00,
        // authoritative party level at +0x21 (see docs/FORMAT.md).
        let mut mon = [0u8; pksave::gen1::offsets::PARTY_MON_SIZE];
        mon[0x00] = 0x99; // Bulbasaur's internal index
        mon[0x21] = 42;
        save.party_mut()
            .add(&mon, "ASH", "BULBA")
            .expect("party has room");

        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let dir = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        write_file(&dir, "POKEMON RED.srm", &save.to_bytes());

        let card = scan_volume(vol.path()).expect("is a card");
        let preview = card.saves[0].preview.as_ref().expect("preview");
        assert_eq!(preview.trainer_name, "ASH");
        assert_eq!(preview.badges, 3);
        assert_eq!(preview.play_time, "12:05");
        assert_eq!(preview.party_summary, "1 mon, lv 42 max");
    }

    // ---- save-state shadowing -------------------------------------------

    #[test]
    fn matching_state_is_detected() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let saves = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        let states = mkdirs(vol.path(), "Saves/CurrentProfile/states/Gambatte");
        write_file(&saves, "POKEMON RED.srm", &valid_save_bytes());
        let state = write_file(&states, "POKEMON RED.state", b"state");
        write_file(&states, "SOME OTHER GAME.state", b"state");

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves[0].shadowing_state.as_deref(), Some(&*state));
    }

    #[test]
    fn non_matching_state_is_ignored() {
        let vol = volume("ONION");
        make_onion_markers(vol.path());
        let saves = mkdirs(vol.path(), "Saves/CurrentProfile/saves/Gambatte");
        let states = mkdirs(vol.path(), "Saves/CurrentProfile/states/Gambatte");
        write_file(&saves, "POKEMON RED.srm", &valid_save_bytes());
        write_file(&states, "POKEMON BLUE.state", b"state");

        let card = scan_volume(vol.path()).expect("is a card");
        assert_eq!(card.saves[0].shadowing_state, None);
    }

    #[test]
    fn state_name_matching_covers_slots_and_auto() {
        assert!(is_shadowing_state_name("POKEMON RED", "POKEMON RED.state"));
        assert!(is_shadowing_state_name("POKEMON RED", "POKEMON RED.state1"));
        assert!(is_shadowing_state_name(
            "POKEMON RED",
            "POKEMON RED.state12"
        ));
        assert!(is_shadowing_state_name(
            "POKEMON RED",
            "POKEMON RED.state.auto"
        ));
        // Our own neutralized backups must not re-trigger the warning.
        assert!(!is_shadowing_state_name(
            "POKEMON RED",
            "POKEMON RED.state.bak-20260715-101112"
        ));
        // Other ROMs' states, prefixes and unrelated files don't match.
        assert!(!is_shadowing_state_name(
            "POKEMON RED",
            "POKEMON BLUE.state"
        ));
        assert!(!is_shadowing_state_name("POKEMON RED", "POKEMON RED.srm"));
        assert!(!is_shadowing_state_name(
            "POKEMON RED",
            "POKEMON RED 2.state"
        ));
        assert!(!is_shadowing_state_name(
            "POKEMON RED",
            "POKEMON RED.statex"
        ));
    }

    #[test]
    fn neutralize_state_renames_and_stops_shadowing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = write_file(dir.path(), "POKEMON RED.state", b"state");
        let renamed = neutralize_state(&state).expect("rename succeeds");
        assert!(!state.exists(), "original state is gone");
        assert!(renamed.exists());
        let name = renamed.file_name().expect("name").to_string_lossy();
        assert!(
            name.starts_with("POKEMON RED.state.bak-"),
            "renamed to a .state.bak-<ts> name: {name}"
        );
        // The renamed file no longer counts as a shadowing state.
        assert!(!is_shadowing_state_name("POKEMON RED", &name));
    }

    #[test]
    fn neutralize_state_does_not_clobber_an_existing_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let state = write_file(dir.path(), "RED.state", b"first");
        let first = neutralize_state(&state).expect("first rename");
        // A second state appears within the same second.
        let state = write_file(dir.path(), "RED.state", b"second");
        let second = neutralize_state(&state).expect("second rename");
        assert_ne!(first, second);
        assert_eq!(fs::read(&first).expect("first bytes"), b"first");
        assert_eq!(fs::read(&second).expect("second bytes"), b"second");
    }

    // ---- poller diff logic ------------------------------------------------

    #[test]
    fn tracker_reports_each_transition_exactly_once() {
        let a = PathBuf::from("/Volumes/A");
        let b = PathBuf::from("/Volumes/B");
        // The injected list_roots() sequence.
        let sequence: Vec<Vec<PathBuf>> = vec![
            vec![],
            vec![a.clone()],
            vec![a.clone()],
            vec![a.clone(), b.clone()],
            vec![b.clone()],
            vec![],
            vec![],
        ];
        let mut tracker = RootTracker::default();
        let diffs: Vec<RootsDiff> = sequence
            .into_iter()
            .map(|roots| tracker.poll(roots))
            .collect();

        assert!(diffs[0].is_empty());
        assert_eq!(diffs[1].added, vec![a.clone()]);
        assert!(diffs[1].removed.is_empty());
        assert!(diffs[2].is_empty(), "steady state reports nothing");
        assert_eq!(diffs[3].added, vec![b.clone()]);
        assert!(diffs[3].removed.is_empty());
        assert!(diffs[4].added.is_empty());
        assert_eq!(diffs[4].removed, vec![a.clone()]);
        assert_eq!(diffs[5].removed, vec![b.clone()]);
        assert!(diffs[6].is_empty());
    }

    #[test]
    fn tracker_reports_first_poll_mounts_as_added() {
        // A card already inserted at app start must still be detected.
        let mut tracker = RootTracker::default();
        let diff = tracker.poll(vec![PathBuf::from("/Volumes/ONION")]);
        assert_eq!(diff.added, vec![PathBuf::from("/Volumes/ONION")]);
        assert!(diff.removed.is_empty());
    }

    #[test]
    fn tracker_handles_simultaneous_swap() {
        let mut tracker = RootTracker::default();
        tracker.poll(vec![PathBuf::from("/Volumes/A")]);
        let diff = tracker.poll(vec![PathBuf::from("/Volumes/B")]);
        assert_eq!(diff.added, vec![PathBuf::from("/Volumes/B")]);
        assert_eq!(diff.removed, vec![PathBuf::from("/Volumes/A")]);
    }

    // ---- eject-note helper --------------------------------------------

    #[test]
    fn containing_card_matches_paths_under_a_card_root() {
        let card = OnionCard {
            root: PathBuf::from("/Volumes/ONION"),
            volume_name: "ONION".into(),
            saves: Vec::new(),
        };
        let cards = vec![card];
        assert!(containing_card(
            &cards,
            Path::new("/Volumes/ONION/Saves/CurrentProfile/saves/Gambatte/RED.srm"),
        )
        .is_some());
        // Not fooled by a sibling volume sharing the name prefix.
        assert!(containing_card(&cards, Path::new("/Volumes/ONION2/RED.srm")).is_none());
        assert!(containing_card(&cards, Path::new("/home/user/RED.srm")).is_none());
    }
}
