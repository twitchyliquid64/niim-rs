//! Bluetooth LE transport (btleplug), PROTOCOL.md §2.1.

use std::sync::Arc;
use std::time::Duration;

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Adapter, Manager, Peripheral};
use futures::StreamExt;
use tokio::sync::mpsc;

use super::{BoxFuture, Connection, Transport};
use crate::proto::ModelInfo;
use crate::{Error, Result};

fn terr(e: impl std::fmt::Display) -> Error {
    Error::Transport(e.to_string())
}

/// A NIIMBOT printer seen in a BLE scan.
#[derive(Debug, Clone)]
pub struct BleDevice {
    // Keeps btleplug's event loop alive: on macOS, dropping the adapter before connecting stops
    // notification delivery ("Event receiver died").
    adapter: Adapter,
    peripheral: Peripheral,
    /// Advertised name, `MODEL-SERIAL` (e.g. `D101-F310020193`).
    pub name: String,
    /// Model, from the name prefix.
    pub model: &'static ModelInfo,
    /// Received signal strength, if known.
    pub rssi: Option<i16>,
}

impl BleDevice {
    /// Serial number, from the name suffix.
    pub fn serial(&self) -> Option<&str> {
        self.name.split_once('-').map(|(_, s)| s)
    }

    /// Connect and subscribe to the data characteristic.
    pub async fn open(&self) -> Result<Connection> {
        let p = &self.peripheral;
        let mut last = None;
        for attempt in 1..=3 {
            match p.connect().await {
                Ok(()) => {
                    last = None;
                    break;
                }
                Err(e) => {
                    log::warn!("BLE connect attempt {attempt} failed: {e}");
                    last = Some(e);
                    tokio::time::sleep(Duration::from_millis(500)).await;
                }
            }
        }
        if let Some(e) = last {
            return Err(terr(e));
        }
        p.discover_services().await.map_err(terr)?;
        // §2.1: the first characteristic with both notify and write-without-response. This skips the
        // Microchip transparent-UART service that the B1 and D101 also expose.
        let ch = p
            .characteristics()
            .into_iter()
            .find(|c| {
                c.properties.contains(CharPropFlags::NOTIFY)
                    && c.properties.contains(CharPropFlags::WRITE_WITHOUT_RESPONSE)
            })
            .ok_or_else(|| {
                Error::Transport("no notify + write-without-response characteristic".into())
            })?;
        log::debug!("{}: using characteristic {}", self.name, ch.uuid);
        p.subscribe(&ch).await.map_err(terr)?;

        let mut notifications = p.notifications().await.map_err(terr)?;
        let (tx, incoming) = mpsc::unbounded_channel();
        let uuid = ch.uuid;
        tokio::spawn(async move {
            while let Some(n) = notifications.next().await {
                if n.uuid == uuid && tx.send(n.value).is_err() {
                    break;
                }
            }
        });
        let transport = BleTransport {
            _adapter: self.adapter.clone(),
            peripheral: p.clone(),
            ch,
            name: self.name.clone(),
        };
        Ok(Connection {
            transport: Arc::new(transport),
            incoming,
        })
    }
}

struct BleTransport {
    _adapter: Adapter,
    peripheral: Peripheral,
    ch: Characteristic,
    name: String,
}

impl Transport for BleTransport {
    fn write<'a>(&'a self, frame: &'a [u8]) -> BoxFuture<'a, Result<()>> {
        Box::pin(async move {
            self.peripheral
                .write(&self.ch, frame, WriteType::WithoutResponse)
                .await
                .map_err(terr)
        })
    }

    fn close(&self) -> BoxFuture<'_, Result<()>> {
        Box::pin(async move {
            if self.peripheral.is_connected().await.unwrap_or(false) {
                self.peripheral.disconnect().await.map_err(terr)?;
            }
            Ok(())
        })
    }

    fn describe(&self) -> String {
        format!("BLE {}", self.name)
    }
}

async fn adapter() -> Result<Adapter> {
    let manager = Manager::new().await.map_err(terr)?;
    manager
        .adapters()
        .await
        .map_err(terr)?
        .into_iter()
        .next()
        .ok_or_else(|| Error::Transport("no Bluetooth adapter".into()))
}

/// Scan for `duration` and return every device whose name is `MODEL-…` for a known model.
///
/// The scan is unfiltered on purpose: the D101 doesn't advertise the NIIMBOT service UUID.
pub async fn scan(duration: Duration) -> Result<Vec<BleDevice>> {
    let central = adapter().await?;
    central
        .start_scan(ScanFilter::default())
        .await
        .map_err(terr)?;
    tokio::time::sleep(duration).await;
    central.stop_scan().await.map_err(terr)?;
    let mut found = Vec::new();
    for peripheral in central.peripherals().await.map_err(terr)? {
        let Some(props) = peripheral.properties().await.map_err(terr)? else {
            continue;
        };
        let Some(name) = props.local_name else {
            continue;
        };
        if let Some(model) = ModelInfo::from_ble_name(&name) {
            found.push(BleDevice {
                adapter: central.clone(),
                peripheral,
                name,
                model,
                rssi: props.rssi,
            });
        }
    }
    found.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(found)
}
