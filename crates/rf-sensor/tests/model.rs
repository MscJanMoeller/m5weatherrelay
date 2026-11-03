use rf_sensor::{
    SensorModelKey,
    model::{
        Attribute, AttributeKind, AttributeValue, DeviceId, Measurement, MeasurementError,
        MeasurementKind, MeasurementValue, Received, Sample, Unit,
    },
};

#[test]
fn each_numeric_kind_has_one_canonical_unit() {
    for (kind, unit) in [
        (MeasurementKind::Temperature, Unit::Celsius),
        (MeasurementKind::Humidity, Unit::Percent),
        (MeasurementKind::WindAverage, Unit::MetresPerSecond),
        (MeasurementKind::WindGust, Unit::MetresPerSecond),
        (MeasurementKind::WindDirection, Unit::Degrees),
        (MeasurementKind::Light, Unit::Lux),
        (MeasurementKind::UvIndex, Unit::UvIndex),
        (MeasurementKind::RainTotal, Unit::Millimetre),
        (MeasurementKind::BatteryVoltage, Unit::Volt),
        (MeasurementKind::SupercapVoltage, Unit::Volt),
    ] {
        assert!(kind.is_numeric());
        assert_eq!(kind.unit(), unit);
        assert_eq!(Measurement::numeric(kind, 1.0).unwrap().unit(), unit);
        assert_eq!(
            Measurement::binary(kind, true),
            Err(MeasurementError::ExpectedBinary { kind })
        );
    }
    assert!(!MeasurementKind::RainActive.is_numeric());
    assert_eq!(MeasurementKind::RainActive.unit(), Unit::None);
    for active in [false, true] {
        let measurement = Measurement::binary(MeasurementKind::RainActive, active).unwrap();
        assert_eq!(measurement.value(), MeasurementValue::Binary(active));
        assert_eq!(measurement.unit(), Unit::None);
    }
}

struct MetadataSample {
    version: [u8; 3],
}

impl Sample for MetadataSample {
    fn device(&self) -> DeviceId {
        DeviceId {
            model: SensorModelKey {
                vendor: "Example",
                model_id: "weather",
            },
            device: 42,
        }
    }

    fn for_each_measurement(&self, _f: &mut impl FnMut(Measurement)) {}

    fn for_each_attribute<'a>(&'a self, f: &mut impl FnMut(Attribute<'a>)) {
        f(Attribute {
            kind: AttributeKind::FirmwareVersion,
            value: AttributeValue::Bytes(&self.version),
        });
    }
}

#[test]
fn attribute_visitor_can_retain_values_borrowed_from_the_sample() {
    let sample = MetadataSample {
        version: [0xff, 0x00, 0x61],
    };
    let mut attributes = Vec::new();
    sample.for_each_attribute(&mut |attribute| attributes.push(attribute));
    assert_eq!(
        attributes,
        [Attribute {
            kind: AttributeKind::FirmwareVersion,
            value: AttributeValue::Bytes(&sample.version),
        }]
    );
}

#[test]
fn measurement_identity_and_unit_are_derived_from_kind() {
    let first = Measurement::numeric(MeasurementKind::Temperature, 12.0).unwrap();
    let next = Measurement::numeric(MeasurementKind::Temperature, 13.0).unwrap();
    let other = Measurement::numeric(MeasurementKind::Humidity, 12.0).unwrap();
    let binary = Measurement::binary(MeasurementKind::RainActive, false).unwrap();
    assert_eq!(first.kind(), next.kind());
    assert_ne!(first.kind(), other.kind());
    assert_eq!(first.kind(), MeasurementKind::Temperature);
    assert_eq!(binary.kind(), MeasurementKind::RainActive);
    assert_eq!(first.value(), MeasurementValue::Numeric(12.0));
    assert_eq!(binary.value(), MeasurementValue::Binary(false));
    assert_eq!(first.unit(), Unit::Celsius);
    assert_eq!(binary.unit(), Unit::None);
}

#[test]
fn measurement_rejects_mismatched_and_nonfinite_values() {
    assert_eq!(
        Measurement::numeric(MeasurementKind::RainActive, 1.0),
        Err(MeasurementError::ExpectedNumeric {
            kind: MeasurementKind::RainActive,
        })
    );
    assert_eq!(
        Measurement::binary(MeasurementKind::WindAverage, true),
        Err(MeasurementError::ExpectedBinary {
            kind: MeasurementKind::WindAverage,
        })
    );
    for value in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            Measurement::numeric(MeasurementKind::WindAverage, value),
            Err(MeasurementError::NonFiniteValue {
                kind: MeasurementKind::WindAverage,
            })
        );
    }
}

#[test]
fn identity_timestamp_and_borrowed_attributes_preserve_values() {
    let id = DeviceId {
        model: SensorModelKey {
            vendor: "Example",
            model_id: "weather",
        },
        device: 42,
    };
    let packet = Received {
        data: id,
        received_at_unix: None,
    };
    assert_eq!(packet.data, id);
    assert_eq!(packet.received_at_unix, None);

    let bytes = [0xff, 0x00, 0x61];
    assert_eq!(AttributeValue::Bytes(&bytes), AttributeValue::Bytes(&bytes));
    assert_ne!(AttributeValue::Text("a"), AttributeValue::Bytes(b"a"));
}
