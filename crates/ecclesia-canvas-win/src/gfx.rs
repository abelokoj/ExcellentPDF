//! Direct2D drawing primitives for the Windows page canvas: stateless helpers operating directly on
//! NSH's active [`ID2D1DeviceContext`], plus DIP-to-physical-pixel transform math so tiles render at
//! crisp 1:1 device pixels regardless of window DPI.

use ecclesia_canvas::page::host::{TriangleDirection, WASH_OPACITY, Wash};
use windows::Win32::Foundation::E_INVALIDARG;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_U, D2D1_ALPHA_MODE_IGNORE, D2D1_COLOR_F, D2D1_FIGURE_BEGIN_FILLED,
    D2D1_FIGURE_END_CLOSED, D2D1_FILL_MODE_ALTERNATE, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_ALIASED, D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_BITMAP_PROPERTIES1,
    D2D1_CAP_STYLE_ROUND, D2D1_ELLIPSE, D2D1_INTERPOLATION_MODE_LINEAR, D2D1_LINE_JOIN_ROUND,
    D2D1_PRIMITIVE_BLEND_MIN, D2D1_PRIMITIVE_BLEND_SOURCE_OVER, D2D1_QUADRATIC_BEZIER_SEGMENT,
    D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES1, ID2D1Bitmap1, ID2D1Brush, ID2D1DeviceContext,
    ID2D1Factory1, ID2D1Geometry, ID2D1SolidColorBrush, ID2D1StrokeStyle1,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{Error, Interface, Result};
use windows_numerics::{Matrix3x2, Vector2};

/// A colour, from `0xRRGGBB`.
pub(crate) fn color(rgb: u32) -> D2D1_COLOR_F {
    let channel = |shift: u32| ((rgb >> shift) & 0xff) as f32 / 255.0;
    D2D1_COLOR_F {
        r: channel(16),
        g: channel(8),
        b: channel(0),
        a: 1.0,
    }
}

/// A rectangle for Direct2D.
pub(crate) fn rect(r: &ecclesia_canvas::Rect) -> D2D_RECT_F {
    D2D_RECT_F {
        left: r.x0,
        top: r.y0,
        right: r.x1,
        bottom: r.y1,
    }
}

/// Builds the Direct2D transform that maps local physical pixel coordinates `(px, py)` through
/// `(1 / scale_x, 1 / scale_y)` into local DIPs and then applies NSH's world transform `prev`.
///
/// Direct2D uses row-vector multiplication (`p' = p * M`), so scaling local pixels before `prev`
/// scales the first row by `1 / scale_x` and the second row by `1 / scale_y` while keeping the
/// translation `(M31, M32)` intact.
pub(crate) fn pixel_transform(prev: &Matrix3x2, scale_x: f32, scale_y: f32) -> Matrix3x2 {
    Matrix3x2 {
        M11: prev.M11 / scale_x,
        M12: prev.M12 / scale_x,
        M21: prev.M21 / scale_y,
        M22: prev.M22 / scale_y,
        M31: prev.M31,
        M32: prev.M32,
    }
}

/// Queries the [`ID2D1Factory1`] that owns `dc` and creates the round-cap, round-join stroke style
/// used for ink polylines.
pub(crate) fn factory_and_round(
    dc: &ID2D1DeviceContext,
) -> Result<(ID2D1Factory1, ID2D1StrokeStyle1)> {
    // SAFETY: COM calls on the live Direct2D context and the factory it returns.
    unsafe {
        let factory: ID2D1Factory1 = dc.GetFactory()?.cast()?;
        let props = D2D1_STROKE_STYLE_PROPERTIES1 {
            startCap: D2D1_CAP_STYLE_ROUND,
            endCap: D2D1_CAP_STYLE_ROUND,
            lineJoin: D2D1_LINE_JOIN_ROUND,
            ..Default::default()
        };
        let round = factory.CreateStrokeStyle(&props, None)?;
        Ok((factory, round))
    }
}

/// Creates an opaque `width` × `height` BGRA bitmap on `dc` at 96 DPI (one unit = one pixel).
pub(crate) fn bitmap(
    dc: &ID2D1DeviceContext,
    width: u32,
    height: u32,
    bgra: &[u8],
) -> Result<ID2D1Bitmap1> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(4));
    if expected != Some(bgra.len()) {
        return Err(Error::from_hresult(E_INVALIDARG));
    }
    let props = D2D1_BITMAP_PROPERTIES1 {
        pixelFormat: D2D1_PIXEL_FORMAT {
            format: DXGI_FORMAT_B8G8R8A8_UNORM,
            alphaMode: D2D1_ALPHA_MODE_IGNORE,
        },
        dpiX: 96.0,
        dpiY: 96.0,
        ..Default::default()
    };
    // SAFETY: `bgra` has been verified to hold `height` rows of `width * 4` bytes and lives for the call.
    unsafe {
        dc.CreateBitmap(
            D2D_SIZE_U { width, height },
            Some(bgra.as_ptr().cast()),
            width * 4,
            &props,
        )
    }
}

/// Sets `dc` and `brush` up for a fill laid as `wash`; the caller restores source-over afterwards.
///
/// # Safety
///
/// Must be called between `BeginDraw` and `EndDraw`, with a brush created by `dc`.
unsafe fn set_wash(dc: &ID2D1DeviceContext, brush: &ID2D1SolidColorBrush, wash: Wash) {
    // SAFETY: caller guarantees `dc` is in an active draw pass and owns `brush`.
    unsafe {
        match wash {
            Wash::Darkest => dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_MIN),
            Wash::Translucent => brush.SetOpacity(WASH_OPACITY),
        }
    }
}

/// Fills `dest` with `rgb`.
pub(crate) fn fill(dc: &ID2D1DeviceContext, dest: &D2D_RECT_F, rgb: u32) -> Result<()> {
    // SAFETY: called inside NSH's active Direct2D draw pass; brush and `dest` live for the calls.
    unsafe {
        let brush = dc.CreateSolidColorBrush(&color(rgb), None)?;
        dc.FillRectangle(dest, &brush);
    }
    Ok(())
}

/// Draws `bitmap` stretched into `dest` at `opacity`.
pub(crate) fn draw_bitmap(
    dc: &ID2D1DeviceContext,
    bitmap: &ID2D1Bitmap1,
    dest: &D2D_RECT_F,
    opacity: f32,
) {
    // SAFETY: called inside NSH's active Direct2D draw pass with live COM objects.
    unsafe {
        dc.DrawBitmap(
            bitmap,
            Some(std::ptr::from_ref(dest)),
            opacity,
            D2D1_INTERPOLATION_MODE_LINEAR,
            None,
            None,
        );
    }
}

/// Fills `quads` (corners in pixels, in order around each) with `rgb` as a highlighter wash.
pub(crate) fn highlight(
    dc: &ID2D1DeviceContext,
    factory: &ID2D1Factory1,
    quads: &[[(f32, f32); 4]],
    rgb: u32,
    wash: Wash,
) -> Result<()> {
    if quads.is_empty() {
        return Ok(());
    }
    let at = |(x, y): (f32, f32)| Vector2 { X: x, Y: y };
    // SAFETY: called inside NSH's active Direct2D draw pass; geometry, sink, and brush live for the calls.
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        for quad in quads {
            sink.BeginFigure(at(quad[0]), D2D1_FIGURE_BEGIN_FILLED);
            sink.AddLines(&[at(quad[1]), at(quad[2]), at(quad[3])]);
            sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        }
        sink.Close()?;
        let brush = dc.CreateSolidColorBrush(&color(rgb), None)?;
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        set_wash(dc, &brush, wash);
        dc.FillGeometry(&geometry, &brush, None::<&ID2D1Brush>);
        dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    }
    Ok(())
}

/// Fills `dest` with corners rounded by `radius` pixels, laid as `wash` says (the reading ruler's band).
pub(crate) fn band(
    dc: &ID2D1DeviceContext,
    dest: &D2D_RECT_F,
    radius: f32,
    rgb: u32,
    wash: Wash,
) -> Result<()> {
    let rounded = D2D1_ROUNDED_RECT {
        rect: *dest,
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: called inside NSH's active Direct2D draw pass; brush and `rounded` live for the calls.
    unsafe {
        let brush = dc.CreateSolidColorBrush(&color(rgb), None)?;
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        set_wash(dc, &brush, wash);
        dc.FillRoundedRectangle(&rounded, &brush);
        dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    }
    Ok(())
}

/// Computes the three triangle vertices inside `dest` pointing in `direction`.
pub(crate) fn rounded_triangle_vertices(
    dest: &D2D_RECT_F,
    direction: TriangleDirection,
) -> [(f32, f32); 3] {
    let cx = (dest.left + dest.right) * 0.5;
    match direction {
        TriangleDirection::Up => [
            (cx, dest.top),
            (dest.right, dest.bottom),
            (dest.left, dest.bottom),
        ],
        TriangleDirection::Down => [
            (dest.left, dest.top),
            (dest.right, dest.top),
            (cx, dest.bottom),
        ],
    }
}

/// Fills an antialiased triangle with softly rounded corners inside `dest`, pointing in `direction`.
pub(crate) fn rounded_triangle(
    dc: &ID2D1DeviceContext,
    factory: &ID2D1Factory1,
    dest: &D2D_RECT_F,
    direction: TriangleDirection,
    corner_radius: f32,
    rgb: u32,
) -> Result<()> {
    let width = dest.right - dest.left;
    let height = dest.bottom - dest.top;
    if !(width > 0.0 && height > 0.0 && width.is_finite() && height.is_finite()) {
        return Ok(());
    }
    let verts = rounded_triangle_vertices(dest, direction);
    let edge_len = |a: (f32, f32), b: (f32, f32)| (b.0 - a.0).hypot(b.1 - a.1);
    let min_edge = edge_len(verts[0], verts[1])
        .min(edge_len(verts[1], verts[2]))
        .min(edge_len(verts[2], verts[0]));
    if min_edge <= 0.0 {
        return Ok(());
    }
    let radius = if corner_radius.is_finite() {
        corner_radius.clamp(0.0, min_edge * 0.38)
    } else {
        0.0
    };
    let offset_toward = |from: (f32, f32), toward: (f32, f32)| {
        let dx = toward.0 - from.0;
        let dy = toward.1 - from.1;
        let len = dx.hypot(dy);
        if len > 0.0 {
            (from.0 + dx / len * radius, from.1 + dy / len * radius)
        } else {
            from
        }
    };
    let corners = [0, 1, 2].map(|i| {
        let prev = verts[(i + 2) % 3];
        let curr = verts[i];
        let next = verts[(i + 1) % 3];
        (offset_toward(curr, prev), curr, offset_toward(curr, next))
    });
    let at = |(x, y): (f32, f32)| Vector2 { X: x, Y: y };
    // SAFETY: called inside NSH's active Direct2D draw pass; geometry, sink, and brush live for the calls.
    unsafe {
        let geometry = factory.CreatePathGeometry()?;
        let sink = geometry.Open()?;
        sink.BeginFigure(at(corners[0].2), D2D1_FIGURE_BEGIN_FILLED);
        for i in [1, 2, 0] {
            let (p_in, vertex, p_out) = corners[i];
            sink.AddLine(at(p_in));
            let bezier = D2D1_QUADRATIC_BEZIER_SEGMENT {
                point1: at(vertex),
                point2: at(p_out),
            };
            sink.AddQuadraticBezier(&bezier);
        }
        sink.EndFigure(D2D1_FIGURE_END_CLOSED);
        sink.Close()?;
        let brush = dc.CreateSolidColorBrush(&color(rgb), None)?;
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        dc.FillGeometry(&geometry, &brush, None::<&ID2D1Brush>);
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    }
    Ok(())
}

/// Lays `rgb` at `alpha` (0 to 1) over all of `screen` except `hole` (corners rounded by `radius`).
pub(crate) fn dim_outside(
    dc: &ID2D1DeviceContext,
    factory: &ID2D1Factory1,
    screen: &D2D_RECT_F,
    hole: &D2D_RECT_F,
    radius: f32,
    rgb: u32,
    alpha: f32,
) -> Result<()> {
    let rounded = D2D1_ROUNDED_RECT {
        rect: *hole,
        radiusX: radius,
        radiusY: radius,
    };
    // SAFETY: called inside NSH's active Direct2D draw pass; geometries, group, and brush live for the calls.
    unsafe {
        let outer: ID2D1Geometry = factory.CreateRectangleGeometry(screen)?.cast()?;
        let inner: ID2D1Geometry = factory.CreateRoundedRectangleGeometry(&rounded)?.cast()?;
        // Alternate filling leaves the inner rounded hole clear inside the outer screen rect.
        let group =
            factory.CreateGeometryGroup(D2D1_FILL_MODE_ALTERNATE, &[Some(outer), Some(inner)])?;
        let shade = D2D1_COLOR_F {
            a: alpha.clamp(0.0, 1.0),
            ..color(rgb)
        };
        let brush = dc.CreateSolidColorBrush(&shade, None)?;
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        dc.FillGeometry(&group, &brush, None::<&ID2D1Brush>);
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    }
    Ok(())
}

/// Draws an antialiased stroke through `points` (`x, y, width` in pixels) in `rgb`, with round caps and joins.
pub(crate) fn polyline(
    dc: &ID2D1DeviceContext,
    round: &ID2D1StrokeStyle1,
    points: &[(f32, f32, f32)],
    rgb: u32,
) -> Result<()> {
    let at = |(x, y, _): (f32, f32, f32)| Vector2 { X: x, Y: y };
    // SAFETY: called inside NSH's active Direct2D draw pass; brush, style, and ellipse live for the calls.
    unsafe {
        let brush = dc.CreateSolidColorBrush(&color(rgb), None)?;
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_PER_PRIMITIVE);
        if let [only] = points {
            let ellipse = D2D1_ELLIPSE {
                point: at(*only),
                radiusX: only.2 / 2.0,
                radiusY: only.2 / 2.0,
            };
            dc.FillEllipse(&ellipse, &brush);
        }
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            dc.DrawLine(at(a), at(b), &brush, (a.2 + b.2) / 2.0, round);
        }
        dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use ecclesia_mupdf::{Background, Document, Matrix};
    use windows::Win32::Graphics::Direct2D::Common::{D2D_POINT_2U, D2D_RECT_U};
    use windows::Win32::Graphics::Direct2D::{
        D2D1_BITMAP_OPTIONS_CANNOT_DRAW, D2D1_BITMAP_OPTIONS_CPU_READ, D2D1_BITMAP_OPTIONS_TARGET,
        D2D1_DEVICE_CONTEXT_OPTIONS_NONE, D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_MAP_OPTIONS_READ,
        D2D1CreateFactory,
    };
    use windows::Win32::Graphics::Direct3D::{
        D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP,
    };
    use windows::Win32::Graphics::Direct3D11::{
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    };
    use windows::Win32::Graphics::Dxgi::IDXGIDevice;

    use super::*;

    fn create_test_dc() -> ID2D1DeviceContext {
        let make = |driver: D3D_DRIVER_TYPE| -> Result<ID3D11Device> {
            let mut dev = None;
            // SAFETY: test Direct3D 11 device creation.
            unsafe {
                D3D11CreateDevice(
                    None,
                    driver,
                    Default::default(),
                    D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                    None,
                    D3D11_SDK_VERSION,
                    Some(&mut dev),
                    None,
                    None,
                )?;
            }
            Ok(dev.expect("D3D11CreateDevice returned a device"))
        };
        let d3d = make(D3D_DRIVER_TYPE_HARDWARE)
            .or_else(|_| make(D3D_DRIVER_TYPE_WARP))
            .expect("d3d11 device");
        let dxgi: IDXGIDevice = d3d.cast().expect("dxgi device");
        // SAFETY: creating a Direct2D factory, device, and device context for the test.
        unsafe {
            let factory: ID2D1Factory1 =
                D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None).expect("d2d factory");
            factory
                .CreateDevice(&dxgi)
                .expect("d2d device")
                .CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)
                .expect("d2d dc")
        }
    }

    #[test]
    fn pixel_transform_maps_physical_pixels_to_dips() {
        // 150% DPI (scale = 1.5), view translated to DIP (200, 80).
        let prev = Matrix3x2 {
            M11: 1.0,
            M12: 0.0,
            M21: 0.0,
            M22: 1.0,
            M31: 200.0,
            M32: 80.0,
        };
        let m = pixel_transform(&prev, 1.5, 1.5);
        // Physical pixel (300, 150) -> local DIP (200, 100) -> target DIP (400, 180) -> target pixel (600, 270).
        let (px, py) = (300.0_f32, 150.0_f32);
        let tx = px * m.M11 + py * m.M21 + m.M31;
        let ty = px * m.M12 + py * m.M22 + m.M32;
        assert!((tx - 400.0).abs() < 1e-4, "expected tx=400, got {tx}");
        assert!((ty - 180.0).abs() < 1e-4, "expected ty=180, got {ty}");

        // Non-uniform scale + rotation in `prev`: verify exact equivalence with `S(1/sx, 1/sy) * prev`.
        let rot = Matrix3x2 {
            M11: 0.0,
            M12: 1.0,
            M21: -1.0,
            M22: 0.0,
            M31: 50.0,
            M32: 25.0,
        };
        let mr = pixel_transform(&rot, 2.0, 1.25);
        let (px, py) = (100.0_f32, 50.0_f32);
        let (lx, ly) = (px / 2.0, py / 1.25);
        let expected_x = lx * rot.M11 + ly * rot.M21 + rot.M31;
        let expected_y = lx * rot.M12 + ly * rot.M22 + rot.M32;
        let got_x = px * mr.M11 + py * mr.M21 + mr.M31;
        let got_y = px * mr.M12 + py * mr.M22 + mr.M32;
        assert!((got_x - expected_x).abs() < 1e-4);
        assert!((got_y - expected_y).abs() < 1e-4);
    }

    #[test]
    fn direct2d_offscreen_target_renders_real_pdf_pages() {
        let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/corpus");
        let dc = create_test_dc();
        let (_factory, _round) = factory_and_round(&dc).expect("factory_and_round");

        let (target_w, target_h) = (900_u32, 700_u32);
        let scale = 1.5_f32;
        let dpi = 96.0 * scale;
        let target_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        let read_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: dpi,
            dpiY: dpi,
            bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        let size = D2D_SIZE_U {
            width: target_w,
            height: target_h,
        };
        // SAFETY: creating offscreen Direct2D target and CPU-readable staging bitmap on `dc`.
        let (target, read) = unsafe {
            let target = dc
                .CreateBitmap(size, None, 0, &target_props)
                .expect("target bitmap");
            let read = dc
                .CreateBitmap(size, None, 0, &read_props)
                .expect("read bitmap");
            dc.SetDpi(dpi, dpi);
            dc.SetTarget(&target);
            (target, read)
        };

        let chrome_bg = 0x112233_u32;
        let canvas_bg = 0xd9d9d9_u32;
        let (canvas_x, canvas_y, canvas_w, canvas_h) = (100_u32, 50_u32, 600_u32, 600_u32);

        for name in ["zlib-manual.pdf", "base14-outline-annot.pdf"] {
            let doc = Document::open(corpus.join(name)).expect("open pdf");
            let page = doc.load_page(0).expect("load page 0");
            let bounds = page.bounds().expect("bounds");
            let zoom = (560.0 / (bounds.x1 - bounds.x0).max(1.0))
                .min(560.0 / (bounds.y1 - bounds.y0).max(1.0));
            let pixmap = page
                .render(&Matrix::scale(zoom, zoom), Background::White)
                .expect("render pixmap");
            let (pw, ph) = (pixmap.width(), pixmap.height());
            let n = pixmap.components();
            let stride = pixmap.stride();
            let mut bgra = Vec::with_capacity(pw as usize * ph as usize * 4);
            for row in pixmap.samples().chunks_exact(stride).take(ph as usize) {
                for px in row[..pw as usize * n].chunks_exact(n) {
                    bgra.extend_from_slice(&[px[2], px[1], px[0], 0xff]);
                }
            }

            // Simulate NSH's active Direct2D draw pass: surrounding chrome + translated canvas.
            let prev = Matrix3x2 {
                M11: 1.0,
                M12: 0.0,
                M21: 0.0,
                M22: 1.0,
                M31: canvas_x as f32 / scale,
                M32: canvas_y as f32 / scale,
            };
            // SAFETY: Direct2D draw pass onto our offscreen `target` bitmap.
            unsafe {
                dc.BeginDraw();
                dc.SetTransform(&Matrix3x2::identity());
                let chrome_color = color(chrome_bg);
                dc.Clear(Some(&chrome_color));

                let px_xform = pixel_transform(&prev, scale, scale);
                dc.SetTransform(&px_xform);
                dc.SetAntialiasMode(D2D1_ANTIALIAS_MODE_ALIASED);
                dc.SetPrimitiveBlend(D2D1_PRIMITIVE_BLEND_SOURCE_OVER);

                let full = D2D_RECT_F {
                    left: 0.0,
                    top: 0.0,
                    right: canvas_w as f32,
                    bottom: canvas_h as f32,
                };
                fill(&dc, &full, canvas_bg).expect("fill canvas background");
                let tile = bitmap(&dc, pw, ph, &bgra).expect("upload bitmap inside draw");
                let dest = D2D_RECT_F {
                    left: 20.0,
                    top: 20.0,
                    right: 20.0 + pw as f32,
                    bottom: 20.0 + ph as f32,
                };
                draw_bitmap(&dc, &tile, &dest, 1.0);
                dc.SetTransform(&prev);
                dc.EndDraw(None, None).expect("EndDraw");
            }

            let copy_rect = D2D_RECT_U {
                left: 0,
                top: 0,
                right: target_w,
                bottom: target_h,
            };
            // SAFETY: copying offscreen target to CPU-readable staging bitmap and reading pixels.
            let (outside_rgb, paper, ink) = unsafe {
                read.CopyFromBitmap(
                    Some(&D2D_POINT_2U { x: 0, y: 0 }),
                    &target,
                    Some(&copy_rect),
                )
                .expect("CopyFromBitmap");
                let mapped = read.Map(D2D1_MAP_OPTIONS_READ).expect("Map");
                let pitch = mapped.pitch as usize;
                let bytes = std::slice::from_raw_parts(mapped.bits, pitch * target_h as usize);

                // Verify pixel (10, 10) outside the canvas rect remained untouched chrome colour.
                let out_off = 10 * pitch + 10 * 4;
                let outside_rgb = (u32::from(bytes[out_off + 2]) << 16)
                    | (u32::from(bytes[out_off + 1]) << 8)
                    | u32::from(bytes[out_off]);

                let bg_r = ((canvas_bg >> 16) & 0xff) as u8;
                let bg_g = ((canvas_bg >> 8) & 0xff) as u8;
                let bg_b = (canvas_bg & 0xff) as u8;
                let mut paper = 0usize;
                let mut ink = 0usize;
                for y in canvas_y..(canvas_y + canvas_h) {
                    let row =
                        &bytes[y as usize * pitch..y as usize * pitch + target_w as usize * 4];
                    for x in canvas_x..(canvas_x + canvas_w) {
                        let off = x as usize * 4;
                        let (b, g, r) = (row[off], row[off + 1], row[off + 2]);
                        if r > 245 && g > 245 && b > 245 {
                            paper += 1;
                        } else if (r, g, b) != (bg_r, bg_g, bg_b) {
                            ink += 1;
                        }
                    }
                }
                read.Unmap().expect("Unmap");
                (outside_rgb, paper, ink)
            };

            assert_eq!(
                outside_rgb, chrome_bg,
                "{name}: canvas background fill must never overwrite surrounding NSH chrome"
            );
            assert!(
                paper > 10_000,
                "{name}: expected white page area in Direct2D target, got {paper} paper px"
            );
            assert!(
                ink > 200,
                "{name}: expected rendered PDF ink pixels in Direct2D target, got {ink} ink px"
            );
        }
    }

    #[test]
    fn rounded_triangle_vertices_match_direction() {
        let dest = D2D_RECT_F {
            left: 10.0,
            top: 20.0,
            right: 20.0,
            bottom: 26.0,
        };
        assert_eq!(
            rounded_triangle_vertices(&dest, TriangleDirection::Up),
            [(15.0, 20.0), (20.0, 26.0), (10.0, 26.0)]
        );
        assert_eq!(
            rounded_triangle_vertices(&dest, TriangleDirection::Down),
            [(10.0, 20.0), (20.0, 20.0), (15.0, 26.0)]
        );
    }

    #[test]
    fn rounded_triangle_renders_both_directions() {
        let dc = create_test_dc();
        let (factory, _round) = factory_and_round(&dc).expect("factory_and_round");
        let (w, h) = (32_u32, 32_u32);
        let size = D2D_SIZE_U {
            width: w,
            height: h,
        };
        let target_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        let read_props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT {
                format: DXGI_FORMAT_B8G8R8A8_UNORM,
                alphaMode: D2D1_ALPHA_MODE_IGNORE,
            },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
            ..Default::default()
        };
        // SAFETY: creating offscreen target and staging bitmaps on the test device context.
        let (target, read) = unsafe {
            let target = dc
                .CreateBitmap(size, None, 0, &target_props)
                .expect("target bitmap");
            let read = dc
                .CreateBitmap(size, None, 0, &read_props)
                .expect("read bitmap");
            dc.SetDpi(96.0, 96.0);
            dc.SetTarget(&target);
            (target, read)
        };

        let bg_rgb = 0xffffff_u32;
        let tri_rgb = 0x707070_u32;
        let dest = D2D_RECT_F {
            left: 8.0,
            top: 10.0,
            right: 24.0,
            bottom: 22.0,
        };
        let empty = D2D_RECT_F {
            left: 8.0,
            top: 10.0,
            right: 8.0,
            bottom: 10.0,
        };
        let copy_rect = D2D_RECT_U {
            left: 0,
            top: 0,
            right: w,
            bottom: h,
        };

        for direction in [TriangleDirection::Up, TriangleDirection::Down] {
            // SAFETY: Direct2D draw pass onto our offscreen `target` bitmap.
            unsafe {
                dc.BeginDraw();
                let clear = color(bg_rgb);
                dc.Clear(Some(&clear));
                rounded_triangle(&dc, &factory, &empty, direction, 2.0, tri_rgb)
                    .expect("empty rounded_triangle");
                rounded_triangle(&dc, &factory, &dest, direction, 2.5, tri_rgb)
                    .expect("rounded_triangle");
                dc.EndDraw(None, None).expect("EndDraw");
            }

            // SAFETY: copying target to CPU-readable staging bitmap and inspecting BGRA pixels.
            let (corner_rgb, drawn_pixels) = unsafe {
                read.CopyFromBitmap(
                    Some(&D2D_POINT_2U { x: 0, y: 0 }),
                    &target,
                    Some(&copy_rect),
                )
                .expect("CopyFromBitmap");
                let mapped = read.Map(D2D1_MAP_OPTIONS_READ).expect("Map");
                let pitch = mapped.pitch as usize;
                let bytes = std::slice::from_raw_parts(mapped.bits, pitch * h as usize);
                let corner_rgb =
                    (u32::from(bytes[2]) << 16) | (u32::from(bytes[1]) << 8) | u32::from(bytes[0]);
                let mut drawn = 0usize;
                for y in 0..h as usize {
                    let row = &bytes[y * pitch..y * pitch + w as usize * 4];
                    for px in row.as_chunks::<4>().0 {
                        if px[0] != 0xff || px[1] != 0xff || px[2] != 0xff {
                            drawn += 1;
                        }
                    }
                }
                read.Unmap().expect("Unmap");
                (corner_rgb, drawn)
            };

            assert_eq!(
                corner_rgb, bg_rgb,
                "{direction:?}: corner outside triangle rect must remain background"
            );
            assert!(
                drawn_pixels > 20,
                "{direction:?}: expected rendered triangle pixels, got {drawn_pixels}"
            );
        }
    }
}
