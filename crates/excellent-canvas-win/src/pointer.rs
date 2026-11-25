//! Pen, touch and mouse samples from `WM_POINTER*` messages (plan §7.3), ported from SumatraPDF Enhanced's
//! `src/PointerInput.h`: the same sample, read with `GetPointerPenInfo` and, for the samples Windows coalesced into one
//! message, `GetPointerPenInfoHistory`.

use windows::Win32::UI::Input::Pointer::{
    GetPointerInfo, GetPointerPenInfo, GetPointerPenInfoHistory, GetPointerTouchInfo,
    GetPointerType, POINTER_FLAG_INCONTACT, POINTER_INFO, POINTER_PEN_INFO, POINTER_TOUCH_INFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PEN_FLAG_BARREL, PEN_FLAG_ERASER, PEN_FLAG_INVERTED, PEN_MASK_PRESSURE, PEN_MASK_ROTATION,
    PEN_MASK_TILT_X, PEN_MASK_TILT_Y, POINTER_INPUT_TYPE, PT_MOUSE, PT_PEN, PT_TOUCH,
    TOUCH_MASK_CONTACTAREA,
};

/// The highest pressure Windows reports.
const PRESSURE_MAX: u32 = 1024;
/// The highest rotation, in degrees.
const ROTATION_MAX: u32 = 359;
/// The steepest tilt either way, in degrees.
const TILT_MAX: i32 = 90;
/// The pointer buttons sit above this bit of the pointer flags.
const BUTTON_SHIFT: u32 = 4;
/// The five pointer buttons.
const BUTTON_MASK: u32 = 0x1f;
/// Coalesced samples read at most per message (Enhanced's figure).
const HISTORY_CAPACITY: usize = 128;

/// What made a sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Device {
    Unknown,
    Mouse,
    Touch,
    Pen,
}

/// One pointer sample. A pen reports only some of pressure, tilt and rotation; the rest are `None`.
#[derive(Debug, Clone, Copy, PartialEq)]
// shortcut: rotation, buttons and the barrel are captured for H.2 but nothing reads them yet.
pub(crate) struct Sample {
    pub(crate) device: Device,
    pub(crate) id: u32,
    /// Where, in screen pixels.
    pub(crate) x: i32,
    pub(crate) y: i32,
    /// The message time, in milliseconds.
    pub(crate) time: u32,
    /// The performance counter when the input happened (0 if Windows didn't say).
    pub(crate) performance_count: u64,
    /// 0 to 1.
    pub(crate) pressure: Option<f32>,
    /// Degrees, -90 to 90.
    pub(crate) tilt_x: Option<i32>,
    pub(crate) tilt_y: Option<i32>,
    /// Degrees clockwise, 0 to 359.
    pub(crate) rotation: Option<u32>,
    /// Pointer buttons: bit 0 the first, up to bit 4 the fifth.
    pub(crate) buttons: u32,
    /// Touching the screen.
    pub(crate) contact: bool,
    /// The pen's eraser end, or the pen turned over.
    pub(crate) eraser: bool,
    /// The barrel button is held.
    pub(crate) barrel: bool,
}

/// A sample from Windows' pointer info and, for a pen, its pen info.
pub(crate) fn normalize(info: &POINTER_INFO, pen: Option<&POINTER_PEN_INFO>) -> Sample {
    let device = device(info.pointerType);
    let mut sample = Sample {
        device,
        id: info.pointerId,
        x: info.ptPixelLocation.x,
        y: info.ptPixelLocation.y,
        time: info.dwTime,
        performance_count: info.PerformanceCount,
        pressure: None,
        tilt_x: None,
        tilt_y: None,
        rotation: None,
        buttons: (info.pointerFlags.0 >> BUTTON_SHIFT) & BUTTON_MASK,
        contact: info.pointerFlags.contains(POINTER_FLAG_INCONTACT),
        eraser: false,
        barrel: false,
    };
    let Some(pen) = pen.filter(|_| device == Device::Pen) else {
        return sample;
    };
    let has = |mask: u32| pen.penMask & mask != 0;
    sample.eraser = pen.penFlags & (PEN_FLAG_INVERTED | PEN_FLAG_ERASER) != 0;
    sample.barrel = pen.penFlags & PEN_FLAG_BARREL != 0;
    sample.pressure =
        has(PEN_MASK_PRESSURE).then(|| pen.pressure.min(PRESSURE_MAX) as f32 / PRESSURE_MAX as f32);
    sample.rotation = has(PEN_MASK_ROTATION).then(|| pen.rotation.min(ROTATION_MAX));
    sample.tilt_x = has(PEN_MASK_TILT_X).then(|| pen.tiltX.clamp(-TILT_MAX, TILT_MAX));
    sample.tilt_y = has(PEN_MASK_TILT_Y).then(|| pen.tiltY.clamp(-TILT_MAX, TILT_MAX));
    sample
}

fn device(kind: POINTER_INPUT_TYPE) -> Device {
    match kind {
        PT_PEN => Device::Pen,
        PT_TOUCH => Device::Touch,
        PT_MOUSE => Device::Mouse,
        _ => Device::Unknown,
    }
}

/// The pointer id in a `WM_POINTER*` message's `wParam`.
pub(crate) fn id(wparam: usize) -> u32 {
    (wparam & 0xffff) as u32
}

/// The newest sample of pointer `id`, or `None` if Windows no longer knows it. (Enhanced returns a sample with only
/// the device then; nothing here could use that.)
pub(crate) fn read(id: u32) -> Option<Sample> {
    let mut kind = POINTER_INPUT_TYPE::default();
    // SAFETY: plain query; `kind` lives for the call.
    unsafe { GetPointerType(id, &mut kind) }.ok()?;
    if kind == PT_PEN {
        let mut pen = POINTER_PEN_INFO::default();
        // SAFETY: plain query; `pen` lives for the call.
        if unsafe { GetPointerPenInfo(id, &mut pen) }.is_ok() {
            return Some(normalize(&pen.pointerInfo, Some(&pen)));
        }
    }
    let mut info = POINTER_INFO::default();
    // SAFETY: plain query; `info` lives for the call.
    unsafe { GetPointerInfo(id, &mut info) }.ok()?;
    Some(normalize(&info, None))
}

/// How wide touch `id`'s contact is, in screen pixels (the wider of its sides); `None` when it isn't a touch or the
/// digitizer doesn't say (H.8: a palm is wider than a fingertip).
pub(crate) fn touch_width(id: u32) -> Option<i32> {
    let mut touch = POINTER_TOUCH_INFO::default();
    // SAFETY: plain query; `touch` lives for the call.
    unsafe { GetPointerTouchInfo(id, &mut touch) }.ok()?;
    if touch.touchMask & TOUCH_MASK_CONTACTAREA == 0 {
        return None;
    }
    let contact = touch.rcContact;
    Some((contact.right - contact.left).max(contact.bottom - contact.top))
}

/// Every sample pen `id` made since the last message, oldest first (the newest is the message's own). Empty if the
/// pointer isn't a pen.
pub(crate) fn pen_history(id: u32) -> Vec<Sample> {
    let mut history = vec![POINTER_PEN_INFO::default(); HISTORY_CAPACITY];
    let mut count = HISTORY_CAPACITY as u32;
    // SAFETY: `history` holds `count` entries and lives for the call; Windows writes at most `count`.
    if unsafe { GetPointerPenInfoHistory(id, &mut count, Some(history.as_mut_ptr())) }.is_err() {
        return Vec::new();
    }
    history.truncate((count as usize).min(HISTORY_CAPACITY));
    chronological(&history)
}

/// Windows lists history newest first; strokes are drawn oldest first.
fn chronological(history: &[POINTER_PEN_INFO]) -> Vec<Sample> {
    history
        .iter()
        .rev()
        .map(|pen| normalize(&pen.pointerInfo, Some(pen)))
        .collect()
}

#[cfg(test)]
mod tests {
    use windows::Win32::Foundation::POINT;
    use windows::Win32::UI::Input::Pointer::POINTER_FLAGS;

    use super::*;

    /// Enhanced's `PointerInputTests`, case by case (its mouse case has no counterpart: mouse ink isn't read here).
    fn pen() -> POINTER_PEN_INFO {
        let mut pen = POINTER_PEN_INFO::default();
        pen.pointerInfo.pointerType = PT_PEN;
        pen.pointerInfo.pointerId = 7;
        pen.pointerInfo.pointerFlags =
            POINTER_FLAGS(POINTER_FLAG_INCONTACT.0 | ((0x1 | 0x2) << BUTTON_SHIFT));
        pen.pointerInfo.dwTime = 100;
        pen.pointerInfo.PerformanceCount = 123_456;
        pen.pointerInfo.ptPixelLocation = POINT { x: -10, y: 20 };
        pen.penMask = PEN_MASK_PRESSURE | PEN_MASK_ROTATION | PEN_MASK_TILT_X | PEN_MASK_TILT_Y;
        pen.penFlags = PEN_FLAG_BARREL | PEN_FLAG_INVERTED | PEN_FLAG_ERASER;
        pen.pressure = 2048;
        pen.tiltX = -100;
        pen.tiltY = 100;
        pen.rotation = 400;
        pen
    }

    #[test]
    fn a_pen_sample_is_clamped_to_its_ranges() {
        let pen = pen();
        let s = normalize(&pen.pointerInfo, Some(&pen));
        assert_eq!(s.device, Device::Pen);
        assert_eq!((s.id, s.x, s.y, s.time), (7, -10, 20, 100));
        assert!(s.contact && s.eraser && s.barrel);
        assert_eq!(s.buttons, 0x3);
        assert_eq!(s.pressure, Some(1.0));
        assert_eq!((s.tilt_x, s.tilt_y), (Some(-TILT_MAX), Some(TILT_MAX)));
        assert_eq!(s.rotation, Some(ROTATION_MAX));
        assert_eq!(s.performance_count, 123_456);
    }

    #[test]
    fn values_the_pen_does_not_report_are_none() {
        let mut pen = pen();
        pen.penMask = 0;
        let s = normalize(&pen.pointerInfo, Some(&pen));
        assert_eq!(
            (s.pressure, s.tilt_x, s.tilt_y, s.rotation),
            (None, None, None, None)
        );

        pen.penMask = PEN_MASK_PRESSURE;
        pen.pressure = 512;
        assert_eq!(normalize(&pen.pointerInfo, Some(&pen)).pressure, Some(0.5));
        pen.pressure = 0;
        assert_eq!(normalize(&pen.pointerInfo, Some(&pen)).pressure, Some(0.0));
    }

    #[test]
    fn touch_ignores_pen_info() {
        let mut pen = pen();
        pen.pointerInfo.pointerType = PT_TOUCH;
        let s = normalize(&pen.pointerInfo, Some(&pen));
        assert_eq!(s.device, Device::Touch);
        assert!(s.pressure.is_none() && !s.eraser);
    }

    #[test]
    fn history_comes_back_oldest_first() {
        let mut history = [POINTER_PEN_INFO::default(); 2];
        history[0].pointerInfo.dwTime = 20;
        history[1].pointerInfo.dwTime = 10;
        let times: Vec<u32> = chronological(&history).iter().map(|s| s.time).collect();
        assert_eq!(times, [10, 20]);
    }

    #[test]
    fn the_pointer_id_is_the_low_word() {
        assert_eq!(id(0xabcd_0042), 0x42);
    }
}
