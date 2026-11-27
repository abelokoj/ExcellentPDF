//! Shared pure-Rust PDF 1.7 parser, stream codec, and writer for `ecclesia-core` document tools.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::{self, Write as _};
use std::io::{Read as _, Write as _};

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;

/// Error returned when parsing or transforming raw PDF bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
#[non_exhaustive]
pub enum PdfDocError {
    /// The input bytes are not a valid PDF structure.
    InvalidPdf(String),
    /// A requested page or object index is out of bounds.
    OutOfRange(String),
    /// A stream could not be compressed or decompressed.
    StreamCodec(String),
    /// An operation is not valid for the current document state.
    InvalidOperation(String),
}

impl fmt::Display for PdfDocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidPdf(msg) => write!(f, "invalid PDF structure: {msg}"),
            Self::OutOfRange(msg) => write!(f, "index out of range: {msg}"),
            Self::StreamCodec(msg) => write!(f, "PDF stream codec error: {msg}"),
            Self::InvalidOperation(msg) => write!(f, "invalid PDF operation: {msg}"),
        }
    }
}

impl std::error::Error for PdfDocError {}

/// Convenience result alias for PDF byte-level operations.
pub(crate) type Result<T> = std::result::Result<T, PdfDocError>;

/// Whether a stream should be Flate-compressed when serialized.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamEncoding {
    Raw,
    Flate(Compression),
}

/// A PDF object value.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PdfVal {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    HexStr(Vec<u8>),
    Array(Vec<PdfVal>),
    Dict(BTreeMap<String, PdfVal>),
    Ref(u32, u16),
    Stream {
        dict: BTreeMap<String, PdfVal>,
        data: Vec<u8>,
    },
}

impl PdfVal {
    pub(crate) fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Int(n) => Some(*n as f32),
            Self::Real(f) => Some(*f as f32),
            _ => None,
        }
    }

    pub(crate) fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Int(n) => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_ref_id(&self) -> Option<u32> {
        match self {
            Self::Ref(id, _) => Some(*id),
            _ => None,
        }
    }

    pub(crate) fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Str(b) | Self::HexStr(b) => Some(b),
            _ => None,
        }
    }

    pub(crate) fn as_text_string(&self) -> Option<String> {
        let bytes = self.as_bytes()?;
        if bytes.starts_with(&[0xFE, 0xFF]) {
            let units: Vec<u16> = bytes[2..]
                .as_chunks::<2>()
                .0
                .iter()
                .map(|c| u16::from_be_bytes(*c))
                .collect();
            Some(String::from_utf16_lossy(&units))
        } else {
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
    }

    pub(crate) fn as_rect(&self) -> Option<[f32; 4]> {
        let Self::Array(arr) = self else {
            return None;
        };
        if arr.len() != 4 {
            return None;
        }
        Some([
            arr[0].as_f32()?,
            arr[1].as_f32()?,
            arr[2].as_f32()?,
            arr[3].as_f32()?,
        ])
    }

    pub(crate) fn as_dict(&self) -> Option<&BTreeMap<String, PdfVal>> {
        match self {
            Self::Dict(d) | Self::Stream { dict: d, .. } => Some(d),
            _ => None,
        }
    }
}

pub(crate) fn rect_val(r: [f32; 4]) -> PdfVal {
    PdfVal::Array(r.into_iter().map(|v| PdfVal::Real(f64::from(v))).collect())
}

pub(crate) fn fmt_f32(v: f32) -> String {
    if (v - v.round()).abs() < 1e-4 {
        format!("{}", v.round() as i64)
    } else {
        let s = format!("{v:.4}");
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        if trimmed == "-0" {
            "0".to_owned()
        } else {
            trimmed.to_owned()
        }
    }
}

pub(crate) fn escape_pdf_string(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '(' => out.push_str("\\("),
            ')' => out.push_str("\\)"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_ascii() => out.push(c),
            c => {
                let mut buf = [0u8; 4];
                for &b in c.encode_utf8(&mut buf).as_bytes() {
                    let _ = write!(out, "\\{b:03o}");
                }
            }
        }
    }
    out
}

pub(crate) fn deflate_bytes(data: &[u8], level: Compression) -> Result<Vec<u8>> {
    let mut encoder = ZlibEncoder::new(Vec::new(), level);
    encoder
        .write_all(data)
        .map_err(|e| PdfDocError::StreamCodec(e.to_string()))?;
    encoder
        .finish()
        .map_err(|e| PdfDocError::StreamCodec(e.to_string()))
}

pub(crate) fn inflate_bytes(data: &[u8]) -> Result<Vec<u8>> {
    let mut decoder = ZlibDecoder::new(data);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|e| PdfDocError::StreamCodec(e.to_string()))?;
    Ok(out)
}

pub(crate) fn decode_stream_data(dict: &BTreeMap<String, PdfVal>, raw: &[u8]) -> Result<Vec<u8>> {
    let filter = dict.get("Filter");
    match filter {
        None => Ok(raw.to_vec()),
        Some(PdfVal::Name(name)) if name == "FlateDecode" => {
            let mut decoded = inflate_bytes(raw)?;
            if let Some(PdfVal::Dict(dp)) = dict.get("DecodeParms")
                && let Some(pred) = dp.get("Predictor").and_then(PdfVal::as_i64)
                && pred >= 10
            {
                let cols = dp.get("Columns").and_then(PdfVal::as_i64).unwrap_or(1) as usize;
                decoded = unfilter_png_predictor(&decoded, cols)?;
            }
            Ok(decoded)
        }
        Some(PdfVal::Array(arr)) if arr.len() == 1 && arr[0].as_name() == Some("FlateDecode") => {
            inflate_bytes(raw)
        }
        Some(other) => Err(PdfDocError::StreamCodec(format!(
            "unsupported stream filter {other:?}"
        ))),
    }
}

fn unfilter_png_predictor(data: &[u8], cols: usize) -> Result<Vec<u8>> {
    let row_len = cols.saturating_add(1);
    if row_len <= 1 || !data.len().is_multiple_of(row_len) {
        return Ok(data.to_vec());
    }
    let mut out = Vec::with_capacity((data.len() / row_len) * cols);
    let mut prev = vec![0u8; cols];
    for row in data.chunks_exact(row_len) {
        let filt = row[0];
        let src = &row[1..];
        let mut cur = vec![0u8; cols];
        for i in 0..cols {
            let a = if i > 0 { cur[i - 1] } else { 0 };
            let b = prev[i];
            let c = if i > 0 { prev[i - 1] } else { 0 };
            cur[i] = match filt {
                0 => src[i],
                1 => src[i].wrapping_add(a),
                2 => src[i].wrapping_add(b),
                3 => src[i].wrapping_add(((u16::from(a) + u16::from(b)) / 2) as u8),
                4 => src[i].wrapping_add(paeth(a, b, c)),
                _ => src[i],
            };
        }
        out.extend_from_slice(&cur);
        prev = cur;
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let (ia, ib, ic) = (i16::from(a), i16::from(b), i16::from(c));
    let p = ia + ib - ic;
    let pa = (p - ia).abs();
    let pb = (p - ib).abs();
    let pc = (p - ic).abs();
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

pub(crate) fn make_stream(
    mut dict: BTreeMap<String, PdfVal>,
    raw_or_plain: &[u8],
    encoding: StreamEncoding,
) -> Result<PdfVal> {
    let data = match encoding {
        StreamEncoding::Raw => raw_or_plain.to_vec(),
        StreamEncoding::Flate(level) => {
            dict.insert("Filter".into(), PdfVal::Name("FlateDecode".into()));
            deflate_bytes(raw_or_plain, level)?
        }
    };
    dict.insert("Length".into(), PdfVal::Int(data.len() as i64));
    Ok(PdfVal::Stream { dict, data })
}

pub(crate) fn serialize_val(val: &PdfVal) -> Vec<u8> {
    let mut out = Vec::new();
    write_val(val, &mut out);
    out
}

fn write_dict(dict: &BTreeMap<String, PdfVal>, out: &mut Vec<u8>) {
    out.extend_from_slice(b"<<");
    for (k, v) in dict {
        out.extend_from_slice(b" /");
        write_name_bytes(k, out);
        out.push(b' ');
        write_val(v, out);
    }
    out.extend_from_slice(b" >>");
}

fn write_name_bytes(name: &str, out: &mut Vec<u8>) {
    for &b in name.as_bytes() {
        if !(0x21..=0x7E).contains(&b) || b == b'#' || is_delim(b) {
            let _ = write!(out, "#{b:02X}");
        } else {
            out.push(b);
        }
    }
}

fn write_val(val: &PdfVal, out: &mut Vec<u8>) {
    match val {
        PdfVal::Null => out.extend_from_slice(b"null"),
        PdfVal::Bool(b) => out.extend_from_slice(if *b { b"true" } else { b"false" }),
        PdfVal::Int(n) => {
            let _ = write!(out, "{n}");
        }
        PdfVal::Real(f) => {
            out.extend_from_slice(fmt_f32(*f as f32).as_bytes());
        }
        PdfVal::Name(n) => {
            out.push(b'/');
            write_name_bytes(n, out);
        }
        PdfVal::Str(bytes) => {
            out.push(b'(');
            for &b in bytes {
                match b {
                    b'\\' => out.extend_from_slice(b"\\\\"),
                    b'(' => out.extend_from_slice(b"\\("),
                    b')' => out.extend_from_slice(b"\\)"),
                    b'\n' => out.extend_from_slice(b"\\n"),
                    b'\r' => out.extend_from_slice(b"\\r"),
                    other => out.push(other),
                }
            }
            out.push(b')');
        }
        PdfVal::HexStr(bytes) => {
            out.push(b'<');
            for &b in bytes {
                let _ = write!(out, "{b:02X}");
            }
            out.push(b'>');
        }
        PdfVal::Array(items) => {
            out.push(b'[');
            for item in items {
                out.push(b' ');
                write_val(item, out);
            }
            out.extend_from_slice(b" ]");
        }
        PdfVal::Dict(map) => write_dict(map, out),
        PdfVal::Ref(id, generation) => {
            let _ = write!(out, "{id} {generation} R");
        }
        PdfVal::Stream { dict, data } => {
            let mut normalized = dict.clone();
            normalized.insert("Length".into(), PdfVal::Int(data.len() as i64));
            write_dict(&normalized, out);
            out.extend_from_slice(b"\nstream\n");
            out.extend_from_slice(data);
            out.extend_from_slice(b"\nendstream");
        }
    }
}

/// Parsed PDF document supporting traditional xref tables, `/Prev` chains, xref streams, and `/ObjStm`.
#[derive(Debug, Clone)]
pub(crate) struct ParsedPdf {
    data: Vec<u8>,
    xref: BTreeMap<u32, ObjLocation>,
    pub(crate) root_id: u32,
    pub(crate) info_id: Option<u32>,
    pub(crate) encrypt_id: Option<u32>,
    pub(crate) size: u32,
    objstm_cache: HashMap<u32, BTreeMap<u32, PdfVal>>,
}

#[derive(Debug, Clone, Copy)]
enum ObjLocation {
    Offset(usize),
    Compressed { stm_id: u32 },
}

impl ParsedPdf {
    pub(crate) fn parse(data: &[u8]) -> Result<Self> {
        if !data.windows(5).take(1024).any(|w| w == b"%PDF-") {
            return Err(PdfDocError::InvalidPdf("missing %PDF- header".into()));
        }
        let startxref_pos = find_last_subslice(data, b"startxref")
            .ok_or_else(|| PdfDocError::InvalidPdf("missing startxref marker".into()))?;
        let mut cur = Cursor::new(data, startxref_pos + b"startxref".len());
        cur.skip_ws();
        let xref_offset = usize::try_from(
            cur.read_int()
                .ok_or_else(|| PdfDocError::InvalidPdf("invalid startxref offset".into()))?,
        )
        .map_err(|_| PdfDocError::InvalidPdf("negative startxref offset".into()))?;

        let mut xref: BTreeMap<u32, ObjLocation> = BTreeMap::new();
        let mut root_id: Option<u32> = None;
        let mut info_id: Option<u32> = None;
        let mut encrypt_id: Option<u32> = None;
        let mut max_size: u32 = 1;
        let mut next_xref = Some(xref_offset);
        let mut guard = 0usize;

        while let Some(offset) = next_xref {
            guard += 1;
            if guard > 128 || offset >= data.len() {
                break;
            }
            let mut c = Cursor::new(data, offset);
            c.skip_ws();
            if c.starts_with(b"xref") {
                c.pos += 4;
                loop {
                    c.skip_ws();
                    if c.starts_with(b"trailer") || c.is_eof() {
                        break;
                    }
                    let Some(first) = c.read_int().and_then(|v| u32::try_from(v).ok()) else {
                        break;
                    };
                    c.skip_ws();
                    let Some(count) = c.read_int().and_then(|v| u32::try_from(v).ok()) else {
                        break;
                    };
                    for i in 0..count {
                        c.skip_ws();
                        let Some(obj_off) = c.read_int().and_then(|v| usize::try_from(v).ok())
                        else {
                            break;
                        };
                        c.skip_ws();
                        let _gen = c.read_int();
                        c.skip_ws();
                        let flag = c.next_byte().unwrap_or(b'f');
                        if flag == b'n' {
                            xref.entry(first + i)
                                .or_insert(ObjLocation::Offset(obj_off));
                        }
                    }
                }
                c.skip_ws();
                if c.starts_with(b"trailer") {
                    c.pos += 7;
                    c.skip_ws();
                    if let Some(PdfVal::Dict(trailer)) = c.parse_val() {
                        if root_id.is_none() {
                            root_id = trailer.get("Root").and_then(PdfVal::as_ref_id);
                        }
                        if info_id.is_none() {
                            info_id = trailer.get("Info").and_then(PdfVal::as_ref_id);
                        }
                        if encrypt_id.is_none() {
                            encrypt_id = trailer.get("Encrypt").and_then(PdfVal::as_ref_id);
                        }
                        if let Some(s) = trailer.get("Size").and_then(PdfVal::as_i64)
                            && let Ok(s32) = u32::try_from(s)
                        {
                            max_size = max_size.max(s32);
                        }
                        next_xref = trailer
                            .get("Prev")
                            .and_then(PdfVal::as_i64)
                            .and_then(|p| usize::try_from(p).ok());
                    } else {
                        next_xref = None;
                    }
                } else {
                    next_xref = None;
                }
            } else {
                // Try parsing an XRef stream object at `offset`.
                if let Some((
                    _id,
                    PdfVal::Stream {
                        dict,
                        data: stm_data,
                    },
                )) = parse_indirect_at(data, offset)
                    && dict.get("Type").and_then(PdfVal::as_name) == Some("XRef")
                {
                    if root_id.is_none() {
                        root_id = dict.get("Root").and_then(PdfVal::as_ref_id);
                    }
                    if info_id.is_none() {
                        info_id = dict.get("Info").and_then(PdfVal::as_ref_id);
                    }
                    if encrypt_id.is_none() {
                        encrypt_id = dict.get("Encrypt").and_then(PdfVal::as_ref_id);
                    }
                    if let Some(s) = dict.get("Size").and_then(PdfVal::as_i64)
                        && let Ok(s32) = u32::try_from(s)
                    {
                        max_size = max_size.max(s32);
                    }
                    if let Ok(decoded) = decode_stream_data(&dict, &stm_data) {
                        parse_xref_stream_entries(&dict, &decoded, &mut xref);
                    }
                    next_xref = dict
                        .get("Prev")
                        .and_then(PdfVal::as_i64)
                        .and_then(|p| usize::try_from(p).ok());
                } else {
                    scan_indirect_objects(data, &mut xref, &mut root_id, &mut max_size);
                    break;
                }
            }
        }

        let root_id = root_id
            .ok_or_else(|| PdfDocError::InvalidPdf("PDF trailer missing /Root reference".into()))?;
        let highest = xref.keys().copied().max().unwrap_or(0) + 1;
        let mut parsed = Self {
            data: data.to_vec(),
            xref,
            root_id,
            info_id,
            encrypt_id,
            size: max_size.max(highest),
            objstm_cache: HashMap::new(),
        };
        parsed.load_object_streams();
        Ok(parsed)
    }

    fn load_object_streams(&mut self) {
        let stm_ids: BTreeSet<u32> = self
            .xref
            .values()
            .filter_map(|loc| match loc {
                ObjLocation::Compressed { stm_id, .. } => Some(*stm_id),
                ObjLocation::Offset(_) => None,
            })
            .collect();
        for stm_id in stm_ids {
            if let Some(PdfVal::Stream { dict, data }) = self.resolve_direct_only(stm_id)
                && dict.get("Type").and_then(PdfVal::as_name) == Some("ObjStm")
                && let (Some(n), Some(first)) = (
                    dict.get("N").and_then(PdfVal::as_i64),
                    dict.get("First").and_then(PdfVal::as_i64),
                )
                && let Ok(decoded) = decode_stream_data(&dict, &data)
            {
                let mut entries = BTreeMap::new();
                let mut c = Cursor::new(&decoded, 0);
                let mut pairs = Vec::new();
                for _ in 0..n.max(0) {
                    c.skip_ws();
                    let Some(oid) = c.read_int().and_then(|v| u32::try_from(v).ok()) else {
                        break;
                    };
                    c.skip_ws();
                    let Some(off) = c.read_int().and_then(|v| usize::try_from(v).ok()) else {
                        break;
                    };
                    pairs.push((oid, first as usize + off));
                }
                for (oid, off) in pairs {
                    if off < decoded.len() {
                        let mut vc = Cursor::new(&decoded, off);
                        if let Some(val) = vc.parse_val() {
                            entries.insert(oid, val);
                        }
                    }
                }
                self.objstm_cache.insert(stm_id, entries);
            }
        }
    }

    pub(crate) fn next_obj_id(&self) -> u32 {
        self.size
    }

    pub(crate) fn object_ids(&self) -> Vec<u32> {
        self.xref.keys().copied().collect()
    }

    fn resolve_direct_only(&self, id: u32) -> Option<PdfVal> {
        let &loc = self.xref.get(&id)?;
        let ObjLocation::Offset(offset) = loc else {
            return None;
        };
        self.parse_object_at(offset)
    }

    fn parse_object_at(&self, offset: usize) -> Option<PdfVal> {
        let mut c = Cursor::new(&self.data, offset);
        c.skip_ws();
        let _obj_num = c.read_int()?;
        c.skip_ws();
        let _gen_num = c.read_int()?;
        c.skip_ws();
        if !c.starts_with(b"obj") {
            return None;
        }
        c.pos += 3;
        c.skip_ws();
        let val = c.parse_val()?;
        if let PdfVal::Dict(dict) = val {
            c.skip_ws();
            if c.starts_with(b"stream") {
                c.pos += 6;
                if c.peek() == Some(b'\r') {
                    c.pos += 1;
                }
                if c.peek() == Some(b'\n') {
                    c.pos += 1;
                }
                let start = c.pos;
                let length = dict.get("Length").and_then(|v| match v {
                    PdfVal::Int(n) => usize::try_from(*n).ok(),
                    PdfVal::Ref(r, _) => self
                        .resolve_direct_only(*r)?
                        .as_i64()
                        .and_then(|n| usize::try_from(n).ok()),
                    _ => None,
                });
                let data = if let Some(len) = length
                    && start + len <= self.data.len()
                {
                    self.data[start..start + len].to_vec()
                } else if let Some(rel) = find_first_subslice(&self.data[start..], b"endstream") {
                    let mut slice = &self.data[start..start + rel];
                    while slice.ends_with(b"\n") || slice.ends_with(b"\r") {
                        slice = &slice[..slice.len() - 1];
                    }
                    slice.to_vec()
                } else {
                    Vec::new()
                };
                return Some(PdfVal::Stream { dict, data });
            }
            return Some(PdfVal::Dict(dict));
        }
        Some(val)
    }

    pub(crate) fn resolve_obj(&self, id: u32) -> Option<PdfVal> {
        match *self.xref.get(&id)? {
            ObjLocation::Offset(offset) => self.parse_object_at(offset),
            ObjLocation::Compressed { stm_id, .. } => {
                self.objstm_cache.get(&stm_id)?.get(&id).cloned()
            }
        }
    }

    pub(crate) fn resolve_val(&self, val: &PdfVal) -> Option<PdfVal> {
        match val {
            PdfVal::Ref(id, _) => self.resolve_obj(*id),
            other => Some(other.clone()),
        }
    }

    pub(crate) fn get_dict(&self, id: u32) -> Option<BTreeMap<String, PdfVal>> {
        match self.resolve_obj(id)? {
            PdfVal::Dict(d) | PdfVal::Stream { dict: d, .. } => Some(d),
            _ => None,
        }
    }

    pub(crate) fn pages_root_id(&self) -> Result<u32> {
        let catalog = self
            .get_dict(self.root_id)
            .ok_or_else(|| PdfDocError::InvalidPdf("PDF /Root catalog not found".into()))?;
        catalog
            .get("Pages")
            .and_then(PdfVal::as_ref_id)
            .ok_or_else(|| PdfDocError::InvalidPdf("PDF /Catalog missing /Pages".into()))
    }

    pub(crate) fn page_objects(&self) -> Result<Vec<u32>> {
        let pages_root = self.pages_root_id()?;
        let mut out = Vec::new();
        self.collect_pages(pages_root, &mut out, 0)?;
        Ok(out)
    }

    fn collect_pages(&self, node_id: u32, out: &mut Vec<u32>, depth: usize) -> Result<()> {
        if depth > 64 {
            return Err(PdfDocError::InvalidPdf("page tree recursion limit".into()));
        }
        let dict = self
            .get_dict(node_id)
            .ok_or_else(|| PdfDocError::InvalidPdf(format!("missing page node {node_id}")))?;
        match dict.get("Type").and_then(PdfVal::as_name) {
            Some("Page") => {
                out.push(node_id);
                Ok(())
            }
            Some("Pages") => {
                if let Some(kids_val) = dict.get("Kids") {
                    let kids = match self.resolve_val(kids_val) {
                        Some(PdfVal::Array(arr)) => arr,
                        _ => Vec::new(),
                    };
                    for kid in kids {
                        if let Some(kid_id) = kid.as_ref_id() {
                            self.collect_pages(kid_id, out, depth + 1)?;
                        }
                    }
                }
                Ok(())
            }
            _ => {
                if dict.contains_key("Kids") {
                    if let Some(kids_val) = dict.get("Kids")
                        && let Some(PdfVal::Array(kids)) = self.resolve_val(kids_val)
                    {
                        for kid in kids {
                            if let Some(kid_id) = kid.as_ref_id() {
                                self.collect_pages(kid_id, out, depth + 1)?;
                            }
                        }
                    }
                } else {
                    out.push(node_id);
                }
                Ok(())
            }
        }
    }

    pub(crate) fn effective_box(&self, page_id: u32, key: &str) -> Result<[f32; 4]> {
        let mut cur = Some(page_id);
        let mut guard = 0usize;
        while let Some(id) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let Some(dict) = self.get_dict(id) else {
                break;
            };
            if let Some(val) = dict.get(key)
                && let Some(rect) = self.resolve_val(val).and_then(|v| v.as_rect())
            {
                return Ok(rect);
            }
            cur = dict.get("Parent").and_then(PdfVal::as_ref_id);
        }
        Err(PdfDocError::InvalidPdf(format!(
            "page {page_id} missing {key}"
        )))
    }

    pub(crate) fn effective_resources(&self, page_id: u32) -> Option<PdfVal> {
        let mut cur = Some(page_id);
        let mut guard = 0usize;
        while let Some(id) = cur {
            guard += 1;
            if guard > 64 {
                break;
            }
            let dict = self.get_dict(id)?;
            if let Some(res) = dict.get("Resources") {
                return self.resolve_val(res);
            }
            cur = dict.get("Parent").and_then(PdfVal::as_ref_id);
        }
        None
    }

    pub(crate) fn page_decoded_content_streams(&self, page_id: u32) -> Result<Vec<Vec<u8>>> {
        let dict = self
            .get_dict(page_id)
            .ok_or_else(|| PdfDocError::InvalidPdf("page object missing".into()))?;
        let Some(contents) = dict.get("Contents") else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        match self.resolve_val(contents) {
            Some(PdfVal::Stream { dict: sd, data }) => {
                out.push(decode_stream_data(&sd, &data)?);
            }
            Some(PdfVal::Array(arr)) => {
                for item in arr {
                    if let Some(PdfVal::Stream { dict: sd, data }) = self.resolve_val(&item) {
                        out.push(decode_stream_data(&sd, &data)?);
                    }
                }
            }
            _ => {}
        }
        Ok(out)
    }
}

fn parse_indirect_at(data: &[u8], offset: usize) -> Option<(u32, PdfVal)> {
    let mut c = Cursor::new(data, offset);
    c.skip_ws();
    let obj_num = c.read_int().and_then(|v| u32::try_from(v).ok())?;
    c.skip_ws();
    let _gen_num = c.read_int()?;
    c.skip_ws();
    if !c.starts_with(b"obj") {
        return None;
    }
    c.pos += 3;
    c.skip_ws();
    let val = c.parse_val()?;
    if let PdfVal::Dict(dict) = val {
        c.skip_ws();
        if c.starts_with(b"stream") {
            c.pos += 6;
            if c.peek() == Some(b'\r') {
                c.pos += 1;
            }
            if c.peek() == Some(b'\n') {
                c.pos += 1;
            }
            let start = c.pos;
            let len = dict
                .get("Length")
                .and_then(PdfVal::as_i64)
                .and_then(|n| usize::try_from(n).ok());
            let stream_bytes = if let Some(l) = len
                && start + l <= data.len()
            {
                data[start..start + l].to_vec()
            } else if let Some(rel) = find_first_subslice(&data[start..], b"endstream") {
                data[start..start + rel].to_vec()
            } else {
                Vec::new()
            };
            return Some((
                obj_num,
                PdfVal::Stream {
                    dict,
                    data: stream_bytes,
                },
            ));
        }
        return Some((obj_num, PdfVal::Dict(dict)));
    }
    Some((obj_num, val))
}

fn parse_xref_stream_entries(
    dict: &BTreeMap<String, PdfVal>,
    decoded: &[u8],
    xref: &mut BTreeMap<u32, ObjLocation>,
) {
    let Some(PdfVal::Array(w_arr)) = dict.get("W") else {
        return;
    };
    if w_arr.len() != 3 {
        return;
    }
    let w = [
        w_arr[0].as_i64().unwrap_or(0).max(0) as usize,
        w_arr[1].as_i64().unwrap_or(0).max(0) as usize,
        w_arr[2].as_i64().unwrap_or(0).max(0) as usize,
    ];
    let entry_len = w[0] + w[1] + w[2];
    if entry_len == 0 {
        return;
    }
    let size = dict
        .get("Size")
        .and_then(PdfVal::as_i64)
        .unwrap_or(0)
        .max(0) as u32;
    let index_pairs: Vec<(u32, u32)> = match dict.get("Index") {
        Some(PdfVal::Array(arr)) => arr
            .as_chunks::<2>()
            .0
            .iter()
            .filter_map(|ch| {
                Some((
                    u32::try_from(ch[0].as_i64()?).ok()?,
                    u32::try_from(ch[1].as_i64()?).ok()?,
                ))
            })
            .collect(),
        _ => vec![(0, size)],
    };
    let read_be = |slice: &[u8]| -> u64 {
        let mut acc = 0u64;
        for &b in slice {
            acc = (acc << 8) | u64::from(b);
        }
        acc
    };
    let mut pos = 0usize;
    for (start, count) in index_pairs {
        for i in 0..count {
            if pos + entry_len > decoded.len() {
                return;
            }
            let f0 = if w[0] == 0 {
                1
            } else {
                read_be(&decoded[pos..pos + w[0]])
            };
            let f1 = read_be(&decoded[pos + w[0]..pos + w[0] + w[1]]);
            let _f2 = read_be(&decoded[pos + w[0] + w[1]..pos + entry_len]);
            pos += entry_len;
            let obj_id = start + i;
            match f0 {
                1 => {
                    xref.entry(obj_id)
                        .or_insert(ObjLocation::Offset(f1 as usize));
                }
                2 => {
                    xref.entry(obj_id)
                        .or_insert(ObjLocation::Compressed { stm_id: f1 as u32 });
                }
                _ => {}
            }
        }
    }
}

fn scan_indirect_objects(
    data: &[u8],
    xref: &mut BTreeMap<u32, ObjLocation>,
    root_id: &mut Option<u32>,
    max_size: &mut u32,
) {
    let mut i = 0;
    while i < data.len() {
        if (i == 0 || data[i - 1] == b'\n' || data[i - 1] == b'\r') && data[i].is_ascii_digit() {
            let mut c = Cursor::new(data, i);
            if let Some(num) = c.read_int().and_then(|v| u32::try_from(v).ok()) {
                c.skip_ws();
                if c.read_int().is_some() {
                    c.skip_ws();
                    if c.starts_with(b"obj")
                        && data
                            .get(c.pos + 3)
                            .is_none_or(|b| is_ws(*b) || *b == b'<' || *b == b'[')
                    {
                        xref.insert(num, ObjLocation::Offset(i));
                        *max_size = (*max_size).max(num + 1);
                        c.pos += 3;
                        c.skip_ws();
                        if let Some(PdfVal::Dict(d)) = c.parse_val()
                            && d.get("Type").and_then(PdfVal::as_name) == Some("Catalog")
                        {
                            *root_id = Some(num);
                        }
                    }
                }
            }
        }
        i += 1;
    }
}

/// Pure-Rust PDF 1.7 file writer.
pub(crate) struct RawPdfWriter {
    next_id: u32,
    objects: BTreeMap<u32, Vec<u8>>,
}

impl RawPdfWriter {
    pub(crate) fn new() -> Self {
        Self {
            next_id: 1,
            objects: BTreeMap::new(),
        }
    }

    pub(crate) fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn set(&mut self, id: u32, body: Vec<u8>) {
        self.next_id = self.next_id.max(id + 1);
        self.objects.insert(id, body);
    }

    pub(crate) fn set_val(&mut self, id: u32, val: &PdfVal) {
        self.next_id = self.next_id.max(id + 1);
        self.objects.insert(id, serialize_val(val));
    }

    pub(crate) fn finish(&self, root_id: u32, info_id: Option<u32>) -> Vec<u8> {
        self.finish_with_encrypt(root_id, info_id, None, None)
    }

    pub(crate) fn finish_with_encrypt(
        &self,
        root_id: u32,
        info_id: Option<u32>,
        encrypt_id: Option<u32>,
        file_id: Option<[u8; 16]>,
    ) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n");

        let total = self.next_id as usize;
        let mut offsets = vec![0usize; total];
        for (&id, body) in &self.objects {
            if (id as usize) < offsets.len() {
                offsets[id as usize] = out.len();
            }
            let _ = writeln!(out, "{id} 0 obj");
            out.extend_from_slice(body);
            out.extend_from_slice(b"\nendobj\n");
        }

        let xref_pos = out.len();
        let _ = writeln!(out, "xref\n0 {total}");
        out.extend_from_slice(b"0000000000 65535 f \n");
        for offset in offsets.iter().skip(1) {
            if *offset == 0 {
                out.extend_from_slice(b"0000000000 65535 f \n");
            } else {
                let _ = writeln!(out, "{offset:010} 00000 n ");
            }
        }
        let mut trailer = format!("trailer\n<< /Size {total} /Root {root_id} 0 R");
        if let Some(info) = info_id {
            let _ = write!(trailer, " /Info {info} 0 R");
        }
        if let Some(enc) = encrypt_id {
            let _ = write!(trailer, " /Encrypt {enc} 0 R");
        }
        if let Some(fid) = file_id {
            trailer.push_str(" /ID [<");
            for b in fid {
                let _ = write!(trailer, "{b:02X}");
            }
            trailer.push_str("> <");
            for b in fid {
                let _ = write!(trailer, "{b:02X}");
            }
            trailer.push_str(">]");
        }
        let _ = writeln!(trailer, " >>\nstartxref\n{xref_pos}\n%%EOF");
        out.extend_from_slice(trailer.as_bytes());
        out
    }
}

/// Rewrites `doc` cleanly with `overrides` applied (and zero old incremental revisions).
pub(crate) fn rewrite_clean(
    doc: &ParsedPdf,
    overrides: &BTreeMap<u32, PdfVal>,
    extra: &BTreeMap<u32, PdfVal>,
    dropped_ids: &BTreeSet<u32>,
) -> Vec<u8> {
    let mut writer = RawPdfWriter::new();
    let max_id = doc
        .size
        .max(overrides.keys().copied().max().unwrap_or(0) + 1)
        .max(extra.keys().copied().max().unwrap_or(0) + 1);
    writer.next_id = max_id;

    for id in doc.object_ids() {
        if dropped_ids.contains(&id) {
            continue;
        }
        if let Some(val) = overrides.get(&id) {
            writer.set_val(id, val);
        } else if let Some(val) = doc.resolve_obj(id) {
            // Skip XRef and ObjStm objects on clean rewrite since we emit flat xref.
            let is_meta_stm = matches!(
                &val,
                PdfVal::Stream { dict, .. }
                    if matches!(dict.get("Type").and_then(PdfVal::as_name), Some("XRef" | "ObjStm"))
            );
            if !is_meta_stm {
                writer.set_val(id, &val);
            }
        }
    }
    for (&id, val) in overrides {
        if !dropped_ids.contains(&id) {
            writer.set_val(id, val);
        }
    }
    for (&id, val) in extra {
        if !dropped_ids.contains(&id) {
            writer.set_val(id, val);
        }
    }
    let info_id = doc.info_id.filter(|id| !dropped_ids.contains(id));
    writer.finish(doc.root_id, info_id)
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8], pos: usize) -> Self {
        Self { data, pos }
    }

    fn is_eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    fn peek(&self) -> Option<u8> {
        self.data.get(self.pos).copied()
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = self.peek()?;
        self.pos += 1;
        Some(b)
    }

    fn starts_with(&self, pat: &[u8]) -> bool {
        self.data[self.pos..].starts_with(pat)
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if is_ws(b) {
                self.pos += 1;
            } else if b == b'%' {
                while let Some(cb) = self.next_byte() {
                    if cb == b'\n' || cb == b'\r' {
                        break;
                    }
                }
            } else {
                break;
            }
        }
    }

    fn read_int(&mut self) -> Option<i64> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let digit_start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == digit_start {
            self.pos = start;
            return None;
        }
        std::str::from_utf8(&self.data[start..self.pos])
            .ok()?
            .parse::<i64>()
            .ok()
    }

    fn parse_val(&mut self) -> Option<PdfVal> {
        self.skip_ws();
        let b = self.peek()?;
        match b {
            b'<' if self.starts_with(b"<<") => {
                self.pos += 2;
                let mut map = BTreeMap::new();
                loop {
                    self.skip_ws();
                    if self.starts_with(b">>") {
                        self.pos += 2;
                        break;
                    }
                    if self.is_eof() {
                        break;
                    }
                    let PdfVal::Name(key) = self.parse_val()? else {
                        break;
                    };
                    let val = self.parse_val()?;
                    map.insert(key, val);
                }
                Some(PdfVal::Dict(map))
            }
            b'<' => {
                self.pos += 1;
                let mut hex = Vec::new();
                while let Some(ch) = self.next_byte() {
                    if ch == b'>' {
                        break;
                    }
                    if ch.is_ascii_hexdigit() {
                        hex.push(ch);
                    }
                }
                if hex.len() % 2 == 1 {
                    hex.push(b'0');
                }
                let mut bytes = Vec::with_capacity(hex.len() / 2);
                for pair in hex.as_chunks::<2>().0 {
                    if let Ok(s) = std::str::from_utf8(pair)
                        && let Ok(byte) = u8::from_str_radix(s, 16)
                    {
                        bytes.push(byte);
                    }
                }
                Some(PdfVal::HexStr(bytes))
            }
            b'[' => {
                self.pos += 1;
                let mut items = Vec::new();
                loop {
                    self.skip_ws();
                    if self.peek() == Some(b']') {
                        self.pos += 1;
                        break;
                    }
                    if self.is_eof() {
                        break;
                    }
                    items.push(self.parse_val()?);
                }
                Some(PdfVal::Array(items))
            }
            b'(' => {
                self.pos += 1;
                let mut bytes = Vec::new();
                let mut depth = 1usize;
                while let Some(ch) = self.next_byte() {
                    match ch {
                        b'\\' => {
                            let Some(esc) = self.next_byte() else {
                                break;
                            };
                            match esc {
                                b'n' => bytes.push(b'\n'),
                                b'r' => bytes.push(b'\r'),
                                b't' => bytes.push(b'\t'),
                                b'b' => bytes.push(0x08),
                                b'f' => bytes.push(0x0C),
                                b'(' | b')' | b'\\' => bytes.push(esc),
                                b'\r' => {
                                    if self.peek() == Some(b'\n') {
                                        self.pos += 1;
                                    }
                                }
                                b'\n' => {}
                                b'0'..=b'7' => {
                                    let mut val = esc - b'0';
                                    for _ in 0..2 {
                                        if let Some(d @ b'0'..=b'7') = self.peek() {
                                            self.pos += 1;
                                            val = val.wrapping_mul(8).wrapping_add(d - b'0');
                                        } else {
                                            break;
                                        }
                                    }
                                    bytes.push(val);
                                }
                                other => bytes.push(other),
                            }
                        }
                        b'(' => {
                            depth += 1;
                            bytes.push(b'(');
                        }
                        b')' => {
                            depth -= 1;
                            if depth == 0 {
                                break;
                            }
                            bytes.push(b')');
                        }
                        other => bytes.push(other),
                    }
                }
                Some(PdfVal::Str(bytes))
            }
            b'/' => {
                self.pos += 1;
                let start = self.pos;
                while let Some(ch) = self.peek() {
                    if is_ws(ch) || is_delim(ch) {
                        break;
                    }
                    self.pos += 1;
                }
                let raw_name = String::from_utf8_lossy(&self.data[start..self.pos]);
                Some(PdfVal::Name(decode_pdf_name(&raw_name)))
            }
            b'+' | b'-' | b'.' | b'0'..=b'9' => {
                let start = self.pos;
                let mut is_real = false;
                if matches!(b, b'+' | b'-') {
                    self.pos += 1;
                }
                while let Some(ch) = self.peek() {
                    if ch.is_ascii_digit() {
                        self.pos += 1;
                    } else if ch == b'.' {
                        is_real = true;
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
                let token = std::str::from_utf8(&self.data[start..self.pos]).ok()?;
                if is_real {
                    Some(PdfVal::Real(token.parse::<f64>().ok()?))
                } else {
                    let n = token.parse::<i64>().ok()?;
                    let saved = self.pos;
                    self.skip_ws();
                    if let Some(generation) = self.read_int().and_then(|g| u16::try_from(g).ok()) {
                        self.skip_ws();
                        if self.peek() == Some(b'R')
                            && self
                                .data
                                .get(self.pos + 1)
                                .is_none_or(|&c| is_ws(c) || is_delim(c))
                            && let Ok(obj_id) = u32::try_from(n)
                        {
                            self.pos += 1;
                            return Some(PdfVal::Ref(obj_id, generation));
                        }
                    }
                    self.pos = saved;
                    Some(PdfVal::Int(n))
                }
            }
            _ => {
                let start = self.pos;
                while let Some(ch) = self.peek() {
                    if is_ws(ch) || is_delim(ch) {
                        break;
                    }
                    self.pos += 1;
                }
                match &self.data[start..self.pos] {
                    b"true" => Some(PdfVal::Bool(true)),
                    b"false" => Some(PdfVal::Bool(false)),
                    b"null" => Some(PdfVal::Null),
                    _ => None,
                }
            }
        }
    }
}

fn decode_pdf_name(raw: &str) -> String {
    if !raw.contains('#') {
        return raw.to_owned();
    }
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#'
            && i + 2 < bytes.len()
            && let Ok(s) = std::str::from_utf8(&bytes[i + 1..i + 3])
            && let Ok(val) = u8::from_str_radix(s, 16)
        {
            out.push(val);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0x00 | b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
}

fn is_delim(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

pub(crate) fn find_last_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

pub(crate) fn find_first_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
