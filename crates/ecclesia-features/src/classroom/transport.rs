//! Classroom LAN transport abstractions: in-memory deterministic bus, local TCP fan-out server,
//! and HTTP relay fallback via `ureq` (plan G.14, G.15).

use std::collections::VecDeque;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::ClassroomError;
use super::protocol::ClassroomPacket;

/// Mockable transport abstraction for classroom host-student message exchange.
pub trait ClassroomTransport: Send {
    /// Sends a packet from the host to connected students, or from a student to the host.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if transport delivery fails.
    fn send(&mut self, packet: &ClassroomPacket) -> Result<(), ClassroomError>;

    /// Drains currently available incoming packets without blocking indefinitely.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError`] if reading or decoding fails.
    fn drain(&mut self) -> Result<Vec<ClassroomPacket>, ClassroomError>;
}

/// Shared in-memory bus connecting one teacher host endpoint with any number of student endpoints.
#[derive(Debug, Default, Clone)]
pub struct InMemoryClassroomBus {
    inner: Arc<Mutex<BusState>>,
}

#[derive(Debug, Default)]
struct BusState {
    host_inbox: VecDeque<ClassroomPacket>,
    student_inboxes: Vec<VecDeque<ClassroomPacket>>,
}

impl InMemoryClassroomBus {
    /// Creates a new in-memory classroom bus.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates the teacher host transport handle.
    #[must_use]
    pub fn host_transport(&self) -> InMemoryHostTransport {
        InMemoryHostTransport {
            inner: Arc::clone(&self.inner),
        }
    }

    /// Registers and returns a new student transport handle.
    #[must_use]
    pub fn connect_student(&self) -> InMemoryStudentTransport {
        let mut guard = self.inner.lock().expect("classroom bus lock poisoned");
        let idx = guard.student_inboxes.len();
        guard.student_inboxes.push(VecDeque::new());
        InMemoryStudentTransport {
            inner: Arc::clone(&self.inner),
            student_slot: idx,
        }
    }
}

/// Teacher host handle over [`InMemoryClassroomBus`].
#[derive(Debug, Clone)]
pub struct InMemoryHostTransport {
    inner: Arc<Mutex<BusState>>,
}

impl ClassroomTransport for InMemoryHostTransport {
    fn send(&mut self, packet: &ClassroomPacket) -> Result<(), ClassroomError> {
        let mut guard = self.inner.lock().expect("classroom bus lock poisoned");
        for inbox in &mut guard.student_inboxes {
            inbox.push_back(packet.clone());
        }
        Ok(())
    }

    fn drain(&mut self) -> Result<Vec<ClassroomPacket>, ClassroomError> {
        let mut guard = self.inner.lock().expect("classroom bus lock poisoned");
        Ok(guard.host_inbox.drain(..).collect())
    }
}

/// Student handle over [`InMemoryClassroomBus`].
#[derive(Debug, Clone)]
pub struct InMemoryStudentTransport {
    inner: Arc<Mutex<BusState>>,
    student_slot: usize,
}

impl ClassroomTransport for InMemoryStudentTransport {
    fn send(&mut self, packet: &ClassroomPacket) -> Result<(), ClassroomError> {
        let mut guard = self.inner.lock().expect("classroom bus lock poisoned");
        guard.host_inbox.push_back(packet.clone());
        Ok(())
    }

    fn drain(&mut self) -> Result<Vec<ClassroomPacket>, ClassroomError> {
        let mut guard = self.inner.lock().expect("classroom bus lock poisoned");
        Ok(guard
            .student_inboxes
            .get_mut(self.student_slot)
            .map(|q| q.drain(..).collect())
            .unwrap_or_default())
    }
}

/// Non-blocking local TCP broadcast server for LAN classroom sessions (plan G.14).
#[derive(Debug)]
pub struct TcpClassroomServer {
    listener: TcpListener,
    local_addr: SocketAddr,
    clients: Vec<TcpStream>,
}

impl TcpClassroomServer {
    /// Binds a LAN TCP classroom listener on `bind_addr` (use `"127.0.0.1:0"` for ephemeral port).
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Io`] if binding fails.
    pub fn bind(bind_addr: &str) -> Result<Self, ClassroomError> {
        let listener = TcpListener::bind(bind_addr)?;
        listener.set_nonblocking(true)?;
        let local_addr = listener.local_addr()?;
        Ok(Self {
            listener,
            local_addr,
            clients: Vec::new(),
        })
    }

    /// Bound local socket address.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Accepts any pending student TCP connections without blocking.
    pub fn accept_pending(&mut self) {
        while let Ok((stream, _)) = self.listener.accept() {
            let _ = stream.set_nonblocking(true);
            let _ = stream.set_nodelay(true);
            self.clients.push(stream);
        }
    }
}

impl ClassroomTransport for TcpClassroomServer {
    fn send(&mut self, packet: &ClassroomPacket) -> Result<(), ClassroomError> {
        self.accept_pending();
        let line = packet.to_json_line()?;
        let payload = format!("{line}\n");
        self.clients.retain_mut(|stream| {
            stream.write_all(payload.as_bytes()).is_ok() && stream.flush().is_ok()
        });
        Ok(())
    }

    fn drain(&mut self) -> Result<Vec<ClassroomPacket>, ClassroomError> {
        self.accept_pending();
        let mut out = Vec::new();
        for stream in &mut self.clients {
            let mut reader = BufReader::new(stream);
            let mut buf = String::new();
            while let Ok(n) = reader.read_line(&mut buf) {
                if n == 0 {
                    break;
                }
                let trimmed = buf.trim();
                if !trimmed.is_empty()
                    && let Ok(pkt) = ClassroomPacket::from_json_line(trimmed)
                {
                    out.push(pkt);
                }
                buf.clear();
            }
        }
        Ok(out)
    }
}

/// Student TCP client connecting to a [`TcpClassroomServer`] on the LAN (plan G.14).
#[derive(Debug)]
pub struct TcpClassroomClient {
    stream: TcpStream,
    read_buf: String,
}

impl TcpClassroomClient {
    /// Connects to a LAN classroom host at `addr`.
    ///
    /// # Errors
    ///
    /// Returns [`ClassroomError::Io`] if connecting fails.
    pub fn connect(addr: SocketAddr) -> Result<Self, ClassroomError> {
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
        stream.set_nonblocking(true)?;
        stream.set_nodelay(true)?;
        Ok(Self {
            stream,
            read_buf: String::new(),
        })
    }
}

impl ClassroomTransport for TcpClassroomClient {
    fn send(&mut self, packet: &ClassroomPacket) -> Result<(), ClassroomError> {
        let line = packet.to_json_line()?;
        self.stream.write_all(format!("{line}\n").as_bytes())?;
        self.stream.flush()?;
        Ok(())
    }

    fn drain(&mut self) -> Result<Vec<ClassroomPacket>, ClassroomError> {
        let mut out = Vec::new();
        let mut tmp = [0_u8; 4096];
        loop {
            match std::io::Read::read(&mut self.stream, &mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(text) = std::str::from_utf8(&tmp[..n]) {
                        self.read_buf.push_str(text);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(e) => return Err(ClassroomError::Io(e)),
            }
        }
        while let Some(pos) = self.read_buf.find('\n') {
            let line = self.read_buf[..pos].trim().to_owned();
            self.read_buf.drain(..=pos);
            if !line.is_empty() {
                out.push(ClassroomPacket::from_json_line(&line)?);
            }
        }
        Ok(out)
    }
}

/// HTTP relay fallback transport backed by `ureq` for off-LAN classroom sessions (plan G.14).
#[derive(Debug, Clone)]
pub struct RelayHttpTransport {
    base_url: String,
    session_code: String,
    cursor_seq: u64,
}

impl RelayHttpTransport {
    /// Creates an HTTP relay transport pointing at `base_url` for `session_code`.
    #[must_use]
    pub fn new(base_url: &str, session_code: &str) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_owned(),
            session_code: session_code.to_owned(),
            cursor_seq: 0,
        }
    }
}

impl ClassroomTransport for RelayHttpTransport {
    fn send(&mut self, packet: &ClassroomPacket) -> Result<(), ClassroomError> {
        let url = format!("{}/classroom/{}/publish", self.base_url, self.session_code);
        let body = packet.to_json_line()?;
        ureq::post(&url)
            .header("Content-Type", "application/json")
            .send(body.as_bytes())
            .map_err(|e| ClassroomError::Protocol(format!("relay publish failed: {e}")))?;
        Ok(())
    }

    fn drain(&mut self) -> Result<Vec<ClassroomPacket>, ClassroomError> {
        let url = format!(
            "{}/classroom/{}/poll?since={}",
            self.base_url, self.session_code, self.cursor_seq
        );
        let mut resp = ureq::get(&url)
            .call()
            .map_err(|e| ClassroomError::Protocol(format!("relay poll failed: {e}")))?;
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| ClassroomError::Protocol(format!("relay read failed: {e}")))?;
        let mut packets = Vec::new();
        for line in text.lines() {
            let trimmed = line.trim();
            if !trimmed.is_empty() {
                packets.push(ClassroomPacket::from_json_line(trimmed)?);
                self.cursor_seq += 1;
            }
        }
        Ok(packets)
    }
}
