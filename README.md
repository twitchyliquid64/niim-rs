# niim-rs

A pure-Rust userland driver for NIIMBOT label makers, with an async Rust programming interface.

```sh
cargo install --git https://github.com/twitchyliquid64/niim-rs niimctl
niimctl label 'Hello'      # prints on the first printer found
niimctl print label.png
```

## Crate map

| Crate     | Role |
|-----------|------|
| `niim-core` (`crates/core`) | The library: device discovery, the printer protocol and the async printing API. |
| `niimctl` (`crates/niimctl`) | A small command-line frontend over `niim-core`. |

## Usage

```sh
niimctl list                                   # printers over BLE and USB
niimctl status -p D101                         # identity, lid, paper, battery, roll
niimctl test-label 40 12 -o label.png          # orientation/placement test image (mm, as read)
niimctl print -p D101 label.png                # print a PNG as the label is read
niimctl print -p B1 label.png -d 3 -n 2        # density 3, two copies
niimctl label -p D101 '[b]Spare keys[/b]\nGarage'   # print a label from markup
```

`-p` matches any part of the printer name (`MODEL-SERIAL`). Set `RUST_LOG=niim_core=trace` to see every packet.

### Labels from markup

`niimctl label` lays out text, QR codes, barcodes, AprilTags and images, as large as fits the label:

```sh
niimctl label 'Hello'
niimctl label '[qr:https://example.com][col][left][big]Shelf 4\n[normal]Bin 12'
niimctl label '[left]Flour[right]2026-11\n[center][code128:PANTRY-07]'
niimctl label --size 50x30 --preview out.png '[img 10mm:logo.png]\n[small]Property of Tom'
niimctl label -f label.txt                     # or `-` to read standard input
```

`\n` or a real newline starts a new line. Tags change what follows them:

| Tag | Effect |
|---|---|
| `[left]` `[center]` `[right]` | Horizontal alignment from here on (default: centre). Mid-line, it starts a new part of the same line: `[left]Name[right]Date`. |
| `[top]` `[middle]` `[bottom]` | Vertical alignment (default: middle). |
| `[small]` `[normal]` `[big]` | Text size relative to the auto-fitted size. |
| `[size:4mm]` | A fixed font size (`mm`, `pt` or `px`), exempt from auto-fit. |
| `[b]` … `[/b]` | Bold. |
| `[col]` | Start a new column to the right. |
| `[qr:DATA]` | QR code. |
| `[code128:DATA]` `[ean13:DIGITS]` `[ean8:DIGITS]` `[upca:DIGITS]` | Barcodes. |
| `[apriltag:ID]` | AprilTag, family tag36h11; `[apriltag 25h9:ID]` or `[apriltag 41h12:ID]` (tagStandard41h12) for another. |
| `[img:FILE.png]` | Image, dithered; `[img threshold:FILE.png]` for logos and line art. |

Codes and images take an optional height (`[qr 10mm:DATA]`, `[img 8mm:logo.png]`). Write `\[`, `\]`
and `\\` for literal characters. Lines don't wrap: text shrinks to fit instead, and `niimctl` warns
when text or a code gets too small to read or scan.

An AprilTag's size (`[apriltag 20mm:42]`) is the tag size a detector is configured with for pose
estimation: the edge of the black square, not the whole tag with its white surround (in
tagStandard41h12, the outer edge of the white ring). Modules are whole dots, so the nearest
achievable size is printed and `niimctl` reports it, e.g. `tag size 20.02 mm (160 dots, 20 per
module)`. The tag tables come from the [AprilTag library](https://github.com/AprilRobotics/apriltag)
(BSD 2-Clause, see `crates/core/src/label/apriltag_tables.rs`).

- **Label size.** The printers can't report it, so it is looked up from the roll's RFID tag (known:
  B1 50 × 30 mm, D101 40 × 12 mm), falling back to the model's stock label. Otherwise pass
  `--size WIDTHxHEIGHT` in mm, as the label is read.
- **Preview.** `--preview out.png` writes the label instead of printing it; areas that may not
  print are grey. With `--size` or `--model B1` it needs no printer.
- **Fonts.** The built-in font is [Atkinson Hyperlegible](https://www.brailleinstitute.org/freefont/)
  (SIL Open Font License, see `crates/core/fonts/OFL.txt`). `--font FILE.ttf` and `--bold-font`
  use your own.

Library:

```rust
let found = niim_core::discover(Duration::from_secs(3)).await?;
let printer = found[0].connect().await?;
let status = printer.status().await?;                 // lid, paper, battery, error
let mut label = printer.label_canvas(50.0, 30.0);     // mm, as the label is read
label.fill_rect(16, 16, 100, 40, true);
printer.print(&label, &PrintOptions::default()).await?;

// Or from markup (feature `label`, on by default):
use niim_core::label::{Fonts, Label, no_images, render};
let size = printer.detect_label_size().await?.expect("unknown roll");
let label = Label::parse(r"[b]Shelf 4[/b]\nBin 12")?;
let rendered = render(&label, &printer.page(size), &Fonts::builtin(), &no_images)?;
printer.print(&rendered.bitmap, &PrintOptions::default()).await?;
```

Label images are given as the label is read. The driver rotates them for side-feed models (D-series),
places them on the printhead using measured per-model geometry, and keeps the protocol details
(`PROTOCOL.md`) out of sight. Print sequences are implemented for the `B1` family only; verified
hardware: **B1** and **D101** over Bluetooth LE. USB support is written but untested.

## Build requirements

Building `niimctl` should need nothing but `cargo`:

- **macOS: no C at all.** No dependency may need a C compiler, `pkg-config` or development headers.
  Linking libraries that ship with the OS (IOKit, CoreFoundation, CoreBluetooth) is fine.
- **Linux: one exception.** Bluetooth goes through `btleplug`, which talks to BlueZ over D-Bus via
  `libdbus-sys`. That needs `pkg-config` and the libdbus headers:

  ```sh
  sudo apt-get install libdbus-1-dev pkg-config   # Debian/Ubuntu
  sudo dnf install dbus-devel pkgconf-pkg-config  # Fedora
  ```

  Nothing else may need C.
- **Cross-platform.** Low-level crates must support at least Linux and macOS. USB uses `nusb`, which
  is pure Rust. CI builds and tests on both platforms.

The `no-c` CI job enforces this. It fails if `cc`, `cmake`, `bindgen`, `pkg-config`, `vcpkg` or
`system-deps` appears in the dependency graph for macOS, or for Linux outside of `libdbus-sys`.
