//! File associations (plan K.19 step 1, R.11 step 2): the app offers to open its document types, per user, and the
//! user makes it the default in the system's own settings (Windows lets no app make itself the default).
//!
//! Windows: a ProgID, "Open with" entries for each extension, and the app's capabilities, so Settings > Default apps
//! lists it. The entries are a pure list ([`entries`]); [`register`] writes them.

use std::path::Path;

/// The app's name in the registry: its ProgID is `<APP>.Document`.
pub const APP: &str = "ExcellentPDF";

/// One registry value under `HKEY_CURRENT_USER`: a key, the value's name (`None` for the key's default value) and
/// its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The key, under `HKEY_CURRENT_USER`.
    pub key: String,
    /// The value's name; `None` for the key's default value.
    pub name: Option<String>,
    /// The value, a string.
    pub value: String,
}

fn entry(key: impl Into<String>, name: Option<&str>, value: impl Into<String>) -> Entry {
    Entry {
        key: key.into(),
        name: name.map(str::to_owned),
        value: value.into(),
    }
}

/// What registering `exe` for `extensions` (without dots) writes; `description` names its documents in the file
/// manager.
pub fn entries(exe: &Path, extensions: &[&str], description: &str) -> Vec<Entry> {
    let exe_text = exe.display().to_string();
    let exe_name = exe.file_name().map_or_else(
        || exe_text.clone(),
        |name| name.to_string_lossy().into_owned(),
    );
    let command = format!("\"{exe_text}\" \"%1\"");
    let prog_id = format!("{APP}.Document");
    let classes = r"Software\Classes";
    let capabilities = format!(r"Software\{APP}\Capabilities");
    let application = format!(r"{classes}\Applications\{exe_name}");
    let mut entries = vec![
        entry(format!(r"{classes}\{prog_id}"), None, description),
        entry(
            format!(r"{classes}\{prog_id}\DefaultIcon"),
            None,
            format!("\"{exe_text}\",0"),
        ),
        entry(
            format!(r"{classes}\{prog_id}\shell\open\command"),
            None,
            command.clone(),
        ),
        entry(format!(r"{application}\shell\open\command"), None, command),
        entry(capabilities.clone(), Some("ApplicationName"), APP),
        entry(
            capabilities.clone(),
            Some("ApplicationDescription"),
            description,
        ),
        entry(
            r"Software\RegisteredApplications",
            Some(APP),
            capabilities.clone(),
        ),
    ];
    for extension in extensions {
        let dotted = format!(".{extension}");
        entries.push(entry(
            format!(r"{classes}\{dotted}\OpenWithProgids"),
            Some(&prog_id),
            "",
        ));
        entries.push(entry(
            format!(r"{application}\SupportedTypes"),
            Some(&dotted),
            "",
        ));
        entries.push(entry(
            format!(r"{capabilities}\FileAssociations"),
            Some(&dotted),
            prog_id.clone(),
        ));
    }
    entries
}

/// Registers `exe` for `extensions` for this user (see [`entries`]) and tells the shell.
///
/// # Errors
///
/// A registry write failed.
#[cfg(windows)]
pub fn register(exe: &Path, extensions: &[&str], description: &str) -> std::io::Result<()> {
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };
    use windows::Win32::UI::Shell::{SHCNE_ASSOCCHANGED, SHCNF_IDLIST, SHChangeNotify};
    use windows::core::{HSTRING, PCWSTR};

    for Entry { key, name, value } in entries(exe, extensions, description) {
        let key = HSTRING::from(key);
        let mut handle = HKEY::default();
        // SAFETY: `key` outlives the call and `handle` is a valid out-pointer.
        unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                &key,
                None,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                None,
                &raw mut handle,
                None,
            )
        }
        .ok()
        .map_err(std::io::Error::other)?;
        let name = name.map(HSTRING::from);
        let name = name
            .as_ref()
            .map_or(PCWSTR::null(), |name| PCWSTR(name.as_ptr()));
        // REG_SZ data: UTF-16 with its terminating NUL, as bytes.
        let data: Vec<u8> = value
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        // SAFETY: `handle` was opened above; `name` and `data` outlive the call.
        let set = unsafe { RegSetValueExW(handle, name, None, REG_SZ, Some(&data)) };
        // SAFETY: `handle` was opened above and is closed once.
        let _ = unsafe { RegCloseKey(handle) };
        set.ok().map_err(std::io::Error::other)?;
    }
    // SAFETY: no items are passed with SHCNE_ASSOCCHANGED.
    unsafe { SHChangeNotify(SHCNE_ASSOCCHANGED, SHCNF_IDLIST, None, None) };
    Ok(())
}

/// Opens the system's default-apps settings at the app, where the user makes it the default viewer.
///
/// # Errors
///
/// The settings could not be opened.
#[cfg(windows)]
pub fn open_default_apps() -> std::io::Result<()> {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    let target = HSTRING::from(format!("ms-settings:defaultapps?registeredAppUser={APP}"));
    // SAFETY: every string outlives the call; no window owns the request.
    let result = unsafe { ShellExecuteW(None, w!("open"), &target, None, None, SW_SHOWNORMAL) };
    // ShellExecute reports success with a value above 32.
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[test]
    fn registering_offers_every_type_and_lists_the_app_in_default_apps() {
        let exe = PathBuf::from(r"C:\Apps\ExcellentPDF\excellentpdf.exe");
        let entries = entries(&exe, &["pdf", "epub"], "Document");
        let value = |key: &str, name: Option<&str>| {
            entries
                .iter()
                .find(|entry| entry.key == key && entry.name.as_deref() == name)
                .map(|entry| entry.value.clone())
        };
        assert_eq!(
            value(
                r"Software\Classes\ExcellentPDF.Document\shell\open\command",
                None
            )
            .as_deref(),
            Some(r#""C:\Apps\ExcellentPDF\excellentpdf.exe" "%1""#)
        );
        for extension in [".pdf", ".epub"] {
            let open_with = format!(r"Software\Classes\{extension}\OpenWithProgids");
            assert_eq!(
                value(&open_with, Some("ExcellentPDF.Document")).as_deref(),
                Some("")
            );
            assert_eq!(
                value(
                    r"Software\ExcellentPDF\Capabilities\FileAssociations",
                    Some(extension)
                )
                .as_deref(),
                Some("ExcellentPDF.Document")
            );
        }
        assert_eq!(
            value(r"Software\RegisteredApplications", Some("ExcellentPDF")).as_deref(),
            Some(r"Software\ExcellentPDF\Capabilities")
        );
        // The extension's own default (the user's choice) is never written.
        assert!(
            !entries
                .iter()
                .any(|entry| entry.key == r"Software\Classes\.pdf" && entry.name.is_none())
        );
    }
}
