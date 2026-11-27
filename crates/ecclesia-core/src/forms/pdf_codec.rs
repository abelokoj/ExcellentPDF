//! Lightweight PDF cross-reference, object parser, and incremental update writer shared by
//! `ecclesia-core::forms` and `ecclesia-core::signing`.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::Read as _;

use flate2::read::ZlibDecoder;

/// Error returned when parsing or updating raw PDF structures fails.
#[derive(Debug, Clone, PartialEq, Eq)]
#[must_use]
pub(crate) struct PdfCodecError(pub(crate) String);

impl PdfCodecError {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Self(msg.into())
    }
}

impl std::fmt::Display for PdfCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for PdfCodecError {}

/// Location of an indirect PDF object in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum XrefLoc {
    Direct(usize),
    Compressed { stm_obj: u32, index: usize },
}

/// Parsed PDF value sufficient for AcroForm, Widget, Catalog, Page, Sig, and DSS inspection.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum PdfVal {
    Null,
    Bool(bool),
    Int(i64),
    Real(f64),
    Name(String),
    Str(Vec<u8>),
    Ref(u32, u16),
    Array(Vec<Self>),
    Dict(BTreeMap<String, Self>),
}

impl PdfVal {
    pub(crate) fn as_dict(&self) -> Option<&BTreeMap<String, Self>> {
        match self {
            Self::Dict(d) => Some(d),
            _ => None,
        }
    }

    pub(crate) fn as_array(&self) -> Option<&[Self]> {
        match self {
            Self::Array(a) => Some(a),
            _ => None,
        }
    }

    pub(crate) fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(s) => Some(s),
            _ => None,
        }
    }

    pub(crate) fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int(i) => Some(*i),
            Self::Real(r) => Some(*r as i64),
            _ => None,
        }
    }

    pub(crate) fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Real(r) => Some(*r),
            Self::Int(i) => Some(*i as f64),
            _ => None,
        }
    }

    pub(crate) fn as_ref(&self) -> Option<(u32, u16)> {
        match self {
            Self::Ref(n, g) => Some((*n, *g)),
            _ => None,
        }
    }

    pub(crate) fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Str(b) => Some(b),
            _ => None,
        }
    }

    pub(crate) fn as_text(&self) -> Option<String> {
        self.as_bytes().map(decode_pdf_text)
    }
}

/// Decodes a PDF text string (PDFDocEncoding / ASCII or UTF-16BE with BOM).
pub(crate) fn decode_pdf_text(bytes: &[u8]) -> String {
    if bytes.starts_with(&[0xFE, 0xFF]) {
        let mut units = Vec::with_capacity((bytes.len().saturating_sub(2)) / 2);
        for chunk in bytes[2..].as_chunks::<2>().0 {
            units.push(u16::from_be_bytes([chunk[0], chunk[1]]));
        }
        return String::from_utf16_lossy(&units);
    }
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(&bytes[3..]).into_owned();
    }
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    bytes.iter().map(|&b| char::from(b)).collect()
}

/// Encodes a Rust string as a PDF literal string `(...)`, using UTF-16BE if non-ASCII.
pub(crate) fn encode_pdf_literal(text: &str) -> String {
    if text.is_ascii() {
        let mut out = String::with_capacity(text.len() + 2);
        out.push('(');
        for ch in text.chars() {
            match ch {
                '\\' => out.push_str("\\\\"),
                '(' => out.push_str("\\("),
                ')' => out.push_str("\\)"),
                '\r' => out.push_str("\\r"),
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                c if !c.is_ascii_control() => out.push(c),
                c => {
                    let _ = write!(out, "\\{:03o}", c as u32);
                }
            }
        }
        out.push(')');
        out
    } else {
        let mut hex = String::with_capacity(text.len() * 4 + 6);
        hex.push_str("<FEFF");
        for unit in text.encode_utf16() {
            let _ = write!(hex, "{unit:04X}");
        }
        hex.push('>');
        hex
    }
}

/// Encodes a PDF name, escaping special characters with `#XX`.
pub(crate) fn encode_pdf_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 1);
    out.push('/');
    for b in name.bytes() {
        if b <= 0x20
            || b >= 0x7F
            || matches!(
                b,
                b'#' | b'/' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'%'
            )
        {
            let _ = write!(out, "#{b:02X}");
        } else {
            out.push(char::from(b));
        }
    }
    out
}

/// Parsed cross-reference index of a PDF buffer.
pub(crate) struct PdfIndex<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) objects: BTreeMap<u32, XrefLoc>,
    pub(crate) size: u32,
    pub(crate) root: Option<(u32, u16)>,
    pub(crate) info: Option<(u32, u16)>,
    pub(crate) id_pair: Option<(Vec<u8>, Vec<u8>)>,
    pub(crate) last_startxref: usize,
    pub(crate) uses_xref_stream: bool,
}

impl<'a> PdfIndex<'a> {
    pub(crate) fn parse(bytes: &'a [u8]) -> Result<Self, PdfCodecError> {
        if !bytes.windows(5).take(1024).any(|w| w == b"%PDF-") {
            return Err(PdfCodecError::new("not a PDF header"));
        }
        let last_startxref = find_last_startxref(bytes)?;
        let mut idx = Self {
            bytes,
            objects: BTreeMap::new(),
            size: 1,
            root: None,
            info: None,
            id_pair: None,
            last_startxref,
            uses_xref_stream: false,
        };
        let mut visited = Vec::new();
        let mut cursor = Some(last_startxref);
        let mut first = true;
        while let Some(offset) = cursor {
            if visited.contains(&offset) || offset >= bytes.len() {
                break;
            }
            visited.push(offset);
            let slice = skip_ws(&bytes[offset..]);
            if slice.starts_with(b"xref") {
                cursor = idx.parse_classic_xref(offset)?;
            } else {
                if first {
                    idx.uses_xref_stream = true;
                }
                cursor = idx.parse_xref_stream(offset)?;
            }
            first = false;
        }
        let max_id = idx.objects.keys().next_back().copied().unwrap_or(0);
        if idx.size <= max_id {
            idx.size = max_id + 1;
        }
        Ok(idx)
    }

    fn parse_classic_xref(&mut self, offset: usize) -> Result<Option<usize>, PdfCodecError> {
        let slice = skip_ws(&self.bytes[offset..]);
        let after_xref = slice
            .strip_prefix(b"xref")
            .ok_or_else(|| PdfCodecError::new("expected xref keyword"))?;
        let trailer_pos = find_subslice(after_xref, b"trailer")
            .ok_or_else(|| PdfCodecError::new("expected trailer after xref"))?;
        let table = String::from_utf8_lossy(&after_xref[..trailer_pos]);
        let mut tokens = table.split_whitespace();
        while let Some(tok) = tokens.next() {
            let start_id: u32 = tok
                .parse()
                .map_err(|_| PdfCodecError::new("invalid xref start object id"))?;
            let count: u32 = tokens
                .next()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| PdfCodecError::new("invalid xref subsection count"))?;
            for i in 0..count {
                let off_tok = tokens
                    .next()
                    .ok_or_else(|| PdfCodecError::new("truncated xref offset"))?;
                let _gen_tok = tokens
                    .next()
                    .ok_or_else(|| PdfCodecError::new("truncated xref generation"))?;
                let flag = tokens
                    .next()
                    .ok_or_else(|| PdfCodecError::new("truncated xref flag"))?;
                if flag == "n"
                    && let Ok(obj_off) = off_tok.parse::<usize>()
                {
                    self.objects
                        .entry(start_id + i)
                        .or_insert(XrefLoc::Direct(obj_off));
                }
            }
        }
        let trailer_bytes = &after_xref[trailer_pos + b"trailer".len()..];
        let mut parser = Parser::new(trailer_bytes);
        let trailer_val = parser.parse_value()?;
        let dict = trailer_val
            .as_dict()
            .ok_or_else(|| PdfCodecError::new("trailer is not a dictionary"))?;
        self.apply_trailer_dict(dict);
        Ok(dict
            .get("Prev")
            .and_then(PdfVal::as_int)
            .and_then(|p| usize::try_from(p).ok()))
    }

    fn parse_xref_stream(&mut self, offset: usize) -> Result<Option<usize>, PdfCodecError> {
        let body = read_indirect_object_bytes(self.bytes, offset)?;
        let header_end = find_subslice(body, b"stream")
            .ok_or_else(|| PdfCodecError::new("xref stream missing stream keyword"))?;
        let mut parser = Parser::new(&body[..header_end]);
        let val = parser.parse_value()?;
        let dict = val
            .as_dict()
            .ok_or_else(|| PdfCodecError::new("xref stream missing dictionary"))?;
        self.apply_trailer_dict(dict);

        let w_arr = dict
            .get("W")
            .and_then(PdfVal::as_array)
            .filter(|w| w.len() == 3)
            .ok_or_else(|| PdfCodecError::new("xref stream missing /W [3]"))?;
        let w1 = usize::try_from(w_arr[0].as_int().unwrap_or(0).max(0))
            .map_err(|_| PdfCodecError::new("invalid /W[0]"))?;
        let w2 = usize::try_from(w_arr[1].as_int().unwrap_or(0).max(0))
            .map_err(|_| PdfCodecError::new("invalid /W[1]"))?;
        let w3 = usize::try_from(w_arr[2].as_int().unwrap_or(0).max(0))
            .map_err(|_| PdfCodecError::new("invalid /W[2]"))?;
        let row_len = w1 + w2 + w3;
        if row_len == 0 {
            return Err(PdfCodecError::new("xref stream /W sum is zero"));
        }

        let raw_stream = extract_stream_bytes(body, header_end)?;
        let decoded = decode_stream_with_dict(raw_stream, dict, row_len)?;

        let index_vals: Vec<i64> = if let Some(arr) = dict.get("Index").and_then(PdfVal::as_array) {
            arr.iter().filter_map(PdfVal::as_int).collect()
        } else {
            vec![0, i64::from(self.size)]
        };

        let mut cursor = 0usize;
        for pair in index_vals.as_chunks::<2>().0 {
            let start = u32::try_from(pair[0].max(0)).unwrap_or(0);
            let count = u32::try_from(pair[1].max(0)).unwrap_or(0);
            for i in 0..count {
                if cursor + row_len > decoded.len() {
                    break;
                }
                let row = &decoded[cursor..cursor + row_len];
                cursor += row_len;
                let typ = if w1 == 0 { 1 } else { read_be_uint(&row[..w1]) };
                let f2 = read_be_uint(&row[w1..w1 + w2]);
                let f3 = read_be_uint(&row[w1 + w2..row_len]);
                match typ {
                    1 => {
                        if let Ok(off) = usize::try_from(f2) {
                            self.objects
                                .entry(start + i)
                                .or_insert(XrefLoc::Direct(off));
                        }
                    }
                    2 => {
                        if let (Ok(stm_obj), Ok(index)) = (u32::try_from(f2), usize::try_from(f3)) {
                            self.objects
                                .entry(start + i)
                                .or_insert(XrefLoc::Compressed { stm_obj, index });
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(dict
            .get("Prev")
            .and_then(PdfVal::as_int)
            .and_then(|p| usize::try_from(p).ok()))
    }

    fn apply_trailer_dict(&mut self, dict: &BTreeMap<String, PdfVal>) {
        if let Some(size) = dict.get("Size").and_then(PdfVal::as_int)
            && let Ok(s) = u32::try_from(size)
        {
            self.size = self.size.max(s);
        }
        if self.root.is_none() {
            self.root = dict.get("Root").and_then(PdfVal::as_ref);
        }
        if self.info.is_none() {
            self.info = dict.get("Info").and_then(PdfVal::as_ref);
        }
        if self.id_pair.is_none()
            && let Some(arr) = dict.get("ID").and_then(PdfVal::as_array)
            && arr.len() >= 2
            && let (Some(a), Some(b)) = (arr[0].as_bytes(), arr[1].as_bytes())
        {
            self.id_pair = Some((a.to_vec(), b.to_vec()));
        }
    }

    /// Reads the raw bytes of indirect object `obj_id` (without `N G obj` / `endobj`).
    pub(crate) fn read_object_body(&self, obj_id: u32) -> Result<Vec<u8>, PdfCodecError> {
        let Some(&loc) = self.objects.get(&obj_id) else {
            return Err(PdfCodecError::new(format!("missing pdf object {obj_id}")));
        };
        match loc {
            XrefLoc::Direct(offset) => Ok(read_indirect_object_bytes(self.bytes, offset)?.to_vec()),
            XrefLoc::Compressed { stm_obj, index } => {
                let Some(XrefLoc::Direct(stm_off)) = self.objects.get(&stm_obj) else {
                    return Err(PdfCodecError::new(format!(
                        "missing object stream {stm_obj}"
                    )));
                };
                let raw = read_indirect_object_bytes(self.bytes, *stm_off)?;
                let hdr_end = find_subslice(raw, b"stream")
                    .ok_or_else(|| PdfCodecError::new("object stream missing stream keyword"))?;
                let mut parser = Parser::new(&raw[..hdr_end]);
                let val = parser.parse_value()?;
                let dict = val
                    .as_dict()
                    .ok_or_else(|| PdfCodecError::new("object stream missing dictionary"))?;
                let first = usize::try_from(
                    dict.get("First")
                        .and_then(PdfVal::as_int)
                        .unwrap_or(0)
                        .max(0),
                )
                .unwrap_or(0);
                let n = usize::try_from(dict.get("N").and_then(PdfVal::as_int).unwrap_or(0).max(0))
                    .unwrap_or(0);
                let raw_stm = extract_stream_bytes(raw, hdr_end)?;
                let decoded = decode_stream_with_dict(raw_stm, dict, 1)?;
                if index >= n || first > decoded.len() {
                    return Err(PdfCodecError::new(format!(
                        "object index {index} out of bounds in objstm {stm_obj}"
                    )));
                }
                let header_tokens: Vec<usize> = String::from_utf8_lossy(&decoded[..first])
                    .split_whitespace()
                    .filter_map(|t| t.parse::<usize>().ok())
                    .collect();
                let start = first + header_tokens.get(index * 2 + 1).copied().unwrap_or(0);
                let end = header_tokens
                    .get((index + 1) * 2 + 1)
                    .map_or(decoded.len(), |&off| first + off)
                    .min(decoded.len());
                if start > end {
                    return Err(PdfCodecError::new("corrupt object stream offsets"));
                }
                Ok(decoded[start..end].to_vec())
            }
        }
    }

    pub(crate) fn read_value(&self, obj_id: u32) -> Result<PdfVal, PdfCodecError> {
        let body = self.read_object_body(obj_id)?;
        let mut parser = Parser::new(&body);
        parser.parse_value()
    }

    pub(crate) fn resolve<'b>(
        &self,
        val: &'b PdfVal,
        owned: &'b mut Option<PdfVal>,
    ) -> Result<&'b PdfVal, PdfCodecError> {
        if let Some((obj_id, _)) = val.as_ref() {
            let resolved = self.read_value(obj_id)?;
            *owned = Some(resolved);
            Ok(owned.as_ref().expect("just assigned Some"))
        } else {
            Ok(val)
        }
    }

    pub(crate) fn read_stream(
        &self,
        obj_id: u32,
    ) -> Result<(BTreeMap<String, PdfVal>, Vec<u8>), PdfCodecError> {
        let body = self.read_object_body(obj_id)?;
        let hdr_end = find_subslice(&body, b"stream")
            .ok_or_else(|| PdfCodecError::new(format!("object {obj_id} has no stream")))?;
        let mut parser = Parser::new(&body[..hdr_end]);
        let dict = parser
            .parse_value()?
            .as_dict()
            .cloned()
            .ok_or_else(|| PdfCodecError::new("stream object has no dict"))?;
        let raw_stm = extract_stream_bytes(&body, hdr_end)?;
        let decoded = decode_stream_with_dict(raw_stm, &dict, 1)?;
        Ok((dict, decoded))
    }

    pub(crate) fn catalog(&self) -> Result<(u32, u16, BTreeMap<String, PdfVal>), PdfCodecError> {
        let (root_id, root_gen) = self
            .root
            .ok_or_else(|| PdfCodecError::new("pdf trailer missing /Root"))?;
        let val = self.read_value(root_id)?;
        let dict = val
            .as_dict()
            .cloned()
            .ok_or_else(|| PdfCodecError::new("/Root is not a dictionary"))?;
        Ok((root_id, root_gen, dict))
    }

    pub(crate) fn collect_pages(&self) -> Result<Vec<(u32, u16)>, PdfCodecError> {
        let (_, _, catalog) = self.catalog()?;
        let pages_root = catalog
            .get("Pages")
            .and_then(PdfVal::as_ref)
            .ok_or_else(|| PdfCodecError::new("catalog missing /Pages"))?;
        let mut out = Vec::new();
        let mut visited = Vec::new();
        self.walk_pages_node(pages_root, &mut visited, &mut out)?;
        Ok(out)
    }

    fn walk_pages_node(
        &self,
        node: (u32, u16),
        visited: &mut Vec<u32>,
        out: &mut Vec<(u32, u16)>,
    ) -> Result<(), PdfCodecError> {
        if visited.contains(&node.0) {
            return Ok(());
        }
        visited.push(node.0);
        let val = self.read_value(node.0)?;
        let dict = val
            .as_dict()
            .ok_or_else(|| PdfCodecError::new("page tree node is not a dictionary"))?;
        if let Some(kids) = dict.get("Kids").and_then(PdfVal::as_array) {
            for kid in kids {
                if let Some(kid_ref) = kid.as_ref() {
                    self.walk_pages_node(kid_ref, visited, out)?;
                }
            }
        } else {
            out.push(node);
        }
        Ok(())
    }
}

/// Incremental PDF builder that preserves all earlier bytes verbatim and appends new/updated objects.
pub(crate) struct IncrementalPdfWriter {
    pub(crate) buf: Vec<u8>,
    pub(crate) next_id: u32,
    pub(crate) entries: BTreeMap<u32, usize>,
    root: (u32, u16),
    info: Option<(u32, u16)>,
    prev_startxref: usize,
    uses_xref_stream: bool,
}

impl IncrementalPdfWriter {
    pub(crate) fn from_index(index: &PdfIndex<'_>) -> Result<Self, PdfCodecError> {
        let root = index
            .root
            .ok_or_else(|| PdfCodecError::new("pdf trailer missing /Root"))?;
        let mut buf = index.bytes.to_vec();
        if !buf.ends_with(b"\n") {
            buf.push(b'\n');
        }
        Ok(Self {
            buf,
            next_id: index.size,
            entries: BTreeMap::new(),
            root,
            info: index.info,
            prev_startxref: index.last_startxref,
            uses_xref_stream: index.uses_xref_stream,
        })
    }

    pub(crate) fn alloc_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub(crate) fn write_object(&mut self, id: u32, gen_num: u16, body: &str) -> usize {
        let offset = self.buf.len();
        let _ = writeln!(
            self.buf_as_string_writer(),
            "{id} {gen_num} obj\n{body}\nendobj"
        );
        self.entries.insert(id, offset);
        offset
    }

    pub(crate) fn write_stream_object(
        &mut self,
        id: u32,
        gen_num: u16,
        dict_entries: &str,
        stream_bytes: &[u8],
    ) -> usize {
        let offset = self.buf.len();
        let header = format!(
            "{id} {gen_num} obj\n<< {dict_entries} /Length {} >>\nstream\n",
            stream_bytes.len()
        );
        self.buf.extend_from_slice(header.as_bytes());
        self.buf.extend_from_slice(stream_bytes);
        self.buf.extend_from_slice(b"\nendstream\nendobj\n");
        self.entries.insert(id, offset);
        offset
    }

    fn buf_as_string_writer(&mut self) -> VecFmtWriter<'_> {
        VecFmtWriter(&mut self.buf)
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.uses_xref_stream {
            let xref_obj_id = self.next_id;
            let total_size = self.next_id + 1;
            let xref_offset = self.buf.len();
            self.entries.insert(xref_obj_id, xref_offset);

            let mut index_arr = String::from("[");
            let mut stream = Vec::with_capacity(self.entries.len() * 7);
            for (&obj_id, &offset) in &self.entries {
                let _ = write!(index_arr, " {obj_id} 1");
                stream.push(1u8);
                stream.extend_from_slice(&(offset as u32).to_be_bytes());
                stream.extend_from_slice(&0u16.to_be_bytes());
            }
            index_arr.push_str(" ]");

            let mut dict = format!(
                "{xref_obj_id} 0 obj\n<< /Type /XRef /Size {total_size} /W [1 4 2] /Index {index_arr} /Root {} {} R /Prev {}",
                self.root.0, self.root.1, self.prev_startxref
            );
            if let Some(info) = self.info {
                let _ = write!(dict, " /Info {} {} R", info.0, info.1);
            }
            let _ = write!(dict, " /Length {} >>\nstream\n", stream.len());
            self.buf.extend_from_slice(dict.as_bytes());
            self.buf.extend_from_slice(&stream);
            let tail = format!("\nendstream\nendobj\nstartxref\n{xref_offset}\n%%EOF\n");
            self.buf.extend_from_slice(tail.as_bytes());
        } else {
            let xref_offset = self.buf.len();
            self.buf.extend_from_slice(b"xref\n");
            for (&obj_id, &offset) in &self.entries {
                let line = format!("{obj_id} 1\n{offset:010} 00000 n \n");
                self.buf.extend_from_slice(line.as_bytes());
            }
            let mut trailer = format!(
                "trailer\n<< /Size {} /Root {} {} R /Prev {}",
                self.next_id, self.root.0, self.root.1, self.prev_startxref
            );
            if let Some(info) = self.info {
                let _ = write!(trailer, " /Info {} {} R", info.0, info.1);
            }
            let _ = write!(trailer, " >>\nstartxref\n{xref_offset}\n%%EOF\n");
            self.buf.extend_from_slice(trailer.as_bytes());
        }
        self.buf
    }
}

struct VecFmtWriter<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for VecFmtWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

/// Formats a [`PdfVal`] back into PDF syntax.
pub(crate) fn format_pdf_val(val: &PdfVal) -> String {
    match val {
        PdfVal::Null => "null".to_owned(),
        PdfVal::Bool(b) => if *b { "true" } else { "false" }.to_owned(),
        PdfVal::Int(i) => i.to_string(),
        PdfVal::Real(r) => format!("{r:.4}"),
        PdfVal::Name(n) => encode_pdf_name(n),
        PdfVal::Str(bytes) => {
            if bytes.iter().all(|&b| (0x20..0x7F).contains(&b)) {
                encode_pdf_literal(&String::from_utf8_lossy(bytes))
            } else {
                let mut out = String::with_capacity(bytes.len() * 2 + 2);
                out.push('<');
                for b in bytes {
                    let _ = write!(out, "{b:02X}");
                }
                out.push('>');
                out
            }
        }
        PdfVal::Ref(n, g) => format!("{n} {g} R"),
        PdfVal::Array(arr) => {
            let mut out = String::from("[");
            for item in arr {
                out.push(' ');
                out.push_str(&format_pdf_val(item));
            }
            out.push_str(" ]");
            out
        }
        PdfVal::Dict(dict) => {
            let mut out = String::from("<<");
            for (k, v) in dict {
                out.push(' ');
                out.push_str(&encode_pdf_name(k));
                out.push(' ');
                out.push_str(&format_pdf_val(v));
            }
            out.push_str(" >>");
            out
        }
    }
}

// ---- Recursive-descent PDF value parser ----

pub(crate) struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    pub(crate) fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn skip_ws_and_comments(&mut self) {
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b.is_ascii_whitespace() || b == 0 {
                self.pos += 1;
            } else if b == b'%' {
                while self.pos < self.bytes.len()
                    && self.bytes[self.pos] != b'\n'
                    && self.bytes[self.pos] != b'\r'
                {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    pub(crate) fn parse_value(&mut self) -> Result<PdfVal, PdfCodecError> {
        self.skip_ws_and_comments();
        let Some(&b) = self.bytes.get(self.pos) else {
            return Err(PdfCodecError::new("unexpected end of pdf value"));
        };
        match b {
            b'<' if self.bytes.get(self.pos + 1) == Some(&b'<') => self.parse_dict(),
            b'<' => self.parse_hex_string(),
            b'(' => self.parse_literal_string(),
            b'[' => self.parse_array(),
            b'/' => self.parse_name(),
            _ => self.parse_atom_or_ref(),
        }
    }

    fn parse_dict(&mut self) -> Result<PdfVal, PdfCodecError> {
        self.pos += 2;
        let mut dict = BTreeMap::new();
        loop {
            self.skip_ws_and_comments();
            if self.bytes.get(self.pos..self.pos + 2) == Some(b">>") {
                self.pos += 2;
                break;
            }
            if self.pos >= self.bytes.len() {
                return Err(PdfCodecError::new("unterminated pdf dictionary"));
            }
            let key = match self.parse_value()? {
                PdfVal::Name(name) => name,
                _ => return Err(PdfCodecError::new("dictionary key must be a name")),
            };
            let val = self.parse_value()?;
            dict.insert(key, val);
        }
        Ok(PdfVal::Dict(dict))
    }

    fn parse_array(&mut self) -> Result<PdfVal, PdfCodecError> {
        self.pos += 1;
        let mut arr = Vec::new();
        loop {
            self.skip_ws_and_comments();
            if self.bytes.get(self.pos) == Some(&b']') {
                self.pos += 1;
                break;
            }
            if self.pos >= self.bytes.len() {
                return Err(PdfCodecError::new("unterminated pdf array"));
            }
            arr.push(self.parse_value()?);
        }
        Ok(PdfVal::Array(arr))
    }

    fn parse_name(&mut self) -> Result<PdfVal, PdfCodecError> {
        self.pos += 1;
        let mut bytes = Vec::new();
        while let Some(&b) = self.bytes.get(self.pos) {
            if b.is_ascii_whitespace() || is_pdf_delimiter(b) {
                break;
            }
            if b == b'#'
                && self.pos + 2 < self.bytes.len()
                && let (Some(h1), Some(h2)) = (
                    hex_nibble(self.bytes[self.pos + 1]),
                    hex_nibble(self.bytes[self.pos + 2]),
                )
            {
                bytes.push((h1 << 4) | h2);
                self.pos += 3;
                continue;
            }
            bytes.push(b);
            self.pos += 1;
        }
        Ok(PdfVal::Name(String::from_utf8_lossy(&bytes).into_owned()))
    }

    fn parse_literal_string(&mut self) -> Result<PdfVal, PdfCodecError> {
        self.pos += 1;
        let mut out = Vec::new();
        let mut depth = 1usize;
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b'(');
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return Ok(PdfVal::Str(out));
                    }
                    out.push(b')');
                }
                b'\\' => {
                    let Some(&esc) = self.bytes.get(self.pos) else {
                        break;
                    };
                    self.pos += 1;
                    match esc {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(0x08),
                        b'f' => out.push(0x0C),
                        b'(' | b')' | b'\\' => out.push(esc),
                        b'\r' => {
                            if self.bytes.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        b'0'..=b'7' => {
                            let mut oct = u16::from(esc - b'0');
                            for _ in 0..2 {
                                if let Some(&d) = self.bytes.get(self.pos)
                                    && (b'0'..=b'7').contains(&d)
                                {
                                    oct = oct * 8 + u16::from(d - b'0');
                                    self.pos += 1;
                                } else {
                                    break;
                                }
                            }
                            out.push((oct & 0xFF) as u8);
                        }
                        other => out.push(other),
                    }
                }
                other => out.push(other),
            }
        }
        Err(PdfCodecError::new("unterminated pdf literal string"))
    }

    fn parse_hex_string(&mut self) -> Result<PdfVal, PdfCodecError> {
        self.pos += 1;
        let mut nibbles = Vec::new();
        while let Some(&b) = self.bytes.get(self.pos) {
            self.pos += 1;
            if b == b'>' {
                if nibbles.len() % 2 == 1 {
                    nibbles.push(0);
                }
                let out = nibbles
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .map(|c| (c[0] << 4) | c[1])
                    .collect();
                return Ok(PdfVal::Str(out));
            }
            if b.is_ascii_whitespace() {
                continue;
            }
            if let Some(n) = hex_nibble(b) {
                nibbles.push(n);
            }
        }
        Err(PdfCodecError::new("unterminated pdf hex string"))
    }

    fn parse_atom_or_ref(&mut self) -> Result<PdfVal, PdfCodecError> {
        let tok1 = self.read_token();
        match tok1.as_str() {
            "null" => return Ok(PdfVal::Null),
            "true" => return Ok(PdfVal::Bool(true)),
            "false" => return Ok(PdfVal::Bool(false)),
            _ => {}
        }
        if let Ok(n) = tok1.parse::<i64>() {
            let saved = self.pos;
            self.skip_ws_and_comments();
            if self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                let tok2 = self.read_token();
                self.skip_ws_and_comments();
                let tok3 = self.read_token();
                if tok3 == "R"
                    && let (Ok(obj_id), Ok(gen_num)) = (u32::try_from(n), tok2.parse::<u16>())
                {
                    return Ok(PdfVal::Ref(obj_id, gen_num));
                }
            }
            self.pos = saved;
            return Ok(PdfVal::Int(n));
        }
        if let Ok(r) = tok1.parse::<f64>() {
            return Ok(PdfVal::Real(r));
        }
        Err(PdfCodecError::new(format!("unexpected pdf token {tok1:?}")))
    }

    fn read_token(&mut self) -> String {
        let start = self.pos;
        while let Some(&b) = self.bytes.get(self.pos) {
            if b.is_ascii_whitespace() || is_pdf_delimiter(b) {
                break;
            }
            self.pos += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.pos]).into_owned()
    }
}

fn is_pdf_delimiter(b: u8) -> bool {
    matches!(
        b,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn hex_nibble(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

pub(crate) fn find_last_startxref(bytes: &[u8]) -> Result<usize, PdfCodecError> {
    let needle = b"startxref";
    let pos = bytes
        .windows(needle.len())
        .rposition(|w| w == needle)
        .ok_or_else(|| PdfCodecError::new("missing startxref in pdf"))?;
    let tail = String::from_utf8_lossy(&bytes[pos + needle.len()..]);
    tail.split_whitespace()
        .next()
        .and_then(|num| num.parse::<usize>().ok())
        .ok_or_else(|| PdfCodecError::new("invalid startxref offset"))
}

pub(crate) fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn skip_ws(mut s: &[u8]) -> &[u8] {
    while let Some(&b) = s.first() {
        if b.is_ascii_whitespace() {
            s = &s[1..];
        } else {
            break;
        }
    }
    s
}

fn read_indirect_object_bytes(bytes: &[u8], offset: usize) -> Result<&[u8], PdfCodecError> {
    if offset >= bytes.len() {
        return Err(PdfCodecError::new("object offset out of bounds"));
    }
    let slice = &bytes[offset..];
    let obj_pos =
        find_subslice(slice, b"obj").ok_or_else(|| PdfCodecError::new("missing obj keyword"))?;
    let after_obj = &slice[obj_pos + 3..];
    let end_pos = find_subslice(after_obj, b"endobj")
        .ok_or_else(|| PdfCodecError::new("missing endobj keyword"))?;
    Ok(&after_obj[..end_pos])
}

fn extract_stream_bytes(obj_body: &[u8], stream_kw_pos: usize) -> Result<&[u8], PdfCodecError> {
    let mut start = stream_kw_pos + b"stream".len();
    if obj_body.get(start) == Some(&b'\r') {
        start += 1;
    }
    if obj_body.get(start) == Some(&b'\n') {
        start += 1;
    }
    let end = find_subslice(&obj_body[start..], b"endstream")
        .map(|rel| start + rel)
        .ok_or_else(|| PdfCodecError::new("missing endstream"))?;
    let mut trimmed = end;
    if trimmed > start && obj_body[trimmed - 1] == b'\n' {
        trimmed -= 1;
    }
    if trimmed > start && obj_body[trimmed - 1] == b'\r' {
        trimmed -= 1;
    }
    Ok(&obj_body[start..trimmed])
}

fn decode_stream_with_dict(
    raw: &[u8],
    dict: &BTreeMap<String, PdfVal>,
    default_cols: usize,
) -> Result<Vec<u8>, PdfCodecError> {
    let is_flate = match dict.get("Filter") {
        Some(PdfVal::Name(n)) => n == "FlateDecode",
        Some(PdfVal::Array(arr)) => arr.iter().any(|v| v.as_name() == Some("FlateDecode")),
        _ => false,
    };
    let decoded = if is_flate {
        let mut dec = ZlibDecoder::new(raw);
        let mut buf = Vec::new();
        dec.read_to_end(&mut buf)
            .map_err(|err| PdfCodecError::new(format!("zlib decode failed: {err}")))?;
        buf
    } else {
        raw.to_vec()
    };

    if let Some(parms) = dict.get("DecodeParms").and_then(PdfVal::as_dict)
        && let Some(pred) = parms.get("Predictor").and_then(PdfVal::as_int)
        && pred >= 10
    {
        let cols = parms
            .get("Columns")
            .and_then(PdfVal::as_int)
            .and_then(|c| usize::try_from(c).ok())
            .unwrap_or(default_cols);
        return unfilter_png_up(&decoded, cols);
    }
    Ok(decoded)
}

fn unfilter_png_up(raw: &[u8], cols: usize) -> Result<Vec<u8>, PdfCodecError> {
    let stride = cols + 1;
    if stride <= 1 || !raw.len().is_multiple_of(stride) {
        return Err(PdfCodecError::new("invalid png-filtered stream"));
    }
    let rows = raw.len() / stride;
    let mut out = vec![0u8; rows * cols];
    for r in 0..rows {
        let filter = raw[r * stride];
        let src_row = &raw[r * stride + 1..(r + 1) * stride];
        for c in 0..cols {
            let up = if r > 0 { out[(r - 1) * cols + c] } else { 0 };
            let left = if c > 0 { out[r * cols + c - 1] } else { 0 };
            let val = match filter {
                0 => src_row[c],
                1 => src_row[c].wrapping_add(left),
                2 => src_row[c].wrapping_add(up),
                _ => src_row[c],
            };
            out[r * cols + c] = val;
        }
    }
    Ok(out)
}

fn read_be_uint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0u64, |acc, &b| (acc << 8) | u64::from(b))
}
