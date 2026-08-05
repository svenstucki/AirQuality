# AirQuality KiCad Project

KiCad 9 PCB project for an ESP32-C6-based air-quality sensor board. The board
combines several prebuilt modules on one PCB and connects them through one shared
3.3 V I2C bus.

## Current Project State

- `AirQuality.kicad_pro`: KiCad 9 project file.
- `AirQuality.kicad_sch`: root schematic with all module placeholders placed,
  shared I2C wiring, power symbols, and I2C pull-up resistors.
- `AirQuality.kicad_pcb`: PCB layout for the module-based board.

Future agents should treat this as an early-stage hardware project: capture the
module interconnects first, then create or assign verified footprints from the
actual modules in hand.

## Modules

| Module | Role | Interface | Expected I2C address |
| --- | --- | --- | --- |
| Nano ESP32-C6 development board | MCU, Wi-Fi/BLE/802.15.4 controller | I2C master | N/A |
| DS3231 RTC module for Raspberry Pi, 5-pin header | Real-time clock | I2C | `0x68` |
| SGP41 module | VOC/NOx gas sensor | I2C | `0x59` |
| SCD41 module | CO2 sensor | I2C | `0x62` |
| SHT41 module | Temperature/humidity sensor | I2C | `0x44` |
| BMP580 module | Barometric pressure sensor | I2C | `0x47` default, `0x46` selectable |

There are no address conflicts in this set.

## Project Libraries

The project has local placeholder libraries at the project root:

- `AirQuality.kicad_sym`: schematic symbols for each module.
- `AirQuality.pretty/`: footprint library with matching placeholder footprints.
- `sym-lib-table` and `fp-lib-table`: project-local KiCad library tables.

These placeholders intentionally model only the module pin-header blocks. Keep
using them until the exact physical module dimensions, outlines, and mounting
details are verified.

The DS3231 RTC symbol intentionally exposes only four functional schematic pins,
but its assigned footprint is a 1x05 header. Physical header pin 4 is present and
left unconnected; the symbol maps its fourth functional pin to footprint pad 5.

## Electrical Intent

- Use one shared bus: `I2C_SDA`, `I2C_SCL`, `3V3`, and `GND`.
- Keep all I2C pull-ups tied to `3V3`, not `5V`.
- The schematic includes `R1` and `R2` as `4.7k` pull-ups from `I2C_SDA` and
  `I2C_SCL` to `3V3` using 0603 SMD footprints.
- Check each module for already-populated pull-up resistors. Multiple breakout
  boards often each include pull-ups, which can make the effective pull-up too
  strong when combined.
- Observed module pull-ups:

  | Module | Populated I2C pull-ups |
  | --- | --- |
  | DS3231 RTC module | `4.3k` on SDA and SCL |
  | SHT41 temperature/humidity module | `10k` on SDA and SCL |
  | SCD41 CO2 module | `10k` on SDA and SCL |
  | BMP580 pressure module | `10k` on SDA and SCL |
  | SGP41 gas module | none observed |

  With these modules installed, the effective bus pull-up is about `1.9k` per
  line before adding schematic `R1`/`R2`; treat `R1` and `R2` as DNP unless the
  module pull-ups are removed or the bus rise time proves too slow.
- Target standard-mode or fast-mode I2C. The DS3231 supports fast-mode I2C
  operation; sensor breakouts should still be checked against their module
  documentation.
- Prefer powering every module from `3V3` unless the exact module documentation
  proves that another supply arrangement is required and does not pull SDA/SCL
  above 3.3 V.
- Do not add `PWR_FLAG` symbols just to silence ERC in this placeholder
  schematic. The modules are powered from external/module headers for now, so
  ignore ERC `power_pin_not_driven` messages until a real onboard power source or
  regulator is added.

## ESP32-C6 Pin Notes

The local Nano ESP32-C6 symbol and footprint use side-aware KiCad pin/pad
numbers: `L1`-`L15` for the left side top-to-bottom and `R1`-`R15` for the right
side top-to-bottom. This avoids duplicate `1`-`15` pin numbers while matching
the module pin tables.

The ESP32-C6 has low-power I2C functions on GPIO6/GPIO7 in Espressif reference
development boards. The Nano ESP32-C6 Tindie listing says I2C is broken out, but
the exact Nano board pin labels and header geometry still need to be verified
from the seller documentation or by measuring the board before assigning the
footprint.

Do not route the PCB from assumptions about Arduino-style labels alone. Confirm:

- Which Nano ESP32-C6 pins expose SDA and SCL.
- Header pitch, row spacing, board outline, USB-C keepout, antenna keepout, and
  mounting constraints.
- Whether the Nano board regulator can supply all attached modules with enough
  margin, especially during wireless transmit current peaks.

## Schematic Guidance

Use connector or module symbols rather than bare IC symbols unless the design is
changed to place the sensors directly on this PCB.

Suggested net names:

- `3V3`
- `GND`
- `I2C_SDA`
- `I2C_SCL`
- Optional: `RTC_INT` or `RTC_SQW` if the DS3231 alarm/square-wave output is
  used later.

Add test points for `3V3`, `GND`, `I2C_SDA`, and `I2C_SCL` if there is room.

## PCB Layout Guidance

- Place the ESP32-C6 antenna area at a board edge with a copper/component keepout
  matching the Nano board vendor guidance.
- Keep environmental sensors away from heat sources such as regulators, USB power
  circuitry, and the ESP32-C6 module when practical.
- Give SCD41, SGP41, SHT41, and BMP580 modules exposure to ambient air. Avoid
  burying them under the MCU board or next to tall connectors that block airflow.
- Keep I2C traces short and direct. This board is module-to-module, so routing is
  expected to be low-speed and uncomplicated.
- Add silkscreen labels for module orientation and the four shared bus pins.

## Source References

- Nano ESP32-C6 development board:
  https://www.tindie.com/products/johnnywu/nanoesp32-c6-development-board/
- ESP32-C6 reference pin functions:
  https://docs.espressif.com/projects/esp-dev-kits/en/latest/esp32c6/esp32-c6-devkitc-1/user_guide.html
- DS3231 product page and datasheet:
  https://www.analog.com/en/products/ds3231.html
- Sensirion SGP41:
  https://sensirion.com/products/catalog/SGP41
- Sensirion SCD4x embedded driver address definition:
  https://github.com/Sensirion/embedded-i2c-scd4x
- Sensirion SHT41:
  https://sensirion.com/products/catalog/SHT41
- Bosch BMP580:
  https://www.bosch-sensortec.com/en/products/environmental-sensors/pressure-sensors/bmp580
- Adafruit BMP58xx breakout pinout notes:
  https://learn.adafruit.com/adafruit-bmp580-bmp581-and-bmp585-temperature-and-pressure-sensor/pinouts

## Next Steps

1. Verify the exact physical modules and capture measured dimensions.
2. Verify whether any module breakout already provides I2C pull-ups; remove or
   mark `R1`/`R2` DNP if the combined pull-up is too strong.
3. Create or import verified footprints for the exact module boards.
4. Update the PCB from the schematic and place the module footprints.
5. Add board outline, mounting holes, antenna keepout, and airflow-conscious
   placement.
6. Run KiCad ERC/DRC before producing fabrication outputs.
