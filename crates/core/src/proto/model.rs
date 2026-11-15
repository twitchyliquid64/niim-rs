//! Printer model table (PROTOCOL.md §8, §11.5).

/// How labels are oriented relative to the printhead (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// The label's width runs across the head (B-series). Images print as viewed.
    Top,
    /// The label's height runs across the head and its length along the feed (D-series, narrow
    /// side-feed). Landscape images are rotated 90° clockwise before printing.
    Left,
}

/// Where the printer puts a raster narrower than the head, and where the label sits (§7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Alignment {
    /// Centred on the head (measured on the B1).
    Center,
    /// Against the last head dot (measured on the D101).
    End,
}

/// Print job sequences, named after niimbluelib's print tasks (§8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrintSequence {
    /// 7-byte PrintStart, PageStart, 6-byte SetPageSize, rows, PageEnd, poll PrintStatus.
    B1,
    /// 1-byte PrintStart, PrintClear, PageStart, 4-byte SetPageSize, PrintQuantity, … (not implemented).
    D110,
    /// Old D11: 2-byte SetPageSize, page-index events (not implemented).
    D11V1,
    /// B21 with check lines, PrintEnd polling (not implemented).
    B21V1,
    /// B21 L2B (not implemented).
    B21L2B,
    /// H1S: 2-byte PrintStart, 9-byte SetPageSize (not implemented).
    H1S,
    /// Protocol ≥ 4: 9-byte PrintStart, 13-byte SetPageSize (not implemented).
    V4,
}

/// Static facts about a printer model.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelInfo {
    /// Model name, e.g. `B1`, `D101`.
    pub name: &'static str,
    /// Model IDs reported by `PrinterInfo(ModelId)`.
    pub ids: &'static [u16],
    /// 203 or 300.
    pub dpi: u16,
    /// Printhead width in dots (maximum raster columns).
    pub head_dots: u16,
    /// Label orientation convention.
    pub orientation: Orientation,
    /// Lowest density.
    pub density_min: u8,
    /// Highest density.
    pub density_max: u8,
    /// Factory default density.
    pub density_default: u8,
    /// Print sequence, if known (before protocol-version overrides).
    pub sequence: Option<PrintSequence>,
    /// Measured raster alignment, if known.
    pub alignment: Option<Alignment>,
    /// The `DD` heartbeat lid byte is inverted on this model.
    pub lid_inverted: bool,
    /// Measured calibration: after alignment, shift the label image this many dots toward higher
    /// head columns so it lands where the physical label is. B1: `+8` (head dot 0 is the left edge
    /// of a 50 mm label, not 1 mm in).
    pub x_offset: i16,
}

impl ModelInfo {
    /// Look up a model by ID. Where several entries share an ID, the first wins (§10).
    pub fn by_id(id: u16) -> Option<&'static ModelInfo> {
        super::models_table::MODELS
            .iter()
            .find(|m| m.ids.contains(&id))
    }

    /// Look up a model by name, case-insensitively.
    pub fn by_name(name: &str) -> Option<&'static ModelInfo> {
        super::models_table::MODELS
            .iter()
            .find(|m| m.name.eq_ignore_ascii_case(name))
    }

    /// All known models.
    pub fn all() -> &'static [ModelInfo] {
        super::models_table::MODELS
    }

    /// The print sequence for this model at a given protocol version (niimbluelib's rules, §8).
    pub fn sequence_for(&self, protocol_version: u8) -> Option<PrintSequence> {
        match (self.name, protocol_version) {
            ("D11", 1 | 2) => Some(PrintSequence::D110),
            ("D110_M", 4) => Some(PrintSequence::V4),
            _ => self
                .sequence
                .or((protocol_version >= 4).then_some(PrintSequence::V4)),
        }
    }

    /// Model name from a BLE advertised name such as `D101-F310020193`.
    pub fn from_ble_name(name: &str) -> Option<&'static ModelInfo> {
        let (model, _) = name.split_once('-')?;
        Self::by_name(model)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookups() {
        let b1 = ModelInfo::by_id(4096).unwrap();
        assert_eq!(
            (b1.name, b1.head_dots, b1.alignment),
            ("B1", 384, Some(Alignment::Center))
        );
        let d101 = ModelInfo::from_ble_name("D101-F310020193").unwrap();
        assert_eq!(
            (d101.ids, d101.lid_inverted, d101.sequence_for(0)),
            (&[2560][..], true, Some(PrintSequence::B1))
        );
        assert_eq!(
            ModelInfo::by_name("D11").unwrap().sequence_for(2),
            Some(PrintSequence::D110)
        );
        assert!(ModelInfo::from_ble_name("WHOOP MGB0587432").is_none());
    }
}
