//! Typed response parsers (PROTOCOL.md §5–§7).

use std::fmt;

/// A response payload didn't have the expected shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// What was being parsed.
    pub what: &'static str,
    /// The offending payload.
    pub data: Vec<u8>,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "malformed {} response: {:02X?}", self.what, self.data)
    }
}

impl std::error::Error for ParseError {}

fn err<T>(what: &'static str, data: &[u8]) -> Result<T, ParseError> {
    Err(ParseError {
        what,
        data: data.to_vec(),
    })
}

/// Result of the Connect handshake (§5.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectResult {
    /// `0`: refused.
    Disconnect,
    /// `1`: connected, protocol 0.
    Connected,
    /// `2`: connected, protocol 1.
    ConnectedNew,
    /// `3`: connected; ask `PrinterStatusData` for the protocol version.
    ConnectedV3,
    /// `90`: firmware errors.
    FirmwareErrors,
    /// Any other value.
    Other(u8),
}

impl ConnectResult {
    /// Parse a `C2` payload.
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        Ok(match data.first() {
            Some(0) => Self::Disconnect,
            Some(1) => Self::Connected,
            Some(2) => Self::ConnectedNew,
            Some(3) => Self::ConnectedV3,
            Some(90) => Self::FirmwareErrors,
            Some(&n) => Self::Other(n),
            None => return err("connect", data),
        })
    }
}

/// Fields of a `PrinterStatusData` (`B5`) response (§5.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StatusData {
    /// 0, 3, 4 or 5.
    pub protocol_version: u8,
    /// Two-colour support flag.
    pub supports_color: bool,
}

impl StatusData {
    /// Parse a `B5` payload. Payloads shorter than 13 bytes (e.g. the D101's 10) mean protocol 0.
    pub fn parse(data: &[u8]) -> Self {
        if data.len() < 13 {
            return Self::default();
        }
        let n = u16::from(data[11]) * 100 + u16::from(data[12]);
        let protocol_version = match n {
            204..300 => 3,
            300..302 => 4,
            302.. => 5,
            _ => 0,
        };
        Self {
            protocol_version,
            supports_color: data[10] > 0,
        }
    }
}

/// Model ID from `PrinterInfo(ModelId)` (`48`).
pub fn parse_model_id(data: &[u8]) -> Result<u16, ParseError> {
    match data {
        [hi] => Ok(u16::from(*hi) << 8),
        [hi, lo] => Ok(u16::from_be_bytes([*hi, *lo])),
        _ => err("model id", data),
    }
}

/// Serial number from `PrinterInfo(SerialNumber)` (`4B`).
///
/// ≥ 8 bytes is ASCII (`I812130338`); 4–7 bytes is uppercase hex of *all* bytes (`F310020193`,
/// matching the BLE name); shorter means none.
pub fn parse_serial(data: &[u8]) -> Option<String> {
    match data.len() {
        8.. => Some(
            String::from_utf8_lossy(data)
                .trim_end_matches('\0')
                .to_owned(),
        ),
        4..8 => Some(data.iter().map(|b| format!("{b:02X}")).collect()),
        _ => None,
    }
}

/// Bluetooth MAC from `PrinterInfo(BluetoothAddress)` (`4D`); sent reversed.
pub fn parse_mac(data: &[u8]) -> Result<[u8; 6], ParseError> {
    let mut mac: [u8; 6] = data
        .try_into()
        .or_else(|_| err("bluetooth address", data))?;
    mac.reverse();
    Ok(mac)
}

/// Battery: values ≤ 4 are a 0–4 bar level, larger values are a percentage.
pub fn battery_percent(raw: u8) -> u8 {
    if raw <= 4 { raw * 25 } else { raw.min(100) }
}

/// A two-byte firmware/hardware version, displayed `major.minor` (e.g. `06 13` → `6.19`).
///
/// For `PrinterInfo` versions this reading is confirmed on the B1 only; some models (D110) use
/// `u16 / 100` instead (§6.1). The raw bytes are kept so callers can re-interpret.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    /// First byte.
    pub major: u8,
    /// Second byte.
    pub minor: u8,
}

impl Version {
    /// Parse two bytes.
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        match data {
            [major, minor] => Ok(Self {
                major: *major,
                minor: *minor,
            }),
            _ => err("version", data),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{:02}", self.major, self.minor)
    }
}

/// Identity block from Heartbeat type 3 (`DE`, 10 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity {
    /// Hardware version.
    pub hardware: Version,
    /// Firmware version.
    pub firmware: Version,
    /// Printhead width in dots, as the firmware uses it.
    pub head_dots: u16,
    /// 203 or 300.
    pub dpi: u16,
    /// Printhead alignment (meaning unconfirmed; B1 reports 2).
    pub alignment: u8,
    /// Reads RFID tags.
    pub rfid: bool,
    /// Writes RFID tags.
    pub rfid_write: bool,
}

impl Identity {
    /// Parse a `DE` payload.
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        let d: &[u8; 10] = data
            .try_into()
            .or_else(|_| err("heartbeat identity", data))?;
        Ok(Self {
            hardware: Version {
                major: d[0],
                minor: d[1],
            },
            firmware: Version {
                major: d[2],
                minor: d[3],
            },
            head_dots: u16::from_be_bytes([d[4], d[5]]),
            dpi: if d[6] == 3 { 300 } else { 203 },
            alignment: d[7],
            rfid: d[8] != 0,
            rfid_write: d[9] != 0,
        })
    }
}

/// Live state from a `DD` or `D9` heartbeat (§6.3). Fields a model doesn't report are `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Heartbeat {
    /// Lid closed.
    pub lid_closed: Option<bool>,
    /// Battery, 0–100 %.
    pub battery_percent: Option<u8>,
    /// Label roll inserted.
    pub paper_present: Option<bool>,
    /// Label roll RFID read OK.
    pub paper_rfid_ok: Option<bool>,
    /// Ribbon inserted (ribbon printers).
    pub ribbon_present: Option<bool>,
    /// Raw temperature byte (`D9` only).
    pub temperature: Option<u8>,
}

impl Heartbeat {
    /// Parse a heartbeat response by command and length. `lid_inverted` applies to `DD` only.
    pub fn parse(cmd: u8, data: &[u8], lid_inverted: bool) -> Result<Self, ParseError> {
        use super::cmd::{IN_HEARTBEAT_ADVANCED1, IN_HEARTBEAT_ADVANCED2};
        let zero_is_true = |b: u8| Some(b == 0);
        let nonzero = |b: u8| Some(b != 0);
        let mut h = Self::default();
        match (cmd, data.len()) {
            (IN_HEARTBEAT_ADVANCED1, 9) => h.lid_closed = zero_is_true(data[8]),
            (IN_HEARTBEAT_ADVANCED1, 10) => {
                h.lid_closed = zero_is_true(data[8]);
                h.battery_percent = Some(battery_percent(data[9]));
            }
            (IN_HEARTBEAT_ADVANCED1, 13) | (IN_HEARTBEAT_ADVANCED1, 19) => {
                let o = data.len() - 4;
                h.lid_closed = zero_is_true(data[o]);
                h.battery_percent = Some(battery_percent(data[o + 1]));
                h.paper_present = zero_is_true(data[o + 2]);
                h.paper_rfid_ok = nonzero(data[o + 3]);
            }
            (IN_HEARTBEAT_ADVANCED1, 20) => {
                h.paper_present = zero_is_true(data[18]);
                h.paper_rfid_ok = nonzero(data[19]);
            }
            (IN_HEARTBEAT_ADVANCED2, 9..) => {
                h.battery_percent = Some(battery_percent(data[2]));
                h.temperature = Some(data[3]);
                h.lid_closed = zero_is_true(data[4]);
                h.paper_present = zero_is_true(data[5]);
                h.paper_rfid_ok = nonzero(data[6]);
                h.ribbon_present = zero_is_true(data[8]);
            }
            _ => return err("heartbeat", data),
        }
        if cmd == IN_HEARTBEAT_ADVANCED1 && lid_inverted {
            h.lid_closed = h.lid_closed.map(|c| !c);
        }
        Ok(h)
    }
}

/// A label roll's RFID tag (`1B`, §6.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RfidTag {
    /// Tag UUID, hex.
    pub uuid: String,
    /// Consumable barcode, e.g. `10262260`.
    pub barcode: String,
    /// Roll serial number.
    pub serial: String,
    /// Total labels on the roll.
    pub total: u16,
    /// Labels used so far.
    pub used: u16,
    /// Label type (§11.3), e.g. 1 = with gaps.
    pub label_type: u8,
}

impl RfidTag {
    /// Parse a `1B` payload; `None` means no tag (1-byte payload).
    pub fn parse(data: &[u8]) -> Result<Option<Self>, ParseError> {
        if data.len() <= 1 {
            return Ok(None);
        }
        let bad = || ParseError {
            what: "rfid",
            data: data.to_vec(),
        };
        let mut r = data;
        let mut take = |n: usize| -> Result<&[u8], ParseError> {
            let (a, b) = r.split_at_checked(n).ok_or_else(bad)?;
            r = b;
            Ok(a)
        };
        let hex = |b: &[u8]| b.iter().map(|x| format!("{x:02x}")).collect::<String>();
        let uuid = hex(take(8)?);
        let n = take(1)?[0] as usize;
        let barcode = String::from_utf8_lossy(take(n)?).into_owned();
        let n = take(1)?[0] as usize;
        let serial = String::from_utf8_lossy(take(n)?).into_owned();
        let tail = take(5)?;
        Ok(Some(Self {
            uuid,
            barcode,
            serial,
            total: u16::from_be_bytes([tail[0], tail[1]]),
            used: u16::from_be_bytes([tail[2], tail[3]]),
            label_type: tail[4],
        }))
    }

    /// Labels left on the roll.
    pub fn remaining(&self) -> u16 {
        self.total.saturating_sub(self.used)
    }
}

/// `PrintStatus` (`B3`) response (§7.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrintStatus {
    /// Pages finished.
    pub page: u16,
    /// Current page print progress, 0–100.
    pub print_percent: u8,
    /// Current page feed progress, 0–100.
    pub feed_percent: u8,
    /// Error code (§11.4), if any. Present on 8- and 10-byte responses.
    pub error: Option<PrinterError>,
    /// Busy flag (byte 7) while a job is running.
    pub busy: bool,
}

impl PrintStatus {
    /// Parse a `B3` payload.
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < 4 {
            return err("print status", data);
        }
        Ok(Self {
            page: u16::from_be_bytes([data[0], data[1]]),
            print_percent: data[2],
            feed_percent: data[3],
            error: data
                .get(6)
                .copied()
                .filter(|&e| e != 0)
                .map(PrinterError::from_code),
            busy: data.get(7).is_some_and(|&b| b != 0),
        })
    }
}

/// Parse a one-byte (or `01 00`) boolean acknowledgement.
pub fn parse_ack(data: &[u8]) -> Result<bool, ParseError> {
    match data.first() {
        Some(&b) => Ok(b == 1),
        None => err("acknowledgement", data),
    }
}

/// Printer error codes (`DB` payload, PrintStatus error byte; §11.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum PrinterError {
    CoverOpen,
    NoPaper,
    LowBattery,
    BatteryException,
    UserCancel,
    DataError,
    Overheat,
    PaperOutException,
    Busy,
    NoPrintHead,
    TemperatureLow,
    PrintHeadLoose,
    NoRibbon,
    WrongRibbon,
    UsedRibbon,
    WrongPaper,
    ReceiveDataTimeout,
    IllegalPage,
    Other(u8),
}

impl PrinterError {
    /// Decode an error code.
    pub fn from_code(code: u8) -> Self {
        match code {
            0x01 => Self::CoverOpen,
            0x02 => Self::NoPaper,
            0x03 => Self::LowBattery,
            0x04 => Self::BatteryException,
            0x05 => Self::UserCancel,
            0x06 => Self::DataError,
            0x07 => Self::Overheat,
            0x08 => Self::PaperOutException,
            0x09 => Self::Busy,
            0x0A => Self::NoPrintHead,
            0x0B => Self::TemperatureLow,
            0x0C => Self::PrintHeadLoose,
            0x0D => Self::NoRibbon,
            0x0E => Self::WrongRibbon,
            0x0F => Self::UsedRibbon,
            0x10 => Self::WrongPaper,
            0x32 => Self::IllegalPage,
            0x34 => Self::ReceiveDataTimeout,
            n => Self::Other(n),
        }
    }
}

impl fmt::Display for PrinterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Other(n) => write!(f, "printer error 0x{n:02X}"),
            e => write!(f, "{e:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_data_protocol_versions() {
        let b1 = [
            0x30, 0x30, 0x03, 0x20, 0, 0xC8, 0, 0, 0, 0x0F, 0x01, 0x02, 0x04, 0x01, 0x98, 0,
        ];
        assert_eq!(
            StatusData::parse(&b1),
            StatusData {
                protocol_version: 3,
                supports_color: true
            }
        );
        let d101 = [0x30, 0x30, 0x06, 0xA4, 0, 0xC8, 0, 0, 0, 0x0A];
        assert_eq!(StatusData::parse(&d101).protocol_version, 0);
    }

    #[test]
    fn serials() {
        assert_eq!(parse_serial(b"I812130338").as_deref(), Some("I812130338"));
        assert_eq!(
            parse_serial(&[0xF3, 0x10, 0x02, 0x01, 0x93]).as_deref(),
            Some("F310020193")
        );
    }

    #[test]
    fn heartbeats() {
        // D101, lid closed (inverted model): [8] = 1.
        let d101 = [0x1E, 0x9A, 0x00, 0x70, 0x00, 0x70, 0x32, 0x46, 0x01, 0x03];
        let h = Heartbeat::parse(0xDD, &d101, true).unwrap();
        assert_eq!(
            (h.lid_closed, h.battery_percent, h.paper_present),
            (Some(true), Some(75), None)
        );
        // B1, no roll.
        let b1 = [
            0x20, 0x72, 0x00, 0x86, 0x00, 0x86, 0x00, 0x00, 0x47, 0x00, 0x04, 0x01, 0x00,
        ];
        let h = Heartbeat::parse(0xDD, &b1, false).unwrap();
        assert_eq!(
            (h.lid_closed, h.battery_percent, h.paper_present),
            (Some(true), Some(100), Some(false))
        );
        let b1_d9 = [0x20, 0x72, 0x04, 0x47, 0x00, 0x00, 0x01, 0x00, 0x00];
        let h = Heartbeat::parse(0xD9, &b1_d9, false).unwrap();
        assert_eq!(
            (h.paper_present, h.paper_rfid_ok, h.temperature),
            (Some(true), Some(true), Some(0x47))
        );
    }

    #[test]
    fn rfid_and_status() {
        let data = [
            0x88, 0x1d, 0xa0, 0x85, 0x34, 0x91, 0x00, 0x00, 0x08, b'0', b'1', b'2', b'2', b'2',
            b'2', b'8', b'1', 0x10, b'P', b'Z', b'0', b'F', b'1', b'2', b'8', b'0', b'0', b'3',
            b'0', b'1', b'0', b'6', b'7', b'5', 0x00, 0xc0, 0x00, 0x32, 0x01,
        ];
        let tag = RfidTag::parse(&data).unwrap().unwrap();
        assert_eq!(
            (tag.barcode.as_str(), tag.total, tag.used, tag.remaining()),
            ("01222281", 192, 50, 142)
        );
        assert_eq!(RfidTag::parse(&[0]).unwrap(), None);

        let s = PrintStatus::parse(&[0, 0, 0, 0, 0x03, 0x1F, 0x02, 0, 0, 0]).unwrap();
        assert_eq!(s.error, Some(PrinterError::NoPaper));
        let s = PrintStatus::parse(&[0, 1, 0x64, 0x64, 0x06, 0xA2, 0, 0]).unwrap();
        assert_eq!((s.page, s.error, s.busy), (1, None, false));
    }
}
