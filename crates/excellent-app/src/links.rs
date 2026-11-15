//! Links (plan R.7), the view: what the link under the mouse does, as a hint in the title bar, and the question
//! asked before a link opens a file or an unlisted protocol. The canvas finds the links and jumps for links inside
//! the document; the model (`excellent_features::read::links`) decides what any other link may do.

use std::path::{Path, PathBuf};

use excellent_features::read::links::{self, Ask, DEFAULT_PROTOCOLS, Follow};
use nsh_base::{button, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::goto::PageState;
use crate::list_dialog::{card, list_dialog};
use crate::settings;
use crate::tabs::TabState;

/// The settings key of the protocols that open without asking (SumatraPDF's `LinkProtocols`).
pub(crate) const PROTOCOLS_KEY: &str = "link_protocols";

/// The link under the mouse, as the canvas reported it.
#[derive(Debug, Clone, PartialEq)]
struct Hover {
    uri: String,
    /// The page it goes to in this document, if it does.
    page: Option<usize>,
}

/// What the canvas reported about links, and the question being asked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct LinkState {
    hover: Signal<Option<Hover>>,
    /// A link waiting for the reader's answer (or a refusal waiting to be read): never [`Follow::Open`].
    asking: Signal<Option<Follow>>,
    /// The shown tab, whose folder relative file links start from.
    tabs: TabState,
}

impl LinkState {
    pub(crate) fn new(tabs: TabState) -> Self {
        Self {
            hover: signal(None),
            asking: signal(None),
            tabs,
        }
    }

    /// Whether the link dialog is showing.
    pub(crate) fn is_asking(self) -> bool {
        self.asking.with(Option::is_some)
    }

    /// Takes the canvas's `linkHover` and `openLink` events.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        let uri = || event.data.get("uri").and_then(JsonValue::as_str);
        match &*event.name {
            "linkHover" => {
                let page = event.data.get("page").and_then(JsonValue::as_u64);
                let hover = uri().map(|uri| Hover {
                    uri: uri.to_owned(),
                    page: page.and_then(|page| usize::try_from(page).ok()),
                });
                self.hover.set(hover);
            }
            "openLink" => {
                if let Some(uri) = uri() {
                    self.follow(uri);
                }
            }
            _ => {}
        }
    }

    /// Follows external link `uri` as the reader's settings allow: at once, after asking, or not at all.
    fn follow(self, uri: &str) {
        let protocols = settings::shared()
            .text(PROTOCOLS_KEY)
            .unwrap_or(DEFAULT_PROTOCOLS)
            .to_owned();
        match links::follow(uri, &protocols) {
            Follow::Open(uri) => open(&uri, Ask::Protocol, None),
            other => {
                // The mouse is on the dialog now, not the link.
                self.hover.set(None);
                self.asking.set(Some(other));
            }
        }
    }
}

/// Hands `uri` to the system: a file or program link through the file opener, anything else as a URL.
/// The request goes out at once; the task only logs a failure (it ends early if the dialog that asked closes first).
fn open(uri: &str, kind: Ask, document: Option<PathBuf>) {
    match kind {
        Ask::File => {
            let base = document.as_deref().and_then(Path::parent);
            let Some(path) = file_path(uri, base) else {
                log::warn!("link: no file in {uri}");
                return;
            };
            let answer = open_path(path);
            spawn(async move {
                if let Err(err) = answer.await {
                    log::warn!("link: cannot open the file: {err}");
                }
            });
        }
        Ask::Protocol => {
            let answer = open_url(uri);
            let uri = uri.to_owned();
            spawn(async move {
                if !answer.await {
                    log::warn!("link: nothing opened {uri}");
                }
            });
        }
    }
}

/// The local file a link names (`file:`, `file:///C:/…`, a path), its `#…` dropped and its `%xx` escapes decoded,
/// made absolute against `base` (the document's folder) when it is relative. `None` when nothing is left.
pub(crate) fn file_path(uri: &str, base: Option<&Path>) -> Option<PathBuf> {
    let rest = match uri.get(..5) {
        Some(scheme) if scheme.eq_ignore_ascii_case("file:") => &uri[5..],
        _ => uri,
    };
    // `file:///C:/x`: an empty host. `file://server/share` stays a network path.
    let rest = rest
        .strip_prefix("//")
        .filter(|r| r.starts_with('/'))
        .unwrap_or(rest);
    let rest = rest.split_once('#').map_or(rest, |(path, _)| path);
    let decoded = percent_decode(rest);
    // `/C:/x`: the slash before a drive letter.
    let bytes = decoded.as_bytes();
    let path = match bytes {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => &decoded[1..],
        _ => decoded.as_str(),
    };
    if path.is_empty() {
        return None;
    }
    let path = Path::new(path);
    Some(match base {
        Some(base) if path.is_relative() => base.join(path),
        _ => path.to_path_buf(),
    })
}

/// `text` with its `%xx` escapes decoded (MuPDF escapes the paths it puts in `file:` links); a `%` that starts no
/// escape stays, and bytes that aren't UTF-8 are replaced.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|hex| {
            let hex = std::str::from_utf8(hex).ok()?;
            u8::from_str_radix(hex, 16).ok()
        });
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What the link under the mouse does, in the title bar: "Go to page iv (4 of 210)" or its address. Shown only while
/// the mouse is over a link.
///
/// shortcut: the hint sits in the title bar until the status bar exists, and a link inside the document shows no
/// preview of its page until thumbnails do (plan R.5).
pub(crate) fn link_hint(state: LinkState, pages: PageState) -> impl IntoElement {
    let line = move || {
        state.hover.with(|hover| match hover {
            Some(Hover {
                page: Some(page), ..
            }) => t!("link.to_page", page = pages.describe(*page)),
            Some(Hover { uri, .. }) => uri.clone(),
            None => String::new(),
        })
    };
    when(
        move || state.hover.with(Option::is_some),
        move || {
            text(line)
                .max_w(px(320.0))
                .truncate()
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground)
        },
    )
}

/// The question before a link opens a file, a program or an unlisted protocol, or the notice that a script link
/// never runs. Cancel comes first, so Enter on the first focus doesn't open anything.
pub(crate) fn link_dialog(state: LinkState) -> impl IntoElement {
    let close = move || state.asking.set(None);
    let asking = move || state.asking.get();
    let (title, body) = match asking() {
        Some(Follow::Ask {
            reason: Ask::File, ..
        }) => (t!("link.file.title"), t!("link.file.body")),
        Some(Follow::Ask { .. }) => (t!("link.protocol.title"), t!("link.protocol.body")),
        _ => (t!("link.refused.title"), t!("link.refused.body")),
    };
    let uri = match asking() {
        Some(Follow::Ask { uri, .. } | Follow::Refuse(uri) | Follow::Open(uri)) => uri,
        None => String::new(),
    };
    let confirm = match asking() {
        Some(Follow::Ask { uri, reason }) => Some((uri, reason)),
        _ => None,
    };
    let mut actions = view().flex_row().justify_end().gap_2().pt_2();
    actions = match confirm {
        Some((uri, reason)) => actions
            .child(dialog_button(t!("link.cancel"), Tone::Quiet, close))
            .child(dialog_button(t!("link.open"), Tone::Primary, move || {
                close();
                open(&uri, reason, untracked(|| state.tabs.shown_path()));
            })),
        None => actions.child(dialog_button(t!("link.ok"), Tone::Primary, close)),
    };
    let content = view()
        .gap_2()
        .p_5()
        .child(
            text(title.clone())
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            text(body)
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(uri)
                .text_size(px(12.0))
                .px_3()
                .py_2()
                .rounded(px(6.0))
                .bg(move || theme().muted)
                .text_color(move || theme().popover_foreground),
        )
        .child(actions);
    list_dialog(title, close, card(content).w(px(440.0)))
}

/// A dialog button's emphasis.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tone {
    Primary,
    Quiet,
}

pub(crate) fn dialog_button(
    label: String,
    tone: Tone,
    press: impl Fn() + 'static,
) -> impl IntoElement {
    let primary = tone == Tone::Primary;
    button()
        .accessibility_label(label.clone())
        .on_press(press)
        .h(px(32.0))
        .px_4()
        .rounded(px(8.0))
        .items_center()
        .justify_center()
        .bg(move || {
            if primary {
                theme().primary
            } else {
                theme().secondary
            }
        })
        .hover(|s| s.opacity(0.9))
        .active(|s| s.opacity(0.7))
        .focus_visible(|s| s.border_2().border_color(move || theme().ring))
        .child(
            text(label)
                .text_size(px(13.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || {
                    if primary {
                        theme().primary_foreground
                    } else {
                        theme().secondary_foreground
                    }
                }),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_links_become_paths() {
        let base = Path::new("docs");
        assert_eq!(
            file_path("file:other.pdf#page=1", Some(base)),
            Some(base.join("other.pdf"))
        );
        assert_eq!(
            file_path("file://server/share/x.pdf", None),
            Some(PathBuf::from("//server/share/x.pdf"))
        );
        assert_eq!(file_path("file:#page=3", Some(base)), None);
    }

    // Drive letters make a path absolute only on Windows.
    #[cfg(windows)]
    #[test]
    fn drive_links_stay_absolute() {
        let base = Path::new(r"C:\docs");
        assert_eq!(
            file_path("file:///C:/Some%20Dir/a.pdf#page=2", Some(base)),
            Some(PathBuf::from("C:/Some Dir/a.pdf"))
        );
        assert_eq!(
            file_path(r"C:\Windows\calc.exe", Some(base)),
            Some(PathBuf::from(r"C:\Windows\calc.exe"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn rooted_links_stay_absolute() {
        let base = Path::new("/docs");
        assert_eq!(
            file_path("file:///home/me/Some%20Dir/a.pdf#page=2", Some(base)),
            Some(PathBuf::from("/home/me/Some Dir/a.pdf"))
        );
    }

    #[test]
    fn escapes_decode_and_stray_percents_stay() {
        assert_eq!(percent_decode("a%20b%C3%A9"), "a bé");
        assert_eq!(percent_decode("100%"), "100%");
        assert_eq!(percent_decode("%zz%4"), "%zz%4");
    }
}
