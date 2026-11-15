//! 1-bit bitmaps and bitmap-row packet encoding (PROTOCOL.md §7.1–§7.2).

use super::{Packet, cmd};

/// A 1-bit image. `true` / bit set = black (burn).
///
/// Rows are packed MSB-first (bit 7 of byte 0 is column 0), and each row is padded to a whole
/// number of bytes, which is exactly the wire format of a bitmap row.
#[derive(Clone, PartialEq, Eq)]
pub struct Bitmap {
    width: usize,
    height: usize,
    stride: usize,
    data: Vec<u8>,
}

impl std::fmt::Debug for Bitmap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Bitmap({}x{})", self.width, self.height)
    }
}

impl Bitmap {
    /// A white bitmap.
    pub fn new(width: usize, height: usize) -> Self {
        let stride = width.div_ceil(8);
        Self {
            width,
            height,
            stride,
            data: vec![0; stride * height],
        }
    }

    /// Build a bitmap from a per-pixel function returning `true` for black.
    pub fn from_fn(width: usize, height: usize, mut f: impl FnMut(usize, usize) -> bool) -> Self {
        let mut b = Self::new(width, height);
        for y in 0..height {
            for x in 0..width {
                if f(x, y) {
                    b.set(x, y, true);
                }
            }
        }
        b
    }

    /// Build a bitmap from 8-bit luminance (0 = black, 255 = white), row-major: pixels darker than
    /// `threshold` become black.
    ///
    /// # Panics
    /// If `luma.len() != width * height`.
    pub fn from_luma(width: usize, height: usize, luma: &[u8], threshold: u8) -> Self {
        assert_eq!(luma.len(), width * height, "luma buffer size mismatch");
        Self::from_fn(width, height, |x, y| luma[y * width + x] < threshold)
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Bytes per row.
    pub fn stride(&self) -> usize {
        self.stride
    }

    /// Whether the pixel at (`x`, `y`) is black. Out-of-range pixels are white.
    pub fn get(&self, x: usize, y: usize) -> bool {
        x < self.width
            && y < self.height
            && self.data[y * self.stride + x / 8] & (0x80 >> (x % 8)) != 0
    }

    /// Set the pixel at (`x`, `y`). Out-of-range writes are ignored.
    pub fn set(&mut self, x: usize, y: usize, black: bool) {
        if x < self.width && y < self.height {
            let byte = &mut self.data[y * self.stride + x / 8];
            if black {
                *byte |= 0x80 >> (x % 8);
            } else {
                *byte &= !(0x80 >> (x % 8));
            }
        }
    }

    /// Fill a rectangle (clipped to the bitmap).
    pub fn fill_rect(&mut self, x: usize, y: usize, w: usize, h: usize, black: bool) {
        for yy in y..(y + h).min(self.height) {
            for xx in x..(x + w).min(self.width) {
                self.set(xx, yy, black);
            }
        }
    }

    /// Packed bytes of row `y`.
    pub fn row(&self, y: usize) -> &[u8] {
        &self.data[y * self.stride..(y + 1) * self.stride]
    }

    /// Copy `src` into this bitmap with its top-left corner at (`x`, `y`) (clipped).
    pub fn blit(&mut self, src: &Bitmap, x: isize, y: isize) {
        for sy in 0..src.height {
            for sx in 0..src.width {
                if src.get(sx, sy) {
                    let (dx, dy) = (x + sx as isize, y + sy as isize);
                    if dx >= 0 && dy >= 0 {
                        self.set(dx as usize, dy as usize, true);
                    }
                }
            }
        }
    }

    /// Rotate 90° clockwise. The result is `height` wide and `width` tall.
    pub fn rotate_cw(&self) -> Bitmap {
        Bitmap::from_fn(self.height, self.width, |x, y| {
            self.get(y, self.height - 1 - x)
        })
    }
}

/// The three "black pixel count" bytes of a bitmap row, niimbluelib's rules (§7.2): per-third
/// counts when the row fits in the printhead's three chunks, otherwise `00, lo, hi` of the total.
pub fn pixel_counts(row: &[u8], head_dots: usize) -> [u8; 3] {
    let chunk = (head_dots / 8 / 3).max(1);
    if row.len() <= chunk * 3 {
        let mut parts = [0u32; 3];
        for (i, b) in row.iter().enumerate() {
            parts[(i / chunk).min(2)] += b.count_ones();
        }
        parts.map(|p| p.min(255) as u8)
    } else {
        let total: u32 = row.iter().map(|b| b.count_ones()).sum();
        [0, (total & 0xFF) as u8, (total >> 8) as u8]
    }
}

/// Maximum raster width (columns) that fits a `0x85` packet: 249 payload bytes of pixels.
pub const MAX_COLUMNS: usize = 249 * 8;

/// Encode a head-space raster (width = columns across the head, height = rows along the feed)
/// into one-way row packets: identical consecutive rows are merged (up to 255), blank runs use
/// `0x84`, others `0x85`.
///
/// # Panics
/// If the raster is wider than [`MAX_COLUMNS`] or taller than 65535 rows.
pub fn encode_rows(raster: &Bitmap, head_dots: usize) -> Vec<Packet> {
    assert!(
        raster.width() <= MAX_COLUMNS,
        "raster too wide for a row packet"
    );
    assert!(raster.height() <= usize::from(u16::MAX), "raster too tall");
    let mut out = Vec::new();
    let mut y = 0;
    while y < raster.height() {
        let row = raster.row(y);
        let mut n = 1;
        while y + n < raster.height() && n < 255 && raster.row(y + n) == row {
            n += 1;
        }
        let [hi, lo] = (y as u16).to_be_bytes();
        if row.iter().all(|&b| b == 0) {
            out.push(Packet::new(cmd::PRINT_EMPTY_ROW, [hi, lo, n as u8]));
        } else {
            let mut d = Vec::with_capacity(6 + row.len());
            d.extend_from_slice(&[hi, lo]);
            d.extend_from_slice(&pixel_counts(row, head_dots));
            d.push(n as u8);
            d.extend_from_slice(row);
            out.push(Packet::new(cmd::PRINT_BITMAP_ROW, d));
        }
        y += n;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_like_capture() {
        // From PROTOCOL.md §7.4: row 0 = 8 black on the left of 16 cols, row 1 blank.
        let b = Bitmap::from_fn(16, 2, |x, y| y == 0 && x < 8);
        let pkts = encode_rows(&b, 384);
        assert_eq!(
            pkts[0].encode(),
            [
                0x55, 0x55, 0x85, 0x08, 0, 0, 8, 0, 0, 1, 0xFF, 0x00, 0x7B, 0xAA, 0xAA
            ]
        );
        assert_eq!(
            pkts[1].encode(),
            [0x55, 0x55, 0x84, 0x03, 0, 1, 1, 0x87, 0xAA, 0xAA]
        );
    }

    #[test]
    fn merges_and_splits_runs() {
        let b = Bitmap::from_fn(8, 600, |_, _| true);
        let pkts = encode_rows(&b, 96);
        let reps: Vec<u8> = pkts.iter().map(|p| p.data[5]).collect();
        assert_eq!(reps, [255, 255, 90]);
    }

    #[test]
    fn rotate_cw() {
        // 3 wide, 2 tall; mark top-left. After CW rotation it's top-right of a 2x3.
        let mut b = Bitmap::new(3, 2);
        b.set(0, 0, true);
        let r = b.rotate_cw();
        assert_eq!((r.width(), r.height()), (2, 3));
        assert!(r.get(1, 0));
    }
}
