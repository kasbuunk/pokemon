//! Async open/save flows, decoupled from the frame loop via a
//! `std::sync::mpsc` channel polled in `App::update`.
//!
//! Native: the rfd async dialog is driven on a background thread with a
//! tiny `pollster` executor. Wasm: the same futures run on the browser
//! event loop through `wasm_bindgen_futures::spawn_local`.

use std::future::Future;
use std::path::PathBuf;
use std::sync::mpsc::Sender;

/// Result of a background I/O operation, delivered to the UI thread.
pub enum IoEvent {
    /// A file was picked and read.
    Opened {
        file_name: String,
        bytes: Vec<u8>,
        /// Where it came from; `None` on wasm.
        path: Option<PathBuf>,
    },
    /// A save-file dialog completed and the bytes were written.
    Saved {
        /// `true` for "Save" (the edited buffer becomes the new baseline),
        /// `false` for "save a copy of the original".
        primary: bool,
        /// Where it was written; `None` on wasm.
        path: Option<PathBuf>,
    },
    /// The user dismissed a dialog.
    Cancelled,
    /// Something went wrong; shown to the user.
    Error(String),
}

/// Everything a save flow needs, captured up front so the future owns it.
pub struct SaveRequest {
    pub default_file_name: String,
    pub bytes: Vec<u8>,
    /// See [`IoEvent::Saved::primary`].
    pub primary: bool,
    /// Native only: if the user picks exactly this path, a timestamped
    /// `.bak-YYYYMMDD-HHMMSS` copy is made before overwriting.
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
    rfd::AsyncFileDialog::new().add_filter("Gen 1 save", &["sav", "srm", "gb", "bak"])
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
/// originally opened file on native; a download on wasm).
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
    let Some(handle) = file_dialog()
        .set_file_name(&request.default_file_name)
        .save_file()
        .await
    else {
        return IoEvent::Cancelled;
    };
    let path = handle.path().to_path_buf();

    if request.original_path.as_deref() == Some(path.as_path()) && path.exists() {
        let backup = backup_path_for(&path);
        if let Err(e) = std::fs::copy(&path, &backup) {
            return IoEvent::Error(format!("could not write backup {}: {e}", backup.display()));
        }
    }
    match std::fs::write(&path, &request.bytes) {
        Ok(()) => IoEvent::Saved {
            primary: request.primary,
            path: Some(path),
        },
        Err(e) => IoEvent::Error(format!("could not write {}: {e}", path.display())),
    }
}

#[cfg(target_arch = "wasm32")]
async fn save_flow(request: SaveRequest) -> IoEvent {
    let Some(handle) = file_dialog()
        .set_file_name(&request.default_file_name)
        .save_file()
        .await
    else {
        return IoEvent::Cancelled;
    };
    match handle.write(&request.bytes).await {
        Ok(()) => IoEvent::Saved {
            primary: request.primary,
            path: None,
        },
        Err(e) => IoEvent::Error(format!("could not save file: {e}")),
    }
}

/// `<original>.bak-YYYYMMDD-HHMMSS`, alongside the original.
#[cfg(not(target_arch = "wasm32"))]
fn backup_path_for(path: &std::path::Path) -> PathBuf {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "save".to_owned());
    path.with_file_name(format!("{name}.bak-{}", backup_timestamp(secs)))
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
}
