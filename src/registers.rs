//! CO5300 user-page registers and QSPI opcodes.

/// Single-line register write opcode.
pub const WRITE_COMMAND: u8 = 0x02;
/// Single-line register read opcode.
pub const READ_COMMAND: u8 = 0x03;
/// Display-memory write opcode with single-line addressing and quad-line pixel data.
pub const WRITE_COLOR: u8 = 0x32;
/// Read the three-byte device identifier.
pub const RDDID: u8 = 0x04;
/// Enter sleep mode.
pub const SLPIN: u8 = 0x10;
/// Exit sleep mode.
pub const SLPOUT: u8 = 0x11;
/// Restore normal display operation after all-pixels-on/off modes.
pub const NORON: u8 = 0x13;
/// Disable color inversion.
pub const INVOFF: u8 = 0x20;
/// Enable color inversion.
pub const INVON: u8 = 0x21;
/// Force all pixels off without modifying display memory.
pub const ALLPOFF: u8 = 0x22;
/// Force all pixels on without modifying display memory.
pub const ALLPON: u8 = 0x23;
/// Disable display output.
pub const DISPOFF: u8 = 0x28;
/// Enable display output.
pub const DISPON: u8 = 0x29;
/// Column window with inclusive start and end addresses.
pub const CASET: u8 = 0x2A;
/// Row window with inclusive start and end addresses.
pub const RASET: u8 = 0x2B;
/// Start writing display memory at the window origin.
pub const RAMWR: u8 = 0x2C;
/// Disable tearing-effect output.
pub const TEOFF: u8 = 0x34;
/// Enable tearing-effect output.
pub const TEON: u8 = 0x35;
/// Address orientation and RGB/BGR color order.
pub const MADCTL: u8 = 0x36;
/// Exit reduced-color mode.
pub const IDMOFF: u8 = 0x38;
/// Enter reduced-color mode.
pub const IDMON: u8 = 0x39;
/// Pixel format; 0x55 selects RGB565.
pub const COLMOD: u8 = 0x3A;
/// Continue writing from the previous display-memory position.
pub const RAMWRC: u8 = 0x3C;
/// Normal-mode brightness, in the range 0..=255.
pub const WRDISBV: u8 = 0x51;
/// Display brightness control enable.
pub const WRCTRLD: u8 = 0x53;
/// High-brightness-mode brightness.
pub const WRHBMDISBV: u8 = 0x63;
/// SPI mode control.
pub const SPIMODE: u8 = 0xC4;
/// First byte of the device identifier.
pub const RDID1: u8 = 0xDA;
/// Second byte of the device identifier.
pub const RDID2: u8 = 0xDB;
/// Third byte of the device identifier.
pub const RDID3: u8 = 0xDC;
/// Select the vendor command page; 0 selects the user page.
pub const PAGE: u8 = 0xFE;
/// BSP configuration register on vendor page 0x20; its meaning is undocumented.
pub const PAGE20_CONFIG_19: u8 = 0x19;
/// Another BSP configuration register on vendor page 0x20; its meaning is undocumented.
pub const PAGE20_CONFIG_1C: u8 = 0x1C;
/// Reverse the row address direction.
pub const MY: u8 = 0x80;
/// Reverse the column address direction.
pub const MX: u8 = 0x40;
/// Swap rows and columns, following Espressif's driver; early datasheets do not describe this bit.
pub const MV: u8 = 0x20;
/// Select BGR color order.
pub const BGR: u8 = 0x08;
