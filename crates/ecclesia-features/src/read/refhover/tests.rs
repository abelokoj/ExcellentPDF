//! SumatraPDF Enhanced's RefHover tests (`src/base/tests/RefHover_ut.cpp`), ported one for one: synthetic pages of
//! fixed-width glyphs, as the engine would give them.

use super::cite::{detect_author_year, detect_numeric, find_numeric_reference, find_surname};
use super::layout::{
    Edge, EntryRegion, destination_region, detect_entry_box, detect_equation_box, edge_hit,
    landscape_box, normalize_glyph_lines, should_resolve_dest_y, should_search_next_page,
    sized_region, strip_watermark_glyphs,
};
use super::{PageText, Rect, RectF};

const PAGE_W: f32 = 612.0;
const PAGE_H: f32 = 792.0;
const CHAR_W: i32 = 6;
const LINE_H: i32 = 12;

/// Adds `s` at (x, y), one fixed-width glyph per character.
fn add(t: &mut PageText, s: &str, x: i32, y: i32) {
    for (i, c) in s.chars().enumerate() {
        t.push(c, Rect::new(x + i as i32 * CHAR_W, y, CHAR_W, LINE_H));
    }
}

fn page(lines: &[(&str, i32, i32)]) -> PageText {
    let mut t = PageText::new();
    for (s, x, y) in lines {
        add(&mut t, s, *x, *y);
    }
    t
}

fn mediabox() -> RectF {
    RectF::new(0.0, 0.0, PAGE_W, PAGE_H)
}

fn entry(t: &PageText, x: f32, y: f32) -> RectF {
    detect_entry_box(t, mediabox(), x, y).map_or_else(RectF::default, |e| e.region)
}

fn surname_at(t: &PageText, x: i32, y: i32) -> Option<(String, u16)> {
    detect_author_year(t, x, y).map(|c| (c.surname, c.year))
}

fn number_at(t: &PageText, x: i32, y: i32) -> Option<u32> {
    detect_numeric(t, x, y).map(|c| c.number)
}

#[test]
fn sparse_text_returns_whole_page() {
    let t = page(&[("Heading only", 100, 100)]);
    assert_eq!(
        entry(&t, 100.0, 100.0),
        RectF::new(0.0, 0.0, PAGE_W, PAGE_H)
    );
}

#[test]
fn negative_dest_y_falls_to_landscape() {
    let mut t = PageText::new();
    for i in 0..10 {
        add(&mut t, "Body text line content here.", 72, 100 + i * 14);
    }
    let b = entry(&t, 72.0, -1.0);
    assert_eq!((b.x, b.y, b.dx), (0.0, 0.0, PAGE_W));
    assert!(b.dy > 0.0 && b.dy < PAGE_H);
}

#[test]
fn dest_before_footer_is_empty() {
    let t = page(&[
        ("body text ending before the destination", 52, 380),
        ("the final line on this page is here", 52, 394),
        ("231", 200, 500),
    ]);
    assert_eq!(detect_entry_box(&t, mediabox(), 52.0, 420.0), None);
}

#[test]
fn late_dest_searches_next_page() {
    let p = RectF::new(0.0, 0.0, 419.53, 558.43);
    assert!(!should_search_next_page(p, 350.0));
    assert!(should_search_next_page(p, 409.53));
}

#[test]
fn bracket_entry_fits_to_one_entry() {
    let t = page(&[
        ("[Foo10] Smith J., 2010, Some title.", 72, 200),
        ("continuation line of the entry.", 92, 215),
        ("[Bar11] Doe J., 2011, Another title.", 72, 240),
        ("continuation line of the entry.", 92, 255),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.bottom() < 240.0);
    assert!(b.y <= 206.0 && b.y >= 188.0);
}

#[test]
fn author_year_entry_fits_to_one_entry() {
    let t = page(&[
        ("Smith, J. (2010). Some title here.", 72, 200),
        ("Journal of Things, 12(3), 45-67.", 92, 215),
        ("Doe, A. (2011). Another work title.", 72, 240),
        ("Other Journal, 4(2), 89-101.", 92, 255),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.y <= 200.0 && b.y >= 188.0);
    assert!(b.bottom() > 215.0 && b.bottom() < 240.0);
    assert!(b.dx < PAGE_W);
}

#[test]
fn single_line_entry_list() {
    let t = page(&[
        ("JVM Java Virtual Machine. 19, 36", 72, 200),
        ("LLM Large Language Model. 45", 72, 215),
        ("PDF Portable Document Format. 7", 72, 230),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.bottom() < 227.0);
    assert!(b.dx < PAGE_W);
}

#[test]
fn gap_separated_entry_list() {
    let t = page(&[
        ("First footnote text goes here.", 72, 200),
        ("Second footnote starts here.", 72, 240),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.bottom() < 240.0);
    assert!(b.dx < PAGE_W);
}

#[test]
fn two_column_right_entry_stays_in_column() {
    let mut t = PageText::new();
    for i in 0..5 {
        add(&mut t, "left column body text line...", 72, 200 + i * 15);
    }
    add(&mut t, "[Foo10] Smith J., 2010, Title.", 320, 200);
    add(&mut t, "continuation of the entry.", 340, 215);
    add(&mut t, "[Bar11] Doe J., 2011, Other.", 320, 240);
    let b = entry(&t, 320.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.x > 246.0);
    assert!(b.bottom() < 240.0);
}

#[test]
fn two_column_left_entry_stays_in_column() {
    let mut t = page(&[
        ("[Foo10] Smith J., 2010, Title", 72, 200),
        ("continuation of the entry.", 92, 215),
        ("[Bar11] Doe J., 2011, Other.", 72, 240),
    ]);
    for i in 0..5 {
        add(&mut t, "right column body text line..", 340, 200 + i * 15);
    }
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.right() < 340.0);
    assert!(b.bottom() < 240.0);
}

#[test]
fn column_wrap_continuation_detected() {
    let t = page(&[
        ("[63] Voelter M: DSL Engineering des", 72, 762),
        ("Languages dslbook org Germany 2013", 340, 40),
        ("[64] Moretti N Xie X et al Title", 340, 65),
    ]);
    let EntryRegion {
        region,
        continuation,
    } = detect_entry_box(&t, mediabox(), 72.0, 762.0).expect("an entry");
    assert!(!region.is_empty());
    assert!(region.x < 246.0);
    assert!(region.y <= 768.0);
    let c = continuation.expect("the entry goes on in the next column");
    assert!(!c.is_empty());
    assert!(c.x > 246.0);
    assert!(c.y <= 46.0);
    assert!(c.bottom() < 65.0);
}

#[test]
fn column_wrap_continuation_rejects_unrelated_body_text() {
    let mut t = page(&[("[47] Zhang X Wang Y Wang J Title A", 72, 750)]);
    for i in 0..6 {
        add(
            &mut t,
            "unrelated running body text here.",
            340,
            40 + i * 14,
        );
    }
    add(&mut t, "[48] Li H Hou L Wang X Guo H Title", 340, 130);
    let found = detect_entry_box(&t, mediabox(), 72.0, 750.0).expect("an entry");
    assert!(!found.region.is_empty());
    assert_eq!(found.continuation, None);
}

#[test]
fn accented_all_caps_heading_detected() {
    let mut t = page(&[
        ("SECCIÓN 2 RESULTADOS", 72, 200),
        ("texto del cuerpo del documento.", 92, 215),
        ("continua en el margen izquierdo.", 72, 230),
    ]);
    for i in 0..3 {
        add(&mut t, "mas lineas de texto del cuerpo.", 72, 245 + i * 15);
    }
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert_eq!((b.x, b.dx), (0.0, PAGE_W));
}

#[test]
fn equation_label_detected() {
    let mut t = page(&[("dH/dt = -sum( ... )", 200, 300), ("(14)", 540, 300)]);
    for i in 0..3 {
        add(
            &mut t,
            "Paragraph text below the equation here.",
            72,
            330 + i * 14,
        );
    }
    let b = detect_equation_box(&t, mediabox(), 300.0).expect("an equation");
    assert_eq!((b.x, b.dx), (0.0, PAGE_W));
    assert!(b.y < 300.0);
    assert!(b.bottom() < 300.0 + 3.0 * LINE_H as f32 + 20.0);
}

#[test]
fn body_text_paren_rejected() {
    let t = page(&[
        ("some body text on another line.", 72, 280),
        ("(14)", 80, 300),
    ]);
    assert_eq!(detect_equation_box(&t, mediabox(), 300.0), None);
}

#[test]
fn non_trailing_paren_rejected() {
    let t = page(&[
        ("dH/dt = ...", 200, 300),
        ("(14)", 540, 300),
        ("trail", 560, 300),
    ]);
    assert_eq!(detect_equation_box(&t, mediabox(), 300.0), None);
}

#[test]
fn bib_year_paren_not_equation_label() {
    let t = page(&[("O. Zimmermann Decisions IGI Global (2013)", 320, 300)]);
    assert_eq!(detect_equation_box(&t, mediabox(), 300.0), None);
    let t = page(&[("some eq body", 200, 300), ("(14)", 540, 300)]);
    assert!(detect_equation_box(&t, mediabox(), 300.0).is_some());
}

#[test]
fn empty_inputs_handled() {
    let t = PageText::new();
    assert_eq!(detect_equation_box(&t, mediabox(), 100.0), None);
    assert_eq!(detect_equation_box(&t, mediabox(), -1.0), None);
}

#[test]
fn french_caption_detected() {
    let mut t = page(&[("Tableau 2: Donnees", 72, 300)]);
    for i in 0..5 {
        add(&mut t, "Paragraphe de texte courant.", 72, 320 + i * 14);
    }
    let b = landscape_box(mediabox(), 300.0, &t);
    assert!(b.y < 250.0);
    assert_eq!(b.dx, PAGE_W);
}

#[test]
fn italian_portuguese_caption_detected() {
    for caption in ["Tabella 2: Dati", "Tabela 2: Dados"] {
        let mut t = page(&[(caption, 72, 300)]);
        for i in 0..5 {
            add(&mut t, "testo del corpo del documento.", 72, 320 + i * 14);
        }
        assert!(landscape_box(mediabox(), 300.0, &t).y < 250.0, "{caption}");
    }
}

#[test]
fn plural_heading_word_not_matched() {
    let t = page(&[
        ("Sections of papers, J. Smith.", 72, 200),
        ("continuation line of the entry.", 92, 215),
        ("Another entry begins here now.", 72, 240),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.dx < PAGE_W);
    assert!(b.bottom() < 240.0);
}

#[test]
fn caption_extension_picks_topmost() {
    let mut t = PageText::new();
    for i in 0..3 {
        add(&mut t, "body paragraph at the dest.", 72, 100 + i * 14);
    }
    // The far caption comes first in the stream; the leading space stands for real extraction's block separator.
    add(&mut t, " Figure 9: far away caption", 72, 700);
    add(&mut t, " Figure 2: near caption", 72, 420);
    let b = landscape_box(mediabox(), 100.0, &t);
    assert!(b.bottom() > 420.0 && b.bottom() < 600.0);
}

#[test]
fn landscape_box_basic_shape() {
    let mut t = PageText::new();
    for i in 0..5 {
        add(&mut t, "some body content.", 72, 400 + i * 14);
    }
    let b = landscape_box(mediabox(), 400.0, &t);
    assert_eq!((b.x, b.dx), (0.0, PAGE_W));
    assert!(b.y >= 0.0 && b.y < 400.0);
    assert!(b.dy > 0.0 && b.bottom() <= PAGE_H);
}

#[test]
fn plain_text_citation_detected() {
    let t = page(&[("as shown in (Smith et al., 2020) earlier", 72, 200)]);
    assert_eq!(surname_at(&t, 152, 206), Some(("Smith".to_owned(), 2020)));
}

#[test]
fn plain_text_citation_no_year() {
    let t = page(&[("plain body text without any citation", 72, 200)]);
    assert_eq!(surname_at(&t, 100, 206), None);
}

#[test]
fn plain_text_citation_rejects_dates() {
    let t = page(&[(
        "Revision: 15 October 2026, 03:30:04PM (America/New_York)",
        72,
        200,
    )]);
    assert_eq!(surname_at(&t, 152, 206), None);
}

#[test]
fn plain_text_citation_requires_hit() {
    let t = page(&[("ordinary words (Smith, 2020) following text", 72, 200)]);
    for (x, y) in [(80, 206), (300, 206), (180, 180)] {
        assert_eq!(surname_at(&t, x, y), None, "({x}, {y})");
    }
}

#[test]
fn narrative_citation_detected() {
    let t = page(&[("as Smith (2020) explains", 72, 200)]);
    assert_eq!(surname_at(&t, 92, 206), Some(("Smith".to_owned(), 2020)));
}

#[test]
fn citation_lookup_skips_source() {
    let source = Some(Rect::new(72, 100, 90, 12));
    let mut t = page(&[("(Smith, 2020)", 72, 100)]);
    assert_eq!(find_surname(&t, "Smith", 2020, source), None);
    add(&mut t, "See (Smith, 2020).", 72, 200);
    add(&mut t, "Smith, J. (2020). A genuine reference.", 72, 300);
    assert_eq!(
        find_surname(&t, "Smith", 2020, source).map(|a| a.y),
        Some(300.0)
    );

    let mut t = page(&[("[7]", 72, 100)]);
    assert_eq!(find_numeric_reference(&t, 7, source), None);
    add(&mut t, "[7] A genuine reference.", 72, 300);
    assert_eq!(
        find_numeric_reference(&t, 7, source).map(|a| a.y),
        Some(300.0)
    );
}

#[test]
fn numeric_citation_requires_hit() {
    let t = page(&[("[7]", 72, 200)]);
    assert_eq!(number_at(&t, 75, 180), None);
    assert_eq!(number_at(&t, 75, 206), Some(7));
}

#[test]
fn surname_found_on_bib_page() {
    let t = page(&[
        ("References", 72, 100),
        ("Smith, J. (2020). Some title.", 72, 130),
        ("continuation of the entry.", 90, 145),
    ]);
    let a = find_surname(&t, "Smith", 2020, None).expect("Smith's entry");
    assert_eq!((a.x, a.y), (72.0, 130.0));
    assert_eq!(find_surname(&t, "Jones", 2021, None), None);
}

#[test]
fn numeric_citation_detected() {
    assert_eq!(
        number_at(&page(&[("see [1] for details", 72, 200)]), 105, 206),
        Some(1)
    );
    assert_eq!(
        number_at(&page(&[("prior work [1, 2] showed", 72, 200)]), 165, 206),
        Some(2)
    );
    assert_eq!(
        number_at(&page(&[("no brackets here at all", 72, 200)]), 100, 206),
        None
    );
}

#[test]
fn numeric_citation_source_distinct_on_line() {
    let t = page(&[("see [1] and again [1] end", 72, 200)]);
    let first = detect_numeric(&t, 105, 206).expect("the first [1]");
    let second = detect_numeric(&t, 189, 206).expect("the second [1]");
    assert_eq!((first.number, second.number), (1, 1));
    assert!(first.source.dx > 0 && second.source.dx > 0);
    assert_eq!(first.source.y, second.source.y);
    assert_ne!(first.source.x, second.source.x);
}

#[test]
fn numeric_citation_range_en_dash() {
    let t = page(&[("deploy [2, 9\u{2013}14].", 72, 200)]);
    assert_eq!(number_at(&t, 141, 206), Some(9));
    assert_eq!(number_at(&t, 159, 206), Some(14));
}

#[test]
fn numeric_citation_line_break_list() {
    let t = page(&[("in AECO [8, 17,", 72, 200), ("18]. These", 72, 220)]);
    assert_eq!(number_at(&t, 129, 206), Some(8));
    assert_eq!(number_at(&t, 75, 224), Some(18));
    assert_eq!(number_at(&t, 147, 206), Some(17));
}

/// The index of the first glyph `c` at (x, y), if any.
fn glyph_at(t: &PageText, c: char, x: Option<i32>, y: i32) -> Option<usize> {
    (0..t.len()).find(|i| {
        t.chars()[*i] == c && t.rects()[*i].y == y && x.is_none_or(|x| t.rects()[*i].x == x)
    })
}

#[test]
fn numeric_citation_wrap_end_of_line() {
    let t = page(&[
        ("a first line of text ending in [42,", 72, 200),
        ("43] maintain live text here", 72, 224),
    ]);
    assert!(glyph_at(&t, '4', Some(72), 224).is_some_and(|i| i > 0));
    assert_eq!(number_at(&t, 75, 230), Some(43));
    let i42 = (0..t.len())
        .find(|i| t.chars()[*i] == '4' && t.rects()[*i].y == 200 && t.chars()[i + 1] == '2')
        .expect("the 42");
    assert_eq!(number_at(&t, t.rects()[i42].x + 3, 206), Some(42));
}

#[test]
fn numeric_citation_wrap_out_of_order() {
    // Unrelated text far above comes between the two halves in the stream, as in the real document.
    let t = page(&[
        ("runtime models [42,", 72, 200),
        ("unrelated heading text far above", 72, 4),
        ("43] maintain live", 72, 220),
    ]);
    assert!(glyph_at(&t, '4', Some(72), 220).is_some_and(|i| i > 0));
    assert_eq!(number_at(&t, 75, 226), Some(43));
}

#[test]
fn numeric_citation_wrap_two_column() {
    let t = page(&[
        ("left column line one ending [42,", 72, 200),
        ("43] left column line two here", 72, 220),
        ("right column first line text", 340, 200),
        ("right column second line text", 340, 220),
    ]);
    assert!(glyph_at(&t, '4', Some(72), 220).is_some_and(|i| i > 0));
    assert_eq!(number_at(&t, 75, 226), Some(43));
}

#[test]
fn plain_text_citation_source_distinct_on_line() {
    let t = page(&[("per (Smith, 2020) and (Jones, 2021) here", 72, 200)]);
    let first = detect_author_year(&t, 110, 206).expect("Smith");
    assert_eq!((first.surname.as_str(), first.year), ("Smith", 2020));
    let second = detect_author_year(&t, 224, 206).expect("Jones");
    assert_eq!((second.surname.as_str(), second.year), ("Jones", 2021));
    assert!(first.source.dx > 0 && second.source.dx > 0);
    assert_eq!(first.source.y, second.source.y);
    assert_ne!(first.source.x, second.source.x);
}

#[test]
fn numeric_reference_found_on_bib_page() {
    let t = page(&[
        ("References", 72, 100),
        ("[1] A. Trentin, Some title 2025.", 72, 130),
        ("[2] B. Other, Another title 2024.", 72, 150),
    ]);
    let a = find_numeric_reference(&t, 1, None).expect("[1]");
    assert_eq!((a.x, a.y), (72.0, 130.0));
    assert_eq!(
        find_numeric_reference(&t, 2, None).map(|a| a.y),
        Some(150.0)
    );
    assert_eq!(find_numeric_reference(&t, 3, None), None);
}

#[test]
fn hanging_indent_narrow_label_full_width() {
    let t = page(&[
        ("[TA05]", 72, 200),
        ("J. Tyree and A. Akerman. Architecture decisions.", 130, 200),
        ("continuation line two of the same entry.", 130, 215),
        ("[Vai20]", 72, 240),
        ("Thomas Vaillant. Log4brains. 2020.", 130, 240),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.right() >= 300.0);
    assert!(b.bottom() < 240.0);
    assert!(b.x <= 78.0);
}

#[test]
fn two_column_hanging_indent_stays_in_column() {
    let mut t = page(&[
        ("[TA05]", 72, 200),
        ("Smith J. 2010. Some title.", 130, 200),
        ("continuation of the entry.", 130, 215),
        ("[Vai20]", 72, 240),
    ]);
    for i in 0..4 {
        add(&mut t, "right column body text line..", 340, 200 + i * 15);
    }
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.right() < 340.0);
    assert!(b.bottom() < 240.0);
}

#[test]
fn last_entry_trailing_footer_trimmed() {
    let t = page(&[
        ("[Zim25]", 72, 600),
        ("Olaf Zimmermann. ADG: A Light Tool.", 130, 600),
        ("continuation of the last entry.", 130, 615),
        ("13", 300, 700),
    ]);
    let b = entry(&t, 72.0, 600.0);
    assert!(!b.is_empty());
    assert!(b.bottom() < 650.0);
}

#[test]
fn normalize_glyph_lines_flattens_line() {
    let out = normalize_glyph_lines(&[
        Rect::new(72, 100, 6, 10),
        Rect::new(78, 105, 4, 5),
        Rect::new(72, 120, 6, 10),
    ]);
    assert_eq!((out[0].y, out[0].dy), (100, 10));
    assert_eq!((out[1].y, out[1].dy), (100, 10));
    assert_eq!((out[2].y, out[2].dy), (120, 10));
}

#[test]
fn variable_glyph_tops_entry_not_hijacked() {
    let mut t = PageText::new();
    // The previous entry's last line: digits topped at 305, its full stop at 310, one baseline.
    for (i, c) in "1622292".chars().enumerate() {
        t.push(c, Rect::new(130 + i as i32 * 6, 305, 6, 8));
    }
    t.push('.', Rect::new(130 + 7 * 6, 310, 4, 3));
    for (i, c) in "[Buc+23]".chars().enumerate() {
        t.push(c, Rect::new(72 + i as i32 * 6, 316, 6, 11));
    }
    for (i, c) in "Georg Buchgeher et al. Using ADRs in Open Source."
        .chars()
        .enumerate()
    {
        t.push(c, Rect::new(130 + i as i32 * 6, 316, 6, 9));
    }
    for (i, c) in "[JB05] A. Jansen and J. Bosch.".chars().enumerate() {
        t.push(c, Rect::new(72 + i as i32 * 6, 352, 6, 10));
    }
    let norm = PageText::from_parts(t.chars().to_vec(), normalize_glyph_lines(t.rects()));
    let b = entry(&norm, 72.0, 312.0);
    assert!(!b.is_empty());
    assert!(b.y > 305.0);
    assert!(b.x <= 78.0);
    assert!(b.right() >= 300.0);
    assert!(b.bottom() < 352.0);
}

#[test]
fn two_column_numeric_left_entry_not_hijacked() {
    let t = page(&[
        ("[1] First left column entry.", 72, 600),
        ("[2] Anvaari Zimmerman", 72, 628),
        ("second line of entry two", 72, 642),
        ("[26] Zimmermann Miksovic Decisions", 244, 624),
        ("connecting enterprise architects", 244, 638),
        ("[27] Zimmermann Zdun Combining", 244, 660),
    ]);
    let b = entry(&t, 72.0, 628.0);
    assert!(!b.is_empty());
    assert!(b.x <= 78.0);
    assert!(b.y <= 630.0);
    assert!(b.right() < 244.0);
    assert!(b.bottom() < 672.0);
}

#[test]
fn two_column_numeric_reference_found() {
    let t = page(&[
        ("[1] Left column one.", 72, 100),
        ("[2] Left column two.", 72, 120),
        ("[3] Right column three.", 320, 100),
        ("[4] Right column four.", 320, 120),
    ]);
    let at = |n| find_numeric_reference(&t, n, None).map(|a| (a.x, a.y));
    assert_eq!(at(4), Some((320.0, 120.0)));
    assert_eq!(at(3), Some((320.0, 100.0)));
    assert_eq!(at(1), Some((72.0, 100.0)));
    assert_eq!(at(9), None);
    let body = page(&[("as reported in [2] by others", 72, 200)]);
    assert_eq!(find_numeric_reference(&body, 2, None), None);
}

#[test]
fn two_column_wide_second_line_stays_in_column() {
    let t = page(&[
        ("[5] Short label.", 72, 200),
        ("http://example.com/a/very/long/url", 72, 214),
        ("[9] Right one.", 300, 200),
        ("right two.", 300, 214),
    ]);
    let b = entry(&t, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.x <= 78.0);
    assert!(b.right() >= 250.0 && b.right() < 300.0);
}

/// Adds a diagonal watermark: tall, wide glyphs stepping right 14 pt and down 6 pt, each on its own baseline.
fn add_watermark(t: &mut PageText, s: &str) {
    for (i, c) in s.chars().enumerate() {
        let i = i as i32;
        t.push(c, Rect::new(190 + i * 14, 180 + i * 6, 20, 40));
    }
}

#[test]
fn strip_watermark_removes_diagonal_stamp() {
    let mut t = page(&[
        ("normal body text line one here", 72, 200),
        ("normal body text line two here", 72, 214),
        ("normal body text line three xx", 72, 228),
    ]);
    let body = t.len();
    add_watermark(&mut t, "UNDERREVIE");
    let kept = strip_watermark_glyphs(&t);
    assert_eq!(kept.len(), body);
    assert!(kept.rects().iter().all(|r| r.dy == LINE_H));

    // A heading of tall glyphs on one baseline is no watermark.
    let mut t = page(&[("normal body text line one here", 72, 200)]);
    for (i, c) in "HEADING!".chars().enumerate() {
        t.push(c, Rect::new(72 + i as i32 * 14, 150, 12, 30));
    }
    assert_eq!(strip_watermark_glyphs(&t).len(), t.len());
}

#[test]
fn two_column_watermark_stays_in_column() {
    let mut t = page(&[
        ("[12] Sample left column entry", 72, 200),
        ("more left column entry text..", 72, 215),
    ]);
    for i in 0..4 {
        add(&mut t, "right column body text line..", 340, 200 + i * 15);
    }
    add_watermark(&mut t, "UNDERREVIEW.");
    let clean = strip_watermark_glyphs(&t);
    let b = entry(&clean, 72.0, 200.0);
    assert!(!b.is_empty());
    assert!(b.x <= 78.0);
    assert!(b.right() < 340.0);
}

#[test]
fn explicit_dest_coordinates() {
    let p = mediabox();
    assert_eq!(
        destination_region(p, 320.0, 200.0),
        Some(RectF::new(320.0, 200.0, PAGE_W - 320.0, PAGE_H - 200.0))
    );
    assert!(!should_resolve_dest_y(p, 0.0));
    assert!(should_resolve_dest_y(p, -1.0));
    assert!(should_resolve_dest_y(p, PAGE_H + 1.0));
}

#[test]
fn preview_resize_edges() {
    let w = Rect::new(100, 200, 400, 300);
    assert_eq!(edge_hit(w, 100, 200, 8), Edge::TopLeft);
    assert_eq!(edge_hit(w, 499, 200, 8), Edge::TopRight);
    assert_eq!(edge_hit(w, 100, 499, 8), Edge::BottomLeft);
    assert_eq!(edge_hit(w, 499, 499, 8), Edge::BottomRight);
    assert_eq!(edge_hit(w, 100, 350, 8), Edge::Left);
    assert_eq!(edge_hit(w, 499, 350, 8), Edge::Right);
    assert_eq!(edge_hit(w, 300, 200, 8), Edge::Top);
    assert_eq!(edge_hit(w, 300, 499, 8), Edge::Bottom);
    assert_eq!(edge_hit(w, 300, 350, 8), Edge::Inside);
    assert_eq!(edge_hit(w, 99, 200, 8), Edge::Outside);
    let region = RectF::new(320.0, 200.0, 60.0, 80.0);
    assert_eq!(
        sized_region(mediabox(), region, (400, 300), 2.0),
        Some(RectF::new(320.0, 200.0, 200.0, 150.0))
    );
    let big = sized_region(mediabox(), region, (2000, 2000), 1.0).expect("a region");
    assert!(big.right() <= PAGE_W && big.bottom() <= PAGE_H);
}

/// A made-up document: its pages' text, all one size.
struct Doc(Vec<PageText>);

impl super::resolve::Pages for Doc {
    fn count(&self) -> usize {
        self.0.len()
    }

    fn text(&mut self, page: usize) -> Option<PageText> {
        self.0.get(page).cloned()
    }

    fn mediabox(&mut self, page: usize) -> Option<RectF> {
        (page < self.0.len()).then(mediabox)
    }
}

fn paper() -> Doc {
    let mut body = page(&[("Earlier work [12] showed this.", 72, 200)]);
    for i in 0..6 {
        add(
            &mut body,
            "Body text of the paper goes on here.",
            72,
            230 + i * 14,
        );
    }
    let mut refs = page(&[("References", 72, 80)]);
    for (i, n) in (10..15).enumerate() {
        let i = i as i32;
        add(
            &mut refs,
            &format!("[{n}] Author {n}, A title of work {n}."),
            72,
            120 + i * 30,
        );
        add(
            &mut refs,
            "continuation of this entry here.",
            92,
            134 + i * 30,
        );
    }
    Doc(vec![body, refs])
}

#[test]
fn hover_numeric_without_link_shows_entry() {
    use super::resolve::{Lookups, shown};
    let mut doc = paper();
    let mut lookups = Lookups::new();
    // "[12]" starts at x = 72 + 13 * 6; the pointer sits on its "1".
    let found = lookups
        .find(&mut doc, 0, 72 + 14 * 6 + 3, 206)
        .expect("[12] and its entry");
    assert_eq!(
        (found.target.page, found.target.x, found.target.y),
        (1, 72.0, 180.0)
    );
    let s = shown(&mut doc, 0, found.source.into(), found.target).expect("a region");
    assert_eq!(s.page, 1);
    assert!(s.region.y <= 186.0 && s.region.y >= 168.0);
    assert!(
        // Entry [12] runs from 180 to 206, padded by 6; entry [13]'s text starts at 210 and ends at 222.
        s.region.bottom() >= 206.0 && s.region.bottom() < 222.0,
        "{s:?}"
    );
    // The second hover comes from the cache, even with the pages gone.
    let mut gone = Doc(vec![doc.0[0].clone(), PageText::new()]);
    assert_eq!(
        lookups.find(&mut gone, 0, 72 + 14 * 6 + 3, 206),
        Some(found)
    );
}

#[test]
fn hover_misses_unlisted_number() {
    let mut doc = paper();
    doc.0[0] = page(&[("Earlier work [99] showed this.", 72, 200)]);
    assert_eq!(
        super::resolve::Lookups::new().find(&mut doc, 0, 72 + 14 * 6 + 3, 206),
        None
    );
}

#[test]
fn dest_y_prefers_a_match_that_starts_its_line() {
    use super::resolve::{Match, dest_y_from_source};
    let src = page(&[("see Eq. (14) above", 72, 100)]);
    let dest = page(&[
        ("the value 14 appears", 72, 300),
        ("(14)", 540, 400),
        ("14 begins this line", 72, 500),
    ]);
    // As in Enhanced: a match with nothing left of it on its line wins; "(14)" has its "(" to the left.
    // The source box covers "(14)": x 72 + 8 * 6.
    let source = RectF::new(120.0, 100.0, 24.0, 12.0);
    assert_eq!(
        dest_y_from_source(&src, source, &dest, Match::Best),
        Some(500.0)
    );
    assert_eq!(
        dest_y_from_source(&src, RectF::default(), &dest, Match::Any),
        None
    );
}

#[test]
fn popover_zoom_fits_room() {
    use super::resolve::{Room, Shown, size, zoom};
    let shown = Shown {
        page: 0,
        region: RectF::new(0.0, 0.0, 400.0, 100.0),
        continuation: None,
    };
    let room = Room {
        screen_width: 1920.0,
        page: RectF::new(0.0, 0.0, 900.0, 1200.0),
        pointer_y: 600.0,
        scale: 1.0,
    };
    assert_eq!(zoom(&shown, 1.0, room), 1.0);
    // 400 pt at 3x is 1200 px, wider than 95 % of the page less the frame: shrink to fit.
    let fit = zoom(&shown, 3.0, room);
    assert!((fit - (900.0 * 0.95 - 8.0) / 400.0).abs() < 1e-4);
    let tiny = Room {
        screen_width: 100.0,
        page: RectF::new(0.0, 0.0, 50.0, 50.0),
        pointer_y: 25.0,
        scale: 1.0,
    };
    assert_eq!(zoom(&shown, 1.0, tiny), size::MIN_ZOOM);
}
