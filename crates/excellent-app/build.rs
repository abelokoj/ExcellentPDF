//! Embeds the app icon (`packaging/windows/excellentpdf.ico`, drawn by `design/icon/render_icon.py`) in the Windows
//! executable as icon group 1, which Explorer, shortcuts and NSH's window class (`LoadIconW(.., 1)`) all use.
//!
//! The resource file is written here rather than by `rc.exe`, which isn't on the path of a plain cargo build: a `.res`
//! is a short list of headed records, and the MSVC linker takes it as an input like an object file.

use std::path::{Path, PathBuf};
use std::{env, fs};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
/// MOVEABLE | PURE | DISCARDABLE, as `rc.exe` marks icons.
const ICON_FLAGS: u16 = 0x1030;

fn main() {
    let icon = Path::new(&env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
        .join("../../packaging/windows/excellentpdf.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let windows_msvc = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !windows_msvc {
        return;
    }
    let ico = fs::read(&icon).unwrap_or_else(|e| panic!("reading {}: {e}", icon.display()));
    let res =
        PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")).join("excellentpdf.res");
    fs::write(&res, resource_file(&ico)).expect("writing the icon resource");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

/// A `.res` holding each image of `ico` as an `RT_ICON` (ids 1..) and the group that lists them as `RT_GROUP_ICON` 1.
fn resource_file(ico: &[u8]) -> Vec<u8> {
    let word = |at: usize| u16::from_le_bytes([ico[at], ico[at + 1]]);
    let dword = |at: usize| u32::from_le_bytes(ico[at..at + 4].try_into().expect("four bytes"));
    assert!(
        ico.len() >= 6 && word(0) == 0 && word(2) == 1,
        "not an .ico file"
    );
    let count = word(4);

    // The empty record every .res starts with.
    let mut out = Vec::new();
    record(&mut out, 0, 0, 0, &[]);
    let mut group = Vec::new();
    group.extend_from_slice(&[0, 0, 1, 0]);
    group.extend_from_slice(&count.to_le_bytes());
    for i in 0..count {
        let entry = 6 + 16 * usize::from(i);
        let (size, offset) = (dword(entry + 8) as usize, dword(entry + 12) as usize);
        let id = i + 1;
        record(
            &mut out,
            RT_ICON,
            id,
            ICON_FLAGS,
            &ico[offset..offset + size],
        );
        // The directory entry as the file has it, its last field the resource id instead of the file offset.
        group.extend_from_slice(&ico[entry..entry + 12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    record(&mut out, RT_GROUP_ICON, 1, ICON_FLAGS, &group);
    out
}

/// Appends one resource record with numeric type and name, language neutral, its data padded to four bytes.
fn record(out: &mut Vec<u8>, kind: u16, name: u16, flags: u16, data: &[u8]) {
    let ordinal = |v: u16| [0xff, 0xff, v.to_le_bytes()[0], v.to_le_bytes()[1]];
    let size = u32::try_from(data.len()).expect("icon image under 4 GB");
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&32u32.to_le_bytes()); // header size
    out.extend_from_slice(&ordinal(kind));
    out.extend_from_slice(&ordinal(name));
    out.extend_from_slice(&0u32.to_le_bytes()); // data version
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // language
    out.extend_from_slice(&0u32.to_le_bytes()); // version
    out.extend_from_slice(&0u32.to_le_bytes()); // characteristics
    out.extend_from_slice(data);
    out.resize(out.len().next_multiple_of(4), 0);
}
