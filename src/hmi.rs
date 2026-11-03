//! Allocation-free sensor presentation and single-button navigation.

use core::fmt::Write;
use heapless::{String, Vec};
use rf_sensor::model::{DeviceId, Measurement, MeasurementKind as Kind, MeasurementValue, Sample};

pub const DEVICE_CAPACITY: usize = 16;
const KIND_COUNT: usize = 11;
const ROWS_PER_PAGE: usize = 5;
const DEBOUNCE_MS: u64 = 30;
const LONG_PRESS_MS: u64 = 800;

const GROUPS: [&[Kind]; 4] = [
    &[Kind::Temperature, Kind::Humidity],
    &[Kind::WindAverage, Kind::WindGust, Kind::WindDirection],
    &[
        Kind::RainTotal,
        Kind::RainActive,
        Kind::Light,
        Kind::UvIndex,
    ],
    &[Kind::BatteryVoltage, Kind::SupercapVoltage],
];
const TITLES: [&str; 4] = ["CLIMATE", "WIND", "RAIN/SUN", "POWER"];

const fn index(kind: Kind) -> usize {
    match kind {
        Kind::Temperature => 0,
        Kind::Humidity => 1,
        Kind::WindAverage => 2,
        Kind::WindGust => 3,
        Kind::WindDirection => 4,
        Kind::Light => 5,
        Kind::UvIndex => 6,
        Kind::RainTotal => 7,
        Kind::BatteryVoltage => 8,
        Kind::SupercapVoltage => 9,
        Kind::RainActive => 10,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateError {
    CapacityExceeded,
    UnsupportedMeasurement(Kind),
    DuplicateMeasurement(Kind),
    CapabilitiesChanged,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Update {
    Existing,
    Discovered([u8; 2]),
}

#[derive(Clone)]
struct Device {
    id: DeviceId,
    alias: [u8; 2],
    capabilities: &'static [Kind],
    values: [Option<Measurement>; KIND_COUNT],
    last_seen_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Page {
    Home,
    Group { group: usize, page: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub lines: [String<10>; 6],
    pub home_temperature: Option<String<10>>,
}

impl Screen {
    fn new() -> Self {
        Self {
            lines: core::array::from_fn(|_| String::new()),
            home_temperature: None,
        }
    }
}

pub struct Hmi {
    home: Option<DeviceId>,
    stale_after_ms: Option<u64>,
    devices: Vec<Device, DEVICE_CAPACITY>,
    next_alias: u16,
    page: Page,
}

impl Hmi {
    pub fn new(home: Option<DeviceId>, stale_after_ms: Option<u64>) -> Self {
        Self {
            home,
            stale_after_ms,
            devices: Vec::new(),
            next_alias: 2,
            page: Page::Home,
        }
    }

    /// Replace a device's entire snapshot only after validating it.
    pub fn update(
        &mut self,
        sample: &impl Sample,
        capabilities: &'static [Kind],
        now_ms: u64,
    ) -> Result<Update, UpdateError> {
        let id = sample.device();
        let position = self.devices.iter().position(|device| device.id == id);
        if let Some(position) = position {
            if self.devices[position].capabilities != capabilities {
                return Err(UpdateError::CapabilitiesChanged);
            }
        } else {
            let reserve_home = self.home.is_some()
                && self.home != Some(id)
                && !self
                    .devices
                    .iter()
                    .any(|device| Some(device.id) == self.home);
            if self.devices.is_full() || (reserve_home && self.devices.len() == DEVICE_CAPACITY - 1)
            {
                return Err(UpdateError::CapacityExceeded);
            }
        }

        let mut values = [None; KIND_COUNT];
        let mut error = None;
        sample.for_each_measurement(&mut |measurement| {
            let kind = measurement.kind();
            if !capabilities.contains(&kind) {
                error = Some(UpdateError::UnsupportedMeasurement(kind));
            } else if values[index(kind)].is_some() {
                error = Some(UpdateError::DuplicateMeasurement(kind));
            } else {
                values[index(kind)] = Some(measurement);
            }
        });
        if let Some(error) = error {
            return Err(error);
        }

        if let Some(position) = position {
            let device = &mut self.devices[position];
            device.values = values;
            device.last_seen_ms = now_ms;
            Ok(Update::Existing)
        } else {
            let alias = if self.home == Some(id) {
                *b"01"
            } else {
                let alias = alias(self.next_alias);
                self.next_alias += 1;
                alias
            };
            self.devices
                .push(Device {
                    id,
                    alias,
                    capabilities,
                    values,
                    last_seen_ms: now_ms,
                })
                .map_err(|_| UpdateError::CapacityExceeded)?;
            Ok(Update::Discovered(alias))
        }
    }

    pub fn home(&mut self) {
        self.page = Page::Home;
    }

    pub fn next(&mut self) {
        let first_group = match self.page {
            Page::Home => 0,
            Page::Group { group, page } => {
                if (page + 1) * ROWS_PER_PAGE < self.row_count(group) {
                    self.page = Page::Group {
                        group,
                        page: page + 1,
                    };
                    return;
                }
                group + 1
            }
        };
        for group in first_group..GROUPS.len() {
            if self.row_count(group) > 0 {
                self.page = Page::Group { group, page: 0 };
                return;
            }
        }
        self.home();
    }

    fn row_count(&self, group: usize) -> usize {
        self.devices
            .iter()
            .filter(|device| group != 0 || Some(device.id) != self.home)
            .map(|device| {
                GROUPS[group]
                    .iter()
                    .filter(|kind| device.capabilities.contains(kind))
                    .count()
            })
            .sum()
    }

    fn freshness(&self, device: &Device, now_ms: u64) -> char {
        match self.stale_after_ms {
            Some(timeout) if now_ms.saturating_sub(device.last_seen_ms) >= timeout => '!',
            Some(_) => ' ',
            None => '?',
        }
    }

    pub fn screen(&self, now_ms: u64) -> Screen {
        let mut screen = Screen::new();
        match self.page {
            Page::Home => {
                let _ = screen.lines[0].push_str("HOME");
                if let Some(home) = self.home {
                    let device = self.devices.iter().find(|device| device.id == home);
                    let status = device.map_or('?', |device| self.freshness(device, now_ms));
                    write!(screen.lines[0], " 01{status}").expect("home header fits");
                    let temperature = device
                        .filter(|_| status != '!')
                        .and_then(|device| device.values[index(Kind::Temperature)]);
                    let humidity = device
                        .filter(|_| status != '!')
                        .and_then(|device| device.values[index(Kind::Humidity)]);
                    screen.home_temperature =
                        Some(value_text(Kind::Temperature, temperature, false));
                    screen.lines[4] = value_text(Kind::Humidity, humidity, true);
                } else {
                    screen.lines[2].push_str("SET HOME").expect("message fits");
                }
            }
            Page::Group { group, page } => {
                screen.lines[0].push_str(TITLES[group]).expect("title fits");
                let mut row = 0;
                for device in &self.devices {
                    if group == 0 && Some(device.id) == self.home {
                        continue;
                    }
                    for &kind in GROUPS[group] {
                        if !device.capabilities.contains(&kind) {
                            continue;
                        }
                        if row >= page * ROWS_PER_PAGE && row < (page + 1) * ROWS_PER_PAGE {
                            let status = self.freshness(device, now_ms);
                            let value = if status == '!' {
                                None
                            } else {
                                device.values[index(kind)]
                            };
                            let line = &mut screen.lines[row - page * ROWS_PER_PAGE + 1];
                            line.push(char::from(device.alias[0])).expect("alias fits");
                            line.push(char::from(device.alias[1])).expect("alias fits");
                            line.push(status).expect("status fits");
                            line.push_str(&value_text(kind, value, true))
                                .expect("formatted measurement fits");
                        }
                        row += 1;
                    }
                }
            }
        }
        screen
    }
}

fn alias(number: u16) -> [u8; 2] {
    const DIGITS: &[u8; 36] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    [
        DIGITS[usize::from(number / 36)],
        DIGITS[usize::from(number % 36)],
    ]
}

fn value_text(kind: Kind, measurement: Option<Measurement>, label: bool) -> String<10> {
    let (prefix, unit, decimals) = match kind {
        Kind::Temperature => ("T", "C", 1),
        Kind::Humidity => ("H", "%", 0),
        Kind::WindAverage => ("A", "m/s", 1),
        Kind::WindGust => ("G", "m/s", 1),
        Kind::WindDirection => ("D", "deg", 0),
        Kind::Light => ("L", "lx", 0),
        Kind::UvIndex => ("U", "", 1),
        Kind::RainTotal => ("R", "mm", 1),
        Kind::BatteryVoltage => ("B", "V", 2),
        Kind::SupercapVoltage => ("S", "V", 2),
        Kind::RainActive => ("r", "", 0),
    };
    let prefix = if label { prefix } else { "" };
    let max_len = if label { 7 } else { 10 };
    let mut result = String::<10>::new();
    match measurement.map(|item| item.value()) {
        None => {
            write!(result, "{prefix}--{unit}").expect("placeholder fits");
        }
        Some(MeasurementValue::Binary(value)) => {
            write!(result, "{prefix}{}", if value { "ON" } else { "OFF" })
                .expect("binary value fits");
        }
        Some(MeasurementValue::Numeric(value)) => {
            let (value, unit, decimals) = if kind == Kind::Light && value.abs() >= 1000.0 {
                (value / 1000.0, "klx", 1)
            } else {
                (value, unit, decimals)
            };
            for precision in (0..=decimals).rev() {
                let mut candidate = String::<64>::new();
                if write!(candidate, "{prefix}{value:.precision$}{unit}").is_ok()
                    && candidate.len() <= max_len
                {
                    result.push_str(&candidate).expect("bounded value fits");
                    return result;
                }
            }
            write!(result, "{prefix}{}OVF", if value < 0.0 { "-" } else { "+" })
                .expect("overflow marker fits");
        }
    }
    result
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ButtonEvent {
    Short,
    Long,
}

pub struct Button {
    raw: bool,
    stable: bool,
    changed_at: u64,
    pressed_at: u64,
    long_fired: bool,
}

impl Button {
    pub const fn new() -> Self {
        Self {
            raw: false,
            stable: false,
            changed_at: 0,
            pressed_at: 0,
            long_fired: false,
        }
    }

    pub fn update(&mut self, pressed: bool, now_ms: u64) -> Option<ButtonEvent> {
        if pressed != self.raw {
            self.raw = pressed;
            self.changed_at = now_ms;
        }
        if self.raw != self.stable && now_ms.saturating_sub(self.changed_at) >= DEBOUNCE_MS {
            self.stable = self.raw;
            if self.stable {
                self.pressed_at = self.changed_at;
                self.long_fired = false;
            } else if !self.long_fired {
                return Some(
                    if self.changed_at.saturating_sub(self.pressed_at) >= LONG_PRESS_MS {
                        ButtonEvent::Long
                    } else {
                        ButtonEvent::Short
                    },
                );
            }
        }
        if self.stable
            && self.raw
            && !self.long_fired
            && now_ms.saturating_sub(self.pressed_at) >= LONG_PRESS_MS
        {
            self.long_fired = true;
            return Some(ButtonEvent::Long);
        }
        None
    }
}

impl Default for Button {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rf_sensor::{SensorModelKey, model::Attribute};
    use std::vec;

    static ALL: [Kind; KIND_COUNT] = [
        Kind::Temperature,
        Kind::Humidity,
        Kind::WindAverage,
        Kind::WindGust,
        Kind::WindDirection,
        Kind::Light,
        Kind::UvIndex,
        Kind::RainTotal,
        Kind::BatteryVoltage,
        Kind::SupercapVoltage,
        Kind::RainActive,
    ];
    static TEMPERATURE: [Kind; 1] = [Kind::Temperature];

    fn id(device: u32) -> DeviceId {
        DeviceId {
            model: SensorModelKey {
                vendor: "Example",
                model_id: "weather",
            },
            device,
        }
    }

    struct TestSample {
        id: DeviceId,
        values: std::vec::Vec<Measurement>,
    }

    impl Sample for TestSample {
        fn device(&self) -> DeviceId {
            self.id
        }

        fn for_each_measurement(&self, f: &mut impl FnMut(Measurement)) {
            for value in &self.values {
                f(*value);
            }
        }

        fn for_each_attribute<'a>(&'a self, _: &mut impl FnMut(Attribute<'a>)) {}
    }

    fn number(kind: Kind, value: f32) -> Measurement {
        Measurement::numeric(kind, value).unwrap()
    }

    fn sample(device: u32) -> TestSample {
        TestSample {
            id: id(device),
            values: vec![
                number(Kind::Temperature, 21.4),
                number(Kind::Humidity, 63.0),
                number(Kind::WindAverage, 3.2),
                number(Kind::WindGust, 6.1),
                number(Kind::WindDirection, 270.0),
                number(Kind::RainTotal, 12.3),
                Measurement::binary(Kind::RainActive, true).unwrap(),
                number(Kind::Light, 12000.0),
                number(Kind::UvIndex, 5.2),
                number(Kind::BatteryVoltage, 1.32),
                number(Kind::SupercapVoltage, 4.85),
            ],
        }
    }

    #[test]
    fn home_requires_the_configured_sensor_and_never_uses_another() {
        let mut hmi = Hmi::new(None, None);
        hmi.update(&sample(2), &ALL, 0).unwrap();
        assert_eq!(hmi.screen(0).lines[2], "SET HOME");
        let mut hmi = Hmi::new(Some(id(1)), None);
        hmi.update(&sample(2), &ALL, 0).unwrap();
        assert_eq!(hmi.screen(0).home_temperature.as_deref(), Some("--C"));
        assert_eq!(
            hmi.update(&sample(1), &ALL, 0),
            Ok(Update::Discovered(*b"01"))
        );
        let screen = hmi.screen(0);
        assert_eq!(screen.lines[0], "HOME 01?");
        assert_eq!(screen.home_temperature.as_deref(), Some("21.4C"));
        assert_eq!(screen.lines[4], "H63%");
    }

    #[test]
    fn groups_and_packs_measurements_across_sensors() {
        let mut hmi = Hmi::new(Some(id(1)), Some(1000));
        assert_eq!(
            hmi.update(&sample(1), &ALL, 0),
            Ok(Update::Discovered(*b"01"))
        );
        assert_eq!(
            hmi.update(&sample(2), &ALL, 0),
            Ok(Update::Discovered(*b"02"))
        );
        hmi.next();
        let climate = hmi.screen(0);
        assert_eq!(climate.lines[0], "CLIMATE");
        assert_eq!(climate.lines[1], "02 T21.4C");
        assert_eq!(climate.lines[2], "02 H63%");
        hmi.next();
        let wind = hmi.screen(0);
        assert_eq!(wind.lines[0], "WIND");
        assert_eq!(wind.lines[1], "01 A3.2m/s");
        assert_eq!(wind.lines[5], "02 G6.1m/s");
        hmi.next();
        let wind_next = hmi.screen(0);
        assert_eq!(wind_next.lines[1], "02 D270deg");
        hmi.next();
        let rain = hmi.screen(0);
        assert_eq!(rain.lines[1], "01 R12.3mm");
        assert_eq!(rain.lines[2], "01 rON");
        assert_eq!(rain.lines[3], "01 L12klx");
        assert_eq!(rain.lines[4], "01 U5.2");
        hmi.next();
        assert_eq!(hmi.screen(0).lines[0], "RAIN/SUN");
        hmi.next();
        let power = hmi.screen(0);
        assert_eq!(power.lines[1], "01 B1.32V");
        assert_eq!(power.lines[2], "01 S4.85V");
        hmi.next();
        assert_eq!(hmi.screen(0).lines[0], "HOME 01 ");
    }

    #[test]
    fn complete_snapshots_clear_missing_values_and_expire_on_time() {
        let mut hmi = Hmi::new(Some(id(1)), Some(1000));
        let mut sensor = sample(1);
        hmi.update(&sensor, &ALL, 100).unwrap();
        sensor
            .values
            .retain(|value| value.kind() != Kind::Temperature);
        hmi.update(&sensor, &ALL, 200).unwrap();
        assert_eq!(hmi.screen(1199).home_temperature.as_deref(), Some("--C"));
        assert_eq!(hmi.screen(1199).lines[4], "H63%");
        let expired = hmi.screen(1200);
        assert_eq!(expired.lines[0], "HOME 01!");
        assert_eq!(expired.lines[4], "H--%");
        hmi.next();
        assert_eq!(hmi.screen(1200).lines[1], "01!A--m/s");
    }

    #[test]
    fn invalid_snapshots_do_not_replace_last_valid_values() {
        let mut hmi = Hmi::new(Some(id(1)), None);
        let mut sensor = sample(1);
        hmi.update(&sensor, &ALL, 0).unwrap();
        sensor.values.push(number(Kind::Temperature, 0.0));
        assert_eq!(
            hmi.update(&sensor, &ALL, 1),
            Err(UpdateError::DuplicateMeasurement(Kind::Temperature))
        );
        assert_eq!(hmi.screen(1).home_temperature.as_deref(), Some("21.4C"));
        let other = TestSample {
            id: id(2),
            values: vec![number(Kind::Humidity, 30.0)],
        };
        assert_eq!(
            hmi.update(&other, &TEMPERATURE, 1),
            Err(UpdateError::UnsupportedMeasurement(Kind::Humidity))
        );
        assert_eq!(
            hmi.update(&sample(1), &TEMPERATURE, 1),
            Err(UpdateError::CapabilitiesChanged)
        );
    }

    #[test]
    fn capacity_is_explicit_and_aliases_do_not_change() {
        let mut hmi = Hmi::new(None, None);
        for n in 0..DEVICE_CAPACITY {
            let sensor = TestSample {
                id: id(n as u32),
                values: vec![],
            };
            assert_eq!(
                hmi.update(&sensor, &TEMPERATURE, 0),
                Ok(Update::Discovered(alias(n as u16 + 2)))
            );
        }
        let overflow = TestSample {
            id: id(100),
            values: vec![],
        };
        assert_eq!(
            hmi.update(&overflow, &TEMPERATURE, 0),
            Err(UpdateError::CapacityExceeded)
        );
        hmi.next();
        assert_eq!(hmi.screen(0).lines[1], "02?T--C");
        assert_eq!(
            hmi.update(&sample(0), &ALL, 1),
            Err(UpdateError::CapabilitiesChanged)
        );
    }

    #[test]
    fn reserves_capacity_for_the_configured_home_sensor() {
        let mut hmi = Hmi::new(Some(id(100)), None);
        for n in 0..DEVICE_CAPACITY - 1 {
            let sensor = TestSample {
                id: id(n as u32),
                values: vec![],
            };
            hmi.update(&sensor, &TEMPERATURE, 0).unwrap();
        }
        let other = TestSample {
            id: id(99),
            values: vec![],
        };
        assert_eq!(
            hmi.update(&other, &TEMPERATURE, 0),
            Err(UpdateError::CapacityExceeded)
        );
        let home = TestSample {
            id: id(100),
            values: vec![],
        };
        assert_eq!(
            hmi.update(&home, &TEMPERATURE, 0),
            Ok(Update::Discovered(*b"01"))
        );
    }

    #[test]
    fn formatting_is_bounded_and_reports_overflow() {
        assert_eq!(
            value_text(
                Kind::WindAverage,
                Some(number(Kind::WindAverage, 123.4)),
                true
            ),
            "A123m/s"
        );
        assert_eq!(
            value_text(
                Kind::WindAverage,
                Some(number(Kind::WindAverage, f32::MAX)),
                true
            ),
            "A+OVF"
        );
        assert_eq!(
            value_text(
                Kind::Temperature,
                Some(number(Kind::Temperature, -f32::MAX)),
                true
            ),
            "T-OVF"
        );
        assert_eq!(
            value_text(Kind::Light, Some(number(Kind::Light, 12000.0)), true),
            "L12klx"
        );
    }

    #[test]
    fn button_debounces_and_does_not_advance_after_long_press() {
        let mut button = Button::new();
        assert_eq!(button.update(true, 10), None);
        assert_eq!(button.update(false, 20), None);
        assert_eq!(button.update(true, 30), None);
        assert_eq!(button.update(true, 60), None);
        assert_eq!(button.update(false, 120), None);
        assert_eq!(button.update(false, 150), Some(ButtonEvent::Short));
        assert_eq!(button.update(true, 200), None);
        assert_eq!(button.update(true, 230), None);
        assert_eq!(button.update(true, 999), None);
        assert_eq!(button.update(true, 1000), Some(ButtonEvent::Long));
        assert_eq!(button.update(false, 1010), None);
        assert_eq!(button.update(false, 1040), None);
    }

    #[test]
    fn release_just_before_long_threshold_remains_short() {
        let mut button = Button::new();
        button.update(true, 0);
        button.update(true, 30);
        button.update(false, 799);
        assert_eq!(button.update(false, 829), Some(ButtonEvent::Short));
    }
}
