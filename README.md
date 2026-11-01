# M5WeatherRelay
Rust firmware for the M5Stack Unit C6L, receiving Ecowitt weather sensor data and integrating it with Home Assistant via the ESPHome Native API.

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
117.3 kHz bandwidth, sync `2D D4` and fixed 64-byte packets. Every 10 seconds
the monitor reports packet count, IRQ flags and DIO1; received packets are
printed in hex with RSSI. Raw packets have been received on hardware but are
not yet validated or decoded; their source has not been confirmed.
Buzzer and RGB data pins are held low.

The receive-only `rf-receiver` crate uses async embedded-hal SPI,
GPIO waits and delays, and follows RadioLib concepts (`init_fsk`,
`start_receive`, `read_packet`). `start_receive` enters continuous receive
mode, so the radio keeps receiving after each packet, as with RadioLib's
`startReceive` and `readData`. Its `receive` method awaits the IRQ pin and
reads one packet. For its own reaction to IRQ flags the firmware uses the
lower-level `wait_for_irq_pin`, `read_irq_flags`, `read_packet` and
`clear_irq_flags`, which also retains diagnostics. `read_packet` clears the
packet flags first and reports `PacketOverrun` if the next packet starts
arriving while the buffer is read; error flags are left for the caller to
clear. Use `standby` and `start_receive` to stop and restart receiving. Do not
cancel `receive` during SPI traffic without checking that the radio still
receives.
The OLED and radio share the same async SPI bus.

The `FskProfile` and caller-owned receive buffer are chip-neutral; GPIO wiring,
IRQ flags and errors are chip-specific. An SX1276 driver can use the same
profile and receive concepts with a different constructor and implementation.
It is **not implemented**: SX1276 supports different bandwidth steps (including
125 kHz rather than SX1262's 117.3 kHz), so it requires a suitable profile.
The current SX1262 driver supports the chip's 150-960 MHz range, the GFSK
receive bandwidths of the `sx1262` driver (the WS90 profile uses 117.3 kHz), a 3 V
TCXO, a two-byte sync word, fixed-length packets and disabled radio CRC and
whitening. Unsupported settings are rejected. The image calibration uses the
datasheet value of the ISM band containing the frequency (430-440, 470-510,
779-787, 863-870 or 902-928 MHz); elsewhere it calibrates 4 MHz around the
frequency as RadioLib does, which Semtech does not specify. Only 868.350 MHz
is validated on hardware, and the board's matching network and antenna may
limit the usable band.
The receiver reuses the local [`sx1262`](../sx1262) async SPI device and FSK
commands, with caller-managed BUSY timing. Driver failures preserve SPI errors
and report the opcode and first byte of invalid responses.

Run host tests for both crates with:

```sh
cargo test --workspace --lib --target x86_64-pc-windows-msvc --locked
```

VS Code's Rust analyzer checks the embedded firmware without test targets;
the tests require the host target above. In a multi-root VS Code workspace,
set `rust-analyzer.cfg.setTest` and `rust-analyzer.check.allTargets` to `false`
in the workspace settings: folder-level settings for these options are ignored.
These settings also apply to other Rust projects in that workspace.

Review dependencies with `cargo tree --locked` and `cargo deny check licenses`.
