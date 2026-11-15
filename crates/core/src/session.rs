//! Layer 3: request/response over a transport (PROTOCOL.md §4).
//!
//! A [`Session`] owns the reader task that re-assembles packets, matches responses to the one
//! in-flight request, turns `NotSupported`/`PrintError` replies into errors, paces writes, and
//! broadcasts everything else (unsolicited packets) to [`Session::events`].

use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::time::Instant;

use crate::proto::{Packet, PrinterError, Reassembler, Request, cmd};
use crate::transport::{Connection, Transport};
use crate::{Error, Result};

/// Minimum gap between writes; printers drop packets without it (§2.1).
pub const DEFAULT_PACING: Duration = Duration::from_millis(10);

struct Waiter {
    accept: Vec<u8>,
    tx: oneshot::Sender<Packet>,
}

struct Inner {
    transport: Arc<dyn Transport>,
    request_lock: Mutex<()>,
    waiter: StdMutex<Option<Waiter>>,
    events: broadcast::Sender<Packet>,
    last_write: Mutex<Instant>,
    pacing: Duration,
}

/// A packet-level connection to one printer. Cheap to clone.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl Session {
    /// Start a session on an open connection. Spawns the reader task (needs a Tokio runtime).
    pub fn new(conn: Connection) -> Self {
        let (events, _) = broadcast::channel(256);
        let inner = Arc::new(Inner {
            transport: conn.transport,
            request_lock: Mutex::new(()),
            waiter: StdMutex::new(None),
            events,
            last_write: Mutex::new(Instant::now() - Duration::from_secs(1)),
            pacing: DEFAULT_PACING,
        });
        tokio::spawn(reader(Arc::downgrade(&inner), conn.incoming));
        Self { inner }
    }

    /// The transport's description.
    pub fn describe(&self) -> String {
        self.inner.transport.describe()
    }

    /// Unsolicited packets (anything not consumed as a response).
    pub fn events(&self) -> broadcast::Receiver<Packet> {
        self.inner.events.subscribe()
    }

    async fn write_paced(&self, frame: &[u8]) -> Result<()> {
        let mut last = self.inner.last_write.lock().await;
        tokio::time::sleep_until(*last + self.inner.pacing).await;
        log::trace!(">> {frame:02X?}");
        let r = self.inner.transport.write(frame).await;
        *last = Instant::now();
        r
    }

    /// Send a request and wait for its response. One-way requests return `None` right after
    /// sending. `NotSupported` and `PrintError` replies become [`Error::NotSupported`] and
    /// [`Error::Printer`].
    pub async fn request(&self, req: &Request) -> Result<Option<Packet>> {
        let _guard = self.inner.request_lock.lock().await;
        if req.is_one_way() {
            self.write_paced(&req.packet.encode()).await?;
            return Ok(None);
        }
        let (tx, rx) = oneshot::channel();
        *self.inner.waiter.lock().unwrap() = Some(Waiter {
            accept: req.accept.clone(),
            tx,
        });
        if let Err(e) = self.write_paced(&req.packet.encode()).await {
            self.inner.waiter.lock().unwrap().take();
            return Err(e);
        }
        let reply = tokio::time::timeout(req.timeout, rx).await;
        self.inner.waiter.lock().unwrap().take();
        let pkt = match reply {
            Ok(Ok(p)) => p,
            Ok(Err(_)) => return Err(Error::Disconnected),
            Err(_) => {
                return Err(Error::Timeout {
                    cmd: req.packet.cmd,
                    timeout: req.timeout,
                });
            }
        };
        match pkt.cmd {
            cmd::IN_NOT_SUPPORTED => Err(Error::NotSupported {
                cmd: req.packet.cmd,
            }),
            cmd::IN_PRINT_ERROR => Err(Error::Printer(PrinterError::from_code(
                pkt.data.first().copied().unwrap_or(0),
            ))),
            _ => Ok(Some(pkt)),
        }
    }

    /// Like [`Session::request`] for a request that must have a response.
    pub async fn call(&self, req: &Request) -> Result<Packet> {
        self.request(req).await?.ok_or(Error::Disconnected)
    }

    /// Send a batch of one-way packets back to back (bitmap rows), holding the request lock so
    /// nothing interleaves.
    pub async fn send_one_way(&self, packets: &[Packet]) -> Result<()> {
        let _guard = self.inner.request_lock.lock().await;
        for p in packets {
            self.write_paced(&p.encode()).await?;
        }
        Ok(())
    }

    /// Close the transport.
    pub async fn close(&self) -> Result<()> {
        self.inner.transport.close().await
    }
}

async fn reader(
    inner: std::sync::Weak<Inner>,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
) {
    let mut reassembler = Reassembler::new();
    while let Some(chunk) = incoming.recv().await {
        let Some(inner) = inner.upgrade() else { return };
        for pkt in reassembler.push(&chunk) {
            log::trace!("<< {pkt:?}");
            let mut slot = inner.waiter.lock().unwrap();
            let matches = slot.as_ref().is_some_and(|w| {
                w.accept.contains(&pkt.cmd)
                    || matches!(pkt.cmd, cmd::IN_NOT_SUPPORTED | cmd::IN_PRINT_ERROR)
            });
            if matches {
                let _ = slot.take().unwrap().tx.send(pkt);
            } else {
                drop(slot);
                log::debug!("unsolicited {pkt:?}");
                let _ = inner.events.send(pkt);
            }
        }
    }
    // Link dropped: fail the pending request.
    if let Some(inner) = inner.upgrade() {
        inner.waiter.lock().unwrap().take();
    }
}
