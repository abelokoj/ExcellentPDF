//! Board exporters (Markdown, outline, SVG, PNG summary, and `.exboard` ZIP archive) (plan X.4).

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::io::{Cursor, Read as _, Write as _};

use flate2::Compression;
use flate2::write::ZlibEncoder;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::notes::export::DeepLink;

use super::{ArgumentRole, BoardError, BoardRect, CanvasBoard, CardKind};

/// PNG magic signature bytes.
const PNG_SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];

/// Exports `board` as Markdown with groups, cards, relations, and `ecclesiapdf://open?...` deep links.
#[must_use]
pub fn export_board_markdown(board: &CanvasBoard) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {}\n", board.title());

    let groups: Vec<_> = board.groups().collect();
    if !groups.is_empty() {
        out.push_str("## Groups\n\n");
        for group in groups {
            let _ = writeln!(out, "### {} (`{}`)", group.title, group.id);
            for card_id in &group.card_ids {
                if let Some(card) = board.card(card_id) {
                    write_card_markdown(&mut out, card);
                }
            }
            out.push('\n');
        }
    }

    let ungrouped: Vec<_> = board
        .cards()
        .filter(|card| card.group_id.is_none())
        .collect();
    if !ungrouped.is_empty() {
        out.push_str("## Cards\n\n");
        for card in ungrouped {
            write_card_markdown(&mut out, card);
        }
        out.push('\n');
    }

    let connectors: Vec<_> = board.connectors().collect();
    if !connectors.is_empty() {
        out.push_str("## Connections\n\n");
        for conn in connectors {
            let label = if conn.label.is_empty() {
                format!("{:?}", conn.kind)
            } else {
                conn.label.clone()
            };
            let _ = writeln!(
                out,
                "- `{}` --({label})--> `{}`",
                conn.from_card, conn.to_card
            );
        }
    }
    out
}

/// Exports `board` as a hierarchical plain-text outline grouped by board groups and argument roles.
#[must_use]
pub fn export_board_outline(board: &CanvasBoard) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "{}", board.title());
    for group in board.groups() {
        let _ = writeln!(out, "  - [Group] {}", group.title);
        for card_id in &group.card_ids {
            if let Some(card) = board.card(card_id) {
                let _ = writeln!(out, "    - [{:?}] {}", card.role, card.title);
            }
        }
    }
    for card in board.cards().filter(|c| c.group_id.is_none()) {
        let _ = writeln!(out, "  - [{:?}] {}", card.role, card.title);
    }
    out
}

/// Exports `board` as a self-contained SVG document with groups, Bezier connectors, and card boxes.
#[must_use]
pub fn export_board_svg(board: &CanvasBoard) -> String {
    let bounds = board.world_bounds().unwrap_or(BoardRect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    });
    let pad = 40.0;
    let vx = bounds.x - pad;
    let vy = bounds.y - pad;
    let vw = (bounds.width + pad * 2.0).max(200.0);
    let vh = (bounds.height + pad * 2.0).max(160.0);

    let mut svg = String::new();
    let _ = writeln!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"{vx:.1} {vy:.1} {vw:.1} {vh:.1}\" width=\"{:.0}\" height=\"{:.0}\">",
        vw.ceil(),
        vh.ceil()
    );
    svg.push_str("<defs><marker id=\"arrow\" viewBox=\"0 0 10 10\" refX=\"9\" refY=\"5\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto-start-reverse\"><path d=\"M 0 1 L 10 5 L 0 9 z\" fill=\"#475569\"/></marker></defs>\n");
    let _ = writeln!(
        svg,
        "<rect x=\"{vx:.1}\" y=\"{vy:.1}\" width=\"{vw:.1}\" height=\"{vh:.1}\" fill=\"#f8fafc\"/>"
    );

    for group in board.groups() {
        let b = group.bounds.normalized();
        let _ = writeln!(
            svg,
            "<g data-group=\"{}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"12\" fill=\"#e2e8f0\" fill-opacity=\"0.45\" stroke=\"#94a3b8\" stroke-dasharray=\"6 4\"/><text x=\"{:.1}\" y=\"{:.1}\" font-family=\"sans-serif\" font-size=\"14\" font-weight=\"bold\" fill=\"#1e293b\">{}</text></g>",
            xml_escape(&group.id),
            b.x,
            b.y,
            b.width,
            b.height,
            b.x + 12.0,
            b.y + 22.0,
            xml_escape(&group.title)
        );
    }

    for conn in board.connectors() {
        if let Some(curve) = board.connector_bezier(&conn.id) {
            let _ = writeln!(
                svg,
                "<path d=\"M {:.1} {:.1} C {:.1} {:.1}, {:.1} {:.1}, {:.1} {:.1}\" fill=\"none\" stroke=\"#475569\" stroke-width=\"2\" marker-end=\"url(#arrow)\"/>",
                curve.start.0,
                curve.start.1,
                curve.cp1.0,
                curve.cp1.1,
                curve.cp2.0,
                curve.cp2.1,
                curve.end.0,
                curve.end.1
            );
            if !conn.label.is_empty() {
                let mid = curve.sample(0.5);
                let _ = writeln!(
                    svg,
                    "<text x=\"{:.1}\" y=\"{:.1}\" font-family=\"sans-serif\" font-size=\"11\" text-anchor=\"middle\" fill=\"#334155\">{}</text>",
                    mid.0,
                    mid.1 - 6.0,
                    xml_escape(&conn.label)
                );
            }
        }
    }

    for card in board.cards() {
        let b = card.bounds.normalized();
        let accent = role_hex_color(card.role);
        let _ = writeln!(
            svg,
            "<g data-card=\"{}\"><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"8\" fill=\"#ffffff\" stroke=\"{accent}\" stroke-width=\"2\"/><rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"6\" rx=\"3\" fill=\"{accent}\"/><text x=\"{:.1}\" y=\"{:.1}\" font-family=\"sans-serif\" font-size=\"13\" font-weight=\"600\" fill=\"#0f172a\">{}</text></g>",
            xml_escape(&card.id),
            b.x,
            b.y,
            b.width,
            b.height,
            b.x,
            b.y,
            b.width,
            b.x + 10.0,
            b.y + 26.0,
            xml_escape(&card.title)
        );
    }

    svg.push_str("</svg>\n");
    svg
}

/// Renders a crisp raster PNG summary image of `board` at `width` × `height` pixels.
///
/// # Errors
///
/// Returns [`BoardError::InvalidInput`] if `width` or `height` is zero or exceeds `8192` px, or
/// [`BoardError::Export`] if PNG compression fails.
pub fn export_board_png_summary(
    board: &CanvasBoard,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, BoardError> {
    if width == 0 || height == 0 || width > 8192 || height > 8192 {
        return Err(BoardError::InvalidInput(format!(
            "invalid PNG dimensions {width}x{height}"
        )));
    }
    let w = width as usize;
    let h = height as usize;
    // RGB pixel buffer initialized to soft slate background (#f8fafc).
    let mut pixels = vec![0_u8; w * h * 3];
    for chunk in pixels.as_chunks_mut::<3>().0 {
        chunk[0] = 248;
        chunk[1] = 250;
        chunk[2] = 252;
    }

    let bounds = board.world_bounds().unwrap_or(BoardRect {
        x: 0.0,
        y: 0.0,
        width: 800.0,
        height: 600.0,
    });
    let pad = 24.0;
    let world_w = bounds.width.max(1.0);
    let world_h = bounds.height.max(1.0);
    let scale = ((f64::from(width) - pad * 2.0) / world_w)
        .min((f64::from(height) - pad * 2.0) / world_h)
        .max(0.001);
    let map_x = |wx: f64| ((wx - bounds.x) * scale + pad).round() as i32;
    let map_y = |wy: f64| ((wy - bounds.y) * scale + pad).round() as i32;

    let mut canvas = RgbCanvas {
        buf: &mut pixels,
        width: w,
        height: h,
    };

    for group in board.groups() {
        let b = group.bounds.normalized();
        canvas.draw_rect_outline(
            PixelRect::new(
                map_x(b.x),
                map_y(b.y),
                map_x(b.x + b.width),
                map_y(b.y + b.height),
            ),
            [148, 163, 184],
        );
    }

    for conn in board.connectors() {
        if let Some(curve) = board.connector_bezier(&conn.id) {
            let pts = curve.polyline(16);
            for pair in pts.windows(2) {
                canvas.draw_line(
                    (map_x(pair[0].0), map_y(pair[0].1)),
                    (map_x(pair[1].0), map_y(pair[1].1)),
                    [71, 85, 105],
                );
            }
        }
    }

    for card in board.cards() {
        let b = card.bounds.normalized();
        let x0 = map_x(b.x);
        let y0 = map_y(b.y);
        let x1 = map_x(b.x + b.width).max(x0 + 4);
        let y1 = map_y(b.y + b.height).max(y0 + 4);
        canvas.fill_rect(PixelRect::new(x0, y0, x1, y1), [255, 255, 255]);
        canvas.draw_rect_outline(PixelRect::new(x0, y0, x1, y1), role_rgb_color(card.role));
        let bar_h = ((y1 - y0) / 5).clamp(2, 6);
        canvas.fill_rect(
            PixelRect::new(x0, y0, x1, y0 + bar_h),
            role_rgb_color(card.role),
        );
    }

    encode_rgb_png(width, height, &pixels)
}

/// Packages `board` and its binary card assets into a self-contained `.exboard` ZIP archive (plan X.1, X.4).
///
/// # Errors
///
/// Returns [`BoardError::Export`] if ZIP creation or JSON serialization fails.
pub fn export_exboard_zip(board: &CanvasBoard) -> Result<Vec<u8>, BoardError> {
    let mut cursor = Cursor::new(Vec::new());
    let options = SimpleFileOptions::default().compression_method(CompressionMethod::Deflated);
    let mut writer = ZipWriter::new(&mut cursor);

    let mut assets = BTreeMap::<String, Vec<u8>>::new();
    let json_text = board.to_json_with_asset_map(&mut assets)?;

    writer
        .start_file("board.json", options)
        .map_err(|e| BoardError::Export(format!("zip board.json: {e}")))?;
    writer
        .write_all(json_text.as_bytes())
        .map_err(|e| BoardError::Export(format!("write board.json: {e}")))?;

    for (asset_name, bytes) in assets {
        writer
            .start_file(format!("assets/{asset_name}"), options)
            .map_err(|e| BoardError::Export(format!("zip asset {asset_name}: {e}")))?;
        writer
            .write_all(&bytes)
            .map_err(|e| BoardError::Export(format!("write asset {asset_name}: {e}")))?;
    }

    writer
        .finish()
        .map_err(|e| BoardError::Export(format!("finish zip: {e}")))?;
    Ok(cursor.into_inner())
}

/// Unpacks a `.exboard` ZIP archive produced by [`export_exboard_zip`].
///
/// # Errors
///
/// Returns [`BoardError::InvalidFormat`] if the archive is malformed or lacks `board.json`.
pub fn import_exboard_zip(bytes: &[u8]) -> Result<CanvasBoard, BoardError> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|e| BoardError::InvalidFormat(format!("invalid exboard zip: {e}")))?;

    let mut board_json = String::new();
    {
        let mut entry = archive
            .by_name("board.json")
            .map_err(|_| BoardError::InvalidFormat("missing board.json in exboard".to_owned()))?;
        entry
            .read_to_string(&mut board_json)
            .map_err(|e| BoardError::InvalidFormat(format!("read board.json: {e}")))?;
    }

    let mut assets = BTreeMap::new();
    for idx in 0..archive.len() {
        let mut entry = archive
            .by_index(idx)
            .map_err(|e| BoardError::InvalidFormat(format!("zip entry {idx}: {e}")))?;
        let Some(rel) = entry.enclosed_name() else {
            continue;
        };
        if let Ok(stripped) = rel.strip_prefix("assets")
            && !entry.is_dir()
            && let Some(name) = stripped.to_str()
        {
            let mut buf = Vec::new();
            entry
                .read_to_end(&mut buf)
                .map_err(|e| BoardError::InvalidFormat(format!("read asset {name}: {e}")))?;
            assets.insert(name.replace('\\', "/"), buf);
        }
    }

    CanvasBoard::from_json_with_assets(&board_json, &assets)
}

fn write_card_markdown(out: &mut String, card: &super::BoardCard) {
    let _ = writeln!(
        out,
        "- **{}** (`{}` | {:?})",
        card.title, card.id, card.role
    );
    match &card.kind {
        CardKind::TextExcerpt { anchor, excerpt } => {
            let link = DeepLink {
                file: anchor.document_path.clone(),
                page: Some(anchor.page + 1),
                annot: None,
            };
            let _ = writeln!(
                out,
                "  > {}\n  > Source: [{} p.{}]({})",
                excerpt.replace('\n', " "),
                anchor.document_path.display(),
                anchor.page + 1,
                link.to_uri()
            );
        }
        CardKind::RegionCrop {
            anchor, caption, ..
        } => {
            let link = DeepLink {
                file: anchor.document_path.clone(),
                page: Some(anchor.page + 1),
                annot: None,
            };
            let _ = writeln!(
                out,
                "  Region crop: {caption} ([p.{}]({}))",
                anchor.page + 1,
                link.to_uri()
            );
        }
        CardKind::StickyNote { markdown } => {
            let _ = writeln!(out, "  {}", markdown.replace('\n', "\n  "));
        }
        CardKind::ImageAsset { caption, mime, .. } => {
            let _ = writeln!(out, "  Image ({mime}): {caption}");
        }
    }
}

fn role_hex_color(role: ArgumentRole) -> &'static str {
    match role {
        ArgumentRole::General => "#64748b",
        ArgumentRole::Claim => "#2563eb",
        ArgumentRole::Support => "#16a34a",
        ArgumentRole::Objection => "#dc2626",
        ArgumentRole::Evidence => "#d97706",
        ArgumentRole::Question => "#7c3aed",
    }
}

fn role_rgb_color(role: ArgumentRole) -> [u8; 3] {
    match role {
        ArgumentRole::General => [100, 116, 139],
        ArgumentRole::Claim => [37, 99, 235],
        ArgumentRole::Support => [22, 163, 74],
        ArgumentRole::Objection => [220, 38, 38],
        ArgumentRole::Evidence => [217, 119, 6],
        ArgumentRole::Question => [124, 58, 237],
    }
}

fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[derive(Clone, Copy)]
struct PixelRect {
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
}

impl PixelRect {
    const fn new(x0: i32, y0: i32, x1: i32, y1: i32) -> Self {
        Self { x0, y0, x1, y1 }
    }
}

struct RgbCanvas<'a> {
    buf: &'a mut [u8],
    width: usize,
    height: usize,
}

impl RgbCanvas<'_> {
    fn put_pixel(&mut self, x: i32, y: i32, rgb: [u8; 3]) {
        if x < 0 || y < 0 {
            return;
        }
        let (ux, uy) = (x as usize, y as usize);
        if ux >= self.width || uy >= self.height {
            return;
        }
        let idx = (uy * self.width + ux) * 3;
        self.buf[idx] = rgb[0];
        self.buf[idx + 1] = rgb[1];
        self.buf[idx + 2] = rgb[2];
    }

    fn fill_rect(&mut self, rect: PixelRect, rgb: [u8; 3]) {
        let lx0 = rect.x0.min(rect.x1).max(0);
        let ly0 = rect.y0.min(rect.y1).max(0);
        let lx1 = rect.x0.max(rect.x1).min(self.width as i32 - 1);
        let ly1 = rect.y0.max(rect.y1).min(self.height as i32 - 1);
        for y in ly0..=ly1 {
            for x in lx0..=lx1 {
                self.put_pixel(x, y, rgb);
            }
        }
    }

    fn draw_rect_outline(&mut self, rect: PixelRect, rgb: [u8; 3]) {
        self.draw_line((rect.x0, rect.y0), (rect.x1, rect.y0), rgb);
        self.draw_line((rect.x1, rect.y0), (rect.x1, rect.y1), rgb);
        self.draw_line((rect.x1, rect.y1), (rect.x0, rect.y1), rgb);
        self.draw_line((rect.x0, rect.y1), (rect.x0, rect.y0), rgb);
    }

    fn draw_line(&mut self, start: (i32, i32), end: (i32, i32), rgb: [u8; 3]) {
        let (mut x0, mut y0) = start;
        let (x1, y1) = end;
        let dx = (x1 - x0).abs();
        let sx = if x0 < x1 { 1 } else { -1 };
        let dy = -(y1 - y0).abs();
        let sy = if y0 < y1 { 1 } else { -1 };
        let mut err = dx + dy;
        loop {
            self.put_pixel(x0, y0, rgb);
            if x0 == x1 && y0 == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x0 += sx;
            }
            if e2 <= dx {
                err += dx;
                y0 += sy;
            }
        }
    }
}

fn encode_rgb_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>, BoardError> {
    let row_bytes = (width as usize) * 3;
    let mut raw = Vec::with_capacity((row_bytes + 1) * height as usize);
    for row in rgb.chunks_exact(row_bytes) {
        raw.push(0); // Filter type 0 (None).
        raw.extend_from_slice(row);
    }

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
    encoder
        .write_all(&raw)
        .map_err(|e| BoardError::Export(format!("zlib encode: {e}")))?;
    let idat = encoder
        .finish()
        .map_err(|e| BoardError::Export(format!("zlib finish: {e}")))?;

    let mut out = Vec::with_capacity(64 + idat.len());
    out.extend_from_slice(&PNG_SIGNATURE);

    let mut ihdr = [0_u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // 8-bit depth
    ihdr[9] = 2; // RGB color type
    write_png_chunk(&mut out, *b"IHDR", &ihdr);
    write_png_chunk(&mut out, *b"IDAT", &idat);
    write_png_chunk(&mut out, *b"IEND", &[]);
    Ok(out)
}

fn write_png_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32_ieee(&kind, data);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn crc32_ieee(kind: &[u8; 4], data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
    for &b in kind.iter().chain(data.iter()) {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}
