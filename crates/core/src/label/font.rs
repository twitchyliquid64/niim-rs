//! Fonts and 1-bit text rasterisation.

use fontdue::{Font, FontSettings};

use super::LabelError;
use crate::Bitmap;

static REGULAR: &[u8] = include_bytes!("../../fonts/AtkinsonHyperlegible-Regular.ttf");
static BOLD: &[u8] = include_bytes!("../../fonts/AtkinsonHyperlegible-Bold.ttf");

/// Glyph coverage (0–255) at or above which a dot is black. Thermal output has no greys.
const COVERAGE_THRESHOLD: u8 = 128;

/// The fonts used for label text.
pub struct Fonts {
    regular: Font,
    /// `None` = embolden the regular font.
    bold: Option<Font>,
}

impl std::fmt::Debug for Fonts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fonts")
            .field("regular", &self.regular.name())
            .field("bold", &self.bold.as_ref().map(|b| b.name()))
            .finish()
    }
}

fn load(data: &[u8]) -> Result<Font, LabelError> {
    let settings = FontSettings {
        scale: 64.0,
        ..FontSettings::default()
    };
    Font::from_bytes(data, settings).map_err(|e| LabelError::Font(e.to_owned()))
}

impl Fonts {
    /// The built-in font, Atkinson Hyperlegible (regular and bold; SIL Open Font License).
    pub fn builtin() -> Self {
        Self {
            regular: load(REGULAR).expect("built-in font"),
            bold: Some(load(BOLD).expect("built-in font")),
        }
    }

    /// Load TrueType/OpenType font data. Without a bold font, bold text is the regular font
    /// thickened.
    pub fn from_bytes(regular: &[u8], bold: Option<&[u8]>) -> Result<Self, LabelError> {
        Ok(Self {
            regular: load(regular)?,
            bold: bold.map(load).transpose()?,
        })
    }

    /// The font for a style, and how many dots to thicken strokes by (synthetic bold).
    fn face(&self, bold: bool, px: f32) -> (&Font, usize) {
        match (bold, &self.bold) {
            (true, Some(f)) => (f, 0),
            (true, None) => (&self.regular, ((px / 14.0).round() as usize).max(1)),
            (false, _) => (&self.regular, 0),
        }
    }

    /// Whether the font for this style has a glyph for `c`.
    pub(super) fn has_glyph(&self, c: char, bold: bool) -> bool {
        self.face(bold, 16.0).0.has_glyph(c)
    }

    /// Size of a run of text at `px` dots per em: (width, ascent, descent), all in whole dots.
    pub(super) fn measure(&self, text: &str, px: f32, bold: bool) -> (i32, i32, i32) {
        let (font, extra) = self.face(bold, px);
        let (ascent, descent) = font
            .horizontal_line_metrics(px)
            .map_or((px * 0.8, px * 0.2), |m| (m.ascent, -m.descent));
        let mut width = 0.0;
        let mut prev = None;
        for c in text.chars() {
            if let Some(p) = prev {
                width += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            width += font.metrics(c, px).advance_width + extra as f32;
            prev = Some(c);
        }
        (
            width.ceil() as i32,
            ascent.ceil() as i32,
            descent.ceil() as i32,
        )
    }

    /// Draw a run of text with its left edge at `x` and its baseline at `baseline`.
    pub(super) fn draw(
        &self,
        out: &mut Bitmap,
        text: &str,
        px: f32,
        bold: bool,
        x: i32,
        baseline: i32,
    ) {
        let (font, extra) = self.face(bold, px);
        let mut pen = x as f32;
        let mut prev = None;
        for c in text.chars() {
            if let Some(p) = prev {
                pen += font.horizontal_kern(p, c, px).unwrap_or(0.0);
            }
            let (m, coverage) = font.rasterize(c, px);
            let gx = pen.round() as i32 + m.xmin;
            let gy = baseline - m.ymin - m.height as i32;
            for row in 0..m.height {
                for col in 0..m.width {
                    if coverage[row * m.width + col] >= COVERAGE_THRESHOLD {
                        let (px_x, px_y) = (gx + col as i32, gy + row as i32);
                        if px_x >= 0 && px_y >= 0 {
                            for e in 0..=extra {
                                out.set(px_x as usize + e, px_y as usize, true);
                            }
                        }
                    }
                }
            }
            pen += m.advance_width + extra as f32;
            prev = Some(c);
        }
    }
}
