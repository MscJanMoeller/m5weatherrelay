use rf_sensor::model::MeasurementKind;
use rf_sensor::{
    DecodeError, Decoded, Decoder, DispatchError, FskSettings, InvalidPacket, InvalidReason,
    ProfileSelectionError, RegistrationError, Registry, RfProfileDefinition, SensorModel,
    SensorModelKey,
};

#[derive(Debug, PartialEq, Eq)]
enum SensorData {
    VendorA(u8),
    VendorB(u8),
    Other(u8),
}

const INVALID_A: InvalidReason = InvalidReason::Other {
    description: "vendor A",
};
const INVALID_B: InvalidReason = InvalidReason::Other {
    description: "vendor B",
};
const INVALID_OTHER: InvalidReason = InvalidReason::Other {
    description: "other",
};

const fn fsk() -> FskSettings<'static> {
    FskSettings {
        frequency_hz: 868_350_000,
        bit_rate: 17_200,
        frequency_deviation_hz: 35_000,
        rx_bandwidth_hz: 117_300,
        preamble_bits: 48,
        sync_word: &[0x2d, 0xd4],
        packet_length: 2,
        crc: None,
        whitening: None,
    }
}

static PROFILES: [RfProfileDefinition; 2] = [
    RfProfileDefinition {
        id: "shared",
        fsk: fsk(),
    },
    RfProfileDefinition {
        id: "other",
        fsk: FskSettings {
            sync_word: &[0xab, 0xcd],
            ..fsk()
        },
    },
];

struct VendorA;

impl Decoder for VendorA {
    type SensorData = u8;
    fn decode(&self, bytes: &[u8]) -> Result<u8, DecodeError> {
        match bytes {
            [0xa0 | 0xdd, value] => Ok(*value),
            [0xee | 0xef, _] => Err(DecodeError::Invalid(INVALID_A)),
            _ => Err(DecodeError::Unsupported),
        }
    }
}

struct VendorB;

impl Decoder for VendorB {
    type SensorData = u8;
    fn decode(&self, bytes: &[u8]) -> Result<u8, DecodeError> {
        match bytes {
            [0xb0 | 0xdd | 0xef, value] => Ok(*value),
            [0xee, _] => Err(DecodeError::Invalid(INVALID_B)),
            _ => Err(DecodeError::Unsupported),
        }
    }
}

fn decode_a(bytes: &[u8]) -> Result<SensorData, DecodeError> {
    VendorA.decode(bytes).map(SensorData::VendorA)
}

fn decode_b(bytes: &[u8]) -> Result<SensorData, DecodeError> {
    VendorB.decode(bytes).map(SensorData::VendorB)
}

fn decode_other(bytes: &[u8]) -> Result<SensorData, DecodeError> {
    match bytes {
        [0xc0 | 0xdd, value] => Ok(SensorData::Other(*value)),
        _ => Err(DecodeError::Unsupported),
    }
}

fn decode_invalid(bytes: &[u8]) -> Result<SensorData, DecodeError> {
    match bytes {
        [0xdd, _] => Err(DecodeError::Invalid(INVALID_OTHER)),
        _ => Err(DecodeError::Unsupported),
    }
}

static MODELS: [SensorModel<SensorData>; 5] = [
    SensorModel {
        vendor: "Vendor A",
        model_id: "shared-model",
        profiles: &["shared"],
        capabilities: &[],
        decode: decode_a,
    },
    SensorModel {
        vendor: "Vendor B",
        model_id: "shared-model",
        profiles: &["shared"],
        capabilities: &[],
        decode: decode_b,
    },
    SensorModel {
        vendor: "Vendor C",
        model_id: "other-model",
        profiles: &["other"],
        capabilities: &[],
        decode: decode_other,
    },
    SensorModel {
        vendor: "Vendor D",
        model_id: "overlap",
        profiles: &["shared"],
        capabilities: &[],
        decode: decode_other,
    },
    SensorModel {
        vendor: "Vendor E",
        model_id: "invalid",
        profiles: &["shared"],
        capabilities: &[],
        decode: decode_invalid,
    },
];
type TestRegistry = Registry<SensorData, 2, 5, 5>;

fn registry() -> TestRegistry {
    TestRegistry::from_definitions(&PROFILES, &MODELS).unwrap()
}

fn model(vendor: &'static str, model_id: &'static str) -> SensorModelKey {
    SensorModelKey { vendor, model_id }
}

#[test]
fn selected_profile_dispatches_all_compatible_vendors_and_devices() {
    let registry = registry();
    let active = registry.select_profile("shared").unwrap();
    assert_eq!(registry.profiles().count(), 2);
    assert_eq!(registry.models().count(), 5);
    assert_eq!(active.models().count(), 4);
    assert_eq!(active.id(), "shared");
    for (packet, expected) in [
        (
            [0xa0, 1],
            Decoded {
                model: model("Vendor A", "shared-model"),
                data: SensorData::VendorA(1),
            },
        ),
        (
            [0xa0, 2],
            Decoded {
                model: model("Vendor A", "shared-model"),
                data: SensorData::VendorA(2),
            },
        ),
        (
            [0xb0, 3],
            Decoded {
                model: model("Vendor B", "shared-model"),
                data: SensorData::VendorB(3),
            },
        ),
    ] {
        assert_eq!(
            active.decode(&packet, |_| panic!("unexpected diagnostic")),
            Ok(expected)
        );
    }
}

#[test]
fn does_not_dispatch_models_for_another_profile_even_at_the_same_frequency() {
    let registry = registry();
    let shared = registry.select_profile("shared").unwrap();
    let other = registry.select_profile("other").unwrap();
    assert_eq!(shared.fsk().frequency_hz, other.fsk().frequency_hz);
    assert_ne!(shared.fsk().sync_word, other.fsk().sync_word);
    assert_eq!(other.models().count(), 1);
    assert_eq!(
        shared.decode(&[0xc0, 5], |_| panic!("unexpected diagnostic")),
        Ok(Decoded {
            model: model("Vendor D", "overlap"),
            data: SensorData::Other(5),
        })
    );
    assert_eq!(
        other.decode(&[0xa0, 5], |_| panic!("unexpected diagnostic")),
        Err(DispatchError::Unsupported)
    );
    assert_eq!(
        other.decode(&[0xc0, 5], |_| panic!("unexpected diagnostic")),
        Ok(Decoded {
            model: model("Vendor C", "other-model"),
            data: SensorData::Other(5)
        })
    );
}

#[test]
fn distinguishes_unsupported_framing_and_all_invalid_candidates() {
    let registry = registry();
    let active = registry.select_profile("shared").unwrap();
    assert_eq!(
        active.decode(&[0xa0], |_| panic!("unexpected diagnostic")),
        Err(DispatchError::UnexpectedLength {
            expected: 2,
            actual: 1
        })
    );
    assert_eq!(
        active.decode(&[0x01, 0], |_| panic!("unexpected diagnostic")),
        Err(DispatchError::Unsupported)
    );
    let mut invalid = Vec::new();
    assert_eq!(
        active.decode(&[0xee, 0], |item| invalid.push(item)),
        Err(DispatchError::Invalid { count: 2 })
    );
    assert_eq!(
        invalid,
        [
            InvalidPacket {
                model: model("Vendor A", "shared-model"),
                error: DecodeError::Invalid(INVALID_A)
            },
            InvalidPacket {
                model: model("Vendor B", "shared-model"),
                error: DecodeError::Invalid(INVALID_B)
            },
        ]
    );
}

#[test]
fn reports_invalid_candidates_even_when_another_model_succeeds() {
    let registry = registry();
    let active = registry.select_profile("shared").unwrap();
    let mut invalid = Vec::new();
    assert_eq!(
        active.decode(&[0xef, 9], |item| invalid.push(item)),
        Ok(Decoded {
            model: model("Vendor B", "shared-model"),
            data: SensorData::VendorB(9)
        })
    );
    assert_eq!(
        invalid,
        [InvalidPacket {
            model: model("Vendor A", "shared-model"),
            error: DecodeError::Invalid(INVALID_A),
        }]
    );
}

#[test]
fn reports_decoder_length_error_with_model_identity() {
    fn wrong_length(_bytes: &[u8]) -> Result<SensorData, DecodeError> {
        Err(DecodeError::UnexpectedLength {
            expected: 3,
            actual: 2,
        })
    }
    static MODEL: SensorModel<SensorData> = SensorModel {
        vendor: "Vendor A",
        model_id: "length-check",
        profiles: &["shared"],
        capabilities: &[],
        decode: wrong_length,
    };
    let mut registry = Registry::<SensorData, 1, 1, 1>::new();
    registry.register_profile(&PROFILES[0]).unwrap();
    registry.register_model(&MODEL).unwrap();
    let mut invalid = Vec::new();
    assert_eq!(
        registry
            .select_profile("shared")
            .unwrap()
            .decode(&[0xa0, 0], |item| invalid.push(item)),
        Err(DispatchError::Invalid { count: 1 })
    );
    assert_eq!(
        invalid,
        [InvalidPacket {
            model: model("Vendor A", "length-check"),
            error: DecodeError::UnexpectedLength {
                expected: 3,
                actual: 2,
            },
        }]
    );
}

#[test]
fn reports_ambiguous_matches_without_returning_data_and_still_reports_errors() {
    let registry = registry();
    let active = registry.select_profile("shared").unwrap();
    let mut invalid = Vec::new();
    assert_eq!(
        active.decode(&[0xdd, 9], |item| invalid.push(item)),
        Err(DispatchError::Ambiguous {
            first: model("Vendor A", "shared-model"),
            second: model("Vendor B", "shared-model"),
            count: 3,
        })
    );
    assert_eq!(
        invalid,
        [InvalidPacket {
            model: model("Vendor E", "invalid"),
            error: DecodeError::Invalid(INVALID_OTHER),
        }]
    );
}

#[test]
fn reports_unknown_or_case_mismatched_profile() {
    let registry = registry();
    assert!(matches!(
        registry.select_profile("missing"),
        Err(ProfileSelectionError::ProfileNotFound)
    ));
    assert!(matches!(
        registry.select_profile("Shared"),
        Err(ProfileSelectionError::ProfileNotFound)
    ));
}

#[test]
fn rejects_duplicate_profile_ids_and_duplicate_vendor_model_keys() {
    static DUPLICATE_PROFILES: [RfProfileDefinition; 2] = [
        RfProfileDefinition {
            id: "same",
            fsk: fsk(),
        },
        RfProfileDefinition {
            id: "same",
            fsk: fsk(),
        },
    ];
    static DUPLICATE_MODELS: [SensorModel<SensorData>; 2] = [
        SensorModel {
            vendor: "Vendor A",
            model_id: "same",
            profiles: &["shared"],
            capabilities: &[],
            decode: decode_a,
        },
        SensorModel {
            vendor: "Vendor A",
            model_id: "same",
            profiles: &["other"],
            capabilities: &[],
            decode: decode_b,
        },
    ];
    assert!(matches!(
        TestRegistry::from_definitions(&DUPLICATE_PROFILES, &[]),
        Err(RegistrationError::DuplicateProfile { id: "same" })
    ));
    assert!(matches!(
        TestRegistry::from_definitions(&PROFILES, &DUPLICATE_MODELS),
        Err(RegistrationError::DuplicateModel {
            vendor: "Vendor A",
            model_id: "same"
        })
    ));
}

#[test]
fn rejects_unknown_or_repeated_model_profile_references() {
    static UNKNOWN: [SensorModel<SensorData>; 1] = [SensorModel {
        vendor: "Vendor A",
        model_id: "model",
        profiles: &["missing"],
        capabilities: &[],
        decode: decode_a,
    }];
    static REPEATED: [SensorModel<SensorData>; 1] = [SensorModel {
        vendor: "Vendor A",
        model_id: "model",
        profiles: &["shared", "shared"],
        capabilities: &[],
        decode: decode_a,
    }];
    assert!(matches!(
        TestRegistry::from_definitions(&PROFILES, &UNKNOWN),
        Err(RegistrationError::UnknownProfileReference {
            model: SensorModelKey {
                vendor: "Vendor A",
                model_id: "model"
            },
            profile_id: "missing"
        })
    ));
    assert!(matches!(
        TestRegistry::from_definitions(&PROFILES, &REPEATED),
        Err(RegistrationError::DuplicateProfileReference {
            model: SensorModelKey {
                vendor: "Vendor A",
                model_id: "model"
            },
            profile_id: "shared"
        })
    ));
}

#[test]
fn rejects_duplicate_measurement_capabilities_without_registering_the_model() {
    const TEMPERATURE: MeasurementKind = MeasurementKind::Temperature;
    static DUPLICATE: SensorModel<SensorData> = SensorModel {
        vendor: "Vendor A",
        model_id: "duplicate-capability",
        profiles: &["shared"],
        capabilities: &[TEMPERATURE, TEMPERATURE],
        decode: decode_a,
    };
    let mut registry = Registry::<SensorData, 2, 1, 1>::new();
    registry.register_profile(&PROFILES[0]).unwrap();
    assert_eq!(
        registry.register_model(&DUPLICATE),
        Err(RegistrationError::DuplicateMeasurementCapability {
            model: model("Vendor A", "duplicate-capability"),
            measurement: TEMPERATURE,
        })
    );
    assert_eq!(registry.models().count(), 0);
    assert_eq!(
        registry.select_profile("shared").unwrap().models().count(),
        0
    );
}

static BOTH_PROFILES: SensorModel<SensorData> = SensorModel {
    vendor: "Vendor F",
    model_id: "both",
    profiles: &["shared", "other"],
    capabilities: &[],
    decode: decode_b,
};

#[test]
fn registration_adds_model_links_to_each_compatible_profile() {
    let mut registry = Registry::<SensorData, 2, 3, 2>::new();
    registry.register_profile(&PROFILES[0]).unwrap();
    registry.register_profile(&PROFILES[1]).unwrap();
    assert_eq!(
        registry.select_profile("shared").unwrap().models().count(),
        0
    );
    assert_eq!(
        registry
            .select_profile("shared")
            .unwrap()
            .decode(&[0xb0, 7], |_| panic!("unexpected diagnostic")),
        Err(DispatchError::Unsupported)
    );
    registry.register_model(&MODELS[0]).unwrap();
    registry.register_model(&MODELS[2]).unwrap();
    registry.register_model(&BOTH_PROFILES).unwrap();
    for (profile_id, expected_vendor) in [("shared", "Vendor A"), ("other", "Vendor C")] {
        let profile = registry.select_profile(profile_id).unwrap();
        let vendors: Vec<_> = profile.models().map(|model| model.vendor).collect();
        assert_eq!(vendors, [expected_vendor, "Vendor F"]);
        assert_eq!(
            profile.decode(&[0xb0, 7], |_| panic!("unexpected diagnostic")),
            Ok(Decoded {
                model: model("Vendor F", "both"),
                data: SensorData::VendorB(7),
            })
        );
    }
    assert_eq!(registry.models().count(), 3);
}

#[test]
fn registration_failure_does_not_add_partial_model_links() {
    static MISSING: SensorModel<SensorData> = SensorModel {
        vendor: "Vendor F",
        model_id: "missing",
        profiles: &["shared", "missing"],
        capabilities: &[],
        decode: decode_a,
    };
    static REPEATED: SensorModel<SensorData> = SensorModel {
        vendor: "Vendor F",
        model_id: "repeated",
        profiles: &["shared", "shared"],
        capabilities: &[],
        decode: decode_a,
    };
    let mut registry = Registry::<SensorData, 2, 3, 1>::new();
    registry.register_profile(&PROFILES[0]).unwrap();
    registry.register_profile(&PROFILES[1]).unwrap();
    registry.register_model(&MODELS[2]).unwrap();
    assert!(matches!(
        registry.register_model(&MISSING),
        Err(RegistrationError::UnknownProfileReference {
            profile_id: "missing",
            ..
        })
    ));
    assert!(matches!(
        registry.register_model(&REPEATED),
        Err(RegistrationError::DuplicateProfileReference {
            profile_id: "shared",
            ..
        })
    ));
    assert_eq!(
        registry.register_model(&BOTH_PROFILES),
        Err(RegistrationError::ProfileModelCapacityExceeded {
            profile_id: "other",
            capacity: 1
        })
    );
    assert_eq!(registry.models().count(), 1);
    assert_eq!(
        registry.select_profile("shared").unwrap().models().count(),
        0
    );
    assert_eq!(
        registry.select_profile("other").unwrap().models().count(),
        1
    );
    registry.register_model(&MODELS[0]).unwrap();
    assert_eq!(
        registry.select_profile("shared").unwrap().models().count(),
        1
    );
}

#[test]
fn registration_reports_capacity_without_mutating_the_registry() {
    let mut registry = Registry::<SensorData, 1, 1, 2>::new();
    registry.register_profile(&PROFILES[0]).unwrap();
    assert_eq!(
        registry.register_profile(&PROFILES[1]),
        Err(RegistrationError::ProfileCapacityExceeded { capacity: 1 })
    );
    assert_eq!(
        registry.register_profile(&PROFILES[0]),
        Err(RegistrationError::DuplicateProfile { id: "shared" })
    );
    registry.register_model(&MODELS[0]).unwrap();
    assert_eq!(
        registry.register_model(&MODELS[1]),
        Err(RegistrationError::ModelCapacityExceeded { capacity: 1 })
    );
    assert_eq!(registry.profiles().count(), 1);
    assert_eq!(registry.models().count(), 1);
    assert_eq!(
        registry.select_profile("shared").unwrap().models().count(),
        1
    );

    let mut empty = Registry::<SensorData, 0, 0, 0>::new();
    assert_eq!(
        empty.register_profile(&PROFILES[0]),
        Err(RegistrationError::ProfileCapacityExceeded { capacity: 0 })
    );

    let mut no_links = Registry::<SensorData, 1, 1, 0>::new();
    no_links.register_profile(&PROFILES[0]).unwrap();
    assert_eq!(
        no_links.register_model(&MODELS[0]),
        Err(RegistrationError::ProfileModelCapacityExceeded {
            profile_id: "shared",
            capacity: 0
        })
    );
    assert_eq!(no_links.models().count(), 0);
}
