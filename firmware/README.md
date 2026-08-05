# AirQuality ESP32-C6 Firmware

Firmware for the ESP32-C6 air-quality board described by the KiCad project in
[`../hardware/`](../hardware/). It
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

The flash layout guard is part of the debug link and fails the build if the
firmware image grows into the persistent-storage start address. The compact
record codec can be tested on the host without selecting the embedded target:

```text
rustc --edition=2024 --test src/storage_format.rs -o /tmp/air-quality-storage-format-tests
/tmp/air-quality-storage-format-tests
```

## BLE GATT service

The device advertises as `AirQuality`. Sensor characteristics are readable and
support notifications. Values use little-endian fixed-point encodings so they
can be consumed without floating-point support on a client.

Both humidity characteristics report relative humidity in %RH × 100. The
firmware applies the SHT41 and SCD41 transfer functions separately; neither is
an absolute-humidity value.

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

### Persistent sensor history service

The firmware includes an optional persistent history service. It is disabled
by default, and the enabled/disabled setting survives a reboot. Disabling it
does not delete existing records; deletion requires the explicit erase command.
The service UUID is:

```text
7c3e3000-4e0a-4a69-9c57-6b5f3f1c0000
```

| Characteristic | UUID suffix | Properties | Payload |
| --- | --- | --- | --- |
| Storage control | `3001` | Read, Write | Exactly one byte: `0` disabled, `1` enabled |
| UTC clock sync | `3002` | Write | Little-endian `u32` Unix seconds |
| Storage metadata | `3003` | Read | 24-byte little-endian structure described below |
| History request | `3004` | Write | Cursor `u32` LE followed by maximum count `u16` LE |
| History data | `3005` | Read, Notify | Chunk structure described below |
| Erase history | `3006` | Write | Exactly the five ASCII bytes `ERASE` |

The full characteristic UUID is formed by inserting the suffix into the
service namespace, for example `7c3e3001-4e0a-4a69-9c57-6b5f3f1c0000` for
storage control.

The metadata payload is:

```text
byte  0       enabled (0/1)
byte  1       clock synchronized (0/1)
bytes 2..5   record count, u32 LE
bytes 6..9   oldest sequence, u32 LE
bytes 10..13 newest sequence, u32 LE
byte  14      oldest timestamp is UTC (0/1)
byte  15      newest timestamp is UTC (0/1)
bytes 16..19 oldest timestamp, u32 LE
bytes 20..23 newest timestamp, u32 LE
```

To retrieve history, write a six-byte request. Use cursor `0xffffffff` to
start at the oldest available record; otherwise the cursor is the first
sequence number to return. The implementation returns at most five records
per chunk so it remains within the current 251-byte packet pool. The history
data payload starts with a four-byte next cursor and one-byte record count,
followed by `count` entries. Each entry is one length byte (currently `44`)
and one serialized record. A zero next cursor means that no record was
returned. Clients can request the returned next cursor to resume retrieval.

Each serialized record is little-endian and contains a format/version marker,
flags, sequence, boot generation, timestamp, sensor status, fresh-value mask,
packed values, CRC16, and a final commit marker. The timestamp is Unix UTC
seconds when the clock-synchronized flag is set; otherwise it is monotonic
uptime seconds for the persisted boot generation. The nine packed fields are,
in order: SHT41 temperature, SHT41 relative humidity, SCD41 CO₂, SCD41
temperature, SCD41 relative humidity, BMP580 pressure, BMP580 temperature,
SGP41 VOC index, and SGP41 NOx index. The fresh mask identifies which fields
are present in that record, so a failed sensor does not cause an old value to
be repeated.

History is aggregated at one-minute boundaries. Only fields with at least one
successful fresh reading in the minute are written; a completely unsuccessful
minute produces no record. A clock sync affects records created after the
sync, and a reboot requires synchronization again.

For the configured 4 MiB flash, the upper `0x200000..0x400000` region is
reserved for storage. Two sectors hold redundant configuration records and
the remaining sectors form a circular log of approximately 30 days at one
record per minute. Flash reads and writes are owned by the storage task, so BLE
handlers do not access flash concurrently with sensor aggregation.

## Logging and troubleshooting

Use `cargo embed` to flash, reset, and view logs. Normal startup reports the configured
pins, sensor addresses, BLE advertising, and successful sensor initialization.
Repeated sensor warnings indicate an I²C wiring, power, address, or pull-up
problem; they do not indicate a firmware-wide failure.

The board should use 3.3 V I²C pull-ups and a common ground. Verify the exact
WS2818B supply and logic-level requirements for the installed module before
connecting an externally powered LED.
