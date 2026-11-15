//! Layer 4: a connected printer, with model-specific behaviour (PROTOCOL.md §5–§8).

use std::fmt;
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::proto::{
    Alignment, Bitmap, ConnectResult, Heartbeat, HeartbeatKind, Identity, InfoKey, ModelInfo,
    Orientation, PrintSequence, PrintStatus, Request, RfidTag, StatusData, Version, encode_rows,
    parse_ack, parse_mac, parse_model_id, parse_serial,
};
use crate::session::Session;
use crate::transport::Connection;
use crate::{Error, Result};

/// What we know about a connected printer.
#[derive(Debug, Clone)]
pub struct PrinterInfo {
    /// Advertised/transport name, e.g. `D101-F310020193`.
    pub name: String,
    /// Model ID from `PrinterInfo(ModelId)`.
    pub model_id: u16,
    /// Model table entry, if the ID is known.
    pub model: Option<&'static ModelInfo>,
    /// Protocol version (§5.2).
    pub protocol_version: u8,
    /// Serial number.
    pub serial: Option<String>,
    /// Bluetooth MAC address.
    pub mac: Option<[u8; 6]>,
    /// Firmware version (raw bytes shown as `a.bb`; see [`Version`]).
    pub firmware: Option<Version>,
    /// Hardware version.
    pub hardware: Option<Version>,
    /// Printhead width in dots.
    pub head_dots: u16,
    /// 203 or 300.
    pub dpi: u16,
    /// Allowed density range.
    pub density_range: (u8, u8),
    /// Factory default density.
    pub density_default: u8,
    /// Two-colour support flag (§5.2).
    pub supports_color: bool,
}

impl PrinterInfo {
    /// Model name, or `unknown(ID)`.
    pub fn model_name(&self) -> String {
        self.model.map_or_else(
            || format!("unknown({})", self.model_id),
            |m| m.name.to_owned(),
        )
    }

    /// Dots per millimetre.
    pub fn dots_per_mm(&self) -> f32 {
        f32::from(self.dpi) / 25.4
    }
}

/// Whether a label roll is loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Paper {
    /// Loaded.
    Present,
    /// Not loaded (or, where detected via RFID, loaded but without a readable tag).
    Absent,
    /// The printer doesn't report it.
    Unknown,
}

/// Normalised live status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// Lid closed, if reported.
    pub lid_closed: Option<bool>,
    /// Label roll.
    pub paper: Paper,
    /// Battery 0–100 %, if reported.
    pub battery_percent: Option<u8>,
    /// Raw temperature byte, if reported.
    pub temperature: Option<u8>,
    /// Current error condition from PrintStatus (e.g. cover open), if any.
    pub error: Option<crate::proto::PrinterError>,
    /// The label roll's RFID tag, when it was read to determine `paper`.
    pub consumable: Option<RfidTag>,
}

/// Progress of a print job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Progress {
    /// Raster rows sent, out of total row packets.
    Sending {
        /// Packets sent.
        sent: usize,
        /// Total packets.
        total: usize,
    },
    /// Printer progress.
    Printing {
        /// Pages finished.
        page: u16,
        /// Total pages.
        total: u16,
        /// Current page print %.
        print_percent: u8,
        /// Current page feed %.
        feed_percent: u8,
    },
}

/// Print options.
#[derive(Clone)]
pub struct PrintOptions {
    /// Density; `None` = the model's default.
    pub density: Option<u8>,
    /// Label type (§11.3); 1 = with gaps.
    pub label_type: u8,
    /// Copies of the page.
    pub copies: u16,
    /// Progress callback.
    pub on_progress: Option<Arc<dyn Fn(Progress) + Send + Sync>>,
}

impl Default for PrintOptions {
    fn default() -> Self {
        Self {
            density: None,
            label_type: 1,
            copies: 1,
            on_progress: None,
        }
    }
}

impl fmt::Debug for PrintOptions {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PrintOptions")
            .field("density", &self.density)
            .field("label_type", &self.label_type)
            .field("copies", &self.copies)
            .finish_non_exhaustive()
    }
}

struct Inner {
    session: Session,
    info: PrinterInfo,
    /// Serialises whole operations (a print job vs. status reads).
    op: Mutex<()>,
    /// Heartbeat type that works on this printer, once found.
    heartbeat: StdMutex<Option<HeartbeatKind>>,
}

/// A connected printer. Cheap to clone; methods take `&self`.
#[derive(Clone)]
pub struct Printer {
    inner: Arc<Inner>,
}

impl fmt::Debug for Printer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Printer")
            .field("info", &self.inner.info)
            .finish_non_exhaustive()
    }
}

async fn optional<T>(f: impl Future<Output = Result<T>>) -> Result<Option<T>> {
    match f.await {
        Ok(v) => Ok(Some(v)),
        Err(Error::NotSupported { .. } | Error::Timeout { .. } | Error::Parse(_)) => Ok(None),
        Err(e) => Err(e),
    }
}

impl Printer {
    /// Run the handshake on an open connection (§5.3).
    pub async fn connect(conn: Connection, name: impl Into<String>) -> Result<Self> {
        let session = Session::new(conn);
        let info = match handshake(&session, name.into()).await {
            Ok(info) => info,
            Err(e) => {
                let _ = session.close().await;
                return Err(e);
            }
        };
        log::info!(
            "connected to {} ({}, protocol {})",
            info.name,
            info.model_name(),
            info.protocol_version
        );
        Ok(Self {
            inner: Arc::new(Inner {
                session,
                info,
                op: Mutex::new(()),
                heartbeat: StdMutex::new(None),
            }),
        })
    }

    /// Printer identity and capabilities.
    pub fn info(&self) -> &PrinterInfo {
        &self.inner.info
    }

    /// The packet-level session (escape hatch for raw commands and unsolicited events).
    pub fn session(&self) -> &Session {
        &self.inner.session
    }

    /// Disconnect.
    pub async fn disconnect(&self) -> Result<()> {
        self.inner.session.close().await
    }

    async fn heartbeat(&self) -> Result<Heartbeat> {
        let s = &self.inner.session;
        let lid_inverted = self.inner.info.model.is_some_and(|m| m.lid_inverted);
        let cached = *self.inner.heartbeat.lock().unwrap();
        let candidates = match cached {
            Some(k) => vec![k],
            None if self.inner.info.protocol_version >= 3 => {
                vec![HeartbeatKind::Advanced2, HeartbeatKind::Advanced1]
            }
            None => vec![HeartbeatKind::Advanced1, HeartbeatKind::Advanced2],
        };
        let mut last = Error::Unsupported("heartbeat".into());
        for kind in candidates {
            match s.call(&Request::heartbeat(kind)).await {
                Ok(p) => {
                    *self.inner.heartbeat.lock().unwrap() = Some(kind);
                    return Ok(Heartbeat::parse(p.cmd, &p.data, lid_inverted)?);
                }
                Err(e @ Error::NotSupported { .. }) => last = e,
                Err(e) => return Err(e),
            }
        }
        Err(last)
    }

    /// Read the label roll's RFID tag (`None` = no tag).
    pub async fn consumable(&self) -> Result<Option<RfidTag>> {
        let _op = self.inner.op.lock().await;
        let p = self.inner.session.call(&Request::rfid_info()).await?;
        Ok(RfidTag::parse(&p.data)?)
    }

    /// Read live status: heartbeat, PrintStatus error, and (where the heartbeat doesn't report
    /// paper, e.g. D101) the RFID tag.
    pub async fn status(&self) -> Result<Status> {
        let _op = self.inner.op.lock().await;
        let s = &self.inner.session;
        let hb = self.heartbeat().await?;
        let error = optional(s.call(&Request::print_status()))
            .await?
            .and_then(|p| PrintStatus::parse(&p.data).ok()?.error);
        let (paper, consumable) = match hb.paper_present {
            Some(true) => (Paper::Present, None),
            Some(false) => (Paper::Absent, None),
            None => match optional(s.call(&Request::rfid_info())).await? {
                Some(p) => match RfidTag::parse(&p.data)? {
                    Some(tag) => (Paper::Present, Some(tag)),
                    None => (Paper::Absent, None),
                },
                None => (Paper::Unknown, None),
            },
        };
        Ok(Status {
            lid_closed: hb.lid_closed,
            paper,
            battery_percent: hb.battery_percent,
            temperature: hb.temperature,
            error,
            consumable,
        })
    }

    /// A blank label image of the given size, in the label's natural reading orientation
    /// (`width_mm` is the long side as you read it).
    pub fn label_canvas(&self, width_mm: f32, height_mm: f32) -> Bitmap {
        let dpmm = self.info().dots_per_mm();
        Bitmap::new(
            (width_mm * dpmm).round() as usize,
            (height_mm * dpmm).round() as usize,
        )
    }

    /// Convert a label image (as viewed) into a full-head-width raster (§7.1): rotate for
    /// `Left` models, then place across the head per the model's alignment, cropping if wider.
    pub fn head_raster(&self, label: &Bitmap) -> Result<Bitmap> {
        let info = self.info();
        let head = usize::from(info.head_dots);
        let model = info
            .model
            .ok_or_else(|| Error::Unsupported(format!("unknown model {}", info.model_id)))?;
        let oriented = match model.orientation {
            Orientation::Top => label.clone(),
            Orientation::Left => label.rotate_cw(),
        };
        let w = oriented.width();
        let offset: isize = match model.alignment.unwrap_or(Alignment::Center) {
            Alignment::Center => (head as isize - w as isize) / 2,
            Alignment::End => head as isize - w as isize,
        } + isize::from(model.x_offset);
        let mut raster = Bitmap::new(head, oriented.height());
        raster.blit(&oriented, offset, 0);
        Ok(raster)
    }

    /// Print a label image, as viewed (see [`Printer::head_raster`]).
    pub async fn print(&self, label: &Bitmap, opts: &PrintOptions) -> Result<()> {
        let raster = self.head_raster(label)?;
        self.print_raw(&raster, opts).await
    }

    /// Print a head-space raster: width = columns across the head (≤ head width), height = rows
    /// along the feed, row 0 first out.
    pub async fn print_raw(&self, raster: &Bitmap, opts: &PrintOptions) -> Result<()> {
        let info = self.info();
        let model = info
            .model
            .ok_or_else(|| Error::Unsupported(format!("unknown model {}", info.model_id)))?;
        match model.sequence_for(info.protocol_version) {
            Some(PrintSequence::B1) => {}
            other => {
                return Err(Error::Unsupported(format!(
                    "print sequence {other:?} for {} (only B1 is implemented)",
                    model.name
                )));
            }
        }
        if raster.width() > usize::from(info.head_dots) {
            return Err(Error::InvalidArgument(format!(
                "raster is {} columns, head is {} dots",
                raster.width(),
                info.head_dots
            )));
        }
        if raster.height() == 0 || raster.height() > usize::from(u16::MAX) {
            return Err(Error::InvalidArgument(format!(
                "raster height {} out of range",
                raster.height()
            )));
        }
        if opts.copies == 0 {
            return Err(Error::InvalidArgument("copies must be at least 1".into()));
        }
        let density = opts.density.unwrap_or(info.density_default);
        if !(info.density_range.0..=info.density_range.1).contains(&density) {
            return Err(Error::InvalidArgument(format!(
                "density {density} outside {}..={}",
                info.density_range.0, info.density_range.1
            )));
        }

        let _op = self.inner.op.lock().await;
        let result = self.print_b1(raster, density, opts).await;
        // Always end the print, even after an error (§10).
        let end = self.inner.session.call(&Request::print_end()).await;
        result?;
        end.map(drop)
    }

    async fn ack(&self, req: &Request) -> Result<()> {
        let p = self.inner.session.call(req).await?;
        if parse_ack(&p.data)? {
            Ok(())
        } else {
            Err(Error::Refused {
                cmd: req.packet.cmd,
            })
        }
    }

    async fn retry_ack(&self, req: &Request, attempts: u32) -> Result<()> {
        for i in 1..=attempts {
            match self.ack(req).await {
                Err(Error::Refused { .. }) if i < attempts => {
                    tokio::time::sleep(Duration::from_millis(500)).await
                }
                r => return r,
            }
        }
        unreachable!()
    }

    /// The `B1` sequence (§7.4, §8): verified on the B1 and D101.
    async fn print_b1(&self, raster: &Bitmap, density: u8, opts: &PrintOptions) -> Result<()> {
        let s = &self.inner.session;
        let progress = |p| {
            if let Some(cb) = &opts.on_progress {
                cb(p);
            }
        };
        let total = opts.copies;
        self.ack(&Request::set_density(density)).await?;
        self.ack(&Request::set_label_type(opts.label_type)).await?;
        self.ack(&Request::print_start_7(total, 0)).await?;
        self.retry_ack(&Request::page_start(), 5).await?;
        self.ack(&Request::set_page_size_6(
            raster.height() as u16,
            raster.width() as u16,
            total,
        ))
        .await?;

        let rows = encode_rows(raster, usize::from(self.info().head_dots));
        for (i, chunk) in rows.chunks(16).enumerate() {
            s.send_one_way(chunk).await?;
            progress(Progress::Sending {
                sent: (i * 16 + chunk.len()).min(rows.len()),
                total: rows.len(),
            });
        }
        self.retry_ack(&Request::page_end(), 20).await?;

        // Poll until all pages are done; give up after 20 s without any progress.
        let mut last = None;
        let mut last_change = Instant::now();
        loop {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let st = PrintStatus::parse(
                &s.call(&Request::print_status().with_timeout(Duration::from_secs(5)))
                    .await?
                    .data,
            )?;
            if let Some(e) = st.error {
                return Err(Error::Printer(e));
            }
            progress(Progress::Printing {
                page: st.page,
                total,
                print_percent: st.print_percent,
                feed_percent: st.feed_percent,
            });
            if st.page >= total {
                return Ok(());
            }
            let key = (st.page, st.print_percent, st.feed_percent);
            if last != Some(key) {
                last = Some(key);
                last_change = Instant::now();
            } else if last_change.elapsed() > Duration::from_secs(20) {
                return Err(Error::Timeout {
                    cmd: crate::proto::cmd::PRINT_STATUS,
                    timeout: Duration::from_secs(20),
                });
            }
        }
    }
}

async fn handshake(s: &Session, name: String) -> Result<PrinterInfo> {
    let connect = ConnectResult::parse(&s.call(&Request::connect()).await?.data)?;
    let mut status = StatusData::default();
    let protocol_version = match connect {
        ConnectResult::Disconnect | ConnectResult::FirmwareErrors => {
            return Err(Error::Handshake(format!("printer answered {connect:?}")));
        }
        ConnectResult::Connected => 0,
        ConnectResult::ConnectedNew => 1,
        ConnectResult::ConnectedV3 | ConnectResult::Other(_) => {
            if let Some(p) = optional(s.call(&Request::printer_status_data())).await? {
                status = StatusData::parse(&p.data);
            }
            status.protocol_version
        }
    };
    let model_id = parse_model_id(&s.call(&Request::printer_info(InfoKey::ModelId)).await?.data)?;
    let model = ModelInfo::by_id(model_id);
    if model.is_none() {
        log::warn!("unknown model ID {model_id}");
    }

    let info_data = |key| async move {
        optional(s.call(&Request::printer_info(key)))
            .await
            .map(|o| o.map(|p| p.data))
    };
    let serial = info_data(InfoKey::SerialNumber)
        .await?
        .and_then(|d| parse_serial(&d));
    let mac = info_data(InfoKey::BluetoothAddress)
        .await?
        .and_then(|d| parse_mac(&d).ok());
    let identity = optional(s.call(&Request::heartbeat(HeartbeatKind::Identity)))
        .await?
        .and_then(|p| Identity::parse(&p.data).ok());
    let (firmware, hardware) = match identity {
        Some(id) => (Some(id.firmware), Some(id.hardware)),
        None => (
            info_data(InfoKey::SoftwareVersion)
                .await?
                .and_then(|d| Version::parse(&d).ok()),
            info_data(InfoKey::HardwareVersion)
                .await?
                .and_then(|d| Version::parse(&d).ok()),
        ),
    };
    let head_dots = model
        .map(|m| m.head_dots)
        .or(identity.map(|i| i.head_dots))
        .unwrap_or(384);
    let dpi = model
        .map(|m| m.dpi)
        .or(identity.map(|i| i.dpi))
        .unwrap_or(203);
    let (density_range, density_default) = model.map_or(((1, 3), 2), |m| {
        ((m.density_min, m.density_max), m.density_default)
    });
    Ok(PrinterInfo {
        name,
        model_id,
        model,
        protocol_version,
        serial,
        mac,
        firmware,
        hardware,
        head_dots,
        dpi,
        density_range,
        density_default,
        supports_color: status.supports_color,
    })
}
