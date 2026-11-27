//! `.expdf` notebook open/working-PDF/save bridge (plan N.4).
//!
//! A `.expdf` file is a ZIP archive containing `document.pdf`, `notebook.json`, and `assets/`. MuPDF and
//! `ecclesia-canvas` open PDF files directly by path, so this module extracts `document.pdf` to an internal working
//! copy for display and editing, and writes any saved PDF changes back into the `.expdf` archive atomically on save.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ecclesia_core::notebook::Notebook;

use crate::settings;

/// Maps canonical/normalized `.expdf` source paths <-> extracted working `.pdf` paths.
#[derive(Default)]
struct ExpdfRegistry {
    expdf_to_work: HashMap<PathBuf, PathBuf>,
    work_to_expdf: HashMap<PathBuf, PathBuf>,
}

static REGISTRY: OnceLock<Mutex<ExpdfRegistry>> = OnceLock::new();

fn registry() -> &'static Mutex<ExpdfRegistry> {
    REGISTRY.get_or_init(|| Mutex::new(ExpdfRegistry::default()))
}

/// Whether `path` has the `.expdf` extension (case-insensitive).
pub(crate) fn is_expdf_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("expdf"))
}

/// Returns the path the page canvas and document registry should open for `path`: for `.expdf`, extracts
/// `document.pdf` into a working file; for all other document paths, returns `path` unchanged.
pub(crate) fn resolve_open_path(path: &Path) -> PathBuf {
    if !is_expdf_path(path) {
        return path.to_path_buf();
    }
    let key = normalize_key(path);
    {
        let guard = registry().lock().expect("expdf registry poisoned");
        if let Some(work) = guard.expdf_to_work.get(&key)
            && work.is_file()
        {
            return work.clone();
        }
    }
    match extract_working_pdf(path) {
        Ok(work) => {
            let mut guard = registry().lock().expect("expdf registry poisoned");
            guard.expdf_to_work.insert(key, work.clone());
            guard
                .work_to_expdf
                .insert(normalize_key(&work), path.to_path_buf());
            work
        }
        Err(err) => {
            log::warn!("expdf: cannot open {}: {err}", path.display());
            path.to_path_buf()
        }
    }
}

/// Given a working PDF path that was just saved by [`crate::editing::Editing`], returns its owning `.expdf` path if
/// any.
pub(crate) fn expdf_for_working_path(work_path: &Path) -> Option<PathBuf> {
    let guard = registry().lock().expect("expdf registry poisoned");
    guard.work_to_expdf.get(&normalize_key(work_path)).cloned()
}

/// Writes the current working PDF bytes back into `expdf_path` atomically, preserving `notebook.json` and `assets/`.
pub(crate) fn sync_working_pdf_to_expdf(work_path: &Path) -> Result<(), String> {
    let Some(expdf_path) = expdf_for_working_path(work_path) else {
        return Ok(());
    };
    let pdf_bytes = fs::read(work_path).map_err(|e| e.to_string())?;
    let mut notebook = Notebook::open(&expdf_path).map_err(|e| e.to_string())?;
    notebook
        .replace_pdf_bytes(pdf_bytes)
        .map_err(|e| e.to_string())?;
    notebook
        .save_atomic(&expdf_path)
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Refreshes the extracted working PDF from `notebook`'s `document.pdf` after a structural notebook edit (such as
/// inserting pages, expanding margins, or reordering sections) and saves `notebook` atomically to `expdf_path`.
pub(crate) fn save_and_refresh_working_pdf(
    expdf_path: &Path,
    notebook: &Notebook,
) -> Result<PathBuf, String> {
    notebook
        .save_atomic(expdf_path)
        .map_err(|e| e.to_string())?;
    let work = working_file_for(expdf_path);
    if let Some(parent) = work.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&work, notebook.pdf_bytes()).map_err(|e| e.to_string())?;
    let mut guard = registry().lock().expect("expdf registry poisoned");
    guard
        .expdf_to_work
        .insert(normalize_key(expdf_path), work.clone());
    guard
        .work_to_expdf
        .insert(normalize_key(&work), expdf_path.to_path_buf());
    Ok(work)
}

fn extract_working_pdf(expdf_path: &Path) -> Result<PathBuf, String> {
    let notebook = Notebook::open(expdf_path).map_err(|e| e.to_string())?;
    let work = working_file_for(expdf_path);
    if let Some(parent) = work.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&work, notebook.pdf_bytes()).map_err(|e| e.to_string())?;
    Ok(work)
}

fn working_file_for(expdf_path: &Path) -> PathBuf {
    let root = settings::folder()
        .unwrap_or_else(std::env::temp_dir)
        .join("expdf-work");
    let hash = fnv1a_64(expdf_path.to_string_lossy().as_bytes());
    let stem = expdf_path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "notebook".to_owned());
    root.join(format!("{stem}-{hash:016x}.pdf"))
}

fn normalize_key(path: &Path) -> PathBuf {
    PathBuf::from(path.to_string_lossy().to_lowercase())
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;
    use ecclesia_core::notebook::{Orientation, PageSize, PaperTemplateId};

    #[test]
    fn expdf_working_pdf_round_trips_and_syncs_back() {
        let dir = std::env::temp_dir().join(format!("ecclesiapdf-expdf-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("dir");
        let expdf = dir.join("lecture.expdf");

        let nb = Notebook::new_blank(
            "Lecture 1",
            PageSize::Letter,
            Orientation::Portrait,
            PaperTemplateId::Cornell,
        )
        .expect("new notebook");
        nb.save_atomic(&expdf).expect("save expdf");

        let work = resolve_open_path(&expdf);
        assert!(work.is_file());
        assert_eq!(expdf_for_working_path(&work), Some(expdf.clone()));
        sync_working_pdf_to_expdf(&work).expect("sync back");
        assert!(Notebook::open(&expdf).is_ok());

        let _ = fs::remove_dir_all(&dir);
    }
}
