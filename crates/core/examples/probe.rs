//! Throwaway BLE probe for reverse-engineering NIIMBOT printers.
//!
//! Usage:
//!   cargo run -p niim-core --example probe -- scan [SECS]
//!   cargo run -p niim-core --example probe -- probe NAME_SUBSTRING
//!   cargo run -p niim-core --example probe -- send NAME_SUBSTRING CMD:HEXDATA [CMD:HEXDATA ...]
//!
//! `probe` runs a fixed, read-only query script. `send` sends arbitrary packets
//! (e.g. `dc:04 40:08`); a Connect handshake is always sent first.
//! Every packet in both directions is logged in the `>>` / `<<` dump format
//! used by niimbluelib's test captures.

use std::time::Duration;

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn frame(cmd: u8, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 8);
    if cmd == 0xC1 {
        out.push(0x03);
    }
    out.extend_from_slice(&[0x55, 0x55, cmd, data.len() as u8]);
    out.extend_from_slice(data);
    out.push(data.iter().fold(cmd ^ data.len() as u8, |a, b| a ^ b));
    out.extend_from_slice(&[0xAA, 0xAA]);
    out
}

fn hex(b: &[u8]) -> String {
    b.iter()
        .map(|x| format!("{x:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

fn name(cmd: u8) -> &'static str {
    match cmd {
        0x00 => "In_NotSupported",
        0x02 => "In_PrintStart",
        0x04 => "In_PageStart",
        0x14 => "In_SetPageSize",
        0x1B => "In_RfidInfo",
        0x1D => "In_RfidInfo2",
        0x41..=0x4F => "In_PrinterInfo*",
        0x68 => "In_SoundSettings",
        0x69 => "In_GetPaperInfo",
        0xB3 => "In_PrintStatus",
        0xB5 => "In_PrinterStatusData",
        0xBF => "In_GetPrinterCapabilities",
        0xC2 => "In_Connect",
        0xC6 => "In_ResetTimeout",
        0xD3 => "In_PrinterCheckLine",
        0xD9 => "In_HeartbeatAdvanced2",
        0xDB => "In_PrintError",
        0xDD => "In_HeartbeatAdvanced1",
        0xDE => "In_HeartbeatBasic",
        0xDF => "In_HeartbeatPrinterInfo",
        0xE0 => "In_PrinterPageIndex",
        0xE4 => "In_PageEnd",
        0xF4 => "In_PrintEnd",
        0xC1 => "Connect",
        0xA5 => "PrinterStatusData",
        0x40 => "PrinterInfo",
        0xDC => "Heartbeat",
        0x58 => "SoundSettings",
        0x1A => "RfidInfo",
        0x1C => "RfidInfo2",
        0x59 => "GetPaperInfo",
        0xAF => "GetPrinterCapabilities",
        0xA3 => "PrintStatus",
        _ => "?",
    }
}

/// Re-assembles notification chunks into packets (see PROTOCOL.md §3.4).
#[derive(Default)]
struct Reassembler(Vec<u8>);

impl Reassembler {
    fn push(&mut self, chunk: &[u8]) -> Vec<Vec<u8>> {
        self.0.extend_from_slice(chunk);
        let mut out = Vec::new();
        loop {
            let Some(start) = self.0.windows(2).position(|w| w == [0x55, 0x55]) else {
                if !self.0.is_empty() && self.0 != [0x55] {
                    println!("   !! discarding junk: {}", hex(&self.0));
                    self.0.clear();
                }
                break;
            };
            if start > 0 {
                println!("   !! discarding junk: {}", hex(&self.0[..start]));
                self.0.drain(..start);
            }
            if self.0.len() < 4 {
                break;
            }
            let total = self.0[3] as usize + 7;
            if self.0.len() < total {
                break;
            }
            let pkt: Vec<u8> = self.0.drain(..total).collect();
            let len = pkt[3] as usize;
            let chk = pkt[4..4 + len].iter().fold(pkt[2] ^ pkt[3], |a, b| a ^ b);
            if pkt[total - 2..] != [0xAA, 0xAA] || chk != pkt[4 + len] {
                println!("   !! bad frame: {}", hex(&pkt));
                continue;
            }
            out.push(pkt);
        }
        out
    }
}

struct Link {
    p: Peripheral,
    ch: Characteristic,
    rx: mpsc::UnboundedReceiver<Vec<u8>>,
}

impl Link {
    /// Send a packet, then collect responses until `wait` elapses with nothing new
    /// (after the first response, a short 300 ms grace period catches stragglers).
    async fn xfer(&mut self, cmd: u8, data: &[u8], wait: Duration) -> Result<Vec<Vec<u8>>> {
        let bytes = frame(cmd, data);
        sleep(Duration::from_millis(10)).await;
        println!(">> {} ({} {})", hex(&bytes), name(cmd), hex(data));
        self.p
            .write(&self.ch, &bytes, WriteType::WithoutResponse)
            .await?;
        let mut got = Vec::new();
        let mut window = wait;
        while let Ok(Some(pkt)) = timeout(window, self.rx.recv()).await {
            let len = pkt[3] as usize;
            println!(
                "<< {} ({}, {} bytes: {})",
                hex(&pkt),
                name(pkt[2]),
                len,
                hex(&pkt[4..4 + len])
            );
            got.push(pkt);
            window = Duration::from_millis(300);
        }
        if got.is_empty() {
            println!("<< (no response within {wait:?})");
        }
        Ok(got)
    }
}

impl Link {
    /// Send a one-way packet (bitmap rows): no response is expected.
    async fn send_only(&mut self, cmd: u8, data: &[u8]) -> Result<()> {
        let bytes = frame(cmd, data);
        sleep(Duration::from_millis(10)).await;
        println!(">> {} ({} {})", hex(&bytes), name(cmd), hex(data));
        self.p
            .write(&self.ch, &bytes, WriteType::WithoutResponse)
            .await?;
        // Surface anything unsolicited without blocking.
        while let Ok(pkt) = self.rx.try_recv() {
            println!("<< {} ({}, unsolicited)", hex(&pkt), name(pkt[2]));
        }
        Ok(())
    }
}

/// Test pattern, `rows` x `cols` (cols multiple of 8), MSB = column 0, 1 = black.
fn test_pattern(rows: usize, cols: usize) -> Vec<Vec<u8>> {
    let mut img = vec![vec![0u8; cols / 8]; rows];
    let mut set = |r: usize, c: usize| img[r][c / 8] |= 0x80 >> (c % 8);
    for r in 0..rows {
        for c in 0..cols {
            let border = r == 0 || r == rows - 1 || c == 0 || c == cols - 1;
            // Big square near (row 0, col 0); small square near (last row, col 0).
            let big = (8..48).contains(&r) && (8..48).contains(&c);
            let small = (rows - 24..rows - 8).contains(&r) && (8..24).contains(&c);
            // Ruler across the head in the middle: tick every 8 cols (1 mm), long every 32.
            let mid = rows / 2;
            let tick = c % 8 == 0 && (mid - 8..mid).contains(&r);
            let long = c % 32 == 0 && (mid - 8..mid + 12).contains(&r);
            if border || big || small || tick || long {
                set(r, c);
            }
        }
    }
    img
}

/// Alignment pattern "b": across the head, 6 bands of 32 cols; band k (0-based) holds k+1
/// small blocks, so visible bands can be identified by counting. Along the feed, a full-width
/// line every 40 rows (5 mm), thick (3 dots) at rows 0 and 160 and at the last rows.
fn pattern_b(rows: usize, cols: usize) -> Vec<Vec<u8>> {
    let mut img = vec![vec![0u8; cols / 8]; rows];
    let mut set = |r: usize, c: usize| img[r][c / 8] |= 0x80 >> (c % 8);
    for r in 0..rows {
        let thick = r < 3 || (160..163).contains(&r) || r + 3 >= rows;
        let line = thick || r % 40 == 0;
        for c in 0..cols {
            let band = c / 32;
            let in_band = (6..26).contains(&(c % 32));
            let block = (0..=band).any(|i| (125 + 12 * i..131 + 12 * i).contains(&r));
            if line || (in_band && block) {
                set(r, c);
            }
        }
    }
    img
}

/// Length ruler "c": short tick (top 32 cols) every 8 rows (1 mm) plus the last row,
/// long line (cols 96..) every 40 rows (5 mm), thick line at rows 160..163.
fn pattern_c(rows: usize, cols: usize) -> Vec<Vec<u8>> {
    let mut img = vec![vec![0u8; cols / 8]; rows];
    let mut set = |r: usize, c: usize| img[r][c / 8] |= 0x80 >> (c % 8);
    for r in 0..rows {
        for c in 0..cols {
            let short = (r % 8 == 0 || r == rows - 1) && c + 32 >= cols;
            let long = r % 40 == 0 && c >= cols / 2;
            let thick = (160..164).contains(&r) && c >= cols / 2;
            if short || long || thick {
                set(r, c);
            }
        }
    }
    img
}

/// niimbluelib "split" pixel counts (PROTOCOL.md §7.2).
fn counts(row: &[u8], head_dots: usize) -> [u8; 3] {
    let chunk = (head_dots / 8 / 3).max(1);
    let mut parts = [0u32; 3];
    let total: u32 = row.iter().map(|b| b.count_ones()).sum();
    if row.len() <= chunk * 3 {
        for (i, b) in row.iter().enumerate() {
            parts[(i / chunk).min(2)] += b.count_ones();
        }
        parts.map(|p| p.min(255) as u8)
    } else {
        [0, (total & 0xFF) as u8, (total >> 8) as u8]
    }
}

/// One test label with the B1 print sequence (PROTOCOL.md §7.4, §8).
async fn print_test(
    name_sub: &str,
    rows: usize,
    cols: usize,
    head: usize,
    density: u8,
    pattern: &str,
) -> Result<()> {
    let mut l = open(name_sub).await?;
    let t = Duration::from_millis(1500);
    l.xfer(0xC1, &[0x01], t).await?;
    let result = async {
        l.xfer(0x21, &[density], t).await?;
        l.xfer(0x23, &[0x01], t).await?;
        l.xfer(0x01, &[0x00, 0x01, 0, 0, 0, 0, 0x00], t).await?;
        l.xfer(0x03, &[0x01], t).await?;
        let (r, c) = (rows as u16, cols as u16);
        let mut ps = Vec::new();
        ps.extend_from_slice(&r.to_be_bytes());
        ps.extend_from_slice(&c.to_be_bytes());
        ps.extend_from_slice(&1u16.to_be_bytes());
        l.xfer(0x13, &ps, t).await?;

        let img = match pattern {
            "b" => pattern_b(rows, cols),
            "c" => pattern_c(rows, cols),
            _ => test_pattern(rows, cols),
        };
        let mut i = 0;
        while i < img.len() {
            let mut n = 1;
            while i + n < img.len() && n < 255 && img[i + n] == img[i] {
                n += 1;
            }
            let pos = (i as u16).to_be_bytes();
            if img[i].iter().all(|&b| b == 0) {
                l.send_only(0x84, &[pos[0], pos[1], n as u8]).await?;
            } else {
                let mut d = vec![pos[0], pos[1]];
                d.extend_from_slice(&counts(&img[i], head));
                d.push(n as u8);
                d.extend_from_slice(&img[i]);
                l.send_only(0x85, &d).await?;
            }
            i += n;
        }

        for _ in 0..20 {
            let got = l.xfer(0xE3, &[0x01], t).await?;
            if got.iter().any(|p| p[2] == 0xE4 && p[4] == 1) {
                break;
            }
            sleep(Duration::from_millis(500)).await;
        }
        for _ in 0..40 {
            sleep(Duration::from_millis(300)).await;
            let got = l.xfer(0xA3, &[0x01], t).await?;
            if got
                .iter()
                .any(|p| p[2] == 0xB3 && p[3] >= 2 && u16::from_be_bytes([p[4], p[5]]) >= 1)
            {
                break;
            }
        }
        Ok::<_, Box<dyn std::error::Error>>(())
    }
    .await;
    // Always end the print, even on error.
    l.xfer(0xF3, &[0x01], t).await?;
    l.p.disconnect().await?;
    result
}

async fn adapter() -> Result<btleplug::platform::Adapter> {
    let manager = Manager::new().await?;
    Ok(manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or("no Bluetooth adapter")?)
}

async fn scan(secs: u64) -> Result<()> {
    let central = adapter().await?;
    central.start_scan(ScanFilter::default()).await?;
    sleep(Duration::from_secs(secs)).await;
    central.stop_scan().await?;
    for p in central.peripherals().await? {
        if let Some(props) = p.properties().await? {
            println!(
                "{:<40} {:<24} rssi={:<5} services={:?} manuf={:?}",
                p.id().to_string(),
                props.local_name.unwrap_or_else(|| "-".into()),
                props.rssi.map_or("-".into(), |r| r.to_string()),
                props.services,
                props
                    .manufacturer_data
                    .iter()
                    .map(|(k, v)| format!("{k:#06x}:{}", hex(v)))
                    .collect::<Vec<_>>(),
            );
        }
    }
    Ok(())
}

async fn open(name_sub: &str) -> Result<Link> {
    let central = adapter().await?;
    central.start_scan(ScanFilter::default()).await?;
    let mut found = None;
    for _ in 0..30 {
        sleep(Duration::from_millis(500)).await;
        for p in central.peripherals().await? {
            if let Some(n) = p.properties().await?.and_then(|pr| pr.local_name)
                && n.to_lowercase().contains(&name_sub.to_lowercase())
            {
                println!("# found {n} ({})", p.id());
                found = Some(p);
            }
        }
        if found.is_some() {
            break;
        }
    }
    central.stop_scan().await?;
    let p = found.ok_or("printer not found")?;

    p.connect().await?;
    p.discover_services().await?;
    println!("# GATT services:");
    for s in p.services() {
        println!("#   service {} primary={}", s.uuid, s.primary);
        for c in &s.characteristics {
            println!("#     char {} props={:?}", c.uuid, c.properties);
        }
    }
    let ch = p
        .characteristics()
        .into_iter()
        .find(|c| {
            c.properties.contains(CharPropFlags::NOTIFY)
                && c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
        })
        .ok_or("no notify+write-without-response characteristic")?;
    println!("# using char {} in service {}", ch.uuid, ch.service_uuid);

    p.subscribe(&ch).await?;
    let mut notifs = p.notifications().await?;
    let (tx, rx) = mpsc::unbounded_channel();
    let uuid = ch.uuid;
    tokio::spawn(async move {
        let mut r = Reassembler::default();
        while let Some(n) = notifs.next().await {
            if n.uuid != uuid {
                continue;
            }
            for pkt in r.push(&n.value) {
                if tx.send(pkt).is_err() {
                    return;
                }
            }
        }
    });
    Ok(Link { p, ch, rx })
}

async fn probe(name_sub: &str) -> Result<()> {
    let mut l = open(name_sub).await?;
    let t = Duration::from_millis(1500);
    l.xfer(0xC1, &[0x01], t).await?;
    l.xfer(0xA5, &[0x01], t).await?;
    for key in [8u8, 11, 13, 10, 7, 3, 1, 2, 6, 9, 12, 14, 15] {
        l.xfer(0x40, &[key], t).await?;
    }
    for ty in [3u8, 1, 2, 4] {
        l.xfer(0xDC, &[ty], t).await?;
    }
    l.xfer(0x58, &[0x02, 0x01, 0x01], t).await?;
    l.xfer(0x58, &[0x02, 0x02, 0x01], t).await?;
    l.xfer(0x1A, &[0x01], Duration::from_millis(6000)).await?;
    l.xfer(0x59, &[0x01], t).await?;
    l.xfer(0xAF, &[0x01], t).await?;
    l.xfer(0xA3, &[0x01], t).await?;
    l.p.disconnect().await?;
    Ok(())
}

async fn send(name_sub: &str, specs: &[String]) -> Result<()> {
    let mut l = open(name_sub).await?;
    let t = Duration::from_millis(1500);
    l.xfer(0xC1, &[0x01], t).await?;
    for spec in specs {
        let (cmd, data) = spec.split_once(':').unwrap_or((spec, ""));
        let cmd = u8::from_str_radix(cmd, 16)?;
        let data: String = data.chars().filter(|c| c.is_ascii_hexdigit()).collect();
        let data = (0..data.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&data[i..i + 2], 16))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        l.xfer(cmd, &data, t).await?;
    }
    l.p.disconnect().await?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("scan") => scan(args.get(1).map_or(Ok(5), |s| s.parse())?).await,
        Some("probe") => probe(args.get(1).ok_or("need NAME")?).await,
        Some("send") => send(args.get(1).ok_or("need NAME")?, &args[2..]).await,
        // print-test NAME ROWS COLS HEAD_DOTS DENSITY [PATTERN a|b]
        Some("print-test") => {
            let n = |i: usize| -> Result<usize> { Ok(args.get(i).ok_or("missing arg")?.parse()?) };
            let pattern = args.get(6).map_or("a", String::as_str);
            print_test(args.get(1).ok_or("need NAME")?, n(2)?, n(3)?, n(4)?, n(5)? as u8, pattern).await
        }
        _ => Err(
            "usage: probe scan [SECS] | probe NAME | send NAME CMD:HEX... | print-test NAME ROWS COLS HEAD DENSITY"
                .into(),
        ),
    }
}
