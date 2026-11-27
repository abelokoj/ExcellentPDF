//! Notebooks, Lecture & Authoring UI (N.1–N.10, N.14, U.8, L.1, L.3):
//! - New `.expdf` notebook with procedural paper templates (N.1, N.4)
//! - Insert page before/after & Expand/Restore margins (N.2, N.3)
//! - Export `.expdf` notebook to standalone PDF (N.4)
//! - Infinite canvas whiteboard & export (N.5)
//! - Typed text blocks with U.8 IME composition bridge (N.6, U.8)
//! - Insert image/sticker media as PDF stamp art (N.7)
//! - Notebook covers & colored sections organizer (N.8)
//! - Page links & `ecclesiapdf://` deep links manager (N.9)
//! - Quick Note `Inbox.expdf` launcher (N.10)
//! - Markdown editor, heading TOC & KaTeX/Mermaid WebView/MuPDF preview (N.14)
//! - Lecture audio recording & ink-stroke timeline sync (L.1)
//! - Import slides & condense 1/2/3 slides per note page (L.3)

pub(crate) mod phase5;

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use ecclesia_core::notebook::{
    AttachAudio, CoverStyle, ExportOptions, Notebook, Orientation, PageSize, PaperTemplateId,
    pdf_ops,
};
use ecclesia_features::audio::{
    AudioBundleSpec, AudioCapture, AudioPlayback, AudioTimeline, CaptureConfig,
    PlaybackState as AudioPlayState, bundle_audio_in_notebook, encode_wav,
};
use ecclesia_features::markdown::{
    PreviewBackend, PreviewConfig, PreviewController, RenderOptions, extract_toc, render_gfm_html,
};
use ecclesia_features::notebook::ime::{ImeBridge, ImeEvent};
use ecclesia_features::notebook::infinite::{
    CanvasPoint, CanvasStroke, InfiniteCanvas, InfiniteExportMode,
};
use ecclesia_features::notebook::links::{DeepLink, add_deep_link, add_internal_link};
use ecclesia_features::notebook::media::{CropRect, PageMedia};
use ecclesia_features::notebook::pages::{
    InsertPageSource, InsertPosition, MarginExpansion, expand_page_margins_notebook,
    expand_page_margins_pdf, insert_page_in_notebook, insert_page_in_pdf,
    remove_added_margins_notebook, remove_added_margins_pdf,
};
use ecclesia_features::notebook::paper::PaperSpec;
use ecclesia_features::notebook::quicknote::{QuickNoteConfig, QuickNoteInbox};
use ecclesia_features::notebook::sections::{add_section, set_cover, tab_strip};
use ecclesia_features::notebook::slides::{
    CondenseNoteStyle, CondenseOptions, SlideNumbers, SlidesPerPage, condense_slides_to_notebook,
};
use ecclesia_features::notebook::textblocks::{BlockKind, ListStyle, TextBlock};
use ecclesia_mupdf::{PagePoint, PageRect};
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use crate::annotate::AnnotateState;
use crate::bookmarks::{caption, field};
use crate::editing::{Editing, show_message};
use crate::expdf;
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, Live, write_atomically};
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// UI state and dialogs for Notebooks, Markdown, Whiteboards, and Lecture tools.
#[derive(Clone, Copy)]
pub(crate) struct NotebooksState {
    editing: Editing,
    annotate: AnnotateState,
    new_open: Signal<bool>,
    insert_page_open: Signal<bool>,
    margins_open: Signal<bool>,
    infinite_open: Signal<bool>,
    text_block_open: Signal<bool>,
    sections_open: Signal<bool>,
    links_open: Signal<bool>,
    markdown_open: Signal<bool>,
    audio_open: Signal<bool>,
    condense_open: Signal<bool>,
    planner_open: Signal<bool>,
    packs_open: Signal<bool>,
    versions_open: Signal<bool>,
    transcript_open: Signal<bool>,
    audio_timeline: Stored<Option<AudioTimeline>>,
    audio_capture: Stored<Option<Box<dyn AudioCapture>>>,
    audio_playback: Stored<Option<Box<dyn AudioPlayback>>>,
    audio_recording: Signal<bool>,
    audio_playing: Signal<bool>,
    audio_status: Signal<String>,
    infinite_canvas: Stored<InfiniteCanvas>,
    quick_note: Stored<QuickNoteInbox>,
}

impl NotebooksState {
    /// Creates per-pane notebook and lecture state.
    pub(crate) fn new(editing: Editing, annotate: AnnotateState, _live: Live) -> Self {
        let inbox = settings::folder()
            .unwrap_or_else(|| std::env::temp_dir().join("ecclesiapdf-inbox"))
            .join("Inbox.expdf");
        Self {
            editing,
            annotate,
            new_open: signal(false),
            insert_page_open: signal(false),
            margins_open: signal(false),
            infinite_open: signal(false),
            text_block_open: signal(false),
            sections_open: signal(false),
            links_open: signal(false),
            markdown_open: signal(false),
            audio_open: signal(false),
            condense_open: signal(false),
            planner_open: signal(false),
            packs_open: signal(false),
            versions_open: signal(false),
            transcript_open: signal(false),
            audio_timeline: stored(None),
            audio_capture: stored(None),
            audio_playback: stored(None),
            audio_recording: signal(false),
            audio_playing: signal(false),
            audio_status: signal(String::new()),
            infinite_canvas: stored(InfiniteCanvas::new()),
            quick_note: stored(QuickNoteInbox::new(QuickNoteConfig::with_inbox_path(inbox))),
        }
    }

    /// Whether any notebook/lecture dialog is currently open (hides native canvas HWND cleanly).
    pub(crate) fn any_dialog_open(self) -> bool {
        self.new_open.get()
            || self.insert_page_open.get()
            || self.margins_open.get()
            || self.infinite_open.get()
            || self.text_block_open.get()
            || self.sections_open.get()
            || self.links_open.get()
            || self.markdown_open.get()
            || self.audio_open.get()
            || self.condense_open.get()
            || self.planner_open.get()
            || self.packs_open.get()
            || self.versions_open.get()
            || self.transcript_open.get()
    }

    pub(crate) fn is_new_open(self) -> bool {
        self.new_open.get()
    }

    pub(crate) fn is_insert_page_open(self) -> bool {
        self.insert_page_open.get()
    }

    pub(crate) fn is_margins_open(self) -> bool {
        self.margins_open.get()
    }

    pub(crate) fn is_infinite_open(self) -> bool {
        self.infinite_open.get()
    }

    pub(crate) fn is_text_block_open(self) -> bool {
        self.text_block_open.get()
    }

    pub(crate) fn is_sections_open(self) -> bool {
        self.sections_open.get()
    }

    pub(crate) fn is_links_open(self) -> bool {
        self.links_open.get()
    }

    pub(crate) fn is_markdown_open(self) -> bool {
        self.markdown_open.get()
    }

    pub(crate) fn is_audio_open(self) -> bool {
        self.audio_open.get()
    }

    pub(crate) fn is_condense_open(self) -> bool {
        self.condense_open.get()
    }

    /// Opens the New `.expdf` Notebook dialog (N.1, N.4).
    pub(crate) fn open_new_notebook(self) {
        self.new_open.set(true);
    }

    /// Opens the Insert Page dialog (N.2).
    pub(crate) fn open_insert_page(self) {
        self.insert_page_open.set(true);
    }

    /// Opens the Expand / Remove Margins dialog (N.3).
    pub(crate) fn open_margins(self) {
        self.margins_open.set(true);
    }

    /// Exports the active `.expdf` notebook (with baked procedural paper templates and bookmarks) to PDF (N.4).
    pub(crate) fn export_notebook_pdf(self) {
        let Some(doc) = self.editing.document() else {
            return;
        };
        let working_path = doc.path().to_path_buf();
        let expdf_path = expdf::expdf_for_working_path(&working_path);
        let nb = if let Some(path) = expdf_path {
            match Notebook::open(&path) {
                Ok(nb) => nb,
                Err(err) => {
                    show_message(t!("notebooks.export_pdf"), err.to_string());
                    return;
                }
            }
        } else {
            let Ok(bytes) = std::fs::read(&working_path) else {
                return;
            };
            match Notebook::from_pdf_bytes("Notebook", bytes, PaperTemplateId::Blank) {
                Ok(nb) => nb,
                Err(err) => {
                    show_message(t!("notebooks.export_pdf"), err.to_string());
                    return;
                }
            }
        };
        let rendered = nb.export_pdf(
            ExportOptions {
                attach_audio: AttachAudio::Omit,
            },
            |_, w, h, tmpl| PaperSpec::for_template(tmpl).to_pdf_content_stream(w, h),
        );
        let pdf_bytes = match rendered {
            Ok(bytes) => bytes,
            Err(err) => {
                show_message(t!("notebooks.export_pdf"), err.to_string());
                return;
            }
        };
        let dialog = SaveDialog::new()
            .file_name(format!("{}.pdf", nb.metadata.title))
            .filter(FileFilter::new(t!("open.filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, pdf_bytes) {
                show_message(t!("notebooks.export_pdf"), err.to_string());
            }
        });
    }

    /// Opens the Infinite Canvas whiteboard dialog (N.5).
    pub(crate) fn open_infinite_canvas(self) {
        self.infinite_open.set(true);
    }

    /// Opens the Typed Text Block + IME composer dialog (N.6, U.8).
    pub(crate) fn open_text_block(self) {
        self.text_block_open.set(true);
    }

    /// Picks an image file and embeds it on the current page as an aspect-preserved PDF Stamp annotation (N.7).
    pub(crate) fn insert_image_file(self) {
        let Some(doc) = self.editing.document() else {
            return;
        };
        let page = self.annotate.active_page();
        let bounds = {
            let Ok(shared) = doc.mupdf() else {
                return;
            };
            let mupdf = shared.lock().expect("document lock poisoned");
            mupdf
                .load_page(page)
                .and_then(|p| p.bounds())
                .unwrap_or(PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 612.0,
                    y1: 792.0,
                })
        };
        let dialog = OpenDialog::files()
            .title(t!("notebooks.image.pick"))
            .filter(FileFilter::new(
                t!("notebooks.image.filter"),
                ["png", "jpg", "jpeg"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&path) else {
                return;
            };
            let center = PagePoint {
                x: (bounds.x0 + bounds.x1) * 0.5,
                y: (bounds.y0 + bounds.y1) * 0.5,
            };
            match PageMedia::from_image_bytes(page, center, bounds, 220.0, bytes) {
                Ok(mut media) => {
                    media.set_crop(CropRect::default());
                    let changes = media.to_pdf_changes("EcclesiaPDF");
                    self.annotate
                        .editing_apply(t!("command.insert_image"), changes);
                }
                Err(err) => show_message(t!("command.insert_image"), err.to_string()),
            }
        });
    }

    /// Opens the Notebook Covers & Colored Sections organizer dialog (N.8).
    pub(crate) fn open_sections(self) {
        self.sections_open.set(true);
    }

    /// Opens the Page & Deep Links manager dialog (N.9).
    pub(crate) fn open_links_manager(self) {
        self.links_open.set(true);
    }

    /// Triggers Quick Note, preparing a fresh writable page in `Inbox.expdf` and opening it (N.10).
    pub(crate) fn trigger_quick_note(self, open_path: impl Fn(PathBuf) + Send + 'static) {
        let res = self.quick_note.with(QuickNoteInbox::trigger);
        match res {
            Ok(outcome) => open_path(outcome.inbox_path),
            Err(err) => show_message(t!("command.quick_note"), err.to_string()),
        }
    }

    /// Opens the Markdown Notes & Preview dialog (N.14).
    pub(crate) fn open_markdown(self) {
        self.markdown_open.set(true);
    }

    /// Opens the Lecture Audio Recording & Sync dialog (L.1).
    pub(crate) fn open_audio(self) {
        self.audio_open.set(true);
    }

    /// Opens the Import Slides & Condense dialog (L.3).
    pub(crate) fn open_condense_slides(self) {
        self.condense_open.set(true);
    }
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
}

fn template_label(id: PaperTemplateId) -> String {
    t!(match id {
        PaperTemplateId::Blank => "notebooks.template.blank",
        PaperTemplateId::Lined => "notebooks.template.lined",
        PaperTemplateId::CollegeRuled => "notebooks.template.college_ruled",
        PaperTemplateId::Grid => "notebooks.template.grid",
        PaperTemplateId::Dot => "notebooks.template.dot",
        PaperTemplateId::Cornell => "notebooks.template.cornell",
        PaperTemplateId::Music => "notebooks.template.music",
        PaperTemplateId::Graph => "notebooks.template.graph",
        PaperTemplateId::Isometric => "notebooks.template.isometric",
        PaperTemplateId::Planner => "notebooks.template.planner",
    })
}

/// Dialog: Create a new `.expdf` notebook with procedural paper template and paper size (N.1, N.4).
pub(crate) fn new_notebook_dialog(
    state: NotebooksState,
    open_path: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.new_open.set(false);
    let title = signal(String::from("Study Notebook"));
    let template = signal(PaperTemplateId::Lined);
    let size = signal(PageSize::Letter);
    let orientation = signal(Orientation::Portrait);

    let create = move || {
        let nb_title = untracked(|| title.get());
        let tmpl = untracked(|| template.get());
        let sz = untracked(|| size.get());
        let orient = untracked(|| orientation.get());
        let Ok(nb) = create_templated_notebook(&nb_title, sz, orient, tmpl) else {
            return;
        };
        let suggested = format!("{}.expdf", nb.metadata.title);
        let dialog = SaveDialog::new()
            .file_name(suggested)
            .filter(FileFilter::new(t!("open.filter.expdf"), ["expdf"]));
        let answer = pick_save_path(dialog);
        close();
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = nb.save_atomic(&path) {
                show_message(t!("command.new_notebook"), err.to_string());
                return;
            }
            open_path(path);
        });
    };

    let tmpl_chips = PaperTemplateId::ALL
        .into_iter()
        .fold(segmented(), |group, id| {
            group.child(segment_of(
                move || template_label(id),
                move || template.get() == id,
                move || template.set(id),
            ))
        });

    let body = view()
        .w(px(560.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("notebooks.new.title"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("notebooks.new.name"))
        .child(
            field(input().value(move || title.get()))
                .on_change(move |s: &str| title.set(s.to_owned()))
                .label(t!("notebooks.new.name")),
        )
        .child(caption("notebooks.new.template"))
        .child(scroll().max_h(px(120.0)).child(tmpl_chips))
        .child(caption("notebooks.new.size"))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    segmented()
                        .child(segment_of(
                            move || String::from("Letter"),
                            move || size.get() == PageSize::Letter,
                            move || size.set(PageSize::Letter),
                        ))
                        .child(segment_of(
                            move || String::from("A4"),
                            move || size.get() == PageSize::A4,
                            move || size.set(PageSize::A4),
                        ))
                        .child(segment_of(
                            move || String::from("A5"),
                            move || size.get() == PageSize::A5,
                            move || size.set(PageSize::A5),
                        )),
                )
                .child(
                    segmented()
                        .child(segment_of(
                            move || t!("notebooks.portrait"),
                            move || orientation.get() == Orientation::Portrait,
                            move || orientation.set(Orientation::Portrait),
                        ))
                        .child(segment_of(
                            move || t!("notebooks.landscape"),
                            move || orientation.get() == Orientation::Landscape,
                            move || orientation.set(Orientation::Landscape),
                        )),
                ),
        )
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("notebooks.create", create)),
        );

    list_dialog(t!("command.new_notebook"), close, card(body))
}

/// Builds a fresh single-page `.expdf` notebook with its procedural template baked on page 0 (N.1, N.4).
pub(crate) fn create_templated_notebook(
    title: &str,
    size: PageSize,
    orientation: Orientation,
    template: PaperTemplateId,
) -> ecclesia_core::notebook::Result<Notebook> {
    let mut nb = Notebook::new_blank(title, size, orientation, template)?;
    if template != PaperTemplateId::Blank {
        let (w, h) = size.dimensions(orientation);
        let stream = PaperSpec::for_template(template).to_pdf_content_stream(w, h);
        let updated = pdf_ops::prepend_page_content(nb.pdf_bytes(), 0, &stream)?;
        nb.replace_pdf_bytes(updated)?;
    }
    Ok(nb)
}

/// Dialog: Insert blank or templated page before/after the current page (N.2).
pub(crate) fn insert_page_dialog(
    state: NotebooksState,
    reload_doc: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.insert_page_open.set(false);
    let after = signal(true);
    let template = signal(PaperTemplateId::Lined);

    let apply = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let working = doc.path().to_path_buf();
        let page = state.annotate.active_page();
        let pos = if untracked(|| after.get()) {
            InsertPosition::After(page)
        } else {
            InsertPosition::Before(page)
        };
        let tmpl = untracked(|| template.get());
        let src = InsertPageSource::Template {
            spec: PaperSpec::for_template(tmpl),
            size: PageSize::Letter,
            orientation: Orientation::Portrait,
        };
        if let Some(expdf_path) = expdf::expdf_for_working_path(&working) {
            let Ok(mut nb) = Notebook::open(&expdf_path) else {
                return;
            };
            if let Err(err) = insert_page_in_notebook(&mut nb, pos, &src) {
                show_message(t!("command.insert_page"), err.to_string());
                return;
            }
            if let Err(err) = expdf::save_and_refresh_working_pdf(&expdf_path, &nb) {
                show_message(t!("command.insert_page"), err.to_string());
                return;
            }
            close();
            reload_doc(expdf_path);
        } else if let Ok(bytes) = std::fs::read(&working) {
            match insert_page_in_pdf(&bytes, pos, &src) {
                Ok((updated, _)) => {
                    if let Err(err) = write_atomically(&working, updated) {
                        show_message(t!("command.insert_page"), err.to_string());
                        return;
                    }
                    close();
                    reload_doc(working);
                }
                Err(err) => show_message(t!("command.insert_page"), err.to_string()),
            }
        }
    };

    let tmpl_chips = PaperTemplateId::ALL
        .into_iter()
        .take(6)
        .fold(segmented(), |g, id| {
            g.child(segment_of(
                move || template_label(id),
                move || template.get() == id,
                move || template.set(id),
            ))
        });

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.insert_page"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("notebooks.insert.before"),
                    move || !after.get(),
                    move || after.set(false),
                ))
                .child(segment_of(
                    move || t!("notebooks.insert.after"),
                    move || after.get(),
                    move || after.set(true),
                )),
        )
        .child(tmpl_chips)
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("ink_tools.apply", apply)),
        );
    list_dialog(t!("command.insert_page"), close, card(body))
}

/// Dialog: Expand or restore page margins without moving existing PDF content (N.3).
pub(crate) fn margins_dialog(
    state: NotebooksState,
    reload_doc: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.margins_open.set(false);
    let side_mode = signal(0_u8); // 0=right 180pt, 1=horizontal 120pt, 2=restore
    let apply = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let working = doc.path().to_path_buf();
        let page = state.annotate.active_page();
        let exp = match untracked(|| side_mode.get()) {
            0 => Some(MarginExpansion::right(180.0)),
            1 => Some(MarginExpansion::horizontal(120.0)),
            _ => None,
        };
        if let Some(expdf_path) = expdf::expdf_for_working_path(&working) {
            let Ok(mut nb) = Notebook::open(&expdf_path) else {
                return;
            };
            let res = match exp {
                Some(m) => expand_page_margins_notebook(&mut nb, page, m).map(|_| ()),
                None => remove_added_margins_notebook(&mut nb, page),
            };
            if let Err(err) = res.and_then(|()| {
                expdf::save_and_refresh_working_pdf(&expdf_path, &nb)
                    .map_err(ecclesia_core::notebook::NotebookError::InvalidFormat)
            }) {
                show_message(t!("command.expand_margins"), err.to_string());
                return;
            }
            close();
            reload_doc(expdf_path);
        } else if let Ok(bytes) = std::fs::read(&working) {
            let res = match exp {
                Some(m) => expand_page_margins_pdf(&bytes, page, m).map(|(b, _)| b),
                None => remove_added_margins_pdf(&bytes, page),
            };
            match res {
                Ok(updated) => {
                    if let Err(err) = write_atomically(&working, updated) {
                        show_message(t!("command.expand_margins"), err.to_string());
                        return;
                    }
                    close();
                    reload_doc(working);
                }
                Err(err) => show_message(t!("command.expand_margins"), err.to_string()),
            }
        }
    };

    let body = view()
        .w(px(440.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.expand_margins"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("notebooks.margins.right"),
                    move || side_mode.get() == 0,
                    move || side_mode.set(0),
                ))
                .child(segment_of(
                    move || t!("notebooks.margins.both"),
                    move || side_mode.get() == 1,
                    move || side_mode.set(1),
                ))
                .child(segment_of(
                    move || t!("notebooks.margins.restore"),
                    move || side_mode.get() == 2,
                    move || side_mode.set(2),
                )),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("ink_tools.apply", apply)),
        );
    list_dialog(t!("command.expand_margins"), close, card(body))
}

/// Dialog: Infinite canvas whiteboard with pan/zoom and PDF export (N.5).
pub(crate) fn infinite_canvas_dialog(state: NotebooksState) -> impl IntoElement {
    let close = move || state.infinite_open.set(false);
    let tiled = signal(false);
    let rev = signal(0_u64);

    let add_sample_stroke = move || {
        let count = state.infinite_canvas.with(|c| {
            let offset = (c.strokes.len() as f64) * 32.0;
            c.push_stroke(CanvasStroke {
                id: ecclesia_core::anchor::new_id(),
                color: [0.15, 0.38, 0.85],
                opacity: 1.0,
                points: vec![
                    CanvasPoint {
                        x: 60.0 + offset,
                        y: 80.0 + offset * 0.4,
                        width: 2.0,
                    },
                    CanvasPoint {
                        x: 240.0 + offset,
                        y: 120.0 + offset * 0.4,
                        width: 2.0,
                    },
                ],
            });
            c.strokes.len()
        });
        let _ = count;
        rev.update(|r| *r += 1);
    };

    let export_pdf = move || {
        let mode = if untracked(|| tiled.get()) {
            InfiniteExportMode::Tiled {
                paper: PageSize::Letter,
                orientation: Orientation::Portrait,
                margin_pt: 36.0,
            }
        } else {
            InfiniteExportMode::SinglePage { padding_pt: 36.0 }
        };
        let bytes = match state.infinite_canvas.with(|c| c.export_pdf(mode)) {
            Ok(bytes) => bytes,
            Err(err) => {
                show_message(t!("command.infinite_canvas"), err.to_string());
                return;
            }
        };
        let dialog = SaveDialog::new()
            .file_name("Whiteboard.pdf".to_owned())
            .filter(FileFilter::new(t!("open.filter.pdf"), ["pdf"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, bytes) {
                show_message(t!("command.infinite_canvas"), err.to_string());
            }
        });
    };

    let body = view()
        .w(px(520.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.infinite_canvas"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(dynamic(move || {
            let _ = rev.get();
            let count = state.infinite_canvas.with(|c| c.strokes.len());
            let bounds = state.infinite_canvas.with(|c| c.content_bounds());
            let summary = match bounds {
                Some(b) => t!(
                    "notebooks.infinite.summary",
                    count = count,
                    width = b.width().round() as i64,
                    height = b.height().round() as i64
                ),
                None => t!("notebooks.infinite.empty"),
            };
            view()
                .p_3()
                .rounded(px(8.0))
                .border_1()
                .border_color(move || theme().border)
                .child(
                    text(summary)
                        .text_size(px(13.0))
                        .text_color(move || theme().muted_foreground),
                )
        }))
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(action_button(
                    "notebooks.infinite.add_stroke",
                    Tone::Plain,
                    add_sample_stroke,
                ))
                .child(
                    segmented()
                        .child(segment_of(
                            move || t!("notebooks.infinite.single_page"),
                            move || !tiled.get(),
                            move || tiled.set(false),
                        ))
                        .child(segment_of(
                            move || t!("notebooks.infinite.tiled"),
                            move || tiled.get(),
                            move || tiled.set(true),
                        )),
                ),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("ink_tools.done", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "notebooks.infinite.export",
                    export_pdf,
                )),
        );
    list_dialog(t!("command.infinite_canvas"), close, card(body))
}

/// Dialog: Typed Text Blocks inside the canvas with U.8 IME composition bridge (N.6, U.8).
pub(crate) fn text_block_dialog(state: NotebooksState) -> impl IntoElement {
    let close = move || state.text_block_open.set(false);
    let kind_idx = signal(0_u8); // 0=Paragraph, 1=Heading, 2=Checklist, 3=Code
    let text_draft = signal(String::new());
    let ime = stored(ImeBridge::default());

    let insert = move || {
        let body_text = untracked(|| text_draft.get());
        if body_text.trim().is_empty() {
            return;
        }
        let page = state.annotate.active_page();
        let rect = PageRect {
            x0: 72.0,
            y0: 120.0,
            x1: 480.0,
            y1: 220.0,
        };
        let kind = match untracked(|| kind_idx.get()) {
            1 => BlockKind::Heading { level: 2 },
            2 => BlockKind::Checkbox { checked: false },
            3 => BlockKind::Code { language: None },
            4 => BlockKind::List {
                style: ListStyle::Bulleted,
                items: body_text.lines().map(str::to_owned).collect(),
            },
            _ => BlockKind::Paragraph,
        };
        let mut block = TextBlock::new(page, rect, kind, body_text.clone());
        ime.with(|bridge| {
            let _ = block.apply_ime(bridge, ImeEvent::Commit(String::new()));
        });
        let annot = block.to_freetext_annotation("EcclesiaPDF");
        state.annotate.editing_apply(
            t!("command.insert_text_block"),
            vec![ecclesia_core::edit::Change::CreateAnnotation(annot)],
        );
        close();
    };

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.insert_text_block"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("notebooks.block.paragraph"),
                    move || kind_idx.get() == 0,
                    move || kind_idx.set(0),
                ))
                .child(segment_of(
                    move || t!("notebooks.block.heading"),
                    move || kind_idx.get() == 1,
                    move || kind_idx.set(1),
                ))
                .child(segment_of(
                    move || t!("notebooks.block.checklist"),
                    move || kind_idx.get() == 2,
                    move || kind_idx.set(2),
                ))
                .child(segment_of(
                    move || t!("notebooks.block.code"),
                    move || kind_idx.get() == 3,
                    move || kind_idx.set(3),
                )),
        )
        .child(
            field(input().value(move || text_draft.get()))
                .multiline(true)
                .h(px(120.0))
                .on_change(move |s: &str| text_draft.set(s.to_owned()))
                .placeholder(t!("notebooks.block.placeholder"))
                .label(t!("command.insert_text_block"))
                .auto_focus(true),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "notebooks.block.insert",
                    insert,
                )),
        );
    list_dialog(t!("command.insert_text_block"), close, card(body))
}

/// Dialog: Notebook Covers & Colored Sections organizer (N.8).
pub(crate) fn sections_dialog(
    state: NotebooksState,
    reload_doc: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.sections_open.set(false);
    let cover_title = signal(String::new());
    let sec_title = signal(String::new());

    let apply_cover = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let Some(expdf_path) = expdf::expdf_for_working_path(doc.path()) else {
            show_message(
                t!("command.notebook_sections"),
                t!("notebooks.expdf_required"),
            );
            return;
        };
        let Ok(mut nb) = Notebook::open(&expdf_path) else {
            return;
        };
        let name = untracked(|| cover_title.get());
        let resolved_title = if name.trim().is_empty() {
            nb.metadata.title.clone()
        } else {
            name
        };
        set_cover(
            &mut nb,
            CoverStyle::Classic,
            resolved_title,
            None,
            [37, 99, 235],
            None,
        );
        if let Err(err) = expdf::save_and_refresh_working_pdf(&expdf_path, &nb) {
            show_message(t!("command.notebook_sections"), err);
        } else {
            reload_doc(expdf_path);
        }
    };

    let add_sec = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let Some(expdf_path) = expdf::expdf_for_working_path(doc.path()) else {
            show_message(
                t!("command.notebook_sections"),
                t!("notebooks.expdf_required"),
            );
            return;
        };
        let Ok(mut nb) = Notebook::open(&expdf_path) else {
            return;
        };
        let name = untracked(|| sec_title.get());
        if name.trim().is_empty() {
            return;
        }
        let start = state
            .annotate
            .active_page()
            .min(nb.metadata.pages.len().saturating_sub(1));
        if let Err(err) = add_section(&mut nb, name, [22, 163, 74], start, 1) {
            show_message(t!("command.notebook_sections"), err.to_string());
            return;
        }
        if let Err(err) = expdf::save_and_refresh_working_pdf(&expdf_path, &nb) {
            show_message(t!("command.notebook_sections"), err);
        } else {
            sec_title.set(String::new());
            reload_doc(expdf_path);
        }
    };

    let tabs_summary = dynamic(move || {
        let Some(doc) = state.editing.document() else {
            return view();
        };
        let Some(expdf_path) = expdf::expdf_for_working_path(doc.path()) else {
            return view().child(
                text(move || t!("notebooks.expdf_required"))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            );
        };
        let Ok(nb) = Notebook::open(&expdf_path) else {
            return view();
        };
        let tabs = tab_strip(&nb.metadata.sections);
        if tabs.is_empty() {
            return view().child(
                text(move || t!("notebooks.sections.empty"))
                    .text_size(px(12.0))
                    .text_color(move || theme().muted_foreground),
            );
        }
        tabs.into_iter().fold(view().gap_1(), |col, tab| {
            let col_rgb = ((u32::from(tab.color[0])) << 16)
                | ((u32::from(tab.color[1])) << 8)
                | u32::from(tab.color[2]);
            col.child(
                view()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .p(px(6.0))
                    .rounded(px(6.0))
                    .border_1()
                    .border_color(move || theme().border)
                    .child(
                        view()
                            .w(px(10.0))
                            .h(px(10.0))
                            .rounded_full()
                            .bg(Hsla::from(rgb(col_rgb))),
                    )
                    .child(
                        text(format!("{} (p.{})", tab.title, tab.start_page + 1))
                            .text_size(px(12.0))
                            .text_color(move || theme().popover_foreground),
                    ),
            )
        })
    });

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.notebook_sections"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view().flex_1().child(
                        field(input().value(move || cover_title.get()))
                            .on_change(move |s: &str| cover_title.set(s.to_owned()))
                            .placeholder(t!("notebooks.cover.placeholder"))
                            .label(t!("notebooks.cover.title")),
                    ),
                )
                .child(action_button(
                    "notebooks.cover.set",
                    Tone::Plain,
                    apply_cover,
                )),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    view().flex_1().child(
                        field(input().value(move || sec_title.get()))
                            .on_change(move |s: &str| sec_title.set(s.to_owned()))
                            .placeholder(t!("notebooks.sections.placeholder"))
                            .label(t!("notebooks.sections.add")),
                    ),
                )
                .child(action_button(
                    "notebooks.sections.add",
                    Tone::Plain,
                    add_sec,
                )),
        )
        .child(tabs_summary)
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.notebook_sections"), close, card(body))
}

/// Dialog: Create internal page links and `ecclesiapdf://` deep links in `.expdf` notebooks (N.9).
pub(crate) fn links_manager_dialog(
    state: NotebooksState,
    reload_doc: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.links_open.set(false);
    let target_page_str = signal(String::from("1"));
    let deep_doc_id = signal(String::new());
    let link_label = signal(String::from("Reference"));

    let add_link = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let Some(expdf_path) = expdf::expdf_for_working_path(doc.path()) else {
            show_message(t!("command.notebook_links"), t!("notebooks.expdf_required"));
            return;
        };
        let Ok(mut nb) = Notebook::open(&expdf_path) else {
            return;
        };
        let src_page = state.annotate.active_page();
        let rect = PageRect {
            x0: 72.0,
            y0: 72.0,
            x1: 220.0,
            y1: 96.0,
        };
        let label = Some(untracked(|| link_label.get()));
        let deep_id = untracked(|| deep_doc_id.get());
        let target_p = untracked(|| target_page_str.get())
            .trim()
            .parse::<usize>()
            .unwrap_or(1)
            .saturating_sub(1);
        let res = if deep_id.trim().is_empty() {
            add_internal_link(&mut nb, src_page, rect, target_p, None, None, label).map(|_| ())
        } else {
            let deep = DeepLink {
                doc_id: deep_id.trim().to_owned(),
                page: Some(target_p),
                point: None,
            };
            add_deep_link(&mut nb, src_page, rect, &deep, label).map(|_| ())
        };
        if let Err(err) = res.and_then(|()| {
            expdf::save_and_refresh_working_pdf(&expdf_path, &nb)
                .map_err(ecclesia_core::notebook::NotebookError::InvalidFormat)
        }) {
            show_message(t!("command.notebook_links"), err.to_string());
        } else {
            reload_doc(expdf_path);
            close();
        }
    };

    let body = view()
        .w(px(440.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.notebook_links"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("notebooks.links.target_page"))
        .child(
            field(input().value(move || target_page_str.get()))
                .on_change(move |s: &str| target_page_str.set(s.to_owned()))
                .label(t!("notebooks.links.target_page")),
        )
        .child(caption("notebooks.links.deep_id"))
        .child(
            field(input().value(move || deep_doc_id.get()))
                .on_change(move |s: &str| deep_doc_id.set(s.to_owned()))
                .placeholder(t!("notebooks.links.deep_placeholder"))
                .label(t!("notebooks.links.deep_id")),
        )
        .child(caption("notebooks.links.label"))
        .child(
            field(input().value(move || link_label.get()))
                .on_change(move |s: &str| link_label.set(s.to_owned()))
                .label(t!("notebooks.links.label")),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "notebooks.links.add",
                    add_link,
                )),
        );
    list_dialog(t!("command.notebook_links"), close, card(body))
}

/// Dialog: Markdown Editor with GitHub Heading TOC and KaTeX/Mermaid HTML / MuPDF Preview (N.14).
pub(crate) fn markdown_dialog(state: NotebooksState) -> impl IntoElement {
    let close = move || state.markdown_open.set(false);
    let source = signal(String::from(
        "# Lecture Notes\n\n## Key Equation\n\n$$E = mc^2$$\n\n- [x] Verify theorem\n",
    ));
    let mut ctrl = PreviewController::new(PreviewConfig::default());
    let webview_ok = signal(true);
    if !untracked(|| webview_ok.get()) {
        ctrl.mark_webview_unavailable();
    }

    let export_html = move || {
        let md = untracked(|| source.get());
        let bundle = PreviewController::new(PreviewConfig::default()).build_preview(&md);
        let dialog = SaveDialog::new()
            .file_name("Notes.html".to_owned())
            .filter(FileFilter::new(t!("study.export.html"), ["html"]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            let Ok(Some(path)) = answer.await else {
                return;
            };
            if let Err(err) = write_atomically(&path, bundle.document_html) {
                show_message(t!("command.markdown_notes"), err.to_string());
            }
        });
    };

    let body = view()
        .w(px(620.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("command.markdown_notes"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(action_button(
                    "notebooks.markdown.toggle_backend",
                    Tone::Plain,
                    move || webview_ok.update(|v| *v = !*v),
                ))
                .child(action_button(
                    "notebooks.markdown.export",
                    Tone::Plain,
                    export_html,
                )),
        )
        .child(
            view()
                .flex_row()
                .gap_3()
                .child(
                    view().flex_1().gap_1().child(
                        field(input().value(move || source.get()))
                            .multiline(true)
                            .h(px(260.0))
                            .on_change(move |s: &str| source.set(s.to_owned()))
                            .label(t!("command.markdown_notes")),
                    ),
                )
                .child(view().w(px(240.0)).gap_2().child(dynamic(move || {
                    let md = source.get();
                    let toc = extract_toc(&md);
                    let rendered = render_gfm_html(&md, RenderOptions::rich_preview());
                    let backend_label = if webview_ok.get() {
                        PreviewBackend::WebView
                    } else {
                        PreviewBackend::MuPdfFallback
                    };
                    let mode_text = match backend_label {
                        PreviewBackend::WebView => t!("notebooks.markdown.webview"),
                        PreviewBackend::MuPdfFallback => t!("notebooks.markdown.mupdf"),
                    };
                    let toc_list = toc.into_iter().fold(view().gap_1(), |col, h| {
                        col.child(
                            text(format!("{} {}", "#".repeat(usize::from(h.level)), h.title))
                                .text_size(px(12.0))
                                .text_color(move || theme().popover_foreground),
                        )
                    });
                    view()
                        .p_2()
                        .rounded(px(8.0))
                        .border_1()
                        .border_color(move || theme().border)
                        .gap_2()
                        .child(
                            text(mode_text)
                                .text_size(px(11.0))
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(
                            text(t!(
                                "notebooks.markdown.stats",
                                headings = rendered.headings.len(),
                                math = if rendered.has_math { 1 } else { 0 },
                                mermaid = if rendered.has_mermaid { 1 } else { 0 }
                            ))
                            .text_size(px(11.0))
                            .text_color(move || theme().muted_foreground),
                        )
                        .child(scroll().max_h(px(190.0)).child(toc_list))
                }))),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.markdown_notes"), close, card(body))
}

/// Dialog: Lecture Audio Recording & Handwriting Timeline Sync (L.1).
pub(crate) fn audio_dialog(state: NotebooksState) -> impl IntoElement {
    let close = move || state.audio_open.set(false);

    let toggle_record = move || {
        let active = untracked(|| state.audio_recording.get());
        if !active {
            let started = state.audio_capture.with(|slot| {
                let cap = slot.as_mut()?;
                let _ = cap.request_permission();
                Some(cap.start(CaptureConfig::default()))
            });
            match started {
                Some(Ok(())) => {
                    let id = ecclesia_core::anchor::new_id();
                    let tl = AudioTimeline::start(id, now_ms(), state.annotate.active_page());
                    state.audio_timeline.with(|slot| *slot = Some(tl));
                    state.audio_recording.set(true);
                    state.audio_status.set(t!("notebooks.audio.recording"));
                }
                Some(Err(err)) => state.audio_status.set(err.to_string()),
                None => state.audio_status.set(t!("audio.backend_unavailable")),
            }
        } else {
            let pcm = state.audio_capture.with(|slot| {
                slot.as_mut()
                    .and_then(|cap| cap.stop().ok())
                    .unwrap_or_default()
            });
            state.audio_recording.set(false);
            let mut recorded_ms: f64 = 0.0;
            let mut started_ms: f64 = 0.0;
            state.audio_timeline.with(|slot| {
                if let Some(tl) = slot.as_mut() {
                    let t_now = now_ms();
                    tl.pause(t_now);
                    recorded_ms = tl.total_duration().as_secs_f64() * 1000.0;
                    started_ms = (t_now - recorded_ms).max(0.0);
                }
            });
            if let Ok(wav) = encode_wav(&pcm, 48_000, 1)
                && let Some(doc) = state.editing.document()
                && let Some(expdf_path) = expdf::expdf_for_working_path(doc.path())
                && let Ok(mut nb) = Notebook::open(&expdf_path)
            {
                let spec = AudioBundleSpec {
                    recording_id: ecclesia_core::anchor::new_id(),
                    format: ecclesia_core::notebook::AudioFormat::Wav,
                    bytes: wav,
                    started_at_ms: started_ms,
                    duration_ms: recorded_ms.max(0.0),
                    sample_rate: 48_000,
                    channels: 1,
                    title: None,
                };
                if bundle_audio_in_notebook(&mut nb, spec).is_ok() {
                    let _ = expdf::save_and_refresh_working_pdf(&expdf_path, &nb);
                }
            }
            state.audio_status.set(t!(
                "notebooks.audio.stopped",
                seconds = (recorded_ms / 1000.0).round() as u64
            ));
        }
    };

    let toggle_play = move || {
        let playing = state.audio_playback.with(|slot| {
            let pb = slot.as_mut()?;
            Some(match pb.state() {
                AudioPlayState::Playing => {
                    pb.pause();
                    false
                }
                AudioPlayState::Stopped | AudioPlayState::Paused => pb.play().is_ok(),
            })
        });
        match playing {
            Some(is_playing) => state.audio_playing.set(is_playing),
            None => {
                state.audio_playing.set(false);
                state.audio_status.set(t!("audio.backend_unavailable"));
            }
        }
    };

    let body = view()
        .w(px(440.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.lecture_audio"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(action_button(
                    "notebooks.audio.record_toggle",
                    Tone::Plain,
                    toggle_record,
                ))
                .child(action_button(
                    "notebooks.audio.play_toggle",
                    Tone::Plain,
                    toggle_play,
                )),
        )
        .child(
            text(move || {
                let s = state.audio_status.get();
                if s.is_empty() {
                    t!("notebooks.audio.idle")
                } else {
                    s
                }
            })
            .text_size(px(12.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.lecture_audio"), close, card(body))
}

/// Dialog: Import a slide PDF and condense 1, 2, or 3 slides per note page (L.3).
pub(crate) fn condense_slides_dialog(
    state: NotebooksState,
    open_path: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.condense_open.set(false);
    let per_page = signal(SlidesPerPage::Three);
    let note_style = signal(CondenseNoteStyle::Lined);

    let import_and_condense = move || {
        let slides = untracked(|| per_page.get());
        let style = untracked(|| note_style.get());
        let open_dlg = OpenDialog::files()
            .title(t!("notebooks.slides.pick"))
            .filter(FileFilter::new(t!("open.filter.pdf"), ["pdf"]));
        let answer = pick_paths(open_dlg);
        close();
        spawn(async move {
            let Ok(Some(paths)) = answer.await else {
                return;
            };
            let Some(src_path) = paths.into_iter().next() else {
                return;
            };
            let Ok(bytes) = std::fs::read(&src_path) else {
                return;
            };
            let title = src_path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Lecture Notes")
                .to_owned();
            let opts = CondenseOptions {
                slides_per_page: slides,
                note_style: style,
                slide_numbers: SlideNumbers::Show,
                ..CondenseOptions::default()
            };
            let nb = match blocking(move || {
                condense_slides_to_notebook(title, &bytes, &opts).map_err(|e| e.to_string())
            })
            .await
            {
                Ok(Ok(nb)) => nb,
                Ok(Err(err)) => {
                    show_message(t!("command.condense_slides"), err);
                    return;
                }
                Err(err) => {
                    show_message(t!("command.condense_slides"), err.to_string());
                    return;
                }
            };
            let save_dlg = SaveDialog::new()
                .file_name(format!("{} Notes.expdf", nb.metadata.title))
                .filter(FileFilter::new(t!("open.filter.expdf"), ["expdf"]));
            let Ok(Some(dest)) = pick_save_path(save_dlg).await else {
                return;
            };
            if let Err(err) = nb.save_atomic(&dest) {
                show_message(t!("command.condense_slides"), err.to_string());
                return;
            }
            open_path(dest);
        });
    };

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.condense_slides"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("notebooks.slides.per_page"))
        .child(
            segmented()
                .child(segment_of(
                    move || String::from("1"),
                    move || per_page.get() == SlidesPerPage::One,
                    move || per_page.set(SlidesPerPage::One),
                ))
                .child(segment_of(
                    move || String::from("2"),
                    move || per_page.get() == SlidesPerPage::Two,
                    move || per_page.set(SlidesPerPage::Two),
                ))
                .child(segment_of(
                    move || String::from("3"),
                    move || per_page.get() == SlidesPerPage::Three,
                    move || per_page.set(SlidesPerPage::Three),
                )),
        )
        .child(caption("notebooks.slides.style"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("notebooks.template.lined"),
                    move || note_style.get() == CondenseNoteStyle::Lined,
                    move || note_style.set(CondenseNoteStyle::Lined),
                ))
                .child(segment_of(
                    move || t!("notebooks.template.cornell"),
                    move || note_style.get() == CondenseNoteStyle::Cornell,
                    move || note_style.set(CondenseNoteStyle::Cornell),
                ))
                .child(segment_of(
                    move || t!("notebooks.template.grid"),
                    move || note_style.get() == CondenseNoteStyle::Grid,
                    move || note_style.set(CondenseNoteStyle::Grid),
                ))
                .child(segment_of(
                    move || t!("notebooks.template.blank"),
                    move || note_style.get() == CondenseNoteStyle::Blank,
                    move || note_style.set(CondenseNoteStyle::Blank),
                )),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "notebooks.slides.choose_pdf",
                    import_and_condense,
                )),
        );
    list_dialog(t!("command.condense_slides"), close, card(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_templated_expdf_notebook_and_exports_pdf() {
        let dir = std::env::temp_dir().join("ecclesiapdf-app-nb-test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sample.expdf");
        let nb = create_templated_notebook(
            "Physics",
            PageSize::Letter,
            Orientation::Portrait,
            PaperTemplateId::Cornell,
        )
        .expect("notebook");
        nb.save_atomic(&path).expect("save expdf");
        let reopened = Notebook::open(&path).expect("reopen expdf");
        let pdf = reopened
            .export_pdf(ExportOptions::default(), |_, w, h, tmpl| {
                PaperSpec::for_template(tmpl).to_pdf_content_stream(w, h)
            })
            .expect("export pdf");
        assert!(pdf.starts_with(b"%PDF"));
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn ime_bridge_commits_into_text_block() {
        let mut block = TextBlock::new(
            0,
            PageRect {
                x0: 10.0,
                y0: 10.0,
                x1: 200.0,
                y1: 50.0,
            },
            BlockKind::Paragraph,
            "Hello ".to_owned(),
        );
        let mut bridge = ImeBridge::default();
        let _ = block.apply_ime(&mut bridge, ImeEvent::Start);
        let _ = block.apply_ime(&mut bridge, ImeEvent::Commit("World".to_owned()));
        assert_eq!(block.plain_text(), "Hello World");
    }
}
