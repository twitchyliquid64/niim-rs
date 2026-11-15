//! Error type.

use std::time::Duration;

use crate::proto::{ParseError, PrinterError};

/// Result alias for this crate.
pub type Result<T, E = Error> = std::result::Result<T, E>;

/// Everything that can go wrong talking to a printer.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The underlying transport (BLE, USB) failed.
    #[error("transport error: {0}")]
    Transport(String),
    /// The connection closed.
    #[error("printer disconnected")]
    Disconnected,
    /// No response within the timeout.
    #[error("no response to command 0x{cmd:02X} within {timeout:?}")]
    Timeout {
        /// Request command ID.
        cmd: u8,
        /// How long we waited.
        timeout: Duration,
    },
    /// The printer answered `NotSupported` (`0x00`).
    #[error("command 0x{cmd:02X} not supported by this printer")]
    NotSupported {
        /// Request command ID.
        cmd: u8,
    },
    /// The printer reported an error (`0xDB`, or the PrintStatus error byte).
    #[error("printer error: {0}")]
    Printer(PrinterError),
    /// The printer answered `00` (refused / not ready) to a request that must succeed.
    #[error("printer refused command 0x{cmd:02X}")]
    Refused {
        /// Request command ID.
        cmd: u8,
    },
    /// A response could not be parsed.
    #[error(transparent)]
    Parse(#[from] ParseError),
    /// The connect handshake failed.
    #[error("connect handshake failed: {0}")]
    Handshake(String),
    /// This model, or this operation on it, isn't supported by the driver yet.
    #[error("unsupported: {0}")]
    Unsupported(String),
    /// Bad caller input (image too wide, density out of range, …).
    #[error("invalid argument: {0}")]
    InvalidArgument(String),
}
