//! Layout and drawing.
//!
//! Each column is measured at a trial base size (`em`, in dots); everything without a fixed
//! size scales with it. The largest `em` at which the column fits its box wins.

use super::code::{BAR_QUIET, Modules, encode};
use super::markup::{CodeKind, Column, HAlign, Item, Label, Line, Size, VAlign};
use super::{Fonts, Gray, LabelError, Page, Rect, Rendered};
use crate::Bitmap;

/// Smallest base size tried, in dots.
const MIN_EM: i32 = 4;
/// Automatic sizes, in em: the side of a QR code or AprilTag, the height of a barcode and of an image.
const QR_EM: f32 = 4.0;
const BARS_EM: f32 = 2.0;
const IMAGE_EM: f32 = 3.0;
/// A barcode is as wide as its space allows, up to modules this wide (in mm).
const MAX_BAR_MODULE_MM: f32 = 0.5;
/// Gaps, in em: between lines, beside a code or image, between parts of a line.
const LINE_GAP_EM: f32 = 0.12;
const ITEM_GAP_EM: f32 = 0.25;
const SEGMENT_GAP_EM: f32 = 0.5;
/// A blank line is this fraction of a text line tall.
const BLANK_LINE: f32 = 0.6;
/// Gap between columns, in mm.
const COLUMN_GAP_MM: f32 = 1.5;
/// Below these, output is hard to read or scan (see the brief's hardware notes).
const MIN_TEXT_MM: f32 = 2.0;
const MIN_QR_MODULE: i32 = 3;
const MIN_BAR_MODULE: i32 = 2;

/// An item with its data loaded and encoded, so measuring is cheap.
enum Prepared<'a> {
    Text {
        text: &'a str,
        size: Size,
        bold: bool,
    },
    /// A QR code or AprilTag.
    Matrix {
        kind: CodeKind,
        data: &'a str,
        side: usize,
        dark: Vec<bool>,
        quiet: usize,
        sized: usize,
        size: Option<f32>,
    },
    Bars {
        dark: Vec<bool>,
        size: Option<f32>,
    },
    Image {
        gray: Gray,
        size: Option<f32>,
        dither: bool,
    },
}

struct PSegment<'a> {
    align: HAlign,
    items: Vec<Prepared<'a>>,
}

struct PLine<'a> {
    segments: Vec<PSegment<'a>>,
    strut: Size,
}

struct PColumn<'a> {
    valign: VAlign,
    lines: Vec<PLine<'a>>,
}

impl PColumn<'_> {
    fn items(&self) -> impl Iterator<Item = &Prepared<'_>> {
        self.lines
            .iter()
            .flat_map(|l| &l.segments)
            .flat_map(|s| &s.items)
    }

    /// Whether the column has text (or blank lines), and so should share spare width.
    fn has_text(&self) -> bool {
        self.items().any(|i| matches!(i, Prepared::Text { .. }))
    }
}

/// How an item is drawn at a given size.
#[derive(Clone, Copy)]
enum Draw {
    Text { px: f32 },
    Matrix { module: i32 },
    Bars { module: i32 },
    Image,
}

#[derive(Clone, Copy)]
struct ItemBox {
    /// Offset from the segment's left edge.
    x: i32,
    w: i32,
    h: i32,
    /// Text only: height above the baseline.
    ascent: i32,
    draw: Draw,
}

struct SegmentBox {
    w: i32,
    items: Vec<ItemBox>,
}

struct LineBox {
    w: i32,
    h: i32,
    /// Extent of the line's text above and below its shared baseline.
    ascent: i32,
    descent: i32,
    segments: Vec<SegmentBox>,
}

struct ColumnBox {
    w: i32,
    h: i32,
    gap: i32,
    lines: Vec<LineBox>,
}

struct Layout<'a> {
    fonts: &'a Fonts,
    dots_per_mm: f32,
}

impl Layout<'_> {
    fn px(&self, size: Size, em: f32) -> f32 {
        match size {
            Size::Relative(r) => em * r,
            Size::Fixed(len) => len.to_dots(self.dots_per_mm),
        }
    }

    /// Measure an item. `width` is the room a barcode may fill.
    fn item(&self, item: &Prepared<'_>, em: f32, width: i32) -> ItemBox {
        let (w, h, ascent, draw) = match item {
            Prepared::Text { text, size, bold } => {
                let px = self.px(*size, em);
                let (w, ascent, descent) = self.fonts.measure(text, px, *bold);
                (w, ascent + descent, ascent, Draw::Text { px })
            }
            Prepared::Matrix {
                kind,
                side,
                quiet,
                sized,
                size,
                ..
            } => {
                let modules = (side + 2 * quiet) as i32;
                let module = match size {
                    // An AprilTag's size is a measurement, not a limit: print the nearest.
                    Some(s) if matches!(kind, CodeKind::AprilTag(_)) => (s / *sized as f32).round(),
                    Some(s) => (s / *sized as f32).floor(),
                    None => (QR_EM * em / modules as f32).floor(),
                };
                let module = (module as i32).max(1);
                (
                    module * modules,
                    module * modules,
                    0,
                    Draw::Matrix { module },
                )
            }
            Prepared::Bars { dark, size } => {
                let modules = (dark.len() + 2 * BAR_QUIET) as i32;
                let widest = (MAX_BAR_MODULE_MM * self.dots_per_mm).round() as i32;
                let module = (width / modules).clamp(1, widest.max(1));
                let h = size.unwrap_or(BARS_EM * em).round().max(1.0) as i32;
                (module * modules, h, 0, Draw::Bars { module })
            }
            Prepared::Image { gray, size, .. } => {
                let h = size.unwrap_or(IMAGE_EM * em).round().max(1.0);
                let aspect = gray.width() as f32 / gray.height().max(1) as f32;
                (
                    (h * aspect).round().max(1.0) as i32,
                    h as i32,
                    0,
                    Draw::Image,
                )
            }
        };
        ItemBox {
            x: 0,
            w,
            h,
            ascent,
            draw,
        }
    }

    fn line(&self, line: &PLine<'_>, em: f32, width: i32) -> LineBox {
        if line.segments.is_empty() {
            let (_, ascent, descent) = self.fonts.measure("", self.px(line.strut, em), false);
            return LineBox {
                w: 0,
                h: ((ascent + descent) as f32 * BLANK_LINE).round() as i32,
                ascent: 0,
                descent: 0,
                segments: Vec::new(),
            };
        }
        let item_gap = (ITEM_GAP_EM * em).round() as i32;
        let segment_gap = (SEGMENT_GAP_EM * em).round() as i32;
        // Barcodes stretch: they share the width the rest of the line leaves.
        let is_bars = |i: &&Prepared<'_>| matches!(i, Prepared::Bars { .. });
        let all = || line.segments.iter().flat_map(|s| &s.items);
        let bars = all().filter(is_bars).count() as i32;
        let bar_width = if bars > 0 {
            let others: i32 = all()
                .filter(|i| !is_bars(i))
                .map(|i| self.item(i, em, 0).w + item_gap)
                .sum();
            let gaps = (line.segments.len() as i32 - 1) * segment_gap;
            (width - others - gaps) / bars
        } else {
            0
        };
        let (mut ascent, mut descent, mut graphic) = (0, 0, 0);
        let mut segments = Vec::with_capacity(line.segments.len());
        for seg in &line.segments {
            let mut x = 0;
            let mut items = Vec::with_capacity(seg.items.len());
            let mut prev_text = true;
            for (i, item) in seg.items.iter().enumerate() {
                let mut b = self.item(item, em, bar_width);
                let is_text = matches!(item, Prepared::Text { .. });
                if i > 0 && !(is_text && prev_text) {
                    x += item_gap;
                }
                b.x = x;
                x += b.w;
                if is_text {
                    ascent = ascent.max(b.ascent);
                    descent = descent.max(b.h - b.ascent);
                } else {
                    graphic = graphic.max(b.h);
                }
                prev_text = is_text;
                items.push(b);
            }
            segments.push(SegmentBox { w: x, items });
        }
        let gaps = (segments.len() as i32 - 1) * segment_gap;
        LineBox {
            w: segments.iter().map(|s| s.w).sum::<i32>() + gaps,
            h: graphic.max(ascent + descent),
            ascent,
            descent,
            segments,
        }
    }

    fn column(&self, col: &PColumn<'_>, em: f32, width: i32) -> ColumnBox {
        let lines: Vec<LineBox> = col.lines.iter().map(|l| self.line(l, em, width)).collect();
        let gap = (LINE_GAP_EM * em).round() as i32;
        ColumnBox {
            w: lines.iter().map(|l| l.w).max().unwrap_or(0),
            h: lines.iter().map(|l| l.h).sum::<i32>() + gap * (lines.len() as i32 - 1).max(0),
            gap,
            lines,
        }
    }

    /// The largest base size at which `col` fits `width` × `height`, and its measurements.
    fn fit(&self, col: &PColumn<'_>, width: i32, height: i32) -> Result<(f32, ColumnBox), ()> {
        let measure = |em: i32| self.column(col, em as f32, width);
        let fits = |b: &ColumnBox| b.w <= width && b.h <= height;
        let smallest = measure(MIN_EM);
        if !fits(&smallest) {
            return Err(());
        }
        // Sizes only grow with em, so binary search for the last one that fits.
        let (mut lo, mut hi) = (MIN_EM, (2 * height).max(MIN_EM));
        let mut best = smallest;
        while lo < hi {
            let mid = (lo + hi + 1) / 2;
            let b = measure(mid);
            if fits(&b) {
                lo = mid;
                best = b;
            } else {
                hi = mid - 1;
            }
        }
        Ok((lo as f32, best))
    }
}

fn prepare<'a>(
    label: &'a Label,
    dots_per_mm: f32,
    images: &dyn Fn(&str) -> Result<Gray, String>,
) -> Result<Vec<PColumn<'a>>, LabelError> {
    let item = |item: &'a Item| -> Result<Prepared<'a>, LabelError> {
        Ok(match item {
            Item::Text { text, size, bold } => Prepared::Text {
                text,
                size: *size,
                bold: *bold,
            },
            Item::Code { kind, data, size } => {
                let size = size.map(|s| s.to_dots(dots_per_mm));
                match encode(*kind, data)? {
                    Modules::Matrix {
                        side,
                        dark,
                        quiet,
                        sized,
                    } => Prepared::Matrix {
                        kind: *kind,
                        data,
                        side,
                        dark,
                        quiet,
                        sized,
                        size,
                    },
                    Modules::Bars(dark) => Prepared::Bars { dark, size },
                }
            }
            Item::Image { path, size, dither } => {
                let image_err = |message: String| LabelError::Image {
                    path: path.clone(),
                    message,
                };
                let gray = images(path).map_err(image_err)?;
                if gray.width() == 0 || gray.height() == 0 {
                    return Err(image_err("the image is empty".into()));
                }
                Prepared::Image {
                    gray,
                    size: size.map(|s| s.to_dots(dots_per_mm)),
                    dither: *dither,
                }
            }
        })
    };
    let line = |l: &'a Line| -> Result<PLine<'a>, LabelError> {
        let segments = l
            .segments
            .iter()
            .map(|s| {
                Ok(PSegment {
                    align: s.align,
                    items: s
                        .items
                        .iter()
                        .map(item)
                        .collect::<Result<_, LabelError>>()?,
                })
            })
            .collect::<Result<_, LabelError>>()?;
        Ok(PLine {
            segments,
            strut: l.strut,
        })
    };
    label
        .columns
        .iter()
        .map(|c: &Column| {
            Ok(PColumn {
                valign: c.valign,
                lines: c
                    .lines
                    .iter()
                    .map(line)
                    .collect::<Result<_, LabelError>>()?,
            })
        })
        .collect()
}

fn draw_item(out: &mut Bitmap, fonts: &Fonts, item: &Prepared<'_>, b: &ItemBox, x: i32, y: i32) {
    let mut rect = |rx: i32, ry: i32, w: i32, h: i32| {
        if rx >= 0 && ry >= 0 {
            out.fill_rect(rx as usize, ry as usize, w as usize, h as usize, true);
        }
    };
    match (item, b.draw) {
        (Prepared::Text { text, bold, .. }, Draw::Text { px }) => {
            fonts.draw(out, text, px, *bold, x, y + b.ascent);
        }
        (
            Prepared::Matrix {
                side, dark, quiet, ..
            },
            Draw::Matrix { module },
        ) => {
            let quiet = *quiet as i32 * module;
            for (i, _) in dark.iter().enumerate().filter(|(_, d)| **d) {
                let (mx, my) = ((i % side) as i32, (i / side) as i32);
                rect(
                    x + quiet + mx * module,
                    y + quiet + my * module,
                    module,
                    module,
                );
            }
        }
        (Prepared::Bars { dark, .. }, Draw::Bars { module }) => {
            let quiet = BAR_QUIET as i32 * module;
            for (i, _) in dark.iter().enumerate().filter(|(_, d)| **d) {
                rect(x + quiet + i as i32 * module, y, module, b.h);
            }
        }
        (Prepared::Image { gray, dither, .. }, Draw::Image) => {
            let scaled = gray.scale(b.w as usize, b.h as usize);
            let bits = if *dither {
                scaled.dither()
            } else {
                scaled.threshold()
            };
            out.blit(&bits, x as isize, y as isize);
        }
        _ => unreachable!("item measured as a different kind"),
    }
}

fn draw_column(out: &mut Bitmap, fonts: &Fonts, col: &PColumn<'_>, boxes: &ColumnBox, area: Rect) {
    let (left, width) = (area.x as i32, area.width as i32);
    let spare = area.height as i32 - boxes.h;
    let mut y = area.y as i32
        + match col.valign {
            VAlign::Top => 0,
            VAlign::Middle => spare / 2,
            VAlign::Bottom => spare,
        };
    for (line, lb) in col.lines.iter().zip(&boxes.lines) {
        // Left parts stack from the left edge, right parts from the right; centred parts sit in
        // the middle, nudged if a neighbour is in the way.
        let seg_gap = if lb.segments.len() > 1 {
            (lb.w - lb.segments.iter().map(|s| s.w).sum::<i32>()) / (lb.segments.len() as i32 - 1)
        } else {
            0
        };
        let mut xs = vec![0; lb.segments.len()];
        let mut from_left = left;
        for (i, (seg, sb)) in line.segments.iter().zip(&lb.segments).enumerate() {
            if seg.align == HAlign::Left {
                xs[i] = from_left;
                from_left += sb.w + seg_gap;
            }
        }
        let mut from_right = left + width;
        for (i, (seg, sb)) in line.segments.iter().zip(&lb.segments).enumerate().rev() {
            if seg.align == HAlign::Right {
                from_right -= sb.w;
                xs[i] = from_right;
                from_right -= seg_gap;
            }
        }
        let centred: Vec<usize> = (0..line.segments.len())
            .filter(|&i| line.segments[i].align == HAlign::Center)
            .collect();
        let centred_w = centred.iter().map(|&i| lb.segments[i].w).sum::<i32>()
            + seg_gap * (centred.len() as i32 - 1).max(0);
        let mut cx = (left + (width - centred_w) / 2)
            .min(from_right - centred_w)
            .max(from_left);
        for i in centred {
            xs[i] = cx;
            cx += lb.segments[i].w + seg_gap;
        }

        let text_top = y + (lb.h - (lb.ascent + lb.descent)) / 2;
        for ((seg, sb), sx) in line.segments.iter().zip(&lb.segments).zip(xs) {
            for (item, b) in seg.items.iter().zip(&sb.items) {
                let iy = match b.draw {
                    // Text of different sizes shares a baseline.
                    Draw::Text { .. } => text_top + lb.ascent - b.ascent,
                    _ => y + (lb.h - b.h) / 2,
                };
                draw_item(out, fonts, item, b, sx + b.x, iy);
            }
        }
        y += lb.h + boxes.gap;
    }
}

fn warnings(
    columns: &[PColumn<'_>],
    boxes: &[ColumnBox],
    fonts: &Fonts,
    dots_per_mm: f32,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut warn = |w: String| {
        if !out.contains(&w) {
            out.push(w);
        }
    };
    for (col, cb) in columns.iter().zip(boxes) {
        let items = col.items().zip(
            cb.lines
                .iter()
                .flat_map(|l| &l.segments)
                .flat_map(|s| &s.items),
        );
        for (item, b) in items {
            match (item, b.draw) {
                (Prepared::Text { text, bold, .. }, Draw::Text { px }) => {
                    let mm = px / dots_per_mm;
                    if mm < MIN_TEXT_MM {
                        warn(format!(
                            "\"{text}\" is only {mm:.1} mm tall and may be hard to read"
                        ));
                    }
                    for c in text.chars().filter(|&c| !fonts.has_glyph(c, *bold)) {
                        warn(format!("the font has no glyph for {c:?}"));
                    }
                }
                (Prepared::Matrix { kind, .. }, Draw::Matrix { module })
                    if module < MIN_QR_MODULE =>
                {
                    warn(match kind {
                        CodeKind::AprilTag(_) => format!(
                            "the AprilTag's modules are {module} dot(s) wide and may not be \
                             detected (give it more room)"
                        ),
                        _ => format!(
                            "the QR code's modules are {module} dot(s) wide and may not scan \
                             (shorten the data or give it more room)"
                        ),
                    });
                }
                (Prepared::Bars { .. }, Draw::Bars { module }) if module < MIN_BAR_MODULE => {
                    warn("the barcode's bars are 1 dot wide and may not scan".to_owned());
                }
                _ => {}
            }
        }
    }
    out
}

/// The printed size of each AprilTag: whoever sets up the detector needs it.
fn notes(columns: &[PColumn<'_>], boxes: &[ColumnBox], dots_per_mm: f32) -> Vec<String> {
    let mut out = Vec::new();
    for (col, cb) in columns.iter().zip(boxes) {
        let items = col.items().zip(
            cb.lines
                .iter()
                .flat_map(|l| &l.segments)
                .flat_map(|s| &s.items),
        );
        for (item, b) in items {
            if let (
                Prepared::Matrix {
                    kind: CodeKind::AprilTag(family),
                    data,
                    sized,
                    ..
                },
                Draw::Matrix { module },
            ) = (item, b.draw)
            {
                let dots = module * *sized as i32;
                let note = format!(
                    "AprilTag {} ID {}: tag size {:.2} mm ({dots} dots, {module} per module)",
                    family.name(),
                    data.trim(),
                    dots as f32 / dots_per_mm,
                );
                if !out.contains(&note) {
                    out.push(note);
                }
            }
        }
    }
    out
}

/// Lay out and draw a label.
///
/// `images` loads the file named by an `[img:…]` tag (this crate does not decode image
/// formats); pass [`super::no_images`] if there is no need.
pub fn render(
    label: &Label,
    page: &Page,
    fonts: &Fonts,
    images: &dyn Fn(&str) -> Result<Gray, String>,
) -> Result<Rendered, LabelError> {
    let mut bitmap = Bitmap::new(page.width, page.height);
    let columns = prepare(label, page.dots_per_mm, images)?;
    if columns.is_empty() {
        return Ok(Rendered {
            bitmap,
            warnings: Vec::new(),
            notes: Vec::new(),
        });
    }
    let layout = Layout {
        fonts,
        dots_per_mm: page.dots_per_mm,
    };
    let safe = page.safe;
    let (safe_w, safe_h) = (safe.width as i32, safe.height as i32);
    let n = columns.len() as i32;
    let col_gap = (COLUMN_GAP_MM * page.dots_per_mm).round() as i32;
    let total = safe_w - col_gap * (n - 1);
    let does_not_fit = |i: usize| {
        let which = if n > 1 {
            format!("column {} of the label", i + 1)
        } else {
            "the label".to_owned()
        };
        LabelError::DoesNotFit(format!(
            "{which} does not fit in {:.0} x {:.0} mm even at the smallest size: \
             use fewer or shorter lines, or smaller fixed sizes",
            page.width as f32 / page.dots_per_mm,
            page.height as f32 / page.dots_per_mm
        ))
    };

    // Columns of only codes and images take their natural width (up to 40% between them); columns
    // with text split what is left. With no text anywhere, every column gets an equal share.
    let text_cols = columns.iter().filter(|c| c.has_text()).count() as i32;
    let graphic_cols = n - text_cols;
    let mut fitted: Vec<Option<(i32, ColumnBox)>> = columns.iter().map(|_| None).collect();
    let mut remaining = total;
    if text_cols > 0 {
        let cap = if graphic_cols > 0 {
            total * 2 / 5 / graphic_cols
        } else {
            0
        };
        for (i, col) in columns.iter().enumerate().filter(|(_, c)| !c.has_text()) {
            let (_, b) = layout.fit(col, cap, safe_h).map_err(|()| does_not_fit(i))?;
            remaining -= b.w;
            fitted[i] = Some((b.w, b));
        }
    }
    let share = remaining / if text_cols > 0 { text_cols } else { n };
    for (i, col) in columns.iter().enumerate() {
        if fitted[i].is_none() {
            let (_, b) = layout
                .fit(col, share, safe_h)
                .map_err(|()| does_not_fit(i))?;
            fitted[i] = Some((share, b));
        }
    }

    let mut x = safe.x;
    let mut boxes = Vec::with_capacity(columns.len());
    for (col, slot) in columns.iter().zip(fitted) {
        let (width, b) = slot.expect("every column fitted");
        let area = Rect {
            x,
            width: width as usize,
            ..safe
        };
        draw_column(&mut bitmap, fonts, col, &b, area);
        x += (width + col_gap) as usize;
        boxes.push(b);
    }
    let warnings = warnings(&columns, &boxes, fonts, page.dots_per_mm);
    let notes = notes(&columns, &boxes, page.dots_per_mm);
    Ok(Rendered {
        bitmap,
        warnings,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::super::no_images;
    use super::*;
    use crate::{LabelSize, Margins};

    /// D101 40 × 12 mm: 320 × 96 dots, safe x 8..312, y 8..88.
    fn d101() -> Page {
        Page::new(LabelSize::new(40.0, 12.0), 203.0 / 25.4, Margins::default())
    }

    /// B1 50 × 30 mm: 400 × 240 dots, safe x 8..376, y 8..232.
    fn b1() -> Page {
        let margins = Margins {
            right: 3.0,
            ..Margins::default()
        };
        Page::new(LabelSize::new(50.0, 30.0), 203.0 / 25.4, margins)
    }

    fn draw(markup: &str, page: &Page) -> Rendered {
        let label = Label::parse(markup).unwrap();
        render(&label, page, &Fonts::builtin(), &no_images).unwrap()
    }

    /// Bounding box of the black pixels within `x0..x1`: (left, top, right, bottom), inclusive.
    fn ink_in(b: &Bitmap, x0: usize, x1: usize) -> Option<(usize, usize, usize, usize)> {
        let mut r: Option<(usize, usize, usize, usize)> = None;
        for y in 0..b.height() {
            for x in x0..x1.min(b.width()) {
                if b.get(x, y) {
                    let (l, t, rt, bt) = r.unwrap_or((x, y, x, y));
                    r = Some((l.min(x), t.min(y), rt.max(x), bt.max(y)));
                }
            }
        }
        r
    }

    fn ink(b: &Bitmap) -> (usize, usize, usize, usize) {
        ink_in(b, 0, b.width()).expect("nothing drawn")
    }

    fn assert_in_safe(r: &Rendered, page: &Page) {
        let (l, t, rt, bt) = ink(&r.bitmap);
        let s = page.safe;
        assert!(
            l >= s.x && t >= s.y && rt < s.x + s.width && bt < s.y + s.height,
            "ink {l},{t}..{rt},{bt} outside safe area {s:?}"
        );
    }

    #[test]
    fn pages_match_measured_safe_areas() {
        let (d, b) = (d101(), b1());
        assert_eq!((d.width, d.height), (320, 96));
        assert_eq!(
            d.safe,
            Rect {
                x: 8,
                y: 8,
                width: 304,
                height: 80
            }
        );
        assert_eq!((b.width, b.height), (400, 240));
        assert_eq!(
            b.safe,
            Rect {
                x: 8,
                y: 8,
                width: 368,
                height: 224
            }
        );
    }

    #[test]
    fn text_fills_the_safe_area() {
        for page in [d101(), b1()] {
            let r = draw("Hxg", &page);
            assert_in_safe(&r, &page);
            assert!(r.warnings.is_empty(), "{:?}", r.warnings);
            let (l, t, rt, bt) = ink(&r.bitmap);
            // A short word is limited by height: its line box (ascent to descent) fills it, so
            // the ink covers most of it. It is centred.
            assert!(
                bt - t > page.safe.height * 6 / 10,
                "ink is {} dots tall",
                bt - t
            );
            let (mid, centre) = ((l + rt) / 2, page.safe.x + page.safe.width / 2);
            assert!(mid.abs_diff(centre) <= 3, "ink centred at {mid}");
        }
        // A long line is limited by width instead.
        let page = d101();
        let r = draw("The quick brown fox jumps over the lazy dog", &page);
        assert_in_safe(&r, &page);
        let (l, _, rt, _) = ink(&r.bitmap);
        assert!(rt - l > page.safe.width * 9 / 10);
    }

    #[test]
    fn alignment() {
        let page = b1();
        let s = page.safe;
        let (right_edge, bottom_edge) = (s.x + s.width - 1, s.y + s.height - 1);
        let near = |a: usize, b: usize| a.abs_diff(b) <= 4;

        let (l, t, ..) = ink(&draw("[left][top][size:4mm]HH", &page).bitmap);
        assert!(near(l, s.x) && t < s.y + 12, "left/top at {l},{t}");
        let (_, _, r, b) = ink(&draw("[right][bottom][size:4mm]HH", &page).bitmap);
        assert!(
            near(r, right_edge) && b > bottom_edge - 12,
            "right/bottom at {r},{b}"
        );
        // Two parts of one line: one at each edge, on the same baseline.
        let both = draw("[left][size:4mm]HH[right]HH", &page).bitmap;
        let (l, t, r, b) = ink(&both);
        assert!(near(l, s.x) && near(r, right_edge));
        assert!(b - t < 40, "one line, not two");
        assert!(ink_in(&both, 150, 250).is_none(), "gap between the parts");
    }

    #[test]
    fn lines_stack_and_scale() {
        let page = b1();
        let one = draw("[left]HHHH", &page).bitmap;
        let two = draw("[left]HHHH\\n[small]HHHH", &page).bitmap;
        let (_, t1, _, b1) = ink(&one);
        let (_, t2, _, b2) = ink(&two);
        assert!(b2 - t2 > b1 - t1, "two lines are taller than one");
        // The second, small line is narrower than the first: find the rows of each.
        let rows: Vec<usize> = (0..two.height())
            .map(|y| (0..two.width()).filter(|&x| two.get(x, y)).count())
            .collect();
        let gap = (t2..b2).find(|&y| rows[y] == 0).expect("gap between lines");
        let width = |y0: usize, y1: usize| {
            let mut span = (usize::MAX, 0);
            for y in y0..y1 {
                for x in 0..two.width() {
                    if two.get(x, y) {
                        span = (span.0.min(x), span.1.max(x));
                    }
                }
            }
            span.1 - span.0
        };
        let (big, small) = (width(t2, gap), width(gap, b2 + 1));
        let ratio = small as f32 / big as f32;
        assert!((ratio - 0.7).abs() < 0.05, "small/normal width = {ratio}");
    }

    #[test]
    fn bold_is_heavier() {
        let page = d101();
        let count = |b: &Bitmap| {
            (0..b.height())
                .flat_map(|y| (0..b.width()).map(move |x| (x, y)))
                .filter(|&(x, y)| b.get(x, y))
                .count()
        };
        let regular = count(&draw("[size:6mm]Hello", &page).bitmap);
        let bold = count(&draw("[size:6mm][b]Hello", &page).bitmap);
        assert!(bold > regular * 11 / 10, "bold {bold} vs regular {regular}");
        // Without a bold font, bold is synthesised.
        let regular_only = Fonts::from_bytes(
            include_bytes!("../../fonts/AtkinsonHyperlegible-Regular.ttf"),
            None,
        )
        .unwrap();
        let label = Label::parse("[size:6mm][b]Hello").unwrap();
        let fake = count(
            &render(&label, &page, &regular_only, &no_images)
                .unwrap()
                .bitmap,
        );
        assert!(fake > regular * 11 / 10, "fake bold {fake} vs {regular}");
    }

    #[test]
    fn qr_fills_height_beside_text() {
        let page = d101();
        // A version 1 code is 21 modules + 2 quiet each side = 25: 3 dots each in 80 dots.
        let r = draw("[qr:BIN-12][col][left]Shelf 4\\nBin 12", &page);
        assert_in_safe(&r, &page);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let (l, t, rt, bt) = ink_in(&r.bitmap, 0, 8 + 75).unwrap();
        assert_eq!(rt - l, bt - t, "QR is square");
        let side = rt - l + 1;
        assert_eq!(side, 21 * 3, "QR modules are 3 dots");
        // A URL needs version 2 (25 modules): only 2 dots each on this label, which warns.
        let r = draw("[qr:https://example.com]", &page);
        assert!(r.warnings.iter().any(|w| w.contains("may not scan")));
        // Text starts to the right of the code.
        let (tl, ..) = ink_in(&r.bitmap, rt + 1, 320).expect("text drawn");
        assert!(tl > rt + 8, "text at {tl}, code ends {rt}");
    }

    #[test]
    fn apriltags() {
        let page = d101();
        // tag36h11 is 10 modules across with its light ring: 8 dots each in 80 dots of height.
        let r = draw("[apriltag:42][col][left]Dock 42", &page);
        assert_in_safe(&r, &page);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        let (l, t, rt, bt) = ink_in(&r.bitmap, 0, 8 + 80).unwrap();
        assert_eq!((rt - l + 1, bt - t + 1), (64, 64), "black square");
        assert_eq!((l, t), (16, 16), "inside a light ring one module wide");
        assert_eq!(
            r.notes,
            ["AprilTag tag36h11 ID 42: tag size 8.01 mm (64 dots, 8 per module)"]
        );
        // A size is the black square's edge, to the nearest whole dot per module: 20 mm is
        // 159.8 dots, so 20 dots per module.
        let page = b1();
        let r = draw("[apriltag 20mm:0]", &page);
        let (l, t, rt, bt) = ink(&r.bitmap);
        assert_eq!((rt - l + 1, bt - t + 1), (160, 160));
        assert!(
            r.notes[0].contains("tag size 20.02 mm (160 dots"),
            "{:?}",
            r.notes
        );
        // tagStandard41h12 has code bits outside its border: the size spans the 5 modules
        // inside the black ring, and the tag is 9 modules across.
        let r = draw("[apriltag 41h12 10mm:0]", &page);
        let (l, t, rt, bt) = ink(&r.bitmap);
        assert_eq!((rt - l + 1, bt - t + 1), (9 * 16, 9 * 16));
        assert!(r.notes[0].contains("tagStandard41h12 ID 0: tag size 10.01 mm (80 dots"));
        // Too small to detect reliably.
        let r = draw("[apriltag 2mm:0]", &page);
        assert!(r.warnings.iter().any(|w| w.contains("may not be detected")));
        assert!(
            render(
                &Label::parse("[apriltag:587]").unwrap(),
                &page,
                &Fonts::builtin(),
                &no_images
            )
            .is_err()
        );
    }

    #[test]
    fn barcodes() {
        let page = b1();
        let r = draw("Shelf 4\\n[code128:ABC-1234]", &page);
        assert_in_safe(&r, &page);
        assert!(r.warnings.is_empty(), "{:?}", r.warnings);
        // The bottom row of ink is all bars: runs are whole multiples of the module (≥ 2 dots).
        let (l, _, rt, bt) = ink(&r.bitmap);
        let mut runs = Vec::new();
        let mut x = l;
        while x <= rt {
            let v = r.bitmap.get(x, bt);
            let n = (x..=rt).take_while(|&x| r.bitmap.get(x, bt) == v).count();
            runs.push(n);
            x += n;
        }
        let module = *runs.iter().min().unwrap();
        assert!(module >= 2, "module {module}");
        assert!(runs.iter().all(|n| n % module == 0), "{runs:?}");
        // Code 128 ends with a 2-module bar (the stop pattern's termination bar).
        assert_eq!(runs.last(), Some(&(2 * module)));
        // An explicit height is honoured.
        let r = draw("[ean13 10mm:590123412345]", &page);
        let (_, t, _, b) = ink(&r.bitmap);
        assert_eq!(b - t + 1, 80);
    }

    #[test]
    fn images() {
        let page = b1();
        // A 20 × 10 image: black left half, mid-grey right half.
        let load = |path: &str| -> Result<Gray, String> {
            if path != "half.png" {
                return Err("no such file".into());
            }
            let luma = (0..200)
                .map(|i| if i % 20 < 10 { 0 } else { 128 })
                .collect();
            Ok(Gray::new(20, 10, luma))
        };
        let fonts = Fonts::builtin();
        let go = |markup: &str| render(&Label::parse(markup).unwrap(), &page, &fonts, &load);
        let dithered = go("[img 10mm:half.png]").unwrap().bitmap;
        let (l, t, r, b) = ink(&dithered);
        assert_eq!(b - t + 1, 80, "10 mm tall");
        assert!((150..=160).contains(&(r - l + 1)), "aspect kept: {}", r - l);
        let cut = go("[img threshold 10mm:half.png]").unwrap().bitmap;
        let (cl, _, cr, _) = ink(&cut);
        assert_eq!(cr - cl + 1, 80, "threshold drops the grey half");
        let e = go("[img:missing.png]").unwrap_err();
        assert!(e.to_string().contains("missing.png"), "{e}");
    }

    #[test]
    fn too_much_content_is_an_error_and_tiny_text_warns() {
        let page = d101();
        let label = Label::parse(&"line\\n".repeat(30)).unwrap();
        let e = render(&label, &page, &Fonts::builtin(), &no_images).unwrap_err();
        assert!(matches!(e, LabelError::DoesNotFit(_)), "{e}");
        let r = draw(&"line\\n".repeat(7), &page);
        assert!(r.warnings.iter().any(|w| w.contains("hard to read")));
        let r = draw("snow ☃", &page);
        assert!(r.warnings.iter().any(|w| w.contains("no glyph")));
        // Nothing to draw is a blank label, not an error.
        let r = draw("", &page);
        assert!(ink_in(&r.bitmap, 0, 320).is_none());
    }
}
