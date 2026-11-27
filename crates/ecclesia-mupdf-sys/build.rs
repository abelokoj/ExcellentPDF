//! Builds MuPDF 1.28.5 as upstream SumatraPDF vendors it, every library MuPDF uses, and our C shim (plan §6.1, K.2).
//!
//! All of MuPDF is built, on every target (`dev notes/dev_decisions.md`, DD1): PDF, XPS, EPUB, MOBI, FB2, CBZ and
//! other comic archives (libarchive with bzip2 and liblzma), SVG, Markdown, images including JPEG XL and WebP, OFD,
//! OCR (Tesseract and Leptonica), barcodes (zxing-cpp and zint), JavaScript (MuJS) and every `mutool` command.
//!
//! Source lists, defines and per-platform choices follow upstream SumatraPDF at the commit in `third_party/README.md`:
//! `premake5.lua` / `premake5.files.lua` for Windows, `cmd/helper/ng-targets.ts` for Linux and macOS. Upstream builds
//! neither Android nor iOS; they follow the Linux and macOS rules, with the differences below.
//!
//! | | Windows | Linux | macOS | Android | iOS |
//! | --- | --- | --- | --- | --- | --- |
//! | JPEG XR | WIC (`load-jxr-win.c`) | MuPDF's stub | MuPDF's stub | MuPDF's stub | MuPDF's stub |
//! | Signing in `pdfsign` / `murun` | CryptoAPI (`pkcs7-windows.c`) | stub | stub | stub | stub |
//! | Other PKCS #7 code | | OpenSSL (`pkcs7-openssl.c`) | Security framework (`pkcs7_mac.c`) | | |
//! | System fonts | font folder | fontconfig | Core Text | none yet | Core Text |
//! | Encrypted ZIP entries (libarchive) | CNG | OpenSSL | CommonCrypto | none | CommonCrypto |
//!
//! libjpeg-turbo's SIMD code is assembled with NASM on x86 and x86-64 (set `NASM` if it isn't on `PATH`). Without
//! NASM, or on ARM, libjpeg-turbo builds its C code; the build says so in a warning.
//!
//! Not built, as upstream: `cmapdump.c` (a MuPDF build tool), and `mutool.c` / `muraster.c`, which only hold `main()`;
//! every `mutool` command they dispatch to is in the library. MuPDF's `noto.c` is replaced by SumatraPDF's
//! `noto_sumatra.c`, which loads the built-in fonts through `ex_set_font_loader`.
//!
//! With the `regenerate-bindings` feature, `bindgen` rewrites `src/bindings.rs` from `csrc/shim.h` (needs libclang,
//! for example from Visual Studio's "C++ Clang tools" component; set `LIBCLANG_PATH` if it isn't found).

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const MUPDF_DEFINES: &[(&str, &str)] = &[
    ("USE_JPIP", "1"),
    ("OPJ_EXPORTS", "1"),
    ("OPJ_STATIC", "1"),
    ("HAVE_LCMS2MT", "1"),
    ("HAVE_WEBP", "1"),
    ("SHARE_JPEG", "1"),
    ("TOFU_CJK_LANG", "1"),
    ("FZ_ENABLE_PDF", "1"),
    ("FZ_ENABLE_SVG", "1"),
    ("FZ_ENABLE_BROTLI", "1"),
    ("FZ_ENABLE_BARCODE", "1"),
    ("FZ_ENABLE_JS", "1"),
    ("FZ_ENABLE_HYPHEN", "0"),
    ("FZ_ENABLE_MD", "1"),
    ("HAVE_LEPTONICA", "1"),
    ("HAVE_TESSERACT", "1"),
    ("HAVE_ZXINGCPP", "1"),
    ("HAVE_LIBARCHIVE", "1"),
    ("LIBARCHIVE_STATIC", "1"),
    // "NAN is not a constant" with some MSVC versions (SumatraPDF's comment).
    ("_UCRT_NOISY_NAN", "1"),
    ("CMARK_GFM_STATIC_DEFINE", "1"),
];

/// libjpeg-turbo's precision-independent core; the 8-bit wrappers in `src/wrapper` are added by name pattern.
const LIBJPEG_SRC: &[&str] = &[
    "jaricom.c",
    "jcapimin.c",
    "jcarith.c",
    "jchuff.c",
    "jcicc.c",
    "jcinit.c",
    "jclhuff.c",
    "jcmarker.c",
    "jcmaster.c",
    "jcomapi.c",
    "jcparam.c",
    "jcphuff.c",
    "jctrans.c",
    "jdapimin.c",
    "jdarith.c",
    "jdatadst.c",
    "jdatasrc.c",
    "jdhuff.c",
    "jdicc.c",
    "jdinput.c",
    "jdlhuff.c",
    "jdmarker.c",
    "jdmaster.c",
    "jdphuff.c",
    "jdtrans.c",
    "jerror.c",
    "jfdctflt.c",
    "jmemmgr.c",
    "jmemnobs.c",
    "jpeg_nbits.c",
];

/// liblzma's decoder half (libarchive never compresses), as upstream's `a-libarchive` project lists it.
const LIBLZMA_SRC: &[&str] = &[
    "common/alone_decoder.c",
    "common/auto_decoder.c",
    "common/block_decoder.c",
    "common/block_header_decoder.c",
    "common/block_util.c",
    "common/common.c",
    "common/filter_common.c",
    "common/filter_decoder.c",
    "common/filter_flags_decoder.c",
    "common/index.c",
    "common/index_decoder.c",
    "common/index_hash.c",
    "common/stream_decoder.c",
    "common/stream_flags_common.c",
    "common/stream_flags_decoder.c",
    "common/vli_decoder.c",
    "common/vli_size.c",
    "check/check.c",
    "check/crc32_fast.c",
    "check/crc64_fast.c",
    "lz/lz_decoder.c",
    "lzma/lzma_decoder.c",
    "lzma/lzma2_decoder.c",
    "rangecoder/price_table.c",
    "delta/delta_common.c",
    "delta/delta_decoder.c",
    "simple/simple_coder.c",
    "simple/simple_decoder.c",
    "simple/x86.c",
];

const LIBLZMA_INCLUDES: &[&str] = &[
    "api",
    "common",
    "check",
    "delta",
    "lz",
    "lzma",
    "rangecoder",
    "simple",
    "",
];

/// Tesseract's x86 kernels: each needs its instruction set enabled (GCC and Clang), and none builds on ARM.
/// Tesseract picks one at run time with `cpuid`.
const TESSERACT_X86_KERNELS: &[(&str, &str)] = &[
    ("dotproductavx.cpp", "-mavx"),
    ("intsimdmatrixavx2.cpp", "-mavx2"),
    ("dotproductfma.cpp", "-mfma"),
    ("dotproductsse.cpp", "-msse4.1"),
    ("intsimdmatrixsse.cpp", "-msse4.1"),
];

const TESSERACT_X86_DEFINES: &[&str] = &["HAVE_AVX", "HAVE_AVX2", "HAVE_SSE4_1", "HAVE_FMA"];

const TESSERACT_INCLUDES: &[&str] = &[
    "include",
    "src/api",
    "src/arch",
    "src/ccmain",
    "src/ccstruct",
    "src/ccutil",
    "src/classify",
    "src/dict",
    "src/lstm",
    "src/textord",
    "src/viewer",
    "src/wordrec",
    "src/cutil",
];

const ZXING_DIRS: &[&str] = &[
    "",
    "aztec",
    "datamatrix",
    "maxicode",
    "oned",
    "pdf417",
    "qrcode",
];

const ZXING_DEFINES: &[&str] = &[
    "ZXING_HAS_CHAR8",
    "ZXING_USE_ZINT",
    "ZXING_EXPERIMENTAL_API",
    "ZINT_NO_PNG",
    "HAVE_ZXINGCPP",
];

/// The operating system the library is built for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Os {
    Windows,
    Linux,
    Android,
    Mac,
    Ios,
    /// Any other target: built like Linux without its system libraries (no fontconfig, no OpenSSL). Untested.
    OtherPosix,
}

/// The processor architecture, as far as the build cares.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Arch {
    X86,
    X86_64,
    Aarch64,
    Other,
}

/// What the build is compiling for (cargo's `CARGO_CFG_TARGET_*`, not the host).
#[derive(Clone, Copy, Debug)]
struct Target {
    os: Os,
    arch: Arch,
    /// The MSVC toolchain (cl.exe), rather than GCC or Clang.
    msvc: bool,
}

impl Target {
    fn from_env() -> Self {
        let var = |name: &str| env::var(name).unwrap_or_else(|_| panic!("cargo sets {name}"));
        let os = match var("CARGO_CFG_TARGET_OS").as_str() {
            "windows" => Os::Windows,
            "linux" => Os::Linux,
            "android" => Os::Android,
            "macos" => Os::Mac,
            "ios" => Os::Ios,
            _ => Os::OtherPosix,
        };
        let arch = match var("CARGO_CFG_TARGET_ARCH").as_str() {
            "x86" => Arch::X86,
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            _ => Arch::Other,
        };
        Self {
            os,
            arch,
            msvc: var("CARGO_CFG_TARGET_ENV") == "msvc",
        }
    }

    fn is_apple(self) -> bool {
        matches!(self.os, Os::Mac | Os::Ios)
    }

    fn is_x86(self) -> bool {
        matches!(self.arch, Arch::X86 | Arch::X86_64)
    }
}

/// Paths of the vendored trees, and the target they are built for.
struct Tree {
    mupdf: PathBuf,
    libs: PathBuf,
    /// SumatraPDF's own MuPDF files (`src/shared/mupdf`).
    sumatra: PathBuf,
    /// SumatraPDF's non-Windows files (`src/ng`).
    sumatra_ng: PathBuf,
    csrc: PathBuf,
    out: PathBuf,
    target: Target,
}

impl Tree {
    fn lib(&self, name: &str) -> PathBuf {
        self.libs.join(name)
    }
}

/// A static library of third-party C code: always optimized, as SumatraPDF does, with its warnings off.
fn library(target: Target) -> cc::Build {
    let mut build = cc::Build::new();
    // `-w` silences the warnings; cargo still shows the compiler's errors, so a failing source says why.
    build.opt_level(2).warnings(false);
    build.define("_CRT_SECURE_NO_WARNINGS", None);
    if target.msvc {
        // Some sources (zxing-cpp's u8 literals, Tesseract) are UTF-8; cl.exe would read them in the ANSI code page.
        build.flag("/utf-8");
    }
    build
}

/// A static library of third-party C++ code (exceptions and RTTI on, the compilers' default).
fn cpp_library(target: Target, std: &str) -> cc::Build {
    let mut build = library(target);
    build.cpp(true).std(std);
    build
}

/// The files in `dir` whose names end with `suffix`, sorted, without those named in `exclude`.
fn sources(dir: &Path, suffix: &str, exclude: &[&str]) -> Vec<PathBuf> {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot list {}: {e}", dir.display()));
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| {
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            name.ends_with(suffix) && !exclude.contains(&name)
        })
        .collect();
    files.sort();
    files
}

fn main() {
    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"));
    let root = manifest_dir.join("../../third_party");
    let tree = Tree {
        mupdf: root.join("mupdf"),
        libs: root.join("amalgamations"),
        sumatra: root.join("sumatra/shared/mupdf"),
        sumatra_ng: root.join("sumatra/ng"),
        csrc: manifest_dir.join("csrc"),
        out: PathBuf::from(env::var("OUT_DIR").expect("cargo sets OUT_DIR")),
        target: Target::from_env(),
    };

    // The libraries are independent, and several are one large file each, so they build side by side.
    let tree = &tree;
    std::thread::scope(|s| {
        s.spawn(|| build_tesseract(tree));
        s.spawn(|| build_zxing(tree));
        s.spawn(|| build_harfbuzz(tree));
        s.spawn(|| build_mupdf(tree));
        s.spawn(|| build_leptonica(tree));
        s.spawn(|| build_libarchive(tree));
        s.spawn(|| build_freetype(tree));
        s.spawn(|| build_libjpeg(tree));
        s.spawn(|| build_cmark(tree));
        s.spawn(|| build_extract(tree));
        // zlib's gz* functions call read, write and lseek, which unistd.h declares; SumatraPDF builds only for Windows,
        // where io.h does. Clang treats an undeclared call as an error.
        let zlib: &[(&str, &str)] = if tree.target.os == Os::Windows {
            &[]
        } else {
            &[("HAVE_UNISTD_H", "1")]
        };
        s.spawn(move || build_simple(tree, "a-zlib", "zlib.c", zlib));
        s.spawn(|| build_simple(tree, "a-brotli", "brotli.c", &[]));
        s.spawn(|| {
            build_simple(
                tree,
                "a-jbig2dec",
                "jbig2dec.c",
                &[("HAVE_STRING_H", "1"), ("JBIG_NO_MEMENTO", "1")],
            )
        });
        s.spawn(|| {
            build_simple(
                tree,
                "a-openjpeg",
                "openjpeg.c",
                &[("USE_JPIP", "1"), ("OPJ_STATIC", "1"), ("OPJ_EXPORTS", "1")],
            )
        });
        s.spawn(|| build_simple(tree, "a-lcms2", "lcms2.c", &[]));
        s.spawn(|| build_simple(tree, "a-mujs", "mujs.c", &[]));
        s.spawn(|| build_simple(tree, "a-gumbo", "gumbo.c", &[]));
        s.spawn(|| build_simple(tree, "a-libwebp", "libwebp.c", &[]));
        s.spawn(|| build_simple(tree, "jxldec", "jxl.c", &[]));
    });

    link_system_libraries(tree.target);
    println!("cargo:rerun-if-changed=csrc");
    println!("cargo:rerun-if-changed=../../third_party");
    println!("cargo:rerun-if-env-changed=NASM");
    regenerate_bindings(tree);
}

/// The OS libraries MuPDF and its libraries call (C++ runtimes come from `cc`).
fn link_system_libraries(target: Target) {
    let libs: &[&str] = match target.os {
        // WIC for JPEG XR; CryptoAPI and CNG for signatures and libarchive; the font folder.
        Os::Windows => &[
            "ole32",
            "windowscodecs",
            "user32",
            "gdi32",
            "advapi32",
            "shell32",
            "crypt32",
            "ncrypt",
            "bcrypt",
        ],
        Os::Linux => &["fontconfig", "crypto"],
        // Apple's iconv is GNU libiconv in its own library, not in libSystem as on glibc.
        Os::Mac | Os::Ios => &["iconv"],
        Os::Android | Os::OtherPosix => &[],
    };
    for lib in libs {
        println!("cargo:rustc-link-lib={lib}");
    }
    let frameworks: &[&str] = match target.os {
        Os::Mac | Os::Ios => &["CoreFoundation", "CoreGraphics", "CoreText", "Security"],
        _ => &[],
    };
    for framework in frameworks {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}

fn build_simple(tree: &Tree, lib: &str, file: &str, defines: &[(&str, &str)]) {
    let dir = tree.lib(lib);
    let mut build = library(tree.target);
    build.include(&dir).file(dir.join(file));
    for (key, value) in defines {
        build.define(key, Some(*value));
    }
    build.compile(lib);
}

fn build_freetype(tree: &Tree) {
    let dir = tree.lib("a-freetype");
    library(tree.target)
        .include(tree.mupdf.join("scripts/freetype"))
        .include(dir.join("include"))
        .include(tree.lib("a-brotli"))
        .include(tree.lib("a-zlib"))
        .define("FT2_BUILD_LIBRARY", None)
        .define("FT_CONFIG_MODULES_H", Some("\"slimftmodules.h\""))
        .define("FT_CONFIG_OPTIONS_H", Some("\"slimftoptions.h\""))
        .file(dir.join("freetype.c"))
        .compile("a-freetype");
}

fn build_harfbuzz(tree: &Tree) {
    let dir = tree.lib("a-harfbuzz");
    let mut build = cpp_library(tree.target, "c++17");
    build
        .flag_if_supported("/bigobj")
        .include(&dir)
        .include(tree.mupdf.join("scripts/freetype"))
        .include(tree.lib("a-freetype/include"))
        .define("HAVE_FALLBACK", Some("1"))
        .define("HAVE_OT", None)
        .define("HAVE_FREETYPE", None)
        // SumatraPDF routes HarfBuzz's allocations through mupdf_load_system_font.c.
        .define("hb_malloc_impl", Some("sumatra_hb_malloc"))
        .define("hb_calloc_impl", Some("sumatra_hb_calloc"))
        .define("hb_realloc_impl", Some("sumatra_hb_realloc"))
        .define("hb_free_impl", Some("sumatra_hb_free"));
    if !tree.target.msvc {
        // The amalgamation is ordered for a compiler without symbol visibility (MSVC); GCC and Clang need this.
        build.define("HB_NO_VISIBILITY", Some("1"));
    }
    build.file(dir.join("harfbuzz.cc")).compile("a-harfbuzz");
}

fn build_extract(tree: &Tree) {
    let dir = tree.lib("a-extract");
    library(tree.target)
        .include(&dir)
        .include(tree.lib("a-zlib"))
        .define("EXTRACT_NO_OWN_MEMENTO", None)
        .file(dir.join("extract.c"))
        .compile("a-extract");
}

fn build_cmark(tree: &Tree) {
    let dir = tree.lib("cmark-gfm");
    library(tree.target)
        .include(dir.join("src"))
        .include(dir.join("extensions"))
        .include(tree.mupdf.join("scripts/cmark-gfm"))
        .define("CMARK_GFM_STATIC_DEFINE", None)
        .files(sources(&dir.join("src"), ".c", &[]))
        .files(sources(&dir.join("extensions"), ".c", &[]))
        .compile("cmark-gfm");
}

fn build_libjpeg(tree: &Tree) {
    let dir = tree.lib("libjpeg-turbo");
    let src = dir.join("src");
    let mut build = library(tree.target);
    build.include(&src);
    build.files(LIBJPEG_SRC.iter().map(|name| src.join(name)));
    // The 8-bit wrappers (MuPDF uses only the 8-bit API), without cjpeg / djpeg's readers and writers.
    let wrapper = sources(
        &src.join("wrapper"),
        "-8.c",
        &["rdcolmap-8.c", "rdppm-8.c", "wrgif-8.c", "wrppm-8.c"],
    );
    build.files(wrapper);
    if assemble_libjpeg_simd(tree, &mut build) {
        build
            .include(dir.join("simd"))
            .file(dir.join("simd/jsimd.c"));
    } else {
        build.define("WITHOUT_SIMD", None);
    }
    build.compile("libjpeg-turbo");
}

/// Assembles libjpeg-turbo's SIMD code into `build` with NASM. Returns false where there is none to build (ARM) or
/// no NASM to build it with.
fn assemble_libjpeg_simd(tree: &Tree, build: &mut cc::Build) -> bool {
    let target = tree.target;
    let (arch_dir, format, defines): (&str, &str, &[&str]) = match (target.arch, target.os) {
        (Arch::X86_64, Os::Windows) => ("x86_64", "win64", &["WIN64", "__x86_64__"]),
        (Arch::X86_64, Os::Mac | Os::Ios) => ("x86_64", "macho64", &["MACHO", "__x86_64__"]),
        (Arch::X86_64, _) => ("x86_64", "elf64", &["ELF", "__x86_64__", "PIC"]),
        (Arch::X86, Os::Windows) => ("i386", "win32", &["WIN32"]),
        (Arch::X86, Os::Mac | Os::Ios) => ("i386", "macho32", &["MACHO"]),
        (Arch::X86, _) => ("i386", "elf32", &["ELF", "PIC"]),
        (Arch::Aarch64 | Arch::Other, _) => return false,
    };
    let nasm = env::var_os("NASM").map_or_else(|| PathBuf::from("nasm"), PathBuf::from);
    if Command::new(&nasm).arg("-v").output().is_err() {
        println!(
            "cargo:warning=NASM not found (install it, or set NASM to its path): libjpeg-turbo is built without SIMD"
        );
        return false;
    }

    let simd = tree.lib("libjpeg-turbo/simd");
    let objects = tree.out.join("libjpeg-simd");
    fs::create_dir_all(&objects).expect("can create OUT_DIR/libjpeg-simd");
    // The *ext-*.asm files are %included by the others, never assembled on their own.
    let asm = sources(&simd.join(arch_dir), ".asm", &[]);
    for file in asm.iter().filter(|f| !f.to_string_lossy().contains("ext-")) {
        let stem = file
            .file_stem()
            .expect("an .asm file has a name")
            .to_string_lossy();
        let object = objects.join(format!("{stem}.o"));
        let mut nasm_cmd = Command::new(&nasm);
        nasm_cmd
            .arg("-f")
            .arg(format)
            .args(defines.iter().map(|d| format!("-D{d}")));
        // NASM wants the trailing separator on include directories.
        nasm_cmd.arg(format!("-I{}/", simd.join("nasm").display()));
        nasm_cmd.arg(format!("-I{}/", simd.join(arch_dir).display()));
        nasm_cmd.arg("-o").arg(&object).arg(file);
        let status = nasm_cmd
            .status()
            .unwrap_or_else(|e| panic!("cannot run NASM: {e}"));
        assert!(status.success(), "NASM failed on {}", file.display());
        build.object(object);
    }
    true
}

fn build_libarchive(tree: &Tree) {
    let target = tree.target;
    let lzma = tree.lib("liblzma");
    let mut build = library(target);
    // On POSIX, our config headers go first: they shadow the Windows ones next to the sources.
    match target.os {
        Os::Windows => {
            build.define("PLATFORM_CONFIG_H", Some("\"config_windows.h\""));
        }
        _ => {
            let liblzma_config = if target.is_apple() { "apple" } else { "linux" };
            build
                .include(tree.csrc.join("compat/libarchive"))
                .include(tree.csrc.join("compat/liblzma").join(liblzma_config))
                .define("PLATFORM_CONFIG_H", Some("\"config_posix.h\""))
                // The POSIX entry points missing from the Windows-built amalgamation.
                .file(tree.sumatra_ng.join("libarchive/libarchive_posix.c"));
        }
    }
    build
        .include(tree.lib("a-libarchive"))
        .include(tree.lib("a-libarchive/libarchive"))
        .include(tree.lib("a-zlib"))
        .include(tree.lib("a-bzip2"))
        .includes(LIBLZMA_INCLUDES.iter().map(|dir| lzma.join(dir)))
        .define("LIBARCHIVE_STATIC", None)
        .define("BZ_NO_STDIO", None)
        .define("HAVE_CONFIG_H", None)
        .define("LZMA_API_STATIC", None)
        .file(tree.lib("a-libarchive/libarchive.c"))
        .file(tree.lib("a-bzip2/bzip2.c"))
        .files(LIBLZMA_SRC.iter().map(|file| lzma.join(file)))
        .compile("a-libarchive");
}

fn build_leptonica(tree: &Tree) {
    let dir = tree.lib("leptonica/src");
    let mut build = library(tree.target);
    // As MuPDF builds it: no image codecs, memory through MuPDF's allocator (leptonica-wrap.c).
    build
        .include(&dir)
        .include(tree.mupdf.join("include"))
        .include(tree.mupdf.join("scripts/tesseract"))
        .define("HAVE_LEPTONICA", None)
        .define("LEPTONICA_INTERCEPT_ALLOC", Some("1"));
    for codec in [
        "LIBPNG",
        "LIBTIFF",
        "LIBJPEG",
        "LIBZ",
        "LIBGIF",
        "LIBUNGIF",
        "LIBWEBP",
        "LIBWEBP_ANIM",
        "LIBJP2K",
    ] {
        build.define(&format!("HAVE_{codec}"), Some("0"));
    }
    build
        .files(sources(&dir, ".c", &[]))
        .file(tree.mupdf.join("source/fitz/leptonica-wrap.c"))
        .compile("leptonica");
}

fn build_tesseract(tree: &Tree) {
    let target = tree.target;
    let dir = tree.lib("tesseract");
    let configure = |build: &mut cc::Build| {
        build
            .includes(TESSERACT_INCLUDES.iter().map(|sub| dir.join(sub)))
            .include(tree.lib("leptonica/src"))
            .include(tree.mupdf.join("include"))
            .include(tree.mupdf.join("scripts/tesseract"));
        // The LSTM engine only, no debug UI, as MuPDF builds it.
        for define in [
            "HAVE_LEPTONICA",
            "HAVE_TESSERACT",
            "TESSERACT_IMAGEDATA_AS_PIX",
            "TESSERACT_DISABLE_DEBUG_FONTS",
            "GRAPHICS_DISABLED",
            "DISABLED_LEGACY_ENGINE",
        ] {
            build.define(define, None);
        }
        if target.is_x86() {
            for define in TESSERACT_X86_DEFINES {
                build.define(define, None);
            }
        }
        if target.is_x86() && target.msvc {
            // cl.exe compiles the intrinsics without flags but doesn't predefine the macros the kernels test.
            for define in ["__AVX__", "__AVX2__", "__FMA__", "__SSE4_1__"] {
                build.define(define, None);
            }
        }
    };

    let kernels: Vec<&str> = TESSERACT_X86_KERNELS
        .iter()
        .map(|(file, _)| *file)
        .collect();
    // GCC and Clang build each x86 kernel with its own instruction-set flag; ARM builds none of them.
    let separate_kernels = !(target.is_x86() && target.msvc);
    let mut build = cpp_library(target, "c++20");
    configure(&mut build);
    for sub in fs::read_dir(dir.join("src")).expect("tesseract/src exists") {
        let sub = sub.expect("readable directory entry").path();
        if sub.is_dir() {
            build.files(sources(
                &sub,
                ".cpp",
                if separate_kernels { &kernels } else { &[] },
            ));
        }
    }
    build
        .file(tree.mupdf.join("source/fitz/tessocr.cpp"))
        .compile("tesseract");

    if target.is_x86() && !target.msvc {
        for (file, flag) in TESSERACT_X86_KERNELS {
            let mut kernel = cpp_library(target, "c++20");
            configure(&mut kernel);
            let stem = file.trim_end_matches(".cpp");
            kernel
                .flag(flag)
                .file(dir.join("src/arch").join(file))
                .compile(&format!("tesseract-{stem}"));
        }
    }
}

fn build_zxing(tree: &Tree) {
    let target = tree.target;
    let core = tree.lib("zxing-cpp/core/src");
    let configure = |build: &mut cc::Build| {
        build
            .include(&core)
            .include(tree.mupdf.join("scripts/zxing-cpp"))
            .include(tree.lib("zint/backend"))
            .include(tree.mupdf.join("include"));
        for define in ZXING_DEFINES {
            build.define(define, None);
        }
        build.define("FZ_ENABLE_BARCODE", Some("1"));
    };

    // zxing-cpp reads barcodes (C++); zint writes them (C).
    let mut cpp = cpp_library(target, "c++20");
    configure(&mut cpp);
    for dir in ZXING_DIRS {
        // ECI.cpp is built as zxing-eci.cpp: zint's eci.c would otherwise make a second eci object.
        cpp.files(sources(&core.join(dir), ".cpp", &["ECI.cpp"]));
    }
    cpp.file(tree.mupdf.join("source/fitz/zxingbarcode.cpp"))
        .file(tree.sumatra.join("zxing-eci.cpp"))
        .compile("zxing");

    let mut c = library(target);
    configure(&mut c);
    c.file(core.join("libzueci/zueci.c"))
        .files(sources(&tree.lib("zint/backend"), ".c", &[]))
        .compile("zint");
}

fn mupdf_includes(tree: &Tree, build: &mut cc::Build) {
    build
        .include(tree.csrc.join("compat"))
        // Resolves noto_sumatra.c's SumatraPDF-relative include; see the header in csrc/compat/sumatra-layout.
        .include(tree.csrc.join("compat/sumatra-layout/src/shared/mupdf"))
        .include(&tree.sumatra)
        // For `#include "mupdf/font_common.h"` in SumatraPDF's macOS font loader.
        .include(
            tree.sumatra
                .parent()
                .expect("sumatra/shared/mupdf has a parent"),
        )
        .include(tree.mupdf.join("source/fitz"))
        .include(tree.mupdf.join("include"))
        .include(tree.mupdf.join("scripts/freetype"))
        .include(tree.mupdf.join("scripts/cmark-gfm"));
    for lib in [
        "jxldec",
        "a-jbig2dec",
        "libjpeg-turbo/src",
        "a-openjpeg",
        "a-freetype/include",
        "a-mujs",
        "a-brotli",
        "cmark-gfm/src",
        "cmark-gfm/extensions",
        "a-harfbuzz",
        "a-lcms2",
        "a-gumbo",
        "a-extract",
        "a-libwebp",
        "a-libarchive",
        // unlibarchive.c includes <archive.h> off Windows.
        "a-libarchive/libarchive",
        "a-zlib",
    ] {
        build.include(tree.lib(lib));
    }
    for (key, value) in MUPDF_DEFINES {
        build.define(key, Some(*value));
    }
    match (tree.target.os, tree.target.arch) {
        (Os::Windows, Arch::Aarch64) => {
            build.define("ARCH_HAS_NEON", Some("1"));
        }
        (Os::Linux, _) => {
            build.define("HAVE_LIBCRYPTO", Some("1"));
        }
        _ => {}
    }
    // mu-threads (the lock functions the shim gives MuPDF) uses Windows threads, or pthreads when this is set.
    if tree.target.os != Os::Windows {
        build.define("HAVE_PTHREAD", None);
    }
}

fn build_mupdf(tree: &Tree) {
    let target = tree.target;
    let source = tree.mupdf.join("source");
    let mut build = library(target);
    mupdf_includes(tree, &mut build);

    // JPEG XR goes through WIC on Windows (patch 0006); elsewhere MuPDF's loader is a stub without jxrlib.
    let jxr = if target.os == Os::Windows {
        "load-jxr.c"
    } else {
        "load-jxr-win.c"
    };
    // noto.c: replaced by noto_sumatra.c. leptonica-wrap.c: in the leptonica library.
    build.files(sources(
        &source.join("fitz"),
        ".c",
        &["noto.c", "leptonica-wrap.c", jxr],
    ));
    for dir in ["cbz", "html", "pdf", "svg", "xps", "reflow"] {
        build.files(sources(&source.join(dir), ".c", &[]));
    }
    // cmapdump.c is a build tool; mutool.c and muraster.c only hold main().
    build.files(sources(
        &source.join("tools"),
        ".c",
        &["cmapdump.c", "mutool.c", "muraster.c"],
    ));
    build.file(source.join("helpers/mu-threads/mu-threads.c"));

    // SumatraPDF's own files compiled into its mupdf library.
    for file in [
        "fonts_map.c",
        "noto_sumatra.c",
        "ofd.c",
        "ofd-crypto.c",
        "ofd-decrypt.c",
        "ofd-seal.c",
    ] {
        build.file(tree.sumatra.join(file));
    }
    let ng = tree.sumatra_ng.join("mupdf");
    match target.os {
        Os::Windows => {
            build.file(tree.sumatra.join("pkcs7-windows.c"));
        }
        // The signing calls left in pdfsign.c and murun.c by patch 0003 report "unavailable".
        _ => {
            build.file(ng.join("pkcs7_stub_posix.c"));
        }
    }
    match target.os {
        Os::Linux => {
            build.file(source.join("helpers/pkcs7/pkcs7-openssl.c"));
        }
        Os::Mac => {
            build.file(ng.join("pkcs7_mac.c"));
        }
        _ => {}
    }
    if target.is_apple() {
        build.file(ng.join("mupdf_load_system_font_mac.c"));
    }
    if target.os == Os::Android {
        build_android_font_loader(tree);
    } else {
        build.file(tree.sumatra.join("mupdf_load_system_font.c"));
    }
    build.file(tree.csrc.join("shim.c"));
    build.compile("mupdf");

    // load-jxl.cpp (patch 0047) is C++; it needs the same includes and defines.
    let mut cpp = cpp_library(target, "c++17");
    mupdf_includes(tree, &mut cpp);
    cpp.file(tree.sumatra.join("load-jxl.cpp"))
        .compile("mupdf-cpp");
}

/// mupdf_load_system_font.c reads fonts through fontconfig wherever `__linux__` is defined, and Android, which has
/// no fontconfig, defines it too. Built as a POSIX system without a font list, it keeps its HarfBuzz allocator.
fn build_android_font_loader(tree: &Tree) {
    let mut build = library(tree.target);
    mupdf_includes(tree, &mut build);
    build
        .flag("-U__linux__")
        .file(tree.sumatra.join("mupdf_load_system_font.c"))
        .compile("mupdf-system-font");
}

#[cfg(feature = "regenerate-bindings")]
fn regenerate_bindings(tree: &Tree) {
    let out = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR"))
        .join("src/bindings.rs");
    bindgen::Builder::default()
        .header(tree.csrc.join("shim.h").to_string_lossy())
        .clang_arg(format!("-I{}", tree.mupdf.join("include").display()))
        // Only the shim's API and the plain MuPDF value types it uses; nothing of MuPDF's internals.
        .allowlist_recursively(false)
        .allowlist_function("ex_.*")
        .allowlist_type("ex_.*")
        .allowlist_type("fz_rect|fz_irect|fz_point|fz_quad|fz_matrix|fz_cookie|fz_error_type|pdf_annot_type")
        .prepend_enum_name(false)
        // MuPDF's object types are declared by hand in src/opaque.rs (opaque, !Send, !Sync).
        .blocklist_type(
            "fz_context|fz_document|fz_page|fz_pixmap|fz_stext_page|fz_buffer|fz_display_list|fz_outline|fz_document_writer|pdf_annot|fz_image",
        )
        .raw_line("use crate::opaque::*;")
        .derive_default(true)
        .layout_tests(true)
        .use_core()
        .wrap_unsafe_ops(true)
        .raw_line("// Generated by bindgen from csrc/shim.h; regenerate with `cargo build -p ecclesia-mupdf-sys --features regenerate-bindings`.")
        .generate()
        .expect("bindgen failed on csrc/shim.h")
        .write_to_file(&out)
        .expect("cannot write src/bindings.rs");
}

#[cfg(not(feature = "regenerate-bindings"))]
fn regenerate_bindings(_tree: &Tree) {}
