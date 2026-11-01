#![no_std]
#![no_main]

use display_interface_spi::SPIInterface;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_executor::Spawner;
use embassy_futures::select::{Either, select};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Delay, Duration, Instant, Timer};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    spi::master::{Config as SpiConfig, Spi},
    timer::timg::TimerGroup,
};
use m5weatherrelay::{
    display::{DISPLAY_ROTATION, write_text},
    hardware::{init_io_expander, set_radio_front_end},
    ws90::WS90_868,
};
use rf_receiver::{
    Sx1262,
    sx1262::{Error as RadioError, IRQ_ERRORS, IRQ_RX_DONE},
};
use ssd1306::{Ssd1306Async, mode::DisplayConfigAsync, size::DisplaySize64x48};

esp_bootloader_esp_idf::esp_app_desc!();

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    let p = esp_hal::init(esp_hal::Config::default());
    let software_interrupt = SoftwareInterruptControl::new(p.SW_INTERRUPT);
    esp_rtos::start(
        TimerGroup::new(p.TIMG0).timer0,
        software_interrupt.software_interrupt0,
    );
    esp_println::println!("M5WeatherRelay: starting");

    let mut i2c = I2c::new(p.I2C0, I2cConfig::default())
        .expect("I2C configuration failed")
        .with_sda(p.GPIO10)
        .with_scl(p.GPIO8);
    init_io_expander(&mut i2c).expect("Unit C6L I/O expander initialization failed");

    let radio_cs = Output::new(p.GPIO23, Level::High, OutputConfig::default());
    let oled_cs = Output::new(p.GPIO6, Level::High, OutputConfig::default());
    let dc = Output::new(p.GPIO18, Level::Low, OutputConfig::default());
    let mut oled_reset = Output::new(p.GPIO15, Level::Low, OutputConfig::default());
    let radio_irq = Input::new(p.GPIO7, InputConfig::default());
    let radio_busy = Input::new(p.GPIO19, InputConfig::default());
    let _buzzer = Output::new(p.GPIO11, Level::Low, OutputConfig::default());
    let _rgb_data = Output::new(p.GPIO2, Level::Low, OutputConfig::default());

    let spi = Spi::new(p.SPI2, SpiConfig::default())
        .expect("SPI configuration failed")
        .with_sck(p.GPIO20)
        .with_mosi(p.GPIO21)
        .with_miso(p.GPIO22)
        .into_async();
    let bus = Mutex::<CriticalSectionRawMutex, _>::new(spi);
    let radio_spi = SpiDevice::new(&bus, radio_cs);
    let oled = SpiDevice::new(&bus, oled_cs);

    Timer::after(Duration::from_millis(10)).await;
    oled_reset.set_high();
    Timer::after(Duration::from_millis(10)).await;
    let mut display = Ssd1306Async::new(
        SPIInterface::new(oled, dc),
        DisplaySize64x48,
        DISPLAY_ROTATION,
    )
    .into_terminal_mode();
    display.init().await.expect("OLED initialization failed");
    display.clear().await.expect("OLED clear failed");
    write_text(&mut display, "M5\nWeather\nRelay")
        .await
        .expect("OLED write failed");

    esp_println::println!("M5WeatherRelay: Embassy and board buses initialized");

    set_radio_front_end(&mut i2c, false, false).expect("Radio reset assertion failed");
    Timer::after(Duration::from_millis(100)).await;
    set_radio_front_end(&mut i2c, true, false).expect("Radio reset release failed");
    Timer::after(Duration::from_millis(10)).await;
    let mut radio = Sx1262::new(radio_spi, radio_busy, radio_irq, Delay);
    let init_status = radio
        .init_fsk(&WS90_868)
        .await
        .expect("SX1262 initialization failed");
    if init_status.cleared_xosc_start_error {
        esp_println::println!("SX1262: cleared oscillator error before enabling TCXO");
    }
    set_radio_front_end(&mut i2c, true, true).expect("Radio front-end enable failed");
    radio
        .start_receive()
        .await
        .expect("SX1262 start receive failed");
    esp_println::println!("SX1262: listening for raw FSK packets at 868.350 MHz");

    let mut packets = 0u32;
    let mut buffer = [0; 64];
    let mut next_poll = Instant::now() + Duration::from_secs(1);
    let mut next_heartbeat = Instant::now() + Duration::from_secs(10);
    loop {
        let irq_woke = match select(radio.wait_for_irq_pin(), Timer::at(next_poll)).await {
            Either::First(result) => {
                result.expect("SX1262 IRQ pin wait failed");
                true
            }
            Either::Second(()) => {
                next_poll = Instant::now() + Duration::from_secs(1);
                false
            }
        };
        let flags = radio
            .read_irq_flags()
            .await
            .expect("SX1262 IRQ read failed");
        if flags & IRQ_ERRORS != 0 {
            esp_println::println!("SX1262: receive error IRQ 0x{:04x}", flags);
            radio
                .start_receive()
                .await
                .expect("SX1262 resume receiving failed");
        } else if flags & IRQ_RX_DONE != 0 {
            match radio.read_packet(&mut buffer).await {
                Ok(packet) => {
                    packets = packets.wrapping_add(1);
                    let rssi = -packet.rssi_half_dbm;
                    esp_println::print!(
                        "SX1262: packet {} RSSI -{}.{} dBm IRQ 0x{:04x}: ",
                        packets,
                        rssi / 2,
                        (rssi % 2) * 5,
                        flags
                    );
                    for byte in &buffer[..packet.len] {
                        esp_println::print!("{:02x}", byte);
                    }
                    esp_println::println!();
                }
                Err(RadioError::InvalidPacketLength(length)) => {
                    esp_println::println!("SX1262: unexpected packet length {}", length);
                }
                Err(error) => panic!("SX1262 packet read failed: {:?}", error),
            }
            radio
                .start_receive()
                .await
                .expect("SX1262 resume receiving failed");
        } else if irq_woke {
            panic!(
                "SX1262: IRQ pin asserted without receive IRQ: 0x{:04x}",
                flags
            );
        }
        if Instant::now() >= next_heartbeat {
            next_heartbeat = Instant::now() + Duration::from_secs(10);
            esp_println::println!(
                "SX1262: listening; {} packets, IRQ 0x{:04x}, DIO1 {}",
                packets,
                flags,
                radio.irq_pin_is_high().expect("SX1262 IRQ pin read failed")
            );
        }
    }
}
