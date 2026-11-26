//! OCR engine trait and backends: deterministic test engine, Tesseract CLI subprocess (hOCR),
//! Windows.Media.Ocr subprocess bridge, and automatic fallback selector.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};

use super::model::{
    LanguagePack, OcrConfig, OcrError, OcrPageResult, PageImageInput, PixelFormat, WordBox,
    discover_installed_packs, preprocess_image, standard_language_packs,
};

/// Object-safe OCR backend capable of recognizing words on rendered page images.
pub trait OcrEngine: Send + Sync {
    /// Human-readable backend name (for diagnostics and reports).
    fn name(&self) -> &str;

    /// Language packs known/available to this engine.
    ///
    /// # Errors
    ///
    /// Returns [`OcrError`] if querying installed languages fails.
    fn available_languages(&self) -> Result<Vec<LanguagePack>, OcrError>;

    /// Recognizes text and word bounding boxes on `image`.
    ///
    /// # Errors
    ///
    /// Returns [`OcrError`] if preprocessing, language selection, or recognition fails.
    fn recognize_page(
        &self,
        image: &PageImageInput,
        config: &OcrConfig,
    ) -> Result<OcrPageResult, OcrError>;
}

/// Deterministic in-memory OCR engine for unit and integration tests.
#[derive(Debug, Clone, Default)]
pub struct DeterministicTestEngine {
    by_page: BTreeMap<usize, Vec<WordBox>>,
    fallback_words: Vec<WordBox>,
}

impl DeterministicTestEngine {
    /// Creates an engine that returns `fallback_words` for any page not explicitly overridden.
    #[must_use]
    pub fn new(fallback_words: Vec<WordBox>) -> Self {
        Self {
            by_page: BTreeMap::new(),
            fallback_words,
        }
    }

    /// Creates an engine returning a single line of space-separated words centered across the page.
    #[must_use]
    pub fn with_text(text: &str) -> Self {
        let tokens: Vec<&str> = text.split_whitespace().collect();
        let count = tokens.len().max(1) as f32;
        let words = tokens
            .into_iter()
            .enumerate()
            .map(|(i, word)| {
                let left = 50.0 + (i as f32) * (500.0 / count);
                let width = (420.0 / count).max(20.0);
                WordBox {
                    text: word.to_owned(),
                    left_px: left,
                    top_px: 100.0,
                    right_px: left + width,
                    bottom_px: 140.0,
                    confidence: 98.5,
                    baseline_px: Some(134.0),
                }
            })
            .collect();
        Self::new(words)
    }

    /// Registers explicit word boxes for `page_index`.
    pub fn set_page_words(&mut self, page_index: usize, words: Vec<WordBox>) {
        self.by_page.insert(page_index, words);
    }
}

impl OcrEngine for DeterministicTestEngine {
    fn name(&self) -> &str {
        "deterministic-test"
    }

    fn available_languages(&self) -> Result<Vec<LanguagePack>, OcrError> {
        let mut packs = standard_language_packs();
        for pack in &mut packs {
            if pack.code == "eng" || pack.code == "deu" {
                pack.installed = true;
            }
        }
        Ok(packs)
    }

    fn recognize_page(
        &self,
        image: &PageImageInput,
        config: &OcrConfig,
    ) -> Result<OcrPageResult, OcrError> {
        if image.width == 0 || image.height == 0 {
            return Err(OcrError::InvalidInput(
                "cannot run ocr on an empty image".to_owned(),
            ));
        }
        let (processed, skew) = preprocess_image(image, &config.preprocess);
        let words = self
            .by_page
            .get(&image.page_index)
            .cloned()
            .unwrap_or_else(|| self.fallback_words.clone());
        Ok(OcrPageResult::from_words(
            image.page_index,
            processed.width,
            processed.height,
            words,
            skew,
            self.name(),
        ))
    }
}

/// Tesseract CLI subprocess backend invoking `tesseract stdin stdout ... hocr`.
#[derive(Debug, Clone)]
pub struct TesseractCliEngine {
    /// Executable name or path (defaults to `"tesseract"`).
    pub executable: PathBuf,
    /// Optional `--tessdata-dir` passed to Tesseract.
    pub tessdata_dir: Option<PathBuf>,
}

impl Default for TesseractCliEngine {
    fn default() -> Self {
        Self {
            executable: PathBuf::from("tesseract"),
            tessdata_dir: None,
        }
    }
}

impl TesseractCliEngine {
    /// Creates a backend targeting a custom `tesseract` executable path.
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
            tessdata_dir: None,
        }
    }

    /// Checks whether `tesseract --version` runs cleanly on this machine.
    #[must_use]
    pub fn is_available(&self) -> bool {
        Command::new(&self.executable)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    }
}

impl OcrEngine for TesseractCliEngine {
    fn name(&self) -> &str {
        "tesseract-cli"
    }

    fn available_languages(&self) -> Result<Vec<LanguagePack>, OcrError> {
        if let Some(dir) = &self.tessdata_dir {
            return Ok(discover_installed_packs(dir));
        }
        let output = Command::new(&self.executable)
            .arg("--list-langs")
            .output()
            .map_err(|err| {
                OcrError::BackendUnavailable(format!(
                    "failed to invoke {}: {err}",
                    self.executable.display()
                ))
            })?;
        if !output.status.success() {
            return Err(OcrError::BackendUnavailable(
                String::from_utf8_lossy(&output.stderr).into_owned(),
            ));
        }
        let stdout = String::from_utf8_lossy(&output.stdout);
        let installed: Vec<&str> = stdout
            .lines()
            .skip(1)
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .collect();
        let mut packs = standard_language_packs();
        for pack in &mut packs {
            pack.installed = installed.iter().any(|&code| code == pack.code);
        }
        Ok(packs)
    }

    fn recognize_page(
        &self,
        image: &PageImageInput,
        config: &OcrConfig,
    ) -> Result<OcrPageResult, OcrError> {
        if image.width == 0 || image.height == 0 {
            return Err(OcrError::InvalidInput("empty image".to_owned()));
        }
        let (processed, skew) = preprocess_image(image, &config.preprocess);
        let ppm = encode_ppm(&processed)?;
        let langs = if config.languages.is_empty() {
            "eng".to_owned()
        } else {
            config.languages.join("+")
        };

        let mut cmd = Command::new(&self.executable);
        cmd.arg("stdin")
            .arg("stdout")
            .arg("-l")
            .arg(&langs)
            .arg("--dpi")
            .arg(processed.dpi.to_string())
            .arg("hocr");
        if let Some(dir) = config.tessdata_dir.as_ref().or(self.tessdata_dir.as_ref()) {
            cmd.arg("--tessdata-dir").arg(dir);
        }
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let mut child = cmd.spawn().map_err(|err| {
            OcrError::BackendUnavailable(format!(
                "could not start {}: {err}",
                self.executable.display()
            ))
        })?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(&ppm)?;
        }
        let output = child.wait_with_output()?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("Failed loading language") {
                return Err(OcrError::MissingLanguage(langs));
            }
            return Err(OcrError::Parse(format!("tesseract exited with {stderr}")));
        }
        let hocr = String::from_utf8_lossy(&output.stdout);
        parse_hocr(
            &hocr,
            image.page_index,
            processed.width,
            processed.height,
            skew,
            self.name(),
        )
    }
}

/// Windows built-in `Windows.Media.Ocr` backend invoked via PowerShell WinRT reflection.
///
/// On non-Windows platforms, [`OcrEngine::recognize_page`] returns [`OcrError::BackendUnavailable`].
#[derive(Debug, Clone, Default)]
pub struct WindowsOcrEngine;

impl WindowsOcrEngine {
    /// Creates a Windows OCR backend handle.
    #[must_use]
    pub fn new() -> Self {
        Self
    }
}

impl OcrEngine for WindowsOcrEngine {
    fn name(&self) -> &str {
        "windows-media-ocr"
    }

    fn available_languages(&self) -> Result<Vec<LanguagePack>, OcrError> {
        #[cfg(not(windows))]
        {
            Ok(standard_language_packs())
        }
        #[cfg(windows)]
        {
            let script = "[Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType=WindowsRuntime] | Out-Null; \
                [Windows.Media.Ocr.OcrEngine]::AvailableRecognizerLanguages | ForEach-Object { $_.LanguageTag }";
            let output = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", script])
                .output()
                .map_err(|err| OcrError::BackendUnavailable(err.to_string()))?;
            let stdout = String::from_utf8_lossy(&output.stdout);
            let tags: Vec<&str> = stdout.lines().map(str::trim).collect();
            let mut packs = standard_language_packs();
            for pack in &mut packs {
                pack.installed = tags.iter().any(|&t| {
                    t.eq_ignore_ascii_case(&pack.bcp47)
                        || t.split('-').next().is_some_and(|prefix| {
                            pack.bcp47
                                .split('-')
                                .next()
                                .is_some_and(|b| b.eq_ignore_ascii_case(prefix))
                        })
                });
            }
            Ok(packs)
        }
    }

    fn recognize_page(
        &self,
        image: &PageImageInput,
        config: &OcrConfig,
    ) -> Result<OcrPageResult, OcrError> {
        #[cfg(not(windows))]
        {
            let _ = (image, config);
            Err(OcrError::BackendUnavailable(
                "Windows.Media.Ocr is only available on Windows".to_owned(),
            ))
        }
        #[cfg(windows)]
        {
            recognize_with_windows_ocr(image, config, self.name())
        }
    }
}

#[cfg(windows)]
fn recognize_with_windows_ocr(
    image: &PageImageInput,
    config: &OcrConfig,
    engine_name: &str,
) -> Result<OcrPageResult, OcrError> {
    static NEXT_TMP: AtomicU64 = AtomicU64::new(1);

    let (processed, skew) = preprocess_image(image, &config.preprocess);
    let bmp = encode_bmp_24(&processed)?;
    let seq = NEXT_TMP.fetch_add(1, Ordering::Relaxed);
    let tmp_path = std::env::temp_dir().join(format!(
        "excellentpdf-winocr-{}-{seq}.bmp",
        std::process::id()
    ));
    fs::write(&tmp_path, &bmp)?;

    let lang = config
        .languages
        .first()
        .and_then(|code| {
            standard_language_packs()
                .into_iter()
                .find(|p| p.code == *code || p.bcp47.eq_ignore_ascii_case(code))
                .map(|p| p.bcp47)
        })
        .unwrap_or_else(|| "en-US".to_owned());

    let escaped_path = tmp_path.to_string_lossy().replace('\'', "''");
    let script = format!(
        "$ErrorActionPreference='Stop'; \
        Add-Type -AssemblyName System.Runtime.WindowsRuntime; \
        $asTask = ([System.WindowsRuntimeSystemExtensions].GetMethods() | Where-Object {{ $_.Name -eq 'AsTask' -and $_.GetParameters().Count -eq 1 -and $_.IsGenericMethodDefinition }})[0]; \
        function Await($op, $t) {{ $m = $asTask.MakeGenericMethod($t); $m.Invoke($null, @($op)).GetAwaiter().GetResult() }}; \
        [Windows.Storage.StorageFile, Windows.Storage, ContentType=WindowsRuntime] | Out-Null; \
        [Windows.Graphics.Imaging.BitmapDecoder, Windows.Graphics.Imaging, ContentType=WindowsRuntime] | Out-Null; \
        [Windows.Media.Ocr.OcrEngine, Windows.Foundation, ContentType=WindowsRuntime] | Out-Null; \
        [Windows.Globalization.Language, Windows.Globalization, ContentType=WindowsRuntime] | Out-Null; \
        $file = Await ([Windows.Storage.StorageFile]::GetFileFromPathAsync('{escaped_path}')) ([Windows.Storage.StorageFile]); \
        $stream = Await ($file.OpenAsync([Windows.Storage.FileAccessMode]::Read)) ([Windows.Storage.Streams.IRandomAccessStream]); \
        $dec = Await ([Windows.Graphics.Imaging.BitmapDecoder]::CreateAsync($stream)) ([Windows.Graphics.Imaging.BitmapDecoder]); \
        $bmp = Await ($dec.GetSoftwareBitmapAsync()) ([Windows.Graphics.Imaging.SoftwareBitmap]); \
        $langObj = [Windows.Globalization.Language]::new('{lang}'); \
        $eng = [Windows.Media.Ocr.OcrEngine]::TryCreateFromLanguage($langObj); \
        if (-not $eng) {{ $eng = [Windows.Media.Ocr.OcrEngine]::TryCreateFromUserProfileLanguages() }}; \
        if (-not $eng) {{ throw 'No Windows OCR language available' }}; \
        $res = Await ($eng.RecognizeAsync($bmp)) ([Windows.Media.Ocr.OcrResult]); \
        $words = @(); \
        foreach ($line in $res.Lines) {{ foreach ($w in $line.Words) {{ $r = $w.BoundingRect; $words += [pscustomobject]@{{ text=$w.Text; left=[double]$r.X; top=[double]$r.Y; right=[double]($r.X+$r.Width); bottom=[double]($r.Y+$r.Height) }} }} }}; \
        $words | ConvertTo-Json -Compress"
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();
    let _ = fs::remove_file(&tmp_path);

    let output = output.map_err(|err| OcrError::BackendUnavailable(err.to_string()))?;
    if !output.status.success() {
        return Err(OcrError::BackendUnavailable(
            String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        ));
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let trimmed = stdout.trim();
    if trimmed.is_empty() || trimmed == "null" {
        return Ok(OcrPageResult::from_words(
            image.page_index,
            processed.width,
            processed.height,
            Vec::new(),
            skew,
            engine_name,
        ));
    }
    let json_val: serde_json::Value = serde_json::from_str(trimmed)
        .map_err(|err| OcrError::Parse(format!("invalid Windows OCR JSON: {err}")))?;
    let items = match &json_val {
        serde_json::Value::Array(arr) => arr.as_slice(),
        obj @ serde_json::Value::Object(_) => std::slice::from_ref(obj),
        _ => &[],
    };
    let mut words = Vec::with_capacity(items.len());
    for item in items {
        let Some(text) = item.get("text").and_then(serde_json::Value::as_str) else {
            continue;
        };
        let left = item
            .get("left")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
        let top = item
            .get("top")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
        let right = item
            .get("right")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
        let bottom = item
            .get("bottom")
            .and_then(serde_json::Value::as_f64)
            .unwrap_or(0.0) as f32;
        words.push(WordBox {
            text: text.to_owned(),
            left_px: left,
            top_px: top,
            right_px: right,
            bottom_px: bottom,
            confidence: 90.0,
            baseline_px: Some(bottom),
        });
    }
    Ok(OcrPageResult::from_words(
        image.page_index,
        processed.width,
        processed.height,
        words,
        skew,
        engine_name,
    ))
}

/// Composite OCR engine that tries Tesseract CLI first and falls back to Windows OCR when available.
#[derive(Debug, Clone, Default)]
pub struct AutoOcrEngine {
    /// Tesseract subprocess configuration.
    pub tesseract: TesseractCliEngine,
    /// Windows OCR fallback configuration.
    pub windows: WindowsOcrEngine,
}

impl OcrEngine for AutoOcrEngine {
    fn name(&self) -> &str {
        "auto-ocr"
    }

    fn available_languages(&self) -> Result<Vec<LanguagePack>, OcrError> {
        if self.tesseract.is_available() {
            return self.tesseract.available_languages();
        }
        self.windows.available_languages()
    }

    fn recognize_page(
        &self,
        image: &PageImageInput,
        config: &OcrConfig,
    ) -> Result<OcrPageResult, OcrError> {
        if self.tesseract.is_available() {
            return self.tesseract.recognize_page(image, config);
        }
        match self.windows.recognize_page(image, config) {
            Ok(res) => Ok(res),
            Err(win_err) => Err(OcrError::BackendUnavailable(format!(
                "no local ocr engine available (tesseract not found; {win_err})"
            ))),
        }
    }
}

/// Encodes `image` as a binary Netpbm PGM (`P5`) or PPM (`P6`) byte vector for subprocess engines.
///
/// # Errors
///
/// Returns [`OcrError::InvalidInput`] if dimensions are zero or the pixel buffer is too short.
pub fn encode_ppm(image: &PageImageInput) -> Result<Vec<u8>, OcrError> {
    if image.width == 0 || image.height == 0 {
        return Err(OcrError::InvalidInput("zero-sized image".to_owned()));
    }
    let pixel_count = image.width as usize * image.height as usize;
    let needed = pixel_count * image.format.bytes_per_pixel();
    if image.pixels.len() < needed {
        return Err(OcrError::InvalidInput(
            "pixel buffer smaller than width * height * bpp".to_owned(),
        ));
    }
    match image.format {
        PixelFormat::Gray8 => {
            let header = format!("P5\n{} {}\n255\n", image.width, image.height);
            let mut out = Vec::with_capacity(header.len() + pixel_count);
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(&image.pixels[..pixel_count]);
            Ok(out)
        }
        PixelFormat::Rgb8 => {
            let header = format!("P6\n{} {}\n255\n", image.width, image.height);
            let mut out = Vec::with_capacity(header.len() + pixel_count * 3);
            out.extend_from_slice(header.as_bytes());
            out.extend_from_slice(&image.pixels[..pixel_count * 3]);
            Ok(out)
        }
        PixelFormat::Rgba8 => {
            let header = format!("P6\n{} {}\n255\n", image.width, image.height);
            let mut out = Vec::with_capacity(header.len() + pixel_count * 3);
            out.extend_from_slice(header.as_bytes());
            for chunk in image.pixels.as_chunks::<4>().0.iter().take(pixel_count) {
                out.extend_from_slice(&chunk[..3]);
            }
            Ok(out)
        }
    }
}

#[cfg(windows)]
fn encode_bmp_24(image: &PageImageInput) -> Result<Vec<u8>, OcrError> {
    if image.width == 0 || image.height == 0 {
        return Err(OcrError::InvalidInput("zero-sized image".to_owned()));
    }
    let w = image.width as usize;
    let h = image.height as usize;
    let row_stride = (w * 3 + 3) & !3;
    let pixel_bytes = row_stride * h;
    let file_size = 54 + pixel_bytes;
    let mut out = Vec::with_capacity(file_size);
    // BITMAPFILEHEADER (14 bytes)
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&[0u8; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    // BITMAPINFOHEADER (40 bytes)
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&24u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
    let ppm_dpi = ((image.dpi as f64) * 39.3701).round() as i32;
    out.extend_from_slice(&ppm_dpi.to_le_bytes());
    out.extend_from_slice(&ppm_dpi.to_le_bytes());
    out.extend_from_slice(&[0u8; 8]);

    let pad = row_stride - w * 3;
    for row in (0..h).rev() {
        for col in 0..w {
            let idx = row * w + col;
            let (r, g, b) = match image.format {
                PixelFormat::Gray8 => {
                    let v = image.pixels.get(idx).copied().unwrap_or(255);
                    (v, v, v)
                }
                PixelFormat::Rgb8 => {
                    let base = idx * 3;
                    (
                        image.pixels.get(base).copied().unwrap_or(255),
                        image.pixels.get(base + 1).copied().unwrap_or(255),
                        image.pixels.get(base + 2).copied().unwrap_or(255),
                    )
                }
                PixelFormat::Rgba8 => {
                    let base = idx * 4;
                    (
                        image.pixels.get(base).copied().unwrap_or(255),
                        image.pixels.get(base + 1).copied().unwrap_or(255),
                        image.pixels.get(base + 2).copied().unwrap_or(255),
                    )
                }
            };
            out.push(b);
            out.push(g);
            out.push(r);
        }
        out.extend(std::iter::repeat_n(0u8, pad));
    }
    Ok(out)
}

/// Parses hOCR HTML/XHTML produced by Tesseract into an [`OcrPageResult`].
///
/// # Errors
///
/// Returns [`OcrError::Parse`] if XML parsing fails.
pub fn parse_hocr(
    hocr: &str,
    page_index: usize,
    image_width: u32,
    image_height: u32,
    skew_degrees: f32,
    engine_name: &str,
) -> Result<OcrPageResult, OcrError> {
    let mut reader = Reader::from_str(hocr);
    reader.config_mut().trim_text(false);
    let mut words = Vec::new();
    let mut active_word: Option<(f32, f32, f32, f32, f32, String)> = None;
    let mut span_depth = 0usize;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if e.local_name().as_ref() == "span" {
                    if active_word.is_some() {
                        span_depth += 1;
                    } else if let Some(parsed) = parse_word_span_attrs(&e)? {
                        active_word = Some((
                            parsed.left,
                            parsed.top,
                            parsed.right,
                            parsed.bottom,
                            parsed.confidence,
                            String::new(),
                        ));
                        span_depth = 1;
                    }
                }
            }
            Ok(Event::Text(t)) => {
                if let Some((_, _, _, _, _, buf)) = active_word.as_mut() {
                    buf.push_str(&t.xml_content(XmlVersion::Implicit1_0));
                }
            }
            Ok(Event::GeneralRef(r)) => {
                if let Some((_, _, _, _, _, buf)) = active_word.as_mut() {
                    if let Ok(Some(ch)) = r.resolve_char_ref() {
                        buf.push(ch);
                    } else {
                        let entity = r.xml_content(XmlVersion::Implicit1_0);
                        match entity.as_ref() {
                            "amp" => buf.push('&'),
                            "lt" => buf.push('<'),
                            "gt" => buf.push('>'),
                            "quot" => buf.push('"'),
                            "apos" => buf.push('\''),
                            _ => {}
                        }
                    }
                }
            }
            Ok(Event::End(e)) => {
                if e.local_name().as_ref() == "span" && active_word.is_some() {
                    span_depth = span_depth.saturating_sub(1);
                    if span_depth == 0
                        && let Some((l, t, r, b, conf, text)) = active_word.take()
                    {
                        let trimmed = text.trim();
                        if !trimmed.is_empty() {
                            words.push(WordBox {
                                text: trimmed.to_owned(),
                                left_px: l,
                                top_px: t,
                                right_px: r,
                                bottom_px: b,
                                confidence: conf,
                                baseline_px: Some(b),
                            });
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(err) => return Err(OcrError::Parse(format!("hocr parse error: {err}"))),
            _ => {}
        }
    }

    Ok(OcrPageResult::from_words(
        page_index,
        image_width,
        image_height,
        words,
        skew_degrees,
        engine_name,
    ))
}

struct WordSpanAttrs {
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    confidence: f32,
}

fn parse_word_span_attrs(
    e: &quick_xml::events::BytesStart<'_>,
) -> Result<Option<WordSpanAttrs>, OcrError> {
    let mut is_word = false;
    let mut title = None;
    for attr in e.attributes() {
        let attr = attr.map_err(|err| OcrError::Parse(err.to_string()))?;
        let key = attr.key.local_name();
        let val = attr
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| OcrError::Parse(err.to_string()))?;
        if key.as_ref() == "class" && val.split_whitespace().any(|c| c == "ocrx_word") {
            is_word = true;
        } else if key.as_ref() == "title" {
            title = Some(val.into_owned());
        }
    }
    if !is_word {
        return Ok(None);
    }
    let Some(title) = title else {
        return Ok(None);
    };
    let mut bbox = None;
    let mut confidence = 85.0f32;
    for part in title.split(';') {
        let part = part.trim();
        if let Some(rest) = part.strip_prefix("bbox ") {
            let nums: Vec<f32> = rest
                .split_whitespace()
                .filter_map(|n| n.parse::<f32>().ok())
                .collect();
            if nums.len() == 4 {
                bbox = Some((nums[0], nums[1], nums[2], nums[3]));
            }
        } else if let Some(rest) = part.strip_prefix("x_wconf ")
            && let Ok(parsed) = rest.trim().parse::<f32>()
        {
            confidence = parsed;
        }
    }
    Ok(bbox.map(|(left, top, right, bottom)| WordSpanAttrs {
        left,
        top,
        right,
        bottom,
        confidence,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hocr_parses_word_boxes_and_confidences() {
        let sample = r#"<?xml version="1.0" encoding="UTF-8"?>
        <html><body>
          <div class='ocr_page' title='bbox 0 0 1000 1400'>
            <p class='ocr_par'><span class='ocr_line' title='bbox 100 200 500 240'>
              <span class='ocrx_word' title='bbox 100 200 210 240; x_wconf 96'>Hello</span>
              <span class='ocrx_word' title='bbox 230 205 390 240; x_wconf 91'>World &amp; Co</span>
            </span></p>
          </div>
        </body></html>"#;

        let res = parse_hocr(sample, 0, 1000, 1400, 0.5, "tesseract-cli").expect("parse hocr");
        assert_eq!(res.words.len(), 2);
        assert_eq!(res.words[0].text, "Hello");
        assert_eq!(res.words[0].confidence, 96.0);
        assert_eq!(res.words[1].text, "World & Co");
        assert_eq!(res.full_text, "Hello World & Co");
        assert!((res.mean_confidence - 93.5).abs() < 0.01);
        assert_eq!(res.skew_degrees, 0.5);
    }

    #[test]
    fn ppm_encodes_rgb_and_gray() {
        let img = PageImageInput {
            page_index: 0,
            width: 2,
            height: 2,
            format: PixelFormat::Gray8,
            pixels: vec![0, 128, 255, 64],
            page_width_pts: 72.0,
            page_height_pts: 72.0,
            dpi: 72,
        };
        let ppm = encode_ppm(&img).expect("ppm");
        assert!(ppm.starts_with(b"P5\n2 2\n255\n"));
        assert_eq!(ppm.len(), b"P5\n2 2\n255\n".len() + 4);
    }
}
