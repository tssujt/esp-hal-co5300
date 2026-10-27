# esp-hal-co5300

A `no_std` CO5300 AMOLED QSPI driver for ESP32-S3, built on esp-hal 1.2.x SPI DMA and using RGB565. The base driver requires no heap allocation. Optional `embedded-graphics` support adds buffered text and graphics rendering.

## Supported hardware

The target board is **Waveshare ESP32-S3-Touch-AMOLED-2.16**. Initialization follows the previously hardware-tested minimal driver and Waveshare BSP **v2.0.1**. The full API has been exercised on this board with `examples/hwtest.rs`.

| Board (official documentation)                                                      | Resolution | Controller / interface | Column offset                                                                                                                                                                                                           | Status                                            |
| ----------------------------------------------------------------------------------- | ---------- | ---------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------- |
| [ESP32-S3-Touch-AMOLED-2.16](https://docs.waveshare.com/ESP32-S3-Touch-AMOLED-2.16) | 480×480    | CO5300 / QSPI          | 0                                                                                                                                                                                                                       | Default target                                    |
| [ESP32-S3-Touch-AMOLED-1.75](https://docs.waveshare.com/ESP32-S3-Touch-AMOLED-1.75) | 466×466    | CO5300 / QSPI          | 6; see `set_gap` in the [official BSP](https://raw.githubusercontent.com/waveshareteam/Waveshare-ESP32-components/bfeb6e6d5737178cdde78b630c8118074da0a657/bsp/esp32_s3_touch_amoled_1_75/esp32_s3_touch_amoled_1_75.c) | Untested; initialization and pins need adaptation |
| [ESP32-S3-Touch-AMOLED-1.64](https://docs.waveshare.com/ESP32-S3-Touch-AMOLED-1.64) | 280×456    | CO5300 / QSPI          | 20; see `offsetx1/2` in the [official V1.1 example](https://raw.githubusercontent.com/waveshareteam/ESP32-S3-Touch-AMOLED-1.64-v2/21b952243d306692774f3b5bfeb3eb5cff10f851/Arduino/examples/06_LVGL_Test/lcd_bsp.c)     | Untested; initialization and pins need adaptation |

Offsets come from the linked official implementations; all use a row offset of 0. Sharing a controller does not make the 2.16 initialization sequence interchangeable with another panel's. Only officially documented models are listed; this is not a blanket compatibility claim for CO5300/ICNA5300 modules.

## Quick start

```toml
[dependencies]
esp-hal-co5300 = { version = "0.1", features = ["embedded-graphics"] }
```

Create an `SpiDma<'_, Blocking>` (`Spi::new(...).with_dma(DMA_CH0)`), two equal-size DMA TX buffers in internal RAM, a small DMA RX buffer, and a reset `Output` using the wiring below. See [examples/waveshare.rs](examples/waveshare.rs) for a complete application. Omit `embedded-graphics` if you only need the base API.

```rust,ignore
use esp_hal_co5300::{Co5300, Config};

let tx = [dma_tx_buffer!(32736).unwrap(), dma_tx_buffer!(32736).unwrap()];
let rx = dma_rx_buffer!(64).unwrap();
let mut lcd = Co5300::new(spi.with_dma(peripherals.DMA_CH0), tx, rx, reset, Config {
    brightness: 160,
    ..Config::default()
})?;
lcd.init()?;
lcd.clear(0x0000)?;
lcd.fill_rect(40, 60, 100, 80, 0xF800)?;
lcd.write_pixels(0, 0, 2, 2, [0xF800, 0x07E0, 0x001F, 0xFFFF])?;
```

- **Pixels:** `write_area` sends a big-endian RGB565 slice in DMA-buffer-sized chunks (RAMWR, then RAMWRC). `write_pixels` accepts row-major `u16` values; it and `fill_rect` / `clear` stream through a 4 KiB stack buffer.
- **DMA pipelining:** the two TX buffers alternate, so the next chunk is copied while the previous one is on the bus. Methods return once their last transfer has started; call `wait()` to block until the bus is idle (e.g. before timing or entering light sleep). Any later command waits automatically.
- **Controls:** brightness, display on/off, sleep, rotation/mirroring/RGB-BGR, TE, inversion, all-pixels-on/off, idle mode, and ID readback. Use `normal_display()` to exit forced all-pixels modes.
- **Validation:** dimensions, offsets, origins, and region sizes must be even; regions must be nonempty and in bounds. Invalid parameters return errors. SPI failures or short iterators can leave partial updates.
- **Orientation:** dimensions and offsets use unrotated axes. The default MADCTL is the BSP's `0xA0`; 90°/270° swap logical dimensions and offsets. Redraw after changing orientation.

See the documentation in [src/driver.rs](src/driver.rs) for detailed API and error semantics.

## Wiring

The 2.16 board already connects these signals. Use **SPI2, 40 MHz, Mode 0, MSB-first**, configuring data pins with `with_sio0` through `with_sio3`.

| Signal | SCK | SIO0 | SIO1 | SIO2 | SIO3 | CS  | RST |
| ------ | --- | ---- | ---- | ---- | ---- | --- | --- |
| GPIO   | 38  | 4    | 5    | 6    | 7    | 12  | 39  |

CS is controlled by SPI; RST is active low. Single-line readback uses bidirectional SIO0. No D/C pin is needed. The TE API configures panel output but does not wait for the TE signal.

## embedded-graphics

`BufferedDisplay` implements `OriginDimensions` and `DrawTarget<Color = Rgb565>`, borrowing a caller-owned `&mut [u16]`. Call `flush()` after drawing. Dirty regions expand to even boundaries while preserving neighboring pixels; `fill_solid` and `fill_contiguous` support clipping and batched fills.

A 480×480 framebuffer requires **460800 bytes**; the example stores it in PSRAM and draws color blocks and text. Construction clears the buffer and marks the whole screen dirty. Failed transfers retain the dirty region for retry.

`flush()` converts native-endian pixels to big-endian bytes in a **4 KiB stack buffer**, packing successive rows without transmitting row padding. Up to 2048 pixels use one RAMWR transaction; larger regions use RAMWRC continuation blocks. It allocates no second full-frame buffer and never reinterprets native-endian pixels as wire bytes.

## Build and test

Install the `esp` Rust toolchain and Xtensa GCC. This repository includes target settings, `build-std`, linker flags, and the example's ESP-IDF application descriptor.

```sh
source "$HOME/export-esp.sh"
cargo +esp build --lib --target xtensa-esp32s3-none-elf
cargo +esp build --lib --features embedded-graphics --target xtensa-esp32s3-none-elf
cargo +esp build --example waveshare --features embedded-graphics --target xtensa-esp32s3-none-elf
cargo +esp clippy --lib --target xtensa-esp32s3-none-elf -- -D warnings
cargo +esp clippy --lib --examples --features embedded-graphics --target xtensa-esp32s3-none-elf -- -D warnings
cargo +stable test --manifest-path tests/host/Cargo.toml --target aarch64-apple-darwin
cargo +stable clippy --manifest-path tests/host/Cargo.toml --all-targets --target aarch64-apple-darwin -- -D warnings
```

Adjust the host-test target for your machine. `+esp` overrides any shell toolchain setting. Linker flags suppress only the RWX-segment warning from esp-hal's bare-metal layout, not Rust warnings. The example ELF is at `target/xtensa-esp32s3-none-elf/debug/examples/waveshare`.

rustfmt, clippy (ESP32-S3 and host tests), typos, mdformat, and file hygiene checks run through [prek](https://github.com/j178/prek), configured in `prek.toml`. Run `prek install` once to enable the Git hook, or `prek run --all-files` to check everything. CI runs the same hooks, the host tests, and the ESP32-S3 builds.

## Limitations

- **Transfer size:** one esp-hal SPI DMA transfer carries at most 32736 bytes, so a 460800-byte frame takes 15 chunks with 32736-byte buffers. The blocking esp-hal API always copies into the DMA buffer; the copy of chunk *n+1* overlaps the transfer of chunk *n*. Zero-copy DMA from PSRAM is only exposed by esp-hal's async API.

- **Memory:** the example uses 2 × 32736 bytes of internal RAM for TX buffers. Smaller buffers work and only increase the number of transactions.

- **Measured on the 2.16 board (40 MHz):**

  | Full-screen operation     | Blocking FIFO (before) | DMA                                |
  | ------------------------- | ---------------------- | ---------------------------------- |
  | `write_area` from PSRAM   | 90 ms                  | 25 ms (~40 fps; bus limit ≈ 23 ms) |
  | 4 × `fill_rect`           | 89 ms                  | 26 ms                              |
  | `write_pixels` (iterator) | 213 ms                 | 130 ms (CPU-bound in the iterator) |
  | `BufferedDisplay::flush`  | 150 ms                 | 71 ms                              |

- Initialization preserves both 600 ms BSP waits; `sleep_in/out` each wait 120 ms. Call `init()` after a reset. `read_id()` uses single-line register 0x04 but returns zeros on the 2.16 board; readback appears unsupported there.

## References / official drivers

| Resource                                                                                                                                                   | Purpose                                                                                                                                                                                                                                                                     |
| ---------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [Waveshare 2.16 documentation (current Wiki entry)](https://docs.waveshare.com/ESP32-S3-Touch-AMOLED-2.16)                                                 | Board specifications, pins, and resources                                                                                                                                                                                                                                   |
| [Waveshare BSP: esp32_s3_touch_amoled_2_16 v2.0.1](https://components.espressif.com/components/waveshare/esp32_s3_touch_amoled_2_16/versions/2.0.1/readme) | **Initialization sequence source**; see `lcd_init_cmds` in the version's [official source](https://raw.githubusercontent.com/waveshareteam/Waveshare-ESP32-components/bfeb6e6d5737178cdde78b630c8118074da0a657/bsp/esp32_s3_touch_amoled_2_16/esp32_s3_touch_amoled_2_16.c) |
| [Espressif esp_lcd_co5300](https://components.espressif.com/components/espressif/esp_lcd_co5300)                                                           | Official controller component                                                                                                                                                                                                                                               |
| [CO5300 datasheet V0.00 (PDF)](https://dl.espressif.com/AE/esp-iot-solution/CO5300_Datasheet_V0.00.pdf)                                                    | Protocol, registers, and timing                                                                                                                                                                                                                                             |
| [esp-hal 1.2.0 / ESP32-S3 API documentation](https://docs.espressif.com/projects/rust/esp-hal/1.2.0/esp32s3/esp_hal/index.html)                            | Available documentation for the 1.2 series; this project locks 1.2.2                                                                                                                                                                                                        |

All linked resources were checked with `curl -L` on **2026-10-27** and returned HTTP 200 after redirects. The old `www.waveshare.com/wiki/ESP32-S3-Touch-AMOLED-2.16` URL and the separate esp-hal 1.2.2 documentation entry returned 404; the accessible alternatives are linked above.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in this crate by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
