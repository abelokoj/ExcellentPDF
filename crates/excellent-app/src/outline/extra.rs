//! Outline Phase 5 integrations:
//! - B.8 Generate Outline from headings, numbering rules, & structure tags with interactive preview
//! - B.11 Import & Export Outlines (Indented text, JSON, pdftk, Markdown)
//! - B.13 Non-PDF Custom Sidecar Outline persistence (`SidecarOutlineStore`)

use std::path::{Path, PathBuf};

use excellent_core::edit::Change;
use excellent_features::outline::Entry;
use excellent_features::outline_extra::{
    GenerateOptions, OutlineFormat, OutlinePreviewTree, SidecarOutlineStore, export_indented_text,
    export_json, export_markdown, export_pdftk,
    generate::{NodeDecision, generate_from_layouts},
    import_indented_text, import_json, import_markdown, import_pdftk,
};
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog, SaveDialog};
use nsh_core::prelude::*;

use super::OutlineState;
use crate::bookmarks::{caption, field};
use crate::list_dialog::{card, list_dialog};
use crate::settings::{self, write_atomically};
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// Per-pane state for Outline Generate (B.8) & Outline Import/Export (B.11) dialogs.
#[derive(Clone, Copy)]
pub(crate) struct OutlineExtraState {
    outline: OutlineState,
    generate_open: Signal<bool>,
    io_open: Signal<bool>,
}

impl OutlineExtraState {
    pub(crate) fn new(outline: OutlineState) -> Self {
        Self {
            outline,
            generate_open: signal(false),
            io_open: signal(false),
        }
    }

    pub(crate) fn any_dialog_open(self) -> bool {
        self.generate_open.get() || self.io_open.get()
    }

    pub(crate) fn is_generate_open(self) -> bool {
        self.generate_open.get()
    }
    pub(crate) fn is_io_open(self) -> bool {
        self.io_open.get()
    }

    pub(crate) fn open_generate(self) {
        self.generate_open.set(true);
    }
    pub(crate) fn open_io(self) {
        self.io_open.set(true);
    }
}

/// Sidecar outline store under `settings::folder()/outlines` for non-PDF documents (plan B.13).
pub(crate) fn sidecar_store() -> SidecarOutlineStore {
    let root = settings::folder()
        .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-outlines"))
        .join("outlines");
    SidecarOutlineStore::new(root)
}

fn is_pdf_file(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("pdf"))
}

fn apply_entries(outline: OutlineState, path: Option<PathBuf>, entries: Vec<Entry>, label: String) {
    if let Some(ref p) = path
        && !is_pdf_file(p)
    {
        let _ = sidecar_store().save(p, &entries);
        outline.entries.set(Some(entries));
        return;
    }
    let existing = outline.entries.get().unwrap_or_default();
    let mut changes = Vec::new();
    for i in (0..existing.len()).rev() {
        changes.push(Change::DeleteOutlineEntry { at: vec![i] });
    }
    fn push_inserts(entries: &[Entry], prefix: &[usize], out: &mut Vec<Change>) {
        for (idx, e) in entries.iter().enumerate() {
            let mut at = prefix.to_vec();
            at.push(idx);
            let uri = e
                .uri
                .clone()
                .or_else(|| e.page.map(|p| format!("#page={}", p + 1)))
                .unwrap_or_default();
            out.push(Change::InsertOutlineEntry {
                at: at.clone(),
                title: e.title.clone(),
                uri,
            });
            push_inserts(&e.children, &at, out);
        }
    }
    push_inserts(&entries, &[], &mut changes);
    if !changes.is_empty() {
        outline.edit(&existing, label, changes);
    }
}

/// Generate Outline Preview dialog (plan B.8).
pub(crate) fn generate_outline_dialog(
    state: OutlineExtraState,
    path: Option<PathBuf>,
) -> impl IntoElement {
    let close = move || state.generate_open.set(false);
    let preview = signal(None::<OutlinePreviewTree>);
    let status = signal(String::new());

    if let Some(ref p) = path
        && let Ok(doc) = excellent_mupdf::Document::open(p)
    {
        let count = doc.page_count().unwrap_or(0).min(120);
        let mut layouts = Vec::with_capacity(count);
        for idx in 0..count {
            if let Ok(pg) = doc.load_page(idx)
                && let Ok(stext) = pg.text(None)
            {
                layouts.push((idx, stext.layout()));
            }
        }
        let layout_refs: Vec<(usize, &excellent_mupdf::TextLayout)> =
            layouts.iter().map(|(i, l)| (*i, l)).collect();
        let tree = generate_from_layouts(&layout_refs, &GenerateOptions::default());
        preview.set(Some(tree));
    }

    let apply_action = {
        let p = path.clone();
        move || {
            let Some(tree) = untracked(|| preview.get()) else {
                return;
            };
            let entries = tree.finalize();
            apply_entries(
                state.outline,
                p.clone(),
                entries,
                t!("command.generate_outline"),
            );
            close();
        }
    };

    let body = view()
        .w(px(580.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.generate_outline"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(scroll().max_h(px(280.0)).child(dynamic(move || {
            let Some(tree) = preview.get() else {
                return view().p_3().child(
                    text(move || t!("outline.generate.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            };
            if tree.roots.is_empty() {
                return view().p_3().child(
                    text(move || t!("outline.generate.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            tree.roots
                .into_iter()
                .enumerate()
                .fold(view().gap_1(), |col, (idx, node)| {
                    let label = format!(
                        "{} (p. {})",
                        node.entry.title,
                        node.entry.page.map_or(0, |p| p + 1)
                    );
                    let keep = node.decision == NodeDecision::Accept;
                    col.child(
                        view()
                            .flex_row()
                            .items_center()
                            .gap_2()
                            .child(segment_of(
                                move || {
                                    if keep {
                                        t!("outline.generate.keep")
                                    } else {
                                        t!("outline.generate.skip")
                                    }
                                },
                                move || keep,
                                move || {
                                    preview.update(|maybe| {
                                        if let Some(t) = maybe.as_mut()
                                            && let Some(n) = t.roots.get_mut(idx)
                                        {
                                            n.decision = if n.decision == NodeDecision::Accept {
                                                NodeDecision::Discard
                                            } else {
                                                NodeDecision::Accept
                                            };
                                        }
                                    });
                                },
                            ))
                            .child(
                                text(label)
                                    .flex_1()
                                    .text_size(px(12.0))
                                    .text_color(move || theme().foreground),
                            ),
                    )
                })
        })))
        .child(dynamic(move || {
            let msg = status.get();
            if msg.is_empty() {
                view()
            } else {
                view().child(
                    text(msg)
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
                    "outline.generate.apply",
                    apply_action,
                )),
        );
    list_dialog(t!("command.generate_outline"), close, card(body))
}

/// Import & Export Outline dialog in Indented Text, JSON, pdftk, and Markdown (plans B.11, B.13).
pub(crate) fn outline_io_dialog(
    state: OutlineExtraState,
    path: Option<PathBuf>,
) -> impl IntoElement {
    let close = move || state.io_open.set(false);
    let fmt_idx = signal(0_u8); // 0=IndentedText, 1=Json, 2=Pdftk, 3=Markdown
    let text_draft = signal(String::new());
    let status = signal(String::new());

    let selected_format = move || match untracked(|| fmt_idx.get()) {
        1 => OutlineFormat::Json,
        2 => OutlineFormat::Pdftk,
        3 => OutlineFormat::Markdown,
        _ => OutlineFormat::IndentedText,
    };

    let refresh_preview = {
        let p = path.clone();
        move || {
            let entries = state
                .outline
                .entries
                .get()
                .or_else(|| {
                    p.as_ref()
                        .and_then(|fp| sidecar_store().load(fp).ok().flatten())
                })
                .unwrap_or_default();
            let rendered = match selected_format() {
                OutlineFormat::Json => export_json(&entries).unwrap_or_default(),
                OutlineFormat::Pdftk => export_pdftk(&entries),
                OutlineFormat::Markdown => export_markdown(&entries),
                OutlineFormat::IndentedText => export_indented_text(&entries),
            };
            text_draft.set(rendered);
        }
    };
    refresh_preview();
    let r0 = refresh_preview.clone();
    let r1 = refresh_preview.clone();
    let r2 = refresh_preview.clone();
    let r3 = refresh_preview;

    let import_action = {
        let p = path.clone();
        move || {
            let raw = untracked(|| text_draft.get());
            let parsed = match selected_format() {
                OutlineFormat::Json => import_json(&raw),
                OutlineFormat::Pdftk => import_pdftk(&raw),
                OutlineFormat::Markdown => import_markdown(&raw),
                OutlineFormat::IndentedText => import_indented_text(&raw),
            };
            match parsed {
                Ok(entries) => {
                    apply_entries(state.outline, p.clone(), entries, t!("command.outline_io"));
                    status.set(t!("outline.io.imported"));
                    close();
                }
                Err(err) => status.set(err.to_string()),
            }
        }
    };

    let load_file = move || {
        let dialog = OpenDialog::files()
            .title(t!("command.outline_io"))
            .filter(FileFilter::new(
                t!("command.outline_io"),
                ["txt", "json", "md"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(p) = paths.into_iter().next()
                && let Ok(txt) = std::fs::read_to_string(&p)
            {
                text_draft.set(txt);
            }
        });
    };

    let save_file = move || {
        let content = untracked(|| text_draft.get());
        let ext = match selected_format() {
            OutlineFormat::Json => "json",
            OutlineFormat::Markdown => "md",
            _ => "txt",
        };
        let dialog = SaveDialog::new()
            .title(t!("command.outline_io"))
            .filter(FileFilter::new(t!("command.outline_io"), [ext]));
        let answer = pick_save_path(dialog);
        spawn(async move {
            if let Ok(Some(p)) = answer.await {
                let _ = write_atomically(&p, content.as_bytes());
            }
        });
    };

    let body = view()
        .w(px(580.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.outline_io"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(caption("outline.io.format"))
        .child(
            segmented()
                .child(segment_of(
                    move || t!("outline.io.indented"),
                    move || fmt_idx.get() == 0,
                    move || {
                        fmt_idx.set(0);
                        r0();
                    },
                ))
                .child(segment_of(
                    move || String::from("JSON"),
                    move || fmt_idx.get() == 1,
                    move || {
                        fmt_idx.set(1);
                        r1();
                    },
                ))
                .child(segment_of(
                    move || String::from("pdftk"),
                    move || fmt_idx.get() == 2,
                    move || {
                        fmt_idx.set(2);
                        r2();
                    },
                ))
                .child(segment_of(
                    move || String::from("Markdown"),
                    move || fmt_idx.get() == 3,
                    move || {
                        fmt_idx.set(3);
                        r3();
                    },
                )),
        )
        .child(
            field(input().value(move || text_draft.get()))
                .multiline(true)
                .h(px(190.0))
                .on_change(move |s: &str| text_draft.set(s.to_owned()))
                .placeholder(t!("outline.io.placeholder"))
                .label(t!("command.outline_io")),
        )
        .child(dynamic(move || {
            let msg = status.get();
            if msg.is_empty() {
                view()
            } else {
                view().child(
                    text(msg)
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
                .child(action_button(
                    "outline.io.load_file",
                    Tone::Plain,
                    load_file,
                ))
                .child(action_button(
                    "outline.io.save_file",
                    Tone::Plain,
                    save_file,
                ))
                .child(crate::shell::primary_button(
                    "outline.io.import",
                    import_action,
                )),
        );
    list_dialog(t!("command.outline_io"), close, card(body))
}
