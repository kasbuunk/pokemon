//! The app-side error type: everything that can go wrong in I/O or
//! loading, shown to the user through the error modal.

use std::path::PathBuf;

use thiserror::Error;

/// An app-level failure, displayed to the user via `to_string()`.
#[derive(Debug, Error)]
pub enum AppError {
    /// Writing the pre-overwrite backup failed; the save was aborted.
    #[error("could not write backup {}: {source}", path.display())]
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Backup {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Writing the picked save path failed.
    #[error("could not write {}: {source}", path.display())]
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Reading a file (e.g. a dropped path) failed.
    #[error("could not read {}: {source}", path.display())]
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    /// Renaming a shadowing save state on the SD card failed.
    #[cfg(not(target_arch = "wasm32"))]
    #[error("could not rename save state {}: {source}", path.display())]
    RenameState {
        path: PathBuf,
        source: std::io::Error,
    },
    /// The browser download could not be triggered.
    #[error("could not save in the browser: {0}")]
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    WasmSave(String),
    /// No file-dialog backend exists (Linux without zenity or an XDG
    /// desktop portal): the dialog would silently do nothing.
    #[error(
        "no file dialog backend found — install zenity or xdg-desktop-portal \
         (or drag and drop a save file onto the window)"
    )]
    NoDialogBackend,
    /// The bytes could not be parsed as a Gen 1 save.
    #[error(transparent)]
    Load(#[from] pksave::gen1::save::LoadError),
}
