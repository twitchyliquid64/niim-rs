//! A pure-Rust userland driver for NIIMBOT label printers, with an async API.
//!
//! ```no_run
//! # async fn demo() -> niim_core::Result<()> {
//! use std::time::Duration;
//! use niim_core::{PrintOptions, discover};
//!
//! let found = discover(Duration::from_secs(3)).await?;
//! let printer = found.first().expect("no printer").connect().await?;
//! println!("{:?}", printer.status().await?);
//!
//! let mut label = printer.label_canvas(50.0, 30.0); // as the label is read
//! label.fill_rect(16, 16, 100, 40, true);
//! printer.print(&label, &PrintOptions::default()).await?;
//! # Ok(()) }
//! ```
//!
//! Layers, bottom to top (see `PROTOCOL.md`):
//! 1. [`proto`]: framing, commands, parsers, rasters, model table (no I/O).
//! 2. [`transport`]: byte links and discovery (BLE, USB, replay).
//! 3. [`session`]: request/response matching, pacing, unsolicited events.
//! 4. [`printer`]: handshake, normalised status, print sequences, label geometry.
//! 5. [`discover`] / [`Discovered`]: find and connect.

#![warn(missing_docs)]

mod error;
pub mod printer;
pub mod proto;
pub mod session;
pub mod transport;

#[cfg(any(feature = "ble", feature = "usb"))]
use std::time::Duration;

pub use error::{Error, Result};
pub use printer::{Paper, PrintOptions, Printer, PrinterInfo, Progress, Status};
pub use proto::{Bitmap, ModelInfo};

#[cfg(any(feature = "ble", feature = "usb"))]
/// A printer found by [`discover`], not yet connected.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub enum Discovered {
    /// Bluetooth LE.
    #[cfg(feature = "ble")]
    Ble(transport::ble::BleDevice),
    /// USB.
    #[cfg(feature = "usb")]
    Usb(transport::usb::UsbDevice),
}

#[cfg(any(feature = "ble", feature = "usb"))]
impl Discovered {
    /// Display name, e.g. `D101-F310020193`.
    pub fn name(&self) -> &str {
        match self {
            #[cfg(feature = "ble")]
            Self::Ble(d) => &d.name,
            #[cfg(feature = "usb")]
            Self::Usb(d) => &d.name,
        }
    }

    /// `"ble"` or `"usb"`.
    pub fn kind(&self) -> &'static str {
        match self {
            #[cfg(feature = "ble")]
            Self::Ble(_) => "ble",
            #[cfg(feature = "usb")]
            Self::Usb(_) => "usb",
        }
    }

    /// Open the link and run the handshake.
    pub async fn connect(&self) -> Result<Printer> {
        let conn = match self {
            #[cfg(feature = "ble")]
            Self::Ble(d) => d.open().await?,
            #[cfg(feature = "usb")]
            Self::Usb(d) => d.open().await?,
        };
        Printer::connect(conn, self.name()).await
    }
}

#[cfg(any(feature = "ble", feature = "usb"))]
/// Find printers on every enabled transport. BLE scans for `ble_scan`; USB is instantaneous.
/// A transport that fails (e.g. no Bluetooth adapter) is logged and skipped.
pub async fn discover(ble_scan: Duration) -> Result<Vec<Discovered>> {
    #[allow(unused_mut)]
    let mut found = Vec::new();
    #[cfg(feature = "usb")]
    match transport::usb::scan().await {
        Ok(devs) => found.extend(devs.into_iter().map(Discovered::Usb)),
        Err(e) => log::warn!("USB discovery failed: {e}"),
    }
    #[cfg(feature = "ble")]
    match transport::ble::scan(ble_scan).await {
        Ok(devs) => found.extend(devs.into_iter().map(Discovered::Ble)),
        Err(e) => log::warn!("BLE discovery failed: {e}"),
    }
    let _ = ble_scan;
    Ok(found)
}
