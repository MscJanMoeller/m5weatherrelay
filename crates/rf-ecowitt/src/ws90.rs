//! WS90 core and v1.6.2 application-record decoding.
//!
//! See `doc/design.md` for packet provenance and validation constraints.

pub const FAMILY: u8 = 0x90;
pub const CORE_LEN: usize = 32;
pub const RECORD_LEN: usize = 39;
pub const RADIO_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Integrity {
    Core,
    ExtendedRecord,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    InvalidLength(usize),
    InvalidFamily(u8),
    CoreCrc { expected: u8, actual: u8 },
    CoreChecksum { expected: u8, actual: u8 },
    RecordCrc { expected: u8, actual: u8 },
    RecordChecksum { expected: u8, actual: u8 },
    NonzeroPadding { offset: usize, value: u8 },
}

/// Validated values; units are encoded in field names to avoid floating point.
/// Missing measurements carry the protocol's invalid sentinel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SensorData {
    /// 24-bit ID transmitted by this individual sensor, not the registry model ID.
    pub id: u32,
    pub light_lux: Option<u32>,
    pub battery_mv: u16,
    pub flags: u8,
    pub temperature_tenths_c: Option<i16>,
    pub humidity_percent: Option<u8>,
    pub wind_average_tenths_m_s: Option<u16>,
    pub wind_direction_degrees: Option<u16>,
    pub wind_gust_tenths_m_s: Option<u16>,
    pub uv_index_tenths: Option<u8>,
    /// The non-sentinel value is raw: the pressure scale is not confirmed.
    pub pressure_raw: Option<u16>,
    pub rain_total_tenths_mm: u16,
    pub rain_started: bool,
    pub supercap_tenths_v: u8,
    pub firmware_raw: u8,
    /// The entire validated core, including unknown fields and its trailer.
    pub raw_core: [u8; CORE_LEN],
    /// Uninterpreted extension; only present when its trailer was validated.
    pub extension: Option<[u8; 5]>,
    pub integrity: Integrity,
}

/// Decode exactly a 32-byte core, 39-byte record, or padded 64-byte RF payload.
/// An extended record is accepted only if both integrity layers pass.
pub fn decode(payload: &[u8]) -> Result<SensorData, Error> {
    match payload.len() {
        CORE_LEN | RECORD_LEN | RADIO_LEN => {}
        len => return Err(Error::InvalidLength(len)),
    }
    if payload[0] != FAMILY {
        return Err(Error::InvalidFamily(payload[0]));
    }
    check_crc(&payload[..30], payload[30], true)?;
    check_sum(&payload[..31], payload[31], true)?;

    let (integrity, extension) = if payload.len() >= RECORD_LEN {
        check_crc(&payload[..37], payload[37], false)?;
        check_sum(&payload[..38], payload[38], false)?;
        if payload.len() == RADIO_LEN {
            for (offset, &value) in payload.iter().enumerate().skip(RECORD_LEN) {
                if value != 0 {
                    return Err(Error::NonzeroPadding { offset, value });
                }
            }
        }
        let mut extension = [0; 5];
        extension.copy_from_slice(&payload[32..37]);
        (Integrity::ExtendedRecord, Some(extension))
    } else {
        (Integrity::Core, None)
    };

    let be16 = |offset: usize| u16::from_be_bytes([payload[offset], payload[offset + 1]]);
    let flags = payload[7];
    let temperature = (u16::from(flags & 0x03) << 8) | u16::from(payload[8]);
    let wind_average = (u16::from(flags & 0x10) << 4) | u16::from(payload[10]);
    let wind_direction = (u16::from(flags & 0x20) << 3) | u16::from(payload[11]);
    let wind_gust = (u16::from(flags & 0x40) << 2) | u16::from(payload[12]);
    let light = be16(4);
    let pressure = be16(14);
    let mut raw_core = [0; CORE_LEN];
    raw_core.copy_from_slice(&payload[..CORE_LEN]);
    Ok(SensorData {
        id: (u32::from(payload[1]) << 16) | (u32::from(payload[2]) << 8) | u32::from(payload[3]),
        light_lux: (light != 0xffff).then_some(u32::from(light) * 10),
        battery_mv: u16::from(payload[6]) * 20,
        flags,
        temperature_tenths_c: (temperature != 0x3ff).then_some(temperature as i16 - 400),
        humidity_percent: (payload[9] != 0xff).then_some(payload[9]),
        wind_average_tenths_m_s: (wind_average != 0x1ff).then_some(wind_average),
        wind_direction_degrees: (wind_direction != 0x1ff).then_some(wind_direction),
        wind_gust_tenths_m_s: (wind_gust != 0x1ff).then_some(wind_gust),
        uv_index_tenths: (payload[13] != 0xff).then_some(payload[13]),
        pressure_raw: (pressure != 0x3fff).then_some(pressure),
        rain_total_tenths_mm: be16(19),
        rain_started: payload[16] & 0x10 != 0,
        supercap_tenths_v: payload[21] & 0x3f,
        firmware_raw: payload[29],
        raw_core,
        extension,
        integrity,
    })
}

fn crc8(bytes: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &byte in bytes {
        crc ^= byte;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 {
                (crc << 1) ^ 0x31
            } else {
                crc << 1
            };
        }
    }
    crc
}

fn check_crc(bytes: &[u8], actual: u8, core: bool) -> Result<(), Error> {
    let expected = crc8(bytes);
    if expected == actual {
        Ok(())
    } else if core {
        Err(Error::CoreCrc { expected, actual })
    } else {
        Err(Error::RecordCrc { expected, actual })
    }
}

fn check_sum(bytes: &[u8], actual: u8, core: bool) -> Result<(), Error> {
    let expected = bytes.iter().fold(0u8, |sum, &byte| sum.wrapping_add(byte));
    if expected == actual {
        Ok(())
    } else if core {
        Err(Error::CoreChecksum { expected, actual })
    } else {
        Err(Error::RecordChecksum { expected, actual })
    }
}
