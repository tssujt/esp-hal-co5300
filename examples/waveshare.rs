#![no_std]
#![no_main]

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::Rectangle,
    text::Text,
};
use esp_backtrace as _;
use esp_hal::{
    delay::Delay,
    dma_rx_buffer, dma_tx_buffer,
    gpio::{Level, Output, OutputConfig},
    main,
    psram::{Psram, PsramConfig, PsramMode},
    spi::{
        Mode,
        master::{Config as SpiConfig, Spi},
    },
    time::Rate,
};
use esp_hal_co5300::{BufferedDisplay, Co5300, Config};

// The ESP-IDF second-stage bootloader requires this descriptor in the application image.
esp_bootloader_esp_idf::esp_app_desc!();

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default());
    let spi = Spi::new(
        peripherals.SPI2,
        SpiConfig::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(peripherals.GPIO38)
    .with_sio0(peripherals.GPIO4)
    .with_sio1(peripherals.GPIO5)
    .with_sio2(peripherals.GPIO6)
    .with_sio3(peripherals.GPIO7)
    .with_cs(peripherals.GPIO12)
    .with_dma(peripherals.DMA_CH0);
    // Two TX buffers alternate so the next chunk is copied while the previous one is sent.
    let tx = [
        dma_tx_buffer!(32736).unwrap(),
        dma_tx_buffer!(32736).unwrap(),
    ];
    let rx = dma_rx_buffer!(64).unwrap();
    let reset = Output::new(peripherals.GPIO39, Level::High, OutputConfig::default());
    let mut display = Co5300::new(
        spi,
        tx,
        rx,
        reset,
        Config {
            brightness: 160,
            ..Config::default()
        },
    )
    .unwrap();
    display.init().unwrap();

    // The base API clears the screen with a 4 KiB scratch buffer and no framebuffer.
    display.clear(0x0000).unwrap();
    display.fill_rect(40, 40, 120, 120, 0xF800).unwrap();
    Delay::new().delay_millis(400);

    // Store the roughly 450 KiB framebuffer in octal PSRAM to preserve internal RAM.
    let psram = Psram::new(
        peripherals.PSRAM,
        PsramConfig {
            mode: PsramMode::OctalSpi,
            ..Default::default()
        },
    );
    let (pointer, bytes) = psram.raw_parts();
    let count = 480 * 480;
    assert!(bytes >= count * core::mem::size_of::<u16>());
    assert_eq!(pointer.align_offset(core::mem::align_of::<u16>()), 0);
    // SAFETY: This function exclusively owns PSRAM; it is not registered with an allocator. Length and alignment are checked.
    // Initialize every u16 before creating a reference. The psram handle lives throughout the non-returning main function.
    let framebuffer = unsafe {
        core::ptr::write_bytes(pointer, 0, count * core::mem::size_of::<u16>());
        core::slice::from_raw_parts_mut(pointer.cast::<u16>(), count)
    };
    let mut graphics = BufferedDisplay::new(&mut display, framebuffer).unwrap();
    graphics.clear(Rgb565::BLACK).unwrap();
    for (x, color) in [(40, Rgb565::RED), (180, Rgb565::GREEN), (320, Rgb565::BLUE)] {
        graphics
            .fill_solid(
                &Rectangle::new(Point::new(x, 100), Size::new(120, 100)),
                color,
            )
            .unwrap();
    }
    // The shadow framebuffer preserves neighboring pixels for odd coordinates and single-pixel glyphs.
    Text::new(
        "CO5300 + Rust",
        Point::new(121, 261),
        MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE),
    )
    .draw(&mut graphics)
    .unwrap();
    Text::new(
        "480 x 480 RGB565",
        Point::new(121, 301),
        MonoTextStyle::new(&FONT_10X20, Rgb565::CYAN),
    )
    .draw(&mut graphics)
    .unwrap();
    graphics.flush().unwrap();
    esp_println::println!("CO5300: color blocks and text ready");
    loop {
        Delay::new().delay_millis(1_000);
    }
}
