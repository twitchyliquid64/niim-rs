//! Greyscale images: scaling and reduction to 1 bit.

use crate::Bitmap;

/// An 8-bit greyscale image (0 = black, 255 = white), row-major.
#[derive(Clone, PartialEq, Eq)]
pub struct Gray {
    width: usize,
    height: usize,
    luma: Vec<u8>,
}

impl std::fmt::Debug for Gray {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Gray({}x{})", self.width, self.height)
    }
}

impl Gray {
    /// Wrap luminance data.
    ///
    /// # Panics
    /// If `luma.len() != width * height`.
    pub fn new(width: usize, height: usize, luma: Vec<u8>) -> Self {
        assert_eq!(luma.len(), width * height, "luma buffer size mismatch");
        Self {
            width,
            height,
            luma,
        }
    }

    /// Width in pixels.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Height in pixels.
    pub fn height(&self) -> usize {
        self.height
    }

    /// Resample to `width` × `height`: each output pixel is the average of the source area it
    /// covers (so shrinking doesn't alias), or the nearest pixel when enlarging.
    pub fn scale(&self, width: usize, height: usize) -> Gray {
        if self.width == 0 || self.height == 0 {
            return Gray::new(width, height, vec![255; width * height]);
        }
        let span = |i: usize, out: usize, src: usize| {
            let a = i * src / out;
            let b = ((i + 1) * src).div_ceil(out).clamp(a + 1, src);
            a..b
        };
        let mut luma = Vec::with_capacity(width * height);
        for y in 0..height {
            let ys = span(y, height, self.height);
            for x in 0..width {
                let xs = span(x, width, self.width);
                let n = (ys.len() * xs.len()) as u32;
                let sum: u32 = ys
                    .clone()
                    .flat_map(|sy| xs.clone().map(move |sx| (sx, sy)))
                    .map(|(sx, sy)| u32::from(self.luma[sy * self.width + sx]))
                    .sum();
                luma.push(((sum + n / 2) / n) as u8);
            }
        }
        Gray::new(width, height, luma)
    }

    /// Reduce to 1 bit at mid-grey. Best for line art and logos.
    pub fn threshold(&self) -> Bitmap {
        Bitmap::from_luma(self.width, self.height, &self.luma, 128)
    }

    /// Reduce to 1 bit with Floyd–Steinberg error diffusion. Best for photos; pure black and
    /// white areas are unchanged.
    pub fn dither(&self) -> Bitmap {
        let (w, h) = (self.width, self.height);
        let mut out = Bitmap::new(w, h);
        // Error carried into the current and next rows, in 1/16ths, padded by one each side.
        let mut cur = vec![0i32; w + 2];
        let mut next = vec![0i32; w + 2];
        for y in 0..h {
            for x in 0..w {
                let v = i32::from(self.luma[y * w + x]) * 16 + cur[x + 1];
                let black = v < 128 * 16;
                if black {
                    out.set(x, y, true);
                }
                let e = (v - if black { 0 } else { 255 * 16 }) / 16;
                cur[x + 2] += e * 7;
                next[x] += e * 3;
                next[x + 1] += e * 5;
                next[x + 2] += e;
            }
            std::mem::swap(&mut cur, &mut next);
            next.fill(0);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn black(b: &Bitmap) -> usize {
        (0..b.height())
            .flat_map(|y| (0..b.width()).map(move |x| (x, y)))
            .filter(|&(x, y)| b.get(x, y))
            .count()
    }

    #[test]
    fn scales_by_area() {
        // A 4×2 image, left half black: halves to 2×1 keeping the split, and doubles cleanly.
        let g = Gray::new(4, 2, vec![0, 0, 255, 255, 0, 0, 255, 255]);
        assert_eq!(g.scale(2, 1).luma, [0, 255]);
        assert_eq!(g.scale(1, 1).luma, [128]);
        let big = g.scale(8, 4);
        assert_eq!(&big.luma[..8], [0, 0, 0, 0, 255, 255, 255, 255]);
        assert_eq!(g.scale(3, 2).width(), 3);
    }

    #[test]
    fn dither_keeps_tone() {
        let (w, h) = (64, 64);
        for (grey, want) in [(0u8, 1.0), (255, 0.0), (128, 0.5), (64, 0.75)] {
            let b = Gray::new(w, h, vec![grey; w * h]).dither();
            let frac = black(&b) as f32 / (w * h) as f32;
            assert!((frac - want).abs() < 0.03, "grey {grey}: {frac}");
        }
        // Thresholding loses mid-greys entirely; dithering doesn't.
        assert_eq!(black(&Gray::new(w, h, vec![200; w * h]).threshold()), 0);
    }
}
