use defmt::{info, warn};
use embassy_futures::select::{Either3, select3};
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, channel::Receiver, signal::Signal,
};
use embassy_time::{Duration, Instant, Timer};
use embedded_storage::nor_flash::{NorFlash, ReadNorFlash};
use esp_storage::FlashStorage;

use crate::{
    sensors::SensorUpdate,
    storage_format::{RECORD_SIZE, Record, Values},
};

pub const STORAGE_START: u32 = 0x0020_0000;
pub const STORAGE_END: u32 = 0x0040_0000;
const SECTOR_SIZE: u32 = 4096;
const CONFIG_SECTORS: u32 = 2;
const RING_START: u32 = STORAGE_START + CONFIG_SECTORS * SECTOR_SIZE;
const SECTOR_SLOTS: u32 = SECTOR_SIZE / RECORD_SIZE as u32;
const RING_SECTORS: u32 = (STORAGE_END - RING_START) / SECTOR_SIZE;
const RING_CAPACITY: u32 = RING_SECTORS * SECTOR_SLOTS;
const CONFIG_MAGIC: u16 = 0x4341;
const CONFIG_VERSION: u8 = 1;
const CONFIG_COMMIT: u32 = 0xcafe_babe;
pub const ERASE_TOKEN: [u8; 5] = *b"ERASE";
const HISTORY_MAX_RECORDS: usize = 5;
const METADATA_SIZE: usize = 24;
const HISTORY_DATA_SIZE: usize = 244;

const _: () = {
    assert!(STORAGE_START % SECTOR_SIZE == 0);
    assert!(STORAGE_END % SECTOR_SIZE == 0);
    assert!(STORAGE_START < STORAGE_END);
    assert!(STORAGE_END <= 0x0040_0000);
};

#[derive(Clone, Copy)]
pub enum StorageCommand {
    SetEnabled(bool),
    SyncClock(u32),
    ReadMetadata,
    ReadHistory { cursor: u32, max_records: u16 },
    Erase([u8; 5]),
}

#[derive(Clone, Copy)]
pub enum StorageResponse {
    Metadata {
        data: [u8; METADATA_SIZE],
    },
    History {
        data: [u8; HISTORY_DATA_SIZE],
        len: u16,
    },
    Ack,
}

#[derive(Clone, Copy, Default)]
struct Config {
    enabled: bool,
    generation: u32,
    boot_generation: u32,
}

impl Config {
    fn encode(&self) -> [u8; 32] {
        let mut out = [0xff; 32];
        out[0..2].copy_from_slice(&CONFIG_MAGIC.to_le_bytes());
        out[2] = CONFIG_VERSION;
        out[3] = self.enabled as u8;
        out[4..8].copy_from_slice(&self.generation.to_le_bytes());
        out[8..12].copy_from_slice(&self.boot_generation.to_le_bytes());
        let crc = crc16(&out[..12]);
        out[12..14].copy_from_slice(&crc.to_le_bytes());
        out[28..32].copy_from_slice(&CONFIG_COMMIT.to_le_bytes());
        out
    }

    fn decode(bytes: &[u8; 32]) -> Option<Self> {
        if u16::from_le_bytes([bytes[0], bytes[1]]) != CONFIG_MAGIC
            || bytes[2] != CONFIG_VERSION
            || u32::from_le_bytes(bytes[28..32].try_into().ok()?) != CONFIG_COMMIT
            || crc16(&bytes[..12]) != u16::from_le_bytes([bytes[12], bytes[13]])
            || bytes[3] > 1
        {
            return None;
        }
        Some(Self {
            enabled: bytes[3] != 0,
            generation: u32::from_le_bytes(bytes[4..8].try_into().ok()?),
            boot_generation: u32::from_le_bytes(bytes[8..12].try_into().ok()?),
        })
    }
}

#[derive(Clone, Copy)]
struct Clock {
    unix_at_sync: u32,
    monotonic_at_sync: u64,
}

#[derive(Clone, Copy)]
struct Ring {
    next_slot: u32,
    count: u32,
    oldest_sequence: u32,
    newest_sequence: u32,
    oldest_timestamp: u32,
    newest_timestamp: u32,
    oldest_utc: bool,
    newest_utc: bool,
}

impl Ring {
    fn empty() -> Self {
        Self {
            next_slot: 0,
            count: 0,
            oldest_sequence: 0,
            newest_sequence: 0,
            oldest_timestamp: 0,
            newest_timestamp: 0,
            oldest_utc: false,
            newest_utc: false,
        }
    }

    fn address(slot: u32) -> u32 {
        RING_START
            + (slot / SECTOR_SLOTS) * SECTOR_SIZE
            + (slot % SECTOR_SLOTS) * RECORD_SIZE as u32
    }

    fn scan(flash: &mut FlashStorage<'static>) -> Self {
        let mut ring = Self::empty();
        let mut newest_slot = 0;
        let mut newest: Option<Record> = None;
        let mut oldest: Option<Record> = None;
        let mut bytes = [0u8; RECORD_SIZE];
        for slot in 0..RING_CAPACITY {
            if flash.read(Self::address(slot), &mut bytes).is_err() {
                warn!("storage flash read failed while scanning slot {}", slot);
                continue;
            }
            if let Some(record) = Record::decode(&bytes) {
                ring.count = ring.count.saturating_add(1).min(RING_CAPACITY);
                if newest.map(|r| record.sequence > r.sequence).unwrap_or(true) {
                    newest = Some(record);
                    newest_slot = slot;
                }
                if oldest.map(|r| record.sequence < r.sequence).unwrap_or(true) {
                    oldest = Some(record);
                }
            }
        }
        if let Some(record) = newest {
            ring.next_slot = (newest_slot + 1) % RING_CAPACITY;
            ring.newest_sequence = record.sequence;
            ring.newest_timestamp = record.timestamp;
            ring.newest_utc = record.utc;
        }
        if let Some(record) = oldest {
            ring.oldest_sequence = record.sequence;
            ring.oldest_timestamp = record.timestamp;
            ring.oldest_utc = record.utc;
        }
        ring
    }

    fn refresh_stats(&mut self, flash: &mut FlashStorage<'static>) {
        let mut current = Self::scan(flash);
        current.next_slot = self.next_slot;
        *self = current;
    }

    fn append(&mut self, flash: &mut FlashStorage<'static>, record: Record) -> Result<(), ()> {
        let slot = self.next_slot;
        let address = Self::address(slot);
        let sector_start = RING_START + (slot / SECTOR_SLOTS) * SECTOR_SIZE;
        let mut existing = [0u8; RECORD_SIZE];
        if flash.read(address, &mut existing).is_err() {
            return Err(());
        }
        if existing.iter().any(|byte| *byte != 0xff) {
            if flash
                .erase(sector_start, sector_start + SECTOR_SIZE)
                .is_err()
            {
                return Err(());
            }
        }
        let bytes = record.encode();
        // The first 40 bytes contain the complete payload and CRC. The final
        // four-byte commit is written separately, so an interrupted write is
        // ignored during recovery.
        if flash.write(address, &bytes[..40]).is_err()
            || flash.write(address + 40, &bytes[40..44]).is_err()
        {
            return Err(());
        }
        self.next_slot = (slot + 1) % RING_CAPACITY;
        self.refresh_stats(flash);
        Ok(())
    }
}

struct Bucket {
    minute: u64,
    values: Values,
    fresh_mask: u16,
    status: u8,
}

impl Bucket {
    fn new(minute: u64) -> Self {
        Self {
            minute,
            values: Values::default(),
            fresh_mask: 0,
            status: 0,
        }
    }

    fn update(&mut self, update: SensorUpdate) {
        let v = update.snapshot;
        let mask = update.fresh_mask;
        self.status = v.status;
        if mask & (1 << 0) != 0 {
            self.values.sht41_temperature_centi_c = v.sht41_temperature_centi_c;
        }
        if mask & (1 << 1) != 0 {
            self.values.sht41_humidity_centi_percent = v.sht41_humidity_centi_percent;
        }
        if mask & (1 << 2) != 0 {
            self.values.scd41_co2_ppm = v.scd41_co2_ppm;
        }
        if mask & (1 << 3) != 0 {
            self.values.scd41_temperature_centi_c = v.scd41_temperature_centi_c;
        }
        if mask & (1 << 4) != 0 {
            self.values.scd41_humidity_centi_percent = v.scd41_humidity_centi_percent;
        }
        if mask & (1 << 5) != 0 {
            self.values.bmp580_pressure_pa = v.bmp580_pressure_pa;
        }
        if mask & (1 << 6) != 0 {
            self.values.bmp580_temperature_centi_c = v.bmp580_temperature_centi_c;
        }
        if mask & (1 << 7) != 0 {
            self.values.sgp41_voc_index = v.sgp41_voc_index;
        }
        if mask & (1 << 8) != 0 {
            self.values.sgp41_nox_index = v.sgp41_nox_index;
        }
        self.fresh_mask |= mask;
    }
}

pub async fn run(
    mut flash: FlashStorage<'static>,
    updates: Receiver<'static, CriticalSectionRawMutex, SensorUpdate, 4>,
    commands: Receiver<'static, CriticalSectionRawMutex, StorageCommand, 4>,
    responses: &'static Signal<CriticalSectionRawMutex, StorageResponse>,
) -> ! {
    let mut config = load_config(&mut flash);
    config.boot_generation = config.boot_generation.wrapping_add(1).max(1);
    config.generation = config.generation.wrapping_add(1).max(1);
    save_config(&mut flash, config);
    let mut ring = Ring::scan(&mut flash);
    let mut clock = None;
    let mut bucket = Bucket::new(Instant::now().as_secs() / 60);
    info!(
        "storage ready: enabled={} records={}",
        config.enabled, ring.count
    );

    loop {
        match select3(
            updates.receive(),
            commands.receive(),
            Timer::after(Duration::from_secs(1)),
        )
        .await
        {
            Either3::First(update) => {
                let minute = Instant::now().as_secs() / 60;
                if config.enabled {
                    if minute != bucket.minute {
                        flush_bucket(
                            &mut flash,
                            &mut ring,
                            &bucket,
                            config.boot_generation,
                            clock,
                        );
                        bucket = Bucket::new(minute);
                    }
                    bucket.update(update);
                }
            }
            Either3::Second(command) => {
                handle_command(
                    command,
                    &mut flash,
                    &mut ring,
                    &mut config,
                    &mut bucket,
                    &mut clock,
                    responses,
                );
            }
            Either3::Third(_) => {
                let minute = Instant::now().as_secs() / 60;
                if config.enabled && minute != bucket.minute {
                    flush_bucket(
                        &mut flash,
                        &mut ring,
                        &bucket,
                        config.boot_generation,
                        clock,
                    );
                    bucket = Bucket::new(minute);
                }
            }
        }
    }
}

fn handle_command(
    command: StorageCommand,
    flash: &mut FlashStorage<'static>,
    ring: &mut Ring,
    config: &mut Config,
    bucket: &mut Bucket,
    clock: &mut Option<Clock>,
    responses: &Signal<CriticalSectionRawMutex, StorageResponse>,
) {
    match command {
        StorageCommand::SetEnabled(enabled) => {
            config.enabled = enabled;
            config.generation = config.generation.wrapping_add(1).max(1);
            save_config(flash, *config);
            bucket.fresh_mask = 0;
            bucket.minute = Instant::now().as_secs() / 60;
            responses.signal(StorageResponse::Ack);
            info!("storage {}", if enabled { "enabled" } else { "disabled" });
        }
        StorageCommand::SyncClock(unix_seconds) => {
            *clock = Some(Clock {
                unix_at_sync: unix_seconds,
                monotonic_at_sync: Instant::now().as_secs(),
            });
            responses.signal(StorageResponse::Ack);
            info!("storage clock synchronized: unix={}", unix_seconds);
        }
        StorageCommand::ReadMetadata => responses.signal(StorageResponse::Metadata {
            data: metadata(*config, clock.is_some(), *ring),
        }),
        StorageCommand::ReadHistory {
            cursor,
            max_records,
        } => {
            responses.signal(StorageResponse::History {
                data: history(flash, ring, cursor, max_records),
                len: history_len(flash, ring, cursor, max_records),
            });
        }
        StorageCommand::Erase(token) => {
            if token == ERASE_TOKEN && erase_history(flash).is_ok() {
                *ring = Ring::empty();
                responses.signal(StorageResponse::Ack);
                info!("storage history erased");
            } else {
                warn!("storage erase rejected or failed");
                responses.signal(StorageResponse::Ack);
            }
        }
    }
}

fn flush_bucket(
    flash: &mut FlashStorage<'static>,
    ring: &mut Ring,
    bucket: &Bucket,
    boot_generation: u32,
    clock: Option<Clock>,
) {
    if bucket.fresh_mask == 0 {
        return;
    }
    let now = Instant::now().as_secs();
    let (timestamp, utc) = clock
        .map(|c| {
            (
                c.unix_at_sync
                    .saturating_add(now.saturating_sub(c.monotonic_at_sync) as u32),
                true,
            )
        })
        .unwrap_or((now as u32, false));
    let sequence = ring.newest_sequence.wrapping_add(1).max(1);
    let record = Record {
        sequence,
        boot_generation,
        timestamp,
        utc,
        status: bucket.status,
        fresh_mask: bucket.fresh_mask,
        values: bucket.values,
    };
    if ring.append(flash, record).is_err() {
        warn!("storage record append failed");
    } else {
        info!(
            "stored measurement sequence={} fresh_mask={}",
            sequence, bucket.fresh_mask
        );
    }
}

fn load_config(flash: &mut FlashStorage<'static>) -> Config {
    let mut best = None;
    for sector in 0..CONFIG_SECTORS {
        let mut bytes = [0u8; 32];
        if flash
            .read(STORAGE_START + sector * SECTOR_SIZE, &mut bytes)
            .is_ok()
        {
            if let Some(config) = Config::decode(&bytes) {
                if best
                    .map(|current: Config| config.generation > current.generation)
                    .unwrap_or(true)
                {
                    best = Some(config);
                }
            }
        }
    }
    best.unwrap_or_default()
}

fn save_config(flash: &mut FlashStorage<'static>, config: Config) {
    let sector = config.generation % CONFIG_SECTORS;
    let address = STORAGE_START + sector * SECTOR_SIZE;
    if flash.erase(address, address + SECTOR_SIZE).is_err() {
        warn!("storage config erase failed");
        return;
    }
    let bytes = config.encode();
    if flash.write(address, &bytes[..28]).is_err()
        || flash.write(address + 28, &bytes[28..32]).is_err()
    {
        warn!("storage config write failed");
    }
}

fn erase_history(flash: &mut FlashStorage<'static>) -> Result<(), ()> {
    flash.erase(RING_START, STORAGE_END).map_err(|_| ())
}

fn metadata(config: Config, synchronized: bool, ring: Ring) -> [u8; METADATA_SIZE] {
    let mut out = [0u8; METADATA_SIZE];
    out[0] = config.enabled as u8;
    out[1] = synchronized as u8;
    out[2..6].copy_from_slice(&ring.count.to_le_bytes());
    out[6..10].copy_from_slice(&ring.oldest_sequence.to_le_bytes());
    out[10..14].copy_from_slice(&ring.newest_sequence.to_le_bytes());
    out[14] = ring.oldest_utc as u8;
    out[15] = ring.newest_utc as u8;
    out[16..20].copy_from_slice(&ring.oldest_timestamp.to_le_bytes());
    out[20..24].copy_from_slice(&ring.newest_timestamp.to_le_bytes());
    out
}

fn history(
    flash: &mut FlashStorage<'static>,
    ring: &Ring,
    cursor: u32,
    max_records: u16,
) -> [u8; HISTORY_DATA_SIZE] {
    let mut out = [0u8; HISTORY_DATA_SIZE];
    let (records, count) = collect_history(flash, ring, cursor, max_records);
    let mut cursor_out = 0u32;
    let mut offset = 5;
    out[4] = count as u8;
    for record in records.iter().flatten() {
        let encoded = record.encode();
        out[offset] = RECORD_SIZE as u8;
        out[offset + 1..offset + 1 + RECORD_SIZE].copy_from_slice(&encoded);
        offset += RECORD_SIZE + 1;
        cursor_out = record.sequence.wrapping_add(1);
    }
    out[..4].copy_from_slice(&cursor_out.to_le_bytes());
    out
}

fn history_len(
    flash: &mut FlashStorage<'static>,
    ring: &Ring,
    cursor: u32,
    max_records: u16,
) -> u16 {
    let (_, count) = collect_history(flash, ring, cursor, max_records);
    (5 + count * (RECORD_SIZE + 1)) as u16
}

fn collect_history(
    flash: &mut FlashStorage<'static>,
    ring: &Ring,
    cursor: u32,
    max_records: u16,
) -> ([Option<Record>; HISTORY_MAX_RECORDS], usize) {
    let mut records = [None; HISTORY_MAX_RECORDS];
    let mut count = 0;
    let start = if cursor == u32::MAX {
        ring.oldest_sequence
    } else {
        cursor
    };
    let limit = (max_records as usize).min(HISTORY_MAX_RECORDS);
    if limit == 0 || ring.count == 0 {
        return (records, 0);
    }
    let mut bytes = [0u8; RECORD_SIZE];
    for slot in 0..RING_CAPACITY {
        if flash.read(Ring::address(slot), &mut bytes).is_err() {
            continue;
        }
        let Some(record) = Record::decode(&bytes) else {
            continue;
        };
        if record.sequence < start {
            continue;
        }
        let mut position = count;
        while position > 0 && records[position - 1].unwrap().sequence > record.sequence {
            if position < limit {
                records[position] = records[position - 1];
            }
            position -= 1;
        }
        if position < limit {
            records[position] = Some(record);
            if count < limit {
                count += 1;
            }
        }
    }
    (records, count)
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
