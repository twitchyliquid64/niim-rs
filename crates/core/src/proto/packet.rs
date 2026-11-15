//! Packet framing and receive-side re-assembly (PROTOCOL.md §3).

use std::fmt;

/// Packet head, `55 55`.
pub const HEAD: [u8; 2] = [0x55, 0x55];
/// Packet tail, `AA AA`.
pub const TAIL: [u8; 2] = [0xAA, 0xAA];
/// Extra byte sent before the Connect (`0xC1`) packet only.
pub const CONNECT_PREFIX: u8 = 0x03;

/// One standard (8-bit length, XOR checksum) packet.
#[derive(Clone, PartialEq, Eq)]
pub struct Packet {
    /// Command ID.
    pub cmd: u8,
    /// Payload, at most 255 bytes.
    pub data: Vec<u8>,
}

impl Packet {
    /// Create a packet.
    ///
    /// # Panics
    /// If `data` is longer than 255 bytes. Every packet this crate builds is bounded well below that;
    /// use [`Packet::try_new`] for untrusted sizes.
    pub fn new(cmd: u8, data: impl Into<Vec<u8>>) -> Self {
        Self::try_new(cmd, data).expect("packet payload longer than 255 bytes")
    }

    /// Create a packet, or return `None` if `data` is longer than 255 bytes.
    pub fn try_new(cmd: u8, data: impl Into<Vec<u8>>) -> Option<Self> {
        let data = data.into();
        (data.len() <= 0xFF).then_some(Self { cmd, data })
    }

    /// XOR checksum over CMD, LEN and DATA.
    pub fn checksum(&self) -> u8 {
        self.data
            .iter()
            .fold(self.cmd ^ self.data.len() as u8, |acc, b| acc ^ b)
    }

    /// Serialise for the wire, including the Connect prefix where required (§3.2).
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.data.len() + 8);
        if self.cmd == super::cmd::CONNECT {
            out.push(CONNECT_PREFIX);
        }
        out.extend_from_slice(&HEAD);
        out.push(self.cmd);
        out.push(self.data.len() as u8);
        out.extend_from_slice(&self.data);
        out.push(self.checksum());
        out.extend_from_slice(&TAIL);
        out
    }
}

impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Packet({:02X}:", self.cmd)?;
        for b in &self.data {
            write!(f, " {b:02X}")?;
        }
        write!(f, ")")
    }
}

/// Re-assembles a byte stream (BLE notifications, USB bulk reads) into packets (§3.4).
///
/// Junk before a packet head is discarded, and frames with a bad tail or checksum are dropped
/// one byte at a time until the stream re-synchronises.
#[derive(Default, Debug)]
pub struct Reassembler {
    buf: Vec<u8>,
}

impl Reassembler {
    /// Create an empty re-assembler.
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of received bytes, returning every complete packet now available.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<Packet> {
        self.buf.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            match self.buf.windows(2).position(|w| w == HEAD) {
                Some(0) => {}
                Some(n) => {
                    log::debug!("discarding {n} junk bytes before packet head");
                    self.buf.drain(..n);
                }
                None => {
                    // Keep a trailing 0x55: it may be the first half of a head.
                    let keep = usize::from(self.buf.last() == Some(&HEAD[0]));
                    let drop = self.buf.len() - keep;
                    if drop > 0 {
                        log::debug!("discarding {drop} junk bytes");
                        self.buf.drain(..drop);
                    }
                    return out;
                }
            }
            if self.buf.len() < 4 {
                return out;
            }
            let len = self.buf[3] as usize;
            let total = len + 7;
            if self.buf.len() < total {
                return out;
            }
            let pkt = Packet {
                cmd: self.buf[2],
                data: self.buf[4..4 + len].to_vec(),
            };
            if self.buf[total - 2..total] != TAIL || self.buf[4 + len] != pkt.checksum() {
                log::debug!("bad frame, resynchronising");
                self.buf.drain(..1);
                continue;
            }
            self.buf.drain(..total);
            out.push(pkt);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_matches_captures() {
        assert_eq!(
            Packet::new(0x40, [0x08]).encode(),
            [0x55, 0x55, 0x40, 0x01, 0x08, 0x49, 0xAA, 0xAA]
        );
        assert_eq!(
            Packet::new(0xC1, [0x01]).encode(),
            [0x03, 0x55, 0x55, 0xC1, 0x01, 0x01, 0xC1, 0xAA, 0xAA]
        );
    }

    #[test]
    fn reassembles_split_and_concatenated() {
        let a = Packet::new(0x4A, [0x04]).encode();
        let b = Packet::new(0xF6, [0x01]).encode();
        let stream: Vec<u8> = [&[0x00, 0x12][..], &a, &b].concat();
        let mut r = Reassembler::new();
        let mut got = Vec::new();
        for chunk in stream.chunks(3) {
            got.extend(r.push(chunk));
        }
        assert_eq!(got, [Packet::new(0x4A, [0x04]), Packet::new(0xF6, [0x01])]);
    }

    #[test]
    fn drops_bad_checksum_and_recovers() {
        let mut bad = Packet::new(0x4A, [0x04]).encode();
        bad[5] ^= 0xFF;
        let good = Packet::new(0x31, [0x01]).encode();
        let mut r = Reassembler::new();
        assert_eq!(r.push(&[bad, good].concat()), [Packet::new(0x31, [0x01])]);
    }
}
