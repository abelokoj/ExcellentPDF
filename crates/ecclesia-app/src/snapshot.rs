//! Region snapshots (O.23), the app's side: the Snapshot command turns snapshot mode on (the canvas's `snapshot`
//! prop); the region the reader then drags (`snapshot`) opens a dialog with the resolution and the picture's size,
//! to copy it as an image or save it as PNG. Rendering runs on a worker. The model is `ecclesia-features`'
//! `snapshot`.

use base64::Engine;
use ecclesia_features::snapshot::{DEFAULT_DPI, RESOLUTIONS, Region};
use nsh_base::theme;
use nsh_core::JsonValue;
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use crate::bookmarks::caption;
use crate::editing::{Editing, show_message};
use crate::list_dialog::{card, list_dialog};
use crate::settings::write_atomically;
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// A pane's snapshots: whether the mode is on, the region taken, and the resolution chosen.
#[derive(Clone, Copy)]
pub(crate) struct SnapshotState {
    editing: Editing,
    on: Signal<bool>,
    taken: Signal<Option<Region>>,
    dpi: Signal<u32>,
}

impl SnapshotState {
    /// Snapshots of the document `editing` shows.
    pub(crate) fn new(editing: Editing) -> Self {
        Self {
            editing,
            on: signal(false),
            taken: signal(None),
            dpi: signal(DEFAULT_DPI),
        }
    }

    /// Turns snapshot mode on, or off again.
    pub(crate) fn toggle(self) {
        self.on.update(|on| *on = !*on);
    }

    /// Whether snapshot mode is on, for the toolbar.
    pub(crate) fn is_on(self) -> bool {
        self.on.get()
    }

    /// The canvas's `snapshot` prop.
    pub(crate) fn prop(self) -> JsonValue {
        JsonValue::from(self.on.get())
    }

    /// The canvas reported a region (`snapshot`): the mode ends and the dialog opens on it.
    pub(crate) fn taken(self, data: &JsonValue) {
        let Some(region) = Region::parse(data) else {
            log::warn!("snapshot: a region that doesn't parse: {data}");
            return;
        };
        self.on.set(false);
        self.taken.set(Some(region));
    }

    /// Renders the region at the chosen resolution on a worker, then hands the PNG to `done`.
    fn render(self, region: Region, done: impl FnOnce(Vec<u8>) + 'static) {
        let Some(document) = self.editing.document() else {
            return;
        };
        let dpi = untracked(|| self.dpi.get());
        spawn(async move {
            let rendered = match document.mupdf() {
                Ok(shared) => blocking(move || {
                    let mupdf = shared.lock().expect("document lock poisoned");
                    region
                        .render_png(&mupdf, dpi)
                        .map_err(|err| err.to_string())
                })
                .await
                .map_err(|err| err.to_string())
                .and_then(|png| png),
                Err(err) => Err(err.to_string()),
            };
            match rendered {
                Ok(png) => done(png),
                Err(err) => {
                    log::warn!("snapshot: {err}");
                    show_message(t!("snapshot.failed"), err);
                }
            }
        });
    }

    /// Copies the region as an image.
    fn copy(self, region: Region) {
        self.render(region, |png| {
            let encoded = base64::engine::general_purpose::STANDARD.encode(png);
            set_clipboard_image(format!("data:image/png;base64,{encoded}"));
        });
    }

    /// Asks where to save the region, and saves it there as PNG.
    fn save(self, region: Region) {
        let stem = self
            .editing
            .document()
            .and_then(|d| {
                d.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .unwrap_or_default();
        let dialog = SaveDialog::new()
            .file_name(region.file_name(&stem))
            .filter(FileFilter::new(t!("snapshot.filter"), ["png"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            self.render(region, move |png| {
                if let Err(err) = write_atomically(&path, png) {
                    log::warn!("snapshot: saving: {err}");
                    show_message(t!("snapshot.failed"), err.to_string());
                }
            });
        });
    }
}

/// The dialog for a region taken: its resolution, the picture's size, Copy and Save PNG.
pub(crate) fn snapshot_dialog(state: SnapshotState) -> impl IntoElement {
    when(
        move || state.taken.get().is_some(),
        move || {
            let region = untracked(|| state.taken.get()).expect("open on a region");
            let close = move || state.taken.set(None);
            let resolutions = RESOLUTIONS.into_iter().fold(segmented(), |group, dpi| {
                group.child(segment_of(
                    move || t!("snapshot.dpi", dpi = dpi.to_string()),
                    move || state.dpi.get() == dpi,
                    move || state.dpi.set(dpi),
                ))
            });
            let size = move || {
                let (w, h) = region.pixels(state.dpi.get());
                t!(
                    "snapshot.size",
                    width = w.to_string(),
                    height = h.to_string()
                )
            };
            let content = view()
                .gap_3()
                .p_4()
                .w(px(420.0))
                .child(
                    text(move || t!("snapshot.title", page = (region.page + 1).to_string()))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(caption("snapshot.resolution"))
                .child(resolutions)
                .child(
                    text(size)
                        .text_size(px(12.0))
                        .tabular_nums()
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    view()
                        .flex_row()
                        .justify_end()
                        .gap_2()
                        .child(action_button("snapshot.cancel", Tone::Plain, close))
                        .child(action_button("snapshot.save", Tone::Plain, move || {
                            close();
                            state.save(region);
                        }))
                        .child(action_button("snapshot.copy", Tone::Plain, move || {
                            close();
                            state.copy(region);
                        })),
                );
            list_dialog(t!("snapshot.title_plain"), close, card(content))
        },
    )
}
