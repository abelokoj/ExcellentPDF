//! Optional packs (K.21, plan §2.1 rule 2, D8): fonts, dictionaries, voices, OCR and AI models live outside the core
//! installer and come as signed, removable packs. This crate finds out what is offered (a signed [`index`]),
//! downloads a pack with resume ([`download`]), checks it ([`file`]: the pack's own signature, and the size and
//! hash the index gives), installs it atomically ([`install`]) and removes it without a trace.
//!
//! A pack is installed into its own folder under the packs root; features ask [`Manager::is_installed`] and offer the
//! download when it isn't.

pub mod download;
pub mod error;
pub mod file;
pub mod index;
pub mod install;
mod manager;

pub use error::Error;
pub use manager::{Installed, Manager};
