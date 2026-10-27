#![no_std]
#![forbid(unsafe_code)]
#![warn(missing_docs)]
//! CO5300 AMOLED driver using half-duplex QSPI with SPI DMA on the ESP32-S3.
//!
//! The default initialization targets Waveshare ESP32-S3-Touch-AMOLED-2.16.
//! No heap allocation is used; pixels are big-endian RGB565 bytes or native-endian `u16` values.
//! Enable `embedded-graphics` to draw through `BufferedDisplay` with a borrowed framebuffer.
//! Call `flush` to align dirty regions to even boundaries while preserving neighboring pixels.

mod driver;
pub mod registers;
mod transport;

pub use driver::{ColorOrder, Config, Driver, Interface, Orientation, Rotation, TearingEffect};
pub use transport::Qspi;

/// CO5300 driver backed by esp-hal blocking SPI DMA.
pub type Co5300<'d> = Driver<Qspi<'d>>;
/// Invalid parameters or a transfer failure retaining the original esp-hal SPI error.
pub type Error = driver::Error<esp_hal::spi::Error>;
/// Generic error type for custom transport implementations.
pub use driver::Error as DriverError;

#[cfg(feature = "embedded-graphics")]
mod graphics;
#[cfg(feature = "embedded-graphics")]
pub use graphics::BufferedDisplay;
