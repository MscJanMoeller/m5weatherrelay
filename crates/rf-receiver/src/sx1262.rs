//! SX1262-specific control and diagnostics.

use crate::{FskProfile, PacketInfo};
use embassy_futures::select::{Either, select};
use embedded_hal::digital::InputPin;
use embedded_hal_async::{delay::DelayNs, digital::Wait, spi::SpiDevice};
use regiface::Command;
use sx1262::{
    Device,
    commands::{
        AddressFiltering, BufferBaseAddressConfig, Calibrate, CalibrateImage, CalibrationConfig,
        ClearDeviceErrors, ClearIrqStatus, CommandStatus, CrcType, DeviceErrors, DioIrqConfig,
        GFSKPacketHeaderType, GFSKPacketParams, GetDeviceErrors, GetIrqStatus, GetPacketStatus,
        GetRawStatus, GetRxBufferStatus, GfskBandwidth, GfskModParams, GfskPulseShape,
        ImageCalibConfig, IrqMask, ModulationParams, PacketParams, PacketType,
        PreambleDetectorLength, RegulatorMode, RfFrequencyConfig, RxMode, SetBufferBaseAddress,
        SetDio3AsTcxoCtrl, SetDioIrqParams, SetModulationParams, SetPacketParams, SetPacketType,
        SetRegulatorMode, SetRfFrequency, SetRx, SetStandby, StandbyConfig, TcxoConfig,
        TcxoVoltage,
    },
    device::{OPCODE_READ_BUFFER, OPCODE_WRITE_REGISTER},
    registers::SyncWord,
};

pub const IRQ_RX_DONE: u16 = IrqMask::RX_DONE.bits();
pub const IRQ_PREAMBLE: u16 = IrqMask::PREAMBLE_DETECTED.bits();
pub const IRQ_SYNC: u16 = IrqMask::SYNC_WORD_VALID.bits();
pub const IRQ_HEADER_ERROR: u16 = IrqMask::HEADER_ERROR.bits();
pub const IRQ_CRC_ERROR: u16 = IrqMask::CRC_ERROR.bits();
pub const IRQ_TIMEOUT: u16 = IrqMask::TIMEOUT.bits();
const IRQ_ERRORS: u16 = IRQ_HEADER_ERROR | IRQ_CRC_ERROR | IRQ_TIMEOUT;
const IRQ_MASK: u16 = IRQ_RX_DONE | IRQ_PREAMBLE | IRQ_SYNC | IRQ_ERRORS;
const IRQ_DIO1_MASK: u16 = IRQ_RX_DONE | IRQ_ERRORS;
const BUSY_TIMEOUT_US: u32 = 250_000;
const TCXO_STARTUP_DELAY_STEPS: u32 = 320;

#[derive(Debug, PartialEq, Eq)]
pub enum Error<S, B, I> {
    Spi(S),
    DriverSerializationError,
    DriverDeserializationError { opcode: u8, first_byte: Option<u8> },
    BusyPin(B),
    IrqPin(I),
    BusyTimeout,
    InvalidStatus(u8),
    CommandFailed { opcode: u8, status: u8 },
    DeviceErrors(u16),
    InvalidPacketLength(u8),
    InvalidProfile,
    NotConfigured,
    NotReceiving,
    BufferTooSmall { required: usize, available: usize },
    ReceiveIrq(u16),
    UnexpectedIrq(u16),
}

/// Outcome after handling the IRQ flags returned by [`Sx1262::get_irq_flags`].
pub enum ReceiveEvent {
    Pending,
    Packet(PacketInfo),
    RadioError(u16),
    InvalidPacketLength(u8),
}

#[derive(Debug, PartialEq, Eq)]
pub struct InitStatus {
    pub cleared_xosc_start_error: bool,
}

pub struct Sx1262<SPI, BUSY, IRQ, DELAY> {
    spi: SPI,
    busy: BUSY,
    irq: IRQ,
    delay: DELAY,
    packet_length: Option<u8>,
    receiving: bool,
}

impl<SPI, BUSY, IRQ, DELAY> Sx1262<SPI, BUSY, IRQ, DELAY>
where
    SPI: SpiDevice,
    BUSY: Wait,
    IRQ: Wait + InputPin,
    DELAY: DelayNs,
{
    pub fn new(spi: SPI, busy: BUSY, irq: IRQ, delay: DELAY) -> Self {
        Self {
            spi,
            busy,
            irq,
            delay,
            packet_length: None,
            receiving: false,
        }
    }

    async fn wait_ready(&mut self) -> Result<(), Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        match select(
            self.busy.wait_for_low(),
            self.delay.delay_us(BUSY_TIMEOUT_US),
        )
        .await
        {
            Either::First(result) => result.map_err(Error::BusyPin),
            Either::Second(()) => Err(Error::BusyTimeout),
        }
    }

    async fn check_status(
        &mut self,
        opcode: u8,
    ) -> Result<(), Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        self.delay.delay_us(1).await;
        self.wait_ready().await?;
        let status = Device::new(&mut self.spi)
            .execute_command_async(GetRawStatus)
            .await
            .map_err(Self::driver_error)?;
        self.delay.delay_us(1).await;
        if status == 0 || status == 0xff {
            return Err(Error::InvalidStatus(status));
        }
        if matches!(
            CommandStatus::from_status_byte(status),
            Ok(CommandStatus::Timeout
                | CommandStatus::ProcessingError
                | CommandStatus::ExecutionFailure)
        ) {
            return Err(Error::CommandFailed { opcode, status });
        }
        Ok(())
    }

    async fn driver_command<C>(
        &mut self,
        command: C,
    ) -> Result<C::ResponseParameters, Error<SPI::Error, BUSY::Error, IRQ::Error>>
    where
        C: Command<IdType = u8>,
    {
        self.wait_ready().await?;
        let response = Device::new(&mut self.spi)
            .execute_command_async(command)
            .await
            .map_err(Self::driver_error)?;
        self.check_status(C::id()).await?;
        Ok(response)
    }

    fn driver_error(
        error: sx1262::Error<SPI::Error>,
    ) -> Error<SPI::Error, BUSY::Error, IRQ::Error> {
        match error {
            sx1262::Error::Spi(error) => Error::Spi(error),
            sx1262::Error::Serialization => Error::DriverSerializationError,
            sx1262::Error::Deserialization { opcode, first_byte } => {
                Error::DriverDeserializationError { opcode, first_byte }
            }
        }
    }

    pub async fn begin_fsk(
        &mut self,
        profile: &FskProfile<'_>,
    ) -> Result<InitStatus, Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        let bandwidth =
            GfskBandwidth::from_hz(profile.rx_bandwidth_hz).ok_or(Error::InvalidProfile)?;
        // The image calibration below only covers this band.
        if !(863_000_000..=870_000_000).contains(&profile.frequency_hz)
            || !GfskModParams::BIT_RATE_BPS.contains(&profile.bit_rate)
            || !GfskModParams::FREQ_DEVIATION_HZ.contains(&profile.frequency_deviation_hz)
            || profile.preamble_bits < 16
            || profile.sync_word.len() != 2
            || profile.packet_length == 0
            || profile.crc_enabled
            || profile.whitening_enabled
        {
            return Err(Error::InvalidProfile);
        }
        self.packet_length = None;
        self.receiving = false;
        self.driver_command(SetStandby {
            config: StandbyConfig::Rc,
        })
        .await?;
        let startup_errors = self.driver_command(GetDeviceErrors).await?.errors.raw;
        if startup_errors & !DeviceErrors::XOSC_START_ERROR != 0 {
            return Err(Error::DeviceErrors(startup_errors));
        }
        let cleared_xosc_start_error = startup_errors & DeviceErrors::XOSC_START_ERROR != 0;
        if cleared_xosc_start_error {
            self.driver_command(ClearDeviceErrors).await?;
        }
        self.driver_command(SetDio3AsTcxoCtrl {
            config: TcxoConfig {
                voltage: TcxoVoltage::V3_0,
                delay: TCXO_STARTUP_DELAY_STEPS,
            },
        })
        .await?;
        self.driver_command(SetRegulatorMode {
            mode: RegulatorMode::LdoOnly,
        })
        .await?;
        self.driver_command(Calibrate {
            config: CalibrationConfig::all(),
        })
        .await?;
        self.driver_command(CalibrateImage {
            config: ImageCalibConfig::BAND_863_870_MHZ,
        })
        .await?;
        let errors = self.driver_command(GetDeviceErrors).await?.errors.raw;
        if errors != 0 {
            return Err(Error::DeviceErrors(errors));
        }
        self.driver_command(SetPacketType {
            packet_type: PacketType::Gfsk,
        })
        .await?;
        self.driver_command(SetRfFrequency {
            config: RfFrequencyConfig {
                frequency: profile.frequency_hz,
            },
        })
        .await?;
        self.driver_command(SetModulationParams {
            params: ModulationParams::Gfsk(GfskModParams {
                bit_rate: profile.bit_rate,
                pulse_shape: GfskPulseShape::NoFilter,
                bandwidth,
                freq_deviation: profile.frequency_deviation_hz,
            }),
        })
        .await?;
        let mut sync_word = [0; 8];
        sync_word[..profile.sync_word.len()].copy_from_slice(profile.sync_word);
        self.wait_ready().await?;
        Device::new(&mut self.spi)
            .write_register_async(SyncWord { value: sync_word })
            .await
            .map_err(Self::driver_error)?;
        self.check_status(OPCODE_WRITE_REGISTER).await?;
        self.driver_command(SetPacketParams {
            params: PacketParams::GFSK(GFSKPacketParams {
                preamble_length: profile.preamble_bits,
                preamble_detector_length: PreambleDetectorLength::Bits16,
                sync_word_length: 16,
                address_filtering: AddressFiltering::Disable,
                packet_type: GFSKPacketHeaderType::Fixed,
                payload_length: profile.packet_length,
                crc_type: CrcType::CrcOff,
                whitening_enable: false,
            }),
        })
        .await?;
        self.driver_command(SetBufferBaseAddress {
            config: BufferBaseAddressConfig {
                tx_base_addr: 0,
                rx_base_addr: 0,
            },
        })
        .await?;
        self.driver_command(SetDioIrqParams {
            config: DioIrqConfig {
                irq_mask: IrqMask::from_bits_retain(IRQ_MASK),
                dio1_mask: IrqMask::from_bits_retain(IRQ_DIO1_MASK),
                dio2_mask: IrqMask::empty(),
                dio3_mask: IrqMask::empty(),
            },
        })
        .await?;
        self.packet_length = Some(profile.packet_length);
        Ok(InitStatus {
            cleared_xosc_start_error,
        })
    }

    pub async fn start_receive(
        &mut self,
    ) -> Result<(), Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        if self.packet_length.is_none() {
            return Err(Error::NotConfigured);
        }
        self.standby().await?;
        self.driver_command(ClearIrqStatus {
            irq_mask: IrqMask::from_bits_retain(0xffff),
        })
        .await?;
        self.driver_command(SetRx {
            mode: RxMode::Continuous,
        })
        .await?;
        self.receiving = true;
        Ok(())
    }

    pub async fn standby(&mut self) -> Result<(), Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        self.receiving = false;
        self.driver_command(SetStandby {
            config: StandbyConfig::Rc,
        })
        .await?;
        Ok(())
    }

    pub async fn wait_for_irq(&mut self) -> Result<(), Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        self.irq.wait_for_high().await.map_err(Error::IrqPin)
    }

    pub fn irq_is_high(&mut self) -> Result<bool, IRQ::Error> {
        self.irq.is_high()
    }

    pub async fn get_irq_flags(
        &mut self,
    ) -> Result<u16, Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        Ok(self.driver_command(GetIrqStatus).await?.irq_mask.bits())
    }

    pub async fn read_data(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<PacketInfo, Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        let length = usize::from(self.packet_length.ok_or(Error::NotConfigured)?);
        if buffer.len() < length {
            return Err(Error::BufferTooSmall {
                required: length,
                available: buffer.len(),
            });
        }
        self.standby().await?;
        let buffer_status = self.driver_command(GetRxBufferStatus).await?.buffer_status;
        if usize::from(buffer_status.payload_length) != length {
            return Err(Error::InvalidPacketLength(buffer_status.payload_length));
        }
        self.wait_ready().await?;
        Device::new(&mut self.spi)
            .read_buffer_async(buffer_status.buffer_pointer, &mut buffer[..length])
            .await
            .map_err(Self::driver_error)?;
        self.check_status(OPCODE_READ_BUFFER).await?;
        let packet_status = self.driver_command(GetPacketStatus).await?.packet_status;
        Ok(PacketInfo {
            len: length,
            rssi_half_dbm: packet_status.gfsk_rssi_avg_half_dbm(),
        })
    }

    /// Handle receive IRQs and rearm before returning a packet or receive error.
    pub async fn handle_irq(
        &mut self,
        flags: u16,
        buffer: &mut [u8],
    ) -> Result<ReceiveEvent, Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        if flags & IRQ_ERRORS != 0 {
            self.start_receive().await?;
            return Ok(ReceiveEvent::RadioError(flags));
        }
        if flags & IRQ_RX_DONE == 0 {
            return Ok(ReceiveEvent::Pending);
        }

        let event = match self.read_data(buffer).await {
            Ok(packet) => ReceiveEvent::Packet(packet),
            Err(Error::InvalidPacketLength(length)) => ReceiveEvent::InvalidPacketLength(length),
            Err(error) => return Err(error),
        };
        self.start_receive().await?;
        Ok(event)
    }

    /// Wait for and read one packet, rearming reception before returning.
    ///
    /// Dropping this future during SPI or rearming can leave the radio state
    /// uncertain; restart and reconfigure it before reuse after cancellation.
    pub async fn receive(
        &mut self,
        buffer: &mut [u8],
    ) -> Result<PacketInfo, Error<SPI::Error, BUSY::Error, IRQ::Error>> {
        let length = usize::from(self.packet_length.ok_or(Error::NotConfigured)?);
        if !self.receiving {
            return Err(Error::NotReceiving);
        }
        if buffer.len() < length {
            return Err(Error::BufferTooSmall {
                required: length,
                available: buffer.len(),
            });
        }
        self.wait_for_irq().await?;
        let flags = self.get_irq_flags().await?;
        match self.handle_irq(flags, buffer).await? {
            ReceiveEvent::Packet(packet) => Ok(packet),
            ReceiveEvent::RadioError(flags) => Err(Error::ReceiveIrq(flags)),
            ReceiveEvent::InvalidPacketLength(length) => Err(Error::InvalidPacketLength(length)),
            ReceiveEvent::Pending => Err(Error::UnexpectedIrq(flags)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use embassy_futures::block_on;
    use embedded_hal::{digital, spi};
    use std::{
        cell::RefCell,
        rc::Rc,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
        task::{Context, Poll, Wake, Waker},
        vec,
        vec::Vec,
    };

    const WS90_868: FskProfile<'static> = FskProfile {
        frequency_hz: 868_350_000,
        bit_rate: 17_200,
        frequency_deviation_hz: 35_000,
        rx_bandwidth_hz: 117_300,
        preamble_bits: 48,
        sync_word: &[0x2d, 0xd4],
        packet_length: 64,
        crc_enabled: false,
        whitening_enabled: false,
    };

    struct MockSpi {
        commands: Vec<Vec<u8>>,
        status: u8,
        response_status: Option<u8>,
        fail_opcode: Option<u8>,
        device_errors: u16,
        calibration_errors: u16,
        irq_flags: u16,
        length: u8,
        offset: u8,
    }

    impl Default for MockSpi {
        fn default() -> Self {
            Self {
                commands: Vec::new(),
                status: 0x24,
                response_status: None,
                fail_opcode: None,
                device_errors: 0,
                calibration_errors: 0,
                irq_flags: 0,
                length: 64,
                offset: 9,
            }
        }
    }

    impl spi::ErrorType for MockSpi {
        type Error = spi::ErrorKind;
    }

    impl SpiDevice for MockSpi {
        async fn transaction(
            &mut self,
            ops: &mut [spi::Operation<'_, u8>],
        ) -> Result<(), Self::Error> {
            match ops {
                [
                    spi::Operation::Write(opcode),
                    spi::Operation::Write(parameters),
                    spi::Operation::Read(response),
                ] => {
                    assert_eq!(opcode.len(), 1);
                    if self.fail_opcode == Some(opcode[0]) {
                        return Err(spi::ErrorKind::Other);
                    }
                    if opcode[0] == 0xc0 {
                        assert!(parameters.is_empty());
                        response.copy_from_slice(&[self.status]);
                        return Ok(());
                    }
                    let mut command = opcode.to_vec();
                    command.extend_from_slice(parameters);
                    if !response.is_empty() {
                        command.push(0);
                    }
                    self.commands.push(command);
                    let response_status = self.response_status.unwrap_or(self.status);
                    match opcode[0] {
                        0x07 => {
                            assert_eq!(*parameters, [0, 0]);
                            self.device_errors = 0;
                        }
                        0x98 => self.device_errors |= self.calibration_errors,
                        0x17 => {
                            let errors = self.device_errors.to_be_bytes();
                            response.copy_from_slice(&[response_status, errors[0], errors[1]]);
                        }
                        0x12 => {
                            let flags = self.irq_flags.to_be_bytes();
                            response.copy_from_slice(&[response_status, flags[0], flags[1]]);
                        }
                        0x13 => {
                            response.copy_from_slice(&[response_status, self.length, self.offset])
                        }
                        0x14 => response.copy_from_slice(&[response_status, 0, 100, 103]),
                        _ => assert!(response.is_empty()),
                    }
                }
                [spi::Operation::Write(header), spi::Operation::Write(data)] => {
                    if self.fail_opcode == Some(header[0]) {
                        return Err(spi::ErrorKind::Other);
                    }
                    let mut command = header.to_vec();
                    command.extend_from_slice(data);
                    self.commands.push(command);
                }
                [spi::Operation::Write(header), spi::Operation::Read(data)] => {
                    if self.fail_opcode == Some(header[0]) {
                        return Err(spi::ErrorKind::Other);
                    }
                    self.commands.push(header.to_vec());
                    match *header {
                        [0x1e, offset, 0] => {
                            assert_eq!(*offset, self.offset);
                            for (i, byte) in data.iter_mut().enumerate() {
                                *byte = i as u8;
                            }
                        }
                        _ => panic!("unexpected read"),
                    }
                }
                _ => panic!("unexpected SPI transaction"),
            }
            Ok(())
        }
    }

    #[derive(Default)]
    struct MockBusy {
        high: bool,
        fail: bool,
    }

    impl digital::ErrorType for MockBusy {
        type Error = digital::ErrorKind;
    }

    impl InputPin for MockBusy {
        fn is_high(&mut self) -> Result<bool, Self::Error> {
            if self.fail {
                Err(digital::ErrorKind::Other)
            } else {
                Ok(self.high)
            }
        }
        fn is_low(&mut self) -> Result<bool, Self::Error> {
            self.is_high().map(|high| !high)
        }
    }

    impl Wait for MockBusy {
        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            if self.fail {
                Err(digital::ErrorKind::Other)
            } else if self.high {
                Ok(())
            } else {
                core::future::pending().await
            }
        }

        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            if self.fail {
                Err(digital::ErrorKind::Other)
            } else if !self.high {
                Ok(())
            } else {
                core::future::pending().await
            }
        }

        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            panic!("unexpected edge wait")
        }
        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            panic!("unexpected edge wait")
        }
        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            panic!("unexpected edge wait")
        }
    }

    #[derive(Default)]
    struct PinState {
        high: bool,
        waker: Option<Waker>,
    }

    #[derive(Clone, Default)]
    struct ControlledPin(Rc<RefCell<PinState>>);

    impl ControlledPin {
        fn set_high(&self, high: bool) {
            let waker = {
                let mut state = self.0.borrow_mut();
                state.high = high;
                state.waker.take()
            };
            if let Some(waker) = waker {
                waker.wake();
            }
        }

        async fn wait_for_level(&mut self, high: bool) -> Result<(), digital::ErrorKind> {
            core::future::poll_fn(|cx| {
                let mut state = self.0.borrow_mut();
                if state.high == high {
                    Poll::Ready(Ok(()))
                } else {
                    state.waker = Some(cx.waker().clone());
                    Poll::Pending
                }
            })
            .await
        }
    }

    impl digital::ErrorType for ControlledPin {
        type Error = digital::ErrorKind;
    }

    impl InputPin for ControlledPin {
        fn is_high(&mut self) -> Result<bool, Self::Error> {
            Ok(self.0.borrow().high)
        }
        fn is_low(&mut self) -> Result<bool, Self::Error> {
            Ok(!self.0.borrow().high)
        }
    }

    impl Wait for ControlledPin {
        async fn wait_for_high(&mut self) -> Result<(), Self::Error> {
            self.wait_for_level(true).await
        }
        async fn wait_for_low(&mut self) -> Result<(), Self::Error> {
            self.wait_for_level(false).await
        }
        async fn wait_for_rising_edge(&mut self) -> Result<(), Self::Error> {
            panic!("unexpected edge wait")
        }
        async fn wait_for_falling_edge(&mut self) -> Result<(), Self::Error> {
            panic!("unexpected edge wait")
        }
        async fn wait_for_any_edge(&mut self) -> Result<(), Self::Error> {
            panic!("unexpected edge wait")
        }
    }

    #[derive(Default)]
    struct PendingTimeout;

    impl DelayNs for PendingTimeout {
        async fn delay_ns(&mut self, ns: u32) {
            if ns == BUSY_TIMEOUT_US * 1_000 {
                core::future::pending::<()>().await;
            }
        }
    }

    #[derive(Default)]
    struct WakeCounter(AtomicUsize);

    impl Wake for WakeCounter {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
        fn wake_by_ref(self: &Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[derive(Default)]
    struct MockDelay {
        nanos: u64,
    }

    impl DelayNs for MockDelay {
        async fn delay_ns(&mut self, ns: u32) {
            self.nanos += u64::from(ns);
        }
    }

    fn radio() -> Sx1262<MockSpi, MockBusy, MockBusy, MockDelay> {
        let mut radio = Sx1262::new(
            MockSpi::default(),
            MockBusy::default(),
            MockBusy {
                high: true,
                fail: false,
            },
            MockDelay::default(),
        );
        radio.packet_length = Some(64);
        radio.receiving = true;
        radio
    }

    #[test]
    fn configures_validated_ws90_profile() {
        block_on(async {
            let mut radio = radio();
            radio.begin_fsk(&WS90_868).await.unwrap();
            assert_eq!(
                radio.spi.commands,
                vec![
                    vec![0x80, 0x00],
                    vec![0x17, 0],
                    vec![0x97, 0x06, 0, 1, 0x40],
                    vec![0x96, 0],
                    vec![0x89, 0x7f],
                    vec![0x98, 0xd7, 0xdb],
                    vec![0x17, 0],
                    vec![0x8a, 0],
                    vec![0x86, 0x36, 0x45, 0x99, 0x99],
                    vec![0x8b, 0x00, 0xe8, 0x8e, 0x00, 0x0b, 0x00, 0x8f, 0x5c],
                    vec![0x0d, 0x06, 0xc0, 0x2d, 0xd4, 0, 0, 0, 0, 0, 0],
                    vec![0x8c, 0, 48, 0x05, 16, 0, 0, 64, 1, 0],
                    vec![0x8f, 0, 0],
                    vec![0x08, 0x02, 0x6e, 0x02, 0x62, 0, 0, 0, 0],
                ]
            );
        });
    }

    #[test]
    fn writes_configured_sync_word_to_driver_register() {
        block_on(async {
            let mut radio = radio();
            radio
                .begin_fsk(&FskProfile {
                    sync_word: &[0xab, 0xcd],
                    ..WS90_868
                })
                .await
                .unwrap();
            assert!(
                radio
                    .spi
                    .commands
                    .contains(&vec![0x0d, 0x06, 0xc0, 0xab, 0xcd, 0, 0, 0, 0, 0, 0])
            );
            assert!(
                radio
                    .spi
                    .commands
                    .contains(&vec![0x8c, 0, 48, 5, 16, 0, 0, 64, 1, 0])
            );
        });
    }

    #[test]
    fn encodes_profile_boundaries_with_driver_commands() {
        block_on(async {
            for (frequency, bit_rate, deviation) in
                [(863_000_000, 600, 600), (870_000_000, 300_000, 200_000)]
            {
                let mut radio = radio();
                radio
                    .begin_fsk(&FskProfile {
                        frequency_hz: frequency,
                        bit_rate,
                        frequency_deviation_hz: deviation,
                        ..WS90_868
                    })
                    .await
                    .unwrap();
                let frequency_bytes = (((frequency as u64) << 25) / 32_000_000) as u32;
                let bitrate_bytes = 32 * 32_000_000u32 / bit_rate;
                let deviation_bytes = (((deviation as u64) << 25) / 32_000_000) as u32;
                let mut expected_frequency = vec![0x86];
                expected_frequency.extend_from_slice(&frequency_bytes.to_be_bytes());
                let mut expected_modulation = vec![0x8b];
                expected_modulation.extend_from_slice(&bitrate_bytes.to_be_bytes()[1..]);
                expected_modulation.extend_from_slice(&[0x00, 0x0b]);
                expected_modulation.extend_from_slice(&deviation_bytes.to_be_bytes()[1..]);
                assert!(radio.spi.commands.contains(&expected_frequency));
                assert!(radio.spi.commands.contains(&expected_modulation));
            }
        });
    }

    #[test]
    fn maps_profile_bandwidth_to_driver_setting() {
        block_on(async {
            let mut radio = radio();
            radio
                .begin_fsk(&FskProfile {
                    rx_bandwidth_hz: 156_200,
                    ..WS90_868
                })
                .await
                .unwrap();
            assert!(
                radio
                    .spi
                    .commands
                    .contains(&vec![0x8b, 0x00, 0xe8, 0x8e, 0x00, 0x1a, 0x00, 0x8f, 0x5c])
            );
        });
    }

    #[test]
    fn rejects_invalid_profiles_before_spi() {
        block_on(async {
            let mut radio = radio();
            for profile in [
                FskProfile {
                    frequency_hz: 433_920_000,
                    ..WS90_868
                },
                FskProfile {
                    bit_rate: 0,
                    ..WS90_868
                },
                FskProfile {
                    frequency_deviation_hz: 0,
                    ..WS90_868
                },
                FskProfile {
                    preamble_bits: 15,
                    ..WS90_868
                },
                FskProfile {
                    rx_bandwidth_hz: 125_000,
                    ..WS90_868
                },
                FskProfile {
                    sync_word: &[0x2d],
                    ..WS90_868
                },
                FskProfile {
                    packet_length: 0,
                    ..WS90_868
                },
                FskProfile {
                    crc_enabled: true,
                    ..WS90_868
                },
                FskProfile {
                    whitening_enabled: true,
                    ..WS90_868
                },
            ] {
                assert_eq!(radio.begin_fsk(&profile).await, Err(Error::InvalidProfile));
                assert!(radio.spi.commands.is_empty());
            }
        });
    }

    #[test]
    fn reads_packet_and_rearms() {
        block_on(async {
            let mut radio = radio();
            let mut buffer = [0; 64];
            radio.spi.irq_flags = IRQ_PREAMBLE | IRQ_SYNC | IRQ_RX_DONE;
            radio.start_receive().await.unwrap();
            assert_eq!(radio.get_irq_flags().await.unwrap(), 0x0e);
            let packet = radio.read_data(&mut buffer).await.unwrap();
            assert_eq!(buffer[0], 0);
            assert_eq!(buffer[63], 63);
            assert_eq!(packet.len, 64);
            assert_eq!(packet.rssi_half_dbm, -103);
            radio.start_receive().await.unwrap();
            assert_eq!(
                radio.spi.commands,
                vec![
                    vec![0x80, 0],
                    vec![0x02, 0xff, 0xff],
                    vec![0x82, 0xff, 0xff, 0xff],
                    vec![0x12, 0],
                    vec![0x80, 0],
                    vec![0x13, 0],
                    vec![0x1e, 9, 0],
                    vec![0x14, 0],
                    vec![0x80, 0],
                    vec![0x02, 0xff, 0xff],
                    vec![0x82, 0xff, 0xff, 0xff],
                ]
            );
        });
    }

    #[test]
    fn reports_invalid_response_status_and_retains_unknown_irq_bits() {
        block_on(async {
            let mut radio = radio();
            radio.spi.response_status = Some(0);
            assert_eq!(
                radio.get_irq_flags().await,
                Err(Error::DriverDeserializationError {
                    opcode: 0x12,
                    first_byte: Some(0),
                })
            );
            radio.spi.response_status = None;
            radio.spi.irq_flags = 0x8200;
            assert_eq!(radio.get_irq_flags().await, Ok(0x8200));
        });
    }

    #[test]
    fn reports_initialization_response_details() {
        block_on(async {
            let mut radio = radio();
            radio.spi.response_status = Some(0);
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Err(Error::DriverDeserializationError {
                    opcode: 0x17,
                    first_byte: Some(0),
                })
            );
        });
    }

    #[test]
    fn accepts_radio_response_with_reserved_command_status() {
        block_on(async {
            let mut radio = radio();
            radio.spi.response_status = Some(0xa2);
            assert!(radio.begin_fsk(&WS90_868).await.is_ok());
        });
    }

    #[test]
    fn preserves_spi_errors_from_driver_operations() {
        block_on(async {
            let mut radio = radio();
            radio.spi.fail_opcode = Some(0x80);
            assert_eq!(
                radio.standby().await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            radio.spi.fail_opcode = Some(0x1e);
            let mut buffer = [0; 64];
            assert_eq!(
                radio.read_data(&mut buffer).await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );

            radio.spi.fail_opcode = Some(0x02);
            assert_eq!(
                radio.start_receive().await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            radio.spi.fail_opcode = Some(0x13);
            assert_eq!(
                radio.read_data(&mut buffer).await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            radio.spi.fail_opcode = Some(0xc0);
            assert_eq!(
                radio.standby().await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            radio.spi.fail_opcode = Some(0x0d);
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            radio.spi.fail_opcode = Some(0x08);
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            assert_eq!(radio.packet_length, None);
            radio.spi.fail_opcode = Some(0x97);
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
        });
    }

    #[test]
    fn supports_other_fixed_packet_lengths() {
        block_on(async {
            let mut radio = radio();
            let profile = FskProfile {
                packet_length: 5,
                ..WS90_868
            };
            radio.begin_fsk(&profile).await.unwrap();
            assert!(
                radio
                    .spi
                    .commands
                    .iter()
                    .any(|cmd| cmd == &[0x8c, 0, 48, 5, 16, 0, 0, 5, 1, 0])
            );
            radio.spi.length = 5;
            let mut buffer = [0xff; 8];
            let packet = radio.read_data(&mut buffer).await.unwrap();
            assert_eq!(packet.len, 5);
            assert_eq!(buffer, [0, 1, 2, 3, 4, 0xff, 0xff, 0xff]);
        });
    }

    #[test]
    fn handles_receive_events_and_rearms() {
        block_on(async {
            let mut radio = radio();
            let mut buffer = [0; 64];
            assert!(matches!(
                radio.handle_irq(IRQ_PREAMBLE | IRQ_SYNC, &mut buffer).await,
                Ok(ReceiveEvent::Pending)
            ));
            assert!(radio.spi.commands.is_empty());

            let packet = match radio.handle_irq(IRQ_RX_DONE, &mut buffer).await.unwrap() {
                ReceiveEvent::Packet(packet) => packet,
                _ => panic!("expected packet"),
            };
            assert_eq!(packet.len, 64);
            assert_eq!(buffer[63], 63);
            assert_eq!(
                radio.spi.commands.last().unwrap(),
                &[0x82, 0xff, 0xff, 0xff]
            );

            radio.spi.commands.clear();
            radio.spi.length = 63;
            assert!(matches!(
                radio.handle_irq(IRQ_RX_DONE, &mut buffer).await,
                Ok(ReceiveEvent::InvalidPacketLength(63))
            ));
            assert!(!radio.spi.commands.iter().any(|cmd| cmd[0] == 0x1e));
            assert_eq!(
                radio.spi.commands.last().unwrap(),
                &[0x82, 0xff, 0xff, 0xff]
            );

            radio.spi.commands.clear();
            assert!(matches!(
                radio.handle_irq(IRQ_RX_DONE | IRQ_CRC_ERROR, &mut buffer).await,
                Ok(ReceiveEvent::RadioError(flags)) if flags == IRQ_RX_DONE | IRQ_CRC_ERROR
            ));
            assert!(!radio.spi.commands.iter().any(|cmd| cmd[0] == 0x13));
            assert_eq!(
                radio.spi.commands.last().unwrap(),
                &[0x82, 0xff, 0xff, 0xff]
            );

            radio.spi.fail_opcode = Some(0x82);
            assert!(matches!(
                radio.handle_irq(IRQ_TIMEOUT, &mut buffer).await,
                Err(Error::Spi(spi::ErrorKind::Other))
            ));
        });
    }

    #[test]
    fn reports_invalid_length_and_device_errors() {
        block_on(async {
            let mut radio = radio();
            let mut buffer = [0; 64];
            radio.spi.length = 63;
            assert!(matches!(
                radio.read_data(&mut buffer).await,
                Err(Error::InvalidPacketLength(63))
            ));
            assert!(!radio.spi.commands.iter().any(|cmd| cmd[0] == 0x1e));
            radio.spi.calibration_errors = 0x10;
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Err(Error::DeviceErrors(0x10))
            );
        });
    }

    #[test]
    fn clears_only_the_expected_pre_tcxo_oscillator_error() {
        block_on(async {
            let mut radio = radio();
            radio.spi.device_errors = 0x20;
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Ok(InitStatus {
                    cleared_xosc_start_error: true
                })
            );
            assert_eq!(radio.spi.commands[2], [0x07, 0, 0]);

            radio.spi.device_errors = 0x21;
            assert_eq!(
                radio.begin_fsk(&WS90_868).await,
                Err(Error::DeviceErrors(0x21))
            );
        });
    }

    #[test]
    fn reports_busy_timeout_pin_spi_and_status_errors() {
        block_on(async {
            let mut radio = radio();
            radio.busy.high = true;
            assert_eq!(radio.start_receive().await, Err(Error::BusyTimeout));
            assert_eq!(radio.delay.nanos, 250_000_000);
            assert!(radio.spi.commands.is_empty());
            radio.busy.high = false;
            radio.busy.fail = true;
            assert_eq!(
                radio.start_receive().await,
                Err(Error::BusyPin(digital::ErrorKind::Other))
            );
            radio.busy.fail = false;
            radio.spi.fail_opcode = Some(0x80);
            assert_eq!(
                radio.start_receive().await,
                Err(Error::Spi(spi::ErrorKind::Other))
            );
            radio.spi.fail_opcode = None;
            for status in [0x00, 0xff] {
                radio.spi.status = status;
                assert_eq!(
                    radio.start_receive().await,
                    Err(Error::InvalidStatus(status))
                );
            }
            for status in [0x26, 0x28, 0x2a] {
                radio.spi.status = status;
                assert_eq!(
                    radio.start_receive().await,
                    Err(Error::CommandFailed {
                        opcode: 0x80,
                        status
                    })
                );
            }
        });
    }

    #[test]
    fn receive_waits_for_irq_wakeup_and_rearms() {
        use core::future::Future;

        let irq = ControlledPin::default();
        let mut radio = Sx1262::new(
            MockSpi::default(),
            MockBusy::default(),
            irq.clone(),
            MockDelay::default(),
        );
        radio.packet_length = Some(64);
        radio.receiving = true;
        radio.spi.irq_flags = IRQ_RX_DONE;
        let mut buffer = [0; 64];
        let counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        {
            let mut receive = core::pin::pin!(radio.receive(&mut buffer));
            assert!(matches!(receive.as_mut().poll(&mut cx), Poll::Pending));
            irq.set_high(true);
            assert_eq!(counter.0.load(Ordering::SeqCst), 1);
            let packet = match receive.as_mut().poll(&mut cx) {
                Poll::Ready(Ok(packet)) => packet,
                _ => panic!("expected received packet"),
            };
            assert_eq!(packet.len, 64);
            assert_eq!(packet.rssi_half_dbm, -103);
        }
        assert_eq!(buffer[63], 63);
        assert_eq!(
            radio.spi.commands.last().unwrap(),
            &[0x82, 0xff, 0xff, 0xff]
        );
        assert!(radio.receiving);
    }

    #[test]
    fn busy_wait_yields_until_pin_goes_low() {
        use core::future::Future;

        let busy = ControlledPin::default();
        busy.set_high(true);
        let mut radio = Sx1262::new(
            MockSpi::default(),
            busy.clone(),
            MockBusy::default(),
            PendingTimeout,
        );
        radio.packet_length = Some(64);
        let counter = Arc::new(WakeCounter::default());
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        {
            let mut start = core::pin::pin!(radio.start_receive());
            assert!(matches!(start.as_mut().poll(&mut cx), Poll::Pending));
            busy.set_high(false);
            assert_eq!(counter.0.load(Ordering::SeqCst), 1);
            assert!(matches!(start.as_mut().poll(&mut cx), Poll::Ready(Ok(()))));
        }
        assert!(radio.receiving);
    }

    #[test]
    fn receive_reports_state_buffer_pin_and_irq_errors() {
        block_on(async {
            let mut radio = radio();
            let mut buffer = [0; 64];
            radio.packet_length = None;
            assert_eq!(radio.receive(&mut buffer).await, Err(Error::NotConfigured));
            radio.packet_length = Some(64);
            radio.receiving = false;
            assert_eq!(radio.receive(&mut buffer).await, Err(Error::NotReceiving));
            radio.receiving = true;
            assert_eq!(
                radio.receive(&mut buffer[..63]).await,
                Err(Error::BufferTooSmall {
                    required: 64,
                    available: 63
                })
            );
            assert!(radio.spi.commands.is_empty());
            radio.irq.fail = true;
            assert_eq!(
                radio.receive(&mut buffer).await,
                Err(Error::IrqPin(digital::ErrorKind::Other))
            );
            radio.irq.fail = false;
            radio.spi.irq_flags = IRQ_CRC_ERROR;
            assert_eq!(
                radio.receive(&mut buffer).await,
                Err(Error::ReceiveIrq(IRQ_CRC_ERROR))
            );
            assert!(radio.receiving);
            radio.spi.irq_flags = 0;
            assert_eq!(
                radio.receive(&mut buffer).await,
                Err(Error::UnexpectedIrq(0))
            );
        });
    }
}
