use rf_receiver::FskProfile;

pub const WS90_868: FskProfile<'static> = FskProfile {
    frequency_hz: 868_350_000,
    bit_rate: 17_200,
    frequency_deviation_hz: 35_000,
    rx_bandwidth_hz: 117_300,
    preamble_bits: 48,
    sync_word: &[0x2d, 0xd4],
    packet_length: 64,
    crc_enabled: false,
    whitening_enabled: false,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uses_validated_868_mhz_radio_profile() {
        assert_eq!(WS90_868.frequency_hz, 868_350_000);
        assert_eq!(WS90_868.bit_rate, 17_200);
        assert_eq!(WS90_868.frequency_deviation_hz, 35_000);
        assert_eq!(WS90_868.rx_bandwidth_hz, 117_300);
        assert_eq!(WS90_868.preamble_bits, 48);
        assert_eq!(WS90_868.sync_word, &[0x2d, 0xd4]);
        assert_eq!(WS90_868.packet_length, 64);
        assert_eq!(
            (WS90_868.crc_enabled, WS90_868.whitening_enabled),
            (false, false)
        );
    }
}
