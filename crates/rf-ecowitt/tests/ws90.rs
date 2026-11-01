use rf_ecowitt::{Error, SensorData, ws90};
use rf_sensor::{DecodeError, Decoded, Decoder, DispatchError, Registry, SensorModelKey};

// Selector-1 capture from WS90_SX1262_Bringup/WS90_FW_REVERSE_ENGINEERING.md.
const CAPTURE: [u8; 39] = [
    0x90, 0x01, 0x13, 0x6c, 0x00, 0x00, 0x6e, 0x89, 0xea, 0x4f, 0x09, 0x4b, 0x12, 0x00, 0x3f, 0xff,
    0x70, 0x05, 0x13, 0x00, 0x08, 0x55, 0x20, 0x06, 0x6b, 0xa6, 0x96, 0xfb, 0x03, 0xa0, 0x78, 0xac,
    0x4c, 0xdd, 0x33, 0x57, 0x1f, 0x13, 0x3d,
];

// Packets reported from the Rust firmware running on the Unit C6L.
const FIRMWARE_CAPTURES: [[u8; ws90::RADIO_LEN]; 3] = [
    [
        0x90, 0x01, 0x13, 0x6c, 0x00, 0x00, 0x77, 0x89, 0xe0, 0x50, 0x00, 0x32, 0x07, 0x00, 0x3f,
        0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0xff, 0x5f, 0xe7, 0x00, 0x00, 0xa0,
        0x5e, 0x02, 0x4b, 0x41, 0x2d, 0x54, 0x1f, 0x23, 0x53, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ],
    [
        0x90, 0x01, 0x13, 0x6c, 0x00, 0x00, 0x77, 0x89, 0xe0, 0x50, 0x00, 0x3d, 0x06, 0x00, 0x3f,
        0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0xff, 0x5f, 0xe8, 0x00, 0x00, 0xa0,
        0x5a, 0x09, 0x4b, 0x41, 0x2d, 0x43, 0x1f, 0xc0, 0xed, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ],
    [
        0x90, 0x01, 0x13, 0x6c, 0x00, 0x00, 0x77, 0x89, 0xe0, 0x50, 0x06, 0x75, 0x07, 0x00, 0x3f,
        0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0x08, 0x00, 0x00, 0xff, 0x5f, 0xe7, 0x00, 0x00, 0xa0,
        0x6e, 0x5b, 0x4b, 0x39, 0x2d, 0x23, 0x1f, 0x7c, 0x25, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ],
];

fn registry() -> Registry<SensorData, Error, 4, 1, 1> {
    Registry::from_definitions(&rf_ecowitt::profiles::RF_PROFILES, &rf_ecowitt::MODELS).unwrap()
}

#[test]
fn decodes_firmware_capture_values() {
    let registry = registry();
    let profile = registry
        .select_profile(rf_ecowitt::profiles::WS90_868_ID)
        .unwrap();
    for (packet, average, direction, gust, extension) in [
        (&FIRMWARE_CAPTURES[0], 0, 50, 7, [75, 65, 45, 84, 31]),
        (&FIRMWARE_CAPTURES[1], 0, 61, 6, [75, 65, 45, 67, 31]),
        (&FIRMWARE_CAPTURES[2], 6, 117, 7, [75, 57, 45, 35, 31]),
    ] {
        let decoded = profile
            .decode(packet, |_| panic!("unexpected invalid candidate"))
            .unwrap();
        assert_eq!(
            decoded.model,
            SensorModelKey {
                vendor: rf_ecowitt::VENDOR,
                model_id: rf_ecowitt::WS90_MODEL_ID,
            }
        );
        let SensorData::Ws90(data) = decoded.data;
        assert_eq!(data.id, 70508);
        assert_eq!(data.light_lux, Some(0));
        assert_eq!(data.battery_mv, 2380);
        assert_eq!(data.flags, 137);
        assert_eq!(data.temperature_tenths_c, Some(80));
        assert_eq!(data.humidity_percent, Some(80));
        assert_eq!(data.wind_average_tenths_m_s, Some(average));
        assert_eq!(data.wind_direction_degrees, Some(direction));
        assert_eq!(data.wind_gust_tenths_m_s, Some(gust));
        assert_eq!(data.uv_index_tenths, Some(0));
        assert_eq!(data.pressure_raw, None);
        assert_eq!(data.rain_total_tenths_mm, 0);
        assert!(!data.rain_started);
        assert_eq!(data.supercap_tenths_v, 8);
        assert_eq!(data.firmware_raw, 160);
        assert_eq!(data.raw_core, packet[..ws90::CORE_LEN]);
        assert_eq!(data.extension, Some(extension));
        assert_eq!(data.integrity, ws90::Integrity::ExtendedRecord);
    }
}

#[test]
fn vendor_and_model_decoders_agree_for_ws90_inputs() {
    let mut radio = [0; ws90::RADIO_LEN];
    radio[..CAPTURE.len()].copy_from_slice(&CAPTURE);
    let mut corrupt = radio;
    corrupt[30] ^= 1;
    let mut unrelated = radio;
    unrelated[0] = 0x80;
    for payload in [
        &[][..],
        &CAPTURE[..ws90::CORE_LEN],
        &CAPTURE[..],
        &radio[..],
        &corrupt[..],
        &unrelated[..],
        &[ws90::FAMILY][..],
    ] {
        assert_eq!(
            rf_ecowitt::Decoder.decode(payload),
            (rf_ecowitt::WS90_MODEL.decode)(payload)
        );
    }
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = (crc << 1) ^ if crc & 0x80 != 0 { 0x31 } else { 0 };
        }
    }
    crc
}

fn repair_core(packet: &mut [u8]) {
    packet[30] = crc8(&packet[..30]);
    packet[31] = packet[..31].iter().fold(0u8, |sum, &b| sum.wrapping_add(b));
}

fn repair_record(packet: &mut [u8]) {
    repair_core(packet);
    packet[37] = crc8(&packet[..37]);
    packet[38] = packet[..38].iter().fold(0u8, |sum, &b| sum.wrapping_add(b));
}

#[test]
fn decodes_captured_record_and_padded_radio_packet() {
    let mut radio = [0; ws90::RADIO_LEN];
    radio[..CAPTURE.len()].copy_from_slice(&CAPTURE);
    let record = ws90::decode(&CAPTURE).unwrap();
    assert_eq!(ws90::decode(&radio), Ok(record.clone()));
    assert_eq!(
        rf_ecowitt::Decoder.decode(&radio),
        Ok(SensorData::Ws90(record.clone()))
    );
    let registry = registry();
    let profile = registry
        .select_profile(rf_ecowitt::profiles::WS90_868_ID)
        .unwrap();
    assert_eq!(
        profile.decode(&radio, |_| panic!("unexpected invalid candidate")),
        Ok(Decoded {
            model: SensorModelKey {
                vendor: "Ecowitt",
                model_id: "ws90"
            },
            data: SensorData::Ws90(record.clone()),
        })
    );
    assert_eq!(record.integrity, ws90::Integrity::ExtendedRecord);
    assert_eq!(record.id, 0x01136c);
    assert_eq!(record.light_lux, Some(0));
    assert_eq!(record.battery_mv, 2200);
    assert_eq!(record.temperature_tenths_c, Some(90));
    assert_eq!(record.humidity_percent, Some(79));
    assert_eq!(record.wind_average_tenths_m_s, Some(9));
    assert_eq!(record.wind_direction_degrees, Some(75));
    assert_eq!(record.wind_gust_tenths_m_s, Some(18));
    assert_eq!(record.uv_index_tenths, Some(0));
    assert_eq!(record.pressure_raw, None);
    assert_eq!(record.rain_total_tenths_mm, 8);
    assert!(record.rain_started);
    assert_eq!(record.supercap_tenths_v, 21);
    assert_eq!(record.firmware_raw, 0xa0);
    assert_eq!(record.raw_core, CAPTURE[..32]);
    assert_eq!(record.extension, Some([0x4c, 0xdd, 0x33, 0x57, 0x1f]));
}

#[test]
fn accepts_core_only_without_claiming_extension_validation() {
    let data = ws90::decode(&CAPTURE[..ws90::CORE_LEN]).unwrap();
    assert_eq!(data.integrity, ws90::Integrity::Core);
    assert_eq!(data.extension, None);
}

#[test]
fn profile_dispatches_distinct_ws90_device_ids() {
    let registry = registry();
    let profile = registry
        .select_profile(rf_ecowitt::profiles::WS90_868_ID)
        .unwrap();
    let mut other_record = CAPTURE;
    other_record[1..4].copy_from_slice(&[0x05, 0x06, 0x07]);
    repair_record(&mut other_record);
    for (record, expected_id) in [(&CAPTURE, 0x01136c), (&other_record, 0x050607)] {
        let mut packet = [0; ws90::RADIO_LEN];
        packet[..record.len()].copy_from_slice(record);
        let decoded = profile
            .decode(&packet, |_| panic!("unexpected invalid candidate"))
            .unwrap();
        assert_eq!(
            decoded.model,
            SensorModelKey {
                vendor: "Ecowitt",
                model_id: "ws90"
            }
        );
        let SensorData::Ws90(data) = decoded.data;
        assert_eq!(data.id, expected_id);
    }
}

#[test]
fn registry_distinguishes_framing_unsupported_and_invalid_packets() {
    let registry = registry();
    let profile = registry
        .select_profile(rf_ecowitt::profiles::WS90_868_ID)
        .unwrap();
    assert_eq!(
        profile.decode(&CAPTURE, |_| panic!("unexpected invalid candidate")),
        Err(DispatchError::UnexpectedLength {
            expected: ws90::RADIO_LEN,
            actual: CAPTURE.len(),
        })
    );
    let mut packet = [0; ws90::RADIO_LEN];
    packet[..CAPTURE.len()].copy_from_slice(&CAPTURE);
    packet[0] = 0x80;
    assert_eq!(
        profile.decode(&packet, |_| panic!("unexpected invalid candidate")),
        Err(DispatchError::Unsupported)
    );
    packet[0] = ws90::FAMILY;
    packet[30] ^= 1;
    let mut invalid = Vec::new();
    assert_eq!(
        profile.decode(&packet, |diagnostic| invalid.push(diagnostic)),
        Err(DispatchError::Invalid { count: 1 })
    );
    assert_eq!(
        invalid[0].model,
        SensorModelKey {
            vendor: "Ecowitt",
            model_id: "ws90"
        }
    );
    assert!(matches!(
        invalid[0].error,
        DecodeError::Invalid(Error::Ws90(ws90::Error::CoreCrc { .. }))
    ));
}

#[test]
fn rejects_unknown_family_and_every_unsupported_length() {
    assert_eq!(
        rf_ecowitt::Decoder.decode(&[]),
        Err(DecodeError::Invalid(Error::EmptyPayload))
    );
    assert_eq!(
        rf_ecowitt::Decoder.decode(&[0x80]),
        Err(DecodeError::Unsupported)
    );
    let mut unknown = CAPTURE;
    unknown[0] = 0x80;
    assert_eq!(
        ws90::decode(&unknown),
        Err(ws90::Error::InvalidFamily(0x80))
    );
    let bytes = [0u8; 65];
    for len in 0..=65 {
        if ![32, 39, 64].contains(&len) {
            assert_eq!(
                ws90::decode(&bytes[..len]),
                Err(ws90::Error::InvalidLength(len))
            );
        }
    }
}

#[test]
fn rejects_each_integrity_failure_and_nonzero_padding() {
    let mut packet = CAPTURE;
    packet[30] ^= 1;
    assert!(matches!(
        ws90::decode(&packet),
        Err(ws90::Error::CoreCrc { .. })
    ));
    packet = CAPTURE;
    packet[31] ^= 1;
    assert!(matches!(
        ws90::decode(&packet),
        Err(ws90::Error::CoreChecksum { .. })
    ));
    packet = CAPTURE;
    packet[37] ^= 1;
    assert!(matches!(
        ws90::decode(&packet),
        Err(ws90::Error::RecordCrc { .. })
    ));
    packet = CAPTURE;
    packet[38] ^= 1;
    assert!(matches!(
        ws90::decode(&packet),
        Err(ws90::Error::RecordChecksum { .. })
    ));
    let mut radio = [0; ws90::RADIO_LEN];
    radio[..CAPTURE.len()].copy_from_slice(&CAPTURE);
    for offset in ws90::RECORD_LEN..ws90::RADIO_LEN {
        radio[offset] = 0x80;
        assert_eq!(
            ws90::decode(&radio),
            Err(ws90::Error::NonzeroPadding {
                offset,
                value: 0x80
            })
        );
        radio[offset] = 0;
    }
}

#[test]
fn rejects_every_single_bit_corruption_in_the_captured_record() {
    for offset in 0..CAPTURE.len() {
        for bit in 0..8 {
            let mut packet = CAPTURE;
            packet[offset] ^= 1 << bit;
            assert!(ws90::decode(&packet).is_err(), "offset {offset}, bit {bit}");
        }
    }
}

#[test]
fn decodes_high_bits_negative_temperatures_and_invalid_sentinels() {
    let mut core = [0; ws90::CORE_LEN];
    core.copy_from_slice(&CAPTURE[..ws90::CORE_LEN]);
    core[4..6].copy_from_slice(&[0xff, 0xff]);
    core[7] = 0x70;
    core[8] = 0;
    core[9] = 0xff;
    core[10..13].copy_from_slice(&[0xff, 0xff, 0xff]);
    core[13] = 0xff;
    core[14..16].copy_from_slice(&[0x03, 0xe8]);
    repair_core(&mut core);
    let data = ws90::decode(&core).unwrap();
    assert_eq!(data.temperature_tenths_c, Some(-400));
    assert_eq!(data.light_lux, None);
    assert_eq!(data.humidity_percent, None);
    assert_eq!(data.wind_average_tenths_m_s, None);
    assert_eq!(data.wind_direction_degrees, None);
    assert_eq!(data.wind_gust_tenths_m_s, None);
    assert_eq!(data.uv_index_tenths, None);
    assert_eq!(data.pressure_raw, Some(1000));

    core[7] = 0x73;
    core[8] = 0xff;
    core[10..13].copy_from_slice(&[0x00, 0x01, 0x02]);
    repair_core(&mut core);
    let data = ws90::decode(&core).unwrap();
    assert_eq!(data.temperature_tenths_c, None);
    assert_eq!(data.wind_average_tenths_m_s, Some(256));
    assert_eq!(data.wind_direction_degrees, Some(257));
    assert_eq!(data.wind_gust_tenths_m_s, Some(258));
}
