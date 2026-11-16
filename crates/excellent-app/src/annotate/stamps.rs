//! The stamp picker (A.20): while the Stamp command waits for a click, the inspector offers the standard stamps, the
//! dynamic ones (filled with the reader's name, the date and time, and the page when placed) and the reader's own
//! library, with Import and Remove. The click then places the chosen one, its art and all, as one step of undo. The
//! model is `excellent-features`' `annotate::stamps`.

use std::io;
use std::path::PathBuf;

use excellent_core::annot::{Annotation, now};
use excellent_features::annotate::stamps::{self, Dynamic, Fields, Item, Standard};
use excellent_mupdf::{PagePoint, PageRect, StampArt};
use nsh_base::theme;
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use super::AnnotateState;
use crate::bookmarks::caption;
use crate::commands::{CommandId, shortcut_label};
use crate::editing::show_message;
use crate::settings_window::{segment, segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// The stamp the next click places.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Choice {
    /// One the PDF standard names, drawn from its name.
    Standard(Standard),
    /// A title with who placed it and when.
    Dynamic(Dynamic),
    /// One from the reader's library.
    Library(Item),
}

impl Default for Choice {
    fn default() -> Self {
        Self::Standard(Standard::Approved)
    }
}

/// The library folder: `stamps` in the app's data folder.
fn folder() -> Option<PathBuf> {
    crate::settings::folder().map(|folder| folder.join("stamps"))
}

/// The stamp `choice` on page `page`, centred on the click `at` inside the page (`bounds`), by `author`, with what it
/// is drawn as when that isn't its name.
///
/// # Errors
///
/// A library stamp's file can't be read, or MuPDF can't lay out a dynamic one.
pub(super) fn made(
    choice: &Choice,
    page: usize,
    at: PagePoint,
    bounds: PageRect,
    author: &str,
) -> io::Result<(Annotation, Option<StampArt>)> {
    Ok(match choice {
        Choice::Standard(standard) => (
            stamps::place(
                page,
                at,
                bounds,
                stamps::STANDARD_SIZE,
                standard.name(),
                standard.color(),
                author,
            ),
            None,
        ),
        Choice::Dynamic(dynamic) => {
            let fields = Fields {
                user: author.to_owned(),
                seconds: now(),
                page: page + 1,
            };
            let detail = fields.fill(&t!("stamps.detail"));
            let (art, size) =
                stamps::dynamic_art(&t!(dynamic.label_key()), &detail, dynamic.color())?;
            (
                stamps::place(
                    page,
                    at,
                    bounds,
                    size,
                    dynamic.name(),
                    dynamic.color(),
                    author,
                ),
                Some(art),
            )
        }
        Choice::Library(item) => {
            let (art, size) = item.art()?;
            (
                stamps::place(
                    page,
                    at,
                    bounds,
                    size,
                    &item.pdf_name(),
                    stamps::LIBRARY_COLOR,
                    author,
                ),
                Some(art),
            )
        }
    })
}

impl AnnotateState {
    /// Reads the library folder again.
    pub(super) fn load_stamps(self) {
        self.stamps.set(
            folder()
                .map(|folder| stamps::library(&folder))
                .unwrap_or_default(),
        );
    }

    /// Asks for a PDF or an image and adds it to the library, chosen.
    fn import_stamp(self) {
        let Some(folder) = folder() else {
            return;
        };
        let dialog = OpenDialog::files()
            .title(t!("stamps.import_pick"))
            .filter(FileFilter::new(
                t!("stamps.import_filter"),
                ["pdf", "png", "jpg", "jpeg"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            let source = match answer.await {
                Ok(Some(paths)) => paths.into_iter().next(),
                Ok(None) => None,
                Err(err) => {
                    log::warn!("stamps: the dialog failed: {err}");
                    None
                }
            };
            let Some(source) = source else {
                return;
            };
            match stamps::import(&folder, &source) {
                Ok(item) => {
                    self.load_stamps();
                    self.stamp.set(Choice::Library(item));
                }
                Err(err) => show_message(t!("stamps.import_failed"), err.to_string()),
            }
        });
    }

    /// Takes the chosen library stamp out of the library (the file it was imported from stays).
    fn remove_stamp(self) {
        let Choice::Library(item) = untracked(|| self.stamp.get()) else {
            return;
        };
        if let Err(err) = stamps::remove(&item) {
            show_message(t!("stamps.remove_failed"), err.to_string());
        }
        self.stamp.set(Choice::default());
        self.load_stamps();
    }
}

/// The inspector while a stamp is being placed: what to click, then the stamps to choose from.
pub(super) fn picker(state: AnnotateState) -> View {
    let standard = Standard::ALL
        .into_iter()
        .fold(segmented(), |group, standard| {
            group.child(segment(
                standard.label_key(),
                move || {
                    state
                        .stamp
                        .with(|stamp| *stamp == Choice::Standard(standard))
                },
                move || state.stamp.set(Choice::Standard(standard)),
            ))
        });
    let dynamic_row = Dynamic::ALL.into_iter().fold(segmented(), |group, made| {
        group.child(segment(
            made.label_key(),
            move || state.stamp.with(|stamp| *stamp == Choice::Dynamic(made)),
            move || state.stamp.set(Choice::Dynamic(made)),
        ))
    });
    let actions = view()
        .flex_row()
        .gap_2()
        .child(action_button("stamps.import", Tone::Plain, move || {
            state.import_stamp();
        }))
        .child(when(
            move || {
                state
                    .stamp
                    .with(|stamp| matches!(stamp, Choice::Library(_)))
            },
            move || {
                action_button("stamps.remove", Tone::Destructive, move || {
                    state.remove_stamp();
                })
            },
        ));
    view()
        .flex_1()
        .p_4()
        .gap_3()
        .child(
            text(move || t!("stamps.placing"))
                .text_size(px(14.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().foreground),
        )
        .child(
            text(move || {
                t!(
                    "stamps.placing_hint",
                    key = shortcut_label(CommandId::Stamp)
                )
            })
            .text_size(px(13.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(caption("stamps.standard"))
        .child(standard)
        .child(caption("stamps.dynamic"))
        .child(dynamic_row)
        .child(caption("stamps.library"))
        .child(library(state))
        .child(actions)
}

/// The library's stamps to choose from, or a word on how to add one.
fn library(state: AnnotateState) -> impl IntoElement {
    dynamic(move || {
        let count = state.stamps.with(Vec::len);
        if count == 0 {
            return text(move || t!("stamps.library_empty"))
                .text_size(px(13.0))
                .text_color(move || theme().muted_foreground)
                .into_element();
        }
        (0..count)
            .fold(segmented(), |group, at| {
                let item = move || state.stamps.with(|items| items.get(at).cloned());
                group.child(segment_of(
                    move || item().map(|item| item.name).unwrap_or_default(),
                    move || {
                        item().is_some_and(|item| {
                            state.stamp.with(|stamp| *stamp == Choice::Library(item))
                        })
                    },
                    move || {
                        if let Some(item) = item() {
                            state.stamp.set(Choice::Library(item));
                        }
                    },
                ))
            })
            .into_element()
    })
}
