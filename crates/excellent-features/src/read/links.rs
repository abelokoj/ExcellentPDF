//! Links (plan R.7), the model: what following a link does. A link inside the document jumps; anything else either
//! opens at once (its protocol is on the reader's list, `http`, `https` and `mailto` by default, as SumatraPDF's
//! `LinkProtocols`), asks first, or never runs (plan §14.2: scripts never run, files and launch actions always ask).

/// The protocols that open without asking unless the reader's settings say otherwise.
pub const DEFAULT_PROTOCOLS: &str = "http,https,mailto";

/// What following a link does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Follow {
    /// Hand the address to the system at once.
    Open(String),
    /// Ask the reader first, saying why.
    Ask {
        /// The link's address.
        uri: String,
        /// Why it asks.
        reason: Ask,
    },
    /// Never follow it.
    Refuse(String),
}

/// Why a link asks before it opens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// It opens a file or starts a program (`file:`, a path, a launch action).
    File,
    /// Its protocol isn't on the reader's list.
    Protocol,
}

/// Decides what following external link `uri` does, given the reader's `protocols` (a comma-separated list, as
/// [`DEFAULT_PROTOCOLS`]). Links inside the document never get here: the canvas jumps.
pub fn follow(uri: &str, protocols: &str) -> Follow {
    let uri = uri.trim();
    let owned = uri.to_owned();
    match scheme(uri).as_deref() {
        // A path, relative or with a drive letter, opens a file.
        None => Follow::Ask {
            uri: owned,
            reason: Ask::File,
        },
        Some(scheme) if scheme.len() == 1 || scheme == "file" => Follow::Ask {
            uri: owned,
            reason: Ask::File,
        },
        // Code, or data the reader can't see: never, whatever the list says.
        Some("javascript" | "vbscript" | "data" | "blob" | "about") => Follow::Refuse(owned),
        Some(scheme) if allows(protocols, scheme) => Follow::Open(owned),
        Some(_) => Follow::Ask {
            uri: owned,
            reason: Ask::Protocol,
        },
    }
}

/// `uri`'s scheme, lower case (RFC 3986: a letter, then letters, digits, `+`, `-` or `.`, then `:`).
fn scheme(uri: &str) -> Option<String> {
    let (scheme, _) = uri.split_once(':')?;
    let mut chars = scheme.chars();
    let first = chars.next()?;
    let valid = first.is_ascii_alphabetic()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    valid.then(|| scheme.to_ascii_lowercase())
}

fn allows(protocols: &str, scheme: &str) -> bool {
    protocols
        .split(',')
        .map(str::trim)
        .any(|allowed| allowed.eq_ignore_ascii_case(scheme))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(follow: Follow) -> Option<Ask> {
        match follow {
            Follow::Ask { reason, .. } => Some(reason),
            _ => None,
        }
    }

    #[test]
    fn web_links_open_and_other_protocols_ask() {
        let open = follow("https://example.com/a", DEFAULT_PROTOCOLS);
        assert_eq!(open, Follow::Open("https://example.com/a".to_owned()));
        assert_eq!(
            follow("MAILTO:a@example.com", DEFAULT_PROTOCOLS),
            Follow::Open("MAILTO:a@example.com".to_owned())
        );
        let ask = follow("ftp://example.com", DEFAULT_PROTOCOLS);
        assert_eq!(reason(ask), Some(Ask::Protocol));
        assert_eq!(
            reason(follow("https://example.com", "mailto")),
            Some(Ask::Protocol)
        );
        assert_eq!(
            follow("ftp://example.com", "http, ftp"),
            Follow::Open("ftp://example.com".to_owned())
        );
    }

    #[test]
    fn scripts_never_run_and_files_always_ask() {
        // Even when the reader's list names them.
        let everything = "javascript,file,data,c";
        assert!(matches!(
            follow("javascript:app.alert(1)", everything),
            Follow::Refuse(_)
        ));
        assert!(matches!(
            follow(" JavaScript:x", DEFAULT_PROTOCOLS),
            Follow::Refuse(_)
        ));
        for file in [
            "file:calc.exe#page=1",
            "file:///C:/x.pdf",
            r"C:\Windows\calc.exe",
            "other.pdf",
        ] {
            assert_eq!(reason(follow(file, everything)), Some(Ask::File), "{file}");
        }
    }
}
