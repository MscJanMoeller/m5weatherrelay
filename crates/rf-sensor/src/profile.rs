/// Chip-neutral fixed-length FSK settings. Each radio validates its own supported
/// frequencies, bandwidths, sync lengths, packet lengths and encoding options.
pub struct FskSettings<'a> {
    /// Carrier frequency in hertz.
    pub frequency_hz: u32,
    /// Bit rate in bits per second.
    pub bit_rate: u32,
    /// Frequency deviation in hertz.
    pub frequency_deviation_hz: u32,
    /// Requested receiver bandwidth in hertz.
    pub rx_bandwidth_hz: u32,
    /// Preamble length in bits.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrcLength {
    OneByte,
    TwoBytes,
}

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
    /// RadioLib's CCITT defaults: polynomial 0x1021, initial 0x1D0F, inverted.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WhiteningConfig {
    /// Seed of the 9-bit whitening shift register (x^9 + x^5 + 1).
    pub initial: u16,
}

impl WhiteningConfig {
    /// Seed 0x01FF, as RadioLib uses for compatibility with SX127x radios.
    pub const DEFAULT: Self = Self { initial: 0x01ff };
}

impl Default for WhiteningConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}
