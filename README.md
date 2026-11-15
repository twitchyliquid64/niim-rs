# niim-rs

A pure-Rust userland driver for NIIMBOT label makers, with an async Rust programming interface.

## Crate map

| Crate     | Role |
|-----------|------|
| `niim-core` (`crates/core`) | The library: device discovery, the printer protocol and the async printing API. |
| `niimctl` (`crates/niimctl`) | A small command-line frontend over `niim-core`. |

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
