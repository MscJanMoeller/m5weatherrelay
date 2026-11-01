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
    /// Fixed payload length in bytes.
    pub packet_length: u8,
    /// Whether the radio checks its hardware CRC.
    pub crc_enabled: bool,
    pub whitening_enabled: bool,
}

/// Metadata for the packet copied into the caller's buffer.
#[derive(Debug, PartialEq, Eq)]
pub struct PacketInfo {
    pub len: usize,
    /// RSSI in half-dBm units, for example -103 represents -51.5 dBm.
    pub rssi_half_dbm: i16,
}
