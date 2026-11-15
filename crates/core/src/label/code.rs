//! QR code and linear barcode encoding into modules (`true` = dark).

use barcoders::sym::{code128::Code128, ean8::EAN8, ean13::EAN13};
use qrcode::{Color, QrCode};

use super::LabelError;
use super::markup::CodeKind;

/// Quiet zone on every side of a QR code, in modules. The standard asks for 4 around a code
/// printed among other content; labels have a blank border and the layout adds gaps, so 2 keeps
/// the code larger.
pub(super) const QR_QUIET: usize = 2;
/// Quiet zone at each end of a linear barcode, in modules.
pub(super) const BAR_QUIET: usize = 10;

/// An encoded code.
pub(super) enum Modules {
    /// A square matrix, row-major, `side` × `side`.
    Matrix { side: usize, dark: Vec<bool> },
    /// Bars, left to right.
    Bars(Vec<bool>),
}

fn err(kind: CodeKind, data: &str, why: impl std::fmt::Display) -> LabelError {
    LabelError::Code(format!("[{}:{data}]: {why}", kind.name()))
}

/// Encode `data` for barcoders' Code 128, which wants explicit character-set switches: set B
/// (printable ASCII), with runs of digits packed two per symbol in set C where that is shorter.
fn code128_sets(data: &str) -> Option<String> {
    const SET_B: char = '\u{0181}';
    const SET_C: char = '\u{0106}';
    if !data.chars().all(|c| (' '..='\u{7f}').contains(&c)) {
        return None;
    }
    let chars: Vec<char> = data.chars().collect();
    let digits_from = |i: usize| chars[i..].iter().take_while(|c| c.is_ascii_digit()).count();
    let mut out = String::new();
    let mut in_c = false;
    let mut i = 0;
    while i < chars.len() {
        let run = digits_from(i);
        if in_c {
            if run >= 2 {
                out.extend(&chars[i..i + 2]);
                i += 2;
                continue;
            }
            out.push(SET_B);
            in_c = false;
        } else {
            // Switching costs a symbol: worth it for 4+ digits at either end, 6+ in the middle.
            let at_edge = i == 0 || i + run == chars.len();
            if run >= 6 || (run >= 4 && at_edge) {
                if i == 0 && run % 2 == 0 {
                    out.push(SET_C);
                    in_c = true;
                    continue;
                }
                if i == 0 {
                    out.push(SET_B);
                }
                // An odd run leaves its first digit in set B.
                if run % 2 == 1 {
                    out.push(chars[i]);
                    i += 1;
                }
                out.push(SET_C);
                in_c = true;
                continue;
            }
            if i == 0 {
                out.push(SET_B);
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    Some(out)
}

fn digits(kind: CodeKind, data: &str, without_check: usize) -> Result<&str, LabelError> {
    let d = data.trim();
    let n = d.chars().count();
    if d.chars().all(|c| c.is_ascii_digit()) && (n == without_check || n == without_check + 1) {
        Ok(d)
    } else {
        Err(err(
            kind,
            data,
            format!(
                "needs {without_check} digits, or {} including the check digit",
                without_check + 1
            ),
        ))
    }
}

/// Encode a code's data.
pub(super) fn encode(kind: CodeKind, data: &str) -> Result<Modules, LabelError> {
    let bars = |r: Result<Vec<u8>, barcoders::error::Error>| {
        r.map(|v| Modules::Bars(v.into_iter().map(|b| b != 0).collect()))
            .map_err(|e| match e {
                barcoders::error::Error::Checksum => err(kind, data, "wrong check digit"),
                e => err(kind, data, e),
            })
    };
    match kind {
        CodeKind::Qr => {
            let qr = QrCode::new(data.as_bytes()).map_err(|e| err(kind, data, e))?;
            Ok(Modules::Matrix {
                side: qr.width(),
                dark: qr.into_colors().iter().map(|&c| c == Color::Dark).collect(),
            })
        }
        CodeKind::Code128 => {
            let coded = code128_sets(data)
                .filter(|_| !data.is_empty())
                .ok_or_else(|| err(kind, data, "only printable ASCII can be encoded"))?;
            bars(Code128::new(coded).map(|c| c.encode()))
        }
        CodeKind::Ean13 => bars(EAN13::new(digits(kind, data, 12)?).map(|c| c.encode())),
        CodeKind::Ean8 => bars(EAN8::new(digits(kind, data, 7)?).map(|c| c.encode())),
        // UPC-A is EAN-13 with a leading zero.
        CodeKind::UpcA => {
            bars(EAN13::new(format!("0{}", digits(kind, data, 11)?)).map(|c| c.encode()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar_count(kind: CodeKind, data: &str) -> usize {
        match encode(kind, data).unwrap() {
            Modules::Bars(b) => b.len(),
            Modules::Matrix { .. } => panic!("not bars"),
        }
    }

    #[test]
    fn code128_charsets() {
        assert_eq!(code128_sets("ABC-12").unwrap(), "\u{181}ABC-12");
        assert_eq!(code128_sets("123456").unwrap(), "\u{106}123456");
        assert_eq!(code128_sets("12345").unwrap(), "\u{181}1\u{106}2345");
        assert_eq!(code128_sets("AB1234").unwrap(), "\u{181}AB\u{106}1234");
        assert_eq!(code128_sets("1234AB").unwrap(), "\u{106}1234\u{181}AB");
        assert_eq!(code128_sets("A1234B").unwrap(), "\u{181}A1234B");
        assert!(code128_sets("né").is_none());
        // Start + n symbols + checksum are 11 modules each; stop is 13.
        assert_eq!(bar_count(CodeKind::Code128, "ABC-12"), 11 * 8 + 13);
        assert_eq!(bar_count(CodeKind::Code128, "123456"), 11 * 5 + 13);
        assert_eq!(bar_count(CodeKind::Code128, "A"), 11 * 3 + 13);
    }

    #[test]
    fn retail_codes() {
        assert_eq!(bar_count(CodeKind::Ean13, "590123412345"), 95);
        assert_eq!(bar_count(CodeKind::Ean13, "5901234123457"), 95);
        assert_eq!(bar_count(CodeKind::UpcA, "036000291452"), 95);
        assert_eq!(bar_count(CodeKind::Ean8, "9638507"), 67);
        let e = encode(CodeKind::Ean13, "5901234123450").err().unwrap();
        assert!(e.to_string().contains("check digit"), "{e}");
        assert!(encode(CodeKind::Ean13, "12345").is_err());
        assert!(encode(CodeKind::UpcA, "12345678901x").is_err());
    }

    #[test]
    fn qr() {
        let Modules::Matrix { side, dark } = encode(CodeKind::Qr, "https://example.com").unwrap()
        else {
            panic!("not a matrix")
        };
        assert_eq!(dark.len(), side * side);
        // Top-left finder pattern: a dark 7×7 ring around a light ring.
        assert!((0..7).all(|i| dark[i] && dark[i * side]));
        assert!(!dark[side + 1] && dark[2 * side + 2]);
    }
}
