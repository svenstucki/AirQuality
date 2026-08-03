# AirQuality ESP32-C6 Firmware

Firmware for the ESP32-C6 air-quality board described by the KiCad project. It
reads the environmental sensors over I²C, publishes the latest values over BLE
GATT, and controls the WS2818B RGB LED.

## Hardware

| Function | ESP32-C6 GPIO |
| --- | ---: |
| I²C SDA | GPIO6 |
| I²C SCL | GPIO7 |
| WS2818B DIN | GPIO8 |

The I²C bus is configured for 400 kHz and uses the 7-bit addresses from the
hardware README:

| Sensor | Address | Default sampling behavior |
| --- | ---: | --- |
| SHT41 temperature/humidity | `0x44` | Every second |
| SGP41 VOC/NOx | `0x59` | Every second |
| SCD41 CO₂ | `0x62` | Periodic measurement, approximately every five seconds |
| BMP580 pressure/temperature | `0x47` | Every second |
| DS3231 RTC | `0x68` | Not used yet |

Sensor failures are non-fatal. The firmware logs the failure with `defmt`,
keeps the last valid value, clears the corresponding BLE status bit, and tries
again on the next scheduled read.

## Development setup

Install the ESP Rust target and probe-rs tools:

```text
rustup target add riscv32imac-unknown-none-elf
cargo install probe-rs-tools --locked
```

The project uses `esp-hal`, Embassy, `esp-radio`, and Trouble. The ESP32-C6 can
use its built-in USB-JTAG-Serial interface as a probe-rs debug probe. Connect
the board, then flash and monitor with:

```text
cargo embed
```

Debug builds are the default for this project; use a release build only when
explicitly requested.

`Embed.toml` enables RTT and decodes channel 0 as defmt. The default log level
is `info`; override it for a one-off build with `DEFMT_LOG=debug` or by using a
local embed configuration.

The installed `cargo-embed` binary must be able to load `libusb` on macOS. If
it reports a missing `libusb-1.0.0.dylib`, repair the local probe-rs/libusb
installation before flashing.

## BLE GATT service

The device advertises as `AirQuality`. Sensor characteristics are readable and
support notifications. Values use little-endian fixed-point encodings so they
can be consumed without floating-point support on a client.

### Air-quality service

Service UUID:

```text
7c3e1000-4e0a-4a69-9c57-6b5f3f1c0000
```

| Characteristic | UUID | Properties | Encoding |
| --- | --- | --- | --- |
| SHT41 temperature | `7c3e1001-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Signed 16-bit °C × 100 |
| SHT41 humidity | `7c3e1002-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Unsigned 16-bit %RH × 100 |
| SCD41 CO₂ | `7c3e1003-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Unsigned 16-bit ppm |
| SCD41 temperature | `7c3e1004-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Signed 16-bit °C × 100 |
| SCD41 humidity | `7c3e1005-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Unsigned 16-bit %RH × 100 |
| BMP580 pressure | `7c3e1006-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Unsigned 32-bit Pa |
| BMP580 temperature | `7c3e1007-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Signed 16-bit °C × 100 |
| SGP41 VOC index | `7c3e1008-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Unsigned 16-bit index, 0–500 |
| SGP41 NOx index | `7c3e1009-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Unsigned 16-bit index, 0–500 |
| Sensor status | `7c3e100a-4e0a-4a69-9c57-6b5f3f1c0000` | Read, Notify | Bitmask; bit 0 SHT41, bit 1 SCD41, bit 2 BMP580, bit 3 SGP41 |

A status bit is set when that sensor has produced a valid current value. A
cleared bit means the value may be stale or unavailable. SGP41 indices remain
zero during the algorithm's initial warm-up period, and its status bit remains
clear until both VOC and NOx indices are nonzero.

### RGB LED service

Service UUID:

```text
7c3e2000-4e0a-4a69-9c57-6b5f3f1c0000
```

The color characteristic is readable, writable with or without response, and
notifiable:

```text
7c3e2001-4e0a-4a69-9c57-6b5f3f1c0000
```

Its payload is exactly three bytes `[R, G, B]`. `[0, 0, 0]` turns the LED off.
The firmware converts this logical RGB order to the WS2818B's wire order
(GRB). Brightness is not a separate field; clients can scale the three color
bytes before writing them.

## Logging and troubleshooting

Use `cargo embed` to flash, reset, and view logs. Normal startup reports the configured
pins, sensor addresses, BLE advertising, and successful sensor initialization.
Repeated sensor warnings indicate an I²C wiring, power, address, or pull-up
problem; they do not indicate a firmware-wide failure.

The board should use 3.3 V I²C pull-ups and a common ground. Verify the exact
WS2818B supply and logic-level requirements for the installed module before
connecting an externally powered LED.
