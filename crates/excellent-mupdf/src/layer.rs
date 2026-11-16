//! A PDF's layers (optional content groups, A.22): read here, changed by [`crate::Edit::set_layer`] and by writing an
//! annotation's [`crate::AnnotData::layer`].

use core::ptr;

use excellent_mupdf_sys::ex_layers;

use crate::context::{check, with_context};
use crate::document::Document;
use crate::error::Result;

/// A layer as the file's default configuration (`/OCProperties /D`) has it: what other readers open with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Layer {
    /// Its name (`/Name`), which names it here too.
    pub name: String,
    /// Whether its content is drawn.
    pub visible: bool,
    /// Whether readers keep it from being shown or hidden (`/Locked`).
    pub locked: bool,
}

impl Document {
    /// Its layers in the file's order; none for a document without any or one that isn't a PDF.
    ///
    /// # Errors
    ///
    /// MuPDF can't read them.
    pub fn layers(&self) -> Result<Vec<Layer>> {
        with_context(|ctx| {
            let mut buf = ptr::null_mut();
            // SAFETY: the document is live and used by this thread only; `buf` receives a new buffer.
            check(|err| unsafe { ex_layers(ctx, self.raw().as_ptr(), &mut buf, err) })?;
            // SAFETY: `buf` is a new buffer we own.
            let bytes = unsafe { crate::buffer::into_vec(ctx, buf) };
            Ok(parse(&bytes))
        })
    }
}

/// The layers the shim listed: per layer a flags byte (`'0'` plus 1 hidden, plus 2 locked) then its name, then a NUL.
fn parse(bytes: &[u8]) -> Vec<Layer> {
    bytes
        .split(|&byte| byte == 0)
        .filter_map(|entry| {
            let (&flags, name) = entry.split_first()?;
            let flags = flags.checked_sub(b'0')?;
            Some(Layer {
                name: String::from_utf8_lossy(name).into_owned(),
                visible: flags & 1 == 0,
                locked: flags & 2 != 0,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shim_s_list_reads_as_layers() {
        let layers = parse(b"0Ada\x003Review notes\x002\x00");
        assert_eq!(
            layers,
            [
                Layer {
                    name: "Ada".into(),
                    visible: true,
                    locked: false
                },
                Layer {
                    name: "Review notes".into(),
                    visible: false,
                    locked: true
                },
                Layer {
                    name: String::new(),
                    visible: true,
                    locked: true
                },
            ]
        );
        assert!(parse(b"").is_empty());
    }
}
