//! The portable app's folder (plan R.10, "portable-mode moves"). A portable copy keeps its library next to itself,
//! often on a USB stick with the reader's documents, so when it runs from somewhere else (another folder, or the stick
//! under another drive letter) the files it knew moved with it: [`move_home`] finds them again.

use std::path::{Component, Path, PathBuf, Prefix};

use rusqlite::{OptionalExtension, Transaction, params};

use crate::Result;

/// Notes that the app runs from `home` now. If it ran from elsewhere before, every file that moved with it gets its
/// new path; `exists` tells whether a file is at a path (see [`moved`]).
///
/// shortcut: a file whose new path is already known under its own row keeps its old row, so that row's history
/// (tabs, recent) stays with the old path; merge the rows if readers move between two copies of one library.
///
/// # Errors
///
/// SQLite failed.
pub fn move_home(tx: &Transaction<'_>, home: &Path, exists: impl Fn(&Path) -> bool) -> Result<()> {
    let old: Option<String> = tx
        .query_row("SELECT value FROM meta WHERE key = 'home'", [], |row| {
            row.get(0)
        })
        .optional()?;
    if let Some(old) = old.map(PathBuf::from).filter(|old| old != home) {
        let files: Vec<(i64, String)> = tx
            .prepare("SELECT id, path FROM file")?
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        let mut rename = tx.prepare("UPDATE OR IGNORE file SET path = ?2 WHERE id = ?1")?;
        for (id, path) in files {
            if let Some(new) = moved(Path::new(&path), &old, home, &exists) {
                rename.execute(params![id, new.to_string_lossy()])?;
            }
        }
    }
    tx.execute(
        "INSERT INTO meta (key, value) VALUES ('home', ?1) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [home.to_string_lossy()],
    )?;
    Ok(())
}

/// Where `path` is now that the app's folder moved from `old` to `new`, if it moved with it: a file inside the old
/// folder is inside the new one; a file elsewhere on the old folder's drive is on the new folder's drive when the
/// drive letter changed, the file isn't at its old path and is at the new one (the stick came back under another
/// letter). `None`: it stays where it was.
pub fn moved(
    path: &Path,
    old: &Path,
    new: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<PathBuf> {
    if let Ok(rest) = path.strip_prefix(old) {
        return Some(new.join(rest));
    }
    let (from, to) = (drive(old)?, drive(new)?);
    if from == to || drive(path)? != from || exists(path) {
        return None;
    }
    let mut there = PathBuf::from(format!("{}:", char::from(to)));
    there.extend(path.components().skip(1));
    exists(&there).then_some(there)
}

/// The drive letter `path` starts with, in upper case (Windows).
fn drive(path: &Path) -> Option<u8> {
    match path.components().next()? {
        Component::Prefix(prefix) => match prefix.kind() {
            Prefix::Disk(letter) | Prefix::VerbatimDisk(letter) => {
                Some(letter.to_ascii_uppercase())
            }
            _ => None,
        },
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn files_inside_the_portable_folder_move_with_it() {
        let moved = moved(
            Path::new("/media/old/ExcellentPDF/papers/a.pdf"),
            Path::new("/media/old/ExcellentPDF"),
            Path::new("/media/new/ExcellentPDF"),
            |_| false,
        );
        assert_eq!(
            moved,
            Some(PathBuf::from("/media/new/ExcellentPDF/papers/a.pdf"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn files_beside_it_follow_a_new_drive_letter_only_when_found_there() {
        let (old, new) = (
            Path::new(r"E:\Apps\ExcellentPDF"),
            Path::new(r"F:\Apps\ExcellentPDF"),
        );
        let there = |path: &Path| path == Path::new(r"F:\Docs\a.pdf");
        assert_eq!(
            moved(Path::new(r"E:\Docs\a.pdf"), old, new, there),
            Some(PathBuf::from(r"F:\Docs\a.pdf"))
        );
        // Not on the new drive, or on another drive: it stays.
        assert_eq!(moved(Path::new(r"E:\Docs\b.pdf"), old, new, there), None);
        assert_eq!(moved(Path::new(r"C:\Docs\a.pdf"), old, new, |_| true), None);
        // Still at its old path: it stays.
        assert_eq!(moved(Path::new(r"E:\Docs\a.pdf"), old, new, |_| true), None);
    }

    #[test]
    fn the_library_finds_its_files_after_the_app_moves() {
        let mut conn = rusqlite::Connection::open_in_memory().expect("open");
        crate::migrations::migrate(&mut conn).expect("migrate");
        let at = |conn: &rusqlite::Connection| -> String {
            conn.query_row("SELECT path FROM file", [], |row| row.get(0))
                .expect("a file")
        };
        let tx = conn.transaction().expect("transaction");
        move_home(&tx, Path::new("/stick/app"), |_| false).expect("first run");
        crate::workspace::file_id(&tx, Path::new("/stick/app/papers/a.pdf")).expect("a file");
        tx.commit().expect("commit");
        let tx = conn.transaction().expect("transaction");
        move_home(&tx, Path::new("/other/app"), |_| false).expect("moved");
        tx.commit().expect("commit");
        assert_eq!(
            PathBuf::from(at(&conn)),
            Path::new("/other/app").join("papers/a.pdf")
        );
    }
}
