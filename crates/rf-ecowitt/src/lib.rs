//! Ecowitt/Fine Offset packet decoding, independent of radio hardware.
#![no_std]

pub mod profiles;
pub mod ws90;

use rf_sensor::model::{Attribute, DeviceId, Measurement, Sample};
use rf_sensor::{DecodeError, InvalidReason, SensorModel};

/// Decoded data for supported Ecowitt sensor families.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorData {
    Ws90(ws90::SensorData),
}

impl Sample for SensorData {
    fn device(&self) -> DeviceId {
        match self {
            Self::Ws90(data) => data.device(),
        }
    }

    fn for_each_measurement(&self, f: &mut impl FnMut(Measurement)) {
        match self {
            Self::Ws90(data) => data.for_each_measurement(f),
        }
    }

    fn for_each_attribute<'a>(&'a self, f: &mut impl FnMut(Attribute<'a>)) {
        match self {
            Self::Ws90(data) => data.for_each_attribute(f),
        }
    }
}

impl From<ws90::Error> for DecodeError {
    fn from(error: ws90::Error) -> Self {
        let reason = match error {
            ws90::Error::InvalidLength(actual) => InvalidReason::InvalidLength {
                expected: &[ws90::CORE_LEN, ws90::RECORD_LEN, ws90::RADIO_LEN],
                actual,
            },
            ws90::Error::InvalidFamily(actual) => InvalidReason::InvalidField {
                field: "family",
                expected: u64::from(ws90::FAMILY),
                actual: u64::from(actual),
            },
            ws90::Error::CoreCrc { expected, actual } => InvalidReason::Integrity {
                scope: "core",
                algorithm: "crc",
                expected: u64::from(expected),
                actual: u64::from(actual),
            },
            ws90::Error::CoreChecksum { expected, actual } => InvalidReason::Integrity {
                scope: "core",
                algorithm: "checksum",
                expected: u64::from(expected),
                actual: u64::from(actual),
            },
            ws90::Error::RecordCrc { expected, actual } => InvalidReason::Integrity {
                scope: "record",
                algorithm: "crc",
                expected: u64::from(expected),
                actual: u64::from(actual),
            },
            ws90::Error::RecordChecksum { expected, actual } => InvalidReason::Integrity {
                scope: "record",
                algorithm: "checksum",
                expected: u64::from(expected),
                actual: u64::from(actual),
            },
            ws90::Error::NonzeroPadding { offset, value } => {
                InvalidReason::NonzeroPadding { offset, value }
            }
        };
        Self::Invalid(reason)
    }
}

/// Dispatches byte-aligned Ecowitt payloads by their family byte.
pub struct Decoder;

impl rf_sensor::Decoder for Decoder {
    type SensorData = SensorData;
    fn decode(&self, payload: &[u8]) -> Result<Self::SensorData, DecodeError> {
        decode_ws90(payload)
    }
}

pub const VENDOR: &str = "Ecowitt";
pub const WS90_MODEL_ID: &str = "ws90";

fn decode_ws90(payload: &[u8]) -> Result<SensorData, DecodeError> {
    match payload.first().copied() {
        None => Err(DecodeError::Invalid(InvalidReason::EmptyPayload)),
        Some(ws90::FAMILY) => ws90::decode(payload)
            .map(SensorData::Ws90)
            .map_err(DecodeError::from),
        Some(_) => Err(DecodeError::Unsupported),
    }
}

pub const WS90_MODEL: SensorModel<SensorData> = SensorModel {
    vendor: VENDOR,
    model_id: WS90_MODEL_ID,
    profiles: &profiles::WS90_PROFILE_IDS,
    capabilities: &ws90::MEASUREMENT_CAPABILITIES,
    decode: decode_ws90,
};

pub static MODELS: [SensorModel<SensorData>; 1] = [WS90_MODEL];
