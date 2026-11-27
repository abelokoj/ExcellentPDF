//! Attachments (O.25), the app's side: the Attachments command opens a dialog listing the PDF's embedded files and
//! file attachment annotations, each to save or, after asking, to open; all of them to save into a folder; and a file
//! to embed. Names are made safe before anything is written, and a program is never opened (plan §14.2). Reading and
//! writing run on workers. The model is `ecclesia-features`' `attachments`.

use std::path::Path;

use ecclesia_core::edit::{Change, Operation};
use ecclesia_features::attachments::{can_open, safe_file_name, target_in};
use ecclesia_mupdf::Attachment;
use nsh_base::{alert_dialog, theme};
use nsh_core::file_dialog::{OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use crate::editing::{Editing, show_message};
use crate::list_dialog::{card, list_dialog};
use crate::settings::write_atomically;
use crate::tabs::{Tone, action_button};

/// The folder attachments are written to before they open: the system's temporary folder, in a folder of ours.
const OPEN_FOLDER: &str = "EcclesiaPDF attachments";

/// A pane's attachments: whether the dialog is open, and what it lists (`None` while it is read).
#[derive(Clone, Copy)]
pub(crate) struct AttachmentState {
    editing: Editing,
    open: Signal<bool>,
    listed: Signal<Option<Vec<Attachment>>>,
}

impl AttachmentState {
    /// The attachments of the document `editing` shows.
    pub(crate) fn new(editing: Editing) -> Self {
        Self {
            editing,
            open: signal(false),
            listed: signal(None),
        }
    }

    /// Opens the dialog and reads the list.
    pub(crate) fn show(self) {
        self.open.set(true);
        self.reload();
    }

    /// Reads the list again, on a worker.
    fn reload(self) {
        self.listed.set(None);
        let Some(document) = self.editing.document() else {
            self.listed.set(Some(Vec::new()));
            return;
        };
        spawn(async move {
            let listed = match document.mupdf() {
                Ok(shared) => blocking(move || {
                    let mupdf = shared.lock().expect("document lock poisoned");
                    mupdf.attachments().map_err(|err| err.to_string())
                })
                .await
                .map_err(|err| err.to_string())
                .and_then(|list| list),
                Err(err) => Err(err.to_string()),
            };
            match listed {
                Ok(list) => self.listed.set(Some(list)),
                Err(err) => {
                    log::warn!("attachments: {err}");
                    self.listed.set(Some(Vec::new()));
                }
            }
        });
    }

    /// Reads the bytes of attachments `indexes` on a worker, then hands them over in order.
    fn read(self, indexes: Vec<usize>, done: impl FnOnce(Vec<Vec<u8>>) + 'static) {
        let Some(document) = self.editing.document() else {
            return;
        };
        spawn(async move {
            let read = match document.mupdf() {
                Ok(shared) => blocking(move || {
                    let mupdf = shared.lock().expect("document lock poisoned");
                    indexes
                        .iter()
                        .map(|&i| mupdf.attachment_contents(i).map_err(|err| err.to_string()))
                        .collect::<Result<Vec<_>, String>>()
                })
                .await
                .map_err(|err| err.to_string())
                .and_then(|bytes| bytes),
                Err(err) => Err(err.to_string()),
            };
            match read {
                Ok(bytes) => done(bytes),
                Err(err) => show_message(t!("attachments.failed"), err),
            }
        });
    }

    /// Asks where to save `attachment`, offering its safe name, and saves it there.
    fn save(self, attachment: Attachment) {
        let dialog = SaveDialog::new().file_name(safe_file_name(&attachment.name));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            self.read(vec![attachment.index], move |mut bytes| {
                write(&path, bytes.pop().unwrap_or_default());
            });
        });
    }

    /// Asks for a folder and saves every attachment into it, each under its safe name, none over another file.
    fn save_all(self, all: Vec<Attachment>) {
        let answer = pick_paths(OpenDialog::folders().title(t!("attachments.pick_folder")));
        spawn(async move {
            let Some(folder) = answer
                .await
                .ok()
                .flatten()
                .and_then(|p| p.into_iter().next())
            else {
                return;
            };
            self.read(all.iter().map(|a| a.index).collect(), move |bytes| {
                for (attachment, bytes) in all.iter().zip(bytes) {
                    write(&target_in(&folder, &attachment.name, |p| p.exists()), bytes);
                }
            });
        });
    }

    /// Asks before opening `attachment` in the app the system gives its kind; a program is only ever saved.
    fn open_file(self, attachment: Attachment) {
        if !can_open(&attachment.name) {
            show_message(
                t!("attachments.not_opened"),
                t!("attachments.not_opened_detail"),
            );
            return;
        }
        let name = safe_file_name(&attachment.name);
        let answer = alert_dialog()
            .title(t!("attachments.open_title", name = name.clone()))
            .description(t!("attachments.open_detail"))
            .cancel("cancel", t!("attachments.cancel"))
            .action("open", t!("attachments.open"))
            .close_on_backdrop_press(true)
            .close_on_escape(true)
            .show();
        spawn(async move {
            if !answer
                .await
                .is_ok_and(|a| a.action.as_deref() == Some("open"))
            {
                return;
            }
            self.read(vec![attachment.index], move |mut bytes| {
                let folder = std::env::temp_dir().join(OPEN_FOLDER);
                if let Err(err) = std::fs::create_dir_all(&folder) {
                    show_message(t!("attachments.failed"), err.to_string());
                    return;
                }
                let path = target_in(&folder, &name, |p| p.exists());
                if write(&path, bytes.pop().unwrap_or_default()) {
                    drop(open_url(file_url(&path)));
                }
            });
        });
    }

    /// Asks for a file and embeds it in the PDF, one step of undo.
    fn add(self) {
        let answer = pick_paths(OpenDialog::files().title(t!("attachments.pick_file")));
        spawn(async move {
            let Some(path) = answer
                .await
                .ok()
                .flatten()
                .and_then(|p| p.into_iter().next())
            else {
                return;
            };
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let read = blocking(move || std::fs::read(&path)).await;
            match read {
                Ok(Ok(bytes)) => {
                    if self.editing.apply(&Operation::new(
                        t!("attachments.add_edit"),
                        vec![Change::AddAttachment { name, bytes }],
                    )) {
                        self.reload();
                    }
                }
                Ok(Err(err)) => show_message(t!("attachments.failed"), err.to_string()),
                Err(err) => show_message(t!("attachments.failed"), err.to_string()),
            }
        });
    }
}

/// Writes `bytes` to `path` atomically; says so when it can't. Returns whether it did.
fn write(path: &Path, bytes: Vec<u8>) -> bool {
    match write_atomically(path, bytes) {
        Ok(()) => true,
        Err(err) => {
            log::warn!("attachments: writing {}: {err}", path.display());
            show_message(t!("attachments.failed"), err.to_string());
            false
        }
    }
}

/// `path` as a `file:` URL.
fn file_url(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let encoded: String = text
        .chars()
        .map(|c| match c {
            ' ' => "%20".to_owned(),
            '#' => "%23".to_owned(),
            '%' => "%25".to_owned(),
            other => other.to_string(),
        })
        .collect();
    if encoded.starts_with('/') {
        format!("file://{encoded}")
    } else {
        format!("file:///{encoded}")
    }
}

/// The dialog: each attachment with its size and where it is, Save and Open; Save all, Add file and Close.
pub(crate) fn attachments_dialog(state: AttachmentState) -> impl IntoElement {
    when(
        move || state.open.get(),
        move || {
            let close = move || state.open.set(false);
            let rows = dynamic(move || {
                let listed = state.listed.get();
                untracked(move || match listed {
                    None => message(t!("attachments.reading")).into_element(),
                    Some(list) if list.is_empty() => message(t!("attachments.none")).into_element(),
                    Some(list) => list
                        .into_iter()
                        .fold(view().gap_2(), |rows, attachment| {
                            rows.child(row(state, attachment))
                        })
                        .into_element(),
                })
            });
            let editable = state.editing.document().is_some_and(|d| d.is_editable());
            let mut footer = view().flex_row().justify_end().gap_2();
            if editable {
                footer = footer.child(action_button("attachments.add", Tone::Plain, move || {
                    state.add()
                }));
            }
            footer = footer
                .child(action_button(
                    "attachments.save_all",
                    Tone::Plain,
                    move || {
                        if let Some(all) =
                            untracked(|| state.listed.get()).filter(|l| !l.is_empty())
                        {
                            state.save_all(all);
                        }
                    },
                ))
                .child(action_button("attachments.close", Tone::Plain, close));
            let content = view()
                .gap_3()
                .p_4()
                .w(px(520.0))
                .child(
                    text(move || t!("attachments.title"))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(scroll().max_h(px(360.0)).child(rows))
                .child(footer);
            list_dialog(t!("attachments.title"), close, card(content))
        },
    )
}

/// One attachment: its safe name, size and place, then Save and, for a kind that may open, Open.
fn row(state: AttachmentState, attachment: Attachment) -> View {
    let name = safe_file_name(&attachment.name);
    let size = attachment
        .size
        .map(|bytes| {
            t!(
                "attachments.size",
                kb = format!("{:.1}", bytes as f64 / 1024.0)
            )
        })
        .unwrap_or_default();
    let place = match attachment.page {
        Some(page) => t!("attachments.on_page", page = (page + 1).to_string()),
        None => t!("attachments.in_document"),
    };
    let saving = attachment.clone();
    let opening = attachment.clone();
    let mut row = view()
        .flex_row()
        .items_center()
        .gap_2()
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .child(
                    text(name)
                        .truncate()
                        .text_size(px(13.0))
                        .text_color(move || theme().popover_foreground),
                )
                .child(
                    text(format!("{place}; {size}"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(action_button("attachments.save", Tone::Plain, move || {
            state.save(saving.clone())
        }));
    if can_open(&attachment.name) {
        row = row.child(action_button("attachments.open", Tone::Plain, move || {
            state.open_file(opening.clone());
        }));
    }
    row
}

/// A line of muted text in the list.
fn message(line: String) -> View {
    view().py_2().child(
        text(line)
            .text_size(px(13.0))
            .text_color(move || theme().muted_foreground),
    )
}
