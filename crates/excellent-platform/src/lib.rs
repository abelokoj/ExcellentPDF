//! Platform services (plan K.19, §5.4): what the app asks of the operating system.
//!
//! Built so far: [`instance`] (single instance) and [`associations`] (file types), for R.11; [`print`] (the print
//! dialog and the printer), for O.22; and [`display`] (the system's high contrast), for U.4. The rest of K.19's
//! services (clipboard, speech, key store, notifications, memory pressure) come with the features that need them.

pub mod associations;
pub mod display;
pub mod instance;
#[cfg(any(windows, target_os = "macos", target_os = "linux"))]
pub mod print;
