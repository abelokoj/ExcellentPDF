//! MuPDF's built-in fonts, for documents that don't embed theirs.
//!
//! SumatraPDF's `noto_sumatra.c` asks a loader for each font by file name. Like upstream SumatraPDF
//! (`cmd/pack-embedded-prebuild.cmd`), we ship the base-14 fonts (URW) and Charis SIL in the binary, about 800 KB;
//! every other font in MuPDF's table (Noto, CJK) is left to a download pack (D8), so the loader returns nothing for
//! it and MuPDF falls back.

use core::ffi::{CStr, c_char, c_int, c_uchar};
use core::ptr;

macro_rules! fonts {
    ($($dir:literal / $file:literal),* $(,)?) => {
        [$((
            $file,
            include_bytes!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../third_party/mupdf/resources/fonts/",
                $dir,
                "/",
                $file
            ))
            .as_slice(),
        )),*]
    };
}

// shortcut: stored uncompressed (upstream compresses its archive); compress them if the 15 MB budget gets tight.
static FONTS: [(&str, &[u8]); 18] = fonts![
    "urw" / "Dingbats.cff",
    "urw" / "NimbusMonoPS-Regular.cff",
    "urw" / "NimbusMonoPS-Italic.cff",
    "urw" / "NimbusMonoPS-Bold.cff",
    "urw" / "NimbusMonoPS-BoldItalic.cff",
    "urw" / "NimbusRoman-Regular.cff",
    "urw" / "NimbusRoman-Italic.cff",
    "urw" / "NimbusRoman-Bold.cff",
    "urw" / "NimbusRoman-BoldItalic.cff",
    "urw" / "NimbusSans-Regular.cff",
    "urw" / "NimbusSans-Italic.cff",
    "urw" / "NimbusSans-Bold.cff",
    "urw" / "NimbusSans-BoldItalic.cff",
    "urw" / "StandardSymbolsPS.cff",
    "sil" / "CharisSIL.cff",
    "sil" / "CharisSIL-Italic.cff",
    "sil" / "CharisSIL-Bold.cff",
    "sil" / "CharisSIL-BoldItalic.cff",
];

/// The bytes of a built-in font by file name.
fn find(file_name: &[u8]) -> Option<&'static [u8]> {
    FONTS
        .iter()
        .find(|(name, _)| name.as_bytes() == file_name)
        .map(|(_, data)| *data)
}

/// The loader the shim hands to MuPDF (`ex_font_loader`): the font's bytes, valid for the life of the process, or
/// NULL.
///
/// # Safety
///
/// `file_name` is a NUL-terminated string and `size` a valid pointer, as MuPDF guarantees.
pub(crate) unsafe extern "C" fn load(file_name: *const c_char, size: *mut c_int) -> *const c_uchar {
    // SAFETY: MuPDF passes a NUL-terminated file name.
    let name = unsafe { CStr::from_ptr(file_name) };
    let Some(data) = find(name.to_bytes()) else {
        return ptr::null();
    };
    let len = c_int::try_from(data.len()).expect("built-in fonts are far smaller than 2 GB");
    // SAFETY: MuPDF passes a valid out-pointer.
    unsafe { size.write(len) };
    data.as_ptr()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_base14_and_nothing_else() {
        let times = find(b"NimbusRoman-Regular.cff").expect("Times is built in");
        assert!(times.len() > 10_000);
        assert!(find(b"NotoSerif-Regular.otf").is_none());
    }
}
