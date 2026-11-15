//! Layer 2: byte transports and discovery.
//!
//! A transport is a write half ([`Transport::write`], one framed packet per call) plus a channel
//! of received byte chunks ([`Connection::incoming`]); chunk boundaries are arbitrary. The
//! incoming channel closing means the link dropped.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use tokio::sync::mpsc;

use crate::Result;

#[cfg(feature = "ble")]
pub mod ble;
pub mod replay;
#[cfg(feature = "usb")]
pub mod usb;

/// A boxed, `Send` future (keeps [`Transport`] dyn-compatible).
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Write side of a byte link to a printer.
pub trait Transport: Send + Sync + 'static {
    /// Write one complete framed packet.
    fn write<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Result<()>>;
    /// Close the link. Idempotent.
    fn close(&self) -> BoxFuture<'_, Result<()>>;
    /// Short human-readable description, e.g. `BLE D101-F310020193`.
    fn describe(&self) -> String;
}

/// An open link: the write side and the stream of received chunks.
pub struct Connection {
    /// Write side.
    pub transport: Arc<dyn Transport>,
    /// Received byte chunks. Closes when the link drops.
    pub incoming: mpsc::UnboundedReceiver<Vec<u8>>,
}
