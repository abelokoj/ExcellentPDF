//! UI Automation for the page canvas (plan U.2): what Narrator and NVDA read. The canvas window answers
//! `WM_GETOBJECT` with [`Root`], a Document whose children are the pages, each holding its links (which can be
//! followed: Invoke) and its notes (annotations with text). The root has the Text pattern over the text the canvas
//! has read (K.13), in reading order, walked by `excellent_text::DocumentText`.
//!
//! Providers hold only the canvas window's handle (and a page and an index): each call finds the canvas again
//! ([`canvas_of`]) and answers `UIA_E_ELEMENTNOTAVAILABLE` when it, its page, link or note is gone. They use COM
//! threading, so calls from screen readers arrive on the window's thread, the only one that knows the canvases.
//! Events go through the window's queue ([`WM_UIA_EVENT`]) so they are raised while the canvas isn't busy.
//!
//! shortcut: the text is the pages read on and near the screen (`Near`'s reach); other pages read as empty until
//! the reader scrolls to them. Reading the whole document for a screen reader's "say all" waits for K.13's
//! document-wide text. `FindText` and text attributes (font, size) answer "none" and "not supported".

// UIA's constants (`UIA_NamePropertyId`, ...) are matched as patterns.
#![allow(non_upper_case_globals)]

use std::cell::{Cell, RefCell};
use std::cmp::Ordering;
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use excellent_canvas::Rect;
use excellent_text::{TextPos, TextUnit as Unit};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
use windows::Win32::System::Com::SAFEARRAY;
use windows::Win32::System::Ole::{SafeArrayCreateVector, SafeArrayPutElement};
use windows::Win32::System::Variant::{VARENUM, VARIANT, VT_R8, VT_UNKNOWN};
use windows::Win32::UI::Accessibility::*;
use windows::Win32::UI::Input::KeyboardAndMouse::GetFocus;
use windows::Win32::UI::WindowsAndMessaging::{GetClientRect, PostMessageW, WM_APP};
use windows::core::{
    BOOL, BSTR, Error, HRESULT, IUnknown, IUnknownImpl, Interface, Ref, Result, implement,
};

use crate::host::Canvas;

/// The canvas window's message that raises a UI Automation event: `wparam` is [`TEXT_CHANGED`] or
/// [`SELECTION_CHANGED`].
pub(crate) const WM_UIA_EVENT: u32 = WM_APP + 2;
const TEXT_CHANGED: usize = 1;
const SELECTION_CHANGED: usize = 2;

thread_local! {
    /// Every canvas on this thread, by its window.
    static CANVASES: RefCell<HashMap<isize, Weak<RefCell<Canvas>>>> = RefCell::default();
}

fn key(hwnd: HWND) -> isize {
    hwnd.0 as isize
}

fn window(key: isize) -> HWND {
    HWND(key as *mut _)
}

/// Lets screen readers find the canvas of window `hwnd`.
pub(crate) fn register(hwnd: HWND, canvas: &Rc<RefCell<Canvas>>) {
    CANVASES.with(|all| all.borrow_mut().insert(key(hwnd), Rc::downgrade(canvas)));
}

/// Window `hwnd` is going: its providers answer "not available" from now on.
pub(crate) fn forget(hwnd: HWND) {
    CANVASES.with(|all| all.borrow_mut().remove(&key(hwnd)));
}

fn canvas_of(key: isize) -> Result<Rc<RefCell<Canvas>>> {
    CANVASES
        .with(|all| all.borrow().get(&key).and_then(Weak::upgrade))
        .ok_or_else(not_available)
}

/// Runs `f` on the canvas of window `key`; "not available" when it is gone or busy.
fn read<T>(key: isize, f: impl FnOnce(&Canvas) -> Result<T>) -> Result<T> {
    let canvas = canvas_of(key)?;
    let canvas = canvas.try_borrow().map_err(|_| not_available())?;
    f(&canvas)
}

/// As [`read`], for a change.
fn write<T>(key: isize, f: impl FnOnce(&mut Canvas) -> Result<T>) -> Result<T> {
    let canvas = canvas_of(key)?;
    let mut canvas = canvas.try_borrow_mut().map_err(|_| not_available())?;
    f(&mut canvas)
}

fn not_available() -> Error {
    Error::from_hresult(HRESULT(UIA_E_ELEMENTNOTAVAILABLE as i32))
}

fn invalid_operation() -> Error {
    Error::from_hresult(HRESULT(UIA_E_INVALIDOPERATION as i32))
}

/// A null result (COM's "no such element" for out parameters).
fn none<T>() -> Result<T> {
    Err(Error::empty())
}

// --- Window messages and events ---------------------------------------------

/// `WM_GETOBJECT`: answers UI Automation with the root provider; `None` for other requests.
pub(crate) fn get_object(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) -> Option<LRESULT> {
    if lparam.0 as i32 != UiaRootObjectId || canvas_of(key(hwnd)).is_err() {
        return None;
    }
    let root: IRawElementProviderSimple = Root { hwnd: key(hwnd) }.into();
    // SAFETY: the window's own message, answered with a live provider.
    Some(unsafe { UiaReturnRawElementProvider(hwnd, wparam, lparam, &root) })
}

/// The text shown changed: pages, links or notes were read, or the document changed.
pub(crate) fn content_changed(hwnd: HWND) {
    post(hwnd, TEXT_CHANGED);
}

/// The selection changed.
pub(crate) fn selection_changed(hwnd: HWND) {
    post(hwnd, SELECTION_CHANGED);
}

fn post(hwnd: HWND, event: usize) {
    // SAFETY: plain calls; a stale window drops the message.
    unsafe {
        if UiaClientsAreListening().as_bool() {
            let _ = PostMessageW(Some(hwnd), WM_UIA_EVENT, WPARAM(event), LPARAM(0));
        }
    }
}

/// [`WM_UIA_EVENT`]: raises the event it names on the canvas's root.
pub(crate) fn raise(hwnd: HWND, wparam: WPARAM) {
    let event = match wparam.0 {
        TEXT_CHANGED => UIA_Text_TextChangedEventId,
        SELECTION_CHANGED => UIA_Text_TextSelectionChangedEventId,
        _ => return,
    };
    let root: IRawElementProviderSimple = Root { hwnd: key(hwnd) }.into();
    // SAFETY: plain call with a live provider.
    let _ = unsafe { UiaRaiseAutomationEvent(&root, event) };
}

// --- Geometry and arrays ------------------------------------------------------

/// The window's client area's top-left corner on the screen.
fn origin(key: isize) -> (f64, f64) {
    let mut p = POINT::default();
    // SAFETY: a point to fill; a stale window handle fails harmlessly.
    let _ = unsafe { ClientToScreen(window(key), &mut p) };
    (f64::from(p.x), f64::from(p.y))
}

/// `area` (client pixels) on the screen.
fn on_screen(key: isize, area: Rect) -> UiaRect {
    let (x, y) = origin(key);
    UiaRect {
        left: x + f64::from(area.x0),
        top: y + f64::from(area.y0),
        width: f64::from(area.x1 - area.x0),
        height: f64::from(area.y1 - area.y0),
    }
}

/// Whether `area` (client pixels) shows in the window.
fn shows(key: isize, area: Rect) -> bool {
    let mut client = RECT::default();
    // SAFETY: a rectangle to fill; a stale window handle fails harmlessly.
    let _ = unsafe { GetClientRect(window(key), &mut client) };
    area.x1 > 0.0
        && area.y1 > 0.0
        && area.x0 < client.right as f32
        && area.y0 < client.bottom as f32
}

/// A new one-dimensional SAFEARRAY of `len` elements of type `vt`, handed over to the caller.
fn array(vt: VARENUM, len: usize) -> Result<*mut SAFEARRAY> {
    // SAFETY: plain call; the array belongs to the caller once returned.
    let array = unsafe { SafeArrayCreateVector(vt, 0, u32::try_from(len).unwrap_or(u32::MAX)) };
    if array.is_null() {
        return Err(Error::from_hresult(
            windows::Win32::Foundation::E_OUTOFMEMORY,
        ));
    }
    Ok(array)
}

/// The rectangles `rects` as UIA wants them: left, top, width and height of each, in a row.
fn rect_array(rects: &[UiaRect]) -> Result<*mut SAFEARRAY> {
    let values: Vec<f64> = rects
        .iter()
        .flat_map(|r| [r.left, r.top, r.width, r.height])
        .collect();
    let out = array(VT_R8, values.len())?;
    for (i, v) in values.iter().enumerate() {
        let index = i32::try_from(i).unwrap_or(i32::MAX);
        // SAFETY: `out` has `values.len()` doubles; the value is copied in.
        unsafe { SafeArrayPutElement(out, &index, std::ptr::from_ref(v).cast())? };
    }
    Ok(out)
}

/// `ranges` in a SAFEARRAY of interfaces.
fn range_array(ranges: &[ITextRangeProvider]) -> Result<*mut SAFEARRAY> {
    let out = array(VT_UNKNOWN, ranges.len())?;
    for (i, range) in ranges.iter().enumerate() {
        let index = i32::try_from(i).unwrap_or(i32::MAX);
        // SAFETY: for VT_UNKNOWN the element is the interface pointer itself, which the array AddRefs.
        unsafe { SafeArrayPutElement(out, &index, range.as_raw())? };
    }
    Ok(out)
}

fn range(key: isize, start: TextPos, end: TextPos) -> ITextRangeProvider {
    Range {
        hwnd: key,
        start: Cell::new(start),
        end: Cell::new(end),
    }
    .into()
}

fn unit(unit: TextUnit) -> Unit {
    match unit {
        TextUnit_Character => Unit::Character,
        TextUnit_Format | TextUnit_Word => Unit::Word,
        TextUnit_Line => Unit::Line,
        TextUnit_Paragraph => Unit::Paragraph,
        TextUnit_Page => Unit::Page,
        _ => Unit::Document,
    }
}

// --- The root -------------------------------------------------------------------

/// The canvas: a Document of pages, with the Text pattern.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IRawElementProviderFragmentRoot,
    ITextProvider
)]
struct Root {
    hwnd: isize,
}

impl Root_Impl {
    fn item(&self, page: usize, part: Part) -> IRawElementProviderFragment {
        Item {
            hwnd: self.hwnd,
            page,
            part,
        }
        .into()
    }
}

impl IRawElementProviderSimple_Impl for Root_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        if pattern == UIA_TextPatternId {
            Ok(self.to_object().to_interface())
        } else {
            none()
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        let name = read(self.hwnd, |canvas| Ok(canvas.file_name()))?;
        Ok(match property {
            UIA_ControlTypePropertyId => VARIANT::from(UIA_DocumentControlTypeId.0),
            UIA_NamePropertyId => VARIANT::from(
                match name {
                    Some(name) => nsh_core::t!("canvas.pages", name = name),
                    None => nsh_core::t!("canvas.pages_none"),
                }
                .as_str(),
            ),
            UIA_AutomationIdPropertyId => VARIANT::from("pages"),
            UIA_IsKeyboardFocusablePropertyId
            | UIA_IsContentElementPropertyId
            | UIA_IsControlElementPropertyId => VARIANT::from(true),
            // SAFETY: plain query.
            UIA_HasKeyboardFocusPropertyId => {
                VARIANT::from(unsafe { GetFocus() } == window(self.hwnd))
            }
            UIA_IsTextPatternAvailablePropertyId => VARIANT::from(true),
            _ => VARIANT::default(),
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        canvas_of(self.hwnd)?;
        // SAFETY: the canvas's live window.
        unsafe { UiaHostProviderFromHwnd(window(self.hwnd)) }
    }
}

impl IRawElementProviderFragment_Impl for Root_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let pages = read(self.hwnd, |canvas| Ok(canvas.page_count()))?;
        match direction {
            NavigateDirection_FirstChild if pages > 0 => Ok(self.item(0, Part::Page)),
            NavigateDirection_LastChild if pages > 0 => Ok(self.item(pages - 1, Part::Page)),
            _ => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        // The window's provider gives the root its runtime id.
        Ok(std::ptr::null_mut())
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        // The window's provider gives the root its bounds.
        Ok(UiaRect::default())
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        // SAFETY: plain call on the canvas's live window.
        let _ = unsafe {
            windows::Win32::UI::Input::KeyboardAndMouse::SetFocus(Some(window(self.hwnd)))
        };
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(self.to_object().to_interface())
    }
}

impl IRawElementProviderFragmentRoot_Impl for Root_Impl {
    fn ElementProviderFromPoint(&self, x: f64, y: f64) -> Result<IRawElementProviderFragment> {
        let (ox, oy) = origin(self.hwnd);
        let (x, y) = ((x - ox) as f32, (y - oy) as f32);
        let inside = |area: Option<Rect>| {
            area.is_some_and(|a| (a.x0..=a.x1).contains(&x) && (a.y0..=a.y1).contains(&y))
        };
        let found = read(self.hwnd, |canvas| {
            let Some(pages) = canvas.shown_pages() else {
                return Ok(None);
            };
            for page in pages {
                if !inside(canvas.page_area(page)) {
                    continue;
                }
                let link = (canvas.links_on(page).iter().enumerate().rev())
                    .find(|(_, link)| inside(canvas.area_on_page(page, link.rect)));
                if let Some((index, _)) = link {
                    return Ok(Some((page, Part::Link(index))));
                }
                let note = (canvas.notes_on(page).iter().enumerate().rev())
                    .find(|(_, note)| inside(canvas.area_on_page(page, note.rect)));
                if let Some((index, _)) = note {
                    return Ok(Some((page, Part::Note(index))));
                }
                return Ok(Some((page, Part::Page)));
            }
            Ok(None)
        })?;
        Ok(match found {
            Some((page, part)) => self.item(page, part),
            None => self.to_object().to_interface(),
        })
    }

    fn GetFocus(&self) -> Result<IRawElementProviderFragment> {
        // The canvas itself has the focus, not a page or link in it.
        none()
    }
}

impl ITextProvider_Impl for Root_Impl {
    fn GetSelection(&self) -> Result<*mut SAFEARRAY> {
        let selected = read(self.hwnd, |canvas| Ok(canvas.selected()))?;
        match selected {
            Some((start, end)) => range_array(&[range(self.hwnd, start, end)]),
            None => range_array(&[]),
        }
    }

    fn GetVisibleRanges(&self) -> Result<*mut SAFEARRAY> {
        let shown = read(self.hwnd, |canvas| {
            let Some(pages) = canvas.shown_pages() else {
                return Ok(None);
            };
            let text = canvas.document_text();
            let start = TextPos {
                page: pages.start,
                index: 0,
            };
            let last = TextPos {
                page: pages.end - 1,
                index: 0,
            };
            Ok(Some((start, text.unit_end(last, Unit::Page))))
        })?;
        match shown {
            Some((start, end)) => range_array(&[range(self.hwnd, start, end)]),
            None => range_array(&[]),
        }
    }

    fn RangeFromChild(&self, child: Ref<IRawElementProviderSimple>) -> Result<ITextRangeProvider> {
        // SAFETY: plain call on a live provider.
        let id = unsafe { child.ok()?.GetPropertyValue(UIA_AutomationIdPropertyId)? };
        let id = BSTR::try_from(&id)
            .map_err(|_| invalid_operation())?
            .to_string();
        let (page, part) = Part::parse(&id).ok_or_else(invalid_operation)?;
        let (start, end) = read(self.hwnd, |canvas| {
            let text = canvas.document_text();
            let top = TextPos { page, index: 0 };
            let area = match part {
                Part::Page => return Ok((top, text.unit_end(top, Unit::Page))),
                Part::Link(i) => canvas.links_on(page).get(i).map(|link| link.rect),
                Part::Note(i) => canvas.notes_on(page).get(i).map(|note| note.rect),
            };
            let chars = area.and_then(|area| canvas.text_within(page, area));
            Ok(chars.map_or((top, top), |chars| {
                (
                    TextPos {
                        page,
                        index: chars.start,
                    },
                    TextPos {
                        page,
                        index: chars.end,
                    },
                )
            }))
        })?;
        Ok(range(self.hwnd, start, end))
    }

    fn RangeFromPoint(&self, point: &UiaPoint) -> Result<ITextRangeProvider> {
        let mut p = POINT {
            x: point.x as i32,
            y: point.y as i32,
        };
        // SAFETY: a point to convert; a stale window handle fails harmlessly.
        let _ = unsafe { ScreenToClient(window(self.hwnd), &mut p) };
        let at = read(self.hwnd, |canvas| {
            Ok(canvas
                .caret_at(p.x, p.y)
                .unwrap_or_else(|| canvas.document_text().start()))
        })?;
        Ok(range(self.hwnd, at, at))
    }

    fn DocumentRange(&self) -> Result<ITextRangeProvider> {
        let (start, end) = read(self.hwnd, |canvas| {
            let text = canvas.document_text();
            Ok((text.start(), text.end()))
        })?;
        Ok(range(self.hwnd, start, end))
    }

    fn SupportedTextSelection(&self) -> Result<SupportedTextSelection> {
        Ok(SupportedTextSelection_Single)
    }
}

// --- Pages, links and notes -------------------------------------------------------

/// What an [`Item`] is on its page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    Page,
    /// The page's link at this index.
    Link(usize),
    /// The page's note at this index.
    Note(usize),
}

impl Part {
    /// The automation id of `self` on `page`: `page-3`, `page-3-link-0`, `page-3-note-1`.
    fn id(self, page: usize) -> String {
        match self {
            Part::Page => format!("page-{page}"),
            Part::Link(i) => format!("page-{page}-link-{i}"),
            Part::Note(i) => format!("page-{page}-note-{i}"),
        }
    }

    /// The page and part an automation id names.
    fn parse(id: &str) -> Option<(usize, Part)> {
        let rest = id.strip_prefix("page-")?;
        let mut parts = rest.split('-');
        let page = parts.next()?.parse().ok()?;
        let part = match (parts.next(), parts.next().map(str::parse)) {
            (None, _) => Part::Page,
            (Some("link"), Some(Ok(i))) => Part::Link(i),
            (Some("note"), Some(Ok(i))) => Part::Note(i),
            _ => return None,
        };
        Some((page, part))
    }

    /// The kind number in the runtime id.
    fn kind(self) -> i32 {
        match self {
            Part::Page => 1,
            Part::Link(_) => 2,
            Part::Note(_) => 3,
        }
    }

    fn index(self) -> usize {
        match self {
            Part::Page => 0,
            Part::Link(i) | Part::Note(i) => i,
        }
    }
}

/// A page, or a link or note on one.
#[implement(
    IRawElementProviderSimple,
    IRawElementProviderFragment,
    IInvokeProvider
)]
struct Item {
    hwnd: isize,
    page: usize,
    part: Part,
}

impl Item_Impl {
    fn item(&self, page: usize, part: Part) -> IRawElementProviderFragment {
        Item {
            hwnd: self.hwnd,
            page,
            part,
        }
        .into()
    }

    /// The parts of page `page`: its links, then its notes.
    fn children(canvas: &Canvas, page: usize) -> Vec<Part> {
        let links = (0..canvas.links_on(page).len()).map(Part::Link);
        links
            .chain((0..canvas.notes_on(page).len()).map(Part::Note))
            .collect()
    }

    /// Whether the item still exists in `canvas`.
    fn exists(&self, canvas: &Canvas) -> bool {
        self.page < canvas.page_count()
            && match self.part {
                Part::Page => true,
                Part::Link(i) => i < canvas.links_on(self.page).len(),
                Part::Note(i) => i < canvas.notes_on(self.page).len(),
            }
    }

    /// Its area, in client pixels.
    fn area(&self, canvas: &Canvas) -> Option<Rect> {
        match self.part {
            Part::Page => canvas.page_area(self.page),
            Part::Link(i) => {
                canvas.area_on_page(self.page, canvas.links_on(self.page).get(i)?.rect)
            }
            Part::Note(i) => {
                canvas.area_on_page(self.page, canvas.notes_on(self.page).get(i)?.rect)
            }
        }
    }

    /// What a screen reader calls it.
    fn name(&self, canvas: &Canvas) -> String {
        match self.part {
            Part::Page => nsh_core::t!(
                "canvas.page",
                number = self.page + 1,
                total = canvas.page_count()
            ),
            Part::Link(i) => {
                let Some(link) = canvas.links_on(self.page).get(i) else {
                    return String::new();
                };
                let words = canvas
                    .text_within(self.page, link.rect)
                    .map(|chars| {
                        canvas.document_text().text(
                            TextPos {
                                page: self.page,
                                index: chars.start,
                            },
                            TextPos {
                                page: self.page,
                                index: chars.end,
                            },
                            None,
                        )
                    })
                    .filter(|words| !words.trim().is_empty());
                match (words, link.target) {
                    (Some(words), _) => words,
                    (None, Some(target)) => {
                        nsh_core::t!("canvas.link_to_page", number = target.page + 1)
                    }
                    (None, None) => link.uri.clone(),
                }
            }
            Part::Note(i) => canvas
                .notes_on(self.page)
                .get(i)
                .map(|note| note.contents.clone())
                .unwrap_or_default(),
        }
    }
}

impl IRawElementProviderSimple_Impl for Item_Impl {
    fn ProviderOptions(&self) -> Result<ProviderOptions> {
        Ok(ProviderOptions_ServerSideProvider | ProviderOptions_UseComThreading)
    }

    fn GetPatternProvider(&self, pattern: UIA_PATTERN_ID) -> Result<IUnknown> {
        if pattern == UIA_InvokePatternId && matches!(self.part, Part::Link(_)) {
            Ok(self.to_object().to_interface())
        } else {
            none()
        }
    }

    fn GetPropertyValue(&self, property: UIA_PROPERTY_ID) -> Result<VARIANT> {
        read(self.hwnd, |canvas| {
            if !self.exists(canvas) {
                return Err(not_available());
            }
            Ok(match property {
                UIA_ControlTypePropertyId => VARIANT::from(match self.part {
                    Part::Link(_) => UIA_HyperlinkControlTypeId.0,
                    Part::Page | Part::Note(_) => UIA_CustomControlTypeId.0,
                }),
                UIA_LocalizedControlTypePropertyId => match self.part {
                    Part::Page => VARIANT::from(nsh_core::t!("canvas.page_type").as_str()),
                    Part::Note(_) => VARIANT::from(nsh_core::t!("canvas.note_type").as_str()),
                    Part::Link(_) => VARIANT::default(),
                },
                UIA_NamePropertyId => VARIANT::from(self.name(canvas).as_str()),
                UIA_AutomationIdPropertyId => VARIANT::from(self.part.id(self.page).as_str()),
                UIA_HelpTextPropertyId => match self.part {
                    Part::Link(i) => match canvas.links_on(self.page).get(i) {
                        Some(link) if link.target.is_none() => VARIANT::from(link.uri.as_str()),
                        _ => VARIANT::default(),
                    },
                    _ => VARIANT::default(),
                },
                UIA_IsOffscreenPropertyId => {
                    VARIANT::from(!self.area(canvas).is_some_and(|area| shows(self.hwnd, area)))
                }
                UIA_IsContentElementPropertyId
                | UIA_IsControlElementPropertyId
                | UIA_IsEnabledPropertyId => VARIANT::from(true),
                UIA_IsKeyboardFocusablePropertyId => VARIANT::from(false),
                _ => VARIANT::default(),
            })
        })
    }

    fn HostRawElementProvider(&self) -> Result<IRawElementProviderSimple> {
        none()
    }
}

impl IRawElementProviderFragment_Impl for Item_Impl {
    fn Navigate(&self, direction: NavigateDirection) -> Result<IRawElementProviderFragment> {
        let found = read(self.hwnd, |canvas| {
            if !self.exists(canvas) {
                return Err(not_available());
            }
            Ok(match (self.part, direction) {
                (Part::Page, NavigateDirection_Parent) => None,
                (Part::Page, NavigateDirection_FirstChild) => {
                    Item_Impl::children(canvas, self.page)
                        .first()
                        .map(|&part| (self.page, part))
                }
                (Part::Page, NavigateDirection_LastChild) => Item_Impl::children(canvas, self.page)
                    .last()
                    .map(|&part| (self.page, part)),
                (Part::Page, NavigateDirection_NextSibling) => {
                    (self.page + 1 < canvas.page_count()).then_some((self.page + 1, Part::Page))
                }
                (Part::Page, NavigateDirection_PreviousSibling) => {
                    self.page.checked_sub(1).map(|page| (page, Part::Page))
                }
                (_, NavigateDirection_Parent) => Some((self.page, Part::Page)),
                (_, NavigateDirection_NextSibling | NavigateDirection_PreviousSibling) => {
                    let siblings = Item_Impl::children(canvas, self.page);
                    let at = siblings.iter().position(|&part| part == self.part);
                    let next = at.and_then(|i| {
                        if direction == NavigateDirection_NextSibling {
                            siblings.get(i + 1)
                        } else {
                            i.checked_sub(1).and_then(|i| siblings.get(i))
                        }
                    });
                    next.map(|&part| (self.page, part))
                }
                _ => None,
            })
        });
        match (found?, self.part, direction) {
            (Some((page, part)), _, _) => Ok(self.item(page, part)),
            (None, Part::Page, NavigateDirection_Parent) => Ok(Root { hwnd: self.hwnd }.into()),
            (None, _, _) => none(),
        }
    }

    fn GetRuntimeId(&self) -> Result<*mut SAFEARRAY> {
        let id = [
            UiaAppendRuntimeId as i32,
            self.part.kind(),
            i32::try_from(self.page).unwrap_or(i32::MAX),
            i32::try_from(self.part.index()).unwrap_or(i32::MAX),
        ];
        let out = array(windows::Win32::System::Variant::VT_I4, id.len())?;
        for (i, v) in id.iter().enumerate() {
            let index = i32::try_from(i).unwrap_or(i32::MAX);
            // SAFETY: `out` has four integers; the value is copied in.
            unsafe { SafeArrayPutElement(out, &index, std::ptr::from_ref(v).cast())? };
        }
        Ok(out)
    }

    fn BoundingRectangle(&self) -> Result<UiaRect> {
        let area = read(self.hwnd, |canvas| Ok(self.area(canvas)))?;
        Ok(area.map_or_else(UiaRect::default, |area| on_screen(self.hwnd, area)))
    }

    fn GetEmbeddedFragmentRoots(&self) -> Result<*mut SAFEARRAY> {
        Ok(std::ptr::null_mut())
    }

    fn SetFocus(&self) -> Result<()> {
        // Pages, links and notes take no keyboard focus of their own: the canvas has it.
        Ok(())
    }

    fn FragmentRoot(&self) -> Result<IRawElementProviderFragmentRoot> {
        Ok(Root { hwnd: self.hwnd }.into())
    }
}

impl IInvokeProvider_Impl for Item_Impl {
    fn Invoke(&self) -> Result<()> {
        let Part::Link(i) = self.part else {
            return Err(invalid_operation());
        };
        write(self.hwnd, |canvas| {
            let link = canvas
                .links_on(self.page)
                .get(i)
                .cloned()
                .ok_or_else(not_available)?;
            canvas.follow(&link);
            Ok(())
        })
    }
}

// --- Text ranges ----------------------------------------------------------------

/// A stretch of the document's text, from caret `start` to caret `end`.
#[implement(ITextRangeProvider)]
struct Range {
    hwnd: isize,
    start: Cell<TextPos>,
    end: Cell<TextPos>,
}

impl Range_Impl {
    fn endpoint(&self, which: TextPatternRangeEndpoint) -> TextPos {
        if which == TextPatternRangeEndpoint_Start {
            self.start.get()
        } else {
            self.end.get()
        }
    }

    /// Moves endpoint `which` to `to`, pulling the other along when they would cross.
    fn set_endpoint(&self, which: TextPatternRangeEndpoint, to: TextPos) {
        if which == TextPatternRangeEndpoint_Start {
            self.start.set(to);
            if self.end.get() < to {
                self.end.set(to);
            }
        } else {
            self.end.set(to);
            if self.start.get() > to {
                self.start.set(to);
            }
        }
    }

    /// Another range of this provider.
    fn other<'a>(range: &'a Ref<ITextRangeProvider>) -> Result<&'a Range> {
        let range = range.ok()?;
        // SAFETY: UI Automation only hands a provider ranges that provider made (the text pattern's rules), and
        // every range this module makes is a `Range`.
        Ok(unsafe { windows::core::AsImpl::<Range>::as_impl(range) })
    }
}

impl ITextRangeProvider_Impl for Range_Impl {
    fn Clone(&self) -> Result<ITextRangeProvider> {
        Ok(range(self.hwnd, self.start.get(), self.end.get()))
    }

    fn Compare(&self, other: Ref<ITextRangeProvider>) -> Result<BOOL> {
        let other = Range_Impl::other(&other)?;
        Ok(BOOL::from(
            self.start.get() == other.start.get() && self.end.get() == other.end.get(),
        ))
    }

    fn CompareEndpoints(
        &self,
        which: TextPatternRangeEndpoint,
        target: Ref<ITextRangeProvider>,
        target_which: TextPatternRangeEndpoint,
    ) -> Result<i32> {
        let other = Range_Impl::other(&target)?;
        let theirs = if target_which == TextPatternRangeEndpoint_Start {
            other.start.get()
        } else {
            other.end.get()
        };
        Ok(match self.endpoint(which).cmp(&theirs) {
            Ordering::Less => -1,
            Ordering::Equal => 0,
            Ordering::Greater => 1,
        })
    }

    fn ExpandToEnclosingUnit(&self, by: TextUnit) -> Result<()> {
        let (start, end) = read(self.hwnd, |canvas| {
            Ok(canvas.document_text().expand(self.start.get(), unit(by)))
        })?;
        self.start.set(start);
        self.end.set(end);
        Ok(())
    }

    fn FindAttribute(
        &self,
        _attribute: UIA_TEXTATTRIBUTE_ID,
        _value: &VARIANT,
        _backward: BOOL,
    ) -> Result<ITextRangeProvider> {
        none()
    }

    fn FindText(
        &self,
        _text: &BSTR,
        _backward: BOOL,
        _ignore_case: BOOL,
    ) -> Result<ITextRangeProvider> {
        none()
    }

    fn GetAttributeValue(&self, attribute: UIA_TEXTATTRIBUTE_ID) -> Result<VARIANT> {
        if attribute == UIA_IsReadOnlyAttributeId {
            return Ok(VARIANT::from(true));
        }
        // SAFETY: plain call.
        Ok(VARIANT::from(unsafe { UiaGetReservedNotSupportedValue()? }))
    }

    fn GetBoundingRectangles(&self) -> Result<*mut SAFEARRAY> {
        let rects = read(self.hwnd, |canvas| {
            let text = canvas.document_text();
            let spans = text.spans(self.start.get(), self.end.get());
            Ok(spans
                .into_iter()
                .flat_map(|(page, chars)| canvas.text_areas(page, chars))
                .filter(|&area| shows(self.hwnd, area))
                .map(|area| on_screen(self.hwnd, area))
                .collect::<Vec<_>>())
        })?;
        rect_array(&rects)
    }

    fn GetEnclosingElement(&self) -> Result<IRawElementProviderSimple> {
        Ok(Root { hwnd: self.hwnd }.into())
    }

    fn GetText(&self, max: i32) -> Result<BSTR> {
        let max = usize::try_from(max).ok();
        let text = read(self.hwnd, |canvas| {
            Ok(canvas
                .document_text()
                .text(self.start.get(), self.end.get(), max))
        })?;
        Ok(BSTR::from(text))
    }

    fn Move(&self, by: TextUnit, count: i32) -> Result<i32> {
        let by = unit(by);
        let collapsed = self.start.get() == self.end.get();
        let (start, end, moved) = read(self.hwnd, |canvas| {
            let text = canvas.document_text();
            let from = text.unit_start(self.start.get(), by);
            let (start, moved) = text.move_by(from, by, count, false);
            let end = if collapsed {
                start
            } else {
                text.unit_end(start, by)
            };
            Ok((start, end, moved))
        })?;
        if moved != 0 {
            self.start.set(start);
            self.end.set(end);
        }
        Ok(moved)
    }

    fn MoveEndpointByUnit(
        &self,
        which: TextPatternRangeEndpoint,
        by: TextUnit,
        count: i32,
    ) -> Result<i32> {
        let from = self.endpoint(which);
        let (to, moved) = read(self.hwnd, |canvas| {
            Ok(canvas.document_text().move_by(from, unit(by), count, true))
        })?;
        self.set_endpoint(which, to);
        Ok(moved)
    }

    fn MoveEndpointByRange(
        &self,
        which: TextPatternRangeEndpoint,
        target: Ref<ITextRangeProvider>,
        target_which: TextPatternRangeEndpoint,
    ) -> Result<()> {
        let other = Range_Impl::other(&target)?;
        let to = if target_which == TextPatternRangeEndpoint_Start {
            other.start.get()
        } else {
            other.end.get()
        };
        self.set_endpoint(which, to);
        Ok(())
    }

    fn Select(&self) -> Result<()> {
        write(self.hwnd, |canvas| {
            canvas.select(self.start.get(), self.end.get());
            Ok(())
        })?;
        selection_changed(window(self.hwnd));
        Ok(())
    }

    fn AddToSelection(&self) -> Result<()> {
        // One selection at a time (`SupportedTextSelection_Single`).
        Err(invalid_operation())
    }

    fn RemoveFromSelection(&self) -> Result<()> {
        Err(invalid_operation())
    }

    fn ScrollIntoView(&self, align_to_top: BOOL) -> Result<()> {
        let at = if align_to_top.as_bool() {
            self.start.get()
        } else {
            self.end.get()
        };
        write(self.hwnd, |canvas| {
            canvas.show_text(at);
            Ok(())
        })
    }

    fn GetChildren(&self) -> Result<*mut SAFEARRAY> {
        // Links and notes are the pages' children, not the text's.
        array(VT_UNKNOWN, 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automation_ids_name_their_page_and_part() {
        for (page, part) in [(0, Part::Page), (12, Part::Link(3)), (4, Part::Note(0))] {
            assert_eq!(Part::parse(&part.id(page)), Some((page, part)));
        }
        assert_eq!(Part::parse("page-2-ink-1"), None);
        assert_eq!(Part::parse("pages"), None);
    }
}
