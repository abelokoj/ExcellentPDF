//! Bounded length-prefixed IPC protocol between the host and sandboxed PDF workers (plan K.22).
//!
//! Each message travels as an [`IpcFrame`]:
//! - 4-byte big-endian magic (`"EXPC"`, [`IPC_MAGIC`])
//! - 4-byte big-endian JSON header byte length
//! - 4-byte big-endian binary payload byte length (for PDF bytes or rendered tile pixels)
//! - UTF-8 JSON header bytes
//! - Raw binary payload bytes
//!
//! Frame lengths are validated against [`SecurityLimits::max_ipc_frame_bytes`] before allocating
//! buffers so a compromised worker cannot trigger an OOM in the UI process.

use std::fmt;
use std::io::{self, Read, Write};

use serde_json::{Value, json};

use super::limits::SecurityLimits;

/// 4-byte magic identifier (`"EXPC"`) starting every [`IpcFrame`].
pub const IPC_MAGIC: u32 = 0x4558_5043;

/// Size of the fixed [`IpcFrame`] header in bytes (`magic` + `json_len` + `blob_len`).
const FRAME_HEADER_BYTES: usize = 12;

/// Request sent from the host process to a sandboxed document worker.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum IpcRequest {
    /// Liveness check carrying an echo sequence number.
    Ping {
        /// Monotonic probe sequence.
        sequence: u64,
    },
    /// Opens an in-memory document inside the worker under `limits`.
    OpenBytes {
        /// Host-assigned document handle id.
        doc_id: u64,
        /// MIME type or format hint (for example `"application/pdf"`).
        mime: String,
        /// Resource caps enforced by the worker while opening and serving the document.
        limits: SecurityLimits,
        /// Raw document bytes carried in the binary frame blob.
        bytes: Vec<u8>,
    },
    /// Renders a rectangular tile of `page` inside the worker.
    RenderTile {
        /// Open document handle id.
        doc_id: u64,
        /// Zero-based page index.
        page: usize,
        /// Left pixel coordinate.
        x0: i32,
        /// Top pixel coordinate.
        y0: i32,
        /// Exclusive right pixel coordinate.
        x1: i32,
        /// Exclusive bottom pixel coordinate.
        y1: i32,
        /// Horizontal render scale.
        scale_x: f32,
        /// Vertical render scale.
        scale_y: f32,
    },
    /// Extracts normalized UTF-8 page text inside the worker.
    ExtractText {
        /// Open document handle id.
        doc_id: u64,
        /// Zero-based page index.
        page: usize,
    },
    /// Replays serialized K.14 operation log records after a worker crash restart.
    ApplyReplayLog {
        /// Open document handle id.
        doc_id: u64,
        /// JSON-serialized operation records to replay in order.
        operations_json: Vec<String>,
    },
    /// Closes `doc_id` inside the worker and releases its pages.
    CloseDocument {
        /// Open document handle id.
        doc_id: u64,
    },
    /// Requests a clean worker shutdown.
    Shutdown,
}

/// Response returned from a sandboxed worker to the host process.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum IpcResponse {
    /// Response to [`IpcRequest::Ping`].
    Pong {
        /// Echoed probe sequence.
        sequence: u64,
        /// OS process id of the worker (or host pid in fallback mode).
        worker_pid: u32,
    },
    /// Response to [`IpcRequest::OpenBytes`].
    Opened {
        /// Open document handle id.
        doc_id: u64,
        /// Total page count reported by the worker.
        page_count: usize,
    },
    /// Response to [`IpcRequest::RenderTile`], carrying raw pixel samples in `samples`.
    TileRendered {
        /// Open document handle id.
        doc_id: u64,
        /// Zero-based page index.
        page: usize,
        /// Tile width in pixels.
        width: u32,
        /// Tile height in pixels.
        height: u32,
        /// Color components per pixel (3 for RGB, 4 for RGBA).
        components: u8,
        /// Byte stride per row.
        stride: usize,
        /// Raw pixel bytes carried in the binary frame blob.
        samples: Vec<u8>,
    },
    /// Response to [`IpcRequest::ExtractText`].
    TextExtracted {
        /// Open document handle id.
        doc_id: u64,
        /// Zero-based page index.
        page: usize,
        /// Extracted UTF-8 text.
        text: String,
    },
    /// Response to [`IpcRequest::ApplyReplayLog`].
    ReplayApplied {
        /// Open document handle id.
        doc_id: u64,
        /// Number of operations replayed.
        applied_count: usize,
    },
    /// Response to [`IpcRequest::CloseDocument`].
    Closed {
        /// Closed document handle id.
        doc_id: u64,
    },
    /// The worker rejected the request due to a [`super::limits::SecurityLimits`] breach.
    LimitRejected {
        /// Human-readable description of the violated limit.
        violation: String,
    },
    /// The worker encountered an engine or protocol error while handling the request.
    WorkerError {
        /// Error detail.
        message: String,
    },
    /// Response to [`IpcRequest::Shutdown`].
    ShuttingDown,
}

/// Wire-level bounded frame carrying a JSON header and an optional binary payload blob.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IpcFrame {
    /// UTF-8 JSON header bytes.
    pub json: Vec<u8>,
    /// Optional binary payload bytes.
    pub blob: Vec<u8>,
}

impl IpcFrame {
    /// Writes this frame into `writer` after verifying total length fits `max_frame_bytes`.
    ///
    /// # Errors
    ///
    /// Returns [`IpcCodecError::FrameTooLarge`] if the encoded frame exceeds `max_frame_bytes`, or
    /// [`IpcCodecError::Io`] on write failure.
    pub fn write_to<W: Write>(
        &self,
        writer: &mut W,
        max_frame_bytes: usize,
    ) -> Result<(), IpcCodecError> {
        let total = self.json.len().saturating_add(self.blob.len());
        if total > max_frame_bytes {
            return Err(IpcCodecError::FrameTooLarge {
                actual: total,
                limit: max_frame_bytes,
            });
        }
        let json_len =
            u32::try_from(self.json.len()).map_err(|_| IpcCodecError::FrameTooLarge {
                actual: self.json.len(),
                limit: max_frame_bytes,
            })?;
        let blob_len =
            u32::try_from(self.blob.len()).map_err(|_| IpcCodecError::FrameTooLarge {
                actual: self.blob.len(),
                limit: max_frame_bytes,
            })?;
        let mut header = [0u8; FRAME_HEADER_BYTES];
        header[0..4].copy_from_slice(&IPC_MAGIC.to_be_bytes());
        header[4..8].copy_from_slice(&json_len.to_be_bytes());
        header[8..12].copy_from_slice(&blob_len.to_be_bytes());
        writer.write_all(&header)?;
        writer.write_all(&self.json)?;
        writer.write_all(&self.blob)?;
        writer.flush()?;
        Ok(())
    }

    /// Reads one frame from `reader`, rejecting oversized or malformed headers before allocating.
    ///
    /// # Errors
    ///
    /// Returns [`IpcCodecError`] on EOF, magic mismatch, oversized length, or I/O failure.
    pub fn read_from<R: Read>(
        reader: &mut R,
        max_frame_bytes: usize,
    ) -> Result<Self, IpcCodecError> {
        let mut header = [0u8; FRAME_HEADER_BYTES];
        reader.read_exact(&mut header)?;
        let magic = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        if magic != IPC_MAGIC {
            return Err(IpcCodecError::BadMagic(magic));
        }
        let json_len = u32::from_be_bytes([header[4], header[5], header[6], header[7]]) as usize;
        let blob_len = u32::from_be_bytes([header[8], header[9], header[10], header[11]]) as usize;
        let total = json_len.saturating_add(blob_len);
        if total > max_frame_bytes {
            return Err(IpcCodecError::FrameTooLarge {
                actual: total,
                limit: max_frame_bytes,
            });
        }
        let mut json = vec![0u8; json_len];
        reader.read_exact(&mut json)?;
        let mut blob = vec![0u8; blob_len];
        reader.read_exact(&mut blob)?;
        Ok(Self { json, blob })
    }
}

/// Errors produced when encoding or decoding IPC frames and messages.
#[derive(Debug)]
#[non_exhaustive]
#[must_use]
pub enum IpcCodecError {
    /// Underlying stream read or write error (including unexpected EOF on worker crash).
    Io(io::Error),
    /// Frame header did not start with [`IPC_MAGIC`].
    BadMagic(u32),
    /// Combined frame payload exceeded `max_ipc_frame_bytes`.
    FrameTooLarge {
        /// Declared or actual payload size in bytes.
        actual: usize,
        /// Configured ceiling in bytes.
        limit: usize,
    },
    /// JSON header failed to parse or lacked required fields.
    MalformedMessage(String),
}

impl fmt::Display for IpcCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "ipc i/o error: {err}"),
            Self::BadMagic(magic) => write!(f, "bad ipc magic {magic:#010x}"),
            Self::FrameTooLarge { actual, limit } => {
                write!(f, "ipc frame size {actual} bytes exceeds limit {limit}")
            }
            Self::MalformedMessage(detail) => write!(f, "malformed ipc message: {detail}"),
        }
    }
}

impl std::error::Error for IpcCodecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(err) => Some(err),
            Self::BadMagic(_) | Self::FrameTooLarge { .. } | Self::MalformedMessage(_) => None,
        }
    }
}

impl From<io::Error> for IpcCodecError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Encodes and writes `request` to `writer` within `max_frame_bytes`.
///
/// # Errors
///
/// Returns [`IpcCodecError`] on serialization, framing limit, or I/O failure.
pub fn write_request<W: Write>(
    writer: &mut W,
    request: &IpcRequest,
    max_frame_bytes: usize,
) -> Result<(), IpcCodecError> {
    let frame = encode_request(request)?;
    frame.write_to(writer, max_frame_bytes)
}

/// Reads and decodes one [`IpcRequest`] from `reader` within `max_frame_bytes`.
///
/// # Errors
///
/// Returns [`IpcCodecError`] on I/O, framing limit, or JSON schema failure.
pub fn read_request<R: Read>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<IpcRequest, IpcCodecError> {
    let frame = IpcFrame::read_from(reader, max_frame_bytes)?;
    decode_request(&frame)
}

/// Encodes and writes `response` to `writer` within `max_frame_bytes`.
///
/// # Errors
///
/// Returns [`IpcCodecError`] on serialization, framing limit, or I/O failure.
pub fn write_response<W: Write>(
    writer: &mut W,
    response: &IpcResponse,
    max_frame_bytes: usize,
) -> Result<(), IpcCodecError> {
    let frame = encode_response(response)?;
    frame.write_to(writer, max_frame_bytes)
}

/// Reads and decodes one [`IpcResponse`] from `reader` within `max_frame_bytes`.
///
/// # Errors
///
/// Returns [`IpcCodecError`] on I/O, framing limit, or JSON schema failure.
pub fn read_response<R: Read>(
    reader: &mut R,
    max_frame_bytes: usize,
) -> Result<IpcResponse, IpcCodecError> {
    let frame = IpcFrame::read_from(reader, max_frame_bytes)?;
    decode_response(&frame)
}

fn encode_request(request: &IpcRequest) -> Result<IpcFrame, IpcCodecError> {
    let (value, blob) = match request {
        IpcRequest::Ping { sequence } => (json!({"op": "ping", "sequence": sequence}), Vec::new()),
        IpcRequest::OpenBytes {
            doc_id,
            mime,
            limits,
            bytes,
        } => (
            json!({
                "op": "open_bytes",
                "doc_id": doc_id,
                "mime": mime,
                "limits": {
                    "max_pages": limits.max_pages,
                    "max_recursion_depth": limits.max_recursion_depth,
                    "max_decompressed_bytes": limits.max_decompressed_bytes,
                    "max_xref_entries": limits.max_xref_entries,
                    "max_compression_ratio": limits.max_compression_ratio,
                    "ratio_check_floor_bytes": limits.ratio_check_floor_bytes,
                    "max_image_pixels": limits.max_image_pixels,
                    "max_object_count": limits.max_object_count,
                    "max_input_bytes": limits.max_input_bytes,
                    "max_ipc_frame_bytes": limits.max_ipc_frame_bytes,
                    "worker_memory_cap_bytes": limits.worker_memory_cap_bytes,
                    "worker_timeout_ms": limits.worker_timeout.as_millis() as u64,
                }
            }),
            bytes.clone(),
        ),
        IpcRequest::RenderTile {
            doc_id,
            page,
            x0,
            y0,
            x1,
            y1,
            scale_x,
            scale_y,
        } => (
            json!({
                "op": "render_tile",
                "doc_id": doc_id,
                "page": page,
                "x0": x0,
                "y0": y0,
                "x1": x1,
                "y1": y1,
                "scale_x": scale_x,
                "scale_y": scale_y,
            }),
            Vec::new(),
        ),
        IpcRequest::ExtractText { doc_id, page } => (
            json!({
                "op": "extract_text",
                "doc_id": doc_id,
                "page": page,
            }),
            Vec::new(),
        ),
        IpcRequest::ApplyReplayLog {
            doc_id,
            operations_json,
        } => (
            json!({
                "op": "apply_replay_log",
                "doc_id": doc_id,
                "operations_json": operations_json,
            }),
            Vec::new(),
        ),
        IpcRequest::CloseDocument { doc_id } => (
            json!({
                "op": "close_document",
                "doc_id": doc_id,
            }),
            Vec::new(),
        ),
        IpcRequest::Shutdown => (json!({"op": "shutdown"}), Vec::new()),
    };
    let json = serde_json::to_vec(&value)
        .map_err(|err| IpcCodecError::MalformedMessage(err.to_string()))?;
    Ok(IpcFrame { json, blob })
}

fn decode_request(frame: &IpcFrame) -> Result<IpcRequest, IpcCodecError> {
    let val: Value = serde_json::from_slice(&frame.json)
        .map_err(|err| IpcCodecError::MalformedMessage(err.to_string()))?;
    let op = req_str(&val, "op")?;
    match op {
        "ping" => Ok(IpcRequest::Ping {
            sequence: req_u64(&val, "sequence")?,
        }),
        "open_bytes" => {
            let lim = val
                .get("limits")
                .ok_or_else(|| IpcCodecError::MalformedMessage("missing 'limits'".to_owned()))?;
            let timeout_ms = req_u64(lim, "worker_timeout_ms")?;
            let limits = SecurityLimits {
                max_pages: req_usize(lim, "max_pages")?,
                max_recursion_depth: req_usize(lim, "max_recursion_depth")?,
                max_decompressed_bytes: req_u64(lim, "max_decompressed_bytes")?,
                max_xref_entries: req_usize(lim, "max_xref_entries")?,
                max_compression_ratio: req_u64(lim, "max_compression_ratio")?,
                ratio_check_floor_bytes: req_u64(lim, "ratio_check_floor_bytes")?,
                max_image_pixels: req_u64(lim, "max_image_pixels")?,
                max_object_count: req_usize(lim, "max_object_count")?,
                max_input_bytes: req_u64(lim, "max_input_bytes")?,
                max_ipc_frame_bytes: req_usize(lim, "max_ipc_frame_bytes")?,
                worker_memory_cap_bytes: req_u64(lim, "worker_memory_cap_bytes")?,
                worker_timeout: std::time::Duration::from_millis(timeout_ms),
            };
            Ok(IpcRequest::OpenBytes {
                doc_id: req_u64(&val, "doc_id")?,
                mime: req_str(&val, "mime")?.to_owned(),
                limits,
                bytes: frame.blob.clone(),
            })
        }
        "render_tile" => Ok(IpcRequest::RenderTile {
            doc_id: req_u64(&val, "doc_id")?,
            page: req_usize(&val, "page")?,
            x0: req_i32(&val, "x0")?,
            y0: req_i32(&val, "y0")?,
            x1: req_i32(&val, "x1")?,
            y1: req_i32(&val, "y1")?,
            scale_x: req_f32(&val, "scale_x")?,
            scale_y: req_f32(&val, "scale_y")?,
        }),
        "extract_text" => Ok(IpcRequest::ExtractText {
            doc_id: req_u64(&val, "doc_id")?,
            page: req_usize(&val, "page")?,
        }),
        "apply_replay_log" => {
            let arr = val
                .get("operations_json")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    IpcCodecError::MalformedMessage("missing 'operations_json' array".to_owned())
                })?;
            let mut ops = Vec::with_capacity(arr.len());
            for item in arr {
                let s = item.as_str().ok_or_else(|| {
                    IpcCodecError::MalformedMessage("operation entry must be string".to_owned())
                })?;
                ops.push(s.to_owned());
            }
            Ok(IpcRequest::ApplyReplayLog {
                doc_id: req_u64(&val, "doc_id")?,
                operations_json: ops,
            })
        }
        "close_document" => Ok(IpcRequest::CloseDocument {
            doc_id: req_u64(&val, "doc_id")?,
        }),
        "shutdown" => Ok(IpcRequest::Shutdown),
        other => Err(IpcCodecError::MalformedMessage(format!(
            "unknown request op '{other}'"
        ))),
    }
}

fn encode_response(response: &IpcResponse) -> Result<IpcFrame, IpcCodecError> {
    let (value, blob) = match response {
        IpcResponse::Pong {
            sequence,
            worker_pid,
        } => (
            json!({
                "status": "pong",
                "sequence": sequence,
                "worker_pid": worker_pid,
            }),
            Vec::new(),
        ),
        IpcResponse::Opened { doc_id, page_count } => (
            json!({
                "status": "opened",
                "doc_id": doc_id,
                "page_count": page_count,
            }),
            Vec::new(),
        ),
        IpcResponse::TileRendered {
            doc_id,
            page,
            width,
            height,
            components,
            stride,
            samples,
        } => (
            json!({
                "status": "tile_rendered",
                "doc_id": doc_id,
                "page": page,
                "width": width,
                "height": height,
                "components": components,
                "stride": stride,
            }),
            samples.clone(),
        ),
        IpcResponse::TextExtracted { doc_id, page, text } => (
            json!({
                "status": "text_extracted",
                "doc_id": doc_id,
                "page": page,
                "text": text,
            }),
            Vec::new(),
        ),
        IpcResponse::ReplayApplied {
            doc_id,
            applied_count,
        } => (
            json!({
                "status": "replay_applied",
                "doc_id": doc_id,
                "applied_count": applied_count,
            }),
            Vec::new(),
        ),
        IpcResponse::Closed { doc_id } => (
            json!({
                "status": "closed",
                "doc_id": doc_id,
            }),
            Vec::new(),
        ),
        IpcResponse::LimitRejected { violation } => (
            json!({
                "status": "limit_rejected",
                "violation": violation,
            }),
            Vec::new(),
        ),
        IpcResponse::WorkerError { message } => (
            json!({
                "status": "worker_error",
                "message": message,
            }),
            Vec::new(),
        ),
        IpcResponse::ShuttingDown => (json!({"status": "shutting_down"}), Vec::new()),
    };
    let json = serde_json::to_vec(&value)
        .map_err(|err| IpcCodecError::MalformedMessage(err.to_string()))?;
    Ok(IpcFrame { json, blob })
}

fn decode_response(frame: &IpcFrame) -> Result<IpcResponse, IpcCodecError> {
    let val: Value = serde_json::from_slice(&frame.json)
        .map_err(|err| IpcCodecError::MalformedMessage(err.to_string()))?;
    let status = req_str(&val, "status")?;
    match status {
        "pong" => Ok(IpcResponse::Pong {
            sequence: req_u64(&val, "sequence")?,
            worker_pid: u32::try_from(req_u64(&val, "worker_pid")?)
                .map_err(|_| IpcCodecError::MalformedMessage("worker_pid overflow".to_owned()))?,
        }),
        "opened" => Ok(IpcResponse::Opened {
            doc_id: req_u64(&val, "doc_id")?,
            page_count: req_usize(&val, "page_count")?,
        }),
        "tile_rendered" => Ok(IpcResponse::TileRendered {
            doc_id: req_u64(&val, "doc_id")?,
            page: req_usize(&val, "page")?,
            width: u32::try_from(req_u64(&val, "width")?)
                .map_err(|_| IpcCodecError::MalformedMessage("width overflow".to_owned()))?,
            height: u32::try_from(req_u64(&val, "height")?)
                .map_err(|_| IpcCodecError::MalformedMessage("height overflow".to_owned()))?,
            components: u8::try_from(req_u64(&val, "components")?)
                .map_err(|_| IpcCodecError::MalformedMessage("components overflow".to_owned()))?,
            stride: req_usize(&val, "stride")?,
            samples: frame.blob.clone(),
        }),
        "text_extracted" => Ok(IpcResponse::TextExtracted {
            doc_id: req_u64(&val, "doc_id")?,
            page: req_usize(&val, "page")?,
            text: req_str(&val, "text")?.to_owned(),
        }),
        "replay_applied" => Ok(IpcResponse::ReplayApplied {
            doc_id: req_u64(&val, "doc_id")?,
            applied_count: req_usize(&val, "applied_count")?,
        }),
        "closed" => Ok(IpcResponse::Closed {
            doc_id: req_u64(&val, "doc_id")?,
        }),
        "limit_rejected" => Ok(IpcResponse::LimitRejected {
            violation: req_str(&val, "violation")?.to_owned(),
        }),
        "worker_error" => Ok(IpcResponse::WorkerError {
            message: req_str(&val, "message")?.to_owned(),
        }),
        "shutting_down" => Ok(IpcResponse::ShuttingDown),
        other => Err(IpcCodecError::MalformedMessage(format!(
            "unknown response status '{other}'"
        ))),
    }
}

fn req_str<'a>(val: &'a Value, key: &str) -> Result<&'a str, IpcCodecError> {
    val.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| IpcCodecError::MalformedMessage(format!("missing string '{key}'")))
}

fn req_u64(val: &Value, key: &str) -> Result<u64, IpcCodecError> {
    val.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| IpcCodecError::MalformedMessage(format!("missing u64 '{key}'")))
}

fn req_usize(val: &Value, key: &str) -> Result<usize, IpcCodecError> {
    let raw = req_u64(val, key)?;
    usize::try_from(raw)
        .map_err(|_| IpcCodecError::MalformedMessage(format!("'{key}' overflows usize")))
}

fn req_i32(val: &Value, key: &str) -> Result<i32, IpcCodecError> {
    let raw = val
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| IpcCodecError::MalformedMessage(format!("missing i64 '{key}'")))?;
    i32::try_from(raw)
        .map_err(|_| IpcCodecError::MalformedMessage(format!("'{key}' overflows i32")))
}

fn req_f32(val: &Value, key: &str) -> Result<f32, IpcCodecError> {
    let raw = val
        .get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| IpcCodecError::MalformedMessage(format!("missing f64 '{key}'")))?;
    Ok(raw as f32)
}
