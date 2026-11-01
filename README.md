# M5WeatherRelay
Rust firmware for the M5Stack Unit C6L. It receives and decodes Ecowitt WS90
weather sensor data. Home Assistant integration via the ESPHome Native API is
planned, not yet implemented.

## Bring-up

Install Rust, the `riscv32imac-unknown-none-elf` target and `espflash`:

```sh
rustup target add riscv32imac-unknown-none-elf
cargo install espflash --locked
cargo build --release --locked
cargo run --release --locked
```

The build requires the sibling checkout `..\sx1262` on the local
`feature-sx1262-driver-improvements` branch. This branch is not published yet; a path
dependency is not pinned by `Cargo.lock` and is not portable by itself.

Connect the Unit C6L over USB. The OLED displays `M5`, `Weather` and `Relay`.
The SX1262 listens at 868.350 MHz using 17.2 kbps FSK, 35 kHz deviation,
117.3 kHz bandwidth, sync `2D D4` and fixed 64-byte packets. The firmware sleeps
until the radio raises DIO1 and then reads the packet. Every 10 seconds
the monitor reports packet count and the DIO1 level; received packets are
printed in hex with RSSI. Each packet is also passed through the Ecowitt
decoder: valid WS90 values are printed with scaled-integer units and rejected
packets include an explicit reason. The decoder has been tested against a
capture from the separate bring-up project. The user has also reported
successful WS90 reception and decoding with the Rust firmware on hardware;
three reported packets are included in the decoder regression tests. The
reported log did not specify the regional profile or independently verify
measurement accuracy.
Buzzer and RGB data pins are held low.

The default is the hardware-tested 868.350 MHz profile. To select a different
firmware-derived carrier, build with exactly one feature:

```sh
cargo build --release --locked --features ws90-433  # 433.925 MHz
cargo build --release --locked --features ws90-915  # 915.000 MHz
cargo build --release --locked --features ws90-920  # 920.900 MHz
```

The alternative bands lack confirmed hardware validation. Use the 433 MHz Unit C6L
variant for 433 MHz and an appropriately matched board and antenna for other
bands; the SX1262's tuning range alone does not establish board compatibility.
The existing GPIO, front-end, TCXO and regulator settings were validated for
the 868 MHz board; verify them on the 433 MHz variant before using it.
Select the sensor's actual region and comply with local regulations. The
selector-4 WS90 profiles use different modem settings and are not implemented.

The allocation-free `rf-sensor` crate provides chip-neutral FSK settings, a
decoder interface and a fixed-capacity registry. The firmware selects an `RfProfile`
before configuring the radio; each packet is dispatched to all models
explicitly associated with that profile. Profile IDs are globally unique and
case-sensitive. Model keys are `(vendor, model_id)`; for example, WS90 uses
`("Ecowitt", "ws90")`. Decoded data retains the individual device ID as well
as the model identity. Different vendors can be composed into an application
registry using typed enums and adapter functions. Register profile definitions
before models; registration builds the profile-to-model links and reports
duplicate keys, invalid references, and capacity failures without partial model
registration. Select a registered profile, then pass its `fsk()` settings to
the receiver and its packets to `decode()`.

The [application registry](src/sensor_registry.rs) owns composition: vendor
crates supply profile definitions, models, and decoders, while the application
provides combined data and error enums and adapts each model decoder.
`SensorModel::with_decoder` retains vendor metadata and compatible profile IDs
when adapting its result types. Register all needed profile definitions before
their models; a shared profile is registered once, and models may reference it
only when their complete RF settings and framing are known to be compatible.
The three capacities are profiles, models, and model links **per profile**.
Increase them when adding vendors. Definitions and models are static, while the
constructed registry owns its links.

The active profile validates packet length and distinguishes unsupported,
invalid and ambiguous packets. A callback reports every recognized invalid
candidate, including when another decoder succeeds. Multiple valid decoders
produce an ambiguity error rather than publishing an arbitrary result.
This does not provide simultaneous reception on different radio profiles.
`rf-ecowitt` currently decodes WS90 only; WS80 and WS85 are future candidates
whose RF envelopes and profile compatibility still need verification.
A 32-byte WS90 core is validated independently; 39-byte records
and 64-byte RF envelopes require both CRC/checksum layers, and the latter
also require zero padding. Invalid sentinels are omitted, unknown extension
bytes are retained, and pressure remains raw because its scale is unverified.
The standalone WS90 decoder accepts those three lengths; the active profile
requires the 64-byte length of its configured RF profile.
See [design notes](doc/design.md) for packet provenance and parser alternatives.

The receive-only `rf-receiver` crate uses async embedded-hal SPI,
GPIO waits and delays, and follows RadioLib concepts (`init_fsk`,
`start_receive`, `read_packet`). `start_receive` enters continuous receive
mode, so the radio keeps receiving after each packet, as with RadioLib's
`startReceive` and `readData`. Its `receive` method awaits the IRQ pin and
reads one packet. The firmware uses the lower-level `wait_for_irq_pin`,
`read_irq_flags`, `read_packet` and `clear_irq_flags` instead, because only the
pin wait can safely race its heartbeat timer. `read_packet` clears the
packet flags first and reports `PacketOverrun` if the next packet starts
arriving while the buffer is read; error flags are left for the caller to
clear. Use `standby` and `start_receive` to stop and restart receiving. Do not
cancel `receive` during SPI traffic without checking that the radio still
receives.
The OLED and radio share the same async SPI bus.

The `FskSettings` in `rf-sensor` and caller-owned receive buffer are chip-neutral;
`rf-receiver` re-exports the settings types. GPIO wiring,
IRQ flags and errors are chip-specific. An SX1276 driver can use the same
profile and receive concepts with a different constructor and implementation.
It is **not implemented**: SX1276 supports different bandwidth steps (including
125 kHz rather than SX1262's 117.3 kHz), so it requires a suitable profile.
The current SX1262 driver supports the chip's 150-960 MHz range, the GFSK
receive bandwidths of the `sx1262` driver (the WS90 profile uses 117.3 kHz), a 1-8
byte sync word and fixed-length packets. The profile's optional
`crc` (1 or 2 bytes, initial value, polynomial, inversion; `CrcConfig::default()`
is the RadioLib CCITT setting) and `whitening` (9-bit seed; `WhiteningConfig::default()`
is RadioLib's 0x01FF) are off when `None`, as in the WS90 profile. The CRC and
whitening settings are covered by host tests only: they are not validated on
hardware, and it is unchecked whether the radio counts the CRC bytes in the
received payload length that `read_packet` compares with the profile's
`packet_length`. The oscillator and regulator depend on the board, not on the
radio link, so they are set with `Sx1262::with_board(BoardConfig)` and not in
the profile. `BoardConfig::default()` is RadioLib's (1.6 V TCXO, DC-DC); `None`
for `tcxo` selects a crystal. The tested 868 MHz Unit C6L uses
`hardware::RADIO_BOARD` (3 V TCXO, LDO only). The preamble detector length follows from the profile: the longest of
8, 16, 24 or 32 bits that fits the shorter of the sync word and the preamble.
Unsupported settings are rejected. The image calibration uses the
datasheet value of the ISM band containing the frequency (430-440, 470-510,
779-787, 863-870 or 902-928 MHz); elsewhere it calibrates 4 MHz around the
frequency as RadioLib does, which Semtech does not specify. Only 868.350 MHz
has a confirmed hardware capture in the separate bring-up project; the board's
matching network and antenna may limit the usable band.
The receiver reuses the local [`sx1262`](../sx1262) async SPI device and FSK
commands, with caller-managed BUSY timing. Driver failures preserve SPI errors
and report the opcode and first byte of invalid responses.

Run host tests for the workspace with:

```sh
cargo test --workspace --target x86_64-pc-windows-msvc --locked
```

VS Code's Rust analyzer checks the embedded firmware without test targets;
the tests require the host target above. In a multi-root VS Code workspace,
set `rust-analyzer.cfg.setTest` and `rust-analyzer.check.allTargets` to `false`
in the workspace settings: folder-level settings for these options are ignored.
These settings also apply to other Rust projects in that workspace.

Review dependencies with `cargo tree --locked` and `cargo deny check licenses`.
