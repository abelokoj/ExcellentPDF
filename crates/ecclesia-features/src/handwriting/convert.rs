//! Convert lassoed handwriting to FreeText annotations or clipboard text (plan H.24).
//!
//! Preserves exact line breaks and page position when turning handwritten ink strokes into a
//! standard PDF `FreeText` annotation inside a single undoable [`Operation`].

use ecclesia_core::annot::{Annotation, Boxed, Shape, Style};
use ecclesia_core::edit::{Change, Operation};
use ecclesia_mupdf::PageRect;

use crate::handwriting::index::{HandwritingStroke, RecognitionError, RecognizedWord, Recognizer};

/// Whether the user chooses to replace the ink with a `FreeText` annotation or copy as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ConversionTarget {
    /// Delete the selected ink strokes and insert a `FreeText` annotation at the same position.
    #[default]
    ReplaceWithFreeText,
    /// Keep the ink untouched and return recognized multi-line text + candidate list for copy.
    CopyTextOnly,
}

/// Output of lassoed handwriting-to-text conversion (plan H.24).
#[derive(Debug, Clone, PartialEq)]
pub struct ConversionOutcome {
    /// Recognized text with preserved line breaks (`\n`).
    pub text: String,
    /// Per-word recognition details and candidate alternatives for candidate UI chips.
    pub words: Vec<RecognizedWord>,
    /// Combined bounding box of the converted handwriting in page points.
    pub bbox: PageRect,
    /// Created `FreeText` annotation when [`ConversionTarget::ReplaceWithFreeText`] was chosen.
    pub freetext: Option<Annotation>,
    /// Single undoable operation deleting the source ink annotations and creating `FreeText`.
    pub operation: Option<Operation>,
}

/// Parameters for converting a lassoed handwriting selection into text or `FreeText` (plan H.24).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ConvertOptions<'a> {
    /// Zero-based destination page index.
    pub page: usize,
    /// BCP-47 recognition language tag (e.g., `"en"`).
    pub lang: &'a str,
    /// Whether to replace the ink with a `FreeText` annotation or copy text only.
    pub target: ConversionTarget,
    /// Style for the created `FreeText` annotation.
    pub style: Style,
    /// Author name for the created `FreeText` annotation.
    pub author: &'a str,
}

/// Recognizes `strokes` via `recognizer`, formats lines preserving line breaks and bounding box,
/// and optionally builds a single undoable [`Operation`] replacing `source_ink_ids` with a
/// `FreeText` annotation (plan H.24 **Done when**).
///
/// # Errors
///
/// Propagates [`RecognitionError`] if `strokes` is empty or recognition fails.
pub fn convert_lasso_to_text(
    strokes: &[HandwritingStroke],
    source_ink_ids: &[String],
    recognizer: &dyn Recognizer,
    options: ConvertOptions<'_>,
) -> Result<ConversionOutcome, RecognitionError> {
    let page = options.page;
    let words = recognizer.recognize(strokes, options.lang)?;
    if words.is_empty() {
        return Err(RecognitionError::InvalidInput(
            "recognizer returned no words".into(),
        ));
    }

    let mut lines: Vec<(usize, Vec<&RecognizedWord>)> = Vec::new();
    for word in &words {
        if let Some(last) = lines.last_mut()
            && last.0 == word.line_index
        {
            last.1.push(word);
        } else {
            lines.push((word.line_index, vec![word]));
        }
    }

    let text = lines
        .iter()
        .map(|(_, line_words)| {
            line_words
                .iter()
                .map(|w| w.text.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n");

    let bbox = words
        .iter()
        .map(|w| w.bbox)
        .reduce(|a, b| PageRect {
            x0: a.x0.min(b.x0),
            y0: a.y0.min(b.y0),
            x1: a.x1.max(b.x1),
            y1: a.y1.max(b.y1),
        })
        .unwrap_or_default();

    // Ensure FreeText box has comfortable padding and non-zero extent.
    let padded_box = PageRect {
        x0: bbox.x0,
        y0: bbox.y0,
        x1: (bbox.x1 + 8.0).max(bbox.x0 + 24.0),
        y1: (bbox.y1 + 6.0).max(bbox.y0 + 18.0),
    };

    let (freetext, operation) = match options.target {
        ConversionTarget::CopyTextOnly => (None, None),
        ConversionTarget::ReplaceWithFreeText => {
            let mut annot = Annotation::new(
                page,
                Shape::Boxed(Boxed::FreeText, padded_box),
                options.style,
                options.author,
            );
            annot.contents.clone_from(&text);

            let mut changes: Vec<Change> = source_ink_ids
                .iter()
                .map(|id| Change::DeleteAnnotation {
                    page,
                    id: id.clone(),
                })
                .collect();
            changes.push(Change::CreateAnnotation(annot.clone()));

            let op = Operation {
                name: "Convert handwriting to text".into(),
                changes,
            };
            (Some(annot), Some(op))
        }
    };

    Ok(ConversionOutcome {
        text,
        words,
        bbox: padded_box,
        freetext,
        operation,
    })
}

#[cfg(test)]
mod tests {
    use ecclesia_mupdf::PagePoint;

    use super::*;
    use crate::handwriting::index::BackendKind;

    struct MultiLineMockRecognizer;
    impl Recognizer for MultiLineMockRecognizer {
        fn backend(&self) -> BackendKind {
            BackendKind::BuiltinOffline
        }

        fn supported_languages(&self) -> &[&str] {
            &["en"]
        }

        fn recognize(
            &self,
            _strokes: &[HandwritingStroke],
            _lang: &str,
        ) -> Result<Vec<RecognizedWord>, RecognitionError> {
            Ok(vec![
                RecognizedWord {
                    text: "First".into(),
                    candidates: vec!["First".into(), "Fast".into()],
                    confidence: 0.95,
                    line_index: 0,
                    stroke_ids: vec!["a".into()],
                    bbox: PageRect {
                        x0: 50.0,
                        y0: 100.0,
                        x1: 110.0,
                        y1: 120.0,
                    },
                },
                RecognizedWord {
                    text: "line".into(),
                    candidates: vec!["line".into()],
                    confidence: 0.92,
                    line_index: 0,
                    stroke_ids: vec!["b".into()],
                    bbox: PageRect {
                        x0: 120.0,
                        y0: 100.0,
                        x1: 170.0,
                        y1: 120.0,
                    },
                },
                RecognizedWord {
                    text: "Second".into(),
                    candidates: vec!["Second".into()],
                    confidence: 0.96,
                    line_index: 1,
                    stroke_ids: vec!["c".into()],
                    bbox: PageRect {
                        x0: 50.0,
                        y0: 140.0,
                        x1: 130.0,
                        y1: 162.0,
                    },
                },
            ])
        }
    }

    #[test]
    fn conversion_keeps_position_and_line_breaks_in_freetext_operation() {
        let strokes = vec![HandwritingStroke {
            id: "a".into(),
            points: vec![PagePoint { x: 50.0, y: 100.0 }],
            timestamps_ms: Vec::new(),
        }];
        let out = convert_lasso_to_text(
            &strokes,
            &["annot-ink-1".into()],
            &MultiLineMockRecognizer,
            ConvertOptions {
                page: 2,
                lang: "en",
                target: ConversionTarget::ReplaceWithFreeText,
                style: Style::default(),
                author: "Ada",
            },
        )
        .expect("convert");

        assert_eq!(out.text, "First line\nSecond");
        let freetext = out.freetext.expect("freetext annot");
        assert_eq!(freetext.page, 2);
        assert_eq!(freetext.contents, "First line\nSecond");
        match freetext.shape {
            Shape::Boxed(Boxed::FreeText, rect) => {
                assert_eq!(rect.x0, 50.0);
                assert_eq!(rect.y0, 100.0);
                assert!(rect.x1 >= 170.0 && rect.y1 >= 162.0);
            }
            other => panic!("unexpected shape: {other:?}"),
        }
        let op = out.operation.expect("operation");
        assert_eq!(op.changes.len(), 2);
        assert!(matches!(
            op.changes[0],
            Change::DeleteAnnotation { page: 2, .. }
        ));
        assert!(matches!(op.changes[1], Change::CreateAnnotation(_)));
    }
}
