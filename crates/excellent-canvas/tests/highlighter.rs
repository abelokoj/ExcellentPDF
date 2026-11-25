//! H.3: text under the highlighter stays fully readable in light and dark mode. The page is drawn black on white with
//! the highlighter's ink multiplied over it, as MuPDF draws the saved annotation, then mapped to each reading theme's
//! colours as the canvas maps its tiles; the text must keep WCAG AA contrast with the highlighted paper in each.

use excellent_core::darkmode::themes::contrast;
use excellent_core::darkmode::{Pictures, ReadingTheme, recolor};
use excellent_ink::pens::PenKind;

/// WCAG AA's contrast for body text.
const AA: f32 = 4.5;

fn rgb(color: u32) -> [f32; 3] {
    [16, 8, 0].map(|shift| f32::from(((color >> shift) & 0xff) as u8) / 255.0)
}

fn byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[test]
fn text_under_the_highlighter_stays_readable_in_every_theme() {
    let pen = PenKind::Highlighter.pen(3.0);
    let ink = rgb(pen.color);
    let text = pen.blend.over([0.0; 3], ink, pen.opacity).map(byte);
    let paper = pen.blend.over([1.0; 3], ink, pen.opacity).map(byte);
    assert_eq!(text, [0; 3], "multiplied, black text stays black");
    assert!(contrast(text, paper) >= AA, "as drawn");
    for theme in ReadingTheme::ALL {
        let Some(palette) = theme.palette() else {
            continue;
        };
        let mut pixels = [text, paper].concat();
        recolor(
            &mut pixels,
            2,
            1,
            6,
            3,
            &palette.page_colors(Pictures::Recolor),
            None,
        );
        let (text, paper) = (
            [pixels[0], pixels[1], pixels[2]],
            [pixels[3], pixels[4], pixels[5]],
        );
        let ratio = contrast(text, paper);
        assert!(
            ratio >= AA,
            "{theme:?}: text {text:?} on {paper:?} is only {ratio:.1}:1"
        );
    }
}
