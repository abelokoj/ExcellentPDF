//! Opening files from outside the app (plan R.11): the command line, a second launch (single instance) and files
//! dropped on the window.
//!
//! A launch's command line becomes a [`Launch`]. The first launch shows it when the window opens; a later launch sends
//! it to the first over the single-instance channel (`ecclesia_platform::instance`) and ends. Its paths are made
//! absolute in the sending launch, whose folder the first doesn't know.

use std::collections::{HashSet, VecDeque};
use std::ffi::OsString;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Mutex, OnceLock};
use std::task::{Context, Poll, Waker};

use ecclesia_core::{formats, formats_extra::ExtraFormat};
use ecclesia_platform::instance::{self, Claim};
use serde_json::{Value, json};

/// What a launch asks for: files to show, the page to show them at, and whether in panes side by side.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Launch {
    /// The files, absolute, in the command line's order; the last is shown.
    pub(crate) files: Vec<PathBuf>,
    /// `--page N`: the page to show them at, from 0.
    pub(crate) page: Option<usize>,
    /// `--split` or `--split-same` (V.13): the files in panes of their own.
    pub(crate) split: Option<Split>,
}

/// How a launch splits the window (V.13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Split {
    /// `--split a.pdf b.pdf`: each file in a pane of its own, side by side.
    Files,
    /// `--split-same a.pdf`: the file in two panes side by side.
    Same,
}

impl Split {
    /// Its name in a single-instance message.
    fn name(self) -> &'static str {
        match self {
            Self::Files => "files",
            Self::Same => "same",
        }
    }

    fn of(name: &str) -> Option<Self> {
        [Self::Files, Self::Same]
            .into_iter()
            .find(|split| split.name() == name)
    }
}

/// Reads a command line (without the program's name) given in `cwd`: files, `--page N` (from 1), `--split` and
/// `--split-same`. Other flags are logged and skipped.
pub(crate) fn parse(args: impl IntoIterator<Item = OsString>, cwd: &Path) -> Launch {
    let mut launch = Launch::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let text = arg.to_string_lossy();
        if let Some(flag) = text.strip_prefix("--") {
            let (name, inline) = match flag.split_once('=') {
                Some((name, value)) => (name, Some(value.to_owned())),
                None => (flag, None),
            };
            match name {
                "page" => {
                    let value = inline
                        .or_else(|| args.next().map(|arg| arg.to_string_lossy().into_owned()));
                    match value.as_deref().map(str::parse::<usize>) {
                        Some(Ok(page)) if page > 0 => launch.page = Some(page - 1),
                        _ => log::warn!("command line: --page takes a page number from 1"),
                    }
                }
                "split" => launch.split = Some(Split::Files),
                "split-same" => launch.split = Some(Split::Same),
                // shortcut: --zoom, --search and --forward-search wait for their features (see R.11's doc).
                _ => log::warn!("command line: unknown flag --{name}"),
            }
        } else {
            launch.files.push(cwd.join(arg));
        }
    }
    launch
}

/// `launch` as the single-instance channel carries it.
fn to_message(launch: &Launch) -> Vec<u8> {
    let files: Vec<String> = launch
        .files
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    json!({ "files": files, "page": launch.page, "split": launch.split.map(Split::name) })
        .to_string()
        .into_bytes()
}

/// A message from another launch; `None` when it is no launch (the channel is the user's own, but anything may
/// write to it).
fn from_message(message: &[u8]) -> Option<Launch> {
    let value: Value = serde_json::from_slice(message).ok()?;
    let files = value
        .get("files")?
        .as_array()?
        .iter()
        .map(|file| file.as_str().map(PathBuf::from))
        .collect::<Option<Vec<_>>>()?;
    // Only absolute paths: a relative one would be read against this instance's folder, not the sender's.
    if !files.iter().all(|file| file.is_absolute()) {
        return None;
    }
    let page = match value.get("page") {
        None | Some(Value::Null) => None,
        Some(page) => Some(usize::try_from(page.as_u64()?).ok()?),
    };
    let split = match value.get("split") {
        None | Some(Value::Null) => None,
        Some(split) => Some(Split::of(split.as_str()?)?),
    };
    Some(Launch { files, page, split })
}

/// The documents among files dropped on the window (`file:` URLs): directories, non-document files,
/// and duplicate paths are left out. A file is taken by its contents ([`formats::is_document`]), so
/// a renamed one still opens.
pub(crate) fn dropped_documents(uris: &[String]) -> Vec<PathBuf> {
    let paths: Vec<PathBuf> = uris
        .iter()
        .filter_map(|uri| crate::links::file_path(uri, None))
        .collect();
    dropped_paths(&paths)
}

/// The documents among filesystem `paths` dropped on the window: directories, non-document files,
/// and duplicate paths are left out while preserving first-seen order.
pub(crate) fn dropped_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = HashSet::new();
    let mut accepted = Vec::new();
    for path in paths {
        if path.is_file() && is_document(path) && seen.insert(path.as_path()) {
            accepted.push(path.clone());
        }
    }
    accepted
}

/// Extracts filesystem paths from a native `"osFileDrop"` event payload (`{ "paths": [...] }`).
///
/// Plain filesystem paths are preserved verbatim; entries beginning with `file:` (case-insensitive)
/// are decoded via [`crate::links::file_path`]. Empty or non-string entries are ignored.
pub(crate) fn paths_from_event(data: &Value) -> Vec<PathBuf> {
    let Some(items) = data.get("paths").and_then(Value::as_array) else {
        return Vec::new();
    };
    items
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|raw| {
            if raw.is_empty() {
                None
            } else if raw
                .get(..5)
                .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file:"))
            {
                crate::links::file_path(raw, None)
            } else {
                Some(PathBuf::from(raw))
            }
        })
        .collect()
}

/// Whether the file at `path` is one the app opens.
fn is_document(path: &Path) -> bool {
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    head(path).is_some_and(|head| {
        formats::is_document(&head, extension.as_deref())
            || ExtraFormat::detect(&head, extension.as_deref()).is_some()
            || (extension.as_deref() == Some("expdf") && head.starts_with(b"PK\x03\x04"))
    })
}

/// The first kilobyte of `path`.
fn head(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read;
    let mut head = Vec::with_capacity(1024);
    std::fs::File::open(path)
        .ok()?
        .take(1024)
        .read_to_end(&mut head)
        .ok()?;
    Some(head)
}

/// The launch the window opens with.
static STARTUP: OnceLock<Launch> = OnceLock::new();

/// The launch the window opens with: this process's command line.
pub(crate) fn startup() -> Option<&'static Launch> {
    STARTUP.get()
}

/// How the process goes on after [`start`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Started {
    /// The app was running: it got this launch's files, and this process ends.
    Forwarded,
    /// This is the app's first instance: it opens its window.
    First,
}

/// Reads the command line and hands it to the running instance, if there is one; otherwise this process becomes the
/// one later launches hand theirs to.
///
/// Setting `ECCLESIAPDF_ISOLATED_INSTANCE=1` bypasses single-instance forwarding (used by isolated
/// automated UI/budget tests so they never collide with a user's open session).
pub fn start() -> Started {
    let cwd = std::env::current_dir().unwrap_or_default();
    let launch = parse(std::env::args_os().skip(1), &cwd);
    if std::env::var_os("ECCLESIAPDF_ISOLATED_INSTANCE").is_some_and(|v| !v.is_empty() && v != "0")
    {
        let _ = STARTUP.set(launch);
        return Started::First;
    }
    match instance::claim(crate::APP_ID, &to_message(&launch)) {
        Ok(Claim::Forwarded) => return Started::Forwarded,
        Ok(Claim::Primary(listener)) => {
            let served = listener.serve(|message| match from_message(&message) {
                Some(launch) => deliver(launch),
                None => log::warn!("single instance: a message that is no launch"),
            });
            if let Err(err) = served {
                log::warn!("single instance: can't listen: {err}");
            }
        }
        // Not fatal: the app runs, and later launches open windows of their own.
        Err(err) => log::warn!("single instance: {err}"),
    }
    let _ = STARTUP.set(launch);
    Started::First
}

/// Later launches waiting to be shown, and the task that shows them.
#[derive(Default)]
struct Inbox {
    launches: VecDeque<Launch>,
    waker: Option<Waker>,
}

static INBOX: Mutex<Inbox> = Mutex::new(Inbox {
    launches: VecDeque::new(),
    waker: None,
});

/// Queues a later launch (from the single-instance thread) and wakes the task that shows it.
fn deliver(launch: Launch) {
    let mut inbox = INBOX.lock().expect("inbox lock poisoned");
    inbox.launches.push_back(launch);
    if let Some(waker) = inbox.waker.take() {
        waker.wake();
    }
}

/// The next later launch, awaited on the engine thread.
pub(crate) fn next() -> impl Future<Output = Launch> {
    Next
}

struct Next;

impl Future for Next {
    type Output = Launch;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Launch> {
        let mut inbox = INBOX.lock().expect("inbox lock poisoned");
        match inbox.launches.pop_front() {
            Some(launch) => Poll::Ready(launch),
            None => {
                inbox.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

/// Offers the app for its document types and opens the system's default-apps settings, where the user makes it the
/// default viewer (Windows lets no app do that itself). Runs on a thread of its own.
pub(crate) fn set_as_default_viewer() {
    #[cfg(windows)]
    {
        let description = nsh_core::t!("associations.description");
        let spawned = std::thread::Builder::new()
            .name("associations".to_owned())
            .spawn(move || {
                use ecclesia_platform::associations;
                let exe = match std::env::current_exe() {
                    Ok(exe) => exe,
                    Err(err) => return log::warn!("associations: where is the app? {err}"),
                };
                if let Err(err) = associations::register(&exe, formats::EXTENSIONS, &description) {
                    log::warn!("associations: {err}");
                }
                if let Err(err) = associations::open_default_apps() {
                    log::warn!("associations: can't open the settings: {err}");
                }
            });
        if let Err(err) = spawned {
            log::warn!("associations: {err}");
        }
    }
    // shortcut: macOS and Linux register file types in the app bundle and the .desktop file (Phase 2).
    #[cfg(not(windows))]
    log::info!("associations: set by the package on this system");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<OsString> {
        list.iter().map(OsString::from).collect()
    }

    #[test]
    fn the_command_line_gives_absolute_files_and_a_page() {
        let cwd = std::env::temp_dir();
        let launch = parse(args(&["a.pdf", "--page", "3", "--zoom", "sub/b.pdf"]), &cwd);
        assert_eq!(launch.files, [cwd.join("a.pdf"), cwd.join("sub/b.pdf")]);
        assert_eq!(launch.page, Some(2));
        assert_eq!(parse(args(&["--page=1"]), &cwd).page, Some(0));
        // Page 0 or no number: no page.
        assert_eq!(parse(args(&["--page", "0"]), &cwd).page, None);
        assert_eq!(parse(args(&["--page", "x"]), &cwd).page, None);
        assert_eq!(launch.split, None);
        assert_eq!(
            parse(args(&["--split", "a.pdf", "b.pdf"]), &cwd).split,
            Some(Split::Files)
        );
        assert_eq!(
            parse(args(&["a.pdf", "--split-same"]), &cwd).split,
            Some(Split::Same)
        );
    }

    #[test]
    fn a_launch_crosses_the_channel_unchanged() {
        let launch = Launch {
            files: vec![std::env::temp_dir().join("a b.pdf")],
            page: Some(4),
            split: Some(Split::Same),
        };
        assert_eq!(from_message(&to_message(&launch)), Some(launch));
        let empty = Launch::default();
        assert_eq!(from_message(&to_message(&empty)), Some(empty));
        // Not a launch: refused.
        assert_eq!(from_message(b"hello"), None);
        assert_eq!(from_message(br#"{"files":["relative.pdf"]}"#), None);
        assert_eq!(from_message(br#"{"files":[],"page":-1}"#), None);
        assert_eq!(from_message(br#"{"files":[],"split":"diagonal"}"#), None);
    }

    #[test]
    fn dropped_files_are_taken_by_their_contents_not_their_names() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
        let uri = |name: &str| {
            let path = corpus.join(name).canonicalize().expect("corpus file");
            format!(
                "file:///{}",
                path.to_string_lossy()
                    .trim_start_matches(r"\\?\")
                    .replace('\\', "/")
            )
        };
        let dropped = dropped_documents(&[
            uri("formats.epub"),
            uri("formats.md"),
            uri("README.md"),
            uri("zlib-manual.pdf"),
        ]);
        let names: Vec<_> = dropped
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        // The corpus README is Markdown too: a dropped .md file is taken when it is text.
        assert_eq!(
            names,
            ["formats.epub", "formats.md", "README.md", "zlib-manual.pdf"]
        );
        // A folder isn't.
        assert!(dropped_documents(&[format!("file:///{}", corpus.to_string_lossy())]).is_empty());
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("ecclesia-open-test-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("create temp test dir");
            Self(path)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn to_file_uri(path: &Path) -> String {
        let normalized = path
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .replace('\\', "/")
            .replace(' ', "%20");
        format!("file:///{}", normalized.trim_start_matches('/'))
    }

    #[test]
    fn paths_from_event_handles_plain_paths_and_file_uris() {
        let payload = json!({
            "paths": [
                r"C:\My Docs\report #1.pdf",
                "file:///C:/My%20Docs/spaced%20file.pdf",
                "FILE:///C:/Other%20Dir/book.epub",
                "",
                42,
                null
            ]
        });
        assert_eq!(
            paths_from_event(&payload),
            vec![
                PathBuf::from(r"C:\My Docs\report #1.pdf"),
                PathBuf::from("C:/My Docs/spaced file.pdf"),
                PathBuf::from("C:/Other Dir/book.epub"),
            ]
        );
        assert!(paths_from_event(&json!({})).is_empty());
        assert!(paths_from_event(&json!({ "paths": "not-an-array" })).is_empty());
    }

    #[test]
    fn dropped_paths_and_documents_filter_and_deduplicate_temp_files() {
        let dir = TempDir::new("drop-filter");
        let pdf = dir.0.join("my sample.pdf");
        let renamed = dir.0.join("renamed.dat");
        let text = dir.0.join("notes.txt");
        let fake_pdf = dir.0.join("fake.pdf");
        let subdir = dir.0.join("nested");

        std::fs::write(&pdf, b"%PDF-1.4\n").expect("write pdf");
        std::fs::write(&renamed, b"%PDF-1.4\n").expect("write renamed pdf");
        std::fs::write(&text, b"plain text, not a supported document\n").expect("write text");
        std::fs::write(&fake_pdf, b"not actually a pdf header\n").expect("write fake pdf");
        std::fs::create_dir(&subdir).expect("create subdir");

        let filtered = dropped_paths(&[
            pdf.clone(),
            text.clone(),
            fake_pdf.clone(),
            subdir.clone(),
            pdf.clone(),
            renamed.clone(),
            dir.0.join("missing.pdf"),
        ]);
        assert_eq!(filtered, vec![pdf.clone(), renamed.clone()]);

        let uris = vec![
            to_file_uri(&pdf),
            to_file_uri(&text),
            to_file_uri(&fake_pdf),
            to_file_uri(&subdir),
            to_file_uri(&pdf),
            to_file_uri(&renamed),
        ];
        let from_uris = dropped_documents(&uris);
        let names: Vec<_> = from_uris
            .iter()
            .map(|p| {
                p.file_name()
                    .expect("file name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        assert_eq!(names, ["my sample.pdf", "renamed.dat"]);
    }
}
