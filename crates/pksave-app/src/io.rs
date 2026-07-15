//! Async open/save flows, decoupled from the frame loop via a
//! `std::sync::mpsc` channel polled in `App::update`.
//!
//! Native: the rfd async dialog is driven on a background thread with a
//! tiny `pollster` executor; the picked path is written through
//! [`write_picked`] (backup on overwrite, fsync before reporting
//! success). Wasm: the open dialog runs on the browser event loop via
//! `wasm_bindgen_futures::spawn_local`; saving triggers a direct browser
//! download (Blob → object URL → anchor click), which cannot be
//! cancelled, so a reported save is an actual download.

use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

use crate::error::AppError;

/// Result of a background I/O operation, delivered to the UI thread.
pub enum IoEvent {
    /// A file was picked and read.
    Opened {
        file_name: String,
        bytes: Vec<u8>,
        /// Where it came from; `None` on wasm.
        path: Option<PathBuf>,
    },
    /// The bytes were written (native) or a download was triggered (wasm).
    Saved {
        /// `true` for "Save" (the edited buffer becomes the new baseline),
        /// `false` for "save a copy of the original".
        primary: bool,
        /// Where it was written; `None` on wasm.
        path: Option<PathBuf>,
        /// The pre-overwrite backup written next to `path`, if any.
        backup: Option<PathBuf>,
        /// Display name of what was saved (the download name on wasm).
        file_name: String,
    },
    /// The user dismissed a dialog.
    Cancelled,
    /// Something went wrong; shown to the user.
    Error(AppError),
}

/// Everything a save flow needs, captured up front so the future owns it.
pub struct SaveRequest {
    pub default_file_name: String,
    pub bytes: Vec<u8>,
    /// See [`IoEvent::Saved::primary`].
    pub primary: bool,
    /// Native only: if the user picks this path (any spelling of it), a
    /// timestamped `.bak-YYYYMMDD-HHMMSS` copy is made before overwriting.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub original_path: Option<PathBuf>,
}

#[cfg(not(target_arch = "wasm32"))]
fn spawn<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = ()>,
{
    std::thread::spawn(move || pollster::block_on(f()));
}

#[cfg(target_arch = "wasm32")]
fn spawn<F, Fut>(f: F)
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = ()> + 'static,
{
    wasm_bindgen_futures::spawn_local(f());
}

fn file_dialog() -> rfd::AsyncFileDialog {
    rfd::AsyncFileDialog::new().add_filter("Gen 1 save", &["sav", "srm", "bak"])
}

/// Show an open dialog and read the picked file.
pub fn spawn_open(tx: Sender<IoEvent>, ctx: egui::Context) {
    spawn(move || async move {
        let event = match file_dialog().pick_file().await {
            Some(handle) => {
                let bytes = handle.read().await;
                IoEvent::Opened {
                    file_name: handle.file_name(),
                    bytes,
                    path: handle_path(&handle),
                }
            }
            None => IoEvent::Cancelled,
        };
        let _ = tx.send(event);
        ctx.request_repaint();
    });
}

/// Show a save dialog and write the bytes (with a backup of the
/// originally opened file on native); trigger a download on wasm.
pub fn spawn_save(tx: Sender<IoEvent>, ctx: egui::Context, request: SaveRequest) {
    spawn(move || async move {
        let event = save_flow(request).await;
        let _ = tx.send(event);
        ctx.request_repaint();
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn handle_path(handle: &rfd::FileHandle) -> Option<PathBuf> {
    Some(handle.path().to_path_buf())
}

#[cfg(target_arch = "wasm32")]
fn handle_path(_handle: &rfd::FileHandle) -> Option<PathBuf> {
    None
}

#[cfg(not(target_arch = "wasm32"))]
async fn save_flow(request: SaveRequest) -> IoEvent {
    let mut dialog = file_dialog().set_file_name(&request.default_file_name);
    if let Some(dir) = surviving_parent(request.original_path.as_deref()) {
        dialog = dialog.set_directory(dir);
    }
    let Some(handle) = dialog.save_file().await else {
        return IoEvent::Cancelled;
    };
    write_picked(
        handle.path(),
        &request.bytes,
        request.original_path.as_deref(),
        request.primary,
    )
}

/// The post-dialog body of the native save flow, synchronous and
/// testable: back up the original when overwriting it, then write and
/// fsync the picked path.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_picked(
    path: &std::path::Path,
    bytes: &[u8],
    original_path: Option<&std::path::Path>,
    primary: bool,
) -> IoEvent {
    use std::io::Write as _;

    let mut backup = None;
    if original_path.is_some_and(|orig| is_same_file(orig, path)) && path.exists() {
        match create_backup(path, now_secs()) {
            Ok(b) => backup = Some(b),
            Err((backup_path, source)) => {
                return IoEvent::Error(AppError::Backup {
                    path: backup_path,
                    source,
                });
            }
        }
    }
    let write = || -> std::io::Result<()> {
        let mut file = std::fs::File::create(path)?;
        file.write_all(bytes)?;
        // Flushed to disk before we tell the user their save is safe.
        file.sync_all()
    };
    match write() {
        Ok(()) => IoEvent::Saved {
            primary,
            path: Some(path.to_path_buf()),
            backup,
            file_name: path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string()),
        },
        Err(source) => IoEvent::Error(AppError::Write {
            path: path.to_path_buf(),
            source,
        }),
    }
}

/// The directory to preset in the save dialog: the originally opened
/// file's parent, but only while it still exists. If the file's volume
/// vanished (SD card pulled mid-edit) the buffer is still in memory and
/// the save dialog falls back to its default location instead of
/// pointing at a dead mount.
#[cfg(not(target_arch = "wasm32"))]
fn surviving_parent(original: Option<&std::path::Path>) -> Option<&std::path::Path> {
    original?.parent().filter(|p| p.is_dir())
}

/// Whether two paths name the same file, resolving symlinks and
/// alternate spellings (`./a.sav` vs `a.sav`). Falls back to plain
/// component equality when either side cannot be canonicalized.
#[cfg(not(target_arch = "wasm32"))]
fn is_same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(ca), Ok(cb)) => ca == cb,
        _ => a == b,
    }
}

/// Copy `path` to a fresh `<path>.bak-<timestamp>` next to it, appending
/// `-2`, `-3`, … when the name is taken (multiple saves within one
/// second must not truncate an earlier backup). Uses create-new
/// semantics so an existing file is never overwritten. On failure
/// returns the attempted backup path with the error.
#[cfg(not(target_arch = "wasm32"))]
fn create_backup(
    path: &std::path::Path,
    secs_since_epoch: u64,
) -> Result<PathBuf, (PathBuf, std::io::Error)> {
    use std::io::Write as _;

    let contents = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return Err((path.to_path_buf(), e)),
    };
    let base = backup_path_for(path, secs_since_epoch);
    let mut candidate = base.clone();
    let mut counter = 2u32;
    loop {
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(mut file) => {
                let result = file.write_all(&contents).and_then(|()| file.sync_all());
                return match result {
                    Ok(()) => Ok(candidate),
                    Err(e) => Err((candidate, e)),
                };
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists && counter <= 1000 => {
                candidate = base.with_file_name(format!(
                    "{}-{counter}",
                    base.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "save.bak".to_owned())
                ));
                counter += 1;
            }
            Err(e) => return Err((candidate, e)),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(target_arch = "wasm32")]
async fn save_flow(request: SaveRequest) -> IoEvent {
    // No file picker: a programmatic anchor click downloads
    // unconditionally, so there is no cancel path and a `Saved` event is
    // honest (rfd's wasm `FileHandle::write` always returns Ok, even
    // when the user dismisses the overlay).
    match trigger_download(&request.bytes, &request.default_file_name) {
        Ok(()) => IoEvent::Saved {
            primary: request.primary,
            path: None,
            backup: None,
            file_name: request.default_file_name,
        },
        Err(message) => IoEvent::Error(AppError::WasmSave(message)),
    }
}

/// Trigger a browser download of `bytes` as `file_name`: Blob → object
/// URL → temporary `<a download>` click → revoke.
#[cfg(target_arch = "wasm32")]
fn trigger_download(bytes: &[u8], file_name: &str) -> Result<(), String> {
    use eframe::wasm_bindgen::JsCast as _;

    let js_err = |e: eframe::wasm_bindgen::JsValue| format!("{e:?}");

    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array);
    let options = web_sys::BlobPropertyBag::new();
    options.set_type("application/octet-stream");
    let blob =
        web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &options).map_err(js_err)?;
    let url = web_sys::Url::create_object_url_with_blob(&blob).map_err(js_err)?;

    let document = web_sys::window()
        .and_then(|w| w.document())
        .ok_or_else(|| "no document".to_owned())?;
    let anchor = document
        .create_element("a")
        .map_err(js_err)?
        .dyn_into::<web_sys::HtmlAnchorElement>()
        .map_err(|_| "could not create anchor element".to_owned())?;
    anchor.set_href(&url);
    anchor.set_download(file_name);
    anchor.style().set_property("display", "none").ok();

    let body = document.body().ok_or_else(|| "no body".to_owned())?;
    body.append_child(&anchor).map_err(js_err)?;
    anchor.click();
    let _ = body.remove_child(&anchor);
    let _ = web_sys::Url::revoke_object_url(&url);
    Ok(())
}

/// `<original>.bak-YYYYMMDD-HHMMSS`, alongside the original.
#[cfg(not(target_arch = "wasm32"))]
fn backup_path_for(path: &std::path::Path, secs_since_epoch: u64) -> PathBuf {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save".to_owned());
    path.with_file_name(format!("{name}.bak-{}", backup_timestamp(secs_since_epoch)))
}

/// `YYYYMMDD-HHMMSS` (UTC) from seconds since the Unix epoch. Pure so it
/// can be unit-tested; no chrono needed for seconds precision.
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub fn backup_timestamp(secs_since_epoch: u64) -> String {
    let days = (secs_since_epoch / 86_400) as i64;
    let rem = secs_since_epoch % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}-{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days-since-epoch to (year, month, day), Howard Hinnant's
/// `civil_from_days` algorithm (proleptic Gregorian).
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn epoch_is_1970() {
        assert_eq!(backup_timestamp(0), "19700101-000000");
    }

    #[test]
    fn known_moments() {
        // 2000-03-01T00:00:00Z (leap-year boundary).
        assert_eq!(backup_timestamp(951_868_800), "20000301-000000");
        // 2026-07-15T12:34:56Z.
        assert_eq!(backup_timestamp(1_784_118_896), "20260715-123456");
        // 2024-02-29T23:59:59Z (leap day).
        assert_eq!(backup_timestamp(1_709_251_199), "20240229-235959");
    }

    #[test]
    fn time_of_day_components() {
        assert_eq!(backup_timestamp(86_399), "19700101-235959");
        assert_eq!(backup_timestamp(86_400), "19700102-000000");
        assert_eq!(backup_timestamp(3_661), "19700101-010101");
    }

    #[test]
    fn backup_name_appends_timestamp_suffix() {
        let path = std::path::Path::new("/saves/poke.sav");
        assert_eq!(
            backup_path_for(path, 0),
            PathBuf::from("/saves/poke.sav.bak-19700101-000000")
        );
    }

    #[test]
    fn write_picked_to_new_path_makes_no_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("new.sav");
        let event = write_picked(&target, b"abc", None, true);
        match event {
            IoEvent::Saved {
                primary,
                path,
                backup,
                file_name,
            } => {
                assert!(primary);
                assert_eq!(path.as_deref(), Some(target.as_path()));
                assert_eq!(backup, None);
                assert_eq!(file_name, "new.sav");
            }
            _ => panic!("expected Saved"),
        }
        assert_eq!(std::fs::read(&target).expect("read back"), b"abc");
        // Only the save itself exists in the directory.
        assert_eq!(std::fs::read_dir(dir.path()).expect("readdir").count(), 1);
    }

    #[test]
    fn write_picked_backs_up_exactly_once_with_presave_bytes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("poke.sav");
        std::fs::write(&target, b"ORIGINAL").expect("seed");
        let event = write_picked(&target, b"EDITED", Some(&target), true);
        let backup = match event {
            IoEvent::Saved { backup, .. } => backup.expect("backup path"),
            _ => panic!("expected Saved"),
        };
        assert_eq!(std::fs::read(&target).expect("target"), b"EDITED");
        assert_eq!(std::fs::read(&backup).expect("backup"), b"ORIGINAL");
        // Exactly two files: the save and one backup.
        assert_eq!(std::fs::read_dir(dir.path()).expect("readdir").count(), 2);
    }

    #[test]
    fn write_picked_backs_up_alternate_spelling_of_same_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("poke.sav");
        std::fs::write(&target, b"ORIGINAL").expect("seed");
        // The "picked" path spells the same file with a `.` component.
        let alt = dir.path().join(".").join("poke.sav");
        let event = write_picked(&alt, b"EDITED", Some(&target), true);
        match event {
            IoEvent::Saved { backup, .. } => assert!(backup.is_some(), "backup expected"),
            _ => panic!("expected Saved"),
        }
    }

    #[test]
    fn backup_collision_in_same_second_appends_counter() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("poke.sav");
        std::fs::write(&target, b"FIRST").expect("seed");
        let first = create_backup(&target, 42).expect("first backup");
        std::fs::write(&target, b"SECOND").expect("reseed");
        let second = create_backup(&target, 42).expect("second backup");
        assert_ne!(first, second);
        assert!(
            second
                .to_string_lossy()
                .ends_with(&format!("{}-2", backup_timestamp(42))),
            "counter suffix: {}",
            second.display()
        );
        // The first backup is untouched.
        assert_eq!(std::fs::read(&first).expect("first bytes"), b"FIRST");
        assert_eq!(std::fs::read(&second).expect("second bytes"), b"SECOND");
        let third = create_backup(&target, 42).expect("third backup");
        assert!(third.to_string_lossy().ends_with("-3"));
    }

    #[test]
    fn write_failure_reports_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("missing-subdir").join("poke.sav");
        match write_picked(&target, b"abc", None, true) {
            IoEvent::Error(AppError::Write { path, .. }) => assert_eq!(path, target),
            _ => panic!("expected write error"),
        }
    }

    #[test]
    fn surviving_parent_is_the_existing_dir_of_the_original() {
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("poke.sav");
        assert_eq!(
            surviving_parent(Some(&original)),
            Some(dir.path()),
            "parent still mounted: preset the dialog there"
        );
    }

    #[test]
    fn surviving_parent_is_none_when_the_volume_vanished() {
        // Simulates a pulled SD card: the original path's parent is gone.
        let dir = tempfile::tempdir().expect("tempdir");
        let original = dir.path().join("card").join("poke.sav");
        assert_eq!(surviving_parent(Some(&original)), None);
        assert_eq!(surviving_parent(None), None);
    }

    #[test]
    fn is_same_file_falls_back_to_raw_equality() {
        // Neither exists, so canonicalize fails on both sides.
        assert!(is_same_file(
            std::path::Path::new("/nonexistent/a.sav"),
            std::path::Path::new("/nonexistent/a.sav")
        ));
        assert!(!is_same_file(
            std::path::Path::new("/nonexistent/a.sav"),
            std::path::Path::new("/nonexistent/b.sav")
        ));
    }
}
