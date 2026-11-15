//! A transport that replays a recorded packet exchange (the `captures/` format), for tests and
//! for developing without hardware.
//!
//! Dump format: `>> HEX…` lines are requests and the `<< HEX…` lines after each are its responses.
//! Text in parentheses and lines starting with `#` are ignored; a `#` line also ends the current
//! request, so orphan `<<` lines after it are skipped. A request that isn't in the dump is answered
//! with `NotSupported`, like niimbluelib's virtual client.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use tokio::sync::mpsc;

use super::{BoxFuture, Connection, Transport};
use crate::proto::{Packet, cmd};
use crate::{Error, Result};

#[derive(Debug, Clone)]
struct Exchange {
    request: Vec<u8>,
    responses: Vec<Vec<u8>>,
}

/// Replay transport. Matching requests are consumed in order, so repeated identical requests get
/// successive recorded answers (the last one repeats once exhausted).
pub struct ReplayTransport {
    script: Mutex<Vec<VecDeque<Exchange>>>,
    sent: Mutex<Vec<Vec<u8>>>,
    tx: Mutex<Option<mpsc::UnboundedSender<Vec<u8>>>>,
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let s: String = s
        .split('(')
        .next()?
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

impl ReplayTransport {
    /// Build from dump text and open it.
    pub fn open(dump: &str) -> (Arc<Self>, Connection) {
        let mut exchanges: Vec<Exchange> = Vec::new();
        let mut current: Option<Exchange> = None;
        for line in dump.lines().map(str::trim) {
            if line.starts_with('#') {
                exchanges.extend(current.take());
            } else if let Some(rest) = line.strip_prefix(">>") {
                exchanges.extend(current.take());
                current = parse_hex(rest).map(|request| Exchange {
                    request,
                    responses: Vec::new(),
                });
            } else if let Some(rest) = line.strip_prefix("<<")
                && let (Some(cur), Some(bytes)) = (current.as_mut(), parse_hex(rest))
            {
                cur.responses.push(bytes);
            }
        }
        exchanges.extend(current);

        // Group identical requests into queues, keeping first-seen order.
        let mut script: Vec<VecDeque<Exchange>> = Vec::new();
        for ex in exchanges {
            match script.iter_mut().find(|q| q[0].request == ex.request) {
                Some(q) => q.push_back(ex),
                None => script.push(VecDeque::from([ex])),
            }
        }

        let (tx, incoming) = mpsc::unbounded_channel();
        let t = Arc::new(Self {
            script: Mutex::new(script),
            sent: Mutex::new(Vec::new()),
            tx: Mutex::new(Some(tx)),
        });
        let conn = Connection {
            transport: t.clone(),
            incoming,
        };
        (t, conn)
    }

    /// Every frame written so far.
    pub fn sent(&self) -> Vec<Vec<u8>> {
        self.sent.lock().unwrap().clone()
    }
}

impl Transport for ReplayTransport {
    fn write<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.sent.lock().unwrap().push(frame.to_vec());
            // One-way row packets get no answer.
            let body = frame.strip_prefix(&[0x03]).unwrap_or(frame);
            if matches!(
                body.get(2),
                Some(
                    &(cmd::PRINT_EMPTY_ROW | cmd::PRINT_BITMAP_ROW | cmd::PRINT_BITMAP_ROW_INDEXED)
                )
            ) {
                return Ok(());
            }
            let responses = {
                let mut script = self.script.lock().unwrap();
                match script.iter_mut().find(|q| q[0].request == frame) {
                    Some(q) if q.len() > 1 => q.pop_front().unwrap().responses,
                    Some(q) => q[0].responses.clone(),
                    None => vec![Packet::new(cmd::IN_NOT_SUPPORTED, [1]).encode()],
                }
            };
            let tx = self.tx.lock().unwrap();
            let tx = tx.as_ref().ok_or(Error::Disconnected)?;
            for r in responses {
                let _ = tx.send(r);
            }
            Ok(())
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<()>> {
        self.tx.lock().unwrap().take();
        Box::pin(async { Ok(()) })
    }

    fn describe(&self) -> String {
        "replay".into()
    }
}
