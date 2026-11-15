# niim-rs

A pure-Rust userland driver for NIIMBOT label makers, with an async Rust programming interface.

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
```

`-p` matches any part of the printer name (`MODEL-SERIAL`). Set `RUST_LOG=niim_core=trace` to see every packet.

Library:

```rust
let found = niim_core::discover(Duration::from_secs(3)).await?;
let printer = found[0].connect().await?;
let status = printer.status().await?;                 // lid, paper, battery, error
let mut label = printer.label_canvas(50.0, 30.0);     // mm, as the label is read
label.fill_rect(16, 16, 100, 40, true);
printer.print(&label, &PrintOptions::default()).await?;
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
