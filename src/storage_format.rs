#![allow(dead_code)]

//! The on-flash format is intentionally independent of the ESP HAL so it can
//! be tested on the host as well as used by the firmware.

pub const RECORD_SIZE: usize = 44;
pub const VALUE_COUNT: usize = 9;
pub const ALL_VALUES_MASK: u16 = (1 << VALUE_COUNT) - 1;
pub const FLAG_UTC: u8 = 1 << 0;
const PAYLOAD_LENGTH_SHIFT: u8 = 1;
const PAYLOAD_LENGTH_MASK: u8 = 0x3e;
const MAGIC: u16 = 0x4151;
const VERSION: u8 = 1;
const COMMIT: u32 = 0xc0de_17ed;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Values {
    pub sht41_temperature_centi_c: i16,
    pub sht41_humidity_centi_percent: u16,
    pub scd41_co2_ppm: u16,
    pub scd41_temperature_centi_c: i16,
    pub scd41_humidity_centi_percent: u16,
    pub bmp580_pressure_pa: u32,
    pub bmp580_temperature_centi_c: i16,
    pub sgp41_voc_index: u16,
    pub sgp41_nox_index: u16,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Record {
    pub sequence: u32,
    pub boot_generation: u32,
    pub timestamp: u32,
    pub utc: bool,
    pub status: u8,
    pub fresh_mask: u16,
    pub values: Values,
}

impl Record {
    pub fn encode(&self) -> [u8; RECORD_SIZE] {
        let mut out = [0xff; RECORD_SIZE];
        out[0..2].copy_from_slice(&MAGIC.to_le_bytes());
        out[2] = VERSION;
        out[3] = (if self.utc { FLAG_UTC } else { 0 })
            | (((self.payload_len() as u8) << PAYLOAD_LENGTH_SHIFT) & PAYLOAD_LENGTH_MASK);
        out[4..8].copy_from_slice(&self.sequence.to_le_bytes());
        out[8..12].copy_from_slice(&self.boot_generation.to_le_bytes());
        out[12..16].copy_from_slice(&self.timestamp.to_le_bytes());
        out[16] = self.status;
        out[17..19].copy_from_slice(&(self.fresh_mask & ALL_VALUES_MASK).to_le_bytes());
        self.encode_payload(&mut out[19..38]);
        let crc = crc16(&out[..38]);
        out[38..40].copy_from_slice(&crc.to_le_bytes());
        out[40..44].copy_from_slice(&COMMIT.to_le_bytes());
        out
    }

    pub fn decode(bytes: &[u8; RECORD_SIZE]) -> Option<Self> {
        if u16::from_le_bytes([bytes[0], bytes[1]]) != MAGIC
            || bytes[2] != VERSION
            || u32::from_le_bytes(bytes[40..44].try_into().ok()?) != COMMIT
        {
            return None;
        }
        let expected_crc = u16::from_le_bytes([bytes[38], bytes[39]]);
        if crc16(&bytes[..38]) != expected_crc {
            return None;
        }
        let fresh_mask = u16::from_le_bytes([bytes[17], bytes[18]]);
        if fresh_mask & !ALL_VALUES_MASK != 0 {
            return None;
        }
        let payload_len = (bytes[3] & PAYLOAD_LENGTH_MASK) >> PAYLOAD_LENGTH_SHIFT;
        if payload_len as usize != payload_len_for_mask(fresh_mask) || payload_len as usize > 19 {
            return None;
        }
        Some(Self {
            sequence: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
            boot_generation: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
            timestamp: u32::from_le_bytes(bytes[12..16].try_into().ok()?),
            utc: bytes[3] & FLAG_UTC != 0,
            status: bytes[16],
            fresh_mask,
            values: decode_payload(fresh_mask, &bytes[19..38])?,
        })
    }

    pub fn payload_len(&self) -> usize {
        payload_len_for_mask(self.fresh_mask & ALL_VALUES_MASK)
    }

    fn encode_payload(&self, payload: &mut [u8]) {
        let mut cursor = 0;
        put_i16(
            payload,
            &mut cursor,
            self.fresh_mask,
            0,
            self.values.sht41_temperature_centi_c,
        );
        put_u16(
            payload,
            &mut cursor,
            self.fresh_mask,
            1,
            self.values.sht41_humidity_centi_percent,
        );
        put_u16(
            payload,
            &mut cursor,
            self.fresh_mask,
            2,
            self.values.scd41_co2_ppm,
        );
        put_i16(
            payload,
            &mut cursor,
            self.fresh_mask,
            3,
            self.values.scd41_temperature_centi_c,
        );
        put_u16(
            payload,
            &mut cursor,
            self.fresh_mask,
            4,
            self.values.scd41_humidity_centi_percent,
        );
        if self.fresh_mask & (1 << 5) != 0 {
            let bytes = self.values.bmp580_pressure_pa.to_le_bytes();
            payload[cursor..cursor + 3].copy_from_slice(&bytes[..3]);
            cursor += 3;
        }
        put_i16(
            payload,
            &mut cursor,
            self.fresh_mask,
            6,
            self.values.bmp580_temperature_centi_c,
        );
        put_u16(
            payload,
            &mut cursor,
            self.fresh_mask,
            7,
            self.values.sgp41_voc_index,
        );
        put_u16(
            payload,
            &mut cursor,
            self.fresh_mask,
            8,
            self.values.sgp41_nox_index,
        );
    }
}

fn decode_payload(mask: u16, payload: &[u8]) -> Option<Values> {
    let mut values = Values::default();
    let mut cursor = 0;
    values.sht41_temperature_centi_c = get_i16(payload, &mut cursor, mask, 0)?;
    values.sht41_humidity_centi_percent = get_u16(payload, &mut cursor, mask, 1)?;
    values.scd41_co2_ppm = get_u16(payload, &mut cursor, mask, 2)?;
    values.scd41_temperature_centi_c = get_i16(payload, &mut cursor, mask, 3)?;
    values.scd41_humidity_centi_percent = get_u16(payload, &mut cursor, mask, 4)?;
    if mask & (1 << 5) != 0 {
        let b = payload.get(cursor..cursor + 3)?;
        values.bmp580_pressure_pa = u32::from_le_bytes([b[0], b[1], b[2], 0]);
        cursor += 3;
    }
    values.bmp580_temperature_centi_c = get_i16(payload, &mut cursor, mask, 6)?;
    values.sgp41_voc_index = get_u16(payload, &mut cursor, mask, 7)?;
    values.sgp41_nox_index = get_u16(payload, &mut cursor, mask, 8)?;
    Some(values)
}

fn put_i16(payload: &mut [u8], cursor: &mut usize, mask: u16, bit: usize, value: i16) {
    if mask & (1 << bit) != 0 {
        payload[*cursor..*cursor + 2].copy_from_slice(&value.to_le_bytes());
        *cursor += 2;
    }
}

fn put_u16(payload: &mut [u8], cursor: &mut usize, mask: u16, bit: usize, value: u16) {
    put_i16(payload, cursor, mask, bit, value as i16);
}

fn get_i16(payload: &[u8], cursor: &mut usize, mask: u16, bit: usize) -> Option<i16> {
    if mask & (1 << bit) == 0 {
        return Some(0);
    }
    let bytes = payload.get(*cursor..*cursor + 2)?;
    *cursor += 2;
    Some(i16::from_le_bytes([bytes[0], bytes[1]]))
}

fn get_u16(payload: &[u8], cursor: &mut usize, mask: u16, bit: usize) -> Option<u16> {
    Some(get_i16(payload, cursor, mask, bit)? as u16)
}

fn payload_len_for_mask(mask: u16) -> usize {
    let widths = [2, 2, 2, 2, 2, 3, 2, 2, 2];
    widths
        .iter()
        .enumerate()
        .filter(|(bit, _)| mask & (1 << bit) != 0)
        .map(|(_, width)| *width)
        .sum()
}

fn crc16(bytes: &[u8]) -> u16 {
    let mut crc = 0xffff;
    for &byte in bytes {
        crc ^= byte as u16;
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xa001
            } else {
                crc >> 1
            };
        }
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Record {
        Record {
            sequence: 42,
            boot_generation: 7,
            timestamp: 1_700_000_000,
            utc: true,
            status: 0x0f,
            fresh_mask: ALL_VALUES_MASK,
            values: Values {
                sht41_temperature_centi_c: 2_615,
                sht41_humidity_centi_percent: 6_012,
                scd41_co2_ppm: 542,
                scd41_temperature_centi_c: 2_600,
                scd41_humidity_centi_percent: 5_987,
                bmp580_pressure_pa: 101_325,
                bmp580_temperature_centi_c: 2_630,
                sgp41_voc_index: 103,
                sgp41_nox_index: 88,
            },
        }
    }

    #[test]
    fn round_trip_packs_all_fresh_values() {
        let record = sample();
        assert_eq!(record.payload_len(), 19);
        assert_eq!(Record::decode(&record.encode()), Some(record));
    }

    #[test]
    fn sparse_payload_omits_stale_values() {
        let mut record = sample();
        record.fresh_mask = (1 << 0) | (1 << 5) | (1 << 8);
        let decoded = Record::decode(&record.encode()).unwrap();
        assert_eq!(decoded.values.sht41_temperature_centi_c, 2_615);
        assert_eq!(decoded.values.bmp580_pressure_pa, 101_325);
        assert_eq!(decoded.values.sgp41_nox_index, 88);
        assert_eq!(decoded.values.scd41_co2_ppm, 0);
    }

    #[test]
    fn crc_and_commit_reject_corrupt_or_partial_records() {
        let mut bytes = sample().encode();
        bytes[25] ^= 1;
        assert!(Record::decode(&bytes).is_none());
        let mut bytes = sample().encode();
        bytes[40..44].fill(0xff);
        assert!(Record::decode(&bytes).is_none());
    }

    #[test]
    fn uptime_timestamp_is_distinguishable_from_utc() {
        let mut record = sample();
        record.utc = false;
        assert!(!Record::decode(&record.encode()).unwrap().utc);
    }
}
