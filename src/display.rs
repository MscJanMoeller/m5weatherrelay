use display_interface::AsyncWriteOnlyDataCommand;
use ssd1306::{
    Ssd1306Async,
    mode::{TerminalDisplaySizeAsync, TerminalModeAsync, TerminalModeError},
    prelude::DisplayRotation,
};

pub const DISPLAY_ROTATION: DisplayRotation = DisplayRotation::Rotate180;

pub async fn write_text<DI, SIZE>(
    display: &mut Ssd1306Async<DI, SIZE, TerminalModeAsync>,
    text: &str,
) -> Result<(), TerminalModeError>
where
    DI: AsyncWriteOnlyDataCommand,
    SIZE: TerminalDisplaySizeAsync,
{
    for character in text.chars() {
        display.print_char(character).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::RefCell;
    use display_interface::{DataFormat, DisplayError};
    use embassy_futures::block_on;
    use ssd1306::{mode::DisplayConfigAsync, size::DisplaySize64x48};
    use std::{rc::Rc, vec::Vec};

    #[derive(Default)]
    struct State {
        commands: Vec<Vec<u8>>,
        data: Vec<Vec<u8>>,
        fail_data: bool,
    }

    struct MockInterface(Rc<RefCell<State>>);

    impl AsyncWriteOnlyDataCommand for MockInterface {
        async fn send_commands(&mut self, data: DataFormat<'_>) -> Result<(), DisplayError> {
            match data {
                DataFormat::U8(bytes) => self.0.borrow_mut().commands.push(bytes.into()),
                _ => panic!("unexpected command format"),
            }
            Ok(())
        }

        async fn send_data(&mut self, data: DataFormat<'_>) -> Result<(), DisplayError> {
            let mut state = self.0.borrow_mut();
            if state.fail_data {
                return Err(DisplayError::BusWriteError);
            }
            match data {
                DataFormat::U8(bytes) => state.data.push(bytes.into()),
                _ => panic!("unexpected data format"),
            }
            Ok(())
        }
    }

    #[test]
    fn writes_every_character_and_handles_newlines() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut display = Ssd1306Async::new(
            MockInterface(state.clone()),
            DisplaySize64x48,
            DISPLAY_ROTATION,
        )
        .into_terminal_mode();
        block_on(display.init()).unwrap();

        block_on(write_text(&mut display, "A\nBC")).unwrap();

        let state = state.borrow();
        assert!(state.commands.iter().any(|command| command == &[0xa0]));
        assert!(state.commands.iter().any(|command| command == &[0xc0]));
        assert_eq!(state.data.len(), 3);
        assert_eq!(state.data[0], [0, 0x3e, 0x09, 0x09, 0x09, 0x09, 0x3e, 0]);
        assert_eq!(state.data[1], [0, 0x3f, 0x25, 0x25, 0x25, 0x25, 0x1a, 0]);
        assert_eq!(state.data[2], [0, 0x1e, 0x21, 0x21, 0x21, 0x21, 0x12, 0]);
        assert_eq!(display.position().unwrap(), (2, 1));
    }

    #[test]
    fn propagates_display_write_errors() {
        let state = Rc::new(RefCell::new(State::default()));
        let mut display = Ssd1306Async::new(
            MockInterface(state.clone()),
            DisplaySize64x48,
            DISPLAY_ROTATION,
        )
        .into_terminal_mode();
        block_on(display.init()).unwrap();
        state.borrow_mut().fail_data = true;

        assert!(matches!(
            block_on(write_text(&mut display, "A")),
            Err(TerminalModeError::InterfaceError(
                DisplayError::BusWriteError
            ))
        ));
    }
}
