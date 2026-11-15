//! Labels from a line of markup: text, QR codes, barcodes, AprilTags and images, laid out to
//! fit.
//!
//! ```
//! use niim_core::label::{Fonts, Label, Page, no_images, render};
//! use niim_core::{LabelSize, Margins};
//!
//! let label = Label::parse(r"[qr:https://example.com][col][left][b]Shelf 4[/b]\nBin 12")?;
//! // With a connected printer, `printer.page(size)` supplies this.
//! let page = Page::new(LabelSize::new(40.0, 12.0), 203.0 / 25.4, Margins::default());
//! let rendered = render(&label, &page, &Fonts::builtin(), &no_images)?;
//! assert_eq!((rendered.bitmap.width(), rendered.bitmap.height()), (320, 96));
//! // printer.print(&rendered.bitmap, &PrintOptions::default()).await?;
//! # Ok::<(), niim_core::label::LabelError>(())
//! ```
//!
//! # Markup
//!
//! Plain text prints as it is. `\n` (the two characters) or a real newline starts a new line.
//! Tags in square brackets change what follows; write `\[`, `\]` and `\\` for literal
//! characters.
//!
//! | Tag | Effect |
//! |---|---|
//! | `[left]` `[center]` `[right]` | Horizontal alignment from here on. Used mid-line, it starts a new part of the same line: `[left]Name[right]Date`. Default: centre. |
//! | `[top]` `[middle]` `[bottom]` | Vertical alignment of the column. Default: middle. |
//! | `[small]` `[normal]` `[big]` | Text size relative to the auto-fitted size (0.7×, 1×, 1.5×). |
//! | `[size:4mm]` | A fixed font size (`mm`, `pt` or `px` = dots), exempt from auto-fit. |
//! | `[b]` … `[/b]` | Bold. |
//! | `[col]` | Start a new column to the right. |
//! | `[qr:DATA]` | QR code. |
//! | `[code128:DATA]` `[ean13:DIGITS]` `[ean8:DIGITS]` `[upca:DIGITS]` | Barcodes. Check digits are added if missing and verified if given. |
//! | `[apriltag:ID]` | AprilTag, family tag36h11; `[apriltag 25h9:ID]` and `[apriltag 41h12:ID]` (tagStandard41h12) pick another. |
//! | `[img:FILE]` | Image, dithered to black and white; `[img threshold:FILE]` cuts at mid-grey instead (crisper for logos). |
//!
//! Codes and images take an optional height before the colon: `[qr 10mm:DATA]`,
//! `[code128 6mm:DATA]`, `[img 8mm:logo.png]`. For an AprilTag the size is the tag size a
//! detector is configured with for pose estimation: the edge of the black square (in
//! tagStandard41h12, the outer edge of the white ring), not the whole tag with its surround.
//! Modules are whole dots, so the size printed is the nearest achievable;
//! [`Rendered::notes`] reports it.
//!
//! # Layout
//!
//! A label is a row of columns (usually one), each a stack of lines. Everything without a fixed
//! size scales together, per column, to the largest size that fits the page's safe area: a
//! lone word fills the label, three lines share it. Lines never wrap by themselves. Columns
//! holding only codes or images are as wide as their content; columns with text share the rest
//! of the width equally.

mod apriltag_tables;
mod code;
mod font;
mod image;
mod layout;
mod markup;

pub use font::Fonts;
pub use image::Gray;
pub use layout::render;
pub use markup::{
    BIG, CodeKind, Column, HAlign, Item, Label, Length, Line, MarkupError, SMALL, Segment, Size,
    TagFamily, VAlign,
};

use crate::{Bitmap, LabelSize, Margins};

/// A rectangle in dots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    /// Left edge.
    pub x: usize,
    /// Top edge.
    pub y: usize,
    /// Width.
    pub width: usize,
    /// Height.
    pub height: usize,
}

/// What a label is laid out on: the canvas (the whole label, as read) and the part of it that
/// prints reliably.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Page {
    /// Canvas width in dots.
    pub width: usize,
    /// Canvas height in dots.
    pub height: usize,
    /// Content is kept inside this.
    pub safe: Rect,
    /// Printer resolution.
    pub dots_per_mm: f32,
}

impl Page {
    /// A label of `size` at `dots_per_mm`, with `margins` kept clear.
    pub fn new(size: LabelSize, dots_per_mm: f32, margins: Margins) -> Self {
        let dots = |mm: f32| (mm.max(0.0) * dots_per_mm).round() as usize;
        let (width, height) = (dots(size.width_mm), dots(size.height_mm));
        let (x, y) = (dots(margins.left).min(width), dots(margins.top).min(height));
        Self {
            width,
            height,
            safe: Rect {
                x,
                y,
                width: (width - x).saturating_sub(dots(margins.right)),
                height: (height - y).saturating_sub(dots(margins.bottom)),
            },
            dots_per_mm,
        }
    }
}

/// A rendered label.
#[derive(Debug, Clone)]
pub struct Rendered {
    /// The label as read, ready for [`crate::Printer::print`].
    pub bitmap: Bitmap,
    /// Things that will print but may disappoint: tiny text, codes too fine to scan, characters
    /// the font lacks.
    pub warnings: Vec<String>,
    /// Things worth knowing about the result: the printed size of each AprilTag.
    pub notes: Vec<String>,
}

/// Why a label could not be rendered.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LabelError {
    /// The markup is malformed.
    #[error(transparent)]
    Markup(#[from] MarkupError),
    /// A font could not be loaded.
    #[error("cannot load font: {0}")]
    Font(String),
    /// A QR code, barcode or AprilTag cannot encode its data.
    #[error("{0}")]
    Code(String),
    /// An image could not be loaded.
    #[error("cannot load image `{path}`: {message}")]
    Image {
        /// The path from the markup.
        path: String,
        /// The loader's error.
        message: String,
    },
    /// The content cannot fit the label even at the smallest size.
    #[error("{0}")]
    DoesNotFit(String),
}

/// An image loader for [`render`] that refuses every image, for labels without `[img:…]`.
pub fn no_images(_path: &str) -> Result<Gray, String> {
    Err("images are not available here".into())
}
