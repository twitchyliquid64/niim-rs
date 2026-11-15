//! Layer 1: the NIIMBOT wire protocol, with no I/O.
//!
//! Everything here is synchronous and deterministic: framing ([`Packet`], [`Reassembler`]),
//! request builders ([`Request`]), response parsers, raster encoding ([`Bitmap`],
//! [`encode_rows`]) and the model table ([`ModelInfo`]). See `PROTOCOL.md` for the specification.

mod model;
mod models_table;
mod packet;
mod raster;
mod request;
mod response;

pub use model::{Alignment, ModelInfo, Orientation, PrintSequence};
pub use packet::{CONNECT_PREFIX, HEAD, Packet, Reassembler, TAIL};
pub use raster::{Bitmap, MAX_COLUMNS, encode_rows, pixel_counts};
pub use request::{HeartbeatKind, InfoKey, Request, cmd};
pub use response::{
    ConnectResult, Heartbeat, Identity, ParseError, PrintStatus, PrinterError, RfidTag, StatusData,
    Version, battery_percent, parse_ack, parse_mac, parse_model_id, parse_serial,
};
