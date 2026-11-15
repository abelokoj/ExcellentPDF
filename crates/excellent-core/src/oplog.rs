//! The operation log (K.14, K.15): every operation applied to a PDF, every undo, redo and save, one JSON line each,
//! flushed to disk before the edit counts. A run that ends before its edits are saved leaves them here, and the next
//! run replays them (recovery).
//!
//! The log starts with the file it applies to (its [`Fingerprint`]) and is written against it: the first record
//! after that creates the file. Saves are incremental, so the file as the log started is still the start of the saved
//! file, and replaying the whole log over those bytes gives the document as it was, saved or not. A save that rewrites
//! the whole file starts a new log.
//!
//! A record half written when the process died is cut off when the log is read, with everything after it: a record
//! counts once its line ends. Each line is written in one call and synced before the next edit is accepted.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use excellent_mupdf::UndoState;
use serde_json::{Map, Value, json};

use crate::edit::Operation;

/// The log's format; a log of another format is not read.
const SCHEMA: u64 = 1;

/// One version of a file: its size, modified time and a hash of its ends ([`crate::document::FileId`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct Fingerprint {
    pub(crate) size: u64,
    /// Nanoseconds since 1970.
    pub(crate) modified: u64,
    pub(crate) sample: u64,
}

impl Fingerprint {
    fn to_json(self) -> Value {
        json!({"size": self.size, "modified": self.modified, "sample": self.sample})
    }

    fn from_json(value: &Value) -> Option<Self> {
        let number = |key: &str| value.get(key)?.as_u64();
        Some(Self {
            size: number("size")?,
            modified: number("modified")?,
            sample: number("sample")?,
        })
    }
}

/// A line of the log.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Record {
    /// The first line: the file the log applies to.
    Start {
        base: Fingerprint,
    },
    /// An operation, numbered from 0 in its log.
    Operation {
        id: u64,
        operation: Operation,
    },
    Undo,
    Redo,
    /// The document was saved, incrementally, as the file `file`.
    Saved {
        file: Fingerprint,
    },
}

impl Record {
    fn from_json(value: &Value) -> Option<Self> {
        let record = value.as_object()?;
        Some(match record.get("kind")?.as_str()? {
            "start" if record.get("schema")?.as_u64()? == SCHEMA => Self::Start {
                base: Fingerprint::from_json(record.get("file")?)?,
            },
            "operation" => Self::Operation {
                id: record.get("op")?.as_u64()?,
                operation: Operation::read_json(record)?,
            },
            "undo" => Self::Undo,
            "redo" => Self::Redo,
            "saved" => Self::Saved {
                file: Fingerprint::from_json(record.get("file")?)?,
            },
            _ => return None,
        })
    }
}

/// The undo history the log records: the operations done, the ones undone (to redo), and the one the saved file ends
/// with. The document has unsaved changes while its last done operation isn't that one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct History {
    done: Vec<u64>,
    undone: Vec<u64>,
    /// The last operation done when the file was saved; `None` while it is the file the log started with.
    saved: Option<u64>,
    /// The next operation's id.
    next: u64,
}

impl History {
    /// Whether the document differs from its saved file.
    pub(crate) fn unsaved(&self) -> bool {
        self.done.last().copied() != self.saved
    }

    /// Where the document is in its history.
    pub(crate) fn state(&self) -> UndoState {
        UndoState {
            position: self.done.len(),
            steps: self.done.len() + self.undone.len(),
        }
    }

    /// Follows `record`; `false` when it can't follow (an undo with nothing done, an id out of order), and then the
    /// record is not one of this log.
    fn play(&mut self, record: &Record) -> bool {
        match *record {
            Record::Start { .. } => false,
            Record::Operation { id, .. } => {
                if id != self.next {
                    return false;
                }
                self.done.push(id);
                self.undone.clear();
                self.next = id + 1;
                true
            }
            Record::Undo => self.done.pop().map(|id| self.undone.push(id)).is_some(),
            Record::Redo => self.undone.pop().map(|id| self.done.push(id)).is_some(),
            Record::Saved { .. } => {
                self.saved = self.done.last().copied();
                true
            }
        }
    }
}

/// A log read back: its records up to the first that is torn, unreadable or doesn't follow, and what they say.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Read {
    pub(crate) records: Vec<Record>,
    /// The bytes those records take.
    len: u64,
    pub(crate) history: History,
    /// The file the log started with.
    pub(crate) base: Fingerprint,
    /// The file as it was last saved, or the base if it never was.
    pub(crate) last: Fingerprint,
}

/// Reads the log at `path`. `None` when it has no readable start (a log of another format, or empty).
///
/// # Errors
///
/// The file can't be read (`NotFound` when there is none).
pub(crate) fn read(path: &Path) -> io::Result<Option<Read>> {
    let bytes = fs::read(path)?;
    let mut lines = bytes.split_inclusive(|&byte| byte == b'\n');
    let parse = |line: &[u8]| -> Option<Record> {
        let text = line.strip_suffix(b"\n")?;
        Record::from_json(&serde_json::from_slice(text).ok()?)
    };
    let Some((first, Some(Record::Start { base }))) = lines.next().map(|line| (line, parse(line)))
    else {
        return Ok(None);
    };
    let mut read = Read {
        records: Vec::new(),
        len: first.len() as u64,
        history: History::default(),
        base,
        last: base,
    };
    for line in lines {
        let Some(record) = parse(line) else { break };
        if !read.history.play(&record) {
            break;
        }
        if let Record::Saved { file } = record {
            read.last = file;
        }
        read.records.push(record);
        read.len += line.len() as u64;
    }
    Ok(Some(read))
}

/// The log being written for an open document.
#[derive(Debug)]
pub(crate) struct Log {
    path: PathBuf,
    base: Fingerprint,
    /// Open once the log has a record past its start; `None` before (nothing written yet).
    file: Option<File>,
    /// The bytes of whole records in the file.
    len: u64,
    /// A failed write couldn't be cut off: the log takes nothing more, so no edit is kept that it can't recover.
    broken: bool,
    history: History,
}

impl Log {
    /// A log at `path` for the file `base`, written from its first record (which replaces any log already there).
    pub(crate) fn new(path: PathBuf, base: Fingerprint) -> Self {
        Self {
            path,
            base,
            file: None,
            len: 0,
            broken: false,
            history: History::default(),
        }
    }

    /// The log `read` from `path`, written on from its last whole record.
    ///
    /// # Errors
    ///
    /// The file can't be opened or cut to its whole records.
    pub(crate) fn resume(path: PathBuf, read: &Read) -> io::Result<Self> {
        let mut file = OpenOptions::new().read(true).write(true).open(&path)?;
        file.set_len(read.len)?;
        file.seek(SeekFrom::End(0))?;
        Ok(Self {
            path,
            base: read.base,
            file: Some(file),
            len: read.len,
            broken: false,
            history: read.history.clone(),
        })
    }

    /// The undo history it records.
    pub(crate) fn history(&self) -> &History {
        &self.history
    }

    /// Records `operation`, applied.
    ///
    /// # Errors
    ///
    /// The record couldn't be written: the operation isn't kept.
    pub(crate) fn operation(&mut self, operation: &Operation) -> io::Result<()> {
        let id = self.history.next;
        let mut record = Map::new();
        record.insert("kind".into(), "operation".into());
        record.insert("op".into(), id.into());
        record.insert("time".into(), now().into());
        operation.write_json(&mut record);
        self.write(&Value::Object(record))?;
        let followed = self.history.play(&Record::Operation {
            id,
            operation: operation.clone(),
        });
        debug_assert!(followed, "the next id follows");
        Ok(())
    }

    /// Records an undo.
    ///
    /// # Errors
    ///
    /// The record couldn't be written.
    pub(crate) fn undo(&mut self) -> io::Result<()> {
        self.write(&json!({"kind": "undo", "time": now()}))?;
        self.history.play(&Record::Undo);
        Ok(())
    }

    /// Records a redo.
    ///
    /// # Errors
    ///
    /// The record couldn't be written.
    pub(crate) fn redo(&mut self) -> io::Result<()> {
        self.write(&json!({"kind": "redo", "time": now()}))?;
        self.history.play(&Record::Redo);
        Ok(())
    }

    /// Records an incremental save as the file `file`.
    ///
    /// # Errors
    ///
    /// The record couldn't be written.
    pub(crate) fn saved(&mut self, file: Fingerprint) -> io::Result<()> {
        let saved = Record::Saved { file };
        self.write(&json!({"kind": "saved", "time": now(), "file": file.to_json()}))?;
        self.history.play(&saved);
        Ok(())
    }

    /// Starts again for the file `base`, with no history: the file it applied to is gone (rewritten, or the
    /// document is closing until it is needed again).
    pub(crate) fn restart(&mut self, base: Fingerprint) {
        self.remove();
        *self = Self::new(std::mem::take(&mut self.path), base);
    }

    /// Deletes the file if this log wrote it; a log found from an earlier run is left for recovery.
    pub(crate) fn remove(&mut self) {
        if self.file.take().is_some()
            && let Err(err) = fs::remove_file(&self.path)
            && err.kind() != io::ErrorKind::NotFound
        {
            log::warn!("oplog: can't remove {}: {err}", self.path.display());
        }
    }

    fn write(&mut self, record: &Value) -> io::Result<()> {
        if self.broken {
            return Err(io::Error::other("the operation log can't be written"));
        }
        let mut line = record.to_string();
        line.push('\n');
        if self.file.is_none() {
            let mut start =
                json!({"kind": "start", "schema": SCHEMA, "file": self.base.to_json()}).to_string();
            start.push('\n');
            if let Some(folder) = self.path.parent() {
                fs::create_dir_all(folder)?;
            }
            let mut file = File::create(&self.path)?;
            file.write_all(start.as_bytes())?;
            self.len = start.len() as u64;
            self.file = Some(file);
        }
        let file = self.file.as_mut().expect("created above");
        let written = file
            .write_all(line.as_bytes())
            .and_then(|()| file.sync_data());
        match written {
            Ok(()) => {
                self.len += line.len() as u64;
                Ok(())
            }
            Err(err) => {
                // Cut the part written, so the records after it can still be read.
                let cut = file
                    .set_len(self.len)
                    .and_then(|()| file.seek(SeekFrom::Start(self.len)));
                self.broken = cut.is_err();
                Err(err)
            }
        }
    }
}

/// Milliseconds since 1970, for the records' times.
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::edit::Change;

    const BASE: Fingerprint = Fingerprint {
        size: 1000,
        modified: 7,
        sample: 42,
    };

    fn temp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("excellent-oplog-{}-{name}", std::process::id()))
    }

    fn rename(title: &str) -> Operation {
        Operation::new(
            "Rename",
            vec![Change::UpdateOutlineEntry {
                at: vec![0],
                title: Some(title.into()),
                uri: None,
                open: false,
            }],
        )
    }

    /// One step a user takes, for the property test.
    #[derive(Debug, Clone)]
    enum Step {
        Operation,
        Undo,
        Redo,
        Save,
    }

    fn step() -> impl Strategy<Value = Step> {
        prop_oneof![
            3 => Just(Step::Operation),
            2 => Just(Step::Undo),
            1 => Just(Step::Redo),
            1 => Just(Step::Save),
        ]
    }

    /// Takes `steps` on a log at `path` the way the document does (an undo only when there is one), and returns it.
    fn take(path: &Path, steps: &[Step]) -> Log {
        let mut log = Log::new(path.to_owned(), BASE);
        for (n, step) in steps.iter().enumerate() {
            let state = log.history().state();
            match step {
                Step::Operation => log.operation(&rename(&n.to_string())).expect("write"),
                Step::Undo if state.can_undo() => log.undo().expect("write"),
                Step::Redo if state.can_redo() => log.redo().expect("write"),
                Step::Save => log
                    .saved(Fingerprint {
                        size: n as u64,
                        ..BASE
                    })
                    .expect("write"),
                Step::Undo | Step::Redo => {}
            }
        }
        log
    }

    #[test]
    fn nothing_is_written_until_the_first_record() {
        let path = temp("lazy.oplog");
        let _ = fs::remove_file(&path);
        let mut log = Log::new(path.clone(), BASE);
        assert!(!path.exists());
        log.operation(&rename("One")).expect("write");
        let read = read(&path).expect("read").expect("a log");
        assert_eq!(read.base, BASE);
        assert!(read.history.unsaved());
        log.remove();
        assert!(!path.exists());
    }

    #[test]
    fn a_log_from_an_earlier_run_is_kept_for_recovery() {
        let path = temp("earlier.oplog");
        let mut first = take(&path, &[Step::Operation, Step::Operation, Step::Undo]);
        drop(first.file.take());
        let read = read(&path).expect("read").expect("a log");
        assert_eq!(
            read.history.state(),
            UndoState {
                position: 1,
                steps: 2
            }
        );

        // A new run's log replaces it only once it writes; removing it before that leaves the old one.
        let mut second = Log::new(path.clone(), BASE);
        second.remove();
        assert!(path.exists());

        let mut resumed = Log::resume(path.clone(), &read).expect("resume");
        resumed.redo().expect("write");
        let again = super::read(&path).expect("read").expect("a log");
        assert_eq!(
            again.history.state(),
            UndoState {
                position: 2,
                steps: 2
            }
        );
        resumed.remove();
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// K.15's fault injection: the process dying at any byte of any write loses no record whose line was
        /// written whole, and the history read back is the history of exactly those records.
        #[test]
        fn a_log_cut_anywhere_keeps_every_whole_record(steps in prop::collection::vec(step(), 1..40)) {
            let path = temp(&format!("cut-{}.oplog", steps.len()));
            let log = take(&path, &steps);
            let Ok(bytes) = fs::read(&path) else {
                // Only undos and redos with nothing to act on: nothing was written.
                prop_assert_eq!(log.history().state().steps, 0);
                return Ok(());
            };
            let whole = read(&path).expect("read").expect("a log").history;
            prop_assert_eq!(&whole, log.history());

            let cut_path = temp(&format!("cut-{}-part.oplog", steps.len()));
            let ends: Vec<usize> = bytes
                .iter()
                .enumerate()
                .filter(|&(_, &byte)| byte == b'\n')
                .map(|(at, _)| at + 1)
                .collect();
            // Every way a cut can fall: at each line's end, a byte either side of it, and inside each line. Every
            // byte would write the file thousands of times a case, which took over 20 minutes on Windows.
            let mut cuts: Vec<usize> = vec![0, bytes.len()];
            let mut start = 0;
            for &end in &ends {
                cuts.extend([(start + end) / 2, end - 1, end, end + 1]);
                start = end;
            }
            cuts.retain(|&cut| cut <= bytes.len());
            cuts.sort_unstable();
            cuts.dedup();
            for cut in cuts {
                fs::write(&cut_path, &bytes[..cut]).expect("write");
                let lines = ends.iter().filter(|&&end| end <= cut).count();
                match read(&cut_path).expect("read") {
                    None => prop_assert_eq!(lines, 0, "the start line is whole at {}", cut),
                    Some(part) => prop_assert_eq!(part.records.len(), lines - 1, "cut at {}", cut),
                }
            }
            let _ = fs::remove_file(&cut_path);
            let _ = fs::remove_file(&path);
        }
    }

    #[test]
    fn saving_marks_the_history_saved_until_it_moves() {
        let mut history = History::default();
        assert!(!history.unsaved());
        assert!(history.play(&Record::Operation {
            id: 0,
            operation: rename("A")
        }));
        assert!(history.unsaved());
        assert!(history.play(&Record::Saved { file: BASE }));
        assert!(!history.unsaved());
        assert!(history.play(&Record::Undo));
        assert!(history.unsaved(), "back before the save");
        assert!(history.play(&Record::Redo));
        assert!(!history.unsaved(), "the saved state again");
        assert!(history.play(&Record::Undo));
        assert!(history.play(&Record::Operation {
            id: 1,
            operation: rename("B")
        }));
        assert!(
            history.unsaved(),
            "a new branch never reaches the saved state"
        );
        assert!(!history.play(&Record::Operation {
            id: 5,
            operation: rename("C")
        }));
        assert!(!history.play(&Record::Redo), "nothing undone");
    }
}
