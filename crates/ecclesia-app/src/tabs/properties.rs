//! Document properties dialog opened from a tab's right-click context menu.
//!
//! Displays the file's name, folder, full path, size, page count, and (for PDF files) `/Info`
//! dictionary metadata entries (`Title`, `Author`, `Subject`, `Keywords`, `Creator`, `Producer`)
//! loaded off the UI thread so large documents never stall the interface.

use std::path::Path;

use ecclesia_core::shared::registry;
use nsh_base::theme;
use nsh_core::prelude::*;

use super::{TabId, TabState, Tone, action_button};
use crate::list_dialog::{card, list_dialog};

/// Standard PDF `/Info` keys shown in SumatraPDF Enhanced order.
const PDF_META_KEYS: [(&str, &str); 6] = [
    ("Title", "tabs.properties.meta.title"),
    ("Author", "tabs.properties.meta.author"),
    ("Subject", "tabs.properties.meta.subject"),
    ("Keywords", "tabs.properties.meta.keywords"),
    ("Creator", "tabs.properties.meta.creator"),
    ("Producer", "tabs.properties.meta.producer"),
];

/// Snapshot of file and PDF metadata shown inside the Properties dialog.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct DocProperties {
    file_name: String,
    folder: String,
    full_path: String,
    file_size: Option<String>,
    page_count: Option<usize>,
    metadata: Vec<(&'static str, String)>,
    loaded: bool,
}

/// Modal document properties dialog for [`TabState::properties_tab`].
pub(crate) fn properties_dialog(state: TabState) -> impl IntoElement {
    let close = move || state.close_properties();
    let id = untracked(|| state.properties_tab());
    effect(move || {
        if id.is_none_or(|id| state.tab_info(id).is_none()) {
            close();
        }
    });
    let content = match id {
        Some(id) => dialog_body(state, id, close),
        None => view(),
    };
    list_dialog(t!("tabs.properties.title"), close, card(content))
}

fn dialog_body(state: TabState, id: TabId, close: impl Fn() + Copy + 'static) -> View {
    let initial_tab = untracked(|| state.tab_info(id));
    let known_pages = untracked(|| state.page_count_for(id));
    let initial = initial_tab
        .as_ref()
        .map(|tab| base_properties(&tab.path, known_pages))
        .unwrap_or_default();
    let props = signal(initial);

    if let Some(tab) = initial_tab {
        let path = tab.path;
        spawn(async move {
            let loaded = blocking(move || load_properties(&path, known_pages)).await;
            if let Ok(info) = loaded {
                props.set(info);
            }
        });
    }

    let missing = move || state.tab_info(id).is_some_and(|tab| tab.missing);

    let header = view()
        .flex_row()
        .items_center()
        .gap_3()
        .px_5()
        .py(px(14.0))
        .border_b_1()
        .border_b_color(move || theme().border)
        .child(
            view()
                .flex_1()
                .min_w(px(0.0))
                .gap(px(2.0))
                .child(
                    text(move || t!("tabs.properties.title"))
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(
                    text(move || props.with(|p| p.file_name.clone()))
                        .truncate()
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                ),
        );

    let details = scroll().max_h(px(420.0)).child(
        view()
            .px_5()
            .py_4()
            .gap_3()
            .child(prop_row("tabs.properties.file_name", move || {
                props.with(|p| p.file_name.clone())
            }))
            .child(prop_row("tabs.properties.folder", move || {
                props.with(|p| p.folder.clone())
            }))
            .child(prop_row("tabs.properties.full_path", move || {
                props.with(|p| p.full_path.clone())
            }))
            .child(prop_row("tabs.properties.file_size", move || {
                props.with(|p| {
                    p.file_size
                        .clone()
                        .unwrap_or_else(|| t!("tabs.properties.unavailable"))
                })
            }))
            .child(prop_row("tabs.properties.pages", move || {
                props.with(|p| {
                    p.page_count.map(|n| n.to_string()).unwrap_or_else(|| {
                        if p.loaded {
                            t!("tabs.properties.unavailable")
                        } else {
                            t!("tabs.properties.loading")
                        }
                    })
                })
            }))
            .child(when(
                move || props.with(|p| !p.metadata.is_empty()),
                move || {
                    view()
                        .mt_1()
                        .pt_3()
                        .gap_3()
                        .border_t_1()
                        .border_t_color(move || theme().border)
                        .child(
                            text(move || t!("tabs.properties.metadata"))
                                .text_size(px(12.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(move || theme().muted_foreground),
                        )
                        .child(each(
                            move || props.with(|p| p.metadata.clone()),
                            |(key, _): &(&'static str, String)| *key,
                            |entry, _| {
                                let label_key = entry.with_untracked(|(key, _)| *key);
                                prop_row(label_key, move || entry.with(|(_, value)| value.clone()))
                            },
                        ))
                },
            )),
    );

    let footer = view()
        .flex_row()
        .items_center()
        .gap_2()
        .px_4()
        .py_3()
        .border_t_1()
        .border_t_color(move || theme().border)
        .child(action_button(
            "tabs.menu.copy_path",
            Tone::Plain,
            move || {
                state.copy_file_path(id);
            },
        ))
        .child(when(
            move || !missing(),
            move || {
                action_button("tabs.menu.show_in_folder", Tone::Plain, move || {
                    state.show_in_folder(id);
                })
            },
        ))
        .child(view().flex_1())
        .child(crate::shell::primary_button("tabs.properties.close", close));

    view()
        .w(px(500.0))
        .child(header)
        .child(details)
        .child(footer)
}

fn prop_row(label_key: &'static str, value: impl Fn() -> String + 'static) -> impl IntoElement {
    view()
        .flex_row()
        .items_start()
        .gap_3()
        .child(
            text(move || t!(label_key))
                .w(px(110.0))
                .flex_shrink(0.0)
                .text_size(px(12.0))
                .font_weight(FontWeight::MEDIUM)
                .text_color(move || theme().muted_foreground),
        )
        .child(
            text(value)
                .flex_1()
                .min_w(px(0.0))
                .text_size(px(13.0))
                .text_color(move || theme().popover_foreground),
        )
}

fn base_properties(path: &Path, known_pages: Option<usize>) -> DocProperties {
    let file_name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let folder = path
        .parent()
        .map(|p| p.display().to_string())
        .unwrap_or_default();
    let file_size = std::fs::metadata(path)
        .ok()
        .map(|meta| crate::packs::format_size(meta.len()));
    DocProperties {
        file_name,
        folder,
        full_path: path.display().to_string(),
        file_size,
        page_count: known_pages,
        metadata: Vec::new(),
        loaded: false,
    }
}

fn load_properties(path: &Path, known_pages: Option<usize>) -> DocProperties {
    let mut info = base_properties(path, known_pages);
    let resolved = crate::expdf::resolve_open_path(path);
    if info.page_count.is_none() {
        info.page_count = read_page_count(&resolved);
    }
    if is_pdf_path(&resolved)
        && let Ok(bytes) = std::fs::read(&resolved)
        && let Ok(entries) = ecclesia_mupdf::pdf_info_entries(&bytes)
    {
        info.metadata = select_metadata(&entries);
    }
    info.loaded = true;
    info
}

fn read_page_count(path: &Path) -> Option<usize> {
    let doc = registry().open(path).ok()?;
    let mupdf = doc.mupdf().ok()?;
    let count = mupdf
        .lock()
        .expect("document lock poisoned")
        .page_count()
        .ok()?;
    Some(count)
}

fn is_pdf_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.to_string_lossy().eq_ignore_ascii_case("pdf"))
}

fn select_metadata(entries: &[(String, String)]) -> Vec<(&'static str, String)> {
    let mut selected = Vec::new();
    for (pdf_key, label_key) in PDF_META_KEYS {
        if let Some((_, value)) = entries
            .iter()
            .find(|(k, v)| k.eq_ignore_ascii_case(pdf_key) && !v.trim().is_empty())
        {
            selected.push((label_key, value.trim().to_owned()));
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_standard_pdf_metadata_in_display_order_and_skips_empty() {
        let raw = vec![
            ("Producer".to_owned(), "MuPDF".to_owned()),
            ("Title".to_owned(), "  Annual Report  ".to_owned()),
            ("Author".to_owned(), "   ".to_owned()),
            ("Creator".to_owned(), "Typst".to_owned()),
            ("CustomKey".to_owned(), "Ignored".to_owned()),
        ];
        let picked = select_metadata(&raw);
        assert_eq!(
            picked,
            vec![
                ("tabs.properties.meta.title", "Annual Report".to_owned()),
                ("tabs.properties.meta.creator", "Typst".to_owned()),
                ("tabs.properties.meta.producer", "MuPDF".to_owned()),
            ]
        );
    }
}
