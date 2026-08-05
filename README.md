# AirQuality

AirQuality is an ESP32-C6 environmental sensor project with persistent history
storage, a WebBluetooth dashboard, and an accompanying KiCad board design.

## Components

- [`firmware/`](firmware/) — embedded Rust firmware for the ESP32-C6. It reads
  the connected environmental sensor modules, exposes live values and history
  over BLE, and controls the RGB LED.
- [`dashboard/`](dashboard/) — dependency-free standalone WebBLE dashboards for
  live sensor values and stored history. Run them from localhost in a browser
  with WebBluetooth support.
- [`hardware/`](hardware/) — KiCad project sources, local symbols and
  footprints, schematic documentation, and the committed fabrication archive.

## Quick start

### Firmware

```text
cd firmware
cargo embed
```

The default workflow uses a debug build and the ESP32-C6 USB-JTAG interface.
See [`firmware/README.md`](firmware/README.md) for sensor, BLE, storage, and
flashing details.

### Dashboards

Serve the dashboard directory from localhost, then open either HTML page in a
WebBluetooth-capable browser:

```text
cd dashboard
python3 -m http.server 8000
```

- [`air-quality.html`](dashboard/air-quality.html) — live sensor dashboard.
- [`air-quality-history.html`](dashboard/air-quality-history.html) — persistent
  history controls, retrieval, graphing, and CSV export.

### Hardware

Open [`hardware/AirQuality.kicad_pro`](hardware/AirQuality.kicad_pro)
with KiCad. The project README documents the module interconnects and layout
considerations. The fabrication archive and generated schematic PDF are in
[`hardware/fabrication/`](hardware/fabrication/).
