//! Extended PDF outline actions: `/GoToR` (remote file), `/Launch`, `/Named`, and `/URI`
//! inspection, editing, and safe target resolution (plan B.12).

use std::fmt::{self, Write as _};
use std::path::{Component, Path, PathBuf};

/// Error returned when parsing, validating, or resolving an outline action.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum OutlineActionError {
    /// Target path escapes the allowed workspace root or is unsafe.
    UnsafePath(String),
    /// The URI or action syntax is malformed.
    InvalidAction(String),
}

impl fmt::Display for OutlineActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsafePath(msg) => write!(f, "unsafe outline action path: {msg}"),
            Self::InvalidAction(msg) => write!(f, "invalid outline action: {msg}"),
        }
    }
}

impl std::error::Error for OutlineActionError {}

/// Convenience result alias for outline action operations.
pub type Result<T> = std::result::Result<T, OutlineActionError>;

/// Whether a `/GoToR` or `/Launch` action opens a new window or reuses the current window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum WindowOpenMode {
    /// Reuse the active document window.
    #[default]
    ReuseCurrent,
    /// Open the target in a new window (`/NewWindow true`).
    NewWindow,
}

/// Security confirmation policy for `/Launch` actions (plan §14.2, B.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum LaunchTrust {
    /// Always prompt/block automatic execution of external programs.
    #[default]
    RequireConfirmation,
    /// Explicitly approved by the user for a non-executable document.
    UserConfirmed,
}

/// Whether a `/URI` action appends image-map coordinates (`/IsMap`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum UriMapMode {
    /// Plain hyperlink URI.
    #[default]
    Plain,
    /// Append click coordinates as query parameters.
    ImageMap,
}

/// Standard PDF named actions (ISO 32000-1 §12.6.4.11) (plan B.12).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NamedAction {
    /// Navigate to the next page (`/NextPage`).
    NextPage,
    /// Navigate to the previous page (`/PrevPage`).
    PrevPage,
    /// Navigate to the first page (`/FirstPage`).
    FirstPage,
    /// Navigate to the last page (`/LastPage`).
    LastPage,
    /// Go back in view history (`/GoBack`).
    GoBack,
    /// Go forward in view history (`/GoForward`).
    GoForward,
    /// Open Find dialog (`/Find`).
    Find,
    /// Open Print dialog (`/Print`).
    Print,
    /// Non-standard named action.
    Custom(String),
}

impl NamedAction {
    /// PDF `/N` name without leading slash.
    #[must_use]
    pub fn pdf_name(&self) -> &str {
        match self {
            Self::NextPage => "NextPage",
            Self::PrevPage => "PrevPage",
            Self::FirstPage => "FirstPage",
            Self::LastPage => "LastPage",
            Self::GoBack => "GoBack",
            Self::GoForward => "GoForward",
            Self::Find => "Find",
            Self::Print => "Print",
            Self::Custom(name) => name.as_str(),
        }
    }

    /// Parses a PDF `/N` action name.
    #[must_use]
    pub fn from_pdf_name(name: &str) -> Self {
        match name.trim_start_matches('/') {
            "NextPage" => Self::NextPage,
            "PrevPage" => Self::PrevPage,
            "FirstPage" => Self::FirstPage,
            "LastPage" => Self::LastPage,
            "GoBack" => Self::GoBack,
            "GoForward" => Self::GoForward,
            "Find" => Self::Find,
            "Print" => Self::Print,
            other => Self::Custom(other.to_owned()),
        }
    }
}

/// Tree icon hint shown beside an outline entry (plan B.12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum OutlineActionIcon {
    /// Internal page destination.
    Page,
    /// External PDF/document file (`/GoToR`).
    RemoteFile,
    /// Web or custom URI (`/URI`).
    WebLink,
    /// Viewer command (`/Named`).
    NamedCommand,
    /// External launch (`/Launch`), displayed with a security warning badge.
    LaunchWarning,
}

/// Extended PDF outline action model (plan B.12).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum OutlineAction {
    /// Internal `/GoTo` page destination (`#page=N`).
    InternalPage {
        /// Zero-based page index.
        page: usize,
    },
    /// Remote GoTo (`/GoToR`) opening another PDF file at an optional zero-based page or named
    /// destination.
    RemoteGoto {
        /// File path or relative path from the current document's directory.
        file: PathBuf,
        /// Optional zero-based target page in the remote document.
        page: Option<usize>,
        /// Optional named destination in the remote document.
        named_dest: Option<String>,
        /// Window behavior.
        window: WindowOpenMode,
    },
    /// External `/Launch` action targeting an application or document file.
    Launch {
        /// Target file/application path.
        target: PathBuf,
        /// Optional command-line parameters.
        parameters: Option<String>,
        /// Window behavior.
        window: WindowOpenMode,
        /// Confirmation policy before opening.
        trust: LaunchTrust,
    },
    /// Named viewer action (`/Named`).
    Named(NamedAction),
    /// Hyperlink URI (`/URI`).
    Uri {
        /// Target URL string.
        url: String,
        /// Whether image-map query coordinates are appended.
        map_mode: UriMapMode,
    },
}

impl OutlineAction {
    /// Icon kind to display in the outline tree (plan B.12).
    #[must_use]
    pub const fn icon(&self) -> OutlineActionIcon {
        match self {
            Self::InternalPage { .. } => OutlineActionIcon::Page,
            Self::RemoteGoto { .. } => OutlineActionIcon::RemoteFile,
            Self::Launch { .. } => OutlineActionIcon::LaunchWarning,
            Self::Named(_) => OutlineActionIcon::NamedCommand,
            Self::Uri { .. } => OutlineActionIcon::WebLink,
        }
    }

    /// Serializes the action as a compact PDF `/A << ... >>` dictionary string (plan B.12).
    #[must_use]
    pub fn to_pdf_action_dict(&self) -> String {
        match self {
            Self::InternalPage { page } => {
                format!("<< /S /GoTo /D [{} /Fit] >>", page)
            }
            Self::RemoteGoto {
                file,
                page,
                named_dest,
                window,
            } => {
                let mut out = format!(
                    "<< /S /GoToR /F ({})",
                    escape_pdf_str(&file.to_string_lossy())
                );
                if let Some(p) = page {
                    let _ = write!(out, " /D [{p} /Fit]");
                } else if let Some(dest) = named_dest {
                    let _ = write!(out, " /D ({})", escape_pdf_str(dest));
                }
                if *window == WindowOpenMode::NewWindow {
                    out.push_str(" /NewWindow true");
                }
                out.push_str(" >>");
                out
            }
            Self::Launch {
                target,
                parameters,
                window,
                ..
            } => {
                let mut out = format!(
                    "<< /S /Launch /F ({})",
                    escape_pdf_str(&target.to_string_lossy())
                );
                if let Some(params) = parameters {
                    let _ = write!(out, " /P ({})", escape_pdf_str(params));
                }
                if *window == WindowOpenMode::NewWindow {
                    out.push_str(" /NewWindow true");
                }
                out.push_str(" >>");
                out
            }
            Self::Named(named) => {
                format!("<< /S /Named /N /{} >>", named.pdf_name())
            }
            Self::Uri { url, map_mode } => {
                let mut out = format!("<< /S /URI /URI ({})", escape_pdf_str(url));
                if *map_mode == UriMapMode::ImageMap {
                    out.push_str(" /IsMap true");
                }
                out.push_str(" >>");
                out
            }
        }
    }

    /// Converts the action to a canonical URI representation compatible with MuPDF and
    /// `ecclesia-features::outline::Entry::uri`.
    #[must_use]
    pub fn to_link_uri(&self) -> String {
        match self {
            Self::InternalPage { page } => format!("#page={}", page + 1),
            Self::RemoteGoto {
                file,
                page,
                named_dest,
                ..
            } => {
                let f = file.to_string_lossy().replace('\\', "/");
                if let Some(p) = page {
                    format!("file://{f}#page={}", p + 1)
                } else if let Some(dest) = named_dest {
                    format!("file://{f}#nameddest={dest}")
                } else {
                    format!("file://{f}")
                }
            }
            Self::Launch { target, .. } => {
                format!("launch:{}", target.to_string_lossy().replace('\\', "/"))
            }
            Self::Named(named) => format!("named:{}", named.pdf_name()),
            Self::Uri { url, .. } => url.clone(),
        }
    }

    /// Parses a canonical link URI or PDF action dictionary string into an [`OutlineAction`].
    ///
    /// # Errors
    ///
    /// Returns [`OutlineActionError::InvalidAction`] if the input is empty or unrecognized.
    pub fn parse(raw: &str) -> Result<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(OutlineActionError::InvalidAction(
                "outline action string is empty".into(),
            ));
        }
        if trimmed.starts_with("<<") {
            return parse_pdf_action_dict(trimmed);
        }
        if let Some(rest) = trimmed.strip_prefix("#page=") {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let p1 = digits
                .parse::<usize>()
                .map_err(|_| OutlineActionError::InvalidAction("invalid #page number".into()))?;
            return Ok(Self::InternalPage {
                page: p1.saturating_sub(1),
            });
        }
        if let Some(rest) = trimmed.strip_prefix("named:") {
            return Ok(Self::Named(NamedAction::from_pdf_name(rest)));
        }
        if let Some(rest) = trimmed.strip_prefix("launch:") {
            return Ok(Self::Launch {
                target: PathBuf::from(rest),
                parameters: None,
                window: WindowOpenMode::ReuseCurrent,
                trust: LaunchTrust::RequireConfirmation,
            });
        }
        if let Some(rest) = trimmed.strip_prefix("file://") {
            let (file_part, frag) = rest.split_once('#').unwrap_or((rest, ""));
            let mut page = None;
            let mut named_dest = None;
            if let Some(pstr) = frag.strip_prefix("page=") {
                let digits: String = pstr.chars().take_while(char::is_ascii_digit).collect();
                if let Ok(p1) = digits.parse::<usize>() {
                    page = Some(p1.saturating_sub(1));
                }
            } else if let Some(dest) = frag.strip_prefix("nameddest=") {
                named_dest = Some(dest.to_owned());
            }
            return Ok(Self::RemoteGoto {
                file: PathBuf::from(file_part),
                page,
                named_dest,
                window: WindowOpenMode::ReuseCurrent,
            });
        }
        if trimmed.contains("://") || trimmed.starts_with("mailto:") {
            return Ok(Self::Uri {
                url: trimmed.to_owned(),
                map_mode: UriMapMode::Plain,
            });
        }
        Err(OutlineActionError::InvalidAction(format!(
            "unrecognized outline action '{trimmed}'"
        )))
    }
}

/// Resolves a `/GoToR` remote file path relative to `current_doc_dir`, rejecting traversal outside
/// `current_doc_dir` when `file` is relative (plan B.12, §14.2).
///
/// # Errors
///
/// Returns [`OutlineActionError::UnsafePath`] if the relative path traverses above `current_doc_dir`.
pub fn resolve_remote_target(current_doc_dir: &Path, file: &Path) -> Result<PathBuf> {
    if file.is_absolute() {
        return Ok(file.to_path_buf());
    }
    let mut depth = 0isize;
    for comp in file.components() {
        match comp {
            Component::ParentDir => {
                depth -= 1;
                if depth < 0 {
                    return Err(OutlineActionError::UnsafePath(format!(
                        "remote path '{}' escapes base directory",
                        file.display()
                    )));
                }
            }
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::RootDir | Component::Prefix(_) => {
                return Err(OutlineActionError::UnsafePath(
                    "unexpected root component in relative path".into(),
                ));
            }
        }
    }
    Ok(current_doc_dir.join(file))
}

/// Checks whether a `/Launch` target is safe to open without executing programs (plan §14.2).
#[must_use]
pub fn is_executable_launch_target(target: &Path) -> bool {
    target
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .is_some_and(|ext| {
            matches!(
                ext.as_str(),
                "exe" | "bat" | "cmd" | "com" | "msi" | "ps1" | "vbs" | "js" | "sh" | "app"
            )
        })
}

fn parse_pdf_action_dict(dict_str: &str) -> Result<OutlineAction> {
    let subtype = extract_pdf_name_field(dict_str, "/S")
        .ok_or_else(|| OutlineActionError::InvalidAction("action dictionary missing /S".into()))?;
    let window = if dict_str.contains("/NewWindow true") {
        WindowOpenMode::NewWindow
    } else {
        WindowOpenMode::ReuseCurrent
    };
    match subtype.as_str() {
        "GoToR" => {
            let file = extract_pdf_paren_field(dict_str, "/F").ok_or_else(|| {
                OutlineActionError::InvalidAction("/GoToR missing /F file".into())
            })?;
            let page = extract_pdf_dest_page(dict_str);
            let named_dest = if page.is_none() {
                extract_pdf_paren_field(dict_str, "/D")
            } else {
                None
            };
            Ok(OutlineAction::RemoteGoto {
                file: PathBuf::from(file),
                page,
                named_dest,
                window,
            })
        }
        "Launch" => {
            let target = extract_pdf_paren_field(dict_str, "/F").ok_or_else(|| {
                OutlineActionError::InvalidAction("/Launch missing /F target".into())
            })?;
            let parameters = extract_pdf_paren_field(dict_str, "/P");
            Ok(OutlineAction::Launch {
                target: PathBuf::from(target),
                parameters,
                window,
                trust: LaunchTrust::RequireConfirmation,
            })
        }
        "Named" => {
            let name = extract_pdf_name_field(dict_str, "/N").ok_or_else(|| {
                OutlineActionError::InvalidAction("/Named missing /N action name".into())
            })?;
            Ok(OutlineAction::Named(NamedAction::from_pdf_name(&name)))
        }
        "URI" => {
            let url = extract_pdf_paren_field(dict_str, "/URI").ok_or_else(|| {
                OutlineActionError::InvalidAction("/URI missing /URI string".into())
            })?;
            let map_mode = if dict_str.contains("/IsMap true") {
                UriMapMode::ImageMap
            } else {
                UriMapMode::Plain
            };
            Ok(OutlineAction::Uri { url, map_mode })
        }
        "GoTo" => {
            let page = extract_pdf_dest_page(dict_str).unwrap_or(0);
            Ok(OutlineAction::InternalPage { page })
        }
        other => Err(OutlineActionError::InvalidAction(format!(
            "unsupported action subtype /{other}"
        ))),
    }
}

fn find_pdf_key_value_slice<'a>(s: &'a str, key: &str, expected_prefix: char) -> Option<&'a str> {
    let mut start = 0usize;
    while let Some(rel) = s[start..].find(key) {
        let pos = start + rel;
        let before_ok = pos == 0
            || s[..pos]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_whitespace() || matches!(c, '<' | '[' | ']'));
        let after = &s[pos + key.len()..];
        let after_boundary_ok = after
            .chars()
            .next()
            .is_some_and(|c| c.is_whitespace() || matches!(c, '/' | '(' | '<' | '['));
        let trimmed = after.trim_start();
        if before_ok && after_boundary_ok && trimmed.starts_with(expected_prefix) {
            return Some(trimmed);
        }
        start = pos + key.len();
    }
    None
}

fn extract_pdf_name_field(s: &str, key: &str) -> Option<String> {
    let rest = find_pdf_key_value_slice(s, key, '/')?;
    let after_slash = rest.strip_prefix('/')?;
    let name: String = after_slash
        .chars()
        .take_while(|c| !c.is_whitespace() && !matches!(c, '/' | '>' | '<' | '[' | ']' | '('))
        .collect();
    (!name.is_empty()).then_some(name)
}

fn extract_pdf_paren_field(s: &str, key: &str) -> Option<String> {
    let rest = find_pdf_key_value_slice(s, key, '(')?;
    let inner = rest.strip_prefix('(')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    let mut depth = 1usize;
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                if let Some(esc) = chars.next() {
                    out.push(esc);
                }
            }
            '(' => {
                depth += 1;
                out.push('(');
            }
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(out);
                }
                out.push(')');
            }
            other => out.push(other),
        }
    }
    None
}

fn extract_pdf_dest_page(s: &str) -> Option<usize> {
    let pos = s.find("/D")?;
    let rest = s[pos + 2..].trim_start();
    let inner = rest.strip_prefix('[')?;
    let digits: String = inner
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<usize>().ok()
}

fn escape_pdf_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outline_actions_round_trip_and_resolve_remote_files_safely() {
        let remote = OutlineAction::RemoteGoto {
            file: PathBuf::from("appendix/data.pdf"),
            page: Some(4),
            named_dest: None,
            window: WindowOpenMode::NewWindow,
        };
        assert_eq!(remote.icon(), OutlineActionIcon::RemoteFile);
        let pdf_dict = remote.to_pdf_action_dict();
        let parsed = OutlineAction::parse(&pdf_dict).expect("parse GoToR");
        assert_eq!(parsed, remote);
        assert_eq!(remote.to_link_uri(), "file://appendix/data.pdf#page=5");
        assert_eq!(
            OutlineAction::parse(&remote.to_link_uri()).expect("uri parse"),
            OutlineAction::RemoteGoto {
                file: PathBuf::from("appendix/data.pdf"),
                page: Some(4),
                named_dest: None,
                window: WindowOpenMode::ReuseCurrent,
            }
        );

        let resolved =
            resolve_remote_target(Path::new("/papers/main"), Path::new("appendix/data.pdf"))
                .expect("safe relative path");
        assert_eq!(resolved, PathBuf::from("/papers/main/appendix/data.pdf"));
        assert!(
            resolve_remote_target(Path::new("/papers/main"), Path::new("../../etc/passwd"))
                .is_err()
        );

        let named = OutlineAction::Named(NamedAction::NextPage);
        assert_eq!(named.icon(), OutlineActionIcon::NamedCommand);
        assert_eq!(
            OutlineAction::parse(&named.to_pdf_action_dict()).expect("named"),
            named
        );

        let uri = OutlineAction::Uri {
            url: "https://example.org/docs".into(),
            map_mode: UriMapMode::ImageMap,
        };
        assert_eq!(uri.icon(), OutlineActionIcon::WebLink);
        assert_eq!(
            OutlineAction::parse(&uri.to_pdf_action_dict()).expect("uri"),
            uri
        );

        let launch = OutlineAction::Launch {
            target: PathBuf::from("tools/run.exe"),
            parameters: Some("--check".into()),
            window: WindowOpenMode::ReuseCurrent,
            trust: LaunchTrust::RequireConfirmation,
        };
        assert_eq!(launch.icon(), OutlineActionIcon::LaunchWarning);
        assert!(is_executable_launch_target(Path::new("tools/run.exe")));
        assert!(!is_executable_launch_target(Path::new("notes.pdf")));
    }
}
