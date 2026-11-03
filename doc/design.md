## M5WeatherRelay

Rust-based firmware for the M5Stack Unit C6L (ESP32-C6). The user has reported
successful WS90 reception and decoding with the firmware on hardware; the
reported log did not specify the regional profile or independently verify
measurement accuracy. Exposing measurements to Home Assistant is planned; the
ESPHome Native API, networking and provisioning are not implemented yet.

## Architecture and planned integrations
- Hardware: M5Stack Unit C6L, ESP32-C6, onboard SX1262 radio, OLED, RGB LED and buzzer. Consider both the 433 MHz and 868/915 MHz radio variants; select a sensor profile compatible with the board's RF matching and antenna.
- Runtime: Embassy async executor and synchronization.
- HAL: `esp-hal`.
- Planned wireless: `esp-radio` for Wi-Fi and BLE; isolate and document vendor blobs.
- Radio: SX1262 via SPI for receiving Ecowitt RF transmissions.
- SX1262 driver: reuse a local `sx1262` feature branch; upstream fixes and pin
  a published revision before sharing builds.
- SPI sharing: Embassy shared-bus abstractions for safe access by SX1262 and OLED.
- RF protocol: Pure Rust vendor-specific packet parsing, validation, checksum handling and measurement decoding, initially WS90.
- Packet parsing: Dedicated bit-level parsing for Ecowitt RF packets.
- Planned Home Assistant integration: implement the ESPHome Native API directly in Rust. No MQTT broker and no custom Home Assistant integration.
- Planned TCP networking: Embassy-compatible networking stack for the ESPHome API server.
- Planned Protocol Buffers: "prost" for ESPHome Native API message encoding and decoding.
- Planned encryption: Noise protocol implementation compatible with ESPHome Native API.
- Planned network discovery: mDNS/DNS-SD advertisement compatible with Home Assistant's built-in ESPHome integration.
- Planned provisioning: Improv BLE for Wi-Fi credential provisioning.


### Software design

Current components:
- "config" — firmware-specific selection of an RF receive profile.
- "sensor_registry" — application-owned registry, combined vendor result types and decoder adapters.
- "rf-receiver" — standalone async `no_std` receiver crate; chip-specific drivers.
- "rf-sensor" — `no_std` chip-neutral FSK settings, decoder interface and allocation-free registry.
- "rf-ecowitt" — Ecowitt family dispatch, WS90 packet validation, raw/physical measurements and regional radio profiles. WS80 and WS85 are candidates, not yet implemented.
- "hardware" — Unit C6L I/O expander and RF board configuration.
- "display" — OLED rotation and text output.
- "main" — board GPIO and SPI setup, radio reception and logging.

Planned components:
- "sensors" — measurement state and sensor abstractions.
- "network" — Wi-Fi, TCP and mDNS.
- "esphome_api" — TCP framing, Protocol Buffers, Noise handshake, entity
  registration and state updates.
- "provisioning" — Improv BLE.

Keep radio reception, Ecowitt decoding, sensor state, networking and the ESPHome API independently testable.

The registry stores unique, case-sensitive profile definitions independently of
sensor models. Each `SensorModelKey` is `(vendor, model ID)`, distinct from the
device IDs sent by individual sensors. Each model explicitly lists compatible
profile IDs. Register profiles before models: registration builds each profile's
model links atomically, rejects invalid declarations and reports capacity
exhaustion. The registry uses fixed capacities for profiles, models, and model
links per profile, without heap allocation. `from_definitions` builds a
registry from static declarations; individual registrations support composition
from multiple vendors. Selecting an `RfProfile` provides the `FskSettings` for
the receiver and dispatches each post-sync payload to every associated model.
A matching frequency alone does not establish compatibility; modulation,
sync, framing, and receiver settings also matter.

The active profile checks the configured fixed payload length. Unrecognized
packets, recognized invalid packets, and multiple valid decoders are reported
separately. Its mandatory invalid-candidate callback exposes every validation
error, even when another model succeeds or several models match. Multiple valid
matches are ambiguous and are not published as one arbitrary model. Registration
rejects duplicate profile or model keys and missing or duplicate model-profile
references. Registry entries share a typed result; the application combines
vendor data using enums and adapter functions. Registry decoders use the common
`DecodeError`: unsupported packets, per-decoder framing errors and recognized
invalid packets remain distinct, with structured details for common failure
types. Model identity is carried by the diagnostic callback. Vendor adapters
convert model-specific errors at the registry boundary; direct WS90 decoding
retains `ws90::Error`. `SensorModel::with_decoder` preserves vendor-supplied
metadata when adapting data types. Vendor crates
provide their profile definitions and models, not the application registry.
Register a shared profile definition once and associate models with it only
after verifying their RF settings and framing. Neither vendor crates nor
`rf-sensor` depend on radio drivers.

Each model declares unique static measurement capabilities through its registry
entry; registration rejects duplicates without changing the registry. A `Sample`
exposes a complete snapshot of the available measurements using a shared kind
for numeric and binary values. Kinds fix the canonical unit (`Unit::None` when
unitless) and separately classify the value as numeric or binary. Checked
constructors reject kind/value mismatches and non-finite numeric values. A
supported kind omitted from a snapshot is unavailable rather than retaining
an earlier value. Protocols that send partial updates require state assembly
before providing a snapshot. The application must separately expire stale
values using a monotonic clock. `Received<T>` optionally associates Unix
receive time with decoded data without requiring the decoder to know the clock.
Measurement identity combines model, transmitted device ID and kind;
transmitted IDs can collide or change on re-pairing.

The WS90 projection reports the sensor's rain-detection bit and uncorrected
accumulated-rain counter. It does not infer a rate, reset, or rollover policy.
The firmware byte and pressure remain protocol-specific until their
interpretation is established. ESPHome/HA device classes, statistics policies,
entity registration and value validation belong to the future mapping layer.

The receiver must be configured with the selected profile before decoding.
The SX1262 receiver currently accepts one fixed packet length per profile. A
single radio cannot simultaneously receive incompatible profiles; profile
switching and OOK support are future work. WS80 and WS85 require independent
RF-envelope and profile compatibility validation before association.

### Design principles
- Maximize open-source Rust dependencies; avoid ESP-IDF where practical.
- Prefer "no_std"-compatible libraries where practical.
- Use Embassy asynchronous tasks and synchronization.
- Avoid unnecessary dynamic allocation.
- Keep hardware, RF decoding and Home Assistant integration independent.
- Write unit tests for packet decoding and protocol handling.
- No dependency on ESPHome Device Builder.
- No modifications or custom integrations in Home Assistant.
- License project code MIT, as specified by `Cargo.toml` and `LICENSE`. Commit `Cargo.lock`; review dependencies and licenses with `cargo tree` and `cargo-deny`.

### Initial milestones
1. Bring up Embassy on the ESP32-C6 and initialize hardware.
2. Establish reliable SX1262 reception.
3. Decode and validate WS90 packets.
4. Maintain current weather measurements.
5. Implement ESPHome Native API compatibility.
6. Advertise the device through mDNS.
7. Expose temperature, humidity, wind, rainfall, solar radiation, UV and other available WS90 measurements as Home Assistant entities.

### Target architecture

SX1262 → RF receiver → RF profile dispatch → vendor decoder → sensor state → ESPHome Native API → Home Assistant

The receive, dispatch and WS90 decoding path is implemented; sensor state and
the Home Assistant integration remain planned.

Wi-Fi provisioning via Improv BLE; discovery via mDNS/DNS-SD.

### WS90 packet and profile decisions

The decoder receives bytes after the radio has removed the preamble and sync
word; it does not search arbitrary bitstreams. It accepts a 32-byte core, a
39-byte application record, or the observed 64-byte envelope with zero padding.
The core has CRC-8 (polynomial `0x31`, initial zero) and an additive checksum;
the record adds its own CRC and checksum. Both layers must pass for a record
or envelope. A core-only result is explicitly marked as such. Sentinel values
are `None`, the pressure scale is unconfirmed and retained as raw, and the
unknown extension is not interpreted. Measurement units use scaled integers.
The 39-byte fixture and the 868 MHz profile come from the sibling bring-up
project's [firmware and capture notes](../../WS90_SX1262_Bringup/WS90_FW_REVERSE_ENGINEERING.md);
the core layout is corroborated by the existing
[rtl_433 decoder](../../rtl_433_ESP/src/rtl_433/devices/fineoffset_ws90.c).

The WS90 v1.6.2 default, and selectors 1-3, share a modem profile but have
firmware-derived carrier frequencies of 433.925, 868.350, 915.000 and
920.900 MHz respectively. Only 868.350 MHz has a confirmed hardware capture
in the separate bring-up project; the other receive profiles need capture
validation with the corresponding sensor and matched board. The reported Rust
firmware capture did not specify its regional profile. Selector 4 has
different modem profiles and is not implemented. Carrier frequency alone must
not be used to infer a board's
usable RF band or local regulatory compliance. The 433 MHz board's GPIO,
front-end, TCXO and regulator settings also need independent validation.

For these byte-aligned packets, direct checked slices and bit masks avoid
allocation and dependencies and make the two integrity layers explicit.
[`pnet_macros`](https://docs.rs/pnet_macros/latest/pnet_macros/) generates
zero-copy packet accessors but its packet declaration requires a `Vec` payload
even for a fixed-size packet; it does not replace WS90 integrity validation.
Alternatives for more complicated
formats include [`packed_struct`](https://docs.rs/packed_struct/latest/packed_struct/)
(`no_std` without its default `std` feature),
[`bitfield-struct`](https://docs.rs/bitfield-struct/latest/bitfield_struct/)
(`no_std` register/bitfield accessors), and
[`deku`](https://docs.rs/deku/latest/deku/) (declarative bit parsing, requires
`alloc` in `no_std`). Reassess dependencies, licenses, and binary size when
adding formats where these abstractions are beneficial.