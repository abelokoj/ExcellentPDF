//! Annotating from the pages (plan A): the canvas's `markup` event (A.1) made into one undoable edit of the shown
//! document, and its `textEdit` event (A.2) into a proposed replacement or insertion, once the reader types the text
//! in a small dialog. Every tool's colour and opacity (A.3) are the settings' and the colour dialog's: `colors`. A
//! click on a markup (`pagePress`), or a markup made with the note tool, opens its note in the inspector (A.4):
//! `note`. The sticky-note, text-box and typewriter commands wait for a click on the page and put a sticky note (A.5)
//! or a text box (A.6) there, its note or text open in the inspector; the line, arrow, rectangle and ellipse commands
//! turn on the canvas's drawing tool (its `drawing` prop), and the shape it draws (`pageDraw`) becomes one (A.8, A.9).
//! The models are `ecclesia-features`'
//! `annotate::markup`, `annotate::replace`, `annotate::colors`, `annotate::notes`, `annotate::sticky` and
//! `annotate::freetext` and `annotate::shapes`.

pub(crate) mod colors;
mod exchange;
pub(crate) mod favorites;
mod flatten;
pub(crate) mod guides;
pub(crate) mod ink_tools;
mod inspector;
mod lasso;
mod layers;
mod list;
pub(crate) mod margin;
mod note;
pub(crate) mod phase5;
mod select;
pub(crate) mod selection_popup;
mod stamps;
pub(crate) mod tag_index;
mod thread;
mod transfer;

pub(crate) use ecclesia_features::annotate::freetext::Kind;
pub(crate) use list::annotation_list;
pub(crate) use note::note_panel;

use ecclesia_core::annot::{Annotation, Markup, Style, TextStyle, page_annotations};
use ecclesia_core::edit::{Change, Operation};
use ecclesia_features::annotate::colors::Tool as ColorTool;
use ecclesia_features::annotate::edit;
use ecclesia_features::annotate::erase::Erased;
use ecclesia_features::annotate::freetext;
use ecclesia_features::annotate::ink::Inked;
use ecclesia_features::annotate::inspector as rules;
use ecclesia_features::annotate::margin::ToScreen;
use ecclesia_features::annotate::markup::{self, Marking};
use ecclesia_features::annotate::notes::{self, Target};
use ecclesia_features::annotate::polygons;
use ecclesia_features::annotate::replace::{self, Mode, Proposal};
use ecclesia_features::annotate::shapes::{self, Drawn};
use ecclesia_features::annotate::sticky::{self, Icon};
use ecclesia_features::annotate::tape;
use ecclesia_features::ink::eyedropper::{SampleArea, SampledColor};
use ecclesia_ink::erase::Eraser;

use nsh_base::{input, theme};
use nsh_core::JsonValue;
use nsh_core::prelude::*;

use crate::bookmarks::{caption, field};
use crate::commands::CommandId;
use crate::editing::Editing;
use crate::list_dialog::{card, list_dialog};
use crate::settings::Live;
use crate::tabs::{Tone, action_button};

/// The settings' keys: a regular polygon's sides and a star's points (A.10).
pub(crate) const POLYGON_SIDES: &str = "polygon_sides";
pub(crate) const STAR_POINTS: &str = "star_points";

/// The count the setting `key` holds, within [`polygons::SIDES`]; [`polygons::DEFAULT_SIDES`] when unset.
fn sides(live: Live, key: &str) -> u8 {
    let (low, high) = (*polygons::SIDES.start(), *polygons::SIDES.end());
    live.read(|s| s.number(key))
        .filter(|n| n.is_finite())
        .map_or(polygons::DEFAULT_SIDES, |n| {
            n.round().clamp(f64::from(low), f64::from(high)) as u8
        })
}

/// What the next click on the pages puts there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Placing {
    /// A sticky note (A.5).
    Sticky,
    /// A text box, typewriter text or a callout (A.6, A.7).
    Text(Kind),
    /// A line, an arrow (A.8), a rectangle or an ellipse (A.9), a regular polygon or a star dragged out on the page,
    /// or a polygon, a polyline or a cloud clicked corner by corner (A.10).
    Shape(shapes::Kind),
    /// The stamp chosen in the inspector (A.20).
    Stamp,
    /// A strip of tape dragged out over what it hides (H.4).
    Tape,
}

/// A pane's annotating: its document's edits, the replacement or insertion waiting for its text, the tool whose
/// colour is being chosen, and the note open in the inspector.
#[derive(Clone, Copy)]
pub(crate) struct AnnotateState {
    editing: Editing,
    live: Live,
    /// The proposal the dialog asks the text of (A.2).
    asking: Signal<Option<replace::Request>>,
    /// The tool the colour dialog opened on (A.3).
    picking: Signal<Option<ColorTool>>,
    /// The note open in the inspector (A.4).
    noting: Signal<Option<note::Noting>>,
    /// Its text as typed, saved when the reader is done.
    draft: Signal<String>,
    /// Counts the notes opened, so the window shows the inspector for each one.
    reveals: Signal<u64>,
    /// What the next click on the page places (A.5, A.6), if anything.
    placing: Signal<Option<Placing>>,
    /// The pages in view, whose notes the margin column shows (A.11), and how each one's points map to the screen.
    shown_pages: Signal<Vec<usize>>,
    page_maps: Signal<Vec<(usize, ToScreen)>>,
    /// The annotations selected on the pages (A.13), all on one page.
    selected: Signal<Vec<Target>>,
    /// The annotation list (A.14).
    listing: list::Listing,
    /// The stamp the Stamp command places, and the reader's library of stamps (A.20).
    stamp: Signal<stamps::Choice>,
    stamps: Signal<Vec<ecclesia_features::annotate::stamps::Item>>,
    /// The saved pen open in the edit dialog (H.7), by its place on the bar.
    editing_pen: Signal<Option<usize>>,
    /// The eraser the pen uses instead of drawing (H.5); `None`: the pen draws.
    eraser: Signal<Option<Eraser>>,
    /// The lasso is on (H.6): a drag on the pages draws a loop.
    lasso: Signal<bool>,
    /// The drawing guide on the page (H.11).
    guide: Signal<Option<guides::GuideKind>>,
    /// What the last stroke along the guide measured, in the reader's words.
    reading: Signal<Option<String>>,
    /// The zoom writing pad is on (H.12).
    zoom_pad: Signal<bool>,
    /// Active drawing layer for new ink and annotations (H.14).
    active_layer: Signal<String>,
    /// Last focused page index for page-level ink tools.
    current_page: Signal<usize>,
    /// Bulk ink restyle dialog open state (H.15).
    restyle_open: Signal<bool>,
    /// Fill & gradient dialog open state (H.16).
    fill_open: Signal<bool>,
    /// Eyedropper tool active state, sampling aperture, and latest sample + loupe (H.17).
    eyedropper_active: Signal<bool>,
    eyedropper_area: Signal<SampleArea>,
    eyedropper_sample: Signal<Option<(SampledColor, Vec<Option<SampledColor>>)>>,
    /// Saved color palettes dialog open state (H.18).
    palettes_open: Signal<bool>,
    /// Ink stroke replay dialog open state (H.22).
    replay_open: Signal<bool>,
    /// Annotation report export dialog open state (A.26).
    report_open: Signal<bool>,
    /// Annotation history timeline dialog open state (A.27).
    history_open: Signal<bool>,
    /// Pattern brush picker dialog open state (H.20).
    pattern_open: Signal<bool>,
    /// Symmetry axis dialog open state (H.21).
    symmetry_open: Signal<bool>,
    /// Searchable handwriting dialog open state (H.23).
    hw_search_open: Signal<bool>,
    /// Convert handwriting to text dialog open state (H.24).
    hw_convert_open: Signal<bool>,
    /// OCR to searchable PDF dialog open state (O.4).
    ocr_open: Signal<bool>,
    /// Floating text-selection pill anchor geometry reported by the canvas.
    selection_popup: Signal<Option<selection_popup::SelectionPopupAnchor>>,
    /// Active markup tool used when clicking one of the circular color swatches in the selection popup.
    active_markup_tool: Signal<selection_popup::ActiveMarkupTool>,
}

impl AnnotateState {
    pub(crate) fn new(editing: Editing, live: Live) -> Self {
        Self {
            editing,
            live,
            asking: signal(None),
            picking: signal(None),
            noting: signal(None),
            draft: signal(String::new()),
            reveals: signal(0),
            placing: signal(None),
            shown_pages: signal(Vec::new()),
            page_maps: signal(Vec::new()),
            selected: signal(Vec::new()),
            listing: list::Listing::new(),
            stamp: signal(stamps::Choice::default()),
            stamps: signal(Vec::new()),
            editing_pen: signal(None),
            eraser: signal(None),
            lasso: signal(false),
            guide: signal(None),
            reading: signal(None),
            zoom_pad: signal(false),
            active_layer: signal(String::from("Ink")),
            current_page: signal(0),
            restyle_open: signal(false),
            fill_open: signal(false),
            eyedropper_active: signal(false),
            eyedropper_area: signal(SampleArea::SinglePixel),
            eyedropper_sample: signal(None),
            palettes_open: signal(false),
            replay_open: signal(false),
            report_open: signal(false),
            history_open: signal(false),
            pattern_open: signal(false),
            symmetry_open: signal(false),
            hw_search_open: signal(false),
            hw_convert_open: signal(false),
            ocr_open: signal(false),
            selection_popup: signal(None),
            active_markup_tool: signal(selection_popup::ActiveMarkupTool::default()),
        }
    }

    /// Current floating text-selection popup anchor (tracked).
    pub(crate) fn selection_popup_anchor(self) -> Option<selection_popup::SelectionPopupAnchor> {
        self.selection_popup.get()
    }

    /// Hides the floating text-selection popup immediately.
    pub(crate) fn dismiss_selection_popup(self) {
        self.selection_popup.set(None);
    }

    /// Active markup mode used when clicking a swatch dot on the floating selection popup (tracked).
    pub(crate) fn active_markup_tool(self) -> selection_popup::ActiveMarkupTool {
        self.active_markup_tool.get()
    }

    /// Updates the active markup mode highlighted on the floating selection popup.
    pub(crate) fn set_active_markup_tool(self, tool: selection_popup::ActiveMarkupTool) {
        self.active_markup_tool.set(tool);
    }

    /// Live settings handle owned by this pane's annotation controller.
    pub(crate) fn live_settings(self) -> Live {
        self.live
    }

    /// Whether any ink or Phase 5 annotation/OCR dialog is currently open.
    pub(crate) fn any_ink_dialog_open(self) -> bool {
        self.restyle_open.get()
            || self.fill_open.get()
            || self.palettes_open.get()
            || self.replay_open.get()
            || self.report_open.get()
            || self.history_open.get()
            || self.pattern_open.get()
            || self.symmetry_open.get()
            || self.hw_search_open.get()
            || self.hw_convert_open.get()
            || self.ocr_open.get()
    }

    /// Whether the Report Export dialog is open (A.26).
    pub(crate) fn is_reporting(self) -> bool {
        self.report_open.get()
    }
    /// Whether the Annotation History dialog is open (A.27).
    pub(crate) fn is_history(self) -> bool {
        self.history_open.get()
    }
    /// Whether the Pattern Brush dialog is open (H.20).
    pub(crate) fn is_pattern(self) -> bool {
        self.pattern_open.get()
    }
    /// Whether the Symmetry dialog is open (H.21).
    pub(crate) fn is_symmetry(self) -> bool {
        self.symmetry_open.get()
    }
    /// Whether the Handwriting Search dialog is open (H.23).
    pub(crate) fn is_hw_searching(self) -> bool {
        self.hw_search_open.get()
    }
    /// Whether the Convert Handwriting dialog is open (H.24).
    pub(crate) fn is_hw_converting(self) -> bool {
        self.hw_convert_open.get()
    }
    /// Whether the OCR dialog is open (O.4).
    pub(crate) fn is_ocr(self) -> bool {
        self.ocr_open.get()
    }

    /// Whether the Restyle dialog is open (H.15).
    pub(crate) fn is_restyling(self) -> bool {
        self.restyle_open.get()
    }

    /// Whether the Fill dialog is open (H.16).
    pub(crate) fn is_filling(self) -> bool {
        self.fill_open.get()
    }

    /// Whether the Color Palettes dialog is open (H.18).
    pub(crate) fn is_palettes(self) -> bool {
        self.palettes_open.get()
    }

    /// Whether the Ink Replay dialog is open (H.22).
    pub(crate) fn is_replaying(self) -> bool {
        self.replay_open.get()
    }

    /// The sticky-note, text-box and typewriter commands (A.5, A.6): the next click on the pages places `what`; run
    /// again, it is called off.
    pub(crate) fn place(self, what: Placing) {
        if !self.editing.can_edit() {
            return;
        }
        let placing = (untracked(|| self.placing.get()) != Some(what)).then_some(what);
        if placing.is_some() {
            self.close_note();
            self.reveals.update(|count| *count += 1);
        }
        if placing == Some(Placing::Stamp) {
            self.load_stamps();
        }
        self.placing.set(placing);
    }

    /// Puts `what` where the click `data` (`pagePress`) landed, as one step of undo, and opens its note or text.
    fn put(self, what: Placing, data: &JsonValue) {
        self.placing.set(None);
        let Some((page, point)) = notes::parse_press(data) else {
            return;
        };
        let Some(document) = self.editing.document() else {
            return;
        };
        let bounds = {
            let Ok(shared) = document.mupdf() else {
                return;
            };
            let mupdf = shared.lock().expect("document lock poisoned");
            match mupdf.load_page(page).and_then(|loaded| loaded.bounds()) {
                Ok(bounds) => bounds,
                Err(err) => {
                    log::warn!("annotate: cannot read page {page}'s size: {err}");
                    return;
                }
            }
        };
        let (made, name, art) = match what {
            Placing::Shape(_) | Placing::Tape => return,
            Placing::Sticky => (
                sticky::place(
                    page,
                    point,
                    bounds,
                    Icon::default(),
                    Style::default(),
                    &author(),
                ),
                t!("sticky.name"),
                None,
            ),
            Placing::Text(kind) => (
                freetext::place(page, point, bounds, kind, TextStyle::default(), &author()),
                text_name(kind),
                None,
            ),
            Placing::Stamp => {
                let choice = untracked(|| self.stamp.get());
                match stamps::made(&choice, page, point, bounds, &author()) {
                    Ok((made, art)) => (made, t!("annotations.kind.stamp"), art),
                    Err(err) => {
                        crate::editing::show_message(t!("stamps.failed"), err.to_string());
                        return;
                    }
                }
            }
        };
        let target = Target {
            page,
            id: made.id.clone(),
        };
        // A stamp drawn from art is made and drawn in one operation, so one step of undo (A.20).
        let changes = self.place_changes(ecclesia_features::annotate::stamps::changes(made, art));
        let operation = Operation::new(name, changes);
        if self.editing.apply(&operation) {
            self.open_note(&target);
            // A text box left empty when the reader is done is taken away again, as Acrobat does.
            self.noting
                .update(|noting| noting.iter_mut().for_each(|noting| noting.fresh = true));
        }
    }

    /// Deletes the open text box (A.6) or line (A.8), as one step of undo named `name`, and closes it.
    fn delete_open(self, name: String) {
        if let Some(noting) = untracked(|| self.noting.get()) {
            if self
                .annotation(&noting.target)
                .is_some_and(|annotation| annotation.locked)
            {
                return;
            }
            let change = Change::DeleteAnnotation {
                page: noting.target.page,
                id: noting.target.id.clone(),
            };
            self.editing.apply(&Operation::new(name, vec![change]));
        }
        self.noting.set(None);
    }

    /// Turns the zoom writing pad on or off (H.12).
    pub(crate) fn toggle_zoom_pad(self) {
        self.zoom_pad.update(|on| *on = !*on);
    }

    /// The canvas's `zoomPad` prop.
    pub(crate) fn zoom_pad_prop(self) -> JsonValue {
        JsonValue::from(self.zoom_pad.get())
    }

    /// The canvas's `drawing` prop: the drawing tool on (A.8), or `null`.
    pub(crate) fn drawing_prop(self) -> JsonValue {
        match self.placing.get() {
            Some(Placing::Shape(kind)) => JsonValue::from(kind.to_string()),
            // Tape is dragged out as a rectangle is (H.4).
            Some(Placing::Tape) => JsonValue::from(shapes::Kind::Rectangle.to_string()),
            _ => JsonValue::Null,
        }
    }

    /// Erases with `eraser` from now on (H.5): the pen erases until a pen is chosen again.
    pub(crate) fn choose_eraser(self, eraser: Eraser) {
        self.eraser.set(Some(eraser));
        self.lasso.set(false);
    }

    /// The canvas's `eraser` prop: the chosen eraser's name, or `null` while the pen draws (H.5).
    pub(crate) fn eraser_prop(self) -> JsonValue {
        self.eraser
            .get()
            .map_or(JsonValue::Null, |eraser| JsonValue::from(eraser.name()))
    }

    /// Whether `id` is the tool now chosen, so its toolbar button shows pressed: the pen (H.2) while no eraser is
    /// chosen, or the eraser (H.5).
    pub(crate) fn is_on(self, id: CommandId) -> bool {
        match id {
            CommandId::Pen(kind) => {
                self.eraser.get().is_none() && !self.lasso.get() && kind == self.kind_in_hand()
            }
            CommandId::Eraser(eraser) => self.eraser.get() == Some(eraser),
            CommandId::Lasso => self.lasso.get(),
            CommandId::Tape => self.placing.get() == Some(Placing::Tape),
            CommandId::ZoomPad => self.zoom_pad.get(),
            CommandId::Ruler => self.guide_on(guides::GuideKind::Ruler),
            CommandId::Protractor => self.guide_on(guides::GuideKind::Protractor),
            CommandId::Compass => self.guide_on(guides::GuideKind::Compass),
            CommandId::Eyedropper => self.eyedropper_active.get(),
            _ => false,
        }
    }

    /// The regular-polygon tool, with as many sides as the settings say (A.10).
    pub(crate) fn regular_polygon(self) -> shapes::Kind {
        shapes::Kind::Regular(sides(self.live, POLYGON_SIDES))
    }

    /// The star tool, with as many points as the settings say (A.10).
    pub(crate) fn star(self) -> shapes::Kind {
        shapes::Kind::Star(sides(self.live, STAR_POINTS))
    }

    /// The canvas drew a shape (`pageDraw`): it becomes an annotation, one step of undo, and the tool goes off, its
    /// inspector open on the new shape.
    fn drawn(self, data: &JsonValue) {
        let taping = untracked(|| self.placing.get()) == Some(Placing::Tape);
        self.placing.set(None);
        let Some(drawn) = Drawn::parse(data) else {
            log::warn!("pageDraw: a shape that doesn't parse: {data}");
            return;
        };
        let Some(made) = drawn.annotation(&author()) else {
            return;
        };
        // Tape (H.4): the rectangle dragged out becomes a strip over what it hides, with no note to open.
        if taping {
            if let ecclesia_core::annot::Shape::Boxed(_, rect) = made.shape {
                let changes = self.place_changes(vec![Change::CreateAnnotation(tape::tape(
                    made.page,
                    rect,
                    &author(),
                ))]);
                self.editing
                    .apply(&Operation::new(t!("tape.edit"), changes));
            }
            return;
        }
        let target = Target {
            page: made.page,
            id: made.id.clone(),
        };
        let name = shape_name(&made);
        let changes = self.place_changes(vec![Change::CreateAnnotation(made)]);
        if self.editing.apply(&Operation::new(name, changes)) {
            self.open_note(&target);
        }
    }

    /// The canvas finished a pen stroke (`ink`, H.1): it becomes an ink annotation, one step of undo.
    fn inked(self, data: &JsonValue) {
        let Some(inked) = Inked::parse(data) else {
            log::warn!("ink: a stroke that doesn't parse: {data}");
            return;
        };
        self.current_page.set(inked.page);
        let made = inked.annotation(&author());
        let changes = self.place_changes(vec![Change::CreateAnnotation(made)]);
        self.editing.apply(&Operation::new(t!("ink.edit"), changes));
    }

    /// The eraser went over a page (`erase`, H.5): the ink it crossed is deleted or cut back, all one step of undo.
    fn erased(self, data: &JsonValue) {
        let Some(erased) = Erased::parse(data) else {
            log::warn!("erase: a pass that doesn't parse: {data}");
            return;
        };
        let Some(document) = self.editing.document() else {
            return;
        };
        let stack = self.layer_stack();
        let changes = {
            let Ok(shared) = document.mupdf() else {
                return;
            };
            let mupdf = shared.lock().expect("document lock poisoned");
            // Hidden or locked layers are never erased (A.22, H.14).
            stack.erase_changes(&erased, &layers::shown_on_page(&mupdf, erased.page))
        };
        if !changes.is_empty() {
            self.editing
                .apply(&Operation::new(t!("ink.erase"), changes));
        }
    }

    /// Makes `change` of the open note's annotation, as it is now, one step of undo named `name`.
    fn change_note(self, name: String, change: impl FnOnce(&Annotation) -> Option<Change>) {
        let Some(noting) = untracked(|| self.noting.get()) else {
            return;
        };
        // A locked annotation takes only its note and its lock (A.12).
        if let Some(change) = self
            .annotation(&noting.target)
            .and_then(|a| change(&a).filter(|change| rules::allows(&a, change)))
        {
            self.editing.apply(&Operation::new(name, vec![change]));
        }
    }

    /// How many notes have been opened: the window shows the inspector when it changes.
    pub(crate) fn reveals(self) -> u64 {
        self.reveals.get()
    }

    /// Opens the note of the markup `target` (A.4), keeping the one open first.
    fn open_note(self, target: &Target) {
        self.save_note();
        let Some(annotation) = self.annotation(target) else {
            return;
        };
        self.draft.set(annotation.contents.clone());
        self.noting.set(Some(note::Noting::of(&annotation)));
        self.reveals.update(|count| *count += 1);
    }

    /// Saves the open note's text into its markup, or a text box's into the box, as one step of undo, when it
    /// changed. A text box just placed and left empty is deleted.
    fn save_note(self) {
        let Some(noting) = untracked(|| self.noting.get()) else {
            return;
        };
        let Some(annotation) = self.annotation(&noting.target) else {
            // Undone, or the document changed: there is nothing to write it to.
            return;
        };
        let draft = untracked(|| self.draft.get());
        let (name, change) = if !annotation.is_text_box() {
            (t!("note.edit"), notes::write(&annotation, &draft))
        } else if noting.fresh && notes::normalize(&draft).is_empty() {
            let gone = Change::DeleteAnnotation {
                page: annotation.page,
                id: annotation.id.clone(),
            };
            (t!("text.delete"), Some(gone))
        } else {
            (t!("text.edit"), freetext::set_text(&annotation, &draft))
        };
        if let Some(change) = change {
            self.editing.apply(&Operation::new(name, vec![change]));
        }
    }

    /// Saves the open note and closes it.
    fn close_note(self) {
        self.save_note();
        self.noting.set(None);
    }

    /// The annotation `target` names, as the shown document has it now.
    fn annotation(self, target: &Target) -> Option<Annotation> {
        let document = self.editing.document()?;
        let shared = document
            .mupdf()
            .map_err(|err| log::warn!("note: cannot read the document: {err}"))
            .ok()?;
        let mupdf = shared.lock().expect("document lock poisoned");
        page_annotations(&mupdf, target.page)
            .map_err(|err| {
                log::warn!(
                    "note: cannot read page {}'s annotations: {err}",
                    target.page
                )
            })
            .ok()?
            .into_iter()
            .find(|annotation| annotation.id == target.id)
    }

    /// A plain click on the page (`pagePress`): a sticky note or text box goes there when one is being placed; else
    /// the annotation under it is selected (A.13), and its note, or the text box's text, opens. With Ctrl it joins
    /// the selection, or leaves it, and no note opens.
    fn press(self, data: &JsonValue) {
        if untracked(|| self.eyedropper_active.get()) {
            self.sample_eyedropper(data);
            return;
        }
        if let Some(what) = untracked(|| self.placing.get())
            && !matches!(what, Placing::Shape(_) | Placing::Tape)
        {
            self.put(what, data);
            return;
        }
        let Some((page, point)) = notes::parse_press(data) else {
            return;
        };
        self.current_page.set(page);
        let Some(document) = self.editing.document() else {
            return;
        };
        let add = data
            .get("add")
            .and_then(JsonValue::as_bool)
            .unwrap_or(false);
        let (target, tapped) = {
            let Ok(shared) = document.mupdf() else {
                return;
            };
            let mupdf = shared.lock().expect("document lock poisoned");
            // What a hidden layer holds isn't on the page to press (A.22).
            let annotations = layers::shown_on_page(&mupdf, page);
            let hit = edit::hit(&annotations, point);
            (
                hit.map(|annotation| Target {
                    page,
                    id: annotation.id.clone(),
                }),
                // A plain click on tape reveals it or hides it again (H.4); Ctrl still selects it.
                hit.filter(|_| !add).and_then(tape::tapped),
            )
        };
        if let Some(change) = tapped {
            self.editing
                .apply(&Operation::new(t!("tape.tap"), vec![change]));
            return;
        }
        if self.select_press(target.clone(), add) {
            return;
        }
        if let Some(target) = target {
            self.open_note(&target);
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
    /// gave a tool a swatch's colour (`toolColor`), or asked for a custom one (`pickColor`); or was clicked
    /// (`pagePress`), which opens the note of the markup there.
    pub(crate) fn on_canvas_event(self, event: &NativeEvent) {
        match &*event.name {
            "markup" => self.mark(&event.data),
            "pagePress" => self.press(&event.data),
            "pageDraw" => self.drawn(&event.data),
            "ink" => self.inked(&event.data),
            "erase" => self.erased(&event.data),
            "guideReading" => self.guide_reading(&event.data),
            "annotDrag" => self.selection_dragged(&event.data),
            "annotMarquee" => self.marquee(&event.data),
            "annotLasso" => self.looped(&event.data),
            "margin" => self.margin_moved(&event.data),
            "selectionPopup" => self
                .selection_popup
                .set(selection_popup::SelectionPopupAnchor::parse(&event.data)),
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
        let noted = data.get("note").and_then(JsonValue::as_bool) == Some(true);
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
                changes.extend(self.place_changes(markup::mark(
                    marking, page.page, page.quads, page.quote, &existing,
                )));
            }
        }
        let target = notes::marked(&changes);
        let made = self
            .editing
            .apply(&Operation::new(name(request.kind), changes));
        if let (true, true, Some(target)) = (made, noted, target) {
            self.open_note(&target);
        }
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

/// A shape's name, for its heading and its undo step: a line with an arrowhead is an arrow.
fn shape_name(annotation: &Annotation) -> String {
    shape_kind_name(shapes::kind_of(annotation).unwrap_or(shapes::Kind::Line))
}

/// The name of a kind of shape.
fn shape_kind_name(kind: shapes::Kind) -> String {
    t!(match kind {
        shapes::Kind::Line => "line.name",
        shapes::Kind::Arrow => "arrow.name",
        shapes::Kind::Rectangle => "rectangle.name",
        shapes::Kind::Ellipse => "ellipse.name",
        shapes::Kind::Regular(_) | shapes::Kind::Star(_) | shapes::Kind::Polygon => "polygon.name",
        shapes::Kind::PolyLine => "polyline.name",
        shapes::Kind::Cloud => "cloud.name",
    })
}

/// A text box's name, or typewriter text's or a callout's, for its heading and its undo step.
fn text_name(kind: Kind) -> String {
    t!(match kind {
        Kind::Box => "text.box_name",
        Kind::Typewriter => "text.typewriter_name",
        Kind::Callout => "callout.name",
    })
}

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

impl AnnotateState {
    /// Applies undoable changes through the owning document's `Editing` state, placing them on the active layer.
    pub(crate) fn editing_apply(self, name: String, changes: Vec<ecclesia_core::edit::Change>) {
        let placed = self.place_changes(changes);
        if !placed.is_empty() {
            self.editing.apply(&Operation::new(name, placed));
        }
    }

    /// Exposes current-page annotations for Study tools cleanly.
    pub(crate) fn page_annotations_for_study(
        self,
        page: usize,
    ) -> Vec<ecclesia_core::annot::Annotation> {
        let Some(doc) = self.editing.document() else {
            return Vec::new();
        };
        let Ok(shared) = doc.mupdf() else {
            return Vec::new();
        };
        let mupdf = shared.lock().expect("document lock poisoned");
        ecclesia_core::annot::page_annotations(&mupdf, page).unwrap_or_default()
    }
}
