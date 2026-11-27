//! Windows top-level window bridge (`ecclesia.window-bridge`): ensures every visible top-level
//! window on NSH's UI thread carries the embedded executable icon, is registered as an OLE
//! `IDropTarget` for `CF_HDROP` file drops from Windows Explorer, and intercepts OS-reserved or
//! NSH-dropped keyboard shortcuts (`F11`, `Ctrl+Q`, and numpad `VK_ADD`/`VK_SUBTRACT`).
//!
//! NSH 0.7's built-in `file_drop` and `F11` handlers target the hidden host HWND rather than
//! visible `app_window` HWNDs, and `Key::from_virtual_key` drops numpad `VK_ADD` / `VK_SUBTRACT`.
//! Subclassing each visible top-level HWND lets icons, Explorer drops (`"osFileDrop"`), and these
//! shortcuts (`"osShortcut"`) work uniformly across the main window, workspace windows, and
//! popped-out windows.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::ffi::OsString;
use std::os::windows::ffi::OsStringExt;
use std::path::PathBuf;

use nsh_core::JsonValue;
use nsh_platform_windows::{ComponentContext, NativeComponent};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINTL, WPARAM};
use windows::Win32::Graphics::Direct2D::ID2D1DeviceContext;
use windows::Win32::System::Com::{DVASPECT_CONTENT, FORMATETC, IDataObject, TYMED_HGLOBAL};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Ole::{
    CF_HDROP, DROPEFFECT, DROPEFFECT_COPY, DROPEFFECT_NONE, IDropTarget, IDropTarget_Impl,
    RegisterDragDrop, ReleaseStgMedium,
};
use windows::Win32::System::SystemServices::MODIFIERKEYS_FLAGS;
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::HiDpi::{GetDpiForWindow, GetSystemMetricsForDpi};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::Shell::{
    DefSubclassProc, DragQueryFileW, HDROP, RemoveWindowSubclass, SetWindowSubclass,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetForegroundWindow, ICON_BIG, ICON_SMALL, IMAGE_ICON, IsWindow,
    IsWindowVisible, LR_SHARED, LoadImageW, MSG, PM_REMOVE, PeekMessageW, SM_CXICON, SM_CXSMICON,
    SendMessageW, WM_CHAR, WM_KEYDOWN, WM_NCDESTROY, WM_SETICON, WM_SYSCHAR, WM_SYSKEYDOWN,
};
use windows::core::{BOOL, HRESULT, PCWSTR, Ref, Result, implement};

/// The component name registered with NSH for the Windows top-level window bridge.
pub(crate) const NAME: &str = "ecclesia.window-bridge";

/// OLE error returned by `RegisterDragDrop` when an HWND already has a registered drop target.
const DRAGDROP_E_ALREADYREGISTERED: HRESULT = HRESULT(0x8004_0101_u32 as i32);

/// Unique `SetWindowSubclass` identifier owned by [`WindowBridge`]'s keyboard shortcut hook.
const SHORTCUT_SUBCLASS_ID: usize = 0xECC1_E51A;

/// Win32 virtual-key code for `'Q'` (`0x51`).
const VK_KEY_Q: u16 = 0x51;
/// Win32 virtual-key code for numpad `+` (`VK_ADD`, `0x6B`).
const VK_KEY_ADD: u16 = 0x6B;
/// Win32 virtual-key code for numpad `-` (`VK_SUBTRACT`, `0x6D`).
const VK_KEY_SUBTRACT: u16 = 0x6D;
/// Win32 virtual-key code for `F11` (`VK_F11`, `0x7A`).
const VK_KEY_F11: u16 = 0x7A;
/// `WM_KEYDOWN` / `WM_SYSKEYDOWN` `lParam` bit 30 indicating an auto-repeated held key.
const KEY_REPEAT_BIT: isize = 1_isize << 30;

/// Modifier keys held at the moment a key-down message is processed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct KeyModifiers {
    ctrl: bool,
    shift: bool,
    alt: bool,
    meta: bool,
}

/// Whether the window's page-scoped plain shortcuts are currently allowed to intercept keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PageKeysMode {
    /// A dialog, Home page, sidebar list, or text input owns plain keys.
    Inactive,
    /// The document canvas is active and accepts unmodified numpad `+`/`-` zoom keys.
    Active,
}

/// Whether a key-down message is the initial press or a keyboard auto-repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KeyPressKind {
    Initial,
    Repeat,
}

/// How the window subclass responds to a `WM_KEYDOWN` / `WM_SYSKEYDOWN` message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShortcutDecision {
    /// Not handled here; pass through to `DefSubclassProc`.
    PassThrough,
    /// Consume the key without emitting an event (e.g. held repeat of `F11` or `Ctrl+Q`).
    ConsumeOnly,
    /// Consume the key and emit this `"osShortcut"` action name to the window bridge.
    Emit(&'static str),
}

/// A live `WindowBridge` instance registered on the UI thread.
struct BridgeEntry {
    id: u64,
    context: ComponentContext,
    hwnd: Option<isize>,
    page_keys_active: bool,
}

thread_local! {
    /// Monotonic identifier generator for active [`WindowBridge`] components on the UI thread.
    static NEXT_BRIDGE_ID: Cell<u64> = const { Cell::new(1) };
    /// Active [`WindowBridge`] contexts on the UI thread, used to deliver `"osFileDrop"` and
    /// `"osShortcut"` events.
    static BRIDGES: RefCell<Vec<BridgeEntry>> = const { RefCell::new(Vec::new()) };
    /// Visible top-level HWNDs on this thread that already received `WM_SETICON`.
    static APPLIED_ICONS: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
    /// Visible top-level HWNDs on this thread already processed by `RegisterDragDrop`.
    static REGISTERED_DROPS: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
    /// Visible top-level HWNDs on this thread already subclassed for OS/numpad shortcuts.
    static SUBCLASSED_WINDOWS: RefCell<HashSet<isize>> = RefCell::new(HashSet::new());
}

/// Lightweight, non-interactive native component that hooks visible top-level windows on NSH's UI
/// thread for app icons, Windows Explorer file drops, and OS/numpad keyboard shortcuts.
pub(crate) struct WindowBridge {
    id: u64,
}

impl WindowBridge {
    /// Creates a new window bridge bound to NSH's `context` and hooks any visible windows.
    pub(crate) fn new(context: ComponentContext) -> Self {
        let id = NEXT_BRIDGE_ID.with(|next| {
            let id = next.get();
            next.set(id.wrapping_add(1));
            id
        });
        BRIDGES.with(|bridges| {
            bridges.borrow_mut().push(BridgeEntry {
                id,
                context,
                hwnd: None,
                page_keys_active: false,
            });
        });
        associate_bridge_hwnd(id);
        ensure_window_hooks();
        Self { id }
    }
}

impl Drop for WindowBridge {
    fn drop(&mut self) {
        BRIDGES.with(|bridges| {
            bridges.borrow_mut().retain(|entry| entry.id != self.id);
        });
    }
}

impl NativeComponent for WindowBridge {
    fn draw(&self, _dc: &ID2D1DeviceContext, _size: (f32, f32)) {
        associate_bridge_hwnd(self.id);
        ensure_window_hooks();
    }

    fn set_prop(&mut self, name: &str, value: Option<&JsonValue>) {
        if name == "pageKeysActive" {
            let active = value.and_then(JsonValue::as_bool).unwrap_or(false);
            BRIDGES.with(|bridges| {
                if let Some(entry) = bridges.borrow_mut().iter_mut().find(|e| e.id == self.id) {
                    entry.page_keys_active = active;
                }
            });
        }
        associate_bridge_hwnd(self.id);
        ensure_window_hooks();
    }

    fn hit(&self, _x: f32, _y: f32, _size: (f32, f32)) -> bool {
        false
    }
}

/// Ensures every visible top-level window on the current UI thread carries the embedded app icon,
/// an OLE file drop target, and the OS/numpad shortcut subclass.
pub(crate) fn ensure_window_hooks() {
    ensure_window_icons();
    ensure_drop_targets();
    ensure_shortcut_subclasses();
}

/// Ensures every visible top-level window on this UI thread has the OS/numpad keyboard shortcut
/// subclass installed, once per live HWND.
pub(crate) fn ensure_shortcut_subclasses() {
    SUBCLASSED_WINDOWS.with(|set| prune_closed(&mut set.borrow_mut()));
    // SAFETY: enumerating top-level windows belonging to the current UI thread.
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(install_shortcut_subclass_proc),
            LPARAM(0),
        );
    }
}

/// Ensures every visible top-level window on this UI thread carries the embedded app icon
/// (`RT_GROUP_ICON` resource id `1`) for both `ICON_BIG` and `ICON_SMALL`, once per live HWND.
pub(crate) fn ensure_window_icons() {
    APPLIED_ICONS.with(|set| prune_closed(&mut set.borrow_mut()));
    // SAFETY: enumerating top-level windows belonging to the current UI thread.
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(apply_icon_proc), LPARAM(0));
    }
}

/// Ensures every visible top-level window on this UI thread has an OLE `IDropTarget` registered
/// for `CF_HDROP` file drops, once per live HWND.
pub(crate) fn ensure_drop_targets() {
    REGISTERED_DROPS.with(|set| prune_closed(&mut set.borrow_mut()));
    // SAFETY: enumerating top-level windows belonging to the current UI thread.
    unsafe {
        let _ = EnumThreadWindows(GetCurrentThreadId(), Some(register_drop_proc), LPARAM(0));
    }
}

/// Removes destroyed window handles from `handles` so recycled HWND values are re-hooked.
fn prune_closed(handles: &mut HashSet<isize>) {
    handles.retain(|&raw| is_live_hwnd(raw));
}

/// Returns whether `raw` still refers to an existing Win32 window.
fn is_live_hwnd(raw: isize) -> bool {
    let hwnd = HWND(std::ptr::without_provenance_mut(raw as usize));
    // SAFETY: `IsWindow` safely validates whether a handle refers to a live window.
    unsafe { IsWindow(Some(hwnd)).as_bool() }
}

/// Associates `bridge_id` with an unclaimed visible top-level HWND on this UI thread if it does
/// not already track a live window.
fn associate_bridge_hwnd(bridge_id: u64) {
    let needs_hwnd = BRIDGES.with(|bridges| {
        let mut bridges = bridges.borrow_mut();
        for entry in bridges.iter_mut() {
            if let Some(raw) = entry.hwnd
                && !is_live_hwnd(raw)
            {
                entry.hwnd = None;
            }
        }
        bridges
            .iter()
            .find(|entry| entry.id == bridge_id)
            .is_some_and(|entry| entry.hwnd.is_none())
    });
    if !needs_hwnd {
        return;
    }

    let visible = visible_thread_hwnds();
    if visible.is_empty() {
        return;
    }
    // SAFETY: plain Win32 query returning the current foreground window handle.
    let foreground = unsafe { GetForegroundWindow() }.0 as isize;

    BRIDGES.with(|bridges| {
        let mut bridges = bridges.borrow_mut();
        let claimed: HashSet<isize> = bridges.iter().filter_map(|entry| entry.hwnd).collect();
        let chosen = if visible.contains(&foreground) && !claimed.contains(&foreground) {
            Some(foreground)
        } else {
            visible.iter().copied().find(|raw| !claimed.contains(raw))
        };
        if let Some(raw) = chosen
            && let Some(entry) = bridges.iter_mut().find(|entry| entry.id == bridge_id)
            && entry.hwnd.is_none()
        {
            entry.hwnd = Some(raw);
        }
    });
}

/// Returns the raw handles of all currently visible top-level windows on this UI thread.
fn visible_thread_hwnds() -> Vec<isize> {
    let mut handles = Vec::new();
    let ptr = (&raw mut handles) as isize;
    // SAFETY: `handles` stays alive on the stack for the entire synchronous `EnumThreadWindows`
    // call, and `collect_visible_proc` only pushes to it on this same UI thread.
    unsafe {
        let _ = EnumThreadWindows(
            GetCurrentThreadId(),
            Some(collect_visible_proc),
            LPARAM(ptr),
        );
    }
    handles
}

/// `EnumThreadWindows` callback that records each visible top-level HWND in the caller's `Vec`.
unsafe extern "system" fn collect_visible_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    // SAFETY: plain Win32 query on a live top-level window enumerated on this thread.
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    if lparam.0 != 0 {
        let handles = lparam.0 as *mut Vec<isize>;
        // SAFETY: `lparam` points to the stack-local `Vec<isize>` owned by `visible_thread_hwnds`.
        unsafe {
            (*handles).push(hwnd.0 as isize);
        }
    }
    BOOL(1)
}

/// `EnumThreadWindows` callback that applies the embedded executable icon once per visible window.
unsafe extern "system" fn apply_icon_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    // SAFETY: plain Win32 query on a live top-level window enumerated on this thread.
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    let already_set = APPLIED_ICONS.with(|set| !set.borrow_mut().insert(hwnd.0 as isize));
    if already_set {
        return BOOL(1);
    }
    // SAFETY: loading shared icon resources from this module (`id = 1`) and sending `WM_SETICON`
    // synchronously to a top-level window on this UI thread.
    unsafe {
        let Ok(instance) = GetModuleHandleW(None) else {
            return BOOL(1);
        };
        let dpi = match GetDpiForWindow(hwnd) {
            0 => 96,
            dpi => dpi,
        };
        for (metric, slot) in [(SM_CXICON, ICON_BIG), (SM_CXSMICON, ICON_SMALL)] {
            let side = GetSystemMetricsForDpi(metric, dpi).max(16);
            if let Ok(handle) = LoadImageW(
                Some(instance.into()),
                PCWSTR(std::ptr::without_provenance::<u16>(1)),
                IMAGE_ICON,
                side,
                side,
                LR_SHARED,
            ) {
                let _ = SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(slot as usize)),
                    Some(LPARAM(handle.0 as isize)),
                );
            }
        }
    }
    BOOL(1)
}

/// `EnumThreadWindows` callback that registers an OLE `IDropTarget` once per visible window.
unsafe extern "system" fn register_drop_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    // SAFETY: plain Win32 query on a live top-level window enumerated on this thread.
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    let raw = hwnd.0 as isize;
    let already_registered = REGISTERED_DROPS.with(|set| set.borrow().contains(&raw));
    if already_registered {
        return BOOL(1);
    }

    let target: IDropTarget = FileDropTarget {
        hwnd,
        offered: Cell::new(false),
    }
    .into();
    // SAFETY: `hwnd` is a live top-level window belonging to the current UI thread, where NSH has
    // initialized OLE. OLE retains `target` until the window is destroyed or revoked.
    match unsafe { RegisterDragDrop(hwnd, &target) } {
        Ok(()) => {
            REGISTERED_DROPS.with(|set| {
                set.borrow_mut().insert(raw);
            });
        }
        Err(err) if err.code() == DRAGDROP_E_ALREADYREGISTERED => {
            REGISTERED_DROPS.with(|set| {
                set.borrow_mut().insert(raw);
            });
        }
        Err(err) => {
            log::warn!("window bridge: RegisterDragDrop failed for HWND {raw:#x}: {err}");
            REGISTERED_DROPS.with(|set| {
                set.borrow_mut().insert(raw);
            });
        }
    }
    BOOL(1)
}

/// Builds the `FORMATETC` descriptor used to query and fetch `CF_HDROP` file lists.
fn hdrop_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
        ..Default::default()
    }
}

/// Returns whether the OLE drag source allows `DROPEFFECT_COPY`.
fn copy_allowed(effect: *mut DROPEFFECT) -> bool {
    if effect.is_null() {
        return false;
    }
    // SAFETY: OLE passes a valid in/out pointer to the allowed drop effects for this callback.
    unsafe { ((*effect).0 & DROPEFFECT_COPY.0) != 0 }
}

/// Writes `value` into OLE's out `effect` pointer when non-null.
fn write_drop_effect(effect: *mut DROPEFFECT, value: DROPEFFECT) {
    if !effect.is_null() {
        // SAFETY: OLE passes a valid writable out pointer for this callback.
        unsafe {
            *effect = value;
        }
    }
}

/// Sets OLE's drop effect to `DROPEFFECT_COPY` when files are offered and copy is allowed.
fn set_drop_effect(effect: *mut DROPEFFECT, offered: bool) {
    let chosen = if offered && copy_allowed(effect) {
        DROPEFFECT_COPY
    } else {
        DROPEFFECT_NONE
    };
    write_drop_effect(effect, chosen);
}

/// Extracts all dropped filesystem paths from a `CF_HDROP` `IDataObject`.
fn extract_drop_paths(data: &IDataObject) -> Vec<String> {
    // SAFETY: typed `CF_HDROP` request; the returned `STGMEDIUM` is released exactly once below.
    let Ok(mut medium) = (unsafe { data.GetData(&hdrop_format()) }) else {
        return Vec::new();
    };
    let mut paths = Vec::new();
    if medium.tymed == TYMED_HGLOBAL.0 as u32 {
        // SAFETY: `hGlobal` is the active union field for `TYMED_HGLOBAL`. Shell reads the
        // `HDROP` handle without taking ownership; `ReleaseStgMedium` frees it below.
        let drop = HDROP(unsafe { medium.u.hGlobal }.0);
        // SAFETY: querying the number of dropped files with `u32::MAX`.
        let count = unsafe { DragQueryFileW(drop, u32::MAX, None) };
        for index in 0..count {
            // SAFETY: querying the UTF-16 length (excluding NUL) of entry `index`.
            let length = unsafe { DragQueryFileW(drop, index, None) };
            let Some(capacity) = length.checked_add(1) else {
                continue;
            };
            let mut buffer = vec![0_u16; capacity as usize];
            // SAFETY: `buffer` includes room for the trailing NUL terminator.
            let copied = unsafe { DragQueryFileW(drop, index, Some(&mut buffer)) };
            if copied != length || copied == 0 {
                continue;
            }
            let path = PathBuf::from(OsString::from_wide(&buffer[..copied as usize]));
            paths.push(path.to_string_lossy().into_owned());
        }
    }
    // SAFETY: `medium` was obtained from `IDataObject::GetData` and is released exactly once;
    // `DragFinish` must not be called because it would double-free the `HGLOBAL`.
    unsafe { ReleaseStgMedium(&mut medium) };
    paths
}

/// Emits `"osFileDrop"` to a single live [`WindowBridge`] context, preferring the bridge bound to
/// `hwnd` and falling back to the primary bridge.
fn emit_file_drop(hwnd: HWND, paths: Vec<String>) {
    let target_raw = hwnd.0 as isize;
    let context = BRIDGES.with(|bridges| {
        let bridges = bridges.borrow();
        bridges
            .iter()
            .find(|entry| entry.hwnd == Some(target_raw))
            .or_else(|| bridges.first())
            .map(|entry| entry.context.clone())
    });
    if let Some(context) = context {
        context.emit("osFileDrop", serde_json::json!({ "paths": paths }));
    }
}

/// Returns whether page-scoped plain shortcuts are currently active for `hwnd`'s bridge.
fn page_keys_mode_for(hwnd: HWND) -> PageKeysMode {
    let target_raw = hwnd.0 as isize;
    let active = BRIDGES.with(|bridges| {
        let bridges = bridges.borrow();
        bridges
            .iter()
            .find(|entry| entry.hwnd == Some(target_raw))
            .or_else(|| bridges.first())
            .is_some_and(|entry| entry.page_keys_active)
    });
    if active {
        PageKeysMode::Active
    } else {
        PageKeysMode::Inactive
    }
}

/// Emits `"osShortcut"` with `action` to the [`WindowBridge`] bound to `hwnd` (falling back to the
/// primary bridge). Returns whether a live bridge received the event.
fn emit_os_shortcut(hwnd: HWND, action: &str) -> bool {
    let target_raw = hwnd.0 as isize;
    let context = BRIDGES.with(|bridges| {
        let bridges = bridges.borrow();
        bridges
            .iter()
            .find(|entry| entry.hwnd == Some(target_raw))
            .or_else(|| bridges.first())
            .map(|entry| entry.context.clone())
    });
    if let Some(context) = context {
        context.emit("osShortcut", serde_json::json!({ "action": action }));
        true
    } else {
        false
    }
}

/// Reads the currently pressed modifier keys for the UI thread's message queue.
fn query_modifiers() -> KeyModifiers {
    // SAFETY: plain queries of this thread's key state as of the current message.
    let is_down = |vk: i32| unsafe { GetKeyState(vk) } < 0;
    KeyModifiers {
        ctrl: is_down(i32::from(VK_CONTROL.0)),
        shift: is_down(i32::from(VK_SHIFT.0)),
        alt: is_down(i32::from(VK_MENU.0)),
        meta: is_down(i32::from(VK_LWIN.0)) || is_down(i32::from(VK_RWIN.0)),
    }
}

/// Decides how the window subclass handles a virtual key press with `mods`.
fn resolve_os_shortcut(
    vk: u16,
    mods: KeyModifiers,
    page_keys: PageKeysMode,
    press: KeyPressKind,
) -> ShortcutDecision {
    if mods.meta {
        return ShortcutDecision::PassThrough;
    }
    let plain = !mods.ctrl && !mods.shift && !mods.alt;
    let ctrl_only = mods.ctrl && !mods.shift && !mods.alt;
    let ctrl_shift = mods.ctrl && mods.shift && !mods.alt;
    let alt_only = mods.alt && !mods.ctrl && !mods.shift;
    let (action, repeats) = match vk {
        VK_KEY_F11 if plain => ("toggleFullScreen", false),
        VK_KEY_Q if ctrl_only => ("quit", false),
        VK_KEY_ADD if ctrl_only => ("zoomIn", true),
        VK_KEY_SUBTRACT if ctrl_only => ("zoomOut", true),
        VK_KEY_ADD if ctrl_shift => ("rotateClockwise", true),
        VK_KEY_SUBTRACT if ctrl_shift => ("rotateAnticlockwise", true),
        VK_KEY_ADD if alt_only => ("autoScrollFaster", true),
        VK_KEY_SUBTRACT if alt_only => ("autoScrollSlower", true),
        VK_KEY_ADD if plain && page_keys == PageKeysMode::Active => ("pageZoomIn", true),
        VK_KEY_SUBTRACT if plain && page_keys == PageKeysMode::Active => ("pageZoomOut", true),
        _ => return ShortcutDecision::PassThrough,
    };
    if press == KeyPressKind::Repeat && !repeats {
        ShortcutDecision::ConsumeOnly
    } else {
        ShortcutDecision::Emit(action)
    }
}

/// Removes any queued `WM_CHAR` / `WM_SYSCHAR` generated by `TranslateMessage` for a consumed key
/// on `hwnd`, preventing stray characters or system menu beeps.
fn drop_queued_char(hwnd: HWND) {
    let mut queued = MSG::default();
    for kind in [WM_CHAR, WM_SYSCHAR] {
        // SAFETY: draining character messages already queued for this thread's live window.
        unsafe {
            let _ = PeekMessageW(&mut queued, Some(hwnd), kind, kind, PM_REMOVE);
        }
    }
}

/// `EnumThreadWindows` callback that installs the shortcut subclass once per visible top-level HWND.
unsafe extern "system" fn install_shortcut_subclass_proc(hwnd: HWND, _lparam: LPARAM) -> BOOL {
    // SAFETY: plain Win32 query on a live top-level window enumerated on this thread.
    if !unsafe { IsWindowVisible(hwnd).as_bool() } {
        return BOOL(1);
    }
    let raw = hwnd.0 as isize;
    let already_subclassed = SUBCLASSED_WINDOWS.with(|set| set.borrow().contains(&raw));
    if already_subclassed {
        return BOOL(1);
    }
    // SAFETY: `hwnd` is a live top-level window on this UI thread, `shortcut_subclass_proc` has
    // static lifetime, and `WM_NCDESTROY` removes the subclass before the HWND is freed.
    let installed = unsafe {
        SetWindowSubclass(hwnd, Some(shortcut_subclass_proc), SHORTCUT_SUBCLASS_ID, 0).as_bool()
    };
    if installed {
        SUBCLASSED_WINDOWS.with(|set| {
            set.borrow_mut().insert(raw);
        });
    } else {
        log::warn!("window bridge: SetWindowSubclass failed for HWND {raw:#x}");
    }
    BOOL(1)
}

/// Top-level window subclass that intercepts `F11`, `Ctrl+Q`, and numpad `VK_ADD`/`VK_SUBTRACT`
/// before NSH's window procedure drops or misroutes them, and cleans itself up on `WM_NCDESTROY`.
unsafe extern "system" fn shortcut_subclass_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    id: usize,
    _ref_data: usize,
) -> LRESULT {
    match msg {
        WM_NCDESTROY => {
            let raw = hwnd.0 as isize;
            SUBCLASSED_WINDOWS.with(|set| {
                set.borrow_mut().remove(&raw);
            });
            BRIDGES.with(|bridges| {
                for entry in bridges.borrow_mut().iter_mut() {
                    if entry.hwnd == Some(raw) {
                        entry.hwnd = None;
                    }
                }
            });
            // SAFETY: removing our own subclass id before forwarding final destruction.
            unsafe {
                let _ = RemoveWindowSubclass(hwnd, Some(shortcut_subclass_proc), id);
                return DefSubclassProc(hwnd, msg, wparam, lparam);
            }
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            let vk = (wparam.0 & 0xFFFF) as u16;
            let press = if (lparam.0 & KEY_REPEAT_BIT) != 0 {
                KeyPressKind::Repeat
            } else {
                KeyPressKind::Initial
            };
            let decision =
                resolve_os_shortcut(vk, query_modifiers(), page_keys_mode_for(hwnd), press);
            match decision {
                ShortcutDecision::PassThrough => {}
                ShortcutDecision::ConsumeOnly => {
                    drop_queued_char(hwnd);
                    return LRESULT(0);
                }
                ShortcutDecision::Emit(action) => {
                    if emit_os_shortcut(hwnd, action) {
                        drop_queued_char(hwnd);
                        return LRESULT(0);
                    }
                }
            }
        }
        _ => {}
    }
    // SAFETY: forwarding unhandled messages to the next procedure in the subclass chain.
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

/// COM `IDropTarget` implementation bound to a single visible top-level HWND.
#[implement(IDropTarget)]
struct FileDropTarget {
    hwnd: HWND,
    offered: Cell<bool>,
}

impl IDropTarget_Impl for FileDropTarget_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        // SAFETY: `QueryGetData` checks `CF_HDROP` availability without reading file data.
        let offered = data
            .as_ref()
            .is_some_and(|data| unsafe { data.QueryGetData(&hdrop_format()) }.is_ok());
        self.offered.set(offered);
        set_drop_effect(effect, offered);
        Ok(())
    }

    fn DragOver(
        &self,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        set_drop_effect(effect, self.offered.get());
        Ok(())
    }

    fn DragLeave(&self) -> Result<()> {
        self.offered.set(false);
        Ok(())
    }

    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _keys: MODIFIERKEYS_FLAGS,
        _point: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> Result<()> {
        let paths = data.as_ref().map(extract_drop_paths).unwrap_or_default();
        self.offered.set(false);
        if !paths.is_empty() {
            write_drop_effect(effect, DROPEFFECT_COPY);
            emit_file_drop(self.hwnd, paths);
        } else {
            write_drop_effect(effect, DROPEFFECT_NONE);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, shift: bool, alt: bool) -> KeyModifiers {
        KeyModifiers {
            ctrl,
            shift,
            alt,
            meta: false,
        }
    }

    #[test]
    fn f11_and_ctrl_q_emit_once_and_consume_repeats() {
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_F11,
                mods(false, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("toggleFullScreen")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_F11,
                mods(false, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Repeat,
            ),
            ShortcutDecision::ConsumeOnly
        );
        // Shift+F11 stays free for Presentation mode in NSH's keymap.
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_F11,
                mods(false, true, false),
                PageKeysMode::Active,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::PassThrough
        );

        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_Q,
                mods(true, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("quit")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_Q,
                mods(true, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Repeat,
            ),
            ShortcutDecision::ConsumeOnly
        );
        // Ctrl+Shift+Q (Quick Note), Ctrl+Alt+Q (AI Panel / AltGr), and plain Q pass through.
        for other in [
            mods(true, true, false),
            mods(true, false, true),
            mods(false, false, false),
        ] {
            assert_eq!(
                resolve_os_shortcut(VK_KEY_Q, other, PageKeysMode::Active, KeyPressKind::Initial),
                ShortcutDecision::PassThrough,
                "{other:?}"
            );
        }
    }

    #[test]
    fn numpad_add_and_subtract_route_by_modifiers_and_page_mode() {
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_ADD,
                mods(true, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Repeat,
            ),
            ShortcutDecision::Emit("zoomIn")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_SUBTRACT,
                mods(true, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Repeat,
            ),
            ShortcutDecision::Emit("zoomOut")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_ADD,
                mods(true, true, false),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("rotateClockwise")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_SUBTRACT,
                mods(true, true, false),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("rotateAnticlockwise")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_ADD,
                mods(false, false, true),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("autoScrollFaster")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_SUBTRACT,
                mods(false, false, true),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("autoScrollSlower")
        );
        // Plain numpad +/- zoom only while page keys are active; in inputs they pass through.
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_ADD,
                mods(false, false, false),
                PageKeysMode::Active,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("pageZoomIn")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_SUBTRACT,
                mods(false, false, false),
                PageKeysMode::Active,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::Emit("pageZoomOut")
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_ADD,
                mods(false, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::PassThrough
        );
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_SUBTRACT,
                mods(false, false, false),
                PageKeysMode::Inactive,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::PassThrough
        );
    }

    #[test]
    fn windows_key_chords_always_pass_through() {
        let with_meta = KeyModifiers {
            ctrl: false,
            shift: false,
            alt: false,
            meta: true,
        };
        assert_eq!(
            resolve_os_shortcut(
                VK_KEY_F11,
                with_meta,
                PageKeysMode::Active,
                KeyPressKind::Initial,
            ),
            ShortcutDecision::PassThrough
        );
    }
}
