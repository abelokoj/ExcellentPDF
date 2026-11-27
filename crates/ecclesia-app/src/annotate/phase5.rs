//! Phase 5 Annotation, Handwriting & OCR UI actions and dialogs:
//! - A.26 Annotation Report Export (Markdown, HTML, CSV, PDF summary + page crops)
//! - A.27 Annotation History Timeline Scrubber & non-destructive restore
//! - H.19 Handwriting Beautify
//! - H.20 Pattern Brushes
//! - H.21 Symmetry axis & mirror strokes
//! - H.23 Searchable Handwriting FTS5 index & search dialog
//! - H.24 Convert Lassoed Handwriting to FreeText / clipboard text
//! - O.4 OCR current page / document to searchable PDF

use std::path::PathBuf;

use ecclesia_core::annot::{Annotation, Shape, Style, page_annotations};
use ecclesia_core::edit::{Change, Operation};
use ecclesia_features::annotate::history::AnnotationTimeline;
use ecclesia_features::annotate::report::{
    ReportConfig, ReportFormat, attach_page_crops, build_report_items, export_csv, export_html,
    export_markdown, export_pdf_summary,
};
use ecclesia_features::handwriting::{
    ConversionTarget, ConvertOptions, HandwritingIndex, HandwritingStroke, OfflineStrokeRecognizer,
    convert_lasso_to_text,
};
use ecclesia_features::ocr::{
    AutoOcrEngine, ExistingTextPolicy, OcrConfig, ocr_pdf_file, standard_language_packs,
};
use ecclesia_ink::beautify::{BeautifyOptions, LetterNormalization, beautify_ink};
use ecclesia_ink::brushes::{PatternBrush, PatternKind, to_pdf_appearance_stream};
use ecclesia_ink::stroke::InkPoint;
use ecclesia_ink::symmetry::{RadialMode, SymmetryAxis, guide_segments, mirror_stroke};
use ecclesia_mupdf::PagePoint;
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, SaveDialog};
use nsh_core::prelude::*;

use super::{AnnotateState, author};
use crate::bookmarks::{caption, field};
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, write_atomically};
use crate::settings_window::{segment_of, segment_owned, segmented};
use crate::tabs::{Tone, action_button};

fn collect_all_annotations(state: AnnotateState) -> Vec<Annotation> {
    let Some(document) = state.editing.document() else {
        return Vec::new();
    };
    let Ok(shared) = document.mupdf() else {
        return Vec::new();
    };
    let mupdf = shared.lock().expect("document lock poisoned");
    let count = mupdf.page_count().unwrap_or(0);
    let mut out = Vec::new();
    for page in 0..count {
        if let Ok(annots) = page_annotations(&mupdf, page) {
            out.extend(annots);
        }
    }
    out
}

fn page_ink_annotations(state: AnnotateState, page: usize) -> Vec<Annotation> {
    let Some(document) = state.editing.document() else {
        return Vec::new();
    };
    let Ok(shared) = document.mupdf() else {
        return Vec::new();
    };
    let mupdf = shared.lock().expect("document lock poisoned");
    let selected_ids: Vec<String> = untracked(|| state.selected.get())
        .into_iter()
        .map(|t| t.id)
        .collect();
    let annots = page_annotations(&mupdf, page).unwrap_or_default();
    let filtered: Vec<Annotation> = annots
        .iter()
        .filter(|a| matches!(a.shape, Shape::Ink(_)))
        .filter(|a| selected_ids.is_empty() || selected_ids.contains(&a.id))
        .cloned()
        .collect();
    if filtered.is_empty() {
        annots
            .into_iter()
            .filter(|a| matches!(a.shape, Shape::Ink(_)))
            .collect()
    } else {
        filtered
    }
}

fn to_hw_strokes(annots: &[Annotation]) -> (Vec<HandwritingStroke>, Vec<String>) {
    let mut strokes = Vec::new();
    let mut ids = Vec::new();
    for a in annots {
        if let Shape::Ink(lists) = &a.shape {
            ids.push(a.id.clone());
            for (idx, pts) in lists.iter().enumerate() {
                strokes.push(HandwritingStroke {
                    id: format!("{}-{}", a.id, idx),
                    points: pts.clone(),
                    timestamps_ms: Vec::new(),
                });
            }
        }
    }
    (strokes, ids)
}

impl AnnotateState {
    /// Opens the Annotation Report Export dialog (plan A.26).
    pub(crate) fn open_report(self) {
        self.report_open.set(true);
    }

    /// Opens the Annotation History Timeline Scrubber dialog (plan A.27).
    pub(crate) fn open_history(self) {
        self.history_open.set(true);
    }

    /// Opens the Pattern Brush picker dialog (plan H.20).
    pub(crate) fn open_pattern_brush(self) {
        self.pattern_open.set(true);
    }

    /// Opens the Symmetry settings & mirror action dialog (plan H.21).
    pub(crate) fn open_symmetry(self) {
        self.symmetry_open.set(true);
    }

    /// Opens the Searchable Handwriting dialog (plan H.23).
    pub(crate) fn open_hw_search(self) {
        self.hw_search_open.set(true);
    }

    /// Opens the Convert Handwriting to Text dialog (plan H.24).
    pub(crate) fn open_hw_convert(self) {
        self.hw_convert_open.set(true);
    }

    /// Opens the OCR to Searchable PDF dialog (plan O.4).
    pub(crate) fn open_ocr(self) {
        self.ocr_open.set(true);
    }

    /// Beautifies selected ink annotations (or active page's ink) as one undoable edit (plan H.19).
    pub(crate) fn beautify_ink_selection(self) {
        if !self.editing.can_edit() {
            return;
        }
        let page = untracked(|| self.current_page.get());
        let ink_annots = page_ink_annotations(self, page);
        let mut changes = Vec::new();
        for mut annot in ink_annots {
            let Shape::Ink(ref strokes) = annot.shape else {
                continue;
            };
            let width = annot.style.width.max(1.5);
            let input: Vec<Vec<InkPoint>> = strokes
                .iter()
                .map(|s| {
                    s.iter()
                        .map(|p| InkPoint {
                            x: p.x,
                            y: p.y,
                            width,
                        })
                        .collect()
                })
                .collect();
            let result = beautify_ink(
                &input,
                BeautifyOptions {
                    strength: 0.55,
                    normalization: LetterNormalization::HeightAndSpacing,
                },
            );
            let updated_strokes: Vec<Vec<PagePoint>> = result
                .strokes
                .into_iter()
                .map(|s| {
                    s.into_iter()
                        .map(|p| PagePoint { x: p.x, y: p.y })
                        .collect()
                })
                .collect();
            annot.shape = Shape::Ink(updated_strokes);
            changes.push(Change::UpdateAnnotation(annot));
        }
        if changes.is_empty() {
            show_message(t!("command.beautify_ink"), t!("ink_tools.beautify.empty"));
        } else {
            self.editing
                .apply(&Operation::new(t!("command.beautify_ink"), changes));
        }
    }

    /// Mirrors selected ink strokes across the active symmetry axis (plan H.21).
    pub(crate) fn apply_symmetry_to_selection(self, axis: SymmetryAxis) {
        if !self.editing.can_edit() {
            return;
        }
        let page = untracked(|| self.current_page.get());
        let ink_annots = page_ink_annotations(self, page);
        let mut changes = Vec::new();
        for annot in ink_annots {
            let Shape::Ink(ref strokes) = annot.shape else {
                continue;
            };
            let width = annot.style.width.max(1.5);
            let mut mirrored_groups = Vec::new();
            for s in strokes {
                let pts: Vec<InkPoint> = s
                    .iter()
                    .map(|p| InkPoint {
                        x: p.x,
                        y: p.y,
                        width,
                    })
                    .collect();
                let m = mirror_stroke(&pts, axis);
                for copy in m.copies {
                    mirrored_groups.push(
                        copy.into_iter()
                            .map(|p| PagePoint { x: p.x, y: p.y })
                            .collect(),
                    );
                }
            }
            if !mirrored_groups.is_empty() {
                let copy_annot = Annotation::new(
                    annot.page,
                    Shape::Ink(mirrored_groups),
                    annot.style,
                    &annot.author,
                );
                changes.push(Change::CreateAnnotation(copy_annot));
            }
        }
        if !changes.is_empty() {
            self.editing
                .apply(&Operation::new(t!("command.symmetry"), changes));
        }
    }
}

/// Dialog: Export Annotation Report (Markdown, HTML, CSV, PDF summary) (plan A.26).
pub(crate) fn report_export_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.report_open.set(false);
    let format_idx = signal(0_u8); // 0=Markdown, 1=HTML, 2=CSV, 3=PDF
    let include_crops = signal(false);
    let status = signal(String::new());

    let export_action = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let path = doc.path().to_path_buf();
        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Document")
            .to_owned();
        let fmt = match untracked(|| format_idx.get()) {
            1 => ReportFormat::Html,
            2 => ReportFormat::Csv,
            3 => ReportFormat::Pdf,
            _ => ReportFormat::Markdown,
        };
        let crops = untracked(|| include_crops.get());
        let config = ReportConfig {
            title,
            file_path: path.clone(),
            filter: None,
            include_crops: crops,
        };
        let annots = collect_all_annotations(state);
        let mut items = build_report_items(&annots, &config);
        if crops && let Ok(shared) = doc.mupdf() {
            let mupdf = shared.lock().expect("document lock poisoned");
            attach_page_crops(&mupdf, &annots, &mut items, 120);
        }

        let (ext, bytes_res): (&str, Result<Vec<u8>, String>) = match fmt {
            ReportFormat::Markdown => ("md", Ok(export_markdown(&config, &items).into_bytes())),
            ReportFormat::Html => ("html", Ok(export_html(&config, &items).into_bytes())),
            ReportFormat::Csv => ("csv", Ok(export_csv(&items).into_bytes())),
            ReportFormat::Pdf => (
                "pdf",
                export_pdf_summary(&config, &items).map_err(|e| e.to_string()),
            ),
            _ => ("md", Ok(export_markdown(&config, &items).into_bytes())),
        };
        let bytes = match bytes_res {
            Ok(b) => b,
            Err(err) => {
                status.set(err);
                return;
            }
        };
        let dialog = SaveDialog::new()
            .title(t!("command.annotation_report"))
            .filter(FileFilter::new(t!("command.annotation_report"), [ext]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(dest)) = answer.await
                && write_atomically(&dest, &bytes).is_ok()
            {
                close();
            }
        });
    };

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.annotation_report"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("annotate.report.format"))
        .child(
            segmented()
                .child(segment_of(
                    move || String::from("Markdown"),
                    move || format_idx.get() == 0,
                    move || format_idx.set(0),
                ))
                .child(segment_of(
                    move || String::from("HTML"),
                    move || format_idx.get() == 1,
                    move || format_idx.set(1),
                ))
                .child(segment_of(
                    move || String::from("CSV"),
                    move || format_idx.get() == 2,
                    move || format_idx.set(2),
                ))
                .child(segment_of(
                    move || String::from("PDF"),
                    move || format_idx.get() == 3,
                    move || format_idx.set(3),
                )),
        )
        .child(segment_of(
            move || t!("annotate.report.include_crops"),
            move || include_crops.get(),
            move || include_crops.update(|v| *v = !*v),
        ))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "annotate.report.export",
                    export_action,
                )),
        );
    list_dialog(t!("command.annotation_report"), close, card(body))
}

/// Dialog: Annotation History Timeline Scrubber & non-destructive restore (plan A.27).
pub(crate) fn history_timeline_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.history_open.set(false);
    let timeline = stored(None::<AnnotationTimeline>);
    let selected_step = signal(0_usize);
    let entries_len = signal(0_usize);
    let status = signal(String::new());

    if state.editing.document().is_some() {
        let log_dir = std::env::temp_dir().join("ecclesiapdf-oplog");
        if let Ok(read_dir) = std::fs::read_dir(&log_dir) {
            let newest = read_dir
                .flatten()
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "oplog"))
                .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok());
            if let Some(entry) = newest {
                match AnnotationTimeline::from_file(&entry.path(), &[]) {
                    Ok(tl) => {
                        let len = tl.entries().len();
                        entries_len.set(len);
                        selected_step.set(len);
                        timeline.with(|slot| *slot = Some(tl));
                    }
                    Err(err) => status.set(err.to_string()),
                }
            }
        }
    }

    let restore_selected = move || {
        let step = untracked(|| selected_step.get());
        let current = collect_all_annotations(state);
        let op = timeline.with(|slot| {
            let tl = slot.as_ref()?;
            let ts = tl
                .entries()
                .get(step.saturating_sub(1))
                .map_or(0, |e| e.timestamp_ms);
            tl.restore_document_at_time(&current, ts)
        });
        if let Some(op) = op {
            state.editing.apply(&op);
            close();
        } else {
            status.set(t!("annotate.history.no_diff"));
        }
    };

    let body = view()
        .w(px(560.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.annotation_history"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(scroll().max_h(px(240.0)).child(dynamic(move || {
            let count = entries_len.get();
            if count == 0 {
                return view().p_3().child(
                    text(move || t!("annotate.history.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            timeline.with(|slot| {
                let Some(tl) = slot.as_ref() else {
                    return view();
                };
                tl.entries()
                    .iter()
                    .enumerate()
                    .fold(view().gap_1(), |col, (idx, e)| {
                        let step_no = idx + 1;
                        let label = format!("#{} — {} ({})", step_no, e.name, e.affected_ids.len());
                        col.child(segment_owned(
                            label,
                            move || selected_step.get() == step_no,
                            move || selected_step.set(step_no),
                        ))
                    })
            })
        })))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("ink_tools.done", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "annotate.history.restore",
                    restore_selected,
                )),
        );
    list_dialog(t!("command.annotation_history"), close, card(body))
}

/// Dialog: Pattern Brushes picker & preview (plan H.20).
pub(crate) fn pattern_brush_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.pattern_open.set(false);
    let kind_idx = signal(0_u8);
    let preview_stream = signal(String::new());

    let update_preview = move || {
        let kind = match untracked(|| kind_idx.get()) {
            1 => PatternKind::Dashes,
            2 => PatternKind::Crosshatch,
            3 => PatternKind::Zigzag,
            4 => PatternKind::Chain,
            5 => PatternKind::Stars,
            _ => PatternKind::Dots,
        };
        let brush = PatternBrush::new(kind);
        let sample = vec![
            InkPoint {
                x: 20.0,
                y: 20.0,
                width: 4.0,
            },
            InkPoint {
                x: 120.0,
                y: 20.0,
                width: 4.0,
            },
        ];
        if let Ok(stream) = to_pdf_appearance_stream(&sample, &brush, 792.0) {
            preview_stream.set(stream.lines().take(3).collect::<Vec<_>>().join(" "));
        }
    };
    update_preview();

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.pattern_brush"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.pattern.dots"),
                    move || kind_idx.get() == 0,
                    move || {
                        kind_idx.set(0);
                        update_preview();
                    },
                ))
                .child(segment_of(
                    move || t!("ink_tools.pattern.dashes"),
                    move || kind_idx.get() == 1,
                    move || {
                        kind_idx.set(1);
                        update_preview();
                    },
                ))
                .child(segment_of(
                    move || t!("ink_tools.pattern.crosshatch"),
                    move || kind_idx.get() == 2,
                    move || {
                        kind_idx.set(2);
                        update_preview();
                    },
                ))
                .child(segment_of(
                    move || t!("ink_tools.pattern.zigzag"),
                    move || kind_idx.get() == 3,
                    move || {
                        kind_idx.set(3);
                        update_preview();
                    },
                ))
                .child(segment_of(
                    move || t!("ink_tools.pattern.chain"),
                    move || kind_idx.get() == 4,
                    move || {
                        kind_idx.set(4);
                        update_preview();
                    },
                ))
                .child(segment_of(
                    move || t!("ink_tools.pattern.stars"),
                    move || kind_idx.get() == 5,
                    move || {
                        kind_idx.set(5);
                        update_preview();
                    },
                )),
        )
        .child(
            text(move || preview_stream.get())
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.pattern_brush"), close, card(body))
}

/// Dialog: Symmetry axis & mirror strokes tool (plan H.21).
pub(crate) fn symmetry_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.symmetry_open.set(false);
    let axis_idx = signal(0_u8); // 0=Vertical, 1=Horizontal, 2=Cross, 3=Radial6

    let apply = move || {
        let axis = match untracked(|| axis_idx.get()) {
            1 => SymmetryAxis::Horizontal { y: 396.0 },
            2 => SymmetryAxis::Cross { x: 306.0, y: 396.0 },
            3 => SymmetryAxis::Radial {
                center: ecclesia_ink::erase::Point { x: 306.0, y: 396.0 },
                spokes: 6,
                mode: RadialMode::Kaleidoscope,
            },
            _ => SymmetryAxis::Vertical { x: 306.0 },
        };
        let _guides = guide_segments(axis, 612.0, 792.0);
        state.apply_symmetry_to_selection(axis);
        close();
    };

    let body = view()
        .w(px(460.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.symmetry"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.symmetry.vertical"),
                    move || axis_idx.get() == 0,
                    move || axis_idx.set(0),
                ))
                .child(segment_of(
                    move || t!("ink_tools.symmetry.horizontal"),
                    move || axis_idx.get() == 1,
                    move || axis_idx.set(1),
                ))
                .child(segment_of(
                    move || t!("ink_tools.symmetry.cross"),
                    move || axis_idx.get() == 2,
                    move || axis_idx.set(2),
                ))
                .child(segment_of(
                    move || t!("ink_tools.symmetry.radial"),
                    move || axis_idx.get() == 3,
                    move || axis_idx.set(3),
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
    list_dialog(t!("command.symmetry"), close, card(body))
}

/// Dialog: Searchable Handwriting FTS5 search & page jump (plan H.23).
pub(crate) fn handwriting_search_dialog(
    state: AnnotateState,
    go_to: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.hw_search_open.set(false);
    let query = signal(String::new());
    let hits = signal(Vec::<(usize, String)>::new());
    let status = signal(String::new());

    let run_index_and_search = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let doc_id = doc.path().to_string_lossy().into_owned();
        let mut idx = match settings::folder() {
            Some(dir) => ecclesia_store::rusqlite::Connection::open(dir.join("handwriting.sqlite"))
                .map_err(ecclesia_features::handwriting::IndexError::from)
                .and_then(HandwritingIndex::from_connection),
            None => HandwritingIndex::in_memory(),
        }
        .unwrap_or_else(|_| HandwritingIndex::in_memory().expect("in-memory sqlite"));
        let recognizer = OfflineStrokeRecognizer::new();
        let page = untracked(|| state.current_page.get());
        let ink = page_ink_annotations(state, page);
        let (strokes, _) = to_hw_strokes(&ink);
        if !strokes.is_empty() {
            let _ = idx.index_page(&doc_id, page, &strokes, &recognizer, "en");
        }
        let q = untracked(|| query.get());
        match idx.search(&q, Some(&doc_id)) {
            Ok(found) => {
                status.set(t!("library.hits_count", count = found.len()));
                hits.set(
                    found
                        .into_iter()
                        .take(30)
                        .map(|h| (h.page, h.word))
                        .collect(),
                );
            }
            Err(err) => status.set(err.to_string()),
        }
    };

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.handwriting_search"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            view()
                .flex_row()
                .gap_2()
                .child(
                    field(input().value(move || query.get()))
                        .flex_1()
                        .on_change(move |s: &str| query.set(s.to_owned()))
                        .placeholder(t!("ink_tools.hw_search.placeholder"))
                        .label(t!("command.handwriting_search"))
                        .auto_focus(true),
                )
                .child(crate::shell::primary_button(
                    "library.search.run",
                    run_index_and_search,
                )),
        )
        .child(
            text(move || status.get())
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(scroll().max_h(px(200.0)).child(dynamic(move || {
            hits.get()
                .into_iter()
                .fold(view().gap_1(), |col, (pg, word)| {
                    let label = format!("p. {} — {}", pg + 1, word);
                    let a11y = label.clone();
                    col.child(
                        nsh_base::button()
                            .accessibility_label(a11y)
                            .h(px(28.0))
                            .px_2()
                            .rounded(px(6.0))
                            .hover(|s| s.bg(move || theme().muted))
                            .on_press(move || {
                                close();
                                go_to(pg);
                            })
                            .child(
                                text(label)
                                    .text_size(px(12.0))
                                    .text_color(move || theme().foreground),
                            ),
                    )
                })
        })))
        .child(view().flex_row().justify_end().child(action_button(
            "ink_tools.done",
            Tone::Plain,
            close,
        )));
    list_dialog(t!("command.handwriting_search"), close, card(body))
}

/// Dialog: Convert Lassoed Handwriting to FreeText or Clipboard Text (plan H.24).
pub(crate) fn handwriting_convert_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.hw_convert_open.set(false);
    let preview = signal(String::new());
    let replace_ink = signal(true);

    let run_recognize = move || {
        let page = untracked(|| state.current_page.get());
        let ink = page_ink_annotations(state, page);
        let (strokes, ids) = to_hw_strokes(&ink);
        if strokes.is_empty() {
            preview.set(t!("ink_tools.hw_convert.empty"));
            return;
        }
        let recognizer = OfflineStrokeRecognizer::new();
        let target = if untracked(|| replace_ink.get()) {
            ConversionTarget::ReplaceWithFreeText
        } else {
            ConversionTarget::CopyTextOnly
        };
        let auth = author();
        let opts = ConvertOptions {
            page,
            lang: "en",
            target,
            style: Style::default(),
            author: &auth,
        };
        match convert_lasso_to_text(&strokes, &ids, &recognizer, opts) {
            Ok(outcome) => {
                preview.set(outcome.text.clone());
                if let Some(op) = outcome.operation {
                    state.editing.apply(&op);
                    close();
                }
            }
            Err(err) => preview.set(err.to_string()),
        }
    };

    let body = view()
        .w(px(480.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.handwriting_to_text"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("ink_tools.hw_convert.replace"),
                    move || replace_ink.get(),
                    move || replace_ink.set(true),
                ))
                .child(segment_of(
                    move || t!("ink_tools.hw_convert.copy_only"),
                    move || !replace_ink.get(),
                    move || replace_ink.set(false),
                )),
        )
        .child(
            text(move || preview.get())
                .text_size(px(13.0))
                .text_color(move || theme().foreground),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    "ink_tools.hw_convert.run",
                    run_recognize,
                )),
        );
    list_dialog(t!("command.handwriting_to_text"), close, card(body))
}

/// Dialog: OCR to Searchable PDF (plan O.4).
pub(crate) fn ocr_dialog(
    state: AnnotateState,
    reload_path: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.ocr_open.set(false);
    let lang_idx = signal(0_usize);
    let skip_text = signal(true);
    let status = signal(String::new());
    let packs = standard_language_packs();

    let run_ocr = move || {
        let Some(doc) = state.editing.document() else {
            return;
        };
        let src = doc.path().to_path_buf();
        let all_packs = standard_language_packs();
        let lang = all_packs
            .get(untracked(|| lang_idx.get()))
            .map_or("eng", |p| p.code.as_str())
            .to_owned();
        let policy = if untracked(|| skip_text.get()) {
            ExistingTextPolicy::SkipPagesWithText
        } else {
            ExistingTextPolicy::ForceAllPages
        };
        let out_path = src.with_extension("ocr.pdf");
        let engine = AutoOcrEngine::default();
        let cfg = OcrConfig {
            languages: vec![lang],
            existing_text: policy,
            ..OcrConfig::default()
        };
        match ocr_pdf_file(&src, &out_path, &engine, &cfg) {
            Ok(report) => {
                status.set(t!(
                    "ocr.done",
                    ocr = report.pages_ocred,
                    skipped = report.page_count.saturating_sub(report.pages_ocred)
                ));
                close();
                reload_path(out_path);
            }
            Err(err) => status.set(err.to_string()),
        }
    };

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.ocr_document"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("ocr.language"))
        .child(
            packs
                .iter()
                .take(4)
                .enumerate()
                .fold(segmented(), |bar, (idx, pack)| {
                    let name = pack.display_name.clone();
                    bar.child(segment_owned(
                        name,
                        move || lang_idx.get() == idx,
                        move || lang_idx.set(idx),
                    ))
                }),
        )
        .child(segment_of(
            move || t!("ocr.skip_text_pages"),
            move || skip_text.get(),
            move || skip_text.update(|v| *v = !*v),
        ))
        .child(dynamic(move || {
            let s = status.get();
            if s.is_empty() {
                view()
            } else {
                view().child(
                    text(s)
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
            }
        }))
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button("ocr.run", run_ocr)),
        );
    list_dialog(t!("command.ocr_document"), close, card(body))
}
