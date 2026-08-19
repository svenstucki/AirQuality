use defmt::{info, warn};
use embassy_futures::join::join;
use embassy_futures::select::{Either3, select3};
use embassy_sync::channel::Sender;
use embassy_sync::{
    blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal, watch::Receiver as WatchReceiver,
};
use embassy_time::Instant;
use esp_hal::peripherals::BT;
use trouble_host::prelude::*;

use crate::{
    led,
    rtc::{ClockState, Command as RtcCommand, Response as RtcResponse},
    sensors::SensorSnapshot,
    storage::{ERASE_TOKEN, StorageCommand, StorageResponse},
};

#[gatt_service(uuid = "7c3e1000-4e0a-4a69-9c57-6b5f3f1c0000")]
pub struct AirQualityService {
    #[characteristic(uuid = "7c3e1001-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub sht41_temperature: [u8; 2],
    #[characteristic(uuid = "7c3e1002-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub sht41_humidity: [u8; 2],
    #[characteristic(uuid = "7c3e1003-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub scd41_co2: [u8; 2],
    #[characteristic(uuid = "7c3e1004-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub scd41_temperature: [u8; 2],
    #[characteristic(uuid = "7c3e1005-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub scd41_humidity: [u8; 2],
    #[characteristic(uuid = "7c3e1006-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0, 0, 0])]
    pub bmp580_pressure: [u8; 4],
    #[characteristic(uuid = "7c3e1007-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub bmp580_temperature: [u8; 2],
    #[characteristic(uuid = "7c3e1008-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub sgp41_voc: [u8; 2],
    #[characteristic(uuid = "7c3e1009-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0, 0])]
    pub sgp41_nox: [u8; 2],
    #[characteristic(uuid = "7c3e100a-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = 0)]
    pub status: u8,
}

#[gatt_service(uuid = "7c3e2000-4e0a-4a69-9c57-6b5f3f1c0000")]
pub struct RgbLedService {
    #[characteristic(uuid = "7c3e2001-4e0a-4a69-9c57-6b5f3f1c0000", read, write, write_without_response, notify, value = [0, 0, 0])]
    pub color: [u8; 3],
}

#[gatt_service(uuid = "7c3e3000-4e0a-4a69-9c57-6b5f3f1c0000")]
pub struct StorageService {
    #[characteristic(uuid = "7c3e3001-4e0a-4a69-9c57-6b5f3f1c0000", read, write, value = [0])]
    pub control: [u8; 1],
    #[characteristic(uuid = "7c3e3002-4e0a-4a69-9c57-6b5f3f1c0000", write)]
    pub clock_sync: [u8; 4],
    #[characteristic(uuid = "7c3e3003-4e0a-4a69-9c57-6b5f3f1c0000", read, value = [0; 24])]
    pub metadata: [u8; 24],
    #[characteristic(uuid = "7c3e3004-4e0a-4a69-9c57-6b5f3f1c0000", write)]
    pub history_request: [u8; 6],
    #[characteristic(uuid = "7c3e3005-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0; 244])]
    pub history_data: [u8; 244],
    #[characteristic(uuid = "7c3e3006-4e0a-4a69-9c57-6b5f3f1c0000", write)]
    pub erase: [u8; 5],
}

#[gatt_service(uuid = "7c3e4000-4e0a-4a69-9c57-6b5f3f1c0000")]
pub struct RtcService {
    #[characteristic(uuid = "7c3e4001-4e0a-4a69-9c57-6b5f3f1c0000", read, notify, value = [0; 5])]
    pub time: [u8; 5],
}

#[gatt_server]
pub struct Server {
    pub air_quality: AirQualityService,
    pub rgb_led: RgbLedService,
    pub storage: StorageService,
    pub rtc: RtcService,
}

pub async fn run(
    bluetooth: BT<'static>,
    mut led_driver: led::Led<'static>,
    updates: &Signal<CriticalSectionRawMutex, SensorSnapshot>,
    storage_commands: Sender<'static, CriticalSectionRawMutex, StorageCommand, 4>,
    storage_responses: &'static Signal<CriticalSectionRawMutex, StorageResponse>,
    rtc_commands: Sender<'static, CriticalSectionRawMutex, RtcCommand, 4>,
    rtc_responses: &'static Signal<CriticalSectionRawMutex, RtcResponse>,
    mut rtc_state: WatchReceiver<'static, CriticalSectionRawMutex, ClockState, 1>,
) -> ! {
    let connector = esp_radio::ble::controller::BleConnector::new(bluetooth, Default::default())
        .expect("BLE connector initialization failed");
    let controller: ExternalController<_, 20> = ExternalController::new(connector);
    let mut resources: HostResources<DefaultPacketPool, 1, 2> = HostResources::new();
    let stack = trouble_host::new(controller, &mut resources);
    let host = stack.build();
    let Host {
        mut runner,
        mut peripheral,
        ..
    } = host;
    let server = Server::new_with_config(GapConfig::Peripheral(PeripheralConfig {
        name: "AirQuality",
        appearance: &appearance::sensor::GENERIC_SENSOR,
    }))
    .expect("GATT server initialization failed");

    info!("BLE advertising as AirQuality");

    let _ = join(
        async {
            loop {
                if runner.run().await.is_err() {
                    warn!("BLE controller runner stopped; restarting");
                }
            }
        },
        async {
            loop {
                let mut adv_data = [0; 31];
                let len = AdStructure::encode_slice(
                    &[
                        AdStructure::Flags(LE_GENERAL_DISCOVERABLE | BR_EDR_NOT_SUPPORTED),
                        AdStructure::CompleteLocalName(b"AirQuality"),
                    ],
                    &mut adv_data,
                )
                .unwrap();
                let advertiser = peripheral
                    .advertise(
                        &Default::default(),
                        Advertisement::ConnectableScannableUndirected {
                            adv_data: &adv_data[..len],
                            scan_data: &[],
                        },
                    )
                    .await
                    .unwrap();
                let conn = advertiser
                    .accept()
                    .await
                    .unwrap()
                    .with_attribute_server(&server)
                    .unwrap();
                let current = updates.wait().await;
                set_server_snapshot(&server, current);
                request_metadata(&storage_commands).await;
                if let StorageResponse::Metadata { data } = storage_responses.wait().await {
                    let _ = server.set(&server.storage.metadata, &data);
                    let _ = server.set(&server.storage.control, &[data[0]]);
                }
                update_rtc_value(&server, &mut rtc_state);
                handle_connection(
                    conn,
                    &server,
                    &mut led_driver,
                    updates,
                    &storage_commands,
                    storage_responses,
                    &rtc_commands,
                    rtc_responses,
                    &mut rtc_state,
                )
                .await;
                info!("BLE client disconnected");
            }
        },
    )
    .await;
    loop {}
}

async fn handle_connection<C>(
    conn: GattConnection<'_, '_, C>,
    server: &Server<'_>,
    led_driver: &mut led::Led<'_>,
    updates: &Signal<CriticalSectionRawMutex, SensorSnapshot>,
    storage_commands: &Sender<'static, CriticalSectionRawMutex, StorageCommand, 4>,
    storage_responses: &Signal<CriticalSectionRawMutex, StorageResponse>,
    rtc_commands: &Sender<'static, CriticalSectionRawMutex, RtcCommand, 4>,
    rtc_responses: &Signal<CriticalSectionRawMutex, RtcResponse>,
    rtc_state: &mut WatchReceiver<'static, CriticalSectionRawMutex, ClockState, 1>,
) where
    C: PacketPool,
{
    loop {
        match select3(
            conn.next(),
            notify_updates(&conn, server, updates),
            notify_rtc(&conn, server, rtc_state),
        )
        .await
        {
            Either3::First(GattConnectionEvent::Disconnected { .. }) => break,
            Either3::First(GattConnectionEvent::Gatt { event }) => match event {
                GattEvent::Read(read) if read.handle() == server.storage.metadata.handle() => {
                    // Metadata is derived from the storage ring, which can change while
                    // a connection is active. Refresh the characteristic before replying
                    // so clients polling it receive current record counts and timestamps.
                    refresh_metadata(server, storage_commands, storage_responses).await;
                    if let Ok(reply) = read.accept() {
                        reply.send().await;
                    }
                }
                GattEvent::Read(read) if read.handle() == server.rtc.time.handle() => {
                    update_rtc_value(server, rtc_state);
                    if let Ok(reply) = read.accept() {
                        reply.send().await;
                    }
                }
                GattEvent::Write(write) if write.handle() == server.rgb_led.color.handle => {
                    let data = write.data();
                    let mut applied_color = None;
                    if data.len() == 3 {
                        let color = [data[0], data[1], data[2]];
                        if led_driver.write_rgb(color).is_ok() {
                            info!("RGB LED updated: {}, {}, {}", color[0], color[1], color[2]);
                            let _ = server.set(&server.rgb_led.color, &color);
                            applied_color = Some(color);
                        } else {
                            warn!("RGB LED write failed");
                        }
                    }
                    if let Ok(reply) = write.accept() {
                        reply.send().await;
                    }
                    if let Some(color) = applied_color {
                        let _ = server.rgb_led.color.notify(&conn, &color).await;
                    }
                }
                GattEvent::Write(write) if write.handle() == server.storage.control.handle => {
                    if write.data().len() == 1 && write.data()[0] <= 1 {
                        let _ = storage_commands
                            .send(StorageCommand::SetEnabled(write.data()[0] != 0))
                            .await;
                        let _ = storage_responses.wait().await;
                        let _ = server.set(&server.storage.control, &[write.data()[0]]);
                        refresh_metadata(server, storage_commands, storage_responses).await;
                    }
                    if let Ok(reply) = write.accept() {
                        reply.send().await;
                    }
                }
                GattEvent::Write(write) if write.handle() == server.storage.clock_sync.handle => {
                    if write.data().len() == 4 {
                        let unix = u32::from_le_bytes(write.data().try_into().unwrap());
                        let _ = rtc_commands.send(RtcCommand::SetUnix(unix)).await;
                        let result = rtc_responses.wait().await;
                        if result.success {
                            let _ = server.set(
                                &server.rtc.time,
                                &result.state.payload(Instant::now().as_secs()),
                            );
                        } else {
                            warn!("BLE clock sync rejected because RTC update failed");
                        }
                        refresh_metadata(server, storage_commands, storage_responses).await;
                    }
                    if let Ok(reply) = write.accept() {
                        reply.send().await;
                    }
                }
                GattEvent::Write(write)
                    if write.handle() == server.storage.history_request.handle =>
                {
                    if write.data().len() == 6 {
                        let cursor = u32::from_le_bytes(write.data()[..4].try_into().unwrap());
                        let max_records =
                            u16::from_le_bytes(write.data()[4..6].try_into().unwrap());
                        let _ = storage_commands
                            .send(StorageCommand::ReadHistory {
                                cursor,
                                max_records,
                            })
                            .await;
                        if let StorageResponse::History { data, len } =
                            storage_responses.wait().await
                        {
                            info!("history response length={}", len);
                            let _ = server.set(&server.storage.history_data, &data);
                            let _ = server.storage.history_data.notify(&conn, &data).await;
                        }
                    }
                    if let Ok(reply) = write.accept() {
                        reply.send().await;
                    }
                }
                GattEvent::Write(write) if write.handle() == server.storage.erase.handle => {
                    if write.data() == ERASE_TOKEN {
                        let _ = storage_commands
                            .send(StorageCommand::Erase(ERASE_TOKEN))
                            .await;
                        let _ = storage_responses.wait().await;
                        refresh_metadata(server, storage_commands, storage_responses).await;
                    }
                    if let Ok(reply) = write.accept() {
                        reply.send().await;
                    }
                }
                event => {
                    if let Ok(reply) = event.accept() {
                        reply.send().await;
                    }
                }
            },
            Either3::First(_) => {}
            Either3::Second(_) => {}
            Either3::Third(_) => {}
        }
    }
}

async fn request_metadata(commands: &Sender<'static, CriticalSectionRawMutex, StorageCommand, 4>) {
    commands.send(StorageCommand::ReadMetadata).await;
}

async fn refresh_metadata(
    server: &Server<'_>,
    commands: &Sender<'static, CriticalSectionRawMutex, StorageCommand, 4>,
    responses: &Signal<CriticalSectionRawMutex, StorageResponse>,
) {
    commands.send(StorageCommand::ReadMetadata).await;
    if let StorageResponse::Metadata { data } = responses.wait().await {
        let _ = server.set(&server.storage.metadata, &data);
        let _ = server.set(&server.storage.control, &[data[0]]);
    }
}

fn update_rtc_value(
    server: &Server<'_>,
    rtc_state: &mut WatchReceiver<'static, CriticalSectionRawMutex, ClockState, 1>,
) {
    if let Some(state) = rtc_state.try_get() {
        let _ = server.set(&server.rtc.time, &state.payload(Instant::now().as_secs()));
    }
}

async fn notify_rtc<C>(
    conn: &GattConnection<'_, '_, C>,
    server: &Server<'_>,
    rtc_state: &mut WatchReceiver<'static, CriticalSectionRawMutex, ClockState, 1>,
) where
    C: PacketPool,
{
    let state = rtc_state.changed().await;
    let payload = state.payload(Instant::now().as_secs());
    let _ = server.set(&server.rtc.time, &payload);
    let _ = server.rtc.time.notify(conn, &payload).await;
}

async fn notify_updates<C>(
    conn: &GattConnection<'_, '_, C>,
    server: &Server<'_>,
    updates: &Signal<CriticalSectionRawMutex, SensorSnapshot>,
) where
    C: PacketPool,
{
    let snapshot = updates.wait().await;
    set_server_snapshot(server, snapshot);
    let _ = server
        .air_quality
        .sht41_temperature
        .notify(conn, &snapshot.sht41_temperature_centi_c.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .sht41_humidity
        .notify(conn, &snapshot.sht41_humidity_centi_percent.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .scd41_co2
        .notify(conn, &snapshot.scd41_co2_ppm.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .scd41_temperature
        .notify(conn, &snapshot.scd41_temperature_centi_c.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .scd41_humidity
        .notify(conn, &snapshot.scd41_humidity_centi_percent.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .bmp580_pressure
        .notify(conn, &snapshot.bmp580_pressure_pa.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .bmp580_temperature
        .notify(conn, &snapshot.bmp580_temperature_centi_c.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .sgp41_voc
        .notify(conn, &snapshot.sgp41_voc_index.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .sgp41_nox
        .notify(conn, &snapshot.sgp41_nox_index.to_le_bytes())
        .await;
    let _ = server
        .air_quality
        .status
        .notify(conn, &snapshot.status)
        .await;
}

fn set_server_snapshot(server: &Server, snapshot: SensorSnapshot) {
    let _ = server.set(
        &server.air_quality.sht41_temperature,
        &snapshot.sht41_temperature_centi_c.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.sht41_humidity,
        &snapshot.sht41_humidity_centi_percent.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.scd41_co2,
        &snapshot.scd41_co2_ppm.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.scd41_temperature,
        &snapshot.scd41_temperature_centi_c.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.scd41_humidity,
        &snapshot.scd41_humidity_centi_percent.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.bmp580_pressure,
        &snapshot.bmp580_pressure_pa.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.bmp580_temperature,
        &snapshot.bmp580_temperature_centi_c.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.sgp41_voc,
        &snapshot.sgp41_voc_index.to_le_bytes(),
    );
    let _ = server.set(
        &server.air_quality.sgp41_nox,
        &snapshot.sgp41_nox_index.to_le_bytes(),
    );
    let _ = server.set(&server.air_quality.status, &snapshot.status);
}
