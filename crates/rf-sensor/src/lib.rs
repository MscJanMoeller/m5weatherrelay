//! Chip-neutral RF profiles and allocation-free sensor registry.
#![no_std]

pub mod model;
mod profile;
pub use profile::{CrcConfig, CrcLength, FskSettings, WhiteningConfig};

/// A decoder's packet recognition and validation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeError {
    /// The packet is not recognized as belonging to this model.
    Unsupported,
    /// The packet was recognized but failed model-specific validation.
    Invalid(InvalidReason),
    /// The recognized packet has an unexpected length for this model.
    UnexpectedLength { expected: usize, actual: usize },
}

/// Details of a recognized but invalid packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidReason {
    EmptyPayload,
    /// Multiple packet lengths are accepted by the decoder.
    InvalidLength {
        expected: &'static [usize],
        actual: usize,
    },
    InvalidField {
        field: &'static str,
        expected: u64,
        actual: u64,
    },
    Integrity {
        scope: &'static str,
        /// Name of the failed check, such as "crc" or "checksum".
        algorithm: &'static str,
        expected: u64,
        actual: u64,
    },
    NonzeroPadding {
        offset: usize,
        value: u8,
    },
    /// A named protocol-specific validation failure without structured fields.
    Other {
        description: &'static str,
    },
}

/// Decode one byte-aligned payload after the radio has removed its sync word.
///
/// Implementations must use `Unsupported` for unrelated packets and return
/// sensor data only after the packet's required integrity checks pass.
pub trait Decoder {
    type SensorData;

    fn decode(&self, payload: &[u8]) -> Result<Self::SensorData, DecodeError>;
}

/// A named receive configuration that can be registered independently of models.
pub struct RfProfileDefinition {
    /// Globally unique, case-sensitive profile identifier.
    pub id: &'static str,
    pub fsk: FskSettings<'static>,
}

/// A sensor model, its vendor, compatible profiles, and typed decoder.
pub struct SensorModel<S> {
    /// Vendor name. Together with `model_id`, this is the registry key.
    pub vendor: &'static str,
    /// Model identifier within this vendor, distinct from an individual device ID.
    pub model_id: &'static str,
    /// IDs of explicitly compatible RF profiles.
    pub profiles: &'static [&'static str],
    /// Unique measurements this model supports, independently of any packet.
    pub capabilities: &'static [model::MeasurementKind],
    pub decode: fn(&[u8]) -> Result<S, DecodeError>,
}

impl<S> SensorModel<S> {
    /// Preserve a vendor's model metadata while adapting its decoded data type.
    pub const fn with_decoder<T>(
        &self,
        decode: fn(&[u8]) -> Result<T, DecodeError>,
    ) -> SensorModel<T> {
        SensorModel {
            vendor: self.vendor,
            model_id: self.model_id,
            profiles: self.profiles,
            capabilities: self.capabilities,
            decode,
        }
    }
}

/// An allocation-free registry of profiles and their compatible model decoders.
///
/// The capacities are the number of profiles, models, and models per profile.
/// Register profiles before models. Multi-vendor applications can use enums
/// and adapter functions to unify vendor-specific results. All registered
/// decoders use the shared [`DecodeError`] type.
pub struct Registry<S: 'static, const P: usize, const M: usize, const L: usize> {
    profiles: [Option<RfProfile<S, L>>; P],
    models: [Option<&'static SensorModel<S>>; M],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileSelectionError {
    ProfileNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistrationError {
    DuplicateProfile {
        id: &'static str,
    },
    DuplicateModel {
        vendor: &'static str,
        model_id: &'static str,
    },
    UnknownProfileReference {
        model: SensorModelKey,
        profile_id: &'static str,
    },
    DuplicateProfileReference {
        model: SensorModelKey,
        profile_id: &'static str,
    },
    DuplicateMeasurementCapability {
        model: SensorModelKey,
        measurement: model::MeasurementKind,
    },
    ProfileCapacityExceeded {
        capacity: usize,
    },
    ModelCapacityExceeded {
        capacity: usize,
    },
    ProfileModelCapacityExceeded {
        profile_id: &'static str,
        capacity: usize,
    },
}

/// Registry identity of a sensor model, not an individual device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SensorModelKey {
    pub vendor: &'static str,
    pub model_id: &'static str,
}

/// Sensor data and the model whose decoder validated it.
#[derive(Debug, PartialEq, Eq)]
pub struct Decoded<S> {
    pub model: SensorModelKey,
    pub data: S,
}

/// A recognized but invalid packet from one candidate decoder.
#[derive(Debug, PartialEq, Eq)]
pub struct InvalidPacket {
    pub model: SensorModelKey,
    pub error: DecodeError,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchError {
    UnexpectedLength {
        expected: usize,
        actual: usize,
    },
    Unsupported,
    Invalid {
        count: usize,
    },
    Ambiguous {
        first: SensorModelKey,
        second: SensorModelKey,
        count: usize,
    },
}

impl<S, const P: usize, const M: usize, const L: usize> Registry<S, P, M, L> {
    pub const fn new() -> Self {
        Self {
            profiles: [const { None }; P],
            models: [None; M],
        }
    }

    /// Build a registry from static definitions, registering profiles first.
    pub fn from_definitions(
        profiles: &'static [RfProfileDefinition],
        models: &'static [SensorModel<S>],
    ) -> Result<Self, RegistrationError> {
        let mut registry = Self::new();
        for profile in profiles {
            registry.register_profile(profile)?;
        }
        for model in models {
            registry.register_model(model)?;
        }
        Ok(registry)
    }

    pub fn profiles(&self) -> impl Iterator<Item = &RfProfile<S, L>> {
        self.profiles.iter().flatten()
    }

    pub fn models(&self) -> impl Iterator<Item = &SensorModel<S>> {
        self.models.iter().flatten().copied()
    }

    pub fn register_profile(
        &mut self,
        definition: &'static RfProfileDefinition,
    ) -> Result<(), RegistrationError> {
        if self.profiles().any(|profile| profile.id() == definition.id) {
            return Err(RegistrationError::DuplicateProfile { id: definition.id });
        }
        let slot = self
            .profiles
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegistrationError::ProfileCapacityExceeded { capacity: P })?;
        *slot = Some(RfProfile {
            definition,
            models: [None; L],
        });
        Ok(())
    }

    /// Add a model to every referenced profile, or leave the registry unchanged.
    pub fn register_model(
        &mut self,
        model: &'static SensorModel<S>,
    ) -> Result<(), RegistrationError> {
        let key = SensorModelKey {
            vendor: model.vendor,
            model_id: model.model_id,
        };
        if self
            .models()
            .any(|other| other.vendor == model.vendor && other.model_id == model.model_id)
        {
            return Err(RegistrationError::DuplicateModel {
                vendor: model.vendor,
                model_id: model.model_id,
            });
        }
        for (index, &measurement) in model.capabilities.iter().enumerate() {
            if model.capabilities[..index].contains(&measurement) {
                return Err(RegistrationError::DuplicateMeasurementCapability {
                    model: key,
                    measurement,
                });
            }
        }
        for (index, &reference) in model.profiles.iter().enumerate() {
            if model.profiles[..index].contains(&reference) {
                return Err(RegistrationError::DuplicateProfileReference {
                    model: key,
                    profile_id: reference,
                });
            }
            let profile = self
                .profiles()
                .find(|profile| profile.id() == reference)
                .ok_or(RegistrationError::UnknownProfileReference {
                    model: key,
                    profile_id: reference,
                })?;
            if profile.models.iter().all(Option::is_some) {
                return Err(RegistrationError::ProfileModelCapacityExceeded {
                    profile_id: reference,
                    capacity: L,
                });
            }
        }
        let model_slot = self
            .models
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegistrationError::ModelCapacityExceeded { capacity: M })?;
        *model_slot = Some(model);
        for profile in self.profiles.iter_mut().flatten() {
            if model.profiles.contains(&profile.id()) {
                // Capacity was checked for every referenced profile above.
                let slot = profile
                    .models
                    .iter_mut()
                    .find(|slot| slot.is_none())
                    .expect("validated profile model capacity");
                *slot = Some(model);
            }
        }
        Ok(())
    }

    /// Select by exact, case-sensitive profile ID.
    pub fn select_profile(
        &self,
        profile_id: &str,
    ) -> Result<&RfProfile<S, L>, ProfileSelectionError> {
        self.profiles()
            .find(|profile| profile.id() == profile_id)
            .ok_or(ProfileSelectionError::ProfileNotFound)
    }
}

impl<S, const P: usize, const M: usize, const L: usize> Default for Registry<S, P, M, L> {
    fn default() -> Self {
        Self::new()
    }
}

/// A registered profile and its compatible decoders.
/// Configure the receiver with its settings before decoding post-sync bytes.
pub struct RfProfile<S: 'static, const L: usize> {
    definition: &'static RfProfileDefinition,
    models: [Option<&'static SensorModel<S>>; L],
}

impl<S, const L: usize> RfProfile<S, L> {
    pub fn id(&self) -> &'static str {
        self.definition.id
    }

    pub fn fsk(&self) -> &FskSettings<'static> {
        &self.definition.fsk
    }

    pub fn models(&self) -> impl Iterator<Item = &SensorModel<S>> {
        self.models.iter().flatten().copied()
    }

    /// Reports every recognized invalid candidate through `on_invalid`, even if
    /// another decoder succeeds. Multiple successful candidates are ambiguous
    /// and no data is returned. The error lists the first two models and the
    /// total number of successful candidates.
    pub fn decode(
        &self,
        payload: &[u8],
        mut on_invalid: impl FnMut(InvalidPacket),
    ) -> Result<Decoded<S>, DispatchError> {
        let expected = usize::from(self.fsk().packet_length);
        if payload.len() != expected {
            return Err(DispatchError::UnexpectedLength {
                expected,
                actual: payload.len(),
            });
        }
        let mut first: Option<Decoded<S>> = None;
        let mut second = None;
        let mut matches = 0;
        let mut invalid = 0;
        for model in self.models() {
            let id = SensorModelKey {
                vendor: model.vendor,
                model_id: model.model_id,
            };
            match (model.decode)(payload) {
                Ok(data) => {
                    matches += 1;
                    if first.is_none() {
                        first = Some(Decoded { model: id, data });
                    } else if second.is_none() {
                        second = Some(id);
                    }
                }
                Err(DecodeError::Unsupported) => {}
                Err(error) => {
                    invalid += 1;
                    on_invalid(InvalidPacket { model: id, error });
                }
            }
        }
        match (first, second) {
            (Some(decoded), Some(second)) => Err(DispatchError::Ambiguous {
                first: decoded.model,
                second,
                count: matches,
            }),
            (Some(decoded), None) => Ok(decoded),
            (None, _) if invalid > 0 => Err(DispatchError::Invalid { count: invalid }),
            (None, _) => Err(DispatchError::Unsupported),
        }
    }
}
