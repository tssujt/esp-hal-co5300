//! Hardware-independent commands, parameter validation, and pixel streaming.

use crate::registers as reg;

/// Stack buffer for generated pixels; also capped by `Interface::max_write_len`.
const CHUNK_BYTES: usize = 4096;
const PANEL_SIZE: u16 = 480;

/// Replaceable blocking transport interface, also used for hardware-independent protocol tests.
pub trait Interface {
    /// Underlying bus error.
    type Error;
    /// Largest data phase `write` accepts; longer pixel data is split into RAMWR + RAMWRC.
    fn max_write_len(&self) -> usize {
        usize::MAX
    }
    /// Write a register or display memory; `quad` affects only the data phase.
    /// The transfer may still be running on return; `data` may be reused immediately.
    fn write(&mut self, register: u8, data: &[u8], quad: bool) -> Result<(), Self::Error>;
    /// Block until every started transfer has completed.
    fn wait(&mut self) {}
    /// Read a register over a single data line; the buffer must be nonempty.
    fn read(&mut self, register: u8, data: &mut [u8]) -> Result<(), Self::Error>;
    /// Set the reset pin level.
    fn reset(&mut self, high: bool);
    /// Block for the specified number of milliseconds.
    fn delay_ms(&mut self, ms: u32);
}

/// Driver errors; invalid external parameters return an error.
#[derive(Debug, PartialEq, Eq)]
pub enum Error<E> {
    /// Underlying SPI error, preserved without conversion.
    Spi(E),
    /// Dimensions must be nonzero and no greater than 480.
    InvalidSize,
    /// Origins, dimensions, and offsets must be even.
    Unaligned,
    /// The region exceeds the logical canvas or the offset window exceeds the panel address range.
    OutOfBounds,
    /// The byte slice or external framebuffer has an incorrect length.
    InvalidDataLength {
        /// Required element count.
        expected: usize,
        /// Actual element count.
        actual: usize,
    },
    /// The iterator ended early; previously completed chunks may already have been written.
    NotEnoughPixels {
        /// Number of pixels required by the window.
        expected: usize,
        /// Pixels consumed from the iterator, including the unsent final chunk.
        received: usize,
    },
    /// Initialization has not succeeded, or a reset requires reinitialization.
    NotInitialized,
}

/// Rotation relative to the native MADCTL axes; the visual direction depends on panel mounting.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Rotation {
    /// Keep the coordinate axes unchanged.
    #[default]
    Deg0,
    /// Swap the axes and reverse row addressing (MADCTL = 0xA0).
    Deg90,
    /// Reverse both row and column addressing.
    Deg180,
    /// Swap the axes and reverse column addressing.
    Deg270,
}

/// Panel color order; input values always use RGB565 encoding.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColorOrder {
    /// Red, green, blue.
    #[default]
    Rgb,
    /// Blue, green, red.
    Bgr,
}

/// MADCTL orientation settings; mirroring is applied along the rotated logical axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Orientation {
    /// Rotation angle.
    pub rotation: Rotation,
    /// Mirror the logical X axis.
    pub mirror_x: bool,
    /// Mirror the logical Y axis.
    pub mirror_y: bool,
    /// RGB/BGR color order.
    pub color_order: ColorOrder,
}

impl Default for Orientation {
    fn default() -> Self {
        // Match the Waveshare BSP's MADCTL value of 0xA0.
        Self {
            rotation: Rotation::Deg90,
            mirror_x: false,
            mirror_y: false,
            color_order: ColorOrder::Rgb,
        }
    }
}

impl Orientation {
    /// Whether rows and columns are swapped, including logical dimensions and offsets.
    pub const fn swaps_axes(self) -> bool {
        matches!(self.rotation, Rotation::Deg90 | Rotation::Deg270)
    }

    /// Build the value written to register 0x36.
    pub const fn madctl(self) -> u8 {
        let mut value = match self.rotation {
            Rotation::Deg0 => 0,
            Rotation::Deg90 => reg::MY | reg::MV,
            Rotation::Deg180 => reg::MY | reg::MX,
            Rotation::Deg270 => reg::MX | reg::MV,
        };
        if self.mirror_x {
            value ^= if self.swaps_axes() { reg::MY } else { reg::MX };
        }
        if self.mirror_y {
            value ^= if self.swaps_axes() { reg::MX } else { reg::MY };
        }
        if matches!(self.color_order, ColorOrder::Bgr) {
            value |= reg::BGR;
        }
        value
    }
}

/// Panel configuration; dimensions and offsets use the unrotated address axes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// Unrotated visible width: a nonzero even value no greater than 480.
    pub width: u16,
    /// Unrotated visible height: a nonzero even value no greater than 480.
    pub height: u16,
    /// Unrotated column address offset; must be even.
    pub column_offset: u16,
    /// Unrotated row address offset; must be even.
    pub row_offset: u16,
    /// Initial brightness, in the range 0..=255.
    pub brightness: u8,
    /// Initial orientation; defaults to the BSP's value of 0xA0.
    pub orientation: Orientation,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            width: PANEL_SIZE,
            height: PANEL_SIZE,
            column_offset: 0,
            row_offset: 0,
            brightness: 255,
            orientation: Orientation::default(),
        }
    }
}

impl Config {
    fn validate<E>(self) -> Result<(), Error<E>> {
        if self.width == 0
            || self.height == 0
            || self.width > PANEL_SIZE
            || self.height > PANEL_SIZE
        {
            return Err(Error::InvalidSize);
        }
        if (self.width | self.height | self.column_offset | self.row_offset) & 1 != 0 {
            return Err(Error::Unaligned);
        }
        if u32::from(self.width) + u32::from(self.column_offset) > u32::from(PANEL_SIZE)
            || u32::from(self.height) + u32::from(self.row_offset) > u32::from(PANEL_SIZE)
        {
            return Err(Error::OutOfBounds);
        }
        Ok(())
    }
}

/// Tearing-effect output mode; the driver does not wait for the TE pin.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TearingEffect {
    /// Disable output.
    Off,
    /// Output vertical blanking information only.
    Vertical,
    /// Output both vertical and horizontal blanking information.
    VerticalAndHorizontal,
}

/// Generic driver; ESP32-S3 applications normally construct it with `Co5300::new`.
pub struct Driver<T: Interface> {
    interface: T,
    config: Config,
    orientation: Orientation,
    initialized: bool,
}

impl<T: Interface> Driver<T> {
    /// Create a driver with a custom transport without accessing hardware.
    pub fn with_interface(interface: T, config: Config) -> Result<Self, Error<T::Error>> {
        config.validate()?;
        Ok(Self {
            interface,
            config,
            orientation: config.orientation,
            initialized: false,
        })
    }

    /// Return the underlying transport without changing the panel state.
    pub fn into_interface(self) -> T {
        self.interface
    }

    /// Current logical dimensions; width and height are swapped at 90 and 270 degrees.
    pub fn dimensions(&self) -> (u16, u16) {
        if self.orientation.swaps_axes() {
            (self.config.height, self.config.width)
        } else {
            (self.config.width, self.config.height)
        }
    }

    /// The most recently applied orientation.
    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    /// Hardware reset: drive low for 10 ms, then wait 150 ms after release; call `init` afterward.
    pub fn reset(&mut self) {
        self.initialized = false;
        self.interface.reset(false);
        self.interface.delay_ms(10);
        self.interface.reset(true);
        self.interface.delay_ms(150);
    }

    /// Reset the panel and execute the Waveshare BSP v2.0.1 initialization sequence.
    /// May be called repeatedly; restores the initial brightness and orientation, including both 600 ms BSP waits.
    pub fn init(&mut self) -> Result<(), Error<T::Error>> {
        self.reset();
        self.command(reg::SLPOUT, &[])?;
        self.interface.delay_ms(600);
        // Keep the vendor-page values from the verified board sequence without inferring their meaning.
        for (register, value) in [
            (reg::PAGE, 0x20),
            (reg::PAGE20_CONFIG_19, 0x10),
            (reg::PAGE20_CONFIG_1C, 0xA0),
            (reg::PAGE, 0x00),
            (reg::SPIMODE, 0x80),
            (reg::COLMOD, 0x55),
            (reg::TEON, 0x00),
            (reg::WRCTRLD, 0x20),
            (reg::WRDISBV, self.config.brightness),
            (reg::WRHBMDISBV, 0xFF),
        ] {
            self.command(register, &[value])?;
        }
        self.orientation = self.config.orientation;
        let (width, height) = self.dimensions();
        self.window(0, 0, width, height)?;
        self.command(reg::MADCTL, &[self.orientation.madctl()])?;
        self.command(reg::DISPON, &[])?;
        self.interface.delay_ms(600);
        self.initialized = true;
        Ok(())
    }

    fn ready(&self) -> Result<(), Error<T::Error>> {
        if self.initialized {
            Ok(())
        } else {
            Err(Error::NotInitialized)
        }
    }

    fn command(&mut self, register: u8, data: &[u8]) -> Result<(), Error<T::Error>> {
        self.interface
            .write(register, data, false)
            .map_err(Error::Spi)
    }

    fn control(&mut self, register: u8, data: &[u8]) -> Result<(), Error<T::Error>> {
        self.ready()?;
        self.command(register, data)
    }

    fn area(&self, x: u16, y: u16, width: u16, height: u16) -> Result<usize, Error<T::Error>> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidSize);
        }
        if (x | y | width | height) & 1 != 0 {
            return Err(Error::Unaligned);
        }
        let (screen_width, screen_height) = self.dimensions();
        if u32::from(x) + u32::from(width) > u32::from(screen_width)
            || u32::from(y) + u32::from(height) > u32::from(screen_height)
        {
            return Err(Error::OutOfBounds);
        }
        Ok(usize::from(width) * usize::from(height))
    }

    // Only call after region validation; every sum is at most 480.
    fn window(&mut self, x: u16, y: u16, width: u16, height: u16) -> Result<(), Error<T::Error>> {
        let (dx, dy) = if self.orientation.swaps_axes() {
            (self.config.row_offset, self.config.column_offset)
        } else {
            (self.config.column_offset, self.config.row_offset)
        };
        let (x, y) = (x + dx, y + dy);
        let [xh, xl] = x.to_be_bytes();
        let [x1h, x1l] = (x + width - 1).to_be_bytes();
        let [yh, yl] = y.to_be_bytes();
        let [y1h, y1l] = (y + height - 1).to_be_bytes();
        self.command(reg::CASET, &[xh, xl, x1h, x1l])?;
        self.command(reg::RASET, &[yh, yl, y1h, y1l])
    }

    /// Even byte count per pixel transaction, at most `limit`.
    fn chunk_len(&self, limit: usize) -> usize {
        (self.interface.max_write_len().min(limit) & !1).max(2)
    }

    fn pixels(&mut self, first: bool, data: &[u8]) -> Result<(), Error<T::Error>> {
        self.interface
            .write(if first { reg::RAMWR } else { reg::RAMWRC }, data, true)
            .map_err(Error::Spi)
    }

    /// Write row-major RGB565 bytes, most significant byte first for each pixel.
    /// The origin and dimensions must be even; the length must equal `width * height * 2`.
    /// Send the slice in the largest transactions the interface accepts: RAMWR, then RAMWRC.
    /// Validate parameters before any SPI operation; a transfer failure can leave a partial update.
    pub fn write_area(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        data: &[u8],
    ) -> Result<(), Error<T::Error>> {
        let expected = self.area(x, y, width, height)? * 2;
        if data.len() != expected {
            return Err(Error::InvalidDataLength {
                expected,
                actual: data.len(),
            });
        }
        self.ready()?;
        self.window(x, y, width, height)?;
        let chunk = self.chunk_len(usize::MAX);
        for (index, part) in data.chunks(chunk).enumerate() {
            self.pixels(index == 0, part)?;
        }
        Ok(())
    }

    /// Write row-major `u16` RGB565 pixels, converting to big-endian bytes in a 4 KiB stack buffer.
    /// Use RAMWR for the first buffer and RAMWRC for subsequent buffers.
    /// Consume only the required pixels; short input returns an error without rolling back completed chunks.
    pub fn write_pixels<I>(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        pixels: I,
    ) -> Result<(), Error<T::Error>>
    where
        I: IntoIterator<Item = u16>,
    {
        let expected = self.area(x, y, width, height)?;
        self.ready()?;
        self.window(x, y, width, height)?;
        let mut pixels = pixels.into_iter();
        let mut buffer = [0; CHUNK_BYTES];
        let chunk = self.chunk_len(CHUNK_BYTES);
        let mut received = 0;
        while received < expected {
            let count = (expected - received).min(chunk / 2);
            let first = received == 0;
            for bytes in buffer[..count * 2].as_chunks_mut::<2>().0 {
                let pixel = pixels
                    .next()
                    .ok_or(Error::NotEnoughPixels { expected, received })?;
                *bytes = pixel.to_be_bytes();
                received += 1;
            }
            self.pixels(first, &buffer[..count * 2])?;
        }
        Ok(())
    }

    /// Fill a solid-color region using a repeated 4 KiB stack buffer, without allocating a full frame.
    /// Use RAMWR for the first buffer and RAMWRC for subsequent buffers.
    pub fn fill_rect(
        &mut self,
        x: u16,
        y: u16,
        width: u16,
        height: u16,
        color: u16,
    ) -> Result<(), Error<T::Error>> {
        let count = self.area(x, y, width, height)?;
        self.ready()?;
        self.window(x, y, width, height)?;
        let mut buffer = [0; CHUNK_BYTES];
        for pixel in buffer.as_chunks_mut::<2>().0 {
            *pixel = color.to_be_bytes();
        }
        let chunk = self.chunk_len(CHUNK_BYTES);
        let bytes = count * 2;
        for offset in (0..bytes).step_by(chunk) {
            self.pixels(offset == 0, &buffer[..(bytes - offset).min(chunk)])?;
        }
        Ok(())
    }

    /// Block until the last pixel or command transfer has finished on the bus.
    /// Other methods return once their final transfer has started.
    pub fn wait(&mut self) {
        self.interface.wait();
    }

    /// Fill the logical canvas with the specified RGB565 color.
    pub fn clear(&mut self, color: u16) -> Result<(), Error<T::Error>> {
        let (width, height) = self.dimensions();
        self.fill_rect(0, 0, width, height, color)
    }

    /// Set brightness in the range 0..=255.
    pub fn set_brightness(&mut self, brightness: u8) -> Result<(), Error<T::Error>> {
        self.control(reg::WRDISBV, &[brightness])
    }
    /// Enable display output without implicitly waking the panel.
    pub fn display_on(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::DISPON, &[])
    }
    /// Disable display output without implicitly entering sleep mode.
    pub fn display_off(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::DISPOFF, &[])
    }
    /// Enter sleep and wait 120 ms; `sleep_out` can be called immediately after this returns.
    pub fn sleep_in(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::SLPIN, &[])?;
        self.interface.delay_ms(120);
        Ok(())
    }
    /// Exit sleep and wait 120 ms; `display_on` can then be called.
    pub fn sleep_out(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::SLPOUT, &[])?;
        self.interface.delay_ms(120);
        Ok(())
    }
    /// Set rotation, mirroring, and RGB/BGR order; update logical dimensions only after success.
    /// Redraw the entire screen after changing orientation; the MV rotation behavior requires hardware verification.
    pub fn set_orientation(&mut self, orientation: Orientation) -> Result<(), Error<T::Error>> {
        self.control(reg::MADCTL, &[orientation.madctl()])?;
        self.orientation = orientation;
        Ok(())
    }
    /// Set TE output, disabling it first so switching between output modes takes effect.
    pub fn set_tearing_effect(&mut self, effect: TearingEffect) -> Result<(), Error<T::Error>> {
        self.control(reg::TEOFF, &[])?;
        match effect {
            TearingEffect::Off => Ok(()),
            TearingEffect::Vertical => self.command(reg::TEON, &[0]),
            TearingEffect::VerticalAndHorizontal => self.command(reg::TEON, &[1]),
        }
    }
    /// Enable or disable color inversion without modifying display memory.
    pub fn invert(&mut self, enabled: bool) -> Result<(), Error<T::Error>> {
        self.control(if enabled { reg::INVON } else { reg::INVOFF }, &[])
    }
    /// Force all pixels on without modifying display memory; restore with `normal_display`.
    pub fn all_pixels_on(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::ALLPON, &[])
    }
    /// Force all pixels off without modifying display memory; use `normal_display` to cancel either forced mode.
    pub fn all_pixels_off(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::ALLPOFF, &[])
    }
    /// Restore normal display-memory output after all-pixels-on/off or partial display modes.
    pub fn normal_display(&mut self) -> Result<(), Error<T::Error>> {
        self.control(reg::NORON, &[])
    }
    /// Enable or disable reduced-color display mode without changing the RGB565 input format.
    pub fn set_idle_mode(&mut self, enabled: bool) -> Result<(), Error<T::Error>> {
        self.control(if enabled { reg::IDMON } else { reg::IDMOFF }, &[])
    }
    /// Read a three-byte ID using 0x04. Board readback is unverified; success does not establish a valid ID.
    /// Uses a single data line with no dummy cycles; see the hardware limitations in the README.
    pub fn read_id(&mut self) -> Result<[u8; 3], Error<T::Error>> {
        self.ready()?;
        let mut id = [0; 3];
        self.interface
            .read(reg::RDDID, &mut id)
            .map_err(Error::Spi)?;
        Ok(id)
    }
}
