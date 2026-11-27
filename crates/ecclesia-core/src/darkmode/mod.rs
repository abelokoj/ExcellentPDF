//! Smart dark mode (plan R.14): pages drawn in the theme's ink and paper, with their pictures kept as they are.
//!
//! This is the path SumatraPDF Enhanced ships by default ("document colors follow theme: smart", its
//! `PreserveImages` mode): the page renders as usual, then each tile's pixels are mapped from black-on-white to
//! ink-on-paper (`recolor`), except one region per page that holds a photo or artwork (`region`, judged from a small
//! sample of each image, `stats`). Enhanced's object-level device (text and vector colours mapped through OKLab
//! while the page is drawn) is opt-in there and not ported yet.
//!
//! `page` ties the model to MuPDF: it lists a page's images once and samples each the first time a rule needs it.
//! `themes` holds the reading themes (R.15), the ink and paper the pages can be drawn in.

mod page;
pub mod paper;
mod recolor;
mod region;
mod stats;
pub mod themes;

pub use page::PageImages;
pub use recolor::recolor;
pub use region::{ImageOnPage, kept_region};
pub use stats::SampleStats;
pub use themes::{Palette, ReadingTheme};

/// A colour, `[r, g, b]`.
pub type Rgb = [u8; 3];

/// What happens to the pictures on a page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Pictures {
    /// Photos and artwork keep their colours; text, lines and paper follow the theme (Enhanced's "smart").
    #[default]
    Keep,
    /// Everything is recoloured, pictures too (Enhanced's "legacy").
    Recolor,
}

/// The colours pages are drawn in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PageColors {
    /// What black becomes.
    pub ink: Rgb,
    /// What white becomes.
    pub paper: Rgb,
    /// What blue link text becomes; `None` leaves it to the ink-and-paper mapping.
    pub link: Option<Rgb>,
    /// Whether pictures keep their colours.
    pub pictures: Pictures,
}

impl PageColors {
    /// True when drawing in these colours changes nothing: black on white, links and pictures as they are.
    pub fn is_plain(&self) -> bool {
        self.ink == [0, 0, 0] && self.paper == [255, 255, 255] && self.link.is_none()
    }
}
