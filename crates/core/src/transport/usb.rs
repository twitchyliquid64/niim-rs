//! USB transport: the printer's CDC-ACM data interface driven directly with `nusb` (§2.2).
//!
//! **Experimental:** not yet tested on hardware. On macOS the system CDC driver may hold the
//! interface, in which case claiming it fails.

use std::sync::Arc;
use std::time::Duration;

use nusb::descriptors::TransferType;
use nusb::transfer::{Bulk, ControlOut, ControlType, Direction, In, Out, Recipient};
use nusb::{DeviceInfo, io::EndpointWrite};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::{Mutex, mpsc};

use super::{BoxFuture, Connection, Transport};
use crate::{Error, Result};

/// NIIMBOT's USB vendor ID.
pub const NIIMBOT_VID: u16 = 0x3513;

const CDC_COMM_CLASS: u8 = 0x02;
const CDC_DATA_CLASS: u8 = 0x0A;
const SET_LINE_CODING: u8 = 0x20;
const SET_CONTROL_LINE_STATE: u8 = 0x22;

fn terr(e: impl std::fmt::Display) -> Error {
    Error::Transport(e.to_string())
}

/// A NIIMBOT printer on USB.
#[derive(Debug, Clone)]
pub struct UsbDevice {
    info: DeviceInfo,
    /// Product string, or `VID:PID`.
    pub name: String,
}

/// List NIIMBOT USB devices (vendor `0x3513`).
pub async fn scan() -> Result<Vec<UsbDevice>> {
    let devices = nusb::list_devices().await.map_err(terr)?;
    Ok(devices
        .filter(|d| d.vendor_id() == NIIMBOT_VID)
        .map(|info| {
            let name = info
                .product_string()
                .map(str::to_owned)
                .unwrap_or_else(|| format!("{:04x}:{:04x}", info.vendor_id(), info.product_id()));
            UsbDevice { info, name }
        })
        .collect())
}

impl UsbDevice {
    /// Open the CDC data interface's bulk endpoints.
    pub async fn open(&self) -> Result<Connection> {
        let device = self.info.open().await.map_err(terr)?;
        let config = device.active_configuration().map_err(terr)?;

        let mut comm_iface = None;
        let mut data = None;
        for alt in config.interface_alt_settings() {
            match alt.class() {
                CDC_COMM_CLASS => comm_iface = comm_iface.or(Some(alt.interface_number())),
                CDC_DATA_CLASS if data.is_none() => {
                    let bulk = |dir: Direction| {
                        alt.endpoints()
                            .find(|e| {
                                e.transfer_type() == TransferType::Bulk && e.direction() == dir
                            })
                            .map(|e| e.address())
                    };
                    if let (Some(i), Some(o)) = (bulk(Direction::In), bulk(Direction::Out)) {
                        data = Some((alt.interface_number(), i, o));
                    }
                }
                _ => {}
            }
        }
        let (data_iface, ep_in, ep_out) = data
            .ok_or_else(|| Error::Transport("no CDC data interface with bulk endpoints".into()))?;

        // Configure the virtual serial line like an OS driver would; harmless if ignored (§2.2).
        if let Some(comm) = comm_iface {
            let line_coding = [0x00, 0xC2, 0x01, 0x00, 0, 0, 8]; // 115200 8N1, little-endian baud
            for (request, value, data) in [
                (SET_LINE_CODING, 0, &line_coding[..]),
                (SET_CONTROL_LINE_STATE, 0x0003, &[][..]),
            ] {
                let req = ControlOut {
                    control_type: ControlType::Class,
                    recipient: Recipient::Interface,
                    request,
                    value,
                    index: u16::from(comm),
                    data,
                };
                if let Err(e) = device.control_out(req, Duration::from_millis(500)).await {
                    log::debug!("USB CDC request 0x{request:02X} failed (ignored): {e}");
                }
            }
        }

        let iface = device
            .detach_and_claim_interface(data_iface)
            .await
            .map_err(terr)?;
        let mut reader = iface.endpoint::<Bulk, In>(ep_in).map_err(terr)?.reader(512);
        let writer = iface
            .endpoint::<Bulk, Out>(ep_out)
            .map_err(terr)?
            .writer(512);

        let (tx, incoming) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let mut buf = [0u8; 512];
            loop {
                match reader.read(&mut buf).await {
                    Ok(0) => continue,
                    Ok(n) => {
                        if tx.send(buf[..n].to_vec()).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        log::debug!("USB read ended: {e}");
                        break;
                    }
                }
            }
        });
        let transport = UsbTransport {
            writer: Mutex::new(Some(writer)),
            name: self.name.clone(),
        };
        Ok(Connection {
            transport: Arc::new(transport),
            incoming,
        })
    }
}

struct UsbTransport {
    writer: Mutex<Option<EndpointWrite<Bulk>>>,
    name: String,
}

impl Transport for UsbTransport {
    fn write<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            let mut guard = self.writer.lock().await;
            let w = guard.as_mut().ok_or(Error::Disconnected)?;
            w.write_all(frame).await.map_err(terr)?;
            w.flush().await.map_err(terr)
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            self.writer.lock().await.take();
            Ok(())
        })
    }

    fn describe(&self) -> String {
        format!("USB {}", self.name)
    }
}
