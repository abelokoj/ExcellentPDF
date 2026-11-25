//! K.21's "Done when": a tampered pack is rejected; interrupted downloads resume; removing a pack leaves no files.
//! The network is a test source that serves a pack from any offset and can drop the connection part way.

use std::cell::Cell;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use ed25519_dalek::SigningKey;
use excellent_packs::download::Source;
use excellent_packs::index::Entry;
use excellent_packs::{Error, Manager, file};
use sha2::{Digest, Sha256};

/// A folder in the temp folder, removed when dropped.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(std::env::temp_dir().join(format!("excellent-packs-{}-{n}", std::process::id())))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn key() -> SigningKey {
    SigningKey::from_bytes(&[42; 32])
}

/// A pack's archive: its manifest and some files, one in a subfolder.
fn archive(id: &str, extra: &[(&str, &[u8])]) -> Vec<u8> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).expect("file");
    write!(
        zip,
        r#"{{"id": "{id}", "version": "1", "title": "Test pack"}}"#
    )
    .expect("write");
    zip.start_file("words/en.txt", options).expect("file");
    zip.write_all(&b"aardvark\nabacus\n".repeat(4000))
        .expect("write");
    for (name, bytes) in extra {
        zip.start_file(*name, options).expect("file");
        zip.write_all(bytes).expect("write");
    }
    zip.finish().expect("finish").into_inner()
}

fn entry_for(id: &str, bytes: &[u8]) -> Entry {
    Entry {
        id: id.into(),
        version: "1".into(),
        title: "Test pack".into(),
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        url: "https://packs.example/test.expack".into(),
        licence: "CC0-1.0".into(),
    }
}

/// Serves `bytes` from any offset; while `drop_after` holds a number, a connection ends with an error after that many
/// bytes.
struct Flaky {
    bytes: Vec<u8>,
    drop_after: Cell<Option<usize>>,
    opened_from: std::cell::RefCell<Vec<u64>>,
}

struct Reader {
    data: Cursor<Vec<u8>>,
    left: Option<usize>,
}

impl Read for Reader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match &mut self.left {
            Some(0) => Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "dropped",
            )),
            Some(left) => {
                let n = buf.len().min(*left);
                let got = self.data.read(&mut buf[..n])?;
                *left -= got;
                Ok(got)
            }
            None => self.data.read(buf),
        }
    }
}

impl Source for Flaky {
    fn open(&self, _url: &str, from: u64) -> Result<(Box<dyn Read>, bool), Error> {
        self.opened_from.borrow_mut().push(from);
        let rest = self.bytes[usize::try_from(from).expect("offset")..].to_vec();
        Ok((
            Box::new(Reader {
                data: Cursor::new(rest),
                left: self.drop_after.take(),
            }),
            true,
        ))
    }
}

#[test]
fn a_tampered_pack_is_rejected() {
    let scratch = Scratch::new();
    let manager = Manager::new(scratch.0.join("packs"), key().verifying_key());
    let mut signed = file::sign(&archive("dict-en", &[]), &key());
    let last = signed.len() - 10;
    signed[last] ^= 0xff;
    let path = scratch.0.join("dict-en.expack");
    std::fs::create_dir_all(&scratch.0).expect("folder");
    std::fs::write(&path, &signed).expect("write");
    assert!(matches!(manager.install_file(&path), Err(Error::Signature)));
    assert!(!manager.is_installed("dict-en"), "nothing installed");

    // Downloaded: the index's hash catches a changed file before its signature is even looked at.
    let good = file::sign(&archive("dict-en", &[]), &key());
    let entry = entry_for("dict-en", &good);
    let source = Flaky {
        bytes: signed,
        drop_after: Cell::new(None),
        opened_from: Default::default(),
    };
    let result = manager.download(&entry, &source, &AtomicBool::new(false), &mut |_, _| {});
    assert!(matches!(result, Err(Error::Hash)), "{result:?}");
    assert!(!manager.is_installed("dict-en"));
}

#[test]
fn an_archive_that_escapes_its_folder_is_refused() {
    let scratch = Scratch::new();
    let manager = Manager::new(scratch.0.join("packs"), key().verifying_key());
    let signed = file::sign(&archive("evil", &[("../outside.txt", b"x")]), &key());
    let path = scratch.0.join("evil.expack");
    std::fs::create_dir_all(&scratch.0).expect("folder");
    std::fs::write(&path, signed).expect("write");
    assert!(matches!(manager.install_file(&path), Err(Error::Unsafe(_))));
    assert!(!scratch.0.join("outside.txt").exists());
    assert!(!manager.is_installed("evil"));
}

#[test]
fn an_interrupted_download_resumes_where_it_stopped() {
    let scratch = Scratch::new();
    let manager = Manager::new(scratch.0.join("packs"), key().verifying_key());
    let bytes = file::sign(&archive("dict-en", &[]), &key());
    let entry = entry_for("dict-en", &bytes);
    let half = bytes.len() / 2;
    let source = Flaky {
        bytes,
        drop_after: Cell::new(Some(half)),
        opened_from: Default::default(),
    };
    let first = manager.download(&entry, &source, &AtomicBool::new(false), &mut |_, _| {});
    assert!(matches!(first, Err(Error::Network(_))), "{first:?}");
    let mut last = 0;
    let manifest = manager
        .download(&entry, &source, &AtomicBool::new(false), &mut |got, _| {
            last = got
        })
        .expect("resumed and installed");
    assert_eq!(manifest.id, "dict-en");
    assert_eq!(
        *source.opened_from.borrow(),
        [0, half as u64],
        "the second request starts where the first stopped"
    );
    assert_eq!(last, entry.size);
    assert!(manager.is_installed("dict-en"));
}

#[test]
fn removing_a_pack_leaves_no_files() {
    let scratch = Scratch::new();
    let root = scratch.0.join("packs");
    let manager = Manager::new(root.clone(), key().verifying_key());
    let path = scratch.0.join("fonts.expack");
    std::fs::create_dir_all(&scratch.0).expect("folder");
    std::fs::write(
        &path,
        file::sign(&archive("fonts", &[("deep/a/b/c.ttf", b"font")]), &key()),
    )
    .expect("write");
    manager.install_file(&path).expect("installed");
    assert_eq!(manager.installed().expect("listed").len(), 1);
    manager.remove("fonts").expect("removed");
    assert!(!manager.is_installed("fonts"));
    let left: Vec<_> = std::fs::read_dir(&root)
        .expect("packs folder")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
    assert!(manager.remove("../x").is_err(), "only a pack id is removed");
}
