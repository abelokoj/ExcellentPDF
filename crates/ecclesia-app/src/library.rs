//! The app's library database (K.20): `library.db`, next to `settings.toml`. Opened once, before the window, by the
//! desktop binary; without it (the tests, a failure) the app runs with nothing saved.

use std::path::Path;
use std::sync::OnceLock;

use ecclesia_store::Store;

/// The database's file name, in the app's folder.
const FILE_NAME: &str = "library.db";

static LIBRARY: OnceLock<Store> = OnceLock::new();

/// Opens the library. A database that can't be opened is logged and left alone: the app works without it, and a
/// newer app's database is never written over.
pub(crate) fn open() {
    let Some(path) = crate::settings::folder().map(|folder| folder.join(FILE_NAME)) else {
        log::warn!("library: the system has no data folder for the app");
        return;
    };
    match Store::open(&path) {
        Ok(store) => {
            // A portable copy finds the files that moved with it before anything reads them (R.10).
            if let Some(home) = crate::settings::portable_folder() {
                let moved =
                    store.write(move |tx| ecclesia_store::home::move_home(tx, &home, Path::exists));
                if let Err(err) = moved.wait() {
                    log::warn!("library: can't follow the app's move: {err}");
                }
            }
            let _ = store.write(|tx| {
                tx.execute_batch(ecclesia_store::study::SCHEMA_SQL)?;
                Ok(())
            });
            let _ = LIBRARY.set(store);
        }
        Err(err) => log::warn!("library: can't open {}: {err}", path.display()),
    }
}

/// The library, once [`open`] has opened it.
pub(crate) fn store() -> Option<&'static Store> {
    LIBRARY.get()
}
