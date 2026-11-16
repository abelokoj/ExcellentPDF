//! App data in SQLite (plan K.20, D7): `library.db`, its numbered migrations, and the one thread that writes it.
//!
//! [`Store`] opens the database in WAL mode, brings its schema up to date ([`migrations`]) and starts the writer
//! thread; every change goes through [`Store::write`] as one transaction, and reads open a connection of their own
//! on any thread ([`Store::read`]). The tables each feature keeps are in modules of their own: [`workspace`] (§11.16),
//! [`workspaces`] (W.1), [`recent`] (R.10), [`bookmarks`] (B.6), [`annotation_tags`] (A.18) and [`annotation_filters`] (A.19) so far; [`home`] finds the files again when a portable copy of the app
//! moves. A table a later feature needs (tags, collections, cards, notes) comes with that feature, in a migration of
//! its own.

pub mod annotation_filters;
pub mod annotation_tags;
pub mod bookmarks;
mod error;
pub mod home;
mod migrations;
pub mod recent;
mod store;
pub mod workspace;
pub mod workspace_file;
pub mod workspaces;

pub use error::Error;
pub use store::{Pending, Store};

/// Callers name the transactions [`Store::write`] gives them with this crate's version of `rusqlite`.
pub use rusqlite;

/// The store's result.
pub type Result<T, E = Error> = std::result::Result<T, E>;
