use defmt::{info, warn};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::{Duration, Timer};
use embedded_hal_async::i2c::I2c;
use gas_index_algorithm::{AlgorithmType, GasIndexAlgorithm};

const SHT41: u8 = 0x44;
const SGP41: u8 = 0x59;
const SCD41: u8 = 0x62;
const BMP580: u8 = 0x47;

#[derive(Clone, Copy, Default)]
pub struct SensorSnapshot {
    pub sht41_temperature_centi_c: i16,
    pub sht41_humidity_centi_percent: u16,
    pub scd41_co2_ppm: u16,
    pub scd41_temperature_centi_c: i16,
    pub scd41_humidity_centi_percent: u16,
    pub bmp580_pressure_pa: u32,
    pub bmp580_temperature_centi_c: i16,
    pub sgp41_voc_index: u16,
    pub sgp41_nox_index: u16,
    pub status: u8,
}

const SHT41_OK: u8 = 1 << 0;
const SCD41_OK: u8 = 1 << 1;
const BMP580_OK: u8 = 1 << 2;
const SGP41_OK: u8 = 1 << 3;

pub async fn run<I2C>(mut i2c: I2C, updates: &Signal<CriticalSectionRawMutex, SensorSnapshot>)
where
    I2C: I2c,
{
    // No sensor has produced a validated measurement at startup.
    let mut snapshot = SensorSnapshot {
        status: 0,
        ..SensorSnapshot::default()
    };
    let mut scd_started = false;
    let mut bmp_initialized = false;
    let mut sgp_conditioning_cycles = 0u8;
    let mut sgp_initialized = false;
    let mut voc_algorithm = GasIndexAlgorithm::new(AlgorithmType::Voc, 1.0);
    let mut nox_algorithm = GasIndexAlgorithm::new(AlgorithmType::Nox, 1.0);

    // Allow externally powered sensor modules to finish their power-up sequence.
    Timer::after(Duration::from_millis(100)).await;
    info!("sensor polling started: SHT41=0x44 SGP41=0x59 SCD41=0x62 BMP580=0x47");

    loop {
        if let Some(measurement) = sht41_measure(&mut i2c).await {
            snapshot.sht41_temperature_centi_c = measurement.0;
            snapshot.sht41_humidity_centi_percent = measurement.1;
            snapshot.status |= SHT41_OK;
        } else {
            snapshot.status &= !SHT41_OK;
            warn!("SHT41 read failed");
        }

        if !sgp_initialized {
            if sgp41_condition(&mut i2c).await {
                sgp_conditioning_cycles += 1;
                if sgp_conditioning_cycles >= 10 {
                    sgp_initialized = true;
                    info!("SGP41 conditioning complete");
                }
            }
        } else {
            let humidity = if snapshot.status & SHT41_OK != 0 {
                snapshot.sht41_humidity_centi_percent
            } else {
                5_000
            };
            let temperature = if snapshot.status & SHT41_OK != 0 {
                snapshot.sht41_temperature_centi_c
            } else {
                2_500
            };
            if let Some((voc_raw, nox_raw)) = sgp41_measure(&mut i2c, temperature, humidity).await {
                snapshot.sgp41_voc_index = voc_algorithm.process(voc_raw as i32) as u16;
                snapshot.sgp41_nox_index = nox_algorithm.process(nox_raw as i32) as u16;
                snapshot.status |= SGP41_OK;
            } else {
                snapshot.status &= !SGP41_OK;
                warn!("SGP41 read failed");
            }
        }

        if !bmp_initialized {
            bmp_initialized = bmp580_init(&mut i2c).await;
        }
        if bmp_initialized {
            if let Some(measurement) = bmp580_measure(&mut i2c).await {
                snapshot.bmp580_pressure_pa = measurement.0;
                snapshot.bmp580_temperature_centi_c = measurement.1;
                snapshot.status |= BMP580_OK;
            } else {
                snapshot.status &= !BMP580_OK;
                warn!("BMP580 read failed");
            }
        }

        if !scd_started {
            scd_started = scd41_start(&mut i2c).await;
        }
        if scd_started {
            match scd41_ready(&mut i2c).await {
                Some(true) => {
                    if let Some(measurement) = scd41_measure(&mut i2c).await {
                        snapshot.scd41_co2_ppm = measurement.0;
                        snapshot.scd41_temperature_centi_c = measurement.1;
                        snapshot.scd41_humidity_centi_percent = measurement.2;
                        snapshot.status |= SCD41_OK;
                    } else {
                        snapshot.status &= !SCD41_OK;
                        warn!("SCD41 measurement read failed");
                    }
                }
                Some(false) => {}
                None => {
                    snapshot.status &= !SCD41_OK;
                    warn!("SCD41 ready check failed");
                }
            }
        }

        updates.signal(snapshot);
        Timer::after(Duration::from_secs(1)).await;
    }
}

async fn write<I2C>(i2c: &mut I2C, address: u8, bytes: &[u8]) -> Result<(), ()>
where
    I2C: I2c,
{
    match i2c.write(address, bytes).await {
        Ok(()) => Ok(()),
        Err(_) => {
            warn!("I2C write error: address={}", address);
            Err(())
        }
    }
}

async fn write_read<I2C>(
    i2c: &mut I2C,
    address: u8,
    command: &[u8],
    buffer: &mut [u8],
) -> Result<(), ()>
where
    I2C: I2c,
{
    match i2c.write_read(address, command, buffer).await {
        Ok(()) => Ok(()),
        Err(_) => {
            warn!("I2C write_read error: address={}", address);
            Err(())
        }
    }
}

async fn sht41_measure<I2C>(i2c: &mut I2C) -> Option<(i16, u16)>
where
    I2C: I2c,
{
    write(i2c, SHT41, &[0xfd]).await.ok()?;
    Timer::after(Duration::from_millis(10)).await;
    let mut data = [0; 6];
    read(i2c, SHT41, &mut data).await.ok()?;
    if crc8(&data[0..2]) != data[2] || crc8(&data[3..5]) != data[5] {
        warn!("SHT41 CRC error");
        return None;
    }
    Some(decode_sensirion_temperature_humidity(
        u16::from_be_bytes([data[0], data[1]]),
        u16::from_be_bytes([data[3], data[4]]),
    ))
}

async fn sgp41_condition<I2C>(i2c: &mut I2C) -> bool
where
    I2C: I2c,
{
    let args = compensated_args(2_500, 5_000);
    let mut command = [0; 8];
    command[..2].copy_from_slice(&[0x26, 0x12]);
    command[2..].copy_from_slice(&args);
    if write(i2c, SGP41, &command).await.is_err() {
        warn!("SGP41 conditioning command failed");
        return false;
    }
    Timer::after(Duration::from_millis(50)).await;

    // The conditioning command returns one raw VOC word and its CRC. Consume
    // and validate it before issuing the next command.
    let mut data = [0; 3];
    if read(i2c, SGP41, &mut data).await.is_err() {
        warn!("SGP41 conditioning read failed");
        return false;
    }
    if crc8(&data[0..2]) != data[2] {
        warn!("SGP41 conditioning CRC error");
        return false;
    }
    true
}

async fn sgp41_measure<I2C>(i2c: &mut I2C, temperature: i16, humidity: u16) -> Option<(u16, u16)>
where
    I2C: I2c,
{
    let args = compensated_args(temperature, humidity);
    let mut command = [0; 8];
    // SGP41 measure_raw_signals command (the SGP40 command is 0x260F).
    command[..2].copy_from_slice(&[0x26, 0x19]);
    command[2..].copy_from_slice(&args);
    if write(i2c, SGP41, &command).await.is_err() {
        warn!("SGP41 measurement command failed");
        return None;
    }
    Timer::after(Duration::from_millis(50)).await;
    let mut data = [0; 6];
    if read(i2c, SGP41, &mut data).await.is_err() {
        warn!("SGP41 measurement read failed");
        return None;
    }
    if crc8(&data[0..2]) != data[2] || crc8(&data[3..5]) != data[5] {
        warn!("SGP41 measurement CRC error");
        return None;
    }
    Some((
        u16::from_be_bytes([data[0], data[1]]),
        u16::from_be_bytes([data[3], data[4]]),
    ))
}

fn compensated_args(temperature_centi_c: i16, humidity_centi_percent: u16) -> [u8; 6] {
    let temperature =
        ((temperature_centi_c as i32 + 4_500) * 65_535 / 17_500).clamp(0, 65_535) as u16;
    let humidity = (humidity_centi_percent as u32 * 65_535 / 10_000).min(65_535) as u16;
    let [t_hi, t_lo] = temperature.to_be_bytes();
    let [h_hi, h_lo] = humidity.to_be_bytes();
    [
        h_hi,
        h_lo,
        crc8(&[h_hi, h_lo]),
        t_hi,
        t_lo,
        crc8(&[t_hi, t_lo]),
    ]
}

async fn scd41_start<I2C>(i2c: &mut I2C) -> bool
where
    I2C: I2c,
{
    // The SCD41 can keep measuring while the ESP32 is reset. Stop an existing
    // session first; an error here is expected when the sensor was idle.
    if write(i2c, SCD41, &[0x3f, 0x86]).await.is_ok() {
        Timer::after(Duration::from_millis(500)).await;
    }

    if write(i2c, SCD41, &[0x21, 0xb1]).await.is_ok() {
        info!("SCD41 periodic measurement started");
        true
    } else {
        warn!("SCD41 start command failed");
        false
    }
}

async fn scd41_ready<I2C>(i2c: &mut I2C) -> Option<bool>
where
    I2C: I2c,
{
    let mut data = [0; 3];
    write_read(i2c, SCD41, &[0xe4, 0xb8], &mut data)
        .await
        .ok()?;
    if crc8(&data[0..2]) != data[2] {
        warn!("SCD41 ready CRC error");
        return None;
    }
    Some(u16::from_be_bytes([data[0], data[1]]) & 0x07ff != 0)
}

async fn scd41_measure<I2C>(i2c: &mut I2C) -> Option<(u16, i16, u16)>
where
    I2C: I2c,
{
    let mut data = [0; 9];
    write_read(i2c, SCD41, &[0xec, 0x05], &mut data)
        .await
        .ok()?;
    if crc8(&data[0..2]) != data[2] || crc8(&data[3..5]) != data[5] || crc8(&data[6..8]) != data[8]
    {
        warn!("SCD41 measurement CRC error");
        return None;
    }
    let co2 = u16::from_be_bytes([data[0], data[1]]);
    let (temperature, humidity) = decode_sensirion_temperature_humidity(
        u16::from_be_bytes([data[3], data[4]]),
        u16::from_be_bytes([data[6], data[7]]),
    );
    Some((co2, temperature, humidity))
}

async fn bmp580_init<I2C>(i2c: &mut I2C) -> bool
where
    I2C: I2c,
{
    let mut chip_id = [0; 1];
    if write_read(i2c, BMP580, &[0x01], &mut chip_id)
        .await
        .is_err()
    {
        return false;
    }
    if chip_id[0] != 0x50 {
        warn!("BMP580 unexpected chip id");
        return false;
    }
    if write(i2c, BMP580, &[0x37, 0x03]).await.is_err() {
        return false;
    }
    Timer::after(Duration::from_millis(10)).await;
    info!("BMP580 initialized");
    true
}

async fn bmp580_measure<I2C>(i2c: &mut I2C) -> Option<(u32, i16)>
where
    I2C: I2c,
{
    let mut data = [0; 6];
    write_read(i2c, BMP580, &[0x1d], &mut data).await.ok()?;
    Some(decode_bmp580(data))
}

fn decode_sensirion_temperature_humidity(temperature_raw: u16, humidity_raw: u16) -> (i16, u16) {
    let temperature = -4_500i32 + (17_500i32 * temperature_raw as i32) / 65_535;
    let humidity = -6_000i32 + (12_500i32 * humidity_raw as i32) / 65_535;
    (temperature as i16, humidity.clamp(0, 10_000) as u16)
}

fn decode_bmp580(data: [u8; 6]) -> (u32, i16) {
    // BMP580 exposes temperature first (0x1D..0x1F), followed by pressure
    // (0x20..0x22). Temperature is signed Q8.16 °C and pressure is Q18.6 Pa.
    let temperature_raw = i32::from_le_bytes([
        data[0],
        data[1],
        data[2],
        if data[2] & 0x80 != 0 { 0xff } else { 0 },
    ]);
    let pressure_raw = u32::from_le_bytes([data[3], data[4], data[5], 0]);
    (pressure_raw / 64, (temperature_raw * 100 / 65_536) as i16)
}

async fn read<I2C>(i2c: &mut I2C, address: u8, buffer: &mut [u8]) -> Result<(), ()>
where
    I2C: I2c,
{
    match i2c.read(address, buffer).await {
        Ok(()) => Ok(()),
        Err(_) => {
            warn!("I2C read error: address={}", address);
            Err(())
        }
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0xff;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::{crc8, decode_bmp580, decode_sensirion_temperature_humidity};

    #[test]
    fn sensirion_crc_matches_reference() {
        assert_eq!(crc8(&[0xbe, 0xef]), 0x92);
    }

    #[test]
    fn sensirion_fixed_point_conversion_is_scaled_and_clamped() {
        assert_eq!(decode_sensirion_temperature_humidity(0, 0), (-4_500, 0));
        assert_eq!(
            decode_sensirion_temperature_humidity(u16::MAX, u16::MAX),
            (13_000, 10_000)
        );
    }

    #[test]
    fn bmp580_register_order_and_scaling_are_correct() {
        // 25 °C as signed Q8.16, followed by 100,000 Pa as Q18.6.
        assert_eq!(
            decode_bmp580([0x00, 0x00, 0x19, 0x00, 0xa8, 0x61]),
            (100_000, 2_500)
        );
    }
}
