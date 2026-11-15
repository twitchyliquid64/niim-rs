//! The label markup language: text with `[tags]`.
//!
//! See the [module documentation](super) for the syntax.

use std::fmt;

/// Horizontal alignment of a run of content within its column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HAlign {
    /// Against the left edge.
    Left,
    /// Centred.
    Center,
    /// Against the right edge.
    Right,
}

/// Vertical alignment of a column's lines within the label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VAlign {
    /// Against the top edge.
    Top,
    /// Centred.
    Middle,
    /// Against the bottom edge.
    Bottom,
}

/// A physical length.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// Millimetres.
    Mm(f32),
    /// Points (1/72 inch).
    Pt(f32),
    /// Printer dots.
    Dots(f32),
}

impl Length {
    /// Convert to printer dots.
    pub fn to_dots(self, dots_per_mm: f32) -> f32 {
        match self {
            Self::Mm(v) => v * dots_per_mm,
            Self::Pt(v) => v * 25.4 / 72.0 * dots_per_mm,
            Self::Dots(v) => v,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        let s = s.trim().to_ascii_lowercase();
        let (num, unit) = ["mm", "pt", "px"]
            .iter()
            .find_map(|unit| Some((s.strip_suffix(unit)?, *unit)))?;
        let v: f32 = num.trim().parse().ok()?;
        let len = match unit {
            "mm" => Self::Mm(v),
            "pt" => Self::Pt(v),
            _ => Self::Dots(v),
        };
        (v.is_finite() && v > 0.0).then_some(len)
    }
}

/// Text size.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Size {
    /// Relative to the column's auto-fitted base size (`1.0` = normal).
    Relative(f32),
    /// A fixed font size, not auto-fitted.
    Fixed(Length),
}

/// Relative size of `[small]` text.
pub const SMALL: f32 = 0.7;
/// Relative size of `[big]` text.
pub const BIG: f32 = 1.5;

/// A barcode symbology.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeKind {
    /// QR code.
    Qr,
    /// Code 128.
    Code128,
    /// EAN-13 (12 digits, or 13 with a check digit).
    Ean13,
    /// EAN-8 (7 digits, or 8 with a check digit).
    Ean8,
    /// UPC-A (11 digits, or 12 with a check digit).
    UpcA,
}

impl CodeKind {
    /// The tag name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Qr => "qr",
            Self::Code128 => "code128",
            Self::Ean13 => "ean13",
            Self::Ean8 => "ean8",
            Self::UpcA => "upca",
        }
    }
}

/// One piece of content.
#[derive(Debug, Clone, PartialEq)]
pub enum Item {
    /// A run of text in one style.
    Text {
        /// The text.
        text: String,
        /// Font size.
        size: Size,
        /// Bold.
        bold: bool,
    },
    /// A QR code or barcode.
    Code {
        /// Symbology.
        kind: CodeKind,
        /// The data to encode.
        data: String,
        /// Height (for a QR code, the side). `None` = sized with the text.
        size: Option<Length>,
    },
    /// An image.
    Image {
        /// Path, resolved by the caller of [`super::render`].
        path: String,
        /// Height. `None` = sized with the text.
        size: Option<Length>,
        /// Dither greys (otherwise threshold at mid-grey).
        dither: bool,
    },
}

/// Items sharing a horizontal alignment on one line.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    /// Where the segment sits in the column.
    pub align: HAlign,
    /// The items, left to right.
    pub items: Vec<Item>,
}

/// One line of a column. A line with no segments is a blank spacer.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Up to a few segments, e.g. one left and one right.
    pub segments: Vec<Segment>,
    /// The text size in effect at the start of the line (sets the height of a blank line).
    pub strut: Size,
}

/// A vertical stack of lines. Columns sit side by side.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Where the lines sit vertically.
    pub valign: VAlign,
    /// The lines, top to bottom.
    pub lines: Vec<Line>,
}

/// A parsed label.
#[derive(Debug, Clone, PartialEq)]
pub struct Label {
    /// The columns, left to right (usually one).
    pub columns: Vec<Column>,
}

/// A markup syntax error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkupError {
    /// Byte offset into the markup.
    pub offset: usize,
    /// What is wrong.
    pub message: String,
}

impl fmt::Display for MarkupError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "markup error at {}: {}", self.offset, self.message)
    }
}

impl std::error::Error for MarkupError {}

const TAGS: &str = "left center right top middle bottom small normal big size:4mm b /b col \
                    qr:DATA code128:DATA ean13:DIGITS ean8:DIGITS upca:DIGITS img:FILE";

struct Parser {
    align: HAlign,
    valign: VAlign,
    size: Size,
    bold: bool,
    columns: Vec<Column>,
    lines: Vec<Line>,
    segments: Vec<Segment>,
    items: Vec<Item>,
    text: String,
    /// The size at the start of the current line.
    strut: Size,
}

impl Parser {
    fn flush_text(&mut self) {
        if !self.text.is_empty() {
            self.items.push(Item::Text {
                text: std::mem::take(&mut self.text),
                size: self.size,
                bold: self.bold,
            });
        }
    }

    fn end_segment(&mut self) {
        self.flush_text();
        let mut items = std::mem::take(&mut self.items);
        // Whitespace at either end of a segment is layout noise (e.g. indentation after a tag).
        if let Some(Item::Text { text, .. }) = items.first_mut() {
            *text = text.trim_start().to_owned();
        }
        if let Some(Item::Text { text, .. }) = items.last_mut() {
            *text = text.trim_end().to_owned();
        }
        items.retain(|i| !matches!(i, Item::Text { text, .. } if text.is_empty()));
        if !items.is_empty() {
            self.segments.push(Segment {
                align: self.align,
                items,
            });
        }
    }

    fn end_line(&mut self) {
        self.end_segment();
        self.lines.push(Line {
            segments: std::mem::take(&mut self.segments),
            strut: self.strut,
        });
        self.strut = self.size;
    }

    fn end_column(&mut self) {
        self.end_line();
        let mut lines = std::mem::take(&mut self.lines);
        let blank = |l: &Line| l.segments.is_empty();
        while lines.last().is_some_and(blank) {
            lines.pop();
        }
        let lead = lines.iter().take_while(|l| blank(l)).count();
        lines.drain(..lead);
        self.columns.push(Column {
            valign: self.valign,
            lines,
        });
    }

    fn set_align(&mut self, align: HAlign) {
        self.end_segment();
        self.align = align;
    }

    fn tag(&mut self, tag: &str) -> Result<(), String> {
        let (head, payload) = match tag.split_once(':') {
            Some((h, p)) => (h, Some(p)),
            None => (tag, None),
        };
        let mut words = head.split_whitespace();
        let name = words.next().unwrap_or("").to_ascii_lowercase();
        let opts: Vec<&str> = words.collect();
        let bare = |what: &str| {
            if payload.is_some() || !opts.is_empty() {
                Err(format!("[{what}] takes no arguments"))
            } else {
                Ok(())
            }
        };
        match name.as_str() {
            "left" => bare("left").map(|()| self.set_align(HAlign::Left)),
            "center" | "centre" => bare("center").map(|()| self.set_align(HAlign::Center)),
            "right" => bare("right").map(|()| self.set_align(HAlign::Right)),
            "top" => bare("top").map(|()| self.valign = VAlign::Top),
            "middle" => bare("middle").map(|()| self.valign = VAlign::Middle),
            "bottom" => bare("bottom").map(|()| self.valign = VAlign::Bottom),
            "small" | "normal" | "big" | "size" | "b" | "/b" => {
                self.flush_text();
                match name.as_str() {
                    "small" => bare("small").map(|()| self.size = Size::Relative(SMALL))?,
                    "normal" => bare("normal").map(|()| self.size = Size::Relative(1.0))?,
                    "big" => bare("big").map(|()| self.size = Size::Relative(BIG))?,
                    "b" => bare("b").map(|()| self.bold = true)?,
                    "/b" => bare("/b").map(|()| self.bold = false)?,
                    _ => {
                        let arg = payload.or(opts.first().copied()).unwrap_or("");
                        let len = Length::parse(arg).ok_or_else(|| {
                            format!(
                                "[size:…] needs a length such as 4mm, 12pt or 24px, not `{arg}`"
                            )
                        })?;
                        self.size = Size::Fixed(len);
                    }
                }
                if self.items.is_empty() && self.segments.is_empty() {
                    self.strut = self.size;
                }
                Ok(())
            }
            "col" | "column" => bare("col").map(|()| self.end_column()),
            "qr" | "code128" | "ean13" | "ean8" | "upca" | "upc" => {
                let kind = match name.as_str() {
                    "qr" => CodeKind::Qr,
                    "code128" => CodeKind::Code128,
                    "ean13" => CodeKind::Ean13,
                    "ean8" => CodeKind::Ean8,
                    _ => CodeKind::UpcA,
                };
                let data = payload.filter(|p| !p.is_empty()).ok_or_else(|| {
                    format!("[{name}] needs data, e.g. [{name}:{}]", example(kind))
                })?;
                let mut size = None;
                for o in opts {
                    size = Some(Length::parse(o).ok_or_else(|| {
                        format!("unknown [{name}] option `{o}` (expected a size such as 8mm)")
                    })?);
                }
                self.flush_text();
                self.items.push(Item::Code {
                    kind,
                    data: data.to_owned(),
                    size,
                });
                Ok(())
            }
            "img" | "image" => {
                let path = payload
                    .map(str::trim)
                    .filter(|p| !p.is_empty())
                    .ok_or("[img] needs a file, e.g. [img:logo.png]")?;
                let (mut size, mut dither) = (None, true);
                for o in opts {
                    match o.to_ascii_lowercase().as_str() {
                        "dither" => dither = true,
                        "threshold" => dither = false,
                        _ => {
                            size = Some(Length::parse(o).ok_or_else(|| {
                                format!(
                                    "unknown [img] option `{o}` (expected a size such as 8mm, \
                                     `dither` or `threshold`)"
                                )
                            })?)
                        }
                    }
                }
                self.flush_text();
                self.items.push(Item::Image {
                    path: path.to_owned(),
                    size,
                    dither,
                });
                Ok(())
            }
            _ => Err(format!(
                "unknown tag [{tag}] (write \\[ for a literal bracket). Tags: {TAGS}"
            )),
        }
    }
}

fn example(kind: CodeKind) -> &'static str {
    match kind {
        CodeKind::Qr => "https://example.com",
        CodeKind::Code128 => "ABC-1234",
        CodeKind::Ean13 => "590123412345",
        CodeKind::Ean8 => "9638507",
        CodeKind::UpcA => "03600029145",
    }
}

impl Label {
    /// Parse markup. See the [module documentation](super) for the syntax.
    pub fn parse(markup: &str) -> Result<Self, MarkupError> {
        let mut p = Parser {
            align: HAlign::Center,
            valign: VAlign::Middle,
            size: Size::Relative(1.0),
            bold: false,
            columns: Vec::new(),
            lines: Vec::new(),
            segments: Vec::new(),
            items: Vec::new(),
            text: String::new(),
            strut: Size::Relative(1.0),
        };
        let mut chars = markup.char_indices().peekable();
        while let Some((at, c)) = chars.next() {
            match c {
                '\\' => match chars.peek().map(|&(_, n)| n) {
                    Some('n') => {
                        chars.next();
                        p.end_line();
                    }
                    Some(n @ ('[' | ']' | '\\')) => {
                        chars.next();
                        p.text.push(n);
                    }
                    _ => p.text.push('\\'),
                },
                '\r' => {}
                '\n' => p.end_line(),
                '[' => {
                    let mut tag = String::new();
                    let mut closed = false;
                    while let Some((_, c)) = chars.next() {
                        match c {
                            ']' => {
                                closed = true;
                                break;
                            }
                            '\\' if matches!(chars.peek(), Some((_, ']' | '\\'))) => {
                                tag.push(chars.next().unwrap().1)
                            }
                            c => tag.push(c),
                        }
                    }
                    if !closed {
                        return Err(MarkupError {
                            offset: at,
                            message: "`[` without a closing `]` (write \\[ for a literal bracket)"
                                .into(),
                        });
                    }
                    p.tag(tag.trim()).map_err(|message| MarkupError {
                        offset: at,
                        message,
                    })?;
                }
                c => p.text.push(c),
            }
        }
        p.end_column();
        // A trailing or doubled [col] makes an empty column; drop those.
        p.columns.retain(|c| !c.lines.is_empty());
        Ok(Self { columns: p.columns })
    }

    /// Whether there is nothing to print.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

impl std::str::FromStr for Label {
    type Err = MarkupError;

    fn from_str(s: &str) -> Result<Self, MarkupError> {
        Self::parse(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(t: &str, size: Size, bold: bool) -> Item {
        Item::Text {
            text: t.into(),
            size,
            bold,
        }
    }

    const NORMAL: Size = Size::Relative(1.0);

    #[test]
    fn plain_lines() {
        // Both the two-character `\n` and a real newline break lines.
        let l = Label::parse("Hello\\nbig\nworld").unwrap();
        assert_eq!(l.columns.len(), 1);
        let lines = &l.columns[0].lines;
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[1].segments,
            [Segment {
                align: HAlign::Center,
                items: vec![text("big", NORMAL, false)]
            }]
        );
    }

    #[test]
    fn sticky_styles_and_segments() {
        let l = Label::parse("[left][big]Name[right][normal]12 [b]kg[/b]\\nnext").unwrap();
        let lines = &l.columns[0].lines;
        assert_eq!(
            lines[0].segments,
            [
                Segment {
                    align: HAlign::Left,
                    items: vec![text("Name", Size::Relative(BIG), false)]
                },
                Segment {
                    align: HAlign::Right,
                    items: vec![text("12 ", NORMAL, false), text("kg", NORMAL, true)]
                },
            ]
        );
        // Alignment and size carry over to the next line.
        assert_eq!(lines[1].segments[0].align, HAlign::Right);
        assert_eq!(lines[1].segments[0].items, [text("next", NORMAL, false)]);
    }

    #[test]
    fn columns_codes_and_images() {
        let l = Label::parse(
            "[qr 10mm:https://example.com/a:b]\n  [col][top]Shelf 4\n[img threshold 5mm: logo.png]",
        )
        .unwrap();
        assert_eq!(l.columns.len(), 2);
        assert_eq!(
            l.columns[0].lines[0].segments[0].items,
            [Item::Code {
                kind: CodeKind::Qr,
                data: "https://example.com/a:b".into(),
                size: Some(Length::Mm(10.0))
            }]
        );
        // The whitespace-only line before [col] is dropped.
        assert_eq!(l.columns[0].lines.len(), 1);
        assert_eq!(l.columns[1].valign, VAlign::Top);
        assert_eq!(
            l.columns[1].lines[1].segments[0].items,
            [Item::Image {
                path: "logo.png".into(),
                size: Some(Length::Mm(5.0)),
                dither: false
            }]
        );
    }

    #[test]
    fn escapes_and_blank_lines() {
        let l = Label::parse("a\\[b\\] c:\\\\d\\x\n\n[size:12pt]z\n").unwrap();
        let lines = &l.columns[0].lines;
        assert_eq!(lines.len(), 3);
        assert_eq!(
            lines[0].segments[0].items,
            [text("a[b] c:\\d\\x", NORMAL, false)]
        );
        assert!(lines[1].segments.is_empty());
        assert_eq!(
            lines[2].segments[0].items,
            [text("z", Size::Fixed(Length::Pt(12.0)), false)]
        );
        assert!(Label::parse(" \n ").unwrap().is_empty());
    }

    #[test]
    fn errors() {
        let e = Label::parse("ab[bogus]").unwrap_err();
        assert_eq!(e.offset, 2);
        assert!(e.message.contains("unknown tag [bogus]"), "{e}");
        assert!(Label::parse("x [left").is_err());
        assert!(Label::parse("[qr]").is_err());
        assert!(Label::parse("[size:huge]").is_err());
        assert!(Label::parse("[left:x]").is_err());
        assert!(Label::parse("[qr wide:x]").is_err());
    }
}
