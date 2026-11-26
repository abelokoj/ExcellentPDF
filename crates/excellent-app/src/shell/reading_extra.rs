//! Reading Phase 5 UI integrations:
//! - R.16 Reflow (Liquid Mode) view & typography
//! - R.22 Bionic Reading toggle & strength controls
//! - R.24 Two-column auto-detect & column-scroll stepping
//! - R.27 SyncTeX forward/inverse search & editor preset settings
//! - R.29 Extra document formats viewer (DjVu, CHM, CBZ, CBR)
//! - R.30 Open URL with progressive download dialog

use std::fs;
use std::path::{Path, PathBuf};

use excellent_core::formats_extra::{
    ChmDocument, ComicArchive, DjVuDocument, ExtraDocument, ExtraFormat,
};
use excellent_core::remote::{RemoteStreamCache, UreqTransport};
use excellent_features::read::columns::{
    ColumnScrollNavigator, ColumnScrollTarget, detect_columns,
};
use excellent_features::reflow::bionic::BionicMode;
use excellent_features::reflow::{ReflowBlock, ReflowConfig, ReflowPage, ReflowTheme};
use excellent_features::synctex::{EditorPreset, SyncTexIndex};
use excellent_mupdf::PageRect;
use nsh_base::{input, theme};
use nsh_core::file_dialog::{FileFilter, OpenDialog};
use nsh_core::prelude::*;

use crate::bookmarks::field;
use crate::editing::show_message;
use crate::list_dialog::{card, list_dialog};
use crate::settings;
use crate::settings_window::{segment_of, segmented};
use crate::tabs::{Tone, action_button};

/// `settings.toml` keys for Bionic Reading & SyncTeX editor presets.
pub(crate) const BIONIC_ENABLED: &str = "bionic_reading";
pub(crate) const BIONIC_STRENGTH: &str = "bionic_strength";
pub(crate) const SYNCTEX_PRESET: &str = "synctex_editor_preset";
pub(crate) const SYNCTEX_CUSTOM_CMD: &str = "synctex_custom_command";

/// Per-pane Phase 5 reading state.
#[derive(Clone, Copy)]
pub(crate) struct ReadingExtraState {
    reflow_open: Signal<bool>,
    bionic_on: Signal<bool>,
    bionic_strength: Signal<f32>,
    reflow_theme_idx: Signal<u8>,
    reflow_font_pt: Signal<f32>,
    synctex_open: Signal<bool>,
    remote_open: Signal<bool>,
    extra_open: Signal<bool>,
    extra_path: Stored<Option<PathBuf>>,
    col_nav: Stored<Option<(usize, ColumnScrollNavigator)>>,
}

impl ReadingExtraState {
    pub(crate) fn new() -> Self {
        let bionic = settings::shared().flag(BIONIC_ENABLED).unwrap_or(false);
        let strength = settings::shared()
            .number(BIONIC_STRENGTH)
            .map_or(0.45, |v| (v as f32).clamp(0.25, 0.7));
        Self {
            reflow_open: signal(false),
            bionic_on: signal(bionic),
            bionic_strength: signal(strength),
            reflow_theme_idx: signal(0),
            reflow_font_pt: signal(14.0),
            synctex_open: signal(false),
            remote_open: signal(false),
            extra_open: signal(false),
            extra_path: stored(None),
            col_nav: stored(None),
        }
    }

    pub(crate) fn any_dialog_open(self) -> bool {
        self.reflow_open.get()
            || self.synctex_open.get()
            || self.remote_open.get()
            || self.extra_open.get()
    }

    pub(crate) fn is_reflow_open(self) -> bool {
        self.reflow_open.get()
    }
    pub(crate) fn is_bionic_on(self) -> bool {
        self.bionic_on.get()
    }
    pub(crate) fn is_synctex_open(self) -> bool {
        self.synctex_open.get()
    }
    pub(crate) fn is_remote_open(self) -> bool {
        self.remote_open.get()
    }
    pub(crate) fn is_extra_open(self) -> bool {
        self.extra_open.get()
    }

    pub(crate) fn toggle_reflow(self) {
        self.reflow_open.update(|v| *v = !*v);
    }

    pub(crate) fn toggle_bionic(self) {
        let next = !untracked(|| self.bionic_on.get());
        self.bionic_on.set(next);
        settings::mutate(|s| s.set_flag(BIONIC_ENABLED, Some(next)));
    }

    pub(crate) fn open_synctex(self) {
        self.synctex_open.set(true);
    }
    pub(crate) fn open_remote(self) {
        self.remote_open.set(true);
    }

    pub(crate) fn open_extra_file(self, path: PathBuf) {
        self.extra_path.with(|slot| *slot = Some(path));
        self.extra_open.set(true);
    }

    /// Steps down through detected two-column layout on `path` at `page` (plan R.24).
    pub(crate) fn step_two_column_scroll(
        self,
        path: Option<PathBuf>,
        page: usize,
        go_to: impl Fn(usize),
    ) {
        let Some(doc_path) = path else {
            return;
        };
        let target = self.col_nav.with(|slot| {
            let needs_init = !matches!(slot, Some((p, _)) if *p == page);
            if needs_init {
                let Ok(doc) = excellent_mupdf::Document::open(&doc_path) else {
                    return None;
                };
                let Ok(p) = doc.load_page(page) else {
                    return None;
                };
                let bounds = p.bounds().unwrap_or(PageRect {
                    x0: 0.0,
                    y0: 0.0,
                    x1: 612.0,
                    y1: 792.0,
                });
                let Ok(text) = p.text(None) else {
                    return None;
                };
                let analysis = detect_columns(&text.layout(), bounds);
                let nav = ColumnScrollNavigator::new(analysis.layout, bounds.height() * 0.45);
                *slot = Some((page, nav));
            }
            slot.as_mut().map(|(_, nav)| nav.step_down(140.0))
        });

        match target {
            Some(ColumnScrollTarget::AdvanceToNextPage) => go_to(page + 1),
            Some(ColumnScrollTarget::JumpToNextColumn { column, .. }) => {
                show_message(
                    t!("reading.columns.title"),
                    t!("reading.columns.jumped", column = column + 1),
                );
            }
            Some(ColumnScrollTarget::ScrollInColumn { .. }) | None => {}
        }
    }
}

/// Reflow (Liquid Mode) & Bionic Reading modal dialog (plans R.16, R.22).
pub(crate) fn reflow_dialog(
    state: ReadingExtraState,
    path: Option<PathBuf>,
    page: usize,
) -> impl IntoElement {
    let close = move || state.reflow_open.set(false);
    let blocks_sig = signal(Vec::<ReflowBlock>::new());

    if let Some(ref p) = path
        && let Ok(doc) = excellent_mupdf::Document::open(p)
        && let Ok(pg) = doc.load_page(page)
        && let Ok(stext) = pg.text(None)
    {
        let bounds = pg.bounds().unwrap_or(PageRect {
            x0: 0.0,
            y0: 0.0,
            x1: 612.0,
            y1: 792.0,
        });
        let extracted = ReflowPage::extract(page, &stext.layout(), bounds, &[]);
        blocks_sig.set(extracted.blocks);
    }

    let body = view()
        .w(px(640.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("command.reflow_mode"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(segment_of(
                    move || t!("command.bionic_reading"),
                    move || state.bionic_on.get(),
                    move || state.toggle_bionic(),
                ))
                .child(action_button(
                    "reading.reflow.smaller",
                    Tone::Plain,
                    move || {
                        state
                            .reflow_font_pt
                            .update(|sz| *sz = (*sz - 1.0).max(10.0))
                    },
                ))
                .child(action_button(
                    "reading.reflow.larger",
                    Tone::Plain,
                    move || {
                        state
                            .reflow_font_pt
                            .update(|sz| *sz = (*sz + 1.0).min(26.0))
                    },
                )),
        )
        .child(
            segmented()
                .child(segment_of(
                    move || t!("reading.reflow.light"),
                    move || state.reflow_theme_idx.get() == 0,
                    move || state.reflow_theme_idx.set(0),
                ))
                .child(segment_of(
                    move || t!("reading.reflow.sepia"),
                    move || state.reflow_theme_idx.get() == 1,
                    move || state.reflow_theme_idx.set(1),
                ))
                .child(segment_of(
                    move || t!("reading.reflow.dark"),
                    move || state.reflow_theme_idx.get() == 2,
                    move || state.reflow_theme_idx.set(2),
                )),
        )
        .child(scroll().max_h(px(340.0)).child(dynamic(move || {
            let font_sz = state.reflow_font_pt.get();
            let bionic = if state.bionic_on.get() {
                BionicMode::On {
                    strength: state.bionic_strength.get(),
                }
            } else {
                BionicMode::Off
            };
            let reflow_theme = match state.reflow_theme_idx.get() {
                1 => ReflowTheme::Sepia,
                2 => ReflowTheme::Dark,
                _ => ReflowTheme::Light,
            };
            let (bg_hex, fg_hex) = reflow_theme.css_colors();
            let bg_u32 =
                u32::from_str_radix(bg_hex.trim_start_matches('#'), 16).unwrap_or(0xFFFFFF);
            let fg_u32 =
                u32::from_str_radix(fg_hex.trim_start_matches('#'), 16).unwrap_or(0x1A1A1A);
            let list = blocks_sig.get();
            if list.is_empty() {
                return view().p_3().child(
                    text(move || t!("reading.reflow.empty"))
                        .text_size(px(12.0))
                        .text_color(move || theme().muted_foreground),
                );
            }
            let cfg = ReflowConfig {
                font_size_pt: font_sz,
                theme: reflow_theme,
                bionic,
                ..ReflowConfig::default()
            };
            let _ = cfg;
            list.into_iter().fold(
                view()
                    .p_4()
                    .gap_2()
                    .rounded(px(8.0))
                    .bg(Hsla::from(rgb(bg_u32))),
                |col, blk| {
                    let display_text = match bionic {
                        BionicMode::On { .. } => {
                            format_bionic_plain(&blk.plain_text, state.bionic_strength.get())
                        }
                        BionicMode::Off => blk.plain_text.clone(),
                    };
                    if display_text.trim().is_empty() {
                        return col;
                    }
                    col.child(
                        text(display_text)
                            .text_size(px(font_sz))
                            .text_color(Hsla::from(rgb(fg_u32))),
                    )
                },
            )
        })))
        .child(
            view()
                .flex_row()
                .justify_end()
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.reflow_mode"), close, card(body))
}

fn format_bionic_plain(input: &str, strength: f32) -> String {
    let ratio = strength.clamp(0.25, 0.7);
    input
        .split_whitespace()
        .map(|w| {
            let chars: Vec<char> = w.chars().collect();
            let prefix_len = ((chars.len() as f32 * ratio).ceil() as usize).clamp(1, chars.len());
            let head: String = chars[..prefix_len].iter().collect();
            let tail: String = chars[prefix_len..].iter().collect();
            format!("*{head}*{tail}")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Configured SyncTeX editor preset from `settings.toml` (R.27).
pub(crate) fn configured_editor_preset() -> EditorPreset {
    let live = settings::shared();
    match live.text(SYNCTEX_PRESET) {
        Some("vim") => EditorPreset::Vim,
        Some("emacs") => EditorPreset::Emacs,
        Some("texstudio") => EditorPreset::TexStudio,
        Some("custom") => {
            let cmd = live
                .text(SYNCTEX_CUSTOM_CMD)
                .unwrap_or("code --goto \"%f:%l\"");
            EditorPreset::Custom(cmd.to_owned())
        }
        _ => EditorPreset::VsCode,
    }
}

/// SyncTeX Forward & Inverse Search dialog (plan R.27).
pub(crate) fn synctex_dialog(
    state: ReadingExtraState,
    pdf_path: Option<PathBuf>,
    current_page: usize,
    go_to: impl Fn(usize) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.synctex_open.set(false);
    let tex_file = signal(String::new());
    let line_num = signal("1".to_owned());
    let result_msg = signal(String::new());

    let run_forward = {
        let p = pdf_path.clone();
        move || {
            let Some(ref pdf) = p else {
                return;
            };
            let line = untracked(|| line_num.get())
                .trim()
                .parse::<u32>()
                .unwrap_or(1);
            let src = PathBuf::from(untracked(|| tex_file.get()).trim());
            match SyncTexIndex::open_for_pdf(pdf)
                .and_then(|idx| idx.forward_search(&src, line, None))
            {
                Ok(hit) => {
                    result_msg.set(t!("reading.synctex.forward_ok", page = hit.page + 1));
                    close();
                    go_to(hit.page);
                }
                Err(err) => result_msg.set(err.to_string()),
            }
        }
    };

    let run_inverse = {
        let p = pdf_path.clone();
        move || {
            let Some(ref pdf) = p else {
                return;
            };
            let pt = excellent_mupdf::PagePoint { x: 100.0, y: 120.0 };
            match SyncTexIndex::open_for_pdf(pdf)
                .and_then(|idx| idx.inverse_search(current_page, pt))
            {
                Ok(hit) => {
                    let preset = configured_editor_preset();
                    let cmd = preset
                        .format_command(&hit.file, hit.line)
                        .unwrap_or_else(|e| e.to_string());
                    result_msg.set(format!("{}:{} -> {cmd}", hit.file.display(), hit.line));
                }
                Err(err) => result_msg.set(err.to_string()),
            }
        }
    };

    let body = view()
        .w(px(520.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.synctex"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            field(input().value(move || tex_file.get()))
                .on_change(move |s: &str| tex_file.set(s.to_owned()))
                .placeholder(t!("reading.synctex.file_placeholder"))
                .label(t!("reading.synctex.file_placeholder"))
                .auto_focus(true),
        )
        .child(
            field(input().value(move || line_num.get()))
                .on_change(move |s: &str| line_num.set(s.to_owned()))
                .placeholder(t!("reading.synctex.line_placeholder"))
                .label(t!("reading.synctex.line_placeholder")),
        )
        .child(dynamic(move || {
            let msg = result_msg.get();
            if msg.is_empty() {
                view()
            } else {
                view().child(
                    text(msg)
                        .text_size(px(12.0))
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
                    "reading.synctex.inverse",
                    Tone::Plain,
                    run_inverse,
                ))
                .child(crate::shell::primary_button(
                    "reading.synctex.forward",
                    run_forward,
                )),
        );
    list_dialog(t!("command.synctex"), close, card(body))
}

/// Open URL with progressive download dialog (plan R.30).
pub(crate) fn open_url_dialog(
    state: ReadingExtraState,
    open_path: impl Fn(PathBuf) + Copy + 'static,
) -> impl IntoElement {
    let close = move || state.remote_open.set(false);
    let url = signal(String::new());
    let progress_text = signal(String::new());
    let busy = signal(false);

    let start_download = move || {
        let raw = untracked(|| url.get());
        let trimmed = raw.trim().to_owned();
        if trimmed.is_empty() || untracked(|| busy.get()) {
            return;
        }
        busy.set(true);
        progress_text.set(t!("reading.url.connecting"));
        let mut cache = match RemoteStreamCache::new(&trimmed, UreqTransport) {
            Ok(c) => c,
            Err(err) => {
                busy.set(false);
                progress_text.set(err.to_string());
                return;
            }
        };
        let filename = trimmed
            .rsplit('/')
            .next()
            .filter(|s| !s.is_empty())
            .unwrap_or("remote.pdf")
            .split('?')
            .next()
            .unwrap_or("remote.pdf")
            .to_owned();
        let dest = settings::folder()
            .unwrap_or_else(|| std::env::temp_dir().join("excellentpdf-downloads"))
            .join("downloads")
            .join(filename);
        match cache.save_copy(&dest, |prog| {
            progress_text.set(t!("reading.url.progress", bytes = prog.downloaded_bytes));
        }) {
            Ok(()) => {
                busy.set(false);
                close();
                open_path(dest);
            }
            Err(err) => {
                busy.set(false);
                progress_text.set(err.to_string());
            }
        }
    };

    let body = view()
        .w(px(500.0))
        .p_4()
        .gap_3()
        .child(
            text(move || t!("command.open_url"))
                .text_size(px(15.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(move || theme().popover_foreground),
        )
        .child(
            field(input().value(move || url.get()))
                .on_change(move |s: &str| url.set(s.to_owned()))
                .placeholder(t!("reading.url.placeholder"))
                .label(t!("reading.url.placeholder"))
                .auto_focus(true),
        )
        .child(dynamic(move || {
            let p = progress_text.get();
            if p.is_empty() {
                view()
            } else {
                view().child(
                    text(p)
                        .text_size(px(12.0))
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
                    "reading.url.open",
                    start_download,
                )),
        );
    list_dialog(t!("command.open_url"), close, card(body))
}

/// Extra format document viewer (DjVu, CHM, CBZ, CBR) (plan R.29).
pub(crate) fn extra_format_dialog(state: ReadingExtraState) -> impl IntoElement {
    let close = move || state.extra_open.set(false);
    let page_idx = signal(0_usize);
    let status = signal(String::new());
    let doc_text = signal(String::new());
    let page_total = signal(0_usize);
    let outline_titles = signal(Vec::<String>::new());

    let load_page =
        move |path: &Path, page: usize| {
            let Ok(bytes) = fs::read(path) else {
                status.set(t!("reading.extra.read_error"));
                return;
            };
            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_ascii_lowercase());
            let Some(fmt) = ExtraFormat::detect(&bytes, ext.as_deref()) else {
                status.set(t!("reading.extra.unsupported"));
                return;
            };
            let parsed: Result<Box<dyn ExtraDocument>, _> =
                match fmt {
                    ExtraFormat::DjVu => DjVuDocument::from_bytes(&bytes)
                        .map(|d| Box::new(d) as Box<dyn ExtraDocument>),
                    ExtraFormat::Chm => ChmDocument::from_bytes(&bytes)
                        .map(|d| Box::new(d) as Box<dyn ExtraDocument>),
                    ExtraFormat::Cbz => ComicArchive::from_cbz_bytes(&bytes)
                        .map(|d| Box::new(d) as Box<dyn ExtraDocument>),
                    ExtraFormat::Cbr => ComicArchive::from_cbr_bytes(&bytes)
                        .map(|d| Box::new(d) as Box<dyn ExtraDocument>),
                };
            match parsed {
                Ok(doc) => {
                    let count = doc.page_count();
                    page_total.set(count);
                    let clamped = page.min(count.saturating_sub(1));
                    page_idx.set(clamped);
                    outline_titles.set(doc.outline().into_iter().map(|i| i.title).collect());
                    match doc.page_text(clamped) {
                        Ok(Some(txt)) => doc_text.set(txt),
                        Ok(None) => doc_text.set(t!(
                            "reading.extra.image_page",
                            page = clamped + 1,
                            count = count
                        )),
                        Err(err) => doc_text.set(err.to_string()),
                    }
                    status.set(format!(
                        "{} — {}/{}",
                        fmt.extension().to_uppercase(),
                        clamped + 1,
                        count
                    ));
                }
                Err(err) => status.set(err.to_string()),
            }
        };

    if let Some(p) = state.extra_path.with(|p| p.clone()) {
        load_page(&p, untracked(|| page_idx.get()));
    }

    let choose_file = move || {
        let dialog = OpenDialog::files()
            .title(t!("command.extra_formats"))
            .filter(FileFilter::new(
                t!("command.extra_formats"),
                ["djvu", "djv", "chm", "cbz", "cbr"],
            ));
        let answer = pick_paths(dialog);
        spawn(async move {
            if let Ok(Some(paths)) = answer.await
                && let Some(p) = paths.into_iter().next()
            {
                state.extra_path.with(|slot| *slot = Some(p.clone()));
                load_page(&p, 0);
            }
        });
    };

    let prev = move || {
        if let Some(p) = state.extra_path.with(|p| p.clone()) {
            let next = untracked(|| page_idx.get()).saturating_sub(1);
            load_page(&p, next);
        }
    };

    let next = move || {
        if let Some(p) = state.extra_path.with(|p| p.clone()) {
            let cur = untracked(|| page_idx.get());
            let total = untracked(|| page_total.get());
            let target = (cur + 1).min(total.saturating_sub(1));
            load_page(&p, target);
        }
    };

    let body = view()
        .w(px(600.0))
        .p_4()
        .gap_3()
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || t!("command.extra_formats"))
                        .flex_1()
                        .text_size(px(15.0))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(move || theme().popover_foreground),
                )
                .child(action_button(
                    "reading.extra.choose",
                    Tone::Plain,
                    choose_file,
                )),
        )
        .child(
            text(move || status.get())
                .text_size(px(12.0))
                .text_color(move || theme().muted_foreground),
        )
        .child(
            scroll().max_h(px(280.0)).child(
                text(move || doc_text.get())
                    .text_size(px(13.0))
                    .text_color(move || theme().foreground),
            ),
        )
        .child(
            view()
                .flex_row()
                .justify_end()
                .gap_2()
                .child(action_button("reading.extra.prev", Tone::Plain, prev))
                .child(action_button("reading.extra.next", Tone::Plain, next))
                .child(crate::shell::primary_button("ink_tools.done", close)),
        );
    list_dialog(t!("command.extra_formats"), close, card(body))
}

/// Helper verifying whether `path` is one of R.29's extra containers (`djvu`, `chm`, `cbr`).
pub(crate) fn is_extra_non_mupdf_container(path: &Path) -> bool {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase());
    matches!(ext.as_deref(), Some("djvu" | "djv" | "chm" | "cbr"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bionic_formatting_highlights_word_prefixes() {
        let out = format_bionic_plain("Reading speed matters", 0.5);
        assert!(out.contains("*Read*ing"));
        assert!(out.contains("*Spe*ed") || out.contains("*spe*ed"));
    }

    #[test]
    fn extra_container_detection_identifies_djvu_chm_cbr() {
        assert!(is_extra_non_mupdf_container(Path::new("book.djvu")));
        assert!(is_extra_non_mupdf_container(Path::new("manual.chm")));
        assert!(is_extra_non_mupdf_container(Path::new("issue.cbr")));
        assert!(!is_extra_non_mupdf_container(Path::new("paper.pdf")));
    }
}
