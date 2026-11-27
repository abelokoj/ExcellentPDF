//! System-installed font discovery and lookup for reflowable documents (EPUB, FB2, MOBI, HTML, Markdown).
//!
//! MuPDF's HTML/EPUB engine calls `fz_load_system_font(ctx, family, bold, italic, 0)` whenever CSS selects a
//! font family that is not embedded in the document. This module scans the OS font catalog once per process,
//! exposes the sorted list of installed font family names for the UI font picker, and resolves MuPDF's C
//! callback back to a font file path and TrueType Collection (`.ttc`) face index.

use core::ffi::{CStr, c_char, c_int};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// One installed font face on disk.
#[derive(Debug, Clone)]
struct FontFace {
    /// UTF-8 file path to `.ttf`, `.otf`, or `.ttc`.
    path: String,
    /// Face index inside a `.ttc` collection (`0` for single-face `.ttf`/`.otf`).
    face_index: i32,
    /// Whether this face is bold.
    bold: bool,
    /// Whether this face is italic/oblique.
    italic: bool,
    /// Whether this face is an unadorned Regular/Bold/Italic weight (preferred over Light/Black/Narrow).
    plain_weight: bool,
}

/// Process-wide cache of installed system font families and their faces.
struct Catalog {
    /// Display family names sorted case-insensitively and deduplicated.
    families: Vec<String>,
    /// Lowercased family name -> candidate faces.
    by_family: HashMap<String, Vec<FontFace>>,
}

static CATALOG: OnceLock<Catalog> = OnceLock::new();

/// Returns the sorted, deduplicated list of system-installed font family names available for reflowable
/// documents (EPUB, FB2, MOBI, HTML, Markdown).
pub fn system_font_families() -> &'static [String] {
    &catalog().families
}

fn catalog() -> &'static Catalog {
    CATALOG.get_or_init(build_catalog)
}

/// Returns whether `name` is a generic CSS font family keyword that MuPDF should resolve via its built-in
/// fallback fonts rather than overriding with an arbitrary system font.
fn is_css_generic(lower: &str) -> bool {
    matches!(
        lower,
        "serif"
            | "sans-serif"
            | "monospace"
            | "cursive"
            | "fantasy"
            | "system-ui"
            | "ui-serif"
            | "ui-sans-serif"
            | "ui-monospace"
            | "ui-rounded"
            | "emoji"
            | "math"
            | "fangsong"
    )
}

/// Resolves `(family, bold, italic)` to a `(path, face_index)` from `catalog`.
fn resolve_in<'a>(
    catalog: &'a Catalog,
    raw_name: &str,
    bold: bool,
    italic: bool,
) -> Option<(&'a str, i32)> {
    let trimmed = raw_name
        .trim()
        .trim_matches(|c| c == '"' || c == '\'')
        .trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    if is_css_generic(&lower) {
        return None;
    }
    let faces = catalog.by_family.get(&lower)?;
    let best = faces.iter().max_by_key(|face| {
        let exact_bold = i32::from(face.bold == bold);
        let exact_italic = i32::from(face.italic == italic);
        let regular_fallback = i32::from(!face.bold && !face.italic);
        let plain = i32::from(face.plain_weight);
        // Priority: exact bold+italic > exact bold > exact italic > plain weight > regular face.
        (exact_bold + exact_italic) * 16 + exact_bold * 4 + plain * 2 + regular_fallback
    })?;
    Some((best.path.as_str(), best.face_index))
}

/// C callback registered with `ex_set_system_font_lookup`.
///
/// # Safety
///
/// `name` must be a valid NUL-terminated UTF-8/ASCII string, `path_buf` must be writable for `path_cap` bytes,
/// and `face_index` must be a valid writable pointer (all guaranteed by `ex_load_system_font_cb` in `shim.c`).
pub(crate) unsafe extern "C" fn lookup(
    name: *const c_char,
    bold: c_int,
    italic: c_int,
    path_buf: *mut c_char,
    path_cap: c_int,
    face_index: *mut c_int,
) -> c_int {
    if name.is_null() || path_buf.is_null() || face_index.is_null() || path_cap <= 1 {
        return 0;
    }
    // SAFETY: MuPDF passes a NUL-terminated family name string.
    let Ok(name_str) = unsafe { CStr::from_ptr(name) }.to_str() else {
        return 0;
    };
    let Some((path, index)) = resolve_in(catalog(), name_str, bold != 0, italic != 0) else {
        return 0;
    };
    let bytes = path.as_bytes();
    let cap = usize::try_from(path_cap).unwrap_or(0);
    if bytes.is_empty() || bytes.len() + 1 > cap || bytes.contains(&0) {
        return 0;
    }
    // SAFETY: `path_buf` has capacity `cap >= bytes.len() + 1` and does not overlap `bytes`.
    unsafe {
        core::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), path_buf, bytes.len());
        *path_buf.add(bytes.len()) = 0;
        face_index.write(index);
    }
    1
}

/// Strips trailing Windows font format tags like `" (TrueType)"` or `" (OpenType)"`.
fn strip_format_tag(raw: &str) -> &str {
    let trimmed = raw.trim();
    if let Some(open) = trimmed.rfind(" (")
        && trimmed.ends_with(')')
    {
        let tag = &trimmed[open + 2..trimmed.len() - 1];
        if tag.eq_ignore_ascii_case("TrueType")
            || tag.eq_ignore_ascii_case("OpenType")
            || tag.eq_ignore_ascii_case("All res")
            || tag.eq_ignore_ascii_case("VGA res")
            || tag.to_ascii_lowercase().contains("truetype")
            || tag.to_ascii_lowercase().contains("opentype")
        {
            return trimmed[..open].trim();
        }
    }
    trimmed
}

/// Parses one face display name (e.g. `"Georgia Bold Italic"`) into `(base_family, bold, italic, plain_weight)`.
fn parse_face_name(name: &str) -> (&str, bool, bool, bool) {
    let mut base = name.trim();
    let mut bold = false;
    let mut italic = false;
    let mut plain_weight = true;

    loop {
        let lower = base.to_ascii_lowercase();
        let stripped = if let Some(rest) = strip_suffix_ci(base, &lower, " bold italic") {
            bold = true;
            italic = true;
            Some(rest)
        } else if let Some(rest) = strip_suffix_ci(base, &lower, " bold oblique") {
            bold = true;
            italic = true;
            Some(rest)
        } else if let Some(rest) = strip_suffix_ci(base, &lower, " italic") {
            italic = true;
            Some(rest)
        } else if let Some(rest) = strip_suffix_ci(base, &lower, " oblique") {
            italic = true;
            Some(rest)
        } else if let Some(rest) = strip_suffix_ci(base, &lower, " bold") {
            bold = true;
            Some(rest)
        } else if let Some(rest) = strip_suffix_ci(base, &lower, " regular") {
            Some(rest)
        } else {
            strip_suffix_ci(base, &lower, " normal")
        };
        match stripped {
            Some(rest) if !rest.is_empty() => base = rest.trim_end(),
            _ => break,
        }
    }

    let lower = base.to_ascii_lowercase();
    for suffix in [
        " light",
        " semilight",
        " extralight",
        " thin",
        " medium",
        " semibold",
        " demibold",
        " extrabold",
        " black",
        " heavy",
        " condensed",
        " narrow",
    ] {
        if lower.ends_with(suffix) {
            plain_weight = false;
            break;
        }
    }

    (base, bold, italic, plain_weight)
}

fn strip_suffix_ci<'a>(orig: &'a str, lower: &str, suffix: &str) -> Option<&'a str> {
    if lower.ends_with(suffix) && orig.len() >= suffix.len() {
        Some(&orig[..orig.len() - suffix.len()])
    } else {
        None
    }
}

/// Adds a registry/file entry into `display_names` and `by_family`.
fn register_entry(
    display_names: &mut BTreeMap<String, String>,
    by_family: &mut HashMap<String, Vec<FontFace>>,
    raw_name: &str,
    path: &Path,
) {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
        return;
    };
    if !(ext.eq_ignore_ascii_case("ttf")
        || ext.eq_ignore_ascii_case("otf")
        || ext.eq_ignore_ascii_case("ttc"))
    {
        return;
    }
    let Some(path_str) = path.to_str() else {
        return;
    };
    if !path.is_file() {
        return;
    }

    let cleaned = strip_format_tag(raw_name);
    if cleaned.is_empty() {
        return;
    }

    for (idx, part) in cleaned.split(" & ").enumerate() {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (family, bold, italic, plain_weight) = parse_face_name(part);
        if family.is_empty() {
            continue;
        }
        let key = family.to_ascii_lowercase();
        display_names
            .entry(key.clone())
            .or_insert_with(|| family.to_owned());
        by_family.entry(key).or_default().push(FontFace {
            path: path_str.to_owned(),
            face_index: i32::try_from(idx).unwrap_or(0),
            bold,
            italic,
            plain_weight,
        });
    }
}

fn build_catalog() -> Catalog {
    let mut display_names: BTreeMap<String, String> = BTreeMap::new();
    let mut by_family: HashMap<String, Vec<FontFace>> = HashMap::new();

    #[cfg(windows)]
    scan_windows_registry(&mut display_names, &mut by_family);

    #[cfg(not(windows))]
    scan_unix_font_dirs(&mut display_names, &mut by_family);

    let families = display_names.into_values().collect();
    Catalog {
        families,
        by_family,
    }
}

#[cfg(windows)]
fn scan_windows_registry(
    display_names: &mut BTreeMap<String, String>,
    by_family: &mut HashMap<String, Vec<FontFace>>,
) {
    use windows::Win32::Foundation::{ERROR_NO_MORE_ITEMS, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, REG_EXPAND_SZ, REG_SZ,
        REG_VALUE_TYPE, RegCloseKey, RegEnumValueW, RegOpenKeyExW,
    };
    use windows::core::PCWSTR;

    let windir = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let system_fonts_dir = windir.join("Fonts");

    let subkey: Vec<u16> = "SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion\\Fonts\0"
        .encode_utf16()
        .collect();

    for root in [HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER] {
        let mut hkey = HKEY::default();
        // SAFETY: `subkey` is NUL-terminated UTF-16 and `hkey` is a valid out-pointer.
        let opened =
            unsafe { RegOpenKeyExW(root, PCWSTR(subkey.as_ptr()), Some(0), KEY_READ, &mut hkey) };
        if opened != ERROR_SUCCESS {
            continue;
        }

        let mut index = 0u32;
        let mut name_buf = vec![0u16; 512];
        let mut data_buf = vec![0u8; 2048];
        loop {
            let mut name_len = u32::try_from(name_buf.len()).unwrap_or(512);
            let mut data_len = u32::try_from(data_buf.len()).unwrap_or(2048);
            let mut value_type = REG_VALUE_TYPE::default();
            // SAFETY: buffers are valid for `name_len` UTF-16 units and `data_len` bytes respectively.
            let status = unsafe {
                RegEnumValueW(
                    hkey,
                    index,
                    Some(windows::core::PWSTR(name_buf.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut value_type.0),
                    Some(data_buf.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            index = index.saturating_add(1);
            if status == ERROR_NO_MORE_ITEMS {
                break;
            }
            if status != ERROR_SUCCESS {
                continue;
            }
            if value_type != REG_SZ && value_type != REG_EXPAND_SZ {
                continue;
            }
            let raw_name = String::from_utf16_lossy(&name_buf[..name_len as usize]);
            let data_slice = &data_buf[..data_len as usize];
            let words: Vec<u16> = data_slice
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .take_while(|&w| w != 0)
                .collect();
            let raw_path = String::from_utf16_lossy(&words);
            let raw_path = raw_path.trim();
            if raw_path.is_empty() {
                continue;
            }
            let candidate = PathBuf::from(raw_path);
            let resolved = if candidate.is_absolute() {
                candidate
            } else {
                system_fonts_dir.join(candidate)
            };
            register_entry(display_names, by_family, &raw_name, &resolved);
        }

        // SAFETY: `hkey` was opened by `RegOpenKeyExW` above and is closed once here.
        let _ = unsafe { RegCloseKey(hkey) };
    }
}

#[cfg(not(windows))]
fn scan_unix_font_dirs(
    display_names: &mut BTreeMap<String, String>,
    by_family: &mut HashMap<String, Vec<FontFace>>,
) {
    let mut dirs = vec![
        PathBuf::from("/System/Library/Fonts"),
        PathBuf::from("/Library/Fonts"),
        PathBuf::from("/usr/share/fonts"),
        PathBuf::from("/usr/local/share/fonts"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        dirs.push(home.join("Library/Fonts"));
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    let mut stack = dirs;
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                let pretty = stem.replace(['_', '-'], " ");
                register_entry(display_names, by_family, &pretty, &path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_windows_registry_font_names_and_styles() {
        assert_eq!(
            strip_format_tag("Georgia Bold Italic (TrueType)"),
            "Georgia Bold Italic"
        );
        assert_eq!(
            strip_format_tag("Cambria & Cambria Math (TrueType)"),
            "Cambria & Cambria Math"
        );
        assert_eq!(parse_face_name("Georgia"), ("Georgia", false, false, true));
        assert_eq!(
            parse_face_name("Georgia Bold"),
            ("Georgia", true, false, true)
        );
        assert_eq!(
            parse_face_name("Georgia Italic"),
            ("Georgia", false, true, true)
        );
        assert_eq!(
            parse_face_name("Georgia Bold Italic"),
            ("Georgia", true, true, true)
        );
        assert_eq!(
            parse_face_name("Segoe UI Semibold"),
            ("Segoe UI Semibold", false, false, false)
        );
    }

    #[test]
    fn css_generic_families_defer_to_mupdf_builtins() {
        assert!(is_css_generic("serif"));
        assert!(is_css_generic("sans-serif"));
        assert!(is_css_generic("monospace"));
        assert!(!is_css_generic("georgia"));
    }

    #[cfg(windows)]
    #[test]
    fn discovers_installed_windows_system_fonts() {
        let families = system_font_families();
        assert!(
            !families.is_empty(),
            "Windows should expose installed TrueType/OpenType font families"
        );
        // Every standard Windows desktop installation ships Arial or Segoe UI.
        let has_common = families
            .iter()
            .any(|f| f.eq_ignore_ascii_case("Arial") || f.eq_ignore_ascii_case("Segoe UI"));
        assert!(has_common, "expected Arial or Segoe UI in {families:?}");
    }
}
