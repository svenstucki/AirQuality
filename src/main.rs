#![no_std]
#![no_main]

esp_bootloader_esp_idf::esp_app_desc!();

mod ble;
mod led;
mod sensors;

use defmt::info;
use embassy_futures::join::join;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, signal::Signal};
use embassy_time::Duration;
use esp_hal::{
    clock::CpuClock,
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    rmt::TxChannelCreator,
    timer::timg::TimerGroup,
};
use sensors::SensorSnapshot;

static SENSOR_UPDATES: Signal<CriticalSectionRawMutex, SensorSnapshot> = Signal::new();
#[esp_rtos::main]
async fn main(_spawner: embassy_executor::Spawner) -> ! {
    esp_alloc::heap_allocator!(size: 72 * 1024);

    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));

    let timer_group = TimerGroup::new(peripherals.TIMG0);
    let software_interrupt = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    esp_rtos::start(timer_group.timer0, software_interrupt.software_interrupt0);

    let i2c = I2c::new(
        peripherals.I2C0,
        I2cConfig::default().with_frequency(esp_hal::time::Rate::from_khz(400)),
    )
    .expect("I2C initialization failed")
    .with_sda(peripherals.GPIO6)
    .with_scl(peripherals.GPIO7)
    .into_async();

    let rmt = esp_hal::rmt::Rmt::new(peripherals.RMT, esp_hal::time::Rate::from_mhz(80))
        .expect("RMT initialization failed");
    let led_channel = rmt
        .channel0
        .configure_tx(
            &esp_hal::rmt::TxChannelConfig::default()
                .with_clk_divider(1)
                .with_idle_output_level(esp_hal::gpio::Level::Low)
                .with_idle_output(true)
                .with_carrier_modulation(false),
        )
        .expect("RMT channel configuration failed")
        .with_pin(peripherals.GPIO8);
    let led = led::Led::new(led_channel);
    info!("air-quality firmware starting; I2C SDA=GPIO6 SCL=GPIO7 LED=GPIO8");

    join(
        sensors::run(i2c, &SENSOR_UPDATES),
        ble::run(peripherals.BT, led, &SENSOR_UPDATES),
    )
    .await;

    loop {
        embassy_time::Timer::after(Duration::from_secs(60)).await;
    }
}

use defmt_rtt as _;
use esp_backtrace as _;
