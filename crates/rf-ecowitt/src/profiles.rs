//! SX1262-compatible receiver profiles recovered from WS90 v1.6.2.
//!
//! Only the 868.350 MHz profile has a confirmed hardware capture in the separate
//! bring-up project. Other frequencies need capture testing with a matched board.
use rf_sensor::{FskSettings, RfProfileDefinition};

const fn ws90(frequency_hz: u32) -> FskSettings<'static> {
    FskSettings {
        frequency_hz,
        bit_rate: 17_200,
        frequency_deviation_hz: 35_000,
        rx_bandwidth_hz: 117_300,
        preamble_bits: 48,
        sync_word: &[0x2d, 0xd4],
        packet_length: 64,
        crc: None,
        whitening: None,
    }
}

pub const WS90_433_925: FskSettings<'static> = ws90(433_925_000);
pub const WS90_868_350: FskSettings<'static> = ws90(868_350_000);
pub const WS90_915_000: FskSettings<'static> = ws90(915_000_000);
pub const WS90_920_900: FskSettings<'static> = ws90(920_900_000);

pub const WS90_433_ID: &str = "ecowitt-ws90-v1-433925";
pub const WS90_868_ID: &str = "ecowitt-ws90-v1-868350";
pub const WS90_915_ID: &str = "ecowitt-ws90-v1-915000";
pub const WS90_920_ID: &str = "ecowitt-ws90-v1-920900";

pub static RF_PROFILES: [RfProfileDefinition; 4] = [
    RfProfileDefinition {
        id: WS90_433_ID,
        fsk: WS90_433_925,
    },
    RfProfileDefinition {
        id: WS90_868_ID,
        fsk: WS90_868_350,
    },
    RfProfileDefinition {
        id: WS90_915_ID,
        fsk: WS90_915_000,
    },
    RfProfileDefinition {
        id: WS90_920_ID,
        fsk: WS90_920_900,
    },
];

pub static WS90_PROFILE_IDS: [&str; 4] = [WS90_433_ID, WS90_868_ID, WS90_915_ID, WS90_920_ID];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_share_the_ws90_modem_settings() {
        for (profile, frequency) in [
            (WS90_433_925, 433_925_000),
            (WS90_868_350, 868_350_000),
            (WS90_915_000, 915_000_000),
            (WS90_920_900, 920_900_000),
        ] {
            assert_eq!(profile.frequency_hz, frequency);
            assert_eq!(profile.bit_rate, 17_200);
            assert_eq!(profile.frequency_deviation_hz, 35_000);
            assert_eq!(profile.rx_bandwidth_hz, 117_300);
            assert_eq!(profile.preamble_bits, 48);
            assert_eq!(profile.sync_word, &[0x2d, 0xd4]);
            assert_eq!(profile.packet_length, 64);
            assert_eq!((profile.crc, profile.whitening), (None, None));
        }
    }
}
