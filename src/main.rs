#![no_std]
#![no_main]

use display_interface_spi::SPIInterface;
use embassy_embedded_hal::shared_bus::asynch::spi::SpiDevice;
use embassy_executor::Spawner;
use embassy_futures::{
    join::join,
    select::{Either, select},
};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use embassy_time::{Delay, Duration, Instant, Ticker, Timer};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Input, InputConfig, Level, Output, OutputConfig},
    i2c::master::{Config as I2cConfig, I2c},
    interrupt::software::SoftwareInterruptControl,
    spi::master::{Config as SpiConfig, Spi},
    timer::timg::TimerGroup,
};
use m5weatherrelay::{
    config::{HMI_HOME_SENSOR, HMI_STALE_AFTER_MS, selected_profile},
    display::{DISPLAY_ROTATION, draw_screen},
    hardware::{RADIO_BOARD, button_pressed, init_io_expander, set_radio_front_end},
    hmi::{Button, ButtonEvent, Hmi, Update},
    sensor_registry,
};
use rf_receiver::{
    Sx1262,
    sx1262::{Error as RadioError, IRQ_ERRORS, IRQ_RX_DONE},
};
use rf_sensor::model::Sample;
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
    .into_buffered_graphics_mode();
    display.init().await.expect("OLED initialization failed");

    esp_println::println!("M5WeatherRelay: Embassy and board buses initialized");

    set_radio_front_end(&mut i2c, false, false).expect("Radio reset assertion failed");
    Timer::after(Duration::from_millis(100)).await;
    set_radio_front_end(&mut i2c, true, false).expect("Radio reset release failed");
    Timer::after(Duration::from_millis(10)).await;
    let registry = sensor_registry::registry().expect("RF registry initialization failed");
    let profile = selected_profile(&registry).expect("RF profile selection failed");
    let mut radio = Sx1262::new(radio_spi, radio_busy, radio_irq, Delay).with_board(RADIO_BOARD);
    radio
        .init_fsk(profile.fsk())
        .await
        .expect("SX1262 initialization failed");
    set_radio_front_end(&mut i2c, true, true).expect("Radio front-end enable failed");
    radio
        .start_receive()
        .await
        .expect("SX1262 start receive failed");
    esp_println::println!(
        "SX1262: listening for FSK packets at {} Hz",
        profile.fsk().frequency_hz
    );

    let hmi =
        Mutex::<CriticalSectionRawMutex, _>::new(Hmi::new(HMI_HOME_SENSOR, HMI_STALE_AFTER_MS));
    let receiver = async {
        let mut packets = 0u32;
        let mut buffer = [0; u8::MAX as usize];
        let mut heartbeat = Ticker::every(Duration::from_secs(10));
        loop {
            // The radio keeps DIO1 high until the IRQ flags are cleared, so no pin change is missed.
            match select(radio.wait_for_irq_pin(), heartbeat.next()).await {
                Either::First(result) => result.expect("SX1262 IRQ pin wait failed"),
                Either::Second(()) => {
                    esp_println::println!(
                        "SX1262: listening; {} packets, DIO1 {}",
                        packets,
                        radio.irq_pin_is_high().expect("SX1262 IRQ pin read failed")
                    );
                    continue;
                }
            }
            let flags = radio
                .read_irq_flags()
                .await
                .expect("SX1262 IRQ read failed");
            if flags & IRQ_ERRORS != 0 {
                esp_println::println!("SX1262: receive error IRQ 0x{:04x}", flags);
                radio
                    .clear_irq_flags(flags)
                    .await
                    .expect("SX1262 IRQ clear failed");
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
                        match profile.decode(&buffer[..packet.len], |invalid| {
                            esp_println::println!(
                                "{} {}: invalid packet: {:?}",
                                invalid.model.vendor,
                                invalid.model.model_id,
                                invalid.error
                            );
                        }) {
                            Ok(decoded) => {
                                esp_println::println!(
                                    "{} {}: {:?}",
                                    decoded.model.vendor,
                                    decoded.model.model_id,
                                    decoded.data
                                );
                                let model = profile
                                    .models()
                                    .find(|model| {
                                        model.vendor == decoded.model.vendor
                                            && model.model_id == decoded.model.model_id
                                    })
                                    .expect("decoded model missing from profile");
                                let device = decoded.data.device();
                                let update = {
                                    let mut hmi = hmi.lock().await;
                                    hmi.update(
                                        &decoded.data,
                                        model.capabilities,
                                        Instant::now().as_millis(),
                                    )
                                };
                                match update {
                                    Ok(Update::Discovered(alias)) => esp_println::println!(
                                        "HMI: {}{} = {} {} device 0x{:08x}",
                                        char::from(alias[0]),
                                        char::from(alias[1]),
                                        device.model.vendor,
                                        device.model.model_id,
                                        device.device
                                    ),
                                    Ok(Update::Existing) => {}
                                    Err(error) => esp_println::println!(
                                        "HMI: rejected {} {} device 0x{:08x}: {:?}",
                                        device.model.vendor,
                                        device.model.model_id,
                                        device.device,
                                        error
                                    ),
                                }
                            }
                            Err(error) => {
                                esp_println::println!(
                                    "{}: rejected packet: {:?}",
                                    profile.id(),
                                    error
                                )
                            }
                        }
                    }
                    Err(RadioError::InvalidPacketLength(length)) => {
                        esp_println::println!("SX1262: unexpected packet length {}", length);
                    }
                    Err(RadioError::PacketOverrun) => {
                        esp_println::println!("SX1262: packet dropped, next packet overwrote it");
                    }
                    Err(error) => panic!("SX1262 packet read failed: {:?}", error),
                }
            } else {
                panic!(
                    "SX1262: IRQ pin asserted without receive IRQ: 0x{:04x}",
                    flags
                );
            }
        }
    };
    let interface = async {
        let mut button = Button::new();
        let mut ticker = Ticker::every(Duration::from_millis(20));
        let mut previous = None;
        loop {
            let now = Instant::now().as_millis();
            let pressed = button_pressed(&mut i2c).expect("HMI button read failed");
            let event = button.update(pressed, now);
            let screen = {
                let mut hmi = hmi.lock().await;
                match event {
                    Some(ButtonEvent::Short) => hmi.next(),
                    Some(ButtonEvent::Long) => hmi.home(),
                    None => {}
                }
                hmi.screen(now)
            };
            if previous.as_ref() != Some(&screen) {
                draw_screen(&mut display, &screen)
                    .await
                    .expect("OLED update failed");
                previous = Some(screen);
            }
            ticker.next().await;
        }
    };
    join(receiver, interface).await;
}
