use embedded_hal::i2c::I2c;
use rf_receiver::sx1262::{BoardConfig, RegulatorMode, TcxoVoltage};

const IO_EXPANDER_ADDRESS: u8 = 0x43;
const IO_EXPANDER_ID: u8 = 0x01;

/// The Unit C6L's radio is clocked by a 3 V TCXO and has no DC-DC inductor.
pub const RADIO_BOARD: BoardConfig = BoardConfig {
    tcxo: Some(TcxoVoltage::V3_0),
    regulator: RegulatorMode::LdoOnly,
};

// M5Unified's Unit C6L defaults: radio out of reset, RF switch and LNA off.
const IO_EXPANDER_CONFIG: [(u8, u8); 7] = [
    (0x05, 0b1000_0000),
    (0x07, 0b0001_1100),
    (0x03, 0b1110_0000),
    (0x0d, 0b1100_0011),
    (0x0b, 0b1100_0011),
    (0x09, 0b0000_0011),
    (0x11, 0b1111_1100),
];

#[derive(Debug, PartialEq, Eq)]
pub enum InitError<E> {
    I2c(E),
    InvalidExpanderId,
}

pub fn init_io_expander<I: I2c>(i2c: &mut I) -> Result<(), InitError<I::Error>> {
    let mut id = [0];
    i2c.write_read(IO_EXPANDER_ADDRESS, &[IO_EXPANDER_ID], &mut id)
        .map_err(InitError::I2c)?;
    if id[0] == 0 {
        return Err(InitError::InvalidExpanderId);
    }

    for (register, value) in IO_EXPANDER_CONFIG {
        i2c.write(IO_EXPANDER_ADDRESS, &[register, value])
            .map_err(InitError::I2c)?;
    }
    Ok(())
}

pub fn set_radio_front_end<I: I2c>(
    i2c: &mut I,
    reset_released: bool,
    receive_enabled: bool,
) -> Result<(), I::Error> {
    let mut output = [0];
    i2c.write_read(IO_EXPANDER_ADDRESS, &[0x05], &mut output)?;
    let radio_bits = (u8::from(reset_released) << 7) | (u8::from(receive_enabled) * 0x60);
    i2c.write(
        IO_EXPANDER_ADDRESS,
        &[0x05, (output[0] & !0xe0) | radio_bits],
    )
}

pub fn button_pressed<I: I2c>(i2c: &mut I) -> Result<bool, I::Error> {
    let mut input = [0];
    i2c.write_read(IO_EXPANDER_ADDRESS, &[0x0f], &mut input)?;
    Ok(input[0] & 1 == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use embedded_hal::i2c::{ErrorKind, ErrorType, Operation};
    use std::vec::Vec;

    #[derive(Default)]
    struct MockI2c {
        id: u8,
        fail_read: bool,
        fail_write_at: Option<usize>,
        output: u8,
        input: u8,
        writes: Vec<(u8, u8)>,
    }

    impl ErrorType for MockI2c {
        type Error = ErrorKind;
    }

    impl I2c for MockI2c {
        fn transaction(
            &mut self,
            address: u8,
            operations: &mut [Operation<'_>],
        ) -> Result<(), Self::Error> {
            assert_eq!(address, IO_EXPANDER_ADDRESS);
            match operations {
                [Operation::Write(register), Operation::Read(data)] => {
                    if self.fail_read {
                        return Err(ErrorKind::Other);
                    }
                    data[0] = match *register {
                        [IO_EXPANDER_ID] => self.id,
                        [0x05] => self.output,
                        [0x0f] => self.input,
                        _ => panic!("unexpected register"),
                    };
                }
                [Operation::Write(bytes)] => {
                    if self.fail_write_at == Some(self.writes.len()) {
                        return Err(ErrorKind::Other);
                    }
                    self.writes.push((bytes[0], bytes[1]));
                    if bytes[0] == 0x05 {
                        self.output = bytes[1];
                    }
                }
                _ => panic!("unexpected I2C transaction"),
            }
            Ok(())
        }
    }

    #[test]
    fn configures_expander_with_radio_front_end_off() {
        let mut i2c = MockI2c {
            id: 1,
            ..Default::default()
        };
        assert_eq!(init_io_expander(&mut i2c), Ok(()));
        assert_eq!(
            i2c.writes,
            [
                (0x05, 0x80),
                (0x07, 0x1c),
                (0x03, 0xe0),
                (0x0d, 0xc3),
                (0x0b, 0xc3),
                (0x09, 0x03),
                (0x11, 0xfc),
            ]
        );
    }

    #[test]
    fn rejects_zero_id_without_writing() {
        let mut i2c = MockI2c::default();
        assert_eq!(
            init_io_expander(&mut i2c),
            Err(InitError::InvalidExpanderId)
        );
        assert!(i2c.writes.is_empty());
    }

    #[test]
    fn propagates_read_error_without_writing() {
        let mut i2c = MockI2c {
            fail_read: true,
            ..Default::default()
        };
        assert_eq!(
            init_io_expander(&mut i2c),
            Err(InitError::I2c(ErrorKind::Other))
        );
        assert!(i2c.writes.is_empty());
    }

    #[test]
    fn stops_on_write_error() {
        for index in 0..IO_EXPANDER_CONFIG.len() {
            let mut i2c = MockI2c {
                id: 1,
                fail_write_at: Some(index),
                ..Default::default()
            };
            assert_eq!(
                init_io_expander(&mut i2c),
                Err(InitError::I2c(ErrorKind::Other))
            );
            assert_eq!(i2c.writes, IO_EXPANDER_CONFIG[..index]);
        }
    }

    #[test]
    fn sets_radio_reset_and_receive_controls_without_changing_other_pins() {
        let mut i2c = MockI2c {
            output: 0x1b,
            ..Default::default()
        };
        set_radio_front_end(&mut i2c, false, false).unwrap();
        set_radio_front_end(&mut i2c, true, true).unwrap();
        assert_eq!(i2c.writes, [(0x05, 0x1b), (0x05, 0xfb)]);
    }

    #[test]
    fn propagates_front_end_read_and_write_errors() {
        let mut i2c = MockI2c {
            fail_read: true,
            ..Default::default()
        };
        assert_eq!(
            set_radio_front_end(&mut i2c, true, true),
            Err(ErrorKind::Other)
        );
        assert!(i2c.writes.is_empty());
        i2c.fail_read = false;
        i2c.fail_write_at = Some(0);
        assert_eq!(
            set_radio_front_end(&mut i2c, true, true),
            Err(ErrorKind::Other)
        );
    }

    #[test]
    fn reads_the_active_low_button_and_propagates_errors() {
        let mut i2c = MockI2c {
            input: 1,
            ..Default::default()
        };
        assert_eq!(button_pressed(&mut i2c), Ok(false));
        i2c.input = 0xfe;
        assert_eq!(button_pressed(&mut i2c), Ok(true));
        i2c.fail_read = true;
        assert_eq!(button_pressed(&mut i2c), Err(ErrorKind::Other));
    }

    #[test]
    fn radio_board_has_the_unit_c6l_tcxo_and_regulator() {
        assert!(matches!(
            RADIO_BOARD,
            BoardConfig {
                tcxo: Some(TcxoVoltage::V3_0),
                regulator: RegulatorMode::LdoOnly,
            }
        ));
    }
}
