//! Staged on-board test: each step prints what the screen should show, then holds it.

#![no_std]
#![no_main]

use embedded_graphics::{
    mono_font::{MonoTextStyle, ascii::FONT_10X20},
    pixelcolor::Rgb565,
    prelude::*,
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
    time::{Instant, Rate},
};
use esp_hal_co5300::{BufferedDisplay, Co5300, Config, Orientation, Rotation, TearingEffect};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

const RED: u16 = 0xF800;
const GREEN: u16 = 0x07E0;
const BLUE: u16 = 0x001F;
const WHITE: u16 = 0xFFFF;
const BLACK: u16 = 0x0000;
const HOLD_MS: u32 = 4_000;

fn step(index: u32, expect: &str) {
    println!("[{:02}] {}", index, expect);
}

fn hold() {
    Delay::new().delay_millis(HOLD_MS);
}

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
    let mut display = Co5300::new(spi, tx, rx, reset, Config::default()).unwrap();

    let start = Instant::now();
    display.init().unwrap();
    println!("init took {} ms", start.elapsed().as_millis());

    // PSRAM layout: [frame bytes for write_area | u16 shadow framebuffer].
    let psram = Psram::new(
        peripherals.PSRAM,
        PsramConfig {
            mode: PsramMode::OctalSpi,
            ..Default::default()
        },
    );
    let (pointer, size) = psram.raw_parts();
    let pixels = 480 * 480;
    assert!(size >= pixels * 4);
    // SAFETY: PSRAM is used only here and is not registered with an allocator; the two slices do not overlap.
    let (frame, shadow) = unsafe {
        core::ptr::write_bytes(pointer, 0, pixels * 4);
        (
            core::slice::from_raw_parts_mut(pointer, pixels * 2),
            core::slice::from_raw_parts_mut(pointer.add(pixels * 2).cast::<u16>(), pixels),
        )
    };

    loop {
        step(1, "read_id (0x04)");
        match display.read_id() {
            Ok(id) => println!("     id = {:02X?}", id),
            Err(e) => println!("     read_id error: {:?}", e),
        }

        step(
            2,
            "fill_rect streaming (RAMWRC): quadrants TL red, TR green, BL blue, BR white",
        );
        let t = Instant::now();
        display.fill_rect(0, 0, 240, 240, RED).unwrap();
        display.fill_rect(240, 0, 240, 240, GREEN).unwrap();
        display.fill_rect(0, 240, 240, 240, BLUE).unwrap();
        display.fill_rect(240, 240, 240, 240, WHITE).unwrap();
        display.wait();
        println!(
            "     4x fill_rect (full screen) took {} ms",
            t.elapsed().as_millis()
        );
        hold();

        step(
            3,
            "write_pixels streaming: horizontal red gradient, dark left -> bright right",
        );
        let t = Instant::now();
        display
            .write_pixels(
                0,
                0,
                480,
                480,
                (0..480 * 480).map(|i| (((i % 480) * 32 / 480) as u16) << 11),
            )
            .unwrap();
        display.wait();
        println!(
            "     write_pixels (full screen) took {} ms",
            t.elapsed().as_millis()
        );
        hold();

        step(
            4,
            "write_area DMA chunks: 8x8 black/white checkerboard, 60px cells",
        );
        for (i, px) in frame.as_chunks_mut::<2>().0.iter_mut().enumerate() {
            let (x, y) = (i % 480, i / 480);
            let color = if (x / 60 + y / 60) % 2 == 0 {
                WHITE
            } else {
                BLACK
            };
            *px = color.to_be_bytes();
        }
        let t = Instant::now();
        display.write_area(0, 0, 480, 480, frame).unwrap();
        display.wait();
        let ms = t.elapsed().as_millis();
        println!(
            "     write_area (full screen) took {} ms (~{} fps)",
            ms,
            1000 / ms.max(1)
        );
        hold();

        for (n, rotation) in [
            Rotation::Deg0,
            Rotation::Deg90,
            Rotation::Deg180,
            Rotation::Deg270,
        ]
        .into_iter()
        .enumerate()
        {
            step(
                5 + n as u32,
                "orientation: red 120px square at logical top-left, green bar along logical top edge",
            );
            println!("     rotation = {:?}", rotation);
            display
                .set_orientation(Orientation {
                    rotation,
                    ..Orientation::default()
                })
                .unwrap();
            display.clear(BLACK).unwrap();
            display.fill_rect(0, 0, 480, 20, GREEN).unwrap();
            display.fill_rect(0, 0, 120, 120, RED).unwrap();
            hold();
        }
        display.set_orientation(Orientation::default()).unwrap();

        step(
            9,
            "invert on: quadrants become cyan / magenta / yellow / black",
        );
        display.fill_rect(0, 0, 240, 240, RED).unwrap();
        display.fill_rect(240, 0, 240, 240, GREEN).unwrap();
        display.fill_rect(0, 240, 240, 240, BLUE).unwrap();
        display.fill_rect(240, 240, 240, 240, WHITE).unwrap();
        display.invert(true).unwrap();
        hold();
        display.invert(false).unwrap();

        step(10, "all_pixels_on: entire screen white");
        display.all_pixels_on().unwrap();
        hold();
        step(11, "all_pixels_off: entire screen black");
        display.all_pixels_off().unwrap();
        hold();
        step(12, "normal_display: quadrants come back");
        display.normal_display().unwrap();
        hold();

        step(13, "idle mode on: reduced colors (quadrants still visible)");
        display.set_idle_mode(true).unwrap();
        hold();
        display.set_idle_mode(false).unwrap();

        step(14, "brightness ramp 255 -> 0 -> 255");
        for b in (0..=255u8).rev().step_by(5).chain((0..=255u8).step_by(5)) {
            display.set_brightness(b).unwrap();
            Delay::new().delay_millis(20);
        }
        display.set_brightness(255).unwrap();

        step(
            15,
            "display_off 2s (black), then display_on: quadrants come back",
        );
        display.display_off().unwrap();
        Delay::new().delay_millis(2_000);
        display.display_on().unwrap();
        hold();

        step(
            16,
            "sleep_in 2s (black), then sleep_out + display_on: quadrants come back",
        );
        display.sleep_in().unwrap();
        Delay::new().delay_millis(2_000);
        display.sleep_out().unwrap();
        display.display_on().unwrap();
        hold();

        step(
            17,
            "tearing effect V / V+H / off: no visible change expected",
        );
        display.set_tearing_effect(TearingEffect::Vertical).unwrap();
        display
            .set_tearing_effect(TearingEffect::VerticalAndHorizontal)
            .unwrap();
        display.set_tearing_effect(TearingEffect::Off).unwrap();

        step(
            18,
            "embedded-graphics: odd-coordinate text on black, then a partial flush",
        );
        {
            let mut g = BufferedDisplay::new(&mut display, shadow).unwrap();
            let style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
            Text::new("esp-hal-co5300 hwtest", Point::new(121, 201), style)
                .draw(&mut g)
                .unwrap();
            let t = Instant::now();
            g.flush().unwrap();
            println!("     full flush took {} ms", t.elapsed().as_millis());
            Text::new(
                "partial flush OK",
                Point::new(151, 263),
                MonoTextStyle::new(&FONT_10X20, Rgb565::YELLOW),
            )
            .draw(&mut g)
            .unwrap();
            let t = Instant::now();
            g.flush().unwrap();
            println!("     partial flush took {} ms", t.elapsed().as_millis());
        }
        hold();
        println!("---- cycle done, restarting ----");
    }
}
