//! Single instance (plan K.19 step 1, R.11 step 3): the first launch of the app listens on a channel named for the
//! app and the user (a named pipe on Windows, a Unix socket elsewhere); a later launch hands its message to it and
//! ends.
//!
//! Messages are opaque bytes of at most [`MAX_MESSAGE`]; the app gives them their meaning.

use std::io::{self, Read, Write};
use std::thread;
use std::time::Duration;

/// The largest message a launch may send; the rest of a longer one is dropped.
pub const MAX_MESSAGE: u64 = 64 * 1024;

/// How often a launch tries a channel that is busy with another launch's message.
const BUSY_TRIES: u32 = 20;

/// Who answered the launch.
#[derive(Debug)]
pub enum Claim {
    /// Another instance took the message: this launch is done.
    Forwarded,
    /// No instance was running: this one is the first and listens.
    Primary(Listener),
}

/// Sends `message` to the instance listening as `name`, or makes this the one that listens. `name` holds the app's
/// id; the user's name is added, so other users' instances never answer.
///
/// # Errors
///
/// The channel could be neither reached nor made.
pub fn claim(name: &str, message: &[u8]) -> io::Result<Claim> {
    let name = channel_name(name);
    // Two first launches at once: the one that loses the race to make the channel sends to the winner.
    for _ in 0..2 {
        if sys::send(&name, message)? {
            return Ok(Claim::Forwarded);
        }
        if let Some(channel) = sys::listen(&name)? {
            return Ok(Claim::Primary(Listener(channel)));
        }
    }
    Err(io::Error::other(
        "the single-instance channel is held but takes no messages",
    ))
}

/// The channel's full name: the app's, then the user's.
fn channel_name(name: &str) -> String {
    let user = std::env::var("USERNAME")
        .or_else(|_| std::env::var("USER"))
        .unwrap_or_default();
    let user: String = user
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    format!("{name}-{user}")
}

/// The first instance's end of the channel.
#[derive(Debug)]
pub struct Listener(sys::Channel);

impl Listener {
    /// Hands every later launch's message to `deliver`, on a thread of its own, for as long as the process runs.
    ///
    /// # Errors
    ///
    /// The thread could not start.
    pub fn serve(self, deliver: impl Fn(Vec<u8>) + Send + 'static) -> io::Result<()> {
        let channel = self.0;
        thread::Builder::new()
            .name("single-instance".to_owned())
            .spawn(move || {
                loop {
                    match channel.accept() {
                        Ok(mut stream) => {
                            let mut message = Vec::new();
                            match (&mut stream).take(MAX_MESSAGE).read_to_end(&mut message) {
                                Ok(_) => deliver(message),
                                Err(err) => {
                                    log::warn!("single instance: a launch's message: {err}")
                                }
                            }
                            channel.done(stream);
                        }
                        Err(err) => {
                            log::warn!("single instance: stopped listening: {err}");
                            return;
                        }
                    }
                }
            })?;
        Ok(())
    }
}

/// Writes `message` to a stream another launch opened, retrying while the channel is busy.
fn write_retrying<W: Write>(
    open: impl Fn() -> io::Result<Option<W>>,
    message: &[u8],
) -> io::Result<bool> {
    for _ in 0..BUSY_TRIES {
        match open() {
            Ok(Some(mut stream)) => {
                stream.write_all(message)?;
                stream.flush()?;
                return Ok(true);
            }
            Ok(None) => return Ok(false),
            Err(err) if is_busy(&err) => thread::sleep(Duration::from_millis(25)),
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::other("the running instance is busy"))
}

#[cfg(windows)]
fn is_busy(err: &io::Error) -> bool {
    // ERROR_PIPE_BUSY: every instance of the pipe is serving another launch.
    err.raw_os_error() == Some(231)
}

#[cfg(unix)]
fn is_busy(err: &io::Error) -> bool {
    err.kind() == io::ErrorKind::WouldBlock
}

#[cfg(windows)]
mod sys {
    //! A named pipe, `\\.\pipe\<name>`, inbound only, local clients only. Its default security lets only this user
    //! (and the system and administrators) write to it.

    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::io::{AsRawHandle, FromRawHandle};

    use windows::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_NO_DATA, ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE,
    };
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_INBOUND};
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE,
        PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };
    use windows::Win32::UI::WindowsAndMessaging::{ASFW_ANY, AllowSetForegroundWindow};
    use windows::core::HSTRING;

    use super::{MAX_MESSAGE, write_retrying};

    /// The pipe's one instance, owned as a file so it closes when dropped.
    #[derive(Debug)]
    pub(super) struct Channel(File);

    fn path(name: &str) -> String {
        format!(r"\\.\pipe\{name}")
    }

    /// Writes `message` to the pipe; `false` when no instance listens.
    pub(super) fn send(name: &str, message: &[u8]) -> io::Result<bool> {
        let path = path(name);
        let open = || match OpenOptions::new().write(true).open(&path) {
            Ok(file) => {
                // The running instance may bring its window to the front: this launch, started by the user, may.
                // SAFETY: no pointers; failure only means the window may flash instead of coming up.
                let _ = unsafe { AllowSetForegroundWindow(ASFW_ANY) };
                Ok(Some(file))
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(err) => Err(err),
        };
        write_retrying(open, message)
    }

    /// Makes the pipe; `None` when another process made it first.
    pub(super) fn listen(name: &str) -> io::Result<Option<Channel>> {
        let name = HSTRING::from(path(name));
        let size = u32::try_from(MAX_MESSAGE).unwrap_or(u32::MAX);
        // SAFETY: `name` is a valid wide string that outlives the call; the default security attributes are used.
        let handle = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_INBOUND | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                0,
                size,
                0,
                None,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let err = io::Error::last_os_error();
            // ERROR_ACCESS_DENIED: the pipe exists (FILE_FLAG_FIRST_PIPE_INSTANCE).
            return match err.raw_os_error() {
                Some(code) if code == ERROR_ACCESS_DENIED.0.cast_signed() => Ok(None),
                _ => Err(err),
            };
        }
        // SAFETY: `handle` is a valid pipe handle that nothing else owns.
        Ok(Some(Channel(unsafe { File::from_raw_handle(handle.0) })))
    }

    impl Channel {
        /// Waits for a launch to open the pipe; reading from the result reads its message.
        pub(super) fn accept(&self) -> io::Result<&File> {
            let handle = HANDLE(self.0.as_raw_handle());
            // SAFETY: `handle` is this channel's open pipe; synchronous, so no OVERLAPPED.
            match unsafe { ConnectNamedPipe(handle, None) } {
                Ok(()) => Ok(&self.0),
                // It connected between the pipe's making and this call.
                Err(err) if err.code() == ERROR_PIPE_CONNECTED.to_hresult() => Ok(&self.0),
                // It connected, wrote and closed before this call: its message is still in the pipe.
                Err(err) if err.code() == ERROR_NO_DATA.to_hresult() => Ok(&self.0),
                Err(err) => Err(io::Error::other(err)),
            }
        }

        /// Ends a launch's connection, so the next can open the pipe.
        pub(super) fn done(&self, _stream: &File) {
            // SAFETY: `handle` is this channel's open pipe.
            if let Err(err) = unsafe { DisconnectNamedPipe(HANDLE(self.0.as_raw_handle())) } {
                log::warn!("single instance: disconnect: {err}");
            }
        }
    }
}

#[cfg(unix)]
mod sys {
    //! A Unix socket in the user's runtime folder.

    use std::io;
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::PathBuf;

    use super::write_retrying;

    #[derive(Debug)]
    pub(super) struct Channel(UnixListener);

    fn path(name: &str) -> PathBuf {
        std::env::var_os("XDG_RUNTIME_DIR")
            .map_or_else(std::env::temp_dir, PathBuf::from)
            .join(format!("{name}.sock"))
    }

    /// Writes `message` to the socket; `false` when no instance listens.
    pub(super) fn send(name: &str, message: &[u8]) -> io::Result<bool> {
        let path = path(name);
        let open = || match UnixStream::connect(&path) {
            Ok(stream) => Ok(Some(stream)),
            Err(err)
                if matches!(
                    err.kind(),
                    io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
                ) =>
            {
                Ok(None)
            }
            Err(err) => Err(err),
        };
        write_retrying(open, message)
    }

    /// Makes the socket, replacing one an ended instance left.
    ///
    /// shortcut: two first launches in the same instant can both replace the socket; the later one wins and the other
    /// listens on nothing. Lock a file beside it if that is ever seen.
    pub(super) fn listen(name: &str) -> io::Result<Option<Channel>> {
        let path = path(name);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        Ok(Some(Channel(UnixListener::bind(path)?)))
    }

    impl Channel {
        pub(super) fn accept(&self) -> io::Result<UnixStream> {
            self.0.accept().map(|(stream, _)| stream)
        }

        pub(super) fn done(&self, _stream: UnixStream) {}
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn a_second_launch_hands_its_message_to_the_first() {
        let name = format!("ExcellentPDF-test-{}", std::process::id());
        let Claim::Primary(listener) = claim(&name, b"first").expect("claim") else {
            panic!("the first launch must listen");
        };
        let (tx, rx) = mpsc::channel();
        listener
            .serve(move |message| tx.send(message).expect("the test waits"))
            .expect("serve");
        for message in [&b"second"[..], b"third"] {
            assert!(matches!(
                claim(&name, message).expect("claim"),
                Claim::Forwarded
            ));
            let got = rx.recv_timeout(Duration::from_secs(5)).expect("delivered");
            assert_eq!(got, message);
        }
    }

    #[test]
    fn the_channel_is_per_user() {
        let name = channel_name("ExcellentPDF");
        assert!(name.starts_with("ExcellentPDF-"));
        assert!(!name.contains(['\\', '/']));
    }
}
