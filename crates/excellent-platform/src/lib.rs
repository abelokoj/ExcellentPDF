//! Platform services (plan K.19, §5.4): what the app asks of the operating system.
//!
//! Built so far, for R.11: [`instance`] (single instance) and [`associations`] (file types). The rest of K.19's
//! services (dialogs, print, clipboard, speech, key store, notifications, memory pressure) come with the features that
//! need them.

pub mod associations;
pub mod instance;
