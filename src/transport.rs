//! esp-hal hardware transport adapter using SPI DMA.
//!
//! Two DMA TX buffers alternate: while one is being clocked out, the next write is copied into
//! the other. A write returns as soon as its transfer has started; the next bus operation, `wait`,
//! or `release` blocks until it completes.

use esp_hal::{
    Blocking,
    delay::Delay,
    dma::{DmaError, DmaRxBuf, DmaTxBuf},
    gpio::Output,
    spi::{
        Error as SpiError,
        master::{Address, Command, DataMode, SpiDma, SpiDmaTransfer},
    },
};

use crate::{Config, Driver, Error, Interface, registers};

/// Largest data phase a single esp-hal SPI DMA transfer accepts.
const MAX_DMA_SIZE: usize = 32736;

/// Owns the SPI DMA bus, its buffers, and the reset pin; the SPI peripheral controls CS.
pub struct Qspi<'d> {
    /// `None` only while `transfer` holds the bus.
    spi: Option<SpiDma<'d, Blocking>>,
    /// Transfer started by the last write and not yet waited for.
    transfer: Option<SpiDmaTransfer<'d, Blocking, DmaTxBuf>>,
    /// TX buffers not owned by an in-flight transfer.
    free: [Option<DmaTxBuf>; 2],
    rx: Option<DmaRxBuf>,
    reset: Output<'d>,
    delay: Delay,
}

impl<'d> Driver<Qspi<'d>> {
    /// Create the driver and validate its configuration without accessing hardware; call `init` next.
    ///
    /// `tx` buffers must be in internal RAM and the same size; larger buffers mean fewer
    /// transactions per frame, up to 32736 bytes each. `rx` only needs to hold register reads.
    pub fn new(
        spi: SpiDma<'d, Blocking>,
        tx: [DmaTxBuf; 2],
        rx: DmaRxBuf,
        reset: Output<'d>,
        config: Config,
    ) -> Result<Self, Error> {
        let [a, b] = tx;
        Self::with_interface(
            Qspi {
                spi: Some(spi),
                transfer: None,
                free: [Some(a), Some(b)],
                rx: Some(rx),
                reset,
                delay: Delay::new(),
            },
            config,
        )
    }

    /// Wait for pending transfers, then return the SPI bus, buffers, and reset pin without
    /// changing the panel state.
    pub fn release(self) -> (SpiDma<'d, Blocking>, [DmaTxBuf; 2], DmaRxBuf, Output<'d>) {
        let mut interface = self.into_interface();
        interface.wait();
        let [a, b] = interface.free;
        (
            interface.spi.expect("bus is idle"),
            [a.expect("buffer is free"), b.expect("buffer is free")],
            interface.rx.expect("rx buffer is present"),
            interface.reset,
        )
    }
}

impl Qspi<'_> {
    fn put_free(&mut self, buffer: DmaTxBuf) {
        let slot = self.free.iter_mut().find(|slot| slot.is_none());
        *slot.expect("at most two buffers") = Some(buffer);
    }

    fn take_free(&mut self) -> DmaTxBuf {
        if let Some(buffer) = self.free.iter_mut().find_map(Option::take) {
            return buffer;
        }
        self.wait();
        self.free
            .iter_mut()
            .find_map(Option::take)
            .expect("buffer is free after wait")
    }
}

impl Interface for Qspi<'_> {
    type Error = SpiError;

    fn max_write_len(&self) -> usize {
        self.free
            .iter()
            .flatten()
            .map(DmaTxBuf::capacity)
            .min()
            .unwrap_or(0)
            .min(MAX_DMA_SIZE)
    }

    fn write(&mut self, register: u8, data: &[u8], quad: bool) -> Result<(), Self::Error> {
        // Copy into a free buffer while the previous transfer, if any, is still running.
        let mut buffer = self.take_free();
        if data.len() > buffer.capacity().min(MAX_DMA_SIZE) {
            self.put_free(buffer);
            return Err(DmaError::Overflow.into());
        }
        buffer.fill(data);
        self.wait();
        let spi = self.spi.take().expect("bus is idle");
        let opcode = if quad {
            registers::WRITE_COLOR
        } else {
            registers::WRITE_COMMAND
        };
        match spi.half_duplex_write_buffer(
            if quad {
                DataMode::Quad
            } else {
                DataMode::Single
            },
            Command::_8Bit(u16::from(opcode), DataMode::Single),
            Address::_24Bit(u32::from(register) << 8, DataMode::Single),
            0,
            data.len(),
            buffer,
        ) {
            Ok(transfer) => {
                self.transfer = Some(transfer);
                Ok(())
            }
            Err((error, spi, buffer)) => {
                self.spi = Some(spi);
                self.put_free(buffer);
                Err(error)
            }
        }
    }

    fn read(&mut self, register: u8, data: &mut [u8]) -> Result<(), Self::Error> {
        self.wait();
        let mut rx = self.rx.take().expect("rx buffer is present");
        if data.len() > rx.capacity() {
            self.rx = Some(rx);
            return Err(DmaError::Overflow.into());
        }
        rx.set_length(data.len());
        let spi = self.spi.take().expect("bus is idle");
        match spi.half_duplex_read_buffer(
            DataMode::Single,
            Command::_8Bit(u16::from(registers::READ_COMMAND), DataMode::Single),
            Address::_24Bit(u32::from(register) << 8, DataMode::Single),
            0,
            data.len(),
            rx,
        ) {
            Ok(transfer) => {
                let (spi, rx) = transfer.wait();
                data.copy_from_slice(&rx.as_slice()[..data.len()]);
                self.spi = Some(spi);
                self.rx = Some(rx);
                Ok(())
            }
            Err((error, spi, rx)) => {
                self.spi = Some(spi);
                self.rx = Some(rx);
                Err(error)
            }
        }
    }

    fn wait(&mut self) {
        if let Some(transfer) = self.transfer.take() {
            let (spi, buffer) = transfer.wait();
            self.spi = Some(spi);
            self.put_free(buffer);
        }
    }

    fn reset(&mut self, high: bool) {
        self.wait();
        if high {
            self.reset.set_high();
        } else {
            self.reset.set_low();
        }
    }

    fn delay_ms(&mut self, ms: u32) {
        self.wait();
        self.delay.delay_millis(ms);
    }
}
