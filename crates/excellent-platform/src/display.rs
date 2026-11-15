//! The display preferences NSH's environment doesn't carry (plan U.4). NSH already follows the system's reduced
//! motion, text size and reading direction; high contrast is read here.

/// Whether the system's high-contrast mode is on. `false` where the system has none or it can't be read.
pub fn high_contrast() -> bool {
    imp::high_contrast()
}

#[cfg(windows)]
mod imp {
    use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
    use windows::Win32::UI::WindowsAndMessaging::{
        SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SystemParametersInfoW,
    };

    pub(super) fn high_contrast() -> bool {
        let mut setting = HIGHCONTRASTW {
            cbSize: size_of::<HIGHCONTRASTW>() as u32,
            ..Default::default()
        };
        // SAFETY: `setting` is a HIGHCONTRASTW with its size set, as SPI_GETHIGHCONTRAST takes; it outlives the call.
        let read = unsafe {
            SystemParametersInfoW(
                SPI_GETHIGHCONTRAST,
                setting.cbSize,
                Some((&raw mut setting).cast()),
                SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
            )
        };
        read.is_ok() && setting.dwFlags.contains(HCF_HIGHCONTRASTON)
    }
}

#[cfg(not(windows))]
mod imp {
    // shortcut: macOS (increase contrast) and Linux (GTK high-contrast theme) come with Phase 2.
    pub(super) fn high_contrast() -> bool {
        false
    }
}
