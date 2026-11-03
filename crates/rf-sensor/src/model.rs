//! Allocation-free, vendor-neutral projection of decoded sensor packets.
//!
//! Decoders retain protocol-specific and diagnostic data. This module describes
//! publishable measurements without depending on any particular integration.

use crate::SensorModelKey;

/// Canonical unit of a measurement, or `None` when it has no unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    Celsius,
    Percent,
    MetresPerSecond,
    Degrees,
    Lux,
    /// Dimensionless UV index.
    UvIndex,
    Millimetre,
    Volt,
}

/// A measurement capability and its stable identity within a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MeasurementKind {
    Temperature,
    /// Relative humidity.
    Humidity,
    /// The sensor's reported average; its averaging interval is model-specific.
    WindAverage,
    WindGust,
    /// Direction in degrees clockwise from north, normalized to [0, 360).
    WindDirection,
    /// Illuminance, not solar irradiance.
    Light,
    UvIndex,
    /// The sensor's accumulated rain counter, not a rate or a period total.
    /// Reset and rollover behavior is model-specific.
    RainTotal,
    BatteryVoltage,
    SupercapVoltage,
    /// The sensor's rain detection status; its clearing behavior is model-specific.
    RainActive,
}

impl MeasurementKind {
    /// The canonical unit, or `Unit::None` for a measurement without a unit.
    pub const fn unit(self) -> Unit {
        match self {
            Self::Temperature => Unit::Celsius,
            Self::Humidity => Unit::Percent,
            Self::WindAverage | Self::WindGust => Unit::MetresPerSecond,
            Self::WindDirection => Unit::Degrees,
            Self::Light => Unit::Lux,
            Self::UvIndex => Unit::UvIndex,
            Self::RainTotal => Unit::Millimetre,
            Self::BatteryVoltage | Self::SupercapVoltage => Unit::Volt,
            Self::RainActive => Unit::None,
        }
    }

    /// Whether this kind accepts a numeric value rather than a binary value.
    pub const fn is_numeric(self) -> bool {
        match self {
            Self::Temperature
            | Self::Humidity
            | Self::WindAverage
            | Self::WindGust
            | Self::WindDirection
            | Self::Light
            | Self::UvIndex
            | Self::RainTotal
            | Self::BatteryVoltage
            | Self::SupercapVoltage => true,
            Self::RainActive => false,
        }
    }
}

/// The value of a numeric or binary measurement.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MeasurementValue {
    Numeric(f32),
    Binary(bool),
}

/// An invalid measurement value for its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeasurementError {
    ExpectedNumeric { kind: MeasurementKind },
    ExpectedBinary { kind: MeasurementKind },
    NonFiniteValue { kind: MeasurementKind },
}

/// One available measurement emitted by a [`Sample`].
///
/// Numeric values are finite and expressed in the kind's canonical unit.
/// Implementors must not publish sentinel values as measurements.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Measurement {
    kind: MeasurementKind,
    value: MeasurementValue,
}

impl Measurement {
    pub fn numeric(kind: MeasurementKind, value: f32) -> Result<Self, MeasurementError> {
        if !kind.is_numeric() {
            return Err(MeasurementError::ExpectedNumeric { kind });
        }
        if !value.is_finite() {
            return Err(MeasurementError::NonFiniteValue { kind });
        }
        Ok(Self {
            kind,
            value: MeasurementValue::Numeric(value),
        })
    }

    pub fn binary(kind: MeasurementKind, active: bool) -> Result<Self, MeasurementError> {
        if kind.is_numeric() {
            return Err(MeasurementError::ExpectedBinary { kind });
        }
        Ok(Self {
            kind,
            value: MeasurementValue::Binary(active),
        })
    }

    pub const fn kind(&self) -> MeasurementKind {
        self.kind
    }

    pub const fn value(&self) -> MeasurementValue {
        self.value
    }

    /// The canonical unit, or `Unit::None` for a measurement without a unit.
    pub const fn unit(&self) -> Unit {
        self.kind.unit()
    }
}

/// Borrowed metadata values; text and opaque bytes are distinct.
///
/// The referenced data is owned by the sample or is static. No allocation or
/// fixed-capacity, externally supplied length is required.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttributeValue<'a> {
    U8(u8),
    U16(u16),
    U32(u32),
    Text(&'a str),
    Bytes(&'a [u8]),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attribute<'a> {
    pub kind: AttributeKind,
    pub value: AttributeValue<'a>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributeKind {
    FirmwareVersion,
}

/// Identity transmitted by an individual device, distinct from its model.
///
/// RF identifiers can collide or change on re-pairing. Publishers should use
/// the complete `(model, device, measurement kind)` and document those limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceId {
    pub model: SensorModelKey,
    pub device: u32,
}

/// Application-owned receive metadata associated with decoded data.
///
/// Unix time is optional because the receiver may not have a synchronized
/// clock. Applications should track monotonic last-seen times separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Received<T> {
    pub data: T,
    pub received_at_unix: Option<u64>,
}

/// A vendor-neutral, complete snapshot of a device's measurements.
///
/// Supported measurements are declared independently on `SensorModel`.
/// A supported measurement omitted from a snapshot is unavailable in that
/// snapshot; the consumer must not retain an earlier value as if it were
/// current. Implementations must emit only
/// supported kinds, at most once per packet, in deterministic order. Protocols
/// that send partial updates require state assembly before implementing this
/// snapshot contract. Absence of packets and stale values are handled by the
/// application, not this trait.
pub trait Sample {
    fn device(&self) -> DeviceId;

    fn for_each_measurement(&self, f: &mut impl FnMut(Measurement));

    fn for_each_attribute<'a>(&'a self, f: &mut impl FnMut(Attribute<'a>));
}
