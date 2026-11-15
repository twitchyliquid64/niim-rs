//! Command IDs and typed request builders (PROTOCOL.md §4, §11.1).

use std::time::Duration;

use super::Packet;

/// Command IDs. Requests are host → printer, `IN_*` are printer → host.
pub mod cmd {
    #![allow(missing_docs)]
    pub const PRINT_START: u8 = 0x01;
    pub const PAGE_START: u8 = 0x03;
    pub const SET_PAGE_SIZE: u8 = 0x13;
    pub const PRINT_QUANTITY: u8 = 0x15;
    pub const RFID_INFO: u8 = 0x1A;
    pub const RFID_INFO2: u8 = 0x1C;
    pub const PRINT_CLEAR: u8 = 0x20;
    pub const SET_DENSITY: u8 = 0x21;
    pub const SET_LABEL_TYPE: u8 = 0x23;
    pub const SET_AUTO_SHUTDOWN: u8 = 0x27;
    pub const PRINTER_INFO: u8 = 0x40;
    pub const SOUND_SETTINGS: u8 = 0x58;
    pub const GET_PAPER_INFO: u8 = 0x59;
    pub const PRINT_BITMAP_ROW_INDEXED: u8 = 0x83;
    pub const PRINT_EMPTY_ROW: u8 = 0x84;
    pub const PRINT_BITMAP_ROW: u8 = 0x85;
    pub const PRINT_STATUS: u8 = 0xA3;
    pub const PRINTER_STATUS_DATA: u8 = 0xA5;
    pub const GET_CAPABILITIES: u8 = 0xAF;
    pub const CONNECT: u8 = 0xC1;
    pub const CANCEL_PRINT: u8 = 0xDA;
    pub const HEARTBEAT: u8 = 0xDC;
    pub const PAGE_END: u8 = 0xE3;
    pub const PRINT_END: u8 = 0xF3;

    pub const IN_NOT_SUPPORTED: u8 = 0x00;
    pub const IN_PRINT_START: u8 = 0x02;
    pub const IN_PAGE_START: u8 = 0x04;
    pub const IN_SET_PAGE_SIZE: u8 = 0x14;
    pub const IN_PRINT_QUANTITY: u8 = 0x16;
    pub const IN_RFID_INFO: u8 = 0x1B;
    pub const IN_RFID_INFO2: u8 = 0x1D;
    pub const IN_PRINT_CLEAR: u8 = 0x30;
    pub const IN_SET_DENSITY: u8 = 0x31;
    pub const IN_SET_LABEL_TYPE: u8 = 0x33;
    pub const IN_SET_AUTO_SHUTDOWN: u8 = 0x37;
    pub const IN_SOUND_SETTINGS: u8 = 0x68;
    pub const IN_PAPER_INFO: u8 = 0x69;
    pub const IN_PRINT_STATUS: u8 = 0xB3;
    pub const IN_PRINTER_STATUS_DATA: u8 = 0xB5;
    pub const IN_CAPABILITIES: u8 = 0xBF;
    pub const IN_CONNECT: u8 = 0xC2;
    pub const IN_RESET_TIMEOUT: u8 = 0xC6;
    pub const IN_CANCEL_PRINT: u8 = 0xD0;
    pub const IN_CHECK_LINE: u8 = 0xD3;
    pub const IN_HEARTBEAT_ADVANCED2: u8 = 0xD9;
    pub const IN_PRINT_ERROR: u8 = 0xDB;
    pub const IN_HEARTBEAT_ADVANCED1: u8 = 0xDD;
    pub const IN_HEARTBEAT_IDENTITY: u8 = 0xDE;
    pub const IN_HEARTBEAT_TYPE2: u8 = 0xDF;
    pub const IN_PAGE_INDEX: u8 = 0xE0;
    pub const IN_PAGE_END: u8 = 0xE4;
    pub const IN_PRINT_END: u8 = 0xF4;
}

const DEFAULT_TIMEOUT: Duration = Duration::from_millis(1000);
const RFID_TIMEOUT: Duration = Duration::from_millis(5500);

/// Keys for `PrinterInfo` (`0x40`), §6.1. The response command is `0x40 + key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
#[allow(missing_docs)]
pub enum InfoKey {
    Density = 1,
    Speed = 2,
    LabelType = 3,
    Language = 6,
    AutoShutdownTime = 7,
    ModelId = 8,
    SoftwareVersion = 9,
    Battery = 10,
    SerialNumber = 11,
    HardwareVersion = 12,
    BluetoothAddress = 13,
    PrintMode = 14,
    Area = 15,
}

/// Heartbeat request types, §6.3.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum HeartbeatKind {
    /// Answered with `DD`: lid, battery, paper (older models).
    Advanced1 = 1,
    /// Answered with `DF` (layout unknown).
    Basic = 2,
    /// Answered with `DE`: firmware/hardware version, head width, DPI.
    Identity = 3,
    /// Answered with `D9`: battery, temperature, lid, paper, ribbon (protocol ≥ 3).
    Advanced2 = 4,
}

/// A request packet plus what the session should wait for.
#[derive(Debug, Clone)]
pub struct Request {
    /// The packet to send.
    pub packet: Packet,
    /// Response command IDs that complete this request. Empty means one-way (no response).
    pub accept: Vec<u8>,
    /// How long to wait for the response.
    pub timeout: Duration,
}

impl Request {
    fn new(cmd: u8, data: impl Into<Vec<u8>>, accept: &[u8]) -> Self {
        Self {
            packet: Packet::new(cmd, data),
            accept: accept.to_vec(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Override the response timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Whether this request expects no response.
    pub fn is_one_way(&self) -> bool {
        self.accept.is_empty()
    }

    /// Connect handshake (§5.1). Sent with the `03` prefix.
    pub fn connect() -> Self {
        Self::new(cmd::CONNECT, [1], &[cmd::IN_CONNECT])
    }

    /// Protocol-version query (§5.2).
    pub fn printer_status_data() -> Self {
        Self::new(
            cmd::PRINTER_STATUS_DATA,
            [1],
            &[cmd::IN_PRINTER_STATUS_DATA],
        )
    }

    /// Read one `PrinterInfo` key (§6.1).
    pub fn printer_info(key: InfoKey) -> Self {
        Self::new(
            cmd::PRINTER_INFO,
            [key as u8],
            &[cmd::PRINTER_INFO + key as u8],
        )
    }

    /// Heartbeat (§6.3).
    pub fn heartbeat(kind: HeartbeatKind) -> Self {
        Self::new(
            cmd::HEARTBEAT,
            [kind as u8],
            &[
                cmd::IN_HEARTBEAT_ADVANCED1,
                cmd::IN_HEARTBEAT_IDENTITY,
                cmd::IN_HEARTBEAT_TYPE2,
                cmd::IN_HEARTBEAT_ADVANCED2,
            ],
        )
    }

    /// Read the label roll's RFID tag (§6.4). Slow on some models.
    pub fn rfid_info() -> Self {
        Self::new(cmd::RFID_INFO, [1], &[cmd::IN_RFID_INFO]).with_timeout(RFID_TIMEOUT)
    }

    /// Set print density (§6.2).
    pub fn set_density(density: u8) -> Self {
        Self::new(cmd::SET_DENSITY, [density], &[cmd::IN_SET_DENSITY])
    }

    /// Set label type (§6.2, §11.3).
    pub fn set_label_type(label_type: u8) -> Self {
        Self::new(cmd::SET_LABEL_TYPE, [label_type], &[cmd::IN_SET_LABEL_TYPE])
    }

    /// 7-byte PrintStart: total pages and page colour (§7.3).
    pub fn print_start_7(total_pages: u16, page_color: u8) -> Self {
        let [hi, lo] = total_pages.to_be_bytes();
        Self::new(
            cmd::PRINT_START,
            [hi, lo, 0, 0, 0, 0, page_color],
            &[cmd::IN_PRINT_START],
        )
    }

    /// PageStart (§7.3).
    pub fn page_start() -> Self {
        Self::new(cmd::PAGE_START, [1], &[cmd::IN_PAGE_START])
    }

    /// 6-byte SetPageSize: rows, cols, copies (§7.3).
    pub fn set_page_size_6(rows: u16, cols: u16, copies: u16) -> Self {
        let mut d = Vec::with_capacity(6);
        d.extend_from_slice(&rows.to_be_bytes());
        d.extend_from_slice(&cols.to_be_bytes());
        d.extend_from_slice(&copies.to_be_bytes());
        Self::new(cmd::SET_PAGE_SIZE, d, &[cmd::IN_SET_PAGE_SIZE])
    }

    /// PageEnd (§7.3).
    pub fn page_end() -> Self {
        Self::new(cmd::PAGE_END, [1], &[cmd::IN_PAGE_END])
    }

    /// PrintStatus (§7.5).
    pub fn print_status() -> Self {
        Self::new(cmd::PRINT_STATUS, [1], &[cmd::IN_PRINT_STATUS])
    }

    /// PrintEnd (§7.3).
    pub fn print_end() -> Self {
        Self::new(cmd::PRINT_END, [1], &[cmd::IN_PRINT_END])
    }

    /// A one-way packet, e.g. a bitmap row (§4.2).
    pub fn one_way(packet: Packet) -> Self {
        Self {
            packet,
            accept: Vec::new(),
            timeout: Duration::ZERO,
        }
    }
}
