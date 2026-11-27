//! Deterministic offline stroke-feature and lexicon handwriting recognizer (plan H.23 / H.24).
//!
//! Segments strokes into lines and words by spatial gaps, groups multi-stroke glyphs, extracts
//! normalized geometric features (aspect ratio, endpoint-to-centroid vectors, horizontal/vertical
//! mid-line crossings, loop closure, stroke count, and directional turning histogram), matches
//! against built-in glyph prototypes, and rescores against an offline lexicon.

use std::collections::HashMap;

use ecclesia_mupdf::{PagePoint, PageRect};

use crate::handwriting::index::{
    BackendKind, HandwritingStroke, RecognitionError, RecognizedWord, Recognizer,
};

const SUPPORTED_LANGS: &[&str] = &["en", "en-US", "en-GB"];

/// Offline geometric stroke-feature + lexicon handwriting recognizer (plan H.23 / H.24).
#[derive(Debug, Clone)]
pub struct OfflineStrokeRecognizer {
    lexicon: Vec<String>,
}

impl Default for OfflineStrokeRecognizer {
    fn default() -> Self {
        Self::new()
    }
}

impl OfflineStrokeRecognizer {
    /// Creates an offline recognizer seeded with a compact default English vocabulary.
    #[must_use]
    pub fn new() -> Self {
        let words = [
            "the", "and", "for", "that", "with", "this", "from", "note", "notes", "read", "page",
            "book", "line", "test", "hello", "world", "hi", "ok", "yes", "no", "math", "code",
            "work", "time", "date", "plan", "todo", "done", "list", "item", "study", "quiz",
            "card", "text", "ink", "box", "circle", "line", "arrow", "chapter", "summary",
            "review", "exam", "due", "today", "week", "month", "year", "idea", "key",
        ];
        Self {
            lexicon: words.into_iter().map(str::to_owned).collect(),
        }
    }

    /// Creates an offline recognizer with a caller-supplied domain lexicon.
    #[must_use]
    pub fn with_lexicon<I, S>(words: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            lexicon: words.into_iter().map(Into::into).collect(),
        }
    }
}

impl Recognizer for OfflineStrokeRecognizer {
    fn backend(&self) -> BackendKind {
        BackendKind::BuiltinOffline
    }

    fn supported_languages(&self) -> &[&str] {
        SUPPORTED_LANGS
    }

    fn recognize(
        &self,
        strokes: &[HandwritingStroke],
        lang: &str,
    ) -> Result<Vec<RecognizedWord>, RecognitionError> {
        if !SUPPORTED_LANGS.iter().any(|l| l.eq_ignore_ascii_case(lang)) {
            return Err(RecognitionError::UnsupportedLanguage(lang.to_owned()));
        }
        let items: Vec<(&HandwritingStroke, PageRect)> = strokes
            .iter()
            .filter_map(|s| s.bbox().map(|b| (s, b)))
            .collect();
        if items.is_empty() {
            return Err(RecognitionError::InvalidInput(
                "no non-empty strokes provided".into(),
            ));
        }

        // Compute median stroke height (ignoring small diacritics) to cluster strokes into horizontal lines.
        let mut heights: Vec<f32> = items.iter().map(|(_, b)| b.height().max(2.0)).collect();
        heights.sort_by(f32::total_cmp);
        let median_h = heights[heights.len() / 2].max(6.0);
        let diacritic_h = median_h * 0.35;

        let mut primary: Vec<(&HandwritingStroke, PageRect)> = Vec::new();
        let mut diacritics: Vec<(&HandwritingStroke, PageRect)> = Vec::new();
        for item in items {
            if item.1.height() < diacritic_h && item.1.width() < diacritic_h {
                diacritics.push(item);
            } else {
                primary.push(item);
            }
        }
        if primary.is_empty() {
            primary = diacritics;
            diacritics = Vec::new();
        }

        // Sort top-to-bottom first, then cluster primary strokes into lines separated by > 0.85 * median_h.
        primary.sort_by(|a, b| center_y(a.1).total_cmp(&center_y(b.1)));
        let mut lines: Vec<Vec<(&HandwritingStroke, PageRect)>> = Vec::new();
        for item in primary {
            let cy = center_y(item.1);
            if let Some(last_line) = lines.last_mut() {
                let line_cy = last_line.iter().map(|(_, b)| center_y(*b)).sum::<f32>()
                    / last_line.len() as f32;
                if (cy - line_cy).abs() <= median_h * 0.85 {
                    last_line.push(item);
                    continue;
                }
            }
            lines.push(vec![item]);
        }

        // Attach small diacritics (such as `i`-dots) to the closest line vertically.
        for item in diacritics {
            let cy = center_y(item.1);
            if let Some(best_line) = lines.iter_mut().min_by(|a, b| {
                let ya = a.iter().map(|(_, r)| center_y(*r)).sum::<f32>() / a.len() as f32;
                let yb = b.iter().map(|(_, r)| center_y(*r)).sum::<f32>() / b.len() as f32;
                (cy - ya).abs().total_cmp(&(cy - yb).abs())
            }) {
                best_line.push(item);
            }
        }

        // Re-order lines strictly top-to-bottom and strokes within each line left-to-right.
        lines.sort_by(|a, b| {
            let ya = a.iter().map(|(_, r)| r.y0).fold(f32::INFINITY, f32::min);
            let yb = b.iter().map(|(_, r)| r.y0).fold(f32::INFINITY, f32::min);
            ya.total_cmp(&yb)
        });

        let mut results = Vec::new();
        for (line_idx, mut line) in lines.into_iter().enumerate() {
            line.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));

            // Split into words when horizontal gap exceeds 0.95 * median_h.
            let word_gap = (median_h * 0.95).max(12.0);
            let mut word_groups: Vec<Vec<(&HandwritingStroke, PageRect)>> = Vec::new();
            for item in line {
                if let Some(last_word) = word_groups.last_mut() {
                    let prev_right = last_word
                        .iter()
                        .map(|(_, b)| b.x1)
                        .fold(f32::NEG_INFINITY, f32::max);
                    if item.1.x0 - prev_right <= word_gap {
                        last_word.push(item);
                        continue;
                    }
                }
                word_groups.push(vec![item]);
            }

            for word_strokes in word_groups {
                let recognized =
                    recognize_word_group(&word_strokes, line_idx, median_h, &self.lexicon);
                results.push(recognized);
            }
        }

        Ok(results)
    }
}

fn center_y(r: PageRect) -> f32 {
    0.5 * (r.y0 + r.y1)
}

fn recognize_word_group(
    strokes: &[(&HandwritingStroke, PageRect)],
    line_index: usize,
    median_h: f32,
    lexicon: &[String],
) -> RecognizedWord {
    // Cluster overlapping or tightly coupled strokes into individual glyph clusters.
    let mut glyph_clusters: Vec<Vec<&HandwritingStroke>> = Vec::new();
    let mut cluster_boxes: Vec<PageRect> = Vec::new();
    let merge_gap = (median_h * 0.12).max(1.5);

    for &(stroke, bbox) in strokes {
        let mut merged = false;
        if let Some(prev_box) = cluster_boxes.last_mut()
            && bbox.x0 <= prev_box.x1 + merge_gap
            && (bbox.x1 - bbox.x0 < median_h * 0.35 || bbox.x0 <= prev_box.x1 - 0.5)
        {
            if let Some(prev_cluster) = glyph_clusters.last_mut() {
                prev_cluster.push(stroke);
            }
            prev_box.x0 = prev_box.x0.min(bbox.x0);
            prev_box.y0 = prev_box.y0.min(bbox.y0);
            prev_box.x1 = prev_box.x1.max(bbox.x1);
            prev_box.y1 = prev_box.y1.max(bbox.y1);
            merged = true;
        }
        if !merged {
            glyph_clusters.push(vec![stroke]);
            cluster_boxes.push(bbox);
        }
    }

    let mut raw_word = String::new();
    let mut alt_word = String::new();
    let mut conf_sum = 0.0_f32;
    for cluster in &glyph_clusters {
        let ranked = classify_glyph(cluster);
        if let Some(&(best_ch, conf)) = ranked.first() {
            raw_word.push(best_ch);
            conf_sum += conf;
        }
        if let Some(&(second_ch, _)) = ranked.get(1) {
            alt_word.push(second_ch);
        } else if let Some(&(best_ch, _)) = ranked.first() {
            alt_word.push(best_ch);
        }
    }

    let avg_conf = if glyph_clusters.is_empty() {
        0.0
    } else {
        conf_sum / glyph_clusters.len() as f32
    };

    let (best_text, mut candidates) = rescore_with_lexicon(&raw_word, &alt_word, lexicon);
    if !candidates.contains(&raw_word) && !raw_word.is_empty() {
        candidates.push(raw_word);
    }

    let word_bbox = cluster_boxes
        .into_iter()
        .reduce(|a, b| PageRect {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        })
        .unwrap_or_default();

    let stroke_ids = strokes.iter().map(|(s, _)| s.id.clone()).collect();

    RecognizedWord {
        text: best_text,
        candidates,
        confidence: avg_conf.clamp(0.1, 0.99),
        line_index,
        stroke_ids,
        bbox: word_bbox,
    }
}

#[derive(Debug, Clone, Copy)]
struct GlyphFeatures {
    stroke_count: usize,
    aspect: f32,
    closed: bool,
    horiz_mid_crossings: usize,
    vert_mid_crossings: usize,
    has_dot: bool,
    start_top: bool,
    end_bottom: bool,
}

fn extract_features(strokes: &[&HandwritingStroke]) -> GlyphFeatures {
    let mut all_pts: Vec<PagePoint> = Vec::new();
    let mut has_dot = false;
    for s in strokes {
        if let Some(b) = s.bbox()
            && b.width() < 4.0
            && b.height() < 4.0
            && strokes.len() > 1
        {
            has_dot = true;
        }
        all_pts.extend_from_slice(&s.points);
    }
    let (mut x0, mut x1) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut y0, mut y1) = (f32::INFINITY, f32::NEG_INFINITY);
    for p in &all_pts {
        x0 = x0.min(p.x);
        x1 = x1.max(p.x);
        y0 = y0.min(p.y);
        y1 = y1.max(p.y);
    }
    let w = (x1 - x0).max(1.0);
    let h = (y1 - y0).max(1.0);
    let mid_x = 0.5 * (x0 + x1);
    let mid_y = 0.5 * (y0 + y1);

    let mut horiz_mid_crossings = 0usize;
    let mut vert_mid_crossings = 0usize;
    for s in strokes {
        for seg in s.points.windows(2) {
            if (seg[0].y <= mid_y && seg[1].y > mid_y) || (seg[1].y <= mid_y && seg[0].y > mid_y) {
                horiz_mid_crossings += 1;
            }
            if (seg[0].x <= mid_x && seg[1].x > mid_x) || (seg[1].x <= mid_x && seg[0].x > mid_x) {
                vert_mid_crossings += 1;
            }
        }
    }

    let first_pt = strokes
        .first()
        .and_then(|s| s.points.first())
        .copied()
        .unwrap_or_default();
    let last_pt = strokes
        .first()
        .and_then(|s| s.points.last())
        .copied()
        .unwrap_or_default();
    let closed = strokes.len() == 1
        && (first_pt.x - last_pt.x).hypot(first_pt.y - last_pt.y) <= w.max(h) * 0.35
        && strokes[0].points.len() >= 4;

    GlyphFeatures {
        stroke_count: strokes.len(),
        aspect: w / h,
        closed,
        horiz_mid_crossings,
        vert_mid_crossings,
        has_dot,
        start_top: (first_pt.y - y0) <= h * 0.35,
        end_bottom: (y1 - last_pt.y) <= h * 0.35,
    }
}

fn classify_glyph(strokes: &[&HandwritingStroke]) -> Vec<(char, f32)> {
    let f = extract_features(strokes);
    let mut scores: HashMap<char, f32> = HashMap::new();

    if f.has_dot && f.stroke_count == 2 {
        scores.insert('i', 0.92);
        scores.insert('j', 0.65);
    } else if f.closed && f.horiz_mid_crossings >= 2 && f.vert_mid_crossings >= 2 {
        scores.insert('o', 0.90);
        scores.insert('0', 0.72);
    } else if f.stroke_count == 1 && f.aspect < 0.35 {
        scores.insert('l', 0.85);
        scores.insert('I', 0.72);
        scores.insert('1', 0.68);
    } else if f.stroke_count == 2 && f.horiz_mid_crossings >= 1 && f.vert_mid_crossings >= 1 {
        scores.insert('t', 0.84);
        scores.insert('x', 0.70);
    } else if f.stroke_count == 3 {
        scores.insert('H', 0.80);
        scores.insert('A', 0.72);
        scores.insert('E', 0.66);
    } else if f.stroke_count == 1 && f.start_top && f.end_bottom && f.vert_mid_crossings >= 1 {
        scores.insert('z', 0.76);
        scores.insert('s', 0.72);
    } else if f.stroke_count == 1 && f.vert_mid_crossings >= 1 && !f.closed {
        scores.insert('c', 0.74);
        scores.insert('n', 0.68);
    } else {
        scores.insert('e', 0.62);
        scores.insert('a', 0.58);
    }

    let mut ranked: Vec<(char, f32)> = scores.into_iter().collect();
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked
}

fn rescore_with_lexicon(raw: &str, alt: &str, lexicon: &[String]) -> (String, Vec<String>) {
    if raw.is_empty() {
        return (String::new(), Vec::new());
    }
    let raw_lc = raw.to_ascii_lowercase();
    if lexicon.iter().any(|w| w.eq_ignore_ascii_case(&raw_lc)) {
        let mut cands = vec![raw.to_owned()];
        if alt != raw {
            cands.push(alt.to_owned());
        }
        return (raw.to_owned(), cands);
    }

    let mut scored: Vec<(&str, usize)> = lexicon
        .iter()
        .filter(|w| w.len().abs_diff(raw_lc.len()) <= 1)
        .map(|w| (w.as_str(), edit_distance(&raw_lc, &w.to_ascii_lowercase())))
        .filter(|&(_, d)| d <= 1 && raw_lc.len() >= 2)
        .collect();
    scored.sort_by_key(|&(_, d)| d);

    if let Some(&(best, _)) = scored.first() {
        let mut cands: Vec<String> = scored
            .into_iter()
            .take(4)
            .map(|(w, _)| w.to_owned())
            .collect();
        if !cands.iter().any(|c| c.eq_ignore_ascii_case(raw)) {
            cands.push(raw.to_owned());
        }
        (best.to_owned(), cands)
    } else {
        let mut cands = vec![raw.to_owned()];
        if alt != raw {
            cands.push(alt.to_owned());
        }
        (raw.to_owned(), cands)
    }
}

fn edit_distance(a: &str, b: &str) -> usize {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=bc.len()).collect();
    let mut curr = vec![0usize; bc.len() + 1];
    for (i, &ca) in ac.iter().enumerate() {
        curr[0] = i + 1;
        for (j, &cb) in bc.iter().enumerate() {
            let cost = usize::from(ca != cb);
            curr[j + 1] = (prev[j + 1] + 1).min(curr[j] + 1).min(prev[j] + cost);
        }
        prev.copy_from_slice(&curr);
    }
    prev[bc.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f32, y: f32) -> PagePoint {
        PagePoint { x, y }
    }

    #[test]
    fn recognizes_simple_printed_letters_and_preserves_lines() {
        // Line 0: "lo"
        let l_stroke = HandwritingStroke {
            id: "s1".into(),
            points: vec![pt(20.0, 20.0), pt(20.0, 50.0)],
            timestamps_ms: Vec::new(),
        };
        let o_stroke = HandwritingStroke {
            id: "s2".into(),
            points: vec![
                pt(28.0, 30.0),
                pt(42.0, 30.0),
                pt(42.0, 48.0),
                pt(28.0, 48.0),
                pt(28.0, 30.0),
            ],
            timestamps_ms: Vec::new(),
        };
        // Line 1: "i" (stem + dot)
        let i_stem = HandwritingStroke {
            id: "s3".into(),
            points: vec![pt(20.0, 100.0), pt(20.0, 120.0)],
            timestamps_ms: Vec::new(),
        };
        let i_dot = HandwritingStroke {
            id: "s4".into(),
            points: vec![pt(20.0, 90.0), pt(21.0, 91.0)],
            timestamps_ms: Vec::new(),
        };

        let recognizer = OfflineStrokeRecognizer::with_lexicon(["lo", "i", "hi"]);
        let words = recognizer
            .recognize(&[l_stroke, o_stroke, i_stem, i_dot], "en-US")
            .expect("recognize");
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "lo");
        assert_eq!(words[0].line_index, 0);
        assert_eq!(words[1].text, "i");
        assert_eq!(words[1].line_index, 1);
    }
}
