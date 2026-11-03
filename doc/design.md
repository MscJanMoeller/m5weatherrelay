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
- "display" — OLED rotation and HMI rendering.
- "hmi" — volatile sensor snapshots, screen selection and button handling.
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

The receive, dispatch and WS90 decoding path and HMI-local sensor snapshots are
implemented; a shared sensor state service and the Home Assistant integration
remain planned.

Wi-Fi provisioning via Improv BLE; discovery via mDNS/DNS-SD.

### HMI test implementation

The Unit C6L has a 64 × 48 pixel monochrome OLED. The firmware maintains
volatile HMI snapshots and polls the active-low user button on P0 of the
PI4IOE5V6408 I/O expander. The pin and polarity follow the
[board pin map](https://docs.m5stack.com/en/unit/Unit_C6L) and M5Unified's
Unit C6L button handling. Screen rendering, button handling and concurrent
radio reception have not yet been validated on hardware. Wi-Fi and the Home
Assistant API are not implemented.

#### Screen layout and measurements

Reserve the upper-right 8 × 8 pixels for one connectivity symbol on every
screen. A compact 5 × 7 pixel font with a one-pixel gap fits up to ten
characters in each of the five remaining eight-pixel-high rows. Use the
remaining header space for a short screen title. The home temperature uses
a 10 × 20 pixel font when it fits; fall back to the compact font if the full
value and unit do not fit. The examples below are schematic: `#` stands for
the connectivity symbol, not a literal character.

The home screen shows temperature and relative humidity from a **configured
sensor**, identified by its complete model and transmitted device ID. Do not
silently substitute another sensor or combine readings from different
sensors. If no home sensor is configured, show `SET HOME`; if it has not
reported or a measurement is unavailable, show `--` with the appropriate
unit. Configure the identity in `src/config.rs`; there is no configuration UI.

```text
HOME 01  #
  21.4C
           <- temperature can occupy two rows
H 63%
```

After home, short presses cycle through screens grouped by measurement type,
across all discovered devices, in this order:

| Group | Common measurement kinds | Display |
|---|---|---|
| Climate | `Temperature`, `Humidity` | Temperature in °C and relative humidity in %. Home already shows these for the configured sensor, so omit its climate rows here. |
| Wind | `WindAverage`, `WindGust`, `WindDirection` | Average and gust in m/s; direction in degrees clockwise from north. Do not label the model-specific average as an instantaneous wind speed. |
| Rain / sun | `RainTotal`, `RainActive`, `Light`, `UvIndex` | Reported rain counter in mm, rain detection on/off, illuminance in lux, and UV index. Do not call the counter a rate, daily total or corrected lifetime total, or illuminance solar irradiance. |
| Power | `BatteryVoltage`, `SupercapVoltage` | Battery and supercapacitor voltages in V. |

Use the model's declared capabilities to decide which rows exist, rather than
the current packet's available values. Omit empty groups. This keeps screens
stable when a value temporarily becomes unavailable or a sensor becomes
stale. Pack up to five measurement rows per screen, keeping each group's
devices and their measurements in a deterministic order; continue the group
on another screen when needed. A device's rows may continue on the next
screen, but every row repeats the device alias. Never combine different
groups on one screen merely to fill space. Home is always present, even when
there are no discovered devices.

For example, one WS90 needs home plus one wind, one rain/sun and one power
screen. Other sensors' climate rows and measurements are packed into the same
group sequence rather than creating a separate set of screens per device:

```text
WIND     #    RAIN/SUN #    POWER    #
01 A3.2m/s    01 R12.3mm    01 B1.32V
01 G6.1m/s    01 rON        01 S4.85V
01 D270deg    01 L12klx     02 B1.20V
02 A1.4m/s    01 UV 5.2
02 G2.8m/s
```

`A` and `G` mean average and gust, `D` direction, `R` rain counter, `L`
illuminance, `U` UV index, `r` rain detection, `B` battery and `S`
supercapacitor. Compact units such as `klx`
may be used with the corresponding scaling. Format values to fit the row,
reducing displayed precision when necessary; if a value still cannot be
represented, show an explicit overflow indicator rather than clipping,
wrapping, or presenting a misleading value.

Each two-character alias identifies one complete `(model, device ID)` for the
current session; reserve `01` for the configured home sensor and assign
other aliases in discovery order without reusing them during that session.
Use two base-36 characters (`01` through `ZZ`), reserving `00`.
The test implementation supports up to 16 devices, reserving a slot for the
configured home sensor if it has not yet been received; capacity failures
are logged. The alias-to-full-identity mapping is printed in diagnostic output. Aliases may
change after reboot, and transmitted IDs can collide or change on re-pairing;
the HMI cannot distinguish devices that share the same complete RF identity.
If alias capacity is exhausted, report it explicitly rather than assigning a
duplicate. Do not reorder existing devices when another is discovered.

An omitted value in a complete snapshot is unavailable immediately; do not
keep an earlier reading as current. Track last receipt using a monotonic clock
and apply a model-specific or explicitly configured expiry policy. The
character immediately after each alias is a status slot: blank for a
non-expired reading, `!` for stale, and `?` for unknown freshness. Once
expired, replace values with `--`; if no expiry policy is available, mark
freshness unknown rather than presenting it as confirmed fresh. For example,
`01!A--m/s` is a stale wind average. Apply the same status convention on
home; do not infer missing measurements or rain resets.

#### Connectivity and button

The implementation displays one slashed Wi-Fi symbol because networking is
not yet implemented. Future connectivity support can use one status position:

| Symbol | Meaning |
|---|---|
| Wi-Fi with slash | Wi-Fi is not connected, including when networking is not enabled. |
| Wi-Fi | Wi-Fi is connected; no Home Assistant connection is confirmed. |
| Home Assistant logomark | Wi-Fi is connected and a Home Assistant connection is confirmed. |

The Home Assistant symbol requires a reliable way to identify an active Home Assistant session;
an arbitrary ESPHome API client alone is not proof. Until that is available,
the plain Wi-Fi symbol makes no claim that Home Assistant is disconnected.
The logomark is not included in this test implementation. Its small-size
readability and the [brand guidelines](https://contributors.home-assistant.io/brand/name-and-logo/)
must be checked before use.

Poll the button every 20 ms and debounce it for 30 ms. A debounced press shorter than
800 ms advances once, wrapping from the last screen to home. Holding it for
800 ms returns immediately to home; releasing that long press must not also
advance a screen. A long press on home stays on home. Navigation must not
block radio reception. If discovered capabilities change, preserve the
currently shown semantic page where possible; otherwise return to home.

#### Validation

Host tests cover layout, row packing, formatting and overflow, multiple
devices, an unconfigured or absent home sensor, unavailable and expired
values, capacity, button timing, and display and expander errors. Confirm on
hardware that the screen is readable, the button operates as expected, and
display updates and button handling do not interfere with RF reception.
Future connectivity support must test all states without treating an
unverified API client as Home Assistant.

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