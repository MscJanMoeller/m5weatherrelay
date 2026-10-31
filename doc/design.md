## M5WeatherRelay

Rust-based firmware for the M5Stack Unit C6L (ESP32-C6), receiving Ecowitt weather sensor transmissions (initially WS90) and exposing measurements to Home Assistant.

## Architecture
- Hardware: M5Stack Unit C6L, ESP32-C6, onboard SX1262 radio, OLED, RGB LED - and buzzer.
- Runtime: Embassy async executor and synchronization.
- HAL: `esp-hal`.
- Wireless: `esp-radio` for Wi-Fi and BLE; isolate and document vendor blobs.
- Radio: SX1262 via SPI for receiving Ecowitt RF transmissions.
- SPI sharing: Embassy shared-bus abstractions for safe access by SX1262 and OLED.
- RF protocol: Pure Rust WS90 packet parsing, validation, checksum handling and measurement decoding.
- Packet parsing: Dedicated bit-level parsing for Ecowitt RF packets.
- Home Assistant integration: Implement the ESPHome Native API directly in Rust. No MQTT broker and no custom Home Assistant integration.
- TCP networking: Embassy-compatible networking stack for the ESPHome API server.
- Protocol Buffers: "prost" for ESPHome Native API message encoding and decoding.
- Encryption: Noise protocol implementation compatible with ESPHome Native API.
- Network discovery: mDNS/DNS-SD advertisement compatible with Home Assistant's built-in ESPHome integration.
- Provisioning: Improv BLE for Wi-Fi credential provisioning.


### Software design

Keep the firmware modular:
- "radio" — SX1262 driver and receive handling.
- "ecowitt" — WS90 packet parsing, validation and measurement decoding.
- "sensors" — measurement state and sensor abstractions.
- "network" — Wi-Fi, TCP and mDNS.
- "esphome_api" — TCP framing, Protocol Buffers, Noise handshake, entity
- registration and state updates.
- "provisioning" — Improv BLE.
- "hardware" — board-specific GPIO, SPI, OLED, LED and buzzer.

Keep radio reception, Ecowitt decoding, sensor state, networking and the ESPHome API independently testable.

### Design principles
- Maximize open-source Rust dependencies; avoid ESP-IDF where practical.
- Prefer "no_std"-compatible libraries where practical.
- Use Embassy asynchronous tasks and synchronization.
- Avoid unnecessary dynamic allocation.
- Keep hardware, RF decoding and Home Assistant integration independent.
- Write unit tests for packet decoding and protocol handling.
- No dependency on ESPHome Device Builder.
- No modifications or custom integrations in Home Assistant.
- License project code MIT OR Apache-2.0. Commit `Cargo.lock`; review dependencies and licenses with `cargo tree` and `cargo-deny`.

### Initial milestones
1. Bring up Embassy on the ESP32-C6 and initialize hardware.
2. Establish reliable SX1262 reception.
3. Decode and validate WS90 packets.
4. Maintain current weather measurements.
5. Implement ESPHome Native API compatibility.
6. Advertise the device through mDNS.
7. Expose temperature, humidity, wind, rainfall, solar radiation, UV and other available WS90 measurements as Home Assistant entities.

### Target architecture

SX1262 → WS90 decoder → sensor state → ESPHome Native API → Home Assistant

Wi-Fi provisioning via Improv BLE; discovery via mDNS/DNS-SD.