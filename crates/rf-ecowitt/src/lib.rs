//! Ecowitt/Fine Offset packet decoding, independent of radio hardware.
#![no_std]

pub mod profiles;
pub mod ws90;

use rf_sensor::{DecodeError, SensorModel};

/// Decoded data for supported Ecowitt sensor families.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SensorData {
    Ws90(ws90::SensorData),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    EmptyPayload,
    Ws90(ws90::Error),
}

/// Dispatches byte-aligned Ecowitt payloads by their family byte.
pub struct Decoder;

impl rf_sensor::Decoder for Decoder {
    type SensorData = SensorData;
    type Error = Error;

    fn decode(&self, payload: &[u8]) -> Result<Self::SensorData, DecodeError<Self::Error>> {
        decode_ws90(payload)
    }
}

pub const VENDOR: &str = "Ecowitt";
pub const WS90_MODEL_ID: &str = "ws90";

fn decode_ws90(payload: &[u8]) -> Result<SensorData, DecodeError<Error>> {
    match payload.first().copied() {
        None => Err(DecodeError::Invalid(Error::EmptyPayload)),
        Some(ws90::FAMILY) => ws90::decode(payload)
            .map(SensorData::Ws90)
            .map_err(|error| DecodeError::Invalid(Error::Ws90(error))),
        Some(_) => Err(DecodeError::Unsupported),
    }
}

pub const WS90_MODEL: SensorModel<SensorData, Error> = SensorModel {
    vendor: VENDOR,
    model_id: WS90_MODEL_ID,
    profiles: &profiles::WS90_PROFILE_IDS,
    decode: decode_ws90,
};

pub static MODELS: [SensorModel<SensorData, Error>; 1] = [WS90_MODEL];
