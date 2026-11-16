//! Annotating from the pages (plan A): the canvas's `markup` event (A.1) made into one undoable edit of the shown
//! document, and its `textEdit` event (A.2) into a proposed replacement or insertion, once the reader types the text
//! in a small dialog. Every tool's colour and opacity (A.3) are the settings' and the colour dialog's: `colors`.
//! The models are `excellent-features`' `annotate::markup`, `annotate::replace` and `annotate::colors`.

pub(crate) mod colors;

use excellent_core::annot::{Markup, Style, page_annotations};
use excellent_core::edit::Operation;
use excellent_features::annotate::colors::Tool as ColorTool;
use excellent_features::annotate::markup::{self, Marking};
use excellent_features::annotate::replace::{self, Mode, Proposal};
use nsh_base::{input, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::bookmarks::{caption, field};
use crate::editing::Editing;
use crate::list_dialog::{card, list_dialog};
use crate::settings::Live;
use crate::tabs::{Tone, action_button};

/// A pane's annotating: its document's edits, the replacement or insertion waiting for its text, and the tool whose
/// colour is being chosen.
#[derive(Clone, Copy)]
pub(crate) struct AnnotateState {
    editing: Editing,
    live: Live,
    /// The proposal the dialog asks the text of (A.2).
    asking: Signal<Option<replace::Request>>,
    /// The tool the colour dialog opened on (A.3).
    picking: Signal<Option<ColorTool>>,
}

impl AnnotateState {
    pub(crate) fn new(editing: Editing, live: Live) -> Self {
        Self {
            editing,
            live,
            asking: signal(None),
            picking: signal(None),
        }
    }

    /// Whether the colour dialog is open.
    pub(crate) fn is_picking(self) -> bool {
        self.picking.with(Option::is_some)
    }

    /// Opens the colour dialog on `tool` (A.3).
    pub(crate) fn pick(self, tool: ColorTool) {
        self.picking.set(Some(tool));
    }

    /// The canvas's `annotationColors` prop.
    pub(crate) fn colors_prop(self) -> JsonValue {
        colors::prop(self.live)
    }

    /// Whether the dialog asking for a proposal's text is open.
    pub(crate) fn is_asking(self) -> bool {
        self.asking.with(Option::is_some)
    }

    /// The canvas asked for a markup over the selected text (`markup`), made as one step of undo merged with the
    /// markups it touches; or to propose a replacement or insertion (`textEdit`), whose text the dialog asks for; or
    /// gave a tool a swatch's colour (`toolColor`), or asked for a custom one (`pickColor`).
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        match &*event.name {
            "markup" => self.mark(&event.data),
            "toolColor" => colors::on_tool_color(self.live, &event.data),
            "pickColor" => {
                let tool = event
                    .data
                    .get("tool")
                    .and_then(JsonValue::as_str)
                    .and_then(ColorTool::parse)
                    .unwrap_or(ColorTool::Highlight);
                self.pick(tool);
            }
            "textEdit" => match replace::Request::parse(&event.data) {
                Some(request) => self.asking.set(Some(request)),
                None => log::warn!("textEdit: a request that doesn't parse: {}", event.data),
            },
            _ => {}
        }
    }

    fn mark(self, data: &JsonValue) {
        let Some(request) = markup::Request::parse(data) else {
            log::warn!("markup: a request that doesn't parse: {data}");
            return;
        };
        let Some(document) = self.editing.document() else {
            return;
        };
        let shared = match document.mupdf() {
            Ok(shared) => shared,
            Err(err) => {
                log::warn!("markup: cannot read the document: {err}");
                return;
            }
        };
        let author = author();
        let marking = Marking {
            kind: request.kind,
            style: style(request.color, request.opacity),
            author: &author,
        };
        let mut changes = Vec::new();
        {
            // Let go of the document before the edit, which takes it again.
            let mupdf = shared.lock().expect("document lock poisoned");
            for page in request.pages {
                let existing = page_annotations(&mupdf, page.page).unwrap_or_else(|err| {
                    log::warn!(
                        "markup: cannot read page {}'s annotations: {err}",
                        page.page
                    );
                    Vec::new()
                });
                changes.extend(markup::mark(
                    marking, page.page, page.quads, page.quote, &existing,
                ));
            }
        }
        self.editing
            .apply(&Operation::new(name(request.kind), changes));
    }

    /// Proposes `text` for the request being asked about, as one step of undo; blank text proposes nothing.
    fn propose(self, request: replace::Request, text: &str) {
        let author = author();
        let proposal = Proposal {
            text,
            style: style(request.color, request.opacity),
            author: &author,
        };
        let page = request.page;
        let (changes, name) = match request.mode {
            Mode::Insert => (
                replace::insert(proposal, page.page, request.caret),
                t!("markup.insert"),
            ),
            Mode::Replace => (
                replace::replace(proposal, page.page, page.quads, request.caret, page.quote),
                t!("markup.replace"),
            ),
        };
        if !changes.is_empty() {
            self.editing.apply(&Operation::new(name, changes));
        }
    }
}

/// The colour dialog (A.3), on the tool it was opened for.
pub(crate) fn color_dialog(state: AnnotateState) -> impl IntoElement {
    colors::color_dialog(state.live, state.picking)
}

/// The dialog that asks for the text of a proposed replacement or insertion (A.2). Replacing starts from the text
/// selected, chosen whole so typing replaces it; Enter proposes, Escape or a press outside cancels.
pub(crate) fn text_edit_dialog(state: AnnotateState) -> impl IntoElement {
    let close = move || state.asking.set(None);
    let content = match untracked(|| state.asking.get()) {
        Some(request) => form(state, request, close),
        None => view(),
    };
    list_dialog(t!("text_edit.dialog"), close, card(content))
}

fn form(
    state: AnnotateState,
    request: replace::Request,
    close: impl Fn() + Copy + 'static,
) -> View {
    let replacing = request.mode == Mode::Replace;
    let typed = signal(request.selected.clone());
    let selected = u32::try_from(request.selected.encode_utf16().count()).unwrap_or(u32::MAX);
    let struck = request.selected.clone();
    let propose = move || {
        let text = untracked(|| typed.get());
        close();
        state.propose(request.clone(), &text);
    };
    let submit = propose.clone();
    let text_field = field(input().value(move || typed.get()))
        .on_change(move |text: &str| typed.set(text.to_owned()))
        .on_submit(move |_: &str| submit())
        .placeholder(t!("text_edit.placeholder"))
        .label(t!(if replacing {
            "text_edit.replace_with"
        } else {
            "text_edit.insert"
        }))
        .auto_focus(true)
        .selected_range((0, selected));
    let mut body = view().w(px(380.0)).p_4().gap_3().child(
        text(move || {
            t!(if replacing {
                "markup.replace"
            } else {
                "markup.insert"
            })
        })
        .text_size(px(14.0))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(move || theme().popover_foreground),
    );
    if replacing {
        body = body.child(caption("text_edit.struck")).child(
            text(move || struck.clone())
                .text_size(px(13.0))
                .line_through()
                .text_color(move || theme().muted_foreground),
        );
    }
    body.child(text_field)
        .child(view().h(px(1.0)).bg(move || theme().border))
        .child(
            view()
                .flex_row()
                .gap_2()
                .justify_end()
                .child(action_button("text_edit.cancel", Tone::Plain, close))
                .child(crate::shell::primary_button(
                    if replacing {
                        "markup.replace"
                    } else {
                        "markup.insert"
                    },
                    propose,
                )),
        )
}

/// The edit's name, as undo shows it.
fn name(kind: Markup) -> String {
    match kind {
        Markup::Highlight => t!("markup.highlight"),
        Markup::Underline => t!("markup.underline"),
        Markup::Squiggly => t!("markup.squiggly"),
        Markup::StrikeOut => t!("markup.strikeout"),
    }
}

/// An annotation's look in `color` at `opacity`.
fn style(color: [f32; 3], opacity: f32) -> Style {
    Style {
        color: Some(color),
        opacity,
        ..Style::default()
    }
}

/// Who annotations are by: the system's user name.
// shortcut: the account's name until a setting for the author's name (the plan has none yet); ask the maintainer.
fn author() -> String {
    std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default()
}
