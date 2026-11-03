//! Application-level composition of RF profiles and sensor decoders.

use rf_sensor::model::{Attribute, DeviceId, Measurement, Sample};
use rf_sensor::{DecodeError, RegistrationError, Registry, RfProfile, SensorModel};

/// Sensor data from all vendors enabled by the application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorData {
    Ecowitt(rf_ecowitt::SensorData),
}

impl Sample for SensorData {
    fn device(&self) -> DeviceId {
        match self {
            Self::Ecowitt(data) => data.device(),
        }
    }

    fn for_each_measurement(&self, f: &mut impl FnMut(Measurement)) {
        match self {
            Self::Ecowitt(data) => data.for_each_measurement(f),
        }
    }

    fn for_each_attribute<'a>(&'a self, f: &mut impl FnMut(Attribute<'a>)) {
        match self {
            Self::Ecowitt(data) => data.for_each_attribute(f),
        }
    }
}

fn decode_ecowitt_ws90(payload: &[u8]) -> Result<SensorData, DecodeError> {
    (rf_ecowitt::WS90_MODEL.decode)(payload).map(SensorData::Ecowitt)
}

const WS90_MODEL: SensorModel<SensorData> =
    rf_ecowitt::WS90_MODEL.with_decoder(decode_ecowitt_ws90);
static MODELS: [SensorModel<SensorData>; 1] = [WS90_MODEL];

const PROFILE_CAPACITY: usize = rf_ecowitt::profiles::RF_PROFILES.len();
const MODEL_CAPACITY: usize = MODELS.len();
const MODELS_PER_PROFILE: usize = MODEL_CAPACITY;

pub type SensorRegistry =
    Registry<SensorData, PROFILE_CAPACITY, MODEL_CAPACITY, MODELS_PER_PROFILE>;
pub type SensorProfile = RfProfile<SensorData, MODELS_PER_PROFILE>;

/// Assemble enabled vendor profiles and adapted models without heap allocation.
pub fn registry() -> Result<SensorRegistry, RegistrationError> {
    let mut registry = SensorRegistry::new();
    for profile in &rf_ecowitt::profiles::RF_PROFILES {
        registry.register_profile(profile)?;
    }
    for model in &MODELS {
        registry.register_model(model)?;
    }
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rf_sensor::{Decoded, DispatchError, InvalidReason, SensorModelKey};

    const CAPTURE: [u8; 39] = [
        0x90, 0x01, 0x13, 0x6c, 0x00, 0x00, 0x6e, 0x89, 0xea, 0x4f, 0x09, 0x4b, 0x12, 0x00, 0x3f,
        0xff, 0x70, 0x05, 0x13, 0x00, 0x08, 0x55, 0x20, 0x06, 0x6b, 0xa6, 0x96, 0xfb, 0x03, 0xa0,
        0x78, 0xac, 0x4c, 0xdd, 0x33, 0x57, 0x1f, 0x13, 0x3d,
    ];

    fn ws90_packet() -> [u8; rf_ecowitt::ws90::RADIO_LEN] {
        let mut packet = [0; rf_ecowitt::ws90::RADIO_LEN];
        packet[..CAPTURE.len()].copy_from_slice(&CAPTURE);
        packet
    }

    #[test]
    fn application_registry_preserves_vendor_metadata_and_data() {
        let registry = registry().unwrap();
        let profile = registry
            .select_profile(rf_ecowitt::profiles::WS90_868_ID)
            .unwrap();
        let model = profile.models().next().unwrap();
        assert_eq!(model.vendor, rf_ecowitt::WS90_MODEL.vendor);
        assert_eq!(model.model_id, rf_ecowitt::WS90_MODEL.model_id);
        assert_eq!(model.profiles, rf_ecowitt::WS90_MODEL.profiles);
        assert_eq!(model.capabilities, rf_ecowitt::WS90_MODEL.capabilities);

        let decoded = profile
            .decode(&ws90_packet(), |_| panic!("unexpected diagnostic"))
            .unwrap();
        assert_eq!(
            decoded.model,
            SensorModelKey {
                vendor: rf_ecowitt::VENDOR,
                model_id: rf_ecowitt::WS90_MODEL_ID,
            }
        );
        let mut projected = Vec::new();
        decoded
            .data
            .for_each_measurement(&mut |item| projected.push(item));
        assert_eq!(
            projected.iter().map(Measurement::kind).collect::<Vec<_>>(),
            model.capabilities
        );
        assert_eq!(decoded.data.device().model, decoded.model);
        let SensorData::Ecowitt(rf_ecowitt::SensorData::Ws90(data)) = decoded.data;
        assert_eq!(data.id, 0x01136c);
    }

    #[derive(Debug, PartialEq, Eq)]
    enum CombinedData {
        Application(SensorData),
        Other(u8),
    }

    fn decode_application(payload: &[u8]) -> Result<CombinedData, DecodeError> {
        (WS90_MODEL.decode)(payload).map(CombinedData::Application)
    }

    fn decode_other_vendor(payload: &[u8]) -> Result<u8, DecodeError> {
        match payload {
            [0xb0, value, ..] => Ok(*value),
            [0xb1, ..] => Err(DecodeError::Invalid(InvalidReason::Other {
                description: "other vendor invalid packet",
            })),
            _ => Err(DecodeError::Unsupported),
        }
    }

    fn adapt_other_vendor(payload: &[u8]) -> Result<CombinedData, DecodeError> {
        (OTHER_VENDOR_MODEL.decode)(payload).map(CombinedData::Other)
    }

    const OTHER_VENDOR_MODEL: SensorModel<u8> = SensorModel {
        vendor: "Example vendor",
        model_id: "example",
        profiles: &[rf_ecowitt::profiles::WS90_868_ID],
        capabilities: &[],
        decode: decode_other_vendor,
    };

    static COMBINED_MODELS: [SensorModel<CombinedData>; 2] = [
        WS90_MODEL.with_decoder(decode_application),
        OTHER_VENDOR_MODEL.with_decoder(adapt_other_vendor),
    ];

    #[test]
    fn application_model_composes_with_another_vendor_on_a_shared_profile() {
        let registry = Registry::<CombinedData, 4, 2, 2>::from_definitions(
            &rf_ecowitt::profiles::RF_PROFILES,
            &COMBINED_MODELS,
        )
        .unwrap();
        let profile = registry
            .select_profile(rf_ecowitt::profiles::WS90_868_ID)
            .unwrap();
        assert_eq!(profile.models().count(), 2);

        let decoded = profile
            .decode(&ws90_packet(), |_| panic!("unexpected diagnostic"))
            .unwrap();
        assert_eq!(decoded.model.vendor, rf_ecowitt::VENDOR);
        assert!(matches!(
            decoded.data,
            CombinedData::Application(SensorData::Ecowitt(rf_ecowitt::SensorData::Ws90(_)))
        ));

        let mut other = [0; rf_ecowitt::ws90::RADIO_LEN];
        other[0] = 0xb0;
        other[1] = 42;
        assert_eq!(
            profile.decode(&other, |_| panic!("unexpected diagnostic")),
            Ok(Decoded {
                model: SensorModelKey {
                    vendor: "Example vendor",
                    model_id: "example",
                },
                data: CombinedData::Other(42),
            })
        );
        let other_profile = registry
            .select_profile(rf_ecowitt::profiles::WS90_915_ID)
            .unwrap();
        assert_eq!(
            other_profile.decode(&other, |_| panic!("unexpected diagnostic")),
            Err(DispatchError::Unsupported)
        );

        other[0] = 0xb1;
        let mut invalid = std::vec::Vec::new();
        assert_eq!(
            profile.decode(&other, |item| invalid.push(item)),
            Err(DispatchError::Invalid { count: 1 })
        );
        assert_eq!(invalid[0].model.vendor, "Example vendor");
        assert_eq!(
            invalid[0].error,
            DecodeError::Invalid(InvalidReason::Other {
                description: "other vendor invalid packet",
            })
        );

        let mut corrupt = ws90_packet();
        corrupt[30] ^= 1;
        invalid.clear();
        assert_eq!(
            profile.decode(&corrupt, |item| invalid.push(item)),
            Err(DispatchError::Invalid { count: 1 })
        );
        assert_eq!(invalid[0].model.vendor, rf_ecowitt::VENDOR);
        assert!(matches!(
            invalid[0].error,
            DecodeError::Invalid(InvalidReason::Integrity {
                scope: "core",
                algorithm: "crc",
                ..
            })
        ));
    }
}
