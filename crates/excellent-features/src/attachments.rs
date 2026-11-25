//! Attachments (O.25, plan §14.2): a PDF's embedded files and file attachment annotations are listed, saved and
//! opened. Their names come from the file, so they are untrusted: a name like `../../startup/run.bat` or `C:\x` must
//! never write outside the folder the reader chose, and a program is never run. This module makes names safe and
//! says which files may be opened.

use std::path::{Path, PathBuf};

/// The name used when a file's own leaves nothing.
const FALLBACK: &str = "attachment";

/// The longest name written, in characters: within every desktop's limit with room for the folder.
const MAX_NAME: usize = 150;

/// Device names Windows reserves in any folder, with any extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Extensions of files that run something when opened: they are saved, never opened (plan §14.2).
const RUNNABLE: [&str; 33] = [
    "exe", "com", "bat", "cmd", "msi", "msp", "msix", "appx", "ps1", "psm1", "vbs", "vbe", "js",
    "jse", "wsf", "wsh", "hta", "scr", "pif", "cpl", "lnk", "url", "reg", "jar", "app", "command",
    "sh", "py", "pl", "rb", "dll", "inf", "html",
];

/// `name` made safe to write as a file in a folder: its last path part only, without characters any desktop
/// forbids in a name or reads as a path, without leading or trailing dots and spaces, not a reserved device name,
/// and not too long (its extension kept). Nothing left gives `attachment`.
#[must_use]
pub fn safe_file_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = last
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return FALLBACK.to_owned();
    }
    let stem = trimmed.split('.').next().unwrap_or_default();
    let named = if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
        format!("_{trimmed}")
    } else {
        trimmed.to_owned()
    };
    shorten(&named)
}

/// `name` cut to [`MAX_NAME`] characters, its extension kept.
fn shorten(name: &str) -> String {
    if name.chars().count() <= MAX_NAME {
        return name.to_owned();
    }
    let extension = Path::new(name)
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| e.chars().count() <= 10)
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let keep = MAX_NAME - extension.chars().count();
    format!("{}{extension}", name.chars().take(keep).collect::<String>())
}

/// Where attachment `name` is saved in `folder`: inside it, always, under its safe name; when `taken` says that
/// name is used, with " (2)", " (3)" and on before its extension.
#[must_use]
pub fn target_in(folder: &Path, name: &str, taken: impl Fn(&Path) -> bool) -> PathBuf {
    let safe = safe_file_name(name);
    let first = folder.join(&safe);
    if !taken(&first) {
        return first;
    }
    let path = Path::new(&safe);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(FALLBACK);
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    (2..)
        .map(|n| folder.join(format!("{stem} ({n}){extension}")))
        .find(|candidate| !taken(candidate))
        .expect("some number is free")
}

/// Whether attachment `name` may be opened in the app the system gives its kind: never one that runs something.
#[must_use]
pub fn can_open(name: &str) -> bool {
    let extension = Path::new(&safe_file_name(name))
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    extension.is_some_and(|e| !RUNNABLE.contains(&e.as_str()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_with_dot_dot_cannot_write_outside_the_chosen_folder() {
        let folder = Path::new("C:/Users/reader/Saved");
        let attacks = [
            "../../evil.txt",
            "..\\..\\Windows\\System32\\evil.dll",
            "/etc/passwd",
            "C:\\boot.ini",
            "C:evil.txt",
            "\\\\server\\share\\evil.txt",
            "..",
            ".",
            "",
            "sub/../../x",
            "evil.txt\u{0}.pdf",
            "....//....//x",
        ];
        for name in attacks {
            let target = target_in(folder, name, |_| false);
            assert_eq!(
                target.parent(),
                Some(folder),
                "{name:?} would be written at {}",
                target.display()
            );
            let file = target.file_name().and_then(|f| f.to_str()).expect("a name");
            assert!(
                !file.contains(['/', '\\', ':']) && file != ".." && file != ".",
                "{name:?} gave {file:?}"
            );
        }
    }

    #[test]
    fn names_are_kept_where_they_are_safe() {
        assert_eq!(safe_file_name("Budget 2026.xlsx"), "Budget 2026.xlsx");
        assert_eq!(safe_file_name("data/table.csv"), "table.csv");
        assert_eq!(safe_file_name("CON.txt"), "_CON.txt");
        assert_eq!(safe_file_name(" .hidden. "), "hidden");
        assert_eq!(safe_file_name("a?b*c.txt"), "abc.txt");
        let long = format!("{}.pdf", "x".repeat(400));
        let short = safe_file_name(&long);
        assert!(short.chars().count() <= MAX_NAME && short.ends_with(".pdf"));
    }

    #[test]
    fn a_used_name_gets_a_number() {
        let folder = Path::new("/saved");
        let taken = |p: &Path| p.ends_with("notes.txt") || p.ends_with("notes (2).txt");
        assert_eq!(
            target_in(folder, "notes.txt", taken),
            folder.join("notes (3).txt")
        );
    }

    #[test]
    fn programs_are_never_opened() {
        assert!(can_open("report.pdf") && can_open("photo.JPG"));
        assert!(!can_open("setup.exe") && !can_open("run.BAT") && !can_open("x.ps1"));
        assert!(
            !can_open("no extension"),
            "an unknown kind is saved, not opened"
        );
        assert!(!can_open("invoice.pdf.exe"));
    }
}
