//! Chip-neutral FSK settings and an async SX1262 receiver.
#![cfg_attr(not(test), no_std)]

pub mod sx1262;
pub use sx1262::Sx1262;

/// Chip-neutral fixed-length FSK settings. Each radio validates its own supported
/// frequencies, bandwidths, sync lengths, packet lengths and encoding options.
pub struct FskProfile<'a> {
    /// Carrier frequency in hertz.
    pub frequency_hz: u32,
    /// Bit rate in bits per second.
    pub bit_rate: u32,
    /// Frequency deviation in hertz.
    pub frequency_deviation_hz: u32,
    /// Requested receiver bandwidth in hertz.
    pub rx_bandwidth_hz: u32,
    /// Preamble length in bits. A receiver uses it, with the sync word length, to choose
    /// how many preamble bits it requires before looking for the sync word.
    pub preamble_bits: u16,
    /// Sync word bytes in transmission order.
    pub sync_word: &'a [u8],
    /// Fixed payload length in bytes, not counting a CRC.
    pub packet_length: u8,
    /// Hardware CRC the radio checks, or `None` if packets carry none.
    pub crc: Option<CrcConfig>,
    /// Data whitening the radio removes, or `None` if packets are not whitened.
    pub whitening: Option<WhiteningConfig>,
}

/// Length of a hardware CRC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrcLength {
    OneByte,
    TwoBytes,
}

/// Hardware CRC settings. They must match the transmitter's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CrcConfig {
    pub length: CrcLength,
    /// Value the CRC register starts from.
    pub initial: u16,
    /// Generator polynomial.
    pub polynomial: u16,
    /// Whether the transmitted CRC is the bitwise inverse of the computed one.
    pub inverted: bool,
}

impl CrcConfig {
    /// 16-bit CCITT CRC: polynomial 0x1021, initial value 0x1D0F, inverted. These
    /// are RadioLib's defaults and the datasheet's CCITT example.
    pub const CCITT: Self = Self {
        length: CrcLength::TwoBytes,
        initial: 0x1d0f,
        polynomial: 0x1021,
        inverted: true,
    };
}

impl Default for CrcConfig {
    fn default() -> Self {
        Self::CCITT
    }
}

/// Data whitening settings. They must match the transmitter's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhiteningConfig {
    /// Seed of the 9-bit whitening shift register (x^9 + x^5 + 1).
    pub initial: u16,
}

impl WhiteningConfig {
    /// Seed 0x01FF, as RadioLib uses for compatibility with SX127x radios. The
    /// SX1262's own reset value is 0x0100.
    pub const DEFAULT: Self = Self { initial: 0x01ff };
}

impl Default for WhiteningConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Metadata for the packet copied into the caller's buffer.
#[derive(Debug, PartialEq, Eq)]
pub struct PacketInfo {
    pub len: usize,
    /// RSSI in half-dBm units, for example -103 represents -51.5 dBm.
    pub rssi_half_dbm: i16,
}
