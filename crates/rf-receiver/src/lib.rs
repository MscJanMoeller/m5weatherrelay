//! Async SX1262 receiver using chip-neutral FSK settings.
#![cfg_attr(not(test), no_std)]

pub mod sx1262;
pub use rf_sensor::{CrcConfig, CrcLength, FskSettings, WhiteningConfig};
pub use sx1262::Sx1262;

/// Metadata for the packet copied into the caller's buffer.
#[derive(Debug, PartialEq, Eq)]
pub struct PacketInfo {
    pub len: usize,
    /// RSSI in half-dBm units, for example -103 represents -51.5 dBm.
    pub rssi_half_dbm: i16,
}
