use crate::sensor_registry::{SensorProfile, SensorRegistry};
use rf_sensor::ProfileSelectionError;

#[cfg(any(
    all(feature = "ws90-433", feature = "ws90-915"),
    all(feature = "ws90-433", feature = "ws90-920"),
    all(feature = "ws90-915", feature = "ws90-920")
))]
compile_error!("select at most one WS90 regional profile");

#[cfg(feature = "ws90-433")]
pub const PROFILE_ID: &str = rf_ecowitt::profiles::WS90_433_ID;
#[cfg(all(not(feature = "ws90-433"), feature = "ws90-915"))]
pub const PROFILE_ID: &str = rf_ecowitt::profiles::WS90_915_ID;
#[cfg(all(
    not(feature = "ws90-433"),
    not(feature = "ws90-915"),
    feature = "ws90-920"
))]
pub const PROFILE_ID: &str = rf_ecowitt::profiles::WS90_920_ID;
#[cfg(not(any(feature = "ws90-433", feature = "ws90-915", feature = "ws90-920")))]
pub const PROFILE_ID: &str = rf_ecowitt::profiles::WS90_868_ID;

pub fn selected_profile(
    registry: &SensorRegistry,
) -> Result<&SensorProfile, ProfileSelectionError> {
    registry.select_profile(PROFILE_ID)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_profile_uses_the_requested_frequency() {
        #[cfg(feature = "ws90-433")]
        let expected = 433_925_000;
        #[cfg(all(not(feature = "ws90-433"), feature = "ws90-915"))]
        let expected = 915_000_000;
        #[cfg(all(
            not(feature = "ws90-433"),
            not(feature = "ws90-915"),
            feature = "ws90-920"
        ))]
        let expected = 920_900_000;
        #[cfg(not(any(feature = "ws90-433", feature = "ws90-915", feature = "ws90-920")))]
        let expected = 868_350_000;
        let registry = crate::sensor_registry::registry().unwrap();
        let selected = selected_profile(&registry).unwrap();
        assert_eq!(selected.id(), PROFILE_ID);
        assert_eq!(selected.fsk().frequency_hz, expected);
        assert_eq!(
            usize::from(selected.fsk().packet_length),
            rf_ecowitt::ws90::RADIO_LEN
        );
        let models: std::vec::Vec<_> = selected.models().collect();
        assert_eq!(models.len(), 1);
        assert_eq!(
            (models[0].vendor, models[0].model_id),
            (rf_ecowitt::VENDOR, rf_ecowitt::WS90_MODEL_ID)
        );
    }
}
