# NIIMBOT printer protocol

This document specifies the host ↔ printer protocol used by NIIMBOT thermal label printers
(D11, D110, B1, B21, B18, H1S, …), in enough detail to write a driver that discovers printers,
reads their status and identity, and prints monochrome rasters.

NIIMBOT does not publish this protocol. Everything here was a clean-room reconstruction from open-source
implementations (see [Sources](#sources)). Statements are marked by how well they are corroborated:

- No marker: implemented consistently by several sources, or confirmed by captured traffic.
- **(1 src)**: only one source does this. It is probably right, but nobody else checks it.
- **(unverified)**: inferred or from general knowledge, not present in any of the sources. Verify on hardware.

All multi-byte integers are **big-endian** unless stated otherwise. Hex bytes are written `55 55 …`.

---

## Target printers for niim-rs: B1 and D101

| | **B1** | **D101** |
|---|---|---|
| Model ID (`PrinterInfo(8)`) | 4096 (`0x1000`) | 2560 (`0x0A00`) |
| Head / DPI | 384 dots, 203 dpi (48 mm) | 192 dots, 203 dpi (24 mm) |
| Orientation (§7.1) | `top`: label width across head | `left`: landscape labels rotated 90° CW |
| Density | 1–5, default 3 | 1–3, default 2 |
| Print sequence (§8) | `B1` | `B1`, **confirmed by test prints** |
| Lid byte in `DD` heartbeat | normal | **inverted** (model is in the inverted list) |
| Evidence | **Strong.** A full init capture (fw 5.22, protocol v3), niimctl prints on it over USB, and niimbluelib's B1 task and tests. | **Strong (BLE).** Identity, status, heartbeat, lid/paper behaviour and printing are all **measured** on our unit. Captures are in `captures/`. |

**B1: measured on hardware (2026-11-14, BLE, unit `B1-I812130338`, no roll loaded; `captures/b1-ble-init.txt`):**

| Item | Result |
|---|---|
| BLE name / adv | `B1-I812130338` (ASCII serial). macOS showed **no manufacturer data** for it, unlike the D101. |
| GATT | Same two services as the D101. `bef8d6c9-…` additionally has `write` (with response). |
| Connect / PrinterStatusData | `3` / 16 bytes `30 30 03 20 00 C8 00 00 00 0F 01 02 04 01 98 00`. n = 204, so **protocol 3** and supportColor. Byte-identical to niimbluelib's fw 5.22 capture. |
| Model / serial / MAC | `10 00` = 4096 / ASCII `I812130338` / `12 08 13 7A 44 14`, i.e. `14:44:7A:13:08:12` |
| Battery / auto-shutdown / label type / **density** | `04` (100%) / `03` / `01` / **`05`**. The density is whatever was last set; it's stored on the printer. |
| SW / HW (`49` / `4C`) | `06 13` / `06 01`. Heartbeat 3 reports the same bytes, so the version is **6.19 / 6.01** (`a.b` form). |
| Heartbeat 3 → `DE` | `06 01 06 13 01 80 02 02 01 00`: hw 6.01, fw 6.19, **384-dot head**, 203 dpi, alignment 2, RFID yes, RFID write no |
| Heartbeat 1 → `DD` (13 B) | `20 72 00 86 00 86 00 00 47 00 04 01 00`: lid [9] = 0 (closed), battery [10] = 4, **paper [11] = 1 (not inserted)**, RFID [12] = 0. [2..4] = [4..6] = `0x0086`, the same sensor-like u16 as on the D101. |
| Heartbeat 4 → `D9` | `20 72 04 47 00 01 00 00 00`: battery 4, temperature 0x47, lid 0, **paper 1 (none)**, both RFIDs 0, ribbon 0 |
| Heartbeat 2 → `DF` | 12 bytes `00 00 00 05 00 …` (D101: `… 39 …`), unknown |
| SoundSettings | **Supported** (unlike the D101): connection sound `02 01 01` = on, power sound `02 02 00` = off |
| RfidInfo / PaperInfo / Capabilities | `00` (no roll) / NotSupported / NotSupported |
| PrintStatus (idle) | **10 bytes** `00 00 00 00 03 1F 02 00 00 00`. Error byte [6] = `02` = **no paper**. |

Across both models, bytes [4..6] of PrintStatus are one less than bytes [2..4] of PrinterStatusData (B1:
`031F` vs `0320`; D101: `06A3` vs `06A4`). Their meaning is unknown.

**B1 test print** (stock 50 × 30 mm roll, RFID barcode `10262260`, 276 labels; `captures/b1-print1.txt`):

- **The `B1` sequence works as documented.** Responses were identical to the D101's: SetPageSize returned
  `01 00`, and there was an unsolicited `D3` at the last row (199).
- **PrintStatus is 10 bytes:** `page, print%, feed%` progress (e.g. `00 00 64 43` = page 0, 100% printed, 67% fed),
  error at [6], and a busy flag at [7] while printing.
- **Orientation:** with the leading edge (first out) at the top, **row 0 is the top and column 0 is the left**. The
  B1 prints the image as viewed, with no rotation.
- **A narrow raster is centred** on the head. A 192-column (24 mm) raster on the 50 mm label left 12 mm on the
  column-0 side and 15 mm on the other, so it is centred to within ~1.5 mm (head/label tolerance). The B1's
  heartbeat-3 "printhead alignment" is `2`, which plausibly means "centre" **(unverified)**.
- **Recommended for B1 50 × 30 labels:** send `cols = 384` (48 mm) centred on the 50 mm label, or a narrower
  raster, which is also centred. Use `rows = 240`.

**D101: measured on hardware (2026-11-14, BLE, unit `D101-F310020193`):**

| Item | Result |
|---|---|
| BLE name | `D101-F310020193`. The suffix is the serial number, in hex. |
| Advertisement | **No service UUIDs advertised.** Manufacturer data has company ID `0x0000` and bytes `01 02 93 0F 94 70` (repeated twice), which is the MAC address in forward order. |
| GATT | Data channel is service `e7810a71-…`, characteristic `bef8d6c9-…` (`read`, `write-without-response`, `notify`). An unrelated Microchip "transparent UART" service `49535343-fe7d-…` is also present; don't use it. |
| Connect | Result `3` (ConnectedV3) |
| PrinterStatusData | **10 bytes**: `30 30 06 A4 00 C8 00 00 00 0A`. That is shorter than 13, so §5.2 gives **protocol 0**. |
| Model ID | `0A 00`, i.e. 2560 |
| Serial (`4B`) | 5 bytes, `F3 10 02 01 93`. This is `F310020193` in hex, matching the BLE name. niimbluelib's "first 4 bytes" rule would wrongly truncate it. |
| BT address (`4D`) | `70 94 0F 93 02 01`, reversed to `01:02:93:0F:94:70` |
| Battery (`4A`) | `03`, i.e. 75% |
| Density (`41`) / label type (`43`) / auto shutdown (`47`) | `02` / `01` / `01` |
| SW / HW version (`49` / `4C`) | `07 02` / `07 01`, either 7.02/7.01 or 17.94/17.93. Still ambiguous. |
| Area (`4F`) | `00 14` (meaning unknown) |
| NotSupported | PrinterInfo 2, 6 and 14; Heartbeat 3 and 4; SoundSettings; GetPaperInfo; GetPrinterCapabilities |
| Heartbeat 1 → `DD` | 10 bytes, e.g. `1E 9A 00 70 00 70 32 46 01 03`. Same layout as the D110. See the state experiments below. |
| Heartbeat 2 → `DF` | 12 bytes `00 00 00 39 00 00 00 00 00 00 00 00` (unknown) |
| RfidInfo → `1B` | Tag present. Barcode `01222281`, serial `PZ0F128003010675`, 192 total, 50 used, type 1. |
| PrintStatus (idle) → `B3` | 8 bytes `00 00 00 00 06 A3 EE 00`. Page 0. Byte [6] is the **error code** (`01` = cover open while the lid is open). Bytes [4..6] = 1699, close to the `06 A4` in PrinterStatusData; possibly a counter. |

**D101 state experiments** (capture in `captures/d101-ble-init.txt`):

| State | `DD` [2..4] = [4..6] (u16) | `DD`[8] | `B3`[6] | RfidInfo |
|---|---|---|---|---|
| Lid closed, roll in | `0x0070` (112) | `01` | `00` | tag present |
| Lid open, roll in | `0x0052` (82) | `00` | `01` | — |
| Lid closed, no roll | `0x0091` (145) | `01` | `00` | `00` (no tag) |

- **Lid:** `DD`[8] is `1` = closed and `0` = open. This is the inverted polarity niimbluelib predicts for model 2560.
- **Paper:** the 10-byte `DD` has **no paper-present flag**. `DD`[2..4] and [4..6] hold the same u16, which tracks
  paper and lid state, probably a raw label/gap-sensor reading. Detect paper with `RfidInfo`, which returns 1 byte
  `00` when no roll is loaded, or by thresholding that value **(unverified)**.
- **Other bytes:** `DD`[0] (`1E`), [6..8] (`32 46`) and [9] (battery) didn't change across states. `DD`[1] drifts
  upward between readings (`9A C6 D3 D4`), like a counter or temperature. `DF` was identical in every state.

**D101 test prints** (BLE, density 2, `T12-40-160` roll = 12 × 40 mm die-cut labels; logs in
`captures/d101-print{1,2,3}.txt`):

- **The `B1` sequence works** (§7.4): SetDensity, SetLabelType 1, 7-byte PrintStart, PageStart, 6-byte
  SetPageSize, rows (`0x85`/`0x84`, niimbluelib split counts with head = 192), PageEnd, PrintStatus polling,
  PrintEnd. Exactly one label feeds per page, with no calibration or extra feed.
- **Responses that differ from the B1:**
  - SetPageSize returns **2 bytes** (`01 00`).
  - The printer sends **unsolicited `D3` = `[u16 row, 01]` at row 199 and at the last row (319)** while rows
    stream in.
  - PrintStatus during printing is `00 00 55 00 xx xx 00 01` (page 0, 85%, byte [7] = 1), then
    `00 01 64 64 xx xx 00 00` when done.
- **Orientation:** raster rows run along the 40 mm length; row 0 is the end of the label that leaves the printer
  first. Columns run across the 12 mm height.
- **Across the head:** the 12 mm label sits on **head dots ≈ 100–191** (the end of the 192-dot head, not the
  centre). A raster narrower than the head is **aligned to its last column**: a 96-column raster lands on head
  dots 96–191, so its columns 0–5 fall just off the label edge.
  - Full-head raster (192 columns): visible columns ≈ 100–191.
  - 96-column raster: visible columns ≈ 6–95.
- **Along the feed:** with `rows = 320` (40 mm), printing starts about **7 rows (0.9 mm) before** the label's
  leading edge, so rows 0–6 are lost. Rows ≈ 7–316 land on the label, and the last couple of rows weren't seen
  either. The *first* label after re-inserting the roll was offset further, losing rows ≥ ~290; it looked like
  the gap sensor hadn't synced yet.
- **Recommended for 12 mm D101 labels:** send `cols = 96` and `rows = length_mm × 8`, and keep content at least
  **1 mm (8 rows) from both ends** and **6 dots from column 0**. Discard (or blank-print) the first label after
  loading a roll.

**D101 still open:**

- **Version encoding.**
- **Wider rolls:** does a 24 mm-high label occupy the whole 192-dot head with the same far-end reference?
- **USB data interface:** whether it exists.

---

## Contents

1. [Architecture overview](#1-architecture-overview)
2. [Transports and discovery](#2-transports-and-discovery)
3. [Packet framing](#3-packet-framing)
4. [Request/response model](#4-requestresponse-model)
5. [Session setup and protocol version](#5-session-setup-and-protocol-version)
6. [Identity, settings and status](#6-identity-settings-and-status)
7. [Printing rasters](#7-printing-rasters)
8. [Model-specific print sequences](#8-model-specific-print-sequences)
9. [Extensions: two-colour and grayscale](#9-extensions-two-colour-and-grayscale)
10. [Implementation guidance](#10-implementation-guidance)
11. [Reference tables](#11-reference-tables)
12. [Discrepancies and open questions](#12-discrepancies-and-open-questions)
13. [Sources](#sources)

---

## 1. Architecture overview

All models use the same three layers:

```
┌──────────────────────────────────────────────────────────────┐
│ Jobs:     connect → query info/status → print job → end       │  model-specific sequencing (§8)
├──────────────────────────────────────────────────────────────┤
│ Commands: 1-byte command ID + payload, request → response     │  shared command set (§4–§7)
├──────────────────────────────────────────────────────────────┤
│ Framing:  55 55 CMD LEN DATA… XOR AA AA                       │  identical on every model (§3)
├──────────────────────────────────────────────────────────────┤
│ Byte transport: BLE GATT char │ USB CDC-ACM │ BT Classic SPP  │  (§2)
└──────────────────────────────────────────────────────────────┘
```

- **Framing** is identical across all models and transports.
- **Commands** come from one shared command set. Individual models implement a subset, and several commands
  have payload variants that differ in length (e.g. `SetPageSize` takes 2, 4, 6, 9 or 13 bytes).
- **Print job sequencing** varies by model and firmware generation. The building blocks are the same everywhere:
  set density and label type, start the print, start a page, set the page size, send rows, end the page, wait,
  end the print. They differ in which payload variant is used, the order, and how completion is detected. §7
  describes the generic job and §8 lists the per-model variants.
- The **raster model** is the same everywhere: 1 bit per pixel, rows sent in feed order, each row spanning the
  printhead. The printhead width (in dots) is per-model.

---

## 2. Transports and discovery

The transports carry a bidirectional byte stream, and the framing is the same on all of them. Packets are
self-delimiting, so a transport's chunk boundaries are irrelevant and the receiver must re-assemble (§3.4).

### 2.1 Bluetooth Low Energy (primary, most models)

**Discovery**

- Printers advertise with a local name that starts with the model name, e.g. `B1-…`, `D110-…`, `B21-…`.
  niimbluelib filters on the first letter of every known model name. NiimPrintX filters on a model-name prefix,
  case-insensitive. The name is `MODEL-SERIAL`. Confirmed on a D101: `D101-F310020193`, where the serial
  matches `PrinterInfo(11)`.
- Data service UUID: `e7810a71-73ae-499d-8c15-faa9aef0c3f2` (niimbluelib's filter, confirmed present on the
  D101). **The D101 does not include it in its advertisement**, so an OS-level scan filtered on this service UUID
  will not find it. Scan unfiltered and match on the name, then check the service after connecting.
- D101 manufacturer data (company ID `0x0000`) carries the 6-byte Bluetooth MAC address, so a printer can be
  identified before connecting. This matters on macOS, which hides MAC addresses from apps.
- Some dual-mode printers also show up as a Bluetooth Classic device under the same name. NiimPrintX picks the
  scan result whose advertisement carries **no** service UUIDs, to avoid the classic-mode twin **(1 src)**. With
  a service-UUID scan filter this filtering is unnecessary.

**Finding the data characteristic**

Don't hard-code the characteristic UUID. Both BLE implementations find it by its properties:

- niimbluelib: iterate primary services with a full 128-bit UUID and take the **first characteristic with
  `notify` and `write-without-response`**.
- NiimPrintX: take the service that has **exactly one characteristic**, whose properties include `read`,
  `write-without-response` and `notify`.

The characteristic is `bef8d6c9-9c21-4c9e-b632-bd58c1009f9f` (`read`, `write-without-response`, `notify`),
confirmed on the D101. The D101 also exposes a Microchip transparent-UART service
(`49535343-fe7d-4ae5-8fa9-9fafd205e455`). None of its characteristics has both `notify` and
`write-without-response`, so the property-based selection above skips it correctly. Don't select by
"any notify characteristic".

**Data flow**

- **Host → printer:** write one complete framed packet per GATT *write-without-response*.
- **Printer → host:** subscribe to notifications on the same characteristic. One notification may contain a
  partial packet, a whole packet, or several concatenated packets. Use a re-assembly buffer (§3.4).
- **Packet size:** the largest packets are bitmap rows: `7 + 6 + ceil(cols/8)` bytes. That's 61 bytes for a
  384-dot head and up to 167 bytes for a 1229-dot head. A write must fit in `ATT_MTU − 3`, so negotiate a large
  MTU. Neither source fragments writes **(unverified: behaviour with a 23-byte MTU is unknown)**.
- **Pacing:** niimbluelib waits **10 ms before every write** (`packetIntervalMs`, see niimblue issue #5), and
  NiimPrintX sleeps 10 ms between bitmap rows. Without pacing, printers drop packets.
- **Connect retries:** GATT connects sometimes drop immediately after succeeding. niimbluelib retries 3 times,
  500 ms apart.

### 2.2 USB (CDC-ACM virtual serial port)

- The printer enumerates as a USB CDC-ACM device (`/dev/ttyACM*` on Linux).
- **Vendor ID `0x3513`** **(1 src: niimbot-rs auto-detects by this VID)**. No source records product IDs.
- All serial implementations open it at **115200 8N1**. For a USB CDC device the baud rate is nominal.
- One source says the port only appears or responds when the printer is **powered off and charging** **(1 src:
  niimbot-rs; niimctl and niimprint don't mention it)**. Treat this as an open question.
- **Driving it with `nusb`** **(unverified, standard CDC-ACM class behaviour)**: claim the CDC Data interface
  (class `0x0A`) and use its bulk IN/OUT endpoint pair as the byte stream. For parity with OS serial drivers,
  consider sending the CDC class requests `SET_LINE_CODING` (115200, 8N1) and `SET_CONTROL_LINE_STATE`
  (DTR|RTS) on the Communications interface (class `0x02`) before use. It is unknown whether the firmware
  requires them.
- The framing, pacing and re-assembly rules are the same as for BLE.

### 2.3 Bluetooth Classic (SPP/RFCOMM): legacy, out of scope here

niimprint connects to older printers (B21, D11) over RFCOMM **channel 1** and gets the same byte stream and
framing. `btleplug` can't do RFCOMM. It is documented here only so the framing description is complete.

---

## 3. Packet framing

### 3.1 Standard packet

```
offset  size  field
0       2     HEAD      = 55 55
2       1     CMD       command ID (§11.1)
3       1     LEN       number of DATA bytes, 0..255
4       LEN   DATA
4+LEN   1     CHECKSUM  = CMD ^ LEN ^ DATA[0] ^ … ^ DATA[LEN-1]
5+LEN   2     TAIL      = AA AA
```

Total length is `LEN + 7`. The checksum is a plain XOR over CMD, LEN and DATA, excluding HEAD and TAIL.

Example: `PrinterInfo(PrinterModelId)` is `55 55 40 01 08 49 AA AA` (`0x40 ^ 0x01 ^ 0x08 = 0x49`).

### 3.2 The Connect prefix

The **Connect** request (CMD `0xC1`) is sent with an extra leading byte **`0x03`** before the head:

```
03 55 55 C1 01 01 C1 AA AA
```

Every source that sends Connect does this, and it appears in every captured trace. No other packet has a prefix.

### 3.3 Packet variants (extensions)

These are only needed for features beyond monochrome printing:

| Variant | Used by | Layout |
|---|---|---|
| **16-bit length** | `PrintBitmapRowGray` `0x8D` (TX only) | `55 55 CMD LEN_HI LEN_LO DATA… CHK AA AA`. CHK = `CMD ^ LEN_HI ^ LEN_LO ^ DATA…` |
| **CRC32** | Firmware upgrade only: TX `0x91 0x92 0x9B 0x9C`, RX `0x90 0x9A 0x9D 0x9E` | `55 55 CMD CHUNK_HI CHUNK_LO LEN DATA… CRC32(4, BE) AA AA`. CRC32 (zlib) over `CMD, CHUNK_HI, CHUNK_LO, LEN, DATA…` |

A parser picks the variant from the CMD byte at offset 2 (niimbluelib `parsePacketBundle`). A monochrome driver
never receives either variant unless it starts a firmware upgrade.

### 3.4 Receive-side re-assembly

Transports deliver arbitrary chunks, so keep a byte buffer and repeat:

1. If the buffer doesn't start with `55 55`, resynchronise by discarding bytes up to the next `55 55`.
   niimbluelib discards the whole buffer instead, which also works because the printer only responds to requests.
2. If fewer than 4 bytes are buffered, wait for more.
3. Read `LEN = buf[3]` and `total = LEN + 7`. If `buf.len() < total`, wait for more.
4. Check that `buf[total-2..total] == AA AA` and that the checksum matches. On failure, drop 1 byte and resync.
5. Emit the packet, remove `total` bytes, and loop, since one chunk may hold several packets.

---

## 4. Request/response model

### 4.1 Basic exchange

- The host sends one request and waits for its response. Keep **at most one request in flight**: niimbluelib
  holds a mutex across send-and-wait.
- The response has a **different command ID** from the request. The mapping is not a fixed offset, so use the
  table in §11.1. Some requests accept several response IDs (`Heartbeat`, `PrinterInfo`).
- Most "action" requests carry a single dummy payload byte `01`. niimbluelib's default payload is `[1]`.
- Many responses are a single **boolean byte**: `01` means success or accepted, `00` means refused or
  not ready yet.
- Default response timeout: **1000 ms**. Use longer timeouts for RFID reads (5500 ms; B1 Pro is slow),
  status polling during a print (5000 ms), and packets sent during a page (10000 ms).

### 4.2 One-way requests

Bitmap row packets (`0x83`, `0x84`, `0x85`, `0x8A`, `0x8D`) get **no response**. Send them back to back,
with the per-packet pacing delay.

### 4.3 Error responses

Any request can be answered by one of these instead of its normal response:

| CMD | Name | DATA | Meaning |
|---|---|---|---|
| `0x00` | NotSupported | `01` | The command, or this payload variant, is not supported by this model/firmware. |
| `0xDB` | PrintError | `[code]` | Print/state error. For example, it is returned for `SetPageSize` when no page is started. Codes are in §11.4. |

A waiter should accept these as terminal responses to whatever request is outstanding.

### 4.4 Unsolicited packets

During printing, the printer may send packets that answer no request:

| CMD | Name | DATA | Notes |
|---|---|---|---|
| `0xE0` | PrinterPageIndex | `u16 page` | Pages completed so far. Old D11 firmware reports completion only this way. |
| `0xC6` | ResetTimeout | ? | Sent before `0xD3`. |
| `0xD3` | PrinterCheckLine | `u16 row, 01` | Some printers send it after `PageEnd`, alongside `0xE4`. The D101 sends it while rows stream in, at row 199 and at the last row. |

niimctl (B1, USB) observed "two mysterious packets" that must be read before sending `PageEnd`. These are
most likely the unsolicited packets above **(unverified)**. A robust driver dispatches every received packet:
it delivers the packet to the pending waiter if the ID matches, and otherwise logs it or routes it to a
subscriber. It must never treat an unexpected packet as the response.

---

## 5. Session setup and protocol version

### 5.1 Connect handshake

After the transport is up, send **Connect**:

```
TX  03 55 55 C1 01 01 C1 AA AA
RX  55 55 C2 01 <result> <chk> AA AA
```

| `result` | Name | Implied protocol version |
|---|---|---|
| 0 | Disconnect | Failure |
| 1 | Connected | 0 (oldest firmware) |
| 2 | ConnectedNew | 1 |
| 3 | ConnectedV3 | Ask with `PrinterStatusData` (§5.2) |
| 90 | FirmwareErrors | Failure |

Re-sending Connect at any time is harmless. Captured B21S traffic shows the vendor app sending it repeatedly,
and niimbot-rs re-sends it to recover after a timeout.

### 5.2 PrinterStatusData (`0xA5` → `0xB5`): protocol version

Only needed when `result == 3`.

```
TX  55 55 A5 01 01 A5 AA AA
RX  55 55 B5 <len> DATA… <chk> AA AA     (len is 13 or 16 in captures)
```

If `len >= 13`:

- `supportColor = DATA[10] > 0`
- `n = DATA[11] * 100 + DATA[12]`
- protocol version = **3** if `204 ≤ n < 300`, **4** if `300 ≤ n < 302`, **5** if `n ≥ 302`, otherwise 0.

The other bytes are unknown.

| Capture | DATA[10..13] | n | Version |
|---|---|---|---|
| B1 fw 5.22 | `01 02 04` | 204 | 3 |
| B21 Pro fw 3.09 | `01 03 02` | 302 | 5 |
| B21S fw 40.28 | `00 02 03` | 203 | 0 |
| D101 (`07 02`) | — (DATA is only 10 bytes) | — | 0 |

The protocol version selects the heartbeat type (§6.3), whether capabilities can be queried (§6.5), and the
default print sequence for unknown models (§8).

### 5.3 Recommended initialisation

The initialisation sequence niimbluelib uses (every step after Connect is best-effort):

1. Connect, plus PrinterStatusData if `result == 3`.
2. `PrinterInfo(PrinterModelId)`: **required**. It selects the printhead width, density range and print sequence.
3. Optionally `PrinterInfo`(SerialNumber, BluetoothAddress, BatteryChargeLevel, AutoShutdownTime) and
   `SoundSettings` reads.
4. `Heartbeat(type 3)` gives firmware/hardware versions, printhead width and DPI class. If that returns
   NotSupported, fall back to `PrinterInfo(SoftWareVersion/HardWareVersion)`.
5. If protocol ≥ 4, read `GetPrinterCapabilities`.
6. Read `RfidInfo`, `GetPaperInfo` and `RfidInfo2` to learn the loaded consumable.
7. Start a periodic heartbeat (§6.3).

---

## 6. Identity, settings and status

### 6.1 PrinterInfo (`0x40`): key/value reads

Request: `40` with payload `[key]`. Response CMD = **`0x40 + key`**, e.g. key 8 → `0x48`.

| Key | Name | Resp | Response DATA → value |
|---|---|---|---|
| 1 | Density | `41` | `u8` current density |
| 2 | Speed | `42` | `u8` |
| 3 | LabelType | `43` | `u8` label type (§11.3) |
| 6 | Language | `46` | `u8` |
| 7 | AutoShutdownTime | `47` | `u8` 1..4 (§6.6) |
| 8 | **PrinterModelId** | `48` | 2 bytes: `u16` model ID (§11.5). If only 1 byte, `id = b << 8`. |
| 9 | SoftWareVersion | `49` | 2 bytes (format ambiguous, see below) |
| 10 | BatteryChargeLevel | `4A` | `u8`. Values ≤ 4 are a 0–4 bar level (× 25 for %). Otherwise it is a percentage. |
| 11 | SerialNumber | `4B` | ≥ 8 bytes: ASCII string, e.g. `G327071185`. 4–7 bytes: uppercase hex of **all** bytes. The D101's 5 bytes `F3 10 02 01 93` give `F310020193`, matching its BLE name; niimbluelib takes only the first 4. < 4 bytes: none. |
| 12 | HardWareVersion | `4C` | 2 bytes (same format ambiguity) |
| 13 | BluetoothAddress | `4D` | 6 bytes, **reversed** (`27 03 07 17 6E 82` → `82:6E:17:07:03:27`) |
| 14 | PrintMode | `4E` | ? |
| 15 | Area | `4F` | ? |

**Version encoding is model-dependent.** For 2 bytes `[a, b]` there are two readings: `a + b/100` and
`((a<<8)|b)/100`. Captures confirm both:

- B1 `05 16` = 5.22 (first form).
- D110 `05 22` = 13.14 (second form; the capture is labelled "D110 V13.14").

niimprint always uses the second form. For an unambiguous version, prefer Heartbeat type 3 (§6.3).

### 6.2 Settings writes

| Request | CMD | Payload | Response |
|---|---|---|---|
| SetDensity | `21` | `[density]` (model range, §11.5) | `31` bool |
| SetLabelType | `23` | `[label type]` (§11.3) | `33` bool |
| SetAutoShutdownTime | `27` | `[1..4]` | `37` |
| SoundSettings get | `58` | `[02, item, 01]` | `68` `[cat, item, on]` |
| SoundSettings set | `58` | `[01, item, on?1:0]` | `68` |
| PrinterReset (resets settings, e.g. sound) | `28` | `[01]` | `38` |

Sound item: `1` = Bluetooth connection sound, `2` = power sound.

`SetPrintSpeed` (`22`→`32`), `SetLanguageType` (`26`→`36`), `SetVolumeLevel` (`2E`→`3E`) and other setters
exist (§11.1), but no source documents their payloads beyond the command ID.

### 6.3 Heartbeat (`0xDC`): live status

Request: `DC` with payload `[type]`. Possible responses are `DD`, `DE`, `DF` and `D9`, depending on type and
model. Dispatch on the **response CMD and its length**.

| Type | Name | Typical response |
|---|---|---|
| 1 | Advanced1 | `DD`: lid, battery, paper, RFID (older models) |
| 2 | Basic | `DF`. On the D101: 12 bytes, `00 00 00 39 00 …`, layout unknown. |
| 3 | "PrinterInfo" | `DE`, 10-byte identity block. NotSupported on old firmware (D110 13.14, D101). |

niimbluelib names `DE` "HeartbeatBasic" and `DF` "HeartbeatPrinterInfo". The hardware shows the opposite:
type 2 is answered with `DF` (D101) and type 3 with `DE` (B1, B21 Pro, C1, …).
| 4 | Advanced2 | `D9`: battery, temperature, lid, paper, ribbon (protocol ≥ 3) |

niimbluelib sends **type 4 if protocol ≥ 3, otherwise type 1**, every **2 s**. After **5 consecutive failures**
it treats the connection as lost.

**`DD` (response to type 1).** Layout depends on DATA length. Lid and paper bytes use `0` = closed/present.

| LEN | Lid | Battery | Paper (0=present) | Paper RFID ok (≠0) | Seen on |
|---|---|---|---|---|---|
| 9 | [8] | — | — | — | (niimprint only) |
| 10 | [8] | [9] | — | — | D110, D101 |
| 13 | [9] | [10] | [11] | [12] | B1, B21S |
| 19 | [15] | [16] | [17] | [18] | |
| 20 | — | — | [18] | [19] | |

Other bytes are unknown. **Lid polarity is inverted** on model IDs `512, 513, 514, 2304, 1792, 3584, 5120,
2560, 3840, 4352, 272, 273, 274`; on those models `0` means open. Battery uses the ≤4 → ×25 rule.

**`D9` (response to type 4).** At least 9 bytes:

```
[0..2]  unknown
[2]     battery (≤4 → ×25)
[3]     temperature (raw u8, e.g. 0x4D = 77)
[4]     lid             0 = closed
[5]     paper           0 = inserted
[6]     paper RFID ok   ≠0
[7]     ribbon RFID ok  ≠0
[8]     ribbon          0 = inserted
[9..11] u16 Wi-Fi RSSI       (if present)
[11]    unknown              (if present)
[12]    lighting error code  (if present)
[13]    voltage state        (if present)
```

**`DE` to type 3: identity block (exactly 10 bytes):**

```
[0] hw major  [1] hw minor   → "a.bb", e.g. 05 0A = 5.10
[2] fw major  [3] fw minor   → e.g. 05 16 = 5.22
[4..6] u16 printhead width in dots (B1: 0x0180 = 384)
[6] resolution class: 2 = 203 dpi, 3 = 300 dpi
[7] printhead alignment (meaning unknown)
[8] supports RFID (bool)
[9] supports RFID write (bool)
```

The printhead width reported here is the raster width the firmware uses. It can be lower than the nominal
head size (B21 Pro reports 576, niimbluelib's table says 591).

Captured examples:

```
B1   TX 55 55 DC 01 03 DE AA AA
     RX 55 55 DE 0A 05 0A 05 16 01 80 02 02 01 00 48 AA AA   hw 5.10 fw 5.22 384 dots 203dpi
B1   TX 55 55 DC 01 04 D9 AA AA
     RX 55 55 D9 09 20 41 04 4D 00 00 01 00 00 F9 AA AA      batt 100% 77° lid closed paper in
D110 TX 55 55 DC 01 01 DC AA AA
     RX 55 55 DD 0A 0F D7 00 64 00 64 32 4D 01 03 72 AA AA   lid [8]=1 (inverted model → closed), batt 75%
```

### 6.4 Consumable info

**RfidInfo (`1A` → `1B`), label roll.** Payload `[01]`. Allow a long timeout (5.5 s).

- If DATA is 1 byte, there is no tag.
- Otherwise the layout is:

  ```
  [8]   tag UUID
  u8 n, n bytes  barcode (ASCII)
  u8 n, n bytes  serial (ASCII)
  u16  total labels
  u16  used labels
  u8   consumable type (label type, §11.3)
  u16  capacity       (optional)
  [8]  second UUID    (optional)
  ```

**RfidInfo2 (`1C` → `1D`)** reads the ribbon tag (ribbon printers only), with the same layout.

**GetPaperInfo (`59` → `69`).** Payload `[01]`. Meaningful only when DATA is 18 bytes; captured 4-byte
responses are meaningless. "/10" means the value is in tenths of a millimetre.

```
u16 gapHeightPx, u16 totalHeightPx, u8 paperType,
u16 gapHeight/10 mm, u16 totalHeight/10 mm,
u16 paperWidthPx, u16 paperWidth/10 mm,
u8 direction, u16 tailLengthPx, u16 tailLength/10 mm
```

On B21 Pro (model 785), `totalHeight` includes the gap.

### 6.5 GetPrinterCapabilities (`AF` → `BF`), protocol ≥ 4

Payload `[01]`. The response is a TLV stream: `u8 type, u8 len, len bytes`, repeated. Ignore unknown types.
Fields marked "bits" are bitmasks, decoded as a list of set bit positions.

| T | Field | Value | T | Field | Value |
|---|---|---|---|---|---|
| 01 | language | bits | 13 | cutter depth range | u8 max, u8 min |
| 02 | print mode | bits | 14 | print control | bits |
| 03 | UHF RFID | bits | 15 | pause time support | bits |
| 04 | printhead DPI | u16 (200/300) | 16 | paper detection | u8 |
| 05 | RFID support | bits | 17 | real-time clock | bits |
| 06 | battery range | u8 max, u8 min | 18 | key functions | 5-byte records: u8 key + 4-byte bits |
| 07 | **density range** | u8 max, u8 min | 19 | unknown | u8 |
| 08 | speed range | u8 max, u8 min | 1A | print colour | bits |
| 09 | supported label types | bits (4 bytes) | 1B | speed/quality mode | bits |
| 0A | **printhead width** | u16 | 1C | tube calibration | bits |
| 0B | max print height | u16 | 1D | partial retransmit | bits |
| 0C | label height & gap | u8 | 1E | max compress lines | u16 |
| 0D | printhead position | u8 | 1F | tube support | bits |
| 0E | volume support | bits | 20 | 16-gray max buffer | u16 |
| 0F | host style | bits | 21 | local template support | bits |
| 10 | print protocol | bits | 22 | image compress support | bits |
| 11 | auto-shutdown range | u8 max, u8 min | 23 | max image compress bytes | u32 |
| 12 | cutter support | bits | 24 | local template max time count | u8 |

When present, these fields replace hard-coded model tables for density range and printhead width.

### 6.6 Enumerations used above

AutoShutdownTime: `1` ≈ 15 min, `2` ≈ 30 min, `3` = 45 or 60 min, `4` = 60 min or never. The exact values
depend on the model.

---

## 7. Printing rasters

### 7.1 Raster model

- A **page** is a 1-bit image of `rows × cols` pixels.
- **cols** is the dimension across the printhead. It must be **≤ the printhead width in dots** (§11.5), and is
  padded to a **multiple of 8**.
- **rows** is the dimension along the paper feed, i.e. label length in dots. A row is one burn line.
- Each row is `cols/8` bytes, with **MSB = leftmost pixel**. Bit `1` = black (burn) and `0` = white. Most
  source images use dark-on-light, so they must be inverted.
- Resolution: 203 dpi ≈ 8 dots/mm and 300 dpi ≈ 11.8 dots/mm.
- **Orientation.** On "top" models (B-series, wide heads) the label's horizontal axis runs across the head:
  a 50 mm wide label on a B1 maps to `cols ≤ 384`. On "left" models (D11, D110, H1S and other narrow
  side-feed printers) labels are usually designed landscape, so niimbluelib **rotates the image 90° clockwise**
  before encoding. The label's height becomes `cols` (≤ 96 for D11/D110) and its length becomes `rows`.
  The printer itself knows nothing about orientation.
- **Rasters narrower than the head.** Senders transmit `cols` = image width and don't pad to the head width.
  On the D101 a narrower raster is **aligned to the last head dot**: column `c` of a `W`-column raster burns
  head dot `192 − W + c`, and the label is registered against that same end of the head (see the D101 section).
  On the **B1** a narrower raster is **centred** on the head (measured). **Alignment is model-specific.** The
  driver needs a per-model rule, or must always send a full-head-width raster, which is the safe choice. Heartbeat type 3 reports a
  "printhead alignment" byte that may govern this **(unverified)**. NiimPrintX implements offsets by padding the
  image itself.

### 7.2 Row packets (one-way, no response)

Row numbers (`pos`) are absolute, 0-based within the page, and must be sent in increasing order covering all
rows. Each packet can cover a run of `repeats` (1..255) identical consecutive rows.

**PrintEmptyRow `0x84`**: blank rows.

```
DATA = u16 pos, u8 repeats
e.g.   55 55 84 03 00 01 01 87 AA AA          (row 1, 1 blank row)
```

**PrintBitmapRow `0x85`**: one row of pixels, repeated.

```
DATA = u16 pos, u8 count0, u8 count1, u8 count2, u8 repeats, row bytes[cols/8]
e.g.   55 55 85 08 00 00 08 00 00 01 FF 00 7B AA AA   (row 0, 8 black px, 16-col row)
```

**PrintBitmapRowIndexed `0x83`**: sparse row, used for **≤ 6 black pixels**.

```
DATA = u16 pos, u8 count0, u8 count1, u8 count2, u8 repeats, u16 x0, u16 x1, …  (column index of each black pixel)
```

niimbluelib uses `0x83` automatically when a row has ≤ 6 black pixels. Its source warns that the printer
**powers off** if an indexed row lists more than 6 pixels. `0x83` is optional: `0x85` works for any row and is
the only form niimprint, NiimPrintX and niimctl use.

**The three "black pixel count" bytes.** Sources disagree, which suggests most firmware ignores them:

| Source | count bytes |
|---|---|
| niimprint, NiimPrintX | always `00 00 00` ("it seems like you can always send zeros"). Works on B21, D11, D110 and others. |
| niimctl (B1) | u16 BE total black pixels, then `00`. Plus `repeats`. |
| niimbluelib, **"split"** (default when it fits) | Split the row's bytes into 3 chunks of `floor(headDots / 8 / 3)` bytes. `countN` = black pixels in chunk N. Used when `cols/8 ≤ 3 × chunk`. |
| niimbluelib, **"total"** (B21 V1, B21 L2B, or row too wide) | `00`, `total & 0xFF`, `total >> 8` |

Recommendation: implement niimbluelib's rules. They have the widest hardware testing and match what the vendor
app sends.

**PrinterCheckLine `0x86`** (only B21 V1 and B21 L2B sequences send it).

```
DATA = u16 row, 01
```

It is sent after every 200th row, when `row % 200 == 199`. Its response is `D3`. niimbluelib still sends it
through the generic send-and-wait path, so it waits for that response.

**Encoding a page (niimbluelib's encoder):**

1. For each row, compute its bytes. An all-zero row is "void".
2. Merge consecutive identical rows into one packet by incrementing `repeats`. Split runs at 255, because
   `repeats` is a `u8` and niimbluelib's mono path doesn't split them.
3. Emit void runs as `0x84`, other runs as `0x85` (or `0x83` if ≤ 6 black pixels), interleaved with `0x86`
   checks where the sequence requires them.

### 7.3 Job and page commands

| Request | CMD | Payload | Response |
|---|---|---|---|
| PrintStart | `01` | see variants below | `02` bool |
| PageStart | `03` | `01` | `04` bool. `00` = refused, retry. |
| SetPageSize | `13` | see variants below | `14` bool. `DB` error if no page has been started. |
| PrintQuantity | `15` | `u16 copies` | `16` |
| PrintClear | `20` | `01` | `30` |
| PageEnd | `E3` | `01` | `E4` bool. `00` = not accepted yet, retry. |
| PrintStatus | `A3` | `01` | `B3` (§7.5) |
| PrintEnd | `F3` | `01` | `F4` bool. `00` = still printing, poll again. |
| CancelPrint | `DA` | `01` | `D0` |

**PrintStart payload variants:**

| Bytes | Layout | Used by |
|---|---|---|
| 1 | `01` | D11 V1, D110, B21 V1, B21 L2B |
| 2 | `u16 totalPages` | H1S |
| 7 | `u16 totalPages, 00 00 00 00, u8 pageColor` | B1 family. **Required on B1**, the only form niimctl found to work. |
| 9 | 7-byte form `+ u8 speed, u8 flag` | D110M V4 / protocol ≥ 4 |

`totalPages` counts every physical label in the job, copies included. `pageColor` is `0` for monochrome
(§11.6). With the 1-byte form on a B1, the paper stops under the head after `PageEnd` and only feeds out on
`PrintEnd`. With the 7-byte form, the B1 waits between pages and feeds after the last one.

**SetPageSize payload variants:**

| Bytes | Layout | Used by |
|---|---|---|
| 2 | `u16 rows` | D11 V1 |
| 4 | `u16 rows, u16 cols` | D110, B21 V1, B21 L2B. On B1 the first print comes out blank or as many copies, so avoid it. |
| 6 | `u16 rows, u16 cols, u16 copies` | B1 family |
| 9 | 6-byte form `+ u16 cutHeight, u8 cutType` | H1S |
| 13 | 9-byte form `+ 00, u8 sendAll, u16 partHeight` (`+ 32-byte template ID`, optional) | D110M V4 / protocol ≥ 4 |

### 7.4 Generic job

```
SetDensity(d)            21 [d]                 → 31 01
SetLabelType(t)          23 [t]                 → 33 01      (t = 1 "with gaps" for normal die-cut labels)
PrintStart(...)          01 [...]               → 02 01
for each distinct page:
    [PrintClear]         20 01                  → 30         (some models)
    PageStart            03 01                  → 04 01
    SetPageSize(...)     13 [...]               → 14 01
    [PrintQuantity(n)]   15 [n]                 → 16         (models using the 4-byte page size)
    row packets…         84/85/83 (one-way), [86 → D3]
    PageEnd              E3 01                  → E4 01      (retry while 00)
    [wait for page]
wait for job completion (§7.5)
PrintEnd                 F3 01                  → F4 01      (always send it, even after an error)
```

Byte-exact trace for a 2-row × 16-col page on a B1-family printer:

```
>> 55 55 21 01 03 23 AA AA                         SetDensity 3
<< 55 55 31 01 01 31 AA AA
>> 55 55 23 01 01 23 AA AA                         SetLabelType 1
<< 55 55 33 01 01 33 AA AA
>> 55 55 01 07 00 01 00 00 00 00 00 07 AA AA       PrintStart(total=1, mono)
<< 55 55 02 01 01 02 AA AA
>> 55 55 03 01 01 03 AA AA                         PageStart
<< 55 55 04 01 01 04 AA AA
>> 55 55 13 06 00 02 00 10 00 01 06 AA AA          SetPageSize(rows=2, cols=16, copies=1)
<< 55 55 14 01 01 14 AA AA
>> 55 55 85 08 00 00 08 00 00 01 FF 00 7B AA AA    row 0: 8 black px on the left
>> 55 55 84 03 00 01 01 87 AA AA                   row 1: blank
>> 55 55 E3 01 01 E3 AA AA                         PageEnd
<< 55 55 E4 01 01 E4 AA AA
>> 55 55 A3 01 01 A3 AA AA                         PrintStatus (poll every ~300 ms)
<< 55 55 B3 04 00 01 64 64 B6 AA AA                page=1, 100%, 100% → done
>> 55 55 F3 01 01 F3 AA AA                         PrintEnd
<< 55 55 F4 01 01 F4 AA AA
```

The `<<` lines are the expected response shapes; only the `>>` lines come from captures. The checksums are
computed with the §3.1 rule.

### 7.5 Completion detection

Sources use three methods. Which one applies depends on the model (§8).

1. **Poll PrintStatus.** Send `A3 01` every ~300 ms and stop when `page == totalPages`.

   The `B3` response is at least 4 bytes: `u16 page` (pages finished), `u8 printProgress` (0–100),
   `u8 feedProgress` (0–100). If it is 10 bytes, `DATA[6]` is an **error code** (§11.4); non-zero means the job
   failed. niimctl decoded B1's 10-byte response as 5 × u16 **(1 src)**. An idle D101 returns 8 bytes:
   `00 00 00 00 06 A3 00 00`. niimctl sends the request with an
   empty payload, which also works.
2. **Wait for the `E0` PrinterPageIndex event** (`u16 page`) until `page == totalPages`. Reset a ~5 s
   inactivity timeout on each event. Required for old D11 (`D11_V1`).
3. **Poll PrintEnd.** Send `F3 01` every ~300 ms until it returns `01`. No separate PrintEnd is needed
   afterwards. Used by B21 V1, and by niimprint for all models.

---

## 8. Model-specific print sequences

niimbluelib selects a print sequence ("print task") by model and protocol version. The rules:

1. An exact `(model, protocolVersion)` entry wins.
2. Otherwise use the model entry.
3. Otherwise, if protocol ≥ 4, use `D110M_V4`.
4. Otherwise the model is unknown, so fall back to the B1 or D110 sequence by experimentation.

| Task | Models | PrintStart | Per page | Completion |
|---|---|---|---|---|
| **D11_V1** | D11, D11S | 1-byte | PrintClear, PageStart, SetPageSize **2-byte** (rows), PrintQuantity(n), rows, PageEnd | `E0` page-index events |
| **D110** | D110, B21S, B21S C2B, D11 at protocol 1 or 2 | 1-byte | PrintClear, PageStart, SetPageSize **4-byte**, PrintQuantity(n), rows, PageEnd | Poll PrintStatus after **each** page, waiting for the cumulative page count |
| **B1** | B1, D110 M, B21 C2B, M2 H, N1, D101 | **7-byte** (total, color) | PageStart, SetPageSize **6-byte** (rows, cols, copies), rows, PageEnd | Poll PrintStatus until `page == total` |
| **B21_V1** | B21 | 1-byte | Once per copy: PageStart, SetPageSize 4-byte, rows with counts "total" and **check lines**, PageEnd | Poll PrintEnd until true |
| **B21_L2B** | B21 L2B | 1-byte | Once per copy: PageStart (**retry** up to 5× / 500 ms while `00`), SetPageSize 4-byte, rows with counts "total" and check lines, PageEnd; then **retry PageEnd** up to 20× / 500 ms until `01` | Done when the last PageEnd succeeds |
| **H1S** | H1S | **2-byte** (total) | PageStart, SetPageSize **9-byte**, rows, PageEnd | Poll PrintStatus |
| **D110M_V4** | D110 M at protocol 4, D11 H, B21 Pro, B1 Pro, C1, EP1C, any protocol ≥ 4 | SetLabelType → SetDensity → **9-byte** (total, color, speed) | One-way PrintStatus (no wait), SetPageSize **13-byte** (no PageStart), rows, PageEnd | Poll PrintStatus. After PrintEnd, send a one-way Heartbeat(1). |

Workarounds:

- **B21 Pro over BLE** ignores the first packet after PrintStart and drops the first packet after PrintEnd. The
  D110M_V4 sequence therefore sends a throwaway one-way PrintStatus before the page and a throwaway one-way
  Heartbeat after PrintEnd. This copies the vendor app.
- **B1** with the 4-byte SetPageSize prints a blank first label or runs off many copies. Use the 6-byte form.
- niimprint's single sequence (density, label type 1, 1-byte PrintStart, PageStart, 4-byte SetPageSize, rows
  with zero counts, PageEnd, poll PrintEnd) works on B21 and D11 but not B1, according to niimctl's author.
  This is why the B1 family needs its own variant.

**Copies:** the B1, H1S and D110M_V4 sequences send the page once with `copies` in SetPageSize, and include
copies in `totalPages`. D11_V1 and D110 use `PrintQuantity`. B21_V1 and B21_L2B re-send the whole page for
each copy.

---

## 9. Extensions: two-colour and grayscale

These are only needed for colour or grayscale models. Monochrome drivers can skip this section.

**Two-colour (black/red), `pageColor = 1` in PrintStart.** Rows are wrapped in `PrintBitmapRowDoubleColor`
`0x8A`, which is one-way.

- Single-colour row: `[mode, 0x85, u16 pos, u8 repeats, bytes…]` with mode `1` = red or `2` = black.
  There are no count bytes.
- Mixed row: `[3, u16 pos, u8 repeats, mask[cols/8], colour bytes…]`. `mask` = black OR red. One colour byte
  (bit set = red) follows for each **non-zero** mask byte, in order.
- Empty row: `[0, 0x84, u16 pos, u8 repeats]`.

**16-level grayscale, `pageColor = 3`.** This uses `PrintBitmapRowGray` `0x8D` with a **16-bit length**
(§3.3), and `cols` must be even.

- Empty: `[0x84, u16 pos, u8 repeats, 00]`.
- Data: `[0x86, u16 pos, u8 repeats, u8 runBytesLen, runs…, data…]`. Data is 4 bits per pixel, high nibble
  first, `0` = white and `15` = black.
- Each run descriptor is 2 bytes: `(pixelCount + 2) | ((dataIndex >> 8) & 1)`, `dataIndex & 0xFF`. The printer
  repeats the last dot of the referenced data byte.
- Check lines every 100 rows.

---

## 10. Implementation guidance

- **I/O model.** Use a single reader task that re-assembles packets (§3.4) and dispatches each one. Give it a
  request API, `send_and_wait(packet, accepted_ids, timeout)`, that also resolves on `0x00` or `0xDB` (§4.3),
  plus a broadcast channel for unsolicited packets (§4.4). Serialise requests with a mutex.
- **Pacing.** Wait 10 ms between writes. When streaming rows, also stop if the transport reports a disconnect.
- **Model table.** Key it on the model ID. Some IDs are shared by several entries (256, 512, 2305), so the
  first match wins and the hardware is the same. Prefer printer-reported values when available: Heartbeat type
  3 for printhead width and DPI, capabilities for density range.
- **Always send PrintEnd** after a job starts, including on error or cancel. Otherwise the printer can stay in
  print mode.
- **Validate before printing.**
  - `cols ≤ printhead width`
  - `density` within the model's range (density 4–5 on a 3-level D11 is clamped by niimprint)
  - `totalPages` matches what is sent
- **Keepalive.** Run a periodic heartbeat (2 s) while connected. It also tracks lid, paper and battery changes.
  niimbluelib re-reads RFID when the lid or RFID state changes.

---

## 11. Reference tables

### 11.1 Commands (request → response)

Response `—` means one-way. Payload `01` means the dummy byte.

| Req | Name | Resp | Payload / notes |
|---|---|---|---|
| `01` | PrintStart | `02` | §7.3 |
| `03` | PageStart | `04` | `01` |
| `05` | PrinterLog | `06` | |
| `07` | PrinterConfig2 | `08` | e.g. set time `[08, 01, u16 year, mon, day, h, m, s]` |
| `09` | GetKeyFunction | `0A` | |
| `0B` | AntiFake | `0C` | `01` long / `02` short (vendor app) |
| `0D` | GetPrintQuality | `0D` | |
| `0E` | TubeSettings | `0E` | |
| `0F` | TubeTypeAndWidth | `0F` | `[01, 01, tubeType, u16 width×100 mm]` |
| `11` | SetCurrentTimeFormat | `11` | |
| `12` | GetCurrentTimeFormatOrTemplateInfo | `12` | |
| `13` | SetPageSize | `14` | §7.3 |
| `15` | PrintQuantity | `16` | `u16` |
| `17` | GetTemplateHistory | `17` | |
| `19` | GetPrinterUsage | `19` | |
| `1A` | RfidInfo (labels) | `1B` | §6.4 |
| `1C` | RfidInfo2 (ribbon) | `1D` | §6.4 |
| `1E` | SetReversePrinterFeedOrMarginTop | `1E` / `1F` | |
| `20` | PrintClear | `30` | `01` |
| `21` | SetDensity | `31` | `[d]` |
| `22` | SetPrintSpeed | `32` | |
| `23` | SetLabelType | `33` | `[t]` |
| `26` | SetLanguageType | `36` | |
| `27` | SetAutoShutdownTime | `37` | `[1..4]` |
| `28` | PrinterReset | `38` | `01` |
| `2D` | SetLabelMaterial | `3D` | |
| `2E` | SetVolumeLevel | `3E` | |
| `2F` | SetAntiSetter | `3F` | |
| `40` | PrinterInfo | `40+key` | §6.1 |
| `54` | RfidSuccessTimes | `64` | |
| `58` | SoundSettings | `68` | §6.2 |
| `59` | GetPaperInfo | `69` | §6.4 |
| `5A` | PrintTestPage | `6A` | |
| `5C` | HalfCut | `6C` | `[01, on]` |
| `70` | GetVolumeLevel | `71` | |
| `83` | PrintBitmapRowIndexed | — | §7.2 |
| `84` | PrintEmptyRow | — | §7.2 |
| `85` | PrintBitmapRow | — | §7.2 |
| `86` | PrinterCheckLine | `D3` | §7.2 |
| `89` | GetCompress | `88` | |
| `8A` | PrintBitmapRowDoubleColor | — | §9 |
| `8D` | PrintBitmapRowGray | — | §9 (16-bit length) |
| `8E` | LabelPositioningCalibration | `8F` | `[v]`. On B1, 1 or 2 feeds about 15 cm of paper. |
| `91` `92` `9B` `9C` | Firmware CRC / Commit / Chunk / NoMoreChunks | — | CRC32 packets, firmware upgrade |
| `A2` | GetPrinterConfigurationWifi | `B2` | |
| `A3` | PrintStatus | `B3` | §7.5 |
| `A5` | PrinterStatusData | `B5` | §5.2 |
| `A6` | Pause | `B6` | |
| `AF` | GetPrinterCapabilities | `BF` | §6.5 |
| `C1` | Connect | `C2` | **`03` prefix**, §5.1 |
| `C3` | GetPrinterFree | `C4` | |
| `DA` | CancelPrint | `D0` | |
| `DC` | Heartbeat | `DD`/`DE`/`DF`/`D9` | §6.3 |
| `E3` | PageEnd | `E4` | `01` |
| `F3` | PrintEnd | `F4` | `01` |
| `F5` | StartFirmwareUpgrade | `F6` | `[major, minor]` |

Printer-originated: `00` NotSupported, `DB` PrintError, `E0` PrinterPageIndex, `C6` ResetTimeout,
`D3` PrinterCheckLine, `90` `9A` `9D` `9E` firmware.

### 11.2 Response ID quick lookup

`00` NotSupported · `02` PrintStart · `04` PageStart · `14` SetPageSize · `16` PrintQuantity · `1B` RfidInfo ·
`30` PrintClear · `31` SetDensity · `33` SetLabelType · `37` SetAutoShutdownTime · `41`–`4F` PrinterInfo ·
`68` SoundSettings · `69` PaperInfo · `B3` PrintStatus · `B5` PrinterStatusData · `BF` Capabilities ·
`C2` Connect · `D0` CancelPrint · `D3` CheckLine · `D9`/`DD`/`DE`/`DF` Heartbeat · `DB` PrintError ·
`E0` PageIndex · `E4` PageEnd · `F4` PrintEnd

### 11.3 Label types (SetLabelType, RFID consumable type)

| Value | Meaning |
|---|---|
| 1 | With gaps (die-cut). Default for most models. |
| 2 | Black mark |
| 3 | Continuous |
| 4 | Perforated |
| 5 | Transparent |
| 6 | PVC tag |
| 10 | Black mark + gap |
| 11 | Heat-shrink tube |

niimprint and NiimPrintX only allow values 1–3.

### 11.4 Print error codes (`0xDB` payload, PrintStatus error byte)

| Code | Meaning | Code | Meaning |
|---|---|---|---|
| 01 | Cover open | 13 | Set print density failed |
| 02 | No paper | 14 | RFID write failed |
| 03 | Low battery | 15 | Set margin failed |
| 04 | Battery exception | 16 | Communication exception |
| 05 | User cancel | 17 | Disconnect |
| 06 | Data error | 18 | Canvas parameter error |
| 07 | Overheat | 19 | Rotation parameter error |
| 08 | Paper-out exception | 1A | JSON parameter error |
| 09 | Printer busy | 1B | Abnormal paper output (B3S) |
| 0A | No printhead | 1C | Check paper |
| 0B | Temperature low | 1D | RFID tag not written |
| 0C | Printhead loose | 1E | Density not supported |
| 0D | No ribbon | 1F | Print mode not supported |
| 0E | Wrong ribbon | 20 | Label material error |
| 0F | Used ribbon | 21 | Label material not supported |
| 10 | Wrong paper | 22 | RFID write not supported |
| 11 | Set paper failed | 32 | Illegal page |
| 12 | Set print mode failed | 33 | Illegal ribbon page |
| | | 34 | Receive data timeout |
| | | 35 | Non-dedicated ribbon |

### 11.5 Models

The model ID comes from `PrinterInfo(8)`. Columns:

- **Head**: printhead width in dots, i.e. the maximum `cols`.
- **Dir**: the image rotation convention (§7.1).
- **Density**: min–max, with the default in parentheses.
- **Task**: the niimbluelib print sequence (§8). `—` means not yet characterised.

This table is generated from niimbluelib's `printer_models.ts`, which is itself generated from vendor data.

| Model | ID | DPI | Head | Dir | Density | Task |
|---|---|---|---|---|---|---|
| A1_PRO | 7424 (0x1D00) | 300 | 296 | left | 1–5 (3) | — |
| A20 | 2817 (0x0B01) | 203 | 400 | top | 1–5 (3) | — |
| A203 | 2818 (0x0B02) | 203 | 400 | top | 1–5 (3) | — |
| A63 | 2054 (0x0806) | 300 | 851 | top | 1–15 (10) | — |
| A8 | 256 (0x0100) | 203 | 576 | top | 1–5 (3) | — |
| A8_P | 273 (0x0111) | 203 | 576 | top | 1–5 (3) | — |
| B1 | 4096 (0x1000) | 203 | 384 | top | 1–5 (3) | B1 |
| B1_PRO | 4097 (0x1001) | 300 | 567 | top | 1–5 (3) | D110M_V4 |
| B1_SE | 4098 (0x1002) | 203 | 384 | top | 1–5 (3) | — |
| B11 | 51457 (0xC901) | 203 | 384 | top | 6–15 (10) | — |
| B16 | 1792 (0x0700) | 203 | 96 | left | 1–3 (2) | — |
| B18 | 3584 (0x0E00) | 203 | 96 | left | 1–3 (2) | — |
| B18S | 3585 (0x0E01) | 203 | 96 | left | 1–3 (2) | — |
| B2 | 6913 (0x1B01) | 203 | 384 | top | 1–5 (3) | — |
| B2_PRO | 6912 (0x1B00) | 300 | 567 | top | 1–5 (3) | — |
| B203 | 2816 (0x0B00) | 203 | 400 | top | 1–5 (3) | — |
| B21 | 768 (0x0300) | 203 | 384 | top | 1–5 (3) | B21_V1 |
| B21_PRO | 785 (0x0311) | 300 | 591 | top | 1–5 (3) | D110M_V4 |
| B21_C2B | 771 (0x0303), 775 (0x0307) | 203 | 384 | top | 1–5 (3) | B1 |
| B21_L2B | 769 (0x0301) | 203 | 384 | top | 1–5 (3) | B21_L2B |
| B21S | 777 (0x0309) | 203 | 384 | top | 1–5 (3) | D110 |
| B21S_C2B | 776 (0x0308) | 203 | 384 | top | 1–5 (3) | D110 |
| B3 | 52993 (0xCF01) | 203 | 600 | top | 1–5 (3) | — |
| B31 | 5632 (0x1600) | 203 | 600 | top | 1–5 (3) | — |
| B32 | 2049 (0x0801) | 300 | 851 | top | 1–15 (10) | — |
| B32R | 2050 (0x0802) | 300 | 851 | top | 1–15 (10) | — |
| B3S | 256 (0x0100), 260 (0x0104), 262 (0x0106) | 203 | 576 | top | 1–5 (3) | — |
| B3S_A | 275 (0x0113) | 203 | 576 | top | 1–5 (3) | — |
| B3S_P | 272 (0x0110) | 203 | 576 | top | 1–5 (3) | — |
| B4 | 6656 (0x1A00) | 203 | 832 | top | 1–5 (3) | — |
| B4_PRO | 6657 (0x1A01) | 300 | 1229 | top | 1–5 (3) | — |
| B50 | 51713 (0xCA01) | 203 | 400 | top | 6–15 (10) | — |
| B50W | 51714 (0xCA02) | 203 | 384 | top | 6–15 (10) | — |
| BETTY | 2561 (0x0A01) | 203 | 192 | left | 1–3 (2) | — |
| C1 | 5120 (0x1400) | 300 | 178 | left | 1–5 (3) | D110M_V4 |
| D101 | 2560 (0x0A00) | 203 | 192 | left | 1–3 (2) | B1 |
| D11 | 512 (0x0200) | 203 | 96 | left | 1–3 (2) | D11_V1 (protocol 1, 2: D110) |
| D11_H | 528 (0x0210) | 300 | 142 | left | 1–5 (3) | D110M_V4 |
| D11_PRO | 531 (0x0213) | 300 | 142 | left | 1–5 (3) | — |
| D110 | 2304 (0x0900), 2305 (0x0901) | 203 | 96 | left | 1–3 (2) | D110 |
| D110_M | 2320 (0x0910) | 203 | 96 | left | 1–5 (3) | B1 (protocol 4: D110M_V4) |
| D11S | 514 (0x0202) | 203 | 96 | left | 1–3 (2) | D11_V1 |
| EP1C | 5121 (0x1401) | 300 | 178 | left | 1–5 (3) | D110M_V4 |
| EP2M_H | 4610 (0x1202) | 300 | 567 | top | 1–5 (3) | — |
| EP3M | 6402 (0x1902) | 300 | 851 | top | 1–5 (3) | — |
| ET10 | 5376 (0x1500) | 203 | 1600 | top | 3–3 (3) | — |
| FUST | 513 (0x0201) | 203 | 96 | left | 1–5 (3) | — |
| H1 | 3840 (0x0F00) | 203 | 96 | left | 1–3 (2) | — |
| H1S | 4352 (0x1100) | 203 | 96 | left | 1–3 (2) | H1S |
| HI_D110 | 2305 (0x0901) | 203 | 120 | left | 1–3 (3) | — |
| HI_NB_D11 | 512 (0x0200) | 203 | 120 | left | 1–3 (2) | — |
| JC_M90 | 51461 (0xC905) | 203 | 384 | top | 6–15 (10) | — |
| JCB3S | 256 (0x0100) | 203 | 576 | top | 1–5 (2) | — |
| K2 | 6144 (0x1800) | 203 | 448 | top | 1–5 (3) | — |
| K3 | 4864 (0x1300) | 203 | 640 | top | 1–5 (3) | — |
| K3_ITD | 4868 (0x1304) | 203 | 640 | top | 1–5 (3) | — |
| K3_W | 4865 (0x1301) | 203 | 640 | top | 1–5 (3) | — |
| K4 | 7168 (0x1C00) | 203 | 832 | top | 1–15 (7) | — |
| M2_H | 4608 (0x1200) | 300 | 567 | top | 1–5 (3) | B1 |
| M3 | 6400 (0x1900) | 300 | 851 | top | 1–5 (3) | — |
| MP3K | 4866 (0x1302) | 203 | 640 | top | 1–5 (3) | — |
| MP3K_W | 4867 (0x1303) | 203 | 640 | top | 1–5 (3) | — |
| N1 | 3586 (0x0E02) | 203 | 96 | left | 1–3 (2) | B1 |
| P1 | 1024 (0x0400) | 300 | 697 | left | 1–5 (3) | — |
| P18 | 1026 (0x0402) | 300 | 662 | left | 1–5 (3) | — |
| P1S | 1025 (0x0401) | 300 | 662 | left | 1–5 (3) | — |
| S1 | 51458 (0xC902) | 203 | 384 | top | 6–15 (10) | — |
| S3 | 51460 (0xC904) | 203 | 384 | top | 6–15 (10) | — |
| S6 | 261 (0x0105), 259 (0x0103), 258 (0x0102), 257 (0x0101) | 203 | 576 | top | 1–5 (3) | — |
| S6_P | 274 (0x0112) | 203 | 600 | top | 1–5 (3) | — |
| T2S | 53250 (0xD002) | 203 | 832 | top | 1–20 (15) | — |
| T6 | 51715 (0xCA03) | 203 | 384 | top | 6–15 (10) | — |
| T7 | 51717 (0xCA05) | 203 | 384 | top | 6–15 (10) | — |
| T8 | 51718 (0xCA06) | 300 | 567 | top | 6–15 (10) | — |
| T8S | 2053 (0x0805) | 300 | 851 | top | 1–15 (10) | — |
| TP2M_H | 4609 (0x1201) | 300 | 591 | top | 1–5 (3) | — |
| Z401 | 2051 (0x0803) | 300 | 851 | top | 1–15 (10) | — |

### 11.6 Page colour (PrintStart)

| Value | Meaning |
|---|---|
| 0 | Single colour |
| 1 | Two colours |
| 2 | Single colour (alternate) |
| 3 | 16-level grayscale |

---

## 12. Discrepancies and open questions

1. **Bitmap-row count bytes:** four different encodings are in use (§7.2), all reported to work. Is there a
   firmware that actually validates them?
2. **Raster alignment:** measured as end-aligned on the D101 and centred on the B1. Does the heartbeat-3
   "alignment" byte (B1: `2`) encode this? The D101 doesn't support heartbeat 3. Until this is known, send
   full-head-width rasters and position content inside them.
3. **Version encoding:** `a.b` vs `u16/100` for PrinterInfo 9 and 12 (§6.1). Is it keyed by model or by
   firmware generation?
4. **USB:** product IDs; whether CDC line-coding and DTR are needed; whether the port is only live while
   powered off and charging.
5. **BLE MTU:** the behaviour when a row packet exceeds `ATT_MTU − 3`; neither source fragments.
6. **Heartbeat type 2** → `DF`: the D101 confirms that niimbluelib swaps the `DE`/`DF` names, but the `DF`
   payload layout is still unknown.
7. **Unknown models** (`—` in §11.5): which print sequence each model needs. A reasonable default is B1 for
   protocol 3, D110M_V4 for protocol ≥ 4, and D110 otherwise. Confirm on hardware.

---

## Sources

| Project | Language | Transport | Notes |
|---|---|---|---|
| [MultiMote/niimbluelib](https://github.com/MultiMote/niimbluelib) (@35110a7, 2026-11-08) | TypeScript | Web Bluetooth, Web Serial | Most complete. It is the source for most command IDs, payload variants, print tasks, the model table and the captured traces (`src/test/dumps.ts`). Not on your list, but it is the reference the other projects copy from. |
| [AndBondStyle/niimprint](https://github.com/AndBondStyle/niimprint) (@35c41a7) | Python | RFCOMM, serial | The original reverse-engineering. Single print sequence, zero count bytes. |
| [labbots/NiimPrintX](https://github.com/labbots/NiimPrintX) (@b987d37) | Python | BLE (bleak) | niimprint's protocol over BLE. Characteristic discovery, 10 ms row pacing. |
| [hairymnstr/niimctl](https://github.com/hairymnstr/niimctl) (@c3eec91) | Python | USB serial | B1 specifics: 7-byte PrintStart, 6-byte SetPageSize, empty-row packets, PrintStatus polling. |
| [wiktorpyk/niimbot-rs](https://github.com/wiktorpyk/niimbot-rs) (@adada84) | Rust | USB serial | Port of niimbluelib's command tables. USB VID `0x3513`, Connect-based recovery. |

Further reading, not consulted directly: the NIIMBOT community wiki at <https://printers.niim.blue/interfacing/proto/>,
which niimctl's README cites.
