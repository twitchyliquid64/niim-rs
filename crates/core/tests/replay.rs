//! End-to-end tests of layers 2–4 against packet captures from real hardware (`captures/`).

use niim_core::proto::Bitmap;
use niim_core::transport::replay::ReplayTransport;
use niim_core::{Paper, PrintOptions, Printer};

const D101_INIT: &str = include_str!("../../../captures/d101-ble-init.txt");
const D101_PRINT: &str = include_str!("../../../captures/d101-print3.txt");
const B1_INIT: &str = include_str!("../../../captures/b1-ble-init.txt");

async fn printer(dump: &str, name: &str) -> (std::sync::Arc<ReplayTransport>, Printer) {
    let (t, conn) = ReplayTransport::open(dump);
    (t, Printer::connect(conn, name).await.expect("handshake"))
}

#[tokio::test(start_paused = true)]
async fn d101_handshake_and_status() {
    let (_, p) = printer(D101_INIT, "D101-F310020193").await;
    let i = p.info();
    assert_eq!(i.model_name(), "D101");
    assert_eq!(i.protocol_version, 0);
    assert_eq!(i.serial.as_deref(), Some("F310020193"));
    assert_eq!(i.mac, Some([0x01, 0x02, 0x93, 0x0F, 0x94, 0x70]));
    assert_eq!((i.head_dots, i.dpi, i.density_range), (192, 203, (1, 3)));
    // No heartbeat-3 on the D101: falls back to PrinterInfo versions.
    assert_eq!(i.firmware.map(|v| v.to_string()).as_deref(), Some("7.02"));

    // Heartbeat 4 is NotSupported → falls back to type 1; paper comes from the RFID tag.
    let s = p.status().await.unwrap();
    assert_eq!(s.lid_closed, Some(true));
    assert_eq!(s.battery_percent, Some(75));
    assert_eq!(s.paper, Paper::Present);
    assert_eq!(s.consumable.unwrap().remaining(), 142);
    assert_eq!(s.error, None);
}

#[tokio::test(start_paused = true)]
async fn b1_handshake_and_status_without_paper() {
    let (_, p) = printer(B1_INIT, "B1-I812130338").await;
    let i = p.info();
    assert_eq!(
        (i.model_name().as_str(), i.protocol_version, i.head_dots),
        ("B1", 3, 384)
    );
    assert_eq!(i.serial.as_deref(), Some("I812130338"));
    assert_eq!(i.firmware.map(|v| v.to_string()).as_deref(), Some("6.19"));
    assert_eq!(i.hardware.map(|v| v.to_string()).as_deref(), Some("6.01"));
    assert!(i.supports_color);

    let s = p.status().await.unwrap();
    assert_eq!(
        (s.lid_closed, s.paper, s.battery_percent),
        (Some(true), Paper::Absent, Some(100))
    );
    assert_eq!(s.error, Some(niim_core::proto::PrinterError::NoPaper));
}

#[tokio::test(start_paused = true)]
async fn d101_print_raw_follows_b1_sequence() {
    let (t, p) = printer(&format!("{D101_INIT}\n{D101_PRINT}"), "D101-F310020193").await;
    let raster = Bitmap::from_fn(192, 320, |x, y| x % 8 == 0 && y % 40 == 0);
    p.print_raw(
        &raster,
        &PrintOptions {
            density: Some(2),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let cmds: Vec<u8> = t
        .sent()
        .iter()
        .map(|f| f[if f[0] == 0x03 { 3 } else { 2 }])
        .filter(|&c| !matches!(c, 0x84 | 0x85))
        .collect();
    let print_part = &cmds[cmds.iter().position(|&c| c == 0x21).unwrap()..];
    // SetDensity, SetLabelType, PrintStart, PageStart, SetPageSize, PageEnd, PrintStatus…, PrintEnd
    assert_eq!(&print_part[..6], [0x21, 0x23, 0x01, 0x03, 0x13, 0xE3]);
    assert_eq!(print_part.last(), Some(&0xF3));
    assert!(print_part.contains(&0xA3));
}

#[tokio::test(start_paused = true)]
async fn label_geometry() {
    let (_, d101) = printer(D101_INIT, "D101").await;
    // 40 x 12 mm landscape label → rotated, 96 columns placed at the far end of the 192-dot head.
    let mut label = d101.label_canvas(40.0, 12.0);
    assert_eq!((label.width(), label.height()), (320, 96));
    label.set(0, 0, true); // top-left as read
    let r = d101.head_raster(&label).unwrap();
    assert_eq!((r.width(), r.height()), (192, 320));
    // Rotated CW: source top-left lands at raster column (96 - 1) + offset 96, row 0.
    assert!(r.get(191, 0));

    let (_, b1) = printer(B1_INIT, "B1").await;
    let mut label = b1.label_canvas(50.0, 30.0);
    assert_eq!((label.width(), label.height()), (400, 240));
    label.set(8, 0, true);
    let r = b1.head_raster(&label).unwrap();
    // 400-wide label on the 384-dot head, calibrated (x_offset +8): head dot 0 is the label's
    // left edge, so nothing is cropped on the left and 16 dots are cropped on the right.
    assert_eq!((r.width(), r.height()), (384, 240));
    assert!(r.get(8, 0));
}
