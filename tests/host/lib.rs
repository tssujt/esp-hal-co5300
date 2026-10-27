//! Reuse production logic with a mock transport; run using the host toolchain.

#[path = "../../src/driver.rs"]
pub mod driver;
#[path = "../../src/graphics.rs"]
pub mod graphics;
#[path = "../../src/registers.rs"]
pub mod registers;

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use crate::{driver::*, graphics::BufferedDisplay, registers as reg};
    use embedded_graphics::{
        pixelcolor::{Rgb565, raw::RawU16},
        prelude::*,
        primitives::Rectangle,
    };

    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Event {
        Write(u8, Vec<u8>, bool),
        Read(u8),
        Reset(bool),
        Delay(u32),
    }
    #[derive(Default)]
    struct State {
        events: Vec<Event>,
        fail: Option<u8>,
        /// Overrides the mock's 512-byte transaction limit.
        max_write: Option<usize>,
    }
    struct Mock(Rc<RefCell<State>>);
    impl Interface for Mock {
        type Error = u8;
        fn max_write_len(&self) -> usize {
            self.0.borrow().max_write.unwrap_or(512)
        }
        fn write(&mut self, register: u8, data: &[u8], quad: bool) -> Result<(), u8> {
            let mut state = self.0.borrow_mut();
            state
                .events
                .push(Event::Write(register, data.to_vec(), quad));
            if state.fail == Some(register) {
                state.fail = None;
                Err(42)
            } else {
                Ok(())
            }
        }
        fn read(&mut self, register: u8, data: &mut [u8]) -> Result<(), u8> {
            self.0.borrow_mut().events.push(Event::Read(register));
            if self.0.borrow_mut().fail.take() == Some(register) {
                return Err(42);
            }
            data.copy_from_slice(&[0x33, 0x11, 0x00]);
            Ok(())
        }
        fn reset(&mut self, high: bool) {
            self.0.borrow_mut().events.push(Event::Reset(high));
        }
        fn delay_ms(&mut self, ms: u32) {
            self.0.borrow_mut().events.push(Event::Delay(ms));
        }
    }
    fn make(config: Config) -> (Driver<Mock>, Rc<RefCell<State>>) {
        let state = Rc::new(RefCell::new(State::default()));
        (
            Driver::with_interface(Mock(state.clone()), config).unwrap(),
            state,
        )
    }
    fn ready() -> (Driver<Mock>, Rc<RefCell<State>>) {
        let (mut driver, state) = make(Config::default());
        driver.init().unwrap();
        state.borrow_mut().events.clear();
        (driver, state)
    }
    fn write(register: u8, data: &[u8]) -> Event {
        Event::Write(register, data.to_vec(), false)
    }
    trait ConcatData {
        fn concat_data(&self) -> Vec<u8>;
    }
    impl ConcatData for Vec<(u8, Vec<u8>)> {
        fn concat_data(&self) -> Vec<u8> {
            self.iter().flat_map(|(_, data)| data.iter().copied()).collect()
        }
    }
    fn pixel_events(state: &Rc<RefCell<State>>) -> Vec<(u8, Vec<u8>)> {
        state
            .borrow()
            .events
            .iter()
            .filter_map(|event| match event {
                Event::Write(register, data, true) => Some((*register, data.clone())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn initialization_matches_reference_bsp_and_delays() {
        let (mut driver, state) = make(Config::default());
        assert_eq!(driver.clear(0), Err(Error::NotInitialized));
        assert!(state.borrow().events.is_empty());
        driver.init().unwrap();
        assert_eq!(
            state.borrow().events,
            vec![
                Event::Reset(false),
                Event::Delay(10),
                Event::Reset(true),
                Event::Delay(150),
                write(0x11, &[]),
                Event::Delay(600),
                write(0xFE, &[0x20]),
                write(0x19, &[0x10]),
                write(0x1C, &[0xA0]),
                write(0xFE, &[0]),
                write(0xC4, &[0x80]),
                write(0x3A, &[0x55]),
                write(0x35, &[0]),
                write(0x53, &[0x20]),
                write(0x51, &[255]),
                write(0x63, &[255]),
                write(0x2A, &[0, 0, 1, 0xDF]),
                write(0x2B, &[0, 0, 1, 0xDF]),
                write(0x36, &[0xA0]),
                write(0x29, &[]),
                Event::Delay(600),
            ]
        );
        driver.reset();
        assert_eq!(driver.clear(0), Err(Error::NotInitialized));
    }

    #[test]
    fn rejects_invalid_configuration_and_regions_before_spi() {
        for config in [
            Config {
                width: 0,
                ..Config::default()
            },
            Config {
                width: 479,
                ..Config::default()
            },
            Config {
                column_offset: u16::MAX - 1,
                ..Config::default()
            },
            Config {
                height: 482,
                ..Config::default()
            },
        ] {
            let state = Rc::new(RefCell::new(State::default()));
            assert!(Driver::with_interface(Mock(state.clone()), config).is_err());
            assert!(state.borrow().events.is_empty());
        }
        let (mut driver, state) = ready();
        assert_eq!(driver.write_area(0, 0, 0, 2, &[]), Err(Error::InvalidSize));
        assert_eq!(driver.fill_rect(1, 0, 2, 2, 0), Err(Error::Unaligned));
        assert_eq!(driver.fill_rect(0, 1, 2, 2, 0), Err(Error::Unaligned));
        assert_eq!(driver.fill_rect(0, 0, 3, 2, 0), Err(Error::Unaligned));
        assert_eq!(driver.fill_rect(0, 0, 2, 3, 0), Err(Error::Unaligned));
        assert_eq!(driver.fill_rect(478, 0, 4, 2, 0), Err(Error::OutOfBounds));
        assert_eq!(
            driver.fill_rect(65534, 0, 65534, 2, 0),
            Err(Error::OutOfBounds)
        );
        assert_eq!(
            driver.write_area(0, 0, 2, 2, &[0; 6]),
            Err(Error::InvalidDataLength {
                expected: 8,
                actual: 6
            })
        );
        assert!(state.borrow().events.is_empty());
    }

    #[test]
    fn byte_slices_split_at_max_write_len_with_ramwrc() {
        let (mut driver, state) = ready();
        let data: Vec<_> = (0..144).collect();
        driver.write_area(2, 4, 18, 4, &data).unwrap();
        assert_eq!(
            &state.borrow().events[..2],
            &[
                write(reg::CASET, &[0, 2, 0, 19]),
                write(reg::RASET, &[0, 4, 0, 7])
            ]
        );
        assert_eq!(pixel_events(&state), vec![(reg::RAMWR, data)]);
        state.borrow_mut().events.clear();
        let frame: Vec<u8> = (0..480 * 480 * 2).map(|index| index as u8).collect();
        driver.write_area(0, 0, 480, 480, &frame).unwrap();
        let chunks = pixel_events(&state);
        assert_eq!(state.borrow().events.len(), 2 + 900);
        assert_eq!(chunks[0].0, reg::RAMWR);
        assert!(chunks[1..].iter().all(|(command, data)| *command == reg::RAMWRC && data.len() == 512));
        assert_eq!(chunks.concat_data(), frame);
        // An odd limit is rounded down so no pixel straddles two transactions.
        state.borrow_mut().events.clear();
        state.borrow_mut().max_write = Some(513);
        driver.write_area(0, 0, 480, 480, &frame).unwrap();
        assert!(pixel_events(&state).iter().all(|(_, data)| data.len() == 512));
        state.borrow_mut().max_write = Some(usize::MAX);
        state.borrow_mut().events.clear();
        driver.write_area(0, 0, 480, 480, &frame).unwrap();
        assert_eq!(pixel_events(&state), vec![(reg::RAMWR, frame)]);
    }

    #[test]
    fn solid_fills_use_512_byte_chunks_and_exact_tail_lengths() {
        let (mut driver, state) = ready();
        for (width, height, lengths) in [
            (2, 2, vec![8]),
            (16, 16, vec![512]),
            (18, 18, vec![512, 136]),
        ] {
            state.borrow_mut().events.clear();
            driver.fill_rect(0, 0, width, height, 0xF81F).unwrap();
            let chunks = pixel_events(&state);
            assert_eq!(
                chunks
                    .iter()
                    .map(|(_, data)| data.len())
                    .collect::<Vec<_>>(),
                lengths
            );
            assert_eq!(chunks[0].0, reg::RAMWR);
            assert!(
                chunks[1..]
                    .iter()
                    .all(|(command, _)| *command == reg::RAMWRC)
            );
            assert!(
                chunks
                    .iter()
                    .all(|(_, data)| *data == [0xF8, 0x1F].repeat(data.len() / 2))
            );
        }
        state.borrow_mut().events.clear();
        driver.clear(0xF81F).unwrap();
        let chunks = pixel_events(&state);
        assert_eq!(chunks.len(), 900);
        assert_eq!(
            chunks.iter().map(|(_, data)| data.len()).sum::<usize>(),
            480 * 480 * 2
        );
        assert_eq!(chunks[0].0, reg::RAMWR);
        assert!(
            chunks[1..]
                .iter()
                .all(|(command, _)| *command == reg::RAMWRC)
        );
        assert!(
            chunks
                .iter()
                .all(|(_, data)| data == &[0xF8, 0x1F].repeat(256))
        );
    }

    #[test]
    fn pixel_iterator_endian_short_input_and_no_overconsumption() {
        let (mut driver, state) = ready();
        let values: Vec<u16> = (0..600).map(|x| 0xAB00 + x).collect();
        driver.write_pixels(0, 0, 300, 2, values.clone()).unwrap();
        let chunks = pixel_events(&state);
        assert_eq!(
            chunks
                .iter()
                .map(|(command, data)| (*command, data.len()))
                .collect::<Vec<_>>(),
            [(reg::RAMWR, 512), (reg::RAMWRC, 512), (reg::RAMWRC, 176)]
        );
        let actual: Vec<_> = pixel_events(&state)
            .into_iter()
            .flat_map(|(_, data)| data)
            .collect();
        assert_eq!(
            actual,
            values
                .iter()
                .flat_map(|x| x.to_be_bytes())
                .collect::<Vec<_>>()
        );
        state.borrow_mut().events.clear();
        assert_eq!(
            driver.write_pixels(0, 0, 300, 2, values[..270].iter().copied()),
            Err(Error::NotEnoughPixels {
                expected: 600,
                received: 270
            })
        );
        assert_eq!(pixel_events(&state).len(), 1);
        let mut pixels = 0..8;
        driver.write_pixels(0, 0, 2, 2, &mut pixels).unwrap();
        assert_eq!(pixels.next(), Some(4));
    }

    #[test]
    fn orientation_offsets_brightness_and_failed_orientation() {
        let config = Config {
            width: 100,
            height: 200,
            column_offset: 4,
            row_offset: 6,
            brightness: 77,
            ..Config::default()
        };
        let (mut driver, state) = make(config);
        driver.init().unwrap();
        assert_eq!(driver.dimensions(), (200, 100));
        assert!(state.borrow().events.contains(&write(reg::WRDISBV, &[77])));
        state.borrow_mut().events.clear();
        driver.fill_rect(0, 0, 2, 2, 0).unwrap();
        assert_eq!(
            &state.borrow().events[..2],
            &[
                write(reg::CASET, &[0, 6, 0, 7]),
                write(reg::RASET, &[0, 4, 0, 5])
            ]
        );
        let orientation = Orientation {
            rotation: Rotation::Deg0,
            mirror_x: true,
            mirror_y: false,
            color_order: ColorOrder::Bgr,
        };
        state.borrow_mut().fail = Some(reg::MADCTL);
        assert_eq!(driver.set_orientation(orientation), Err(Error::Spi(42)));
        assert_eq!(driver.orientation(), config.orientation);
        driver.set_orientation(orientation).unwrap();
        assert_eq!(driver.dimensions(), (100, 200));
        assert_eq!(orientation.madctl(), 0x48);
        for (rotation, value) in [
            (Rotation::Deg0, 0),
            (Rotation::Deg90, 0xA0),
            (Rotation::Deg180, 0xC0),
            (Rotation::Deg270, 0x60),
        ] {
            let base = Orientation {
                rotation,
                mirror_x: false,
                mirror_y: false,
                color_order: ColorOrder::Rgb,
            };
            assert_eq!(base.madctl(), value);
            assert_eq!(
                Orientation {
                    mirror_x: true,
                    ..base
                }
                .madctl(),
                value ^ if base.swaps_axes() { 0x80 } else { 0x40 }
            );
        }
    }

    #[test]
    fn sleep_te_controls_and_read_errors() {
        let (mut driver, state) = ready();
        driver.sleep_in().unwrap();
        driver.sleep_out().unwrap();
        driver
            .set_tearing_effect(TearingEffect::VerticalAndHorizontal)
            .unwrap();
        driver.display_off().unwrap();
        driver.display_on().unwrap();
        driver.invert(true).unwrap();
        driver.invert(false).unwrap();
        driver.all_pixels_on().unwrap();
        driver.all_pixels_off().unwrap();
        driver.normal_display().unwrap();
        driver.set_idle_mode(true).unwrap();
        driver.set_idle_mode(false).unwrap();
        assert_eq!(
            state.borrow().events,
            vec![
                write(0x10, &[]),
                Event::Delay(120),
                write(0x11, &[]),
                Event::Delay(120),
                write(0x34, &[]),
                write(0x35, &[1]),
                write(0x28, &[]),
                write(0x29, &[]),
                write(0x21, &[]),
                write(0x20, &[]),
                write(0x23, &[]),
                write(0x22, &[]),
                write(0x13, &[]),
                write(0x39, &[]),
                write(0x38, &[])
            ]
        );
        assert_eq!(driver.read_id(), Ok([0x33, 0x11, 0x00]));
        state.borrow_mut().fail = Some(reg::RDDID);
        assert_eq!(driver.read_id(), Err(Error::Spi(42)));
    }

    #[test]
    fn failed_init_and_stream_propagate_spi_errors() {
        let (mut driver, state) = make(Config::default());
        state.borrow_mut().fail = Some(reg::COLMOD);
        assert_eq!(driver.init(), Err(Error::Spi(42)));
        assert_eq!(driver.clear(0), Err(Error::NotInitialized));
        driver.init().unwrap();
        state.borrow_mut().events.clear();
        state.borrow_mut().fail = Some(reg::RAMWR);
        assert_eq!(
            driver.write_area(0, 0, 300, 2, &[0; 1200]),
            Err(Error::Spi(42))
        );
        assert_eq!(pixel_events(&state), vec![(reg::RAMWR, vec![0; 512])]);
        state.borrow_mut().events.clear();
        state.borrow_mut().fail = Some(reg::RAMWRC);
        assert_eq!(
            driver.write_pixels(0, 0, 300, 2, core::iter::repeat(0)),
            Err(Error::Spi(42))
        );
        assert_eq!(pixel_events(&state).len(), 2);
        state.borrow_mut().events.clear();
        state.borrow_mut().fail = Some(reg::RAMWRC);
        assert_eq!(driver.clear(0), Err(Error::Spi(42)));
        assert_eq!(pixel_events(&state).len(), 2);
    }

    #[test]
    fn buffered_flush_packs_rows_into_512_byte_chunks_and_retries_from_start() {
        let (mut driver, state) = make(Config {
            width: 32,
            height: 32,
            ..Config::default()
        });
        driver.init().unwrap();
        let mut buffer = [0; 32 * 32];
        let mut graphics = BufferedDisplay::new(&mut driver, &mut buffer).unwrap();
        graphics.flush().unwrap();
        state.borrow_mut().events.clear();
        graphics
            .fill_contiguous(
                &Rectangle::new(Point::new(1, 3), Size::new(18, 18)),
                (1..=18 * 18).map(|value| Rgb565::from(RawU16::new(value))),
            )
            .unwrap();
        let expected: Vec<u8> = (2..22)
            .flat_map(|y| {
                graphics.framebuffer()[y * 32..y * 32 + 20]
                    .iter()
                    .flat_map(|pixel| pixel.to_be_bytes())
            })
            .collect();
        state.borrow_mut().fail = Some(reg::RAMWRC);
        assert_eq!(graphics.flush(), Err(Error::Spi(42)));
        state.borrow_mut().events.clear();
        graphics.flush().unwrap();
        assert_eq!(
            &state.borrow().events[..2],
            &[
                write(reg::CASET, &[0, 0, 0, 19]),
                write(reg::RASET, &[0, 2, 0, 21]),
            ]
        );
        assert_eq!(
            pixel_events(&state),
            vec![
                (reg::RAMWR, expected[..512].to_vec()),
                (reg::RAMWRC, expected[512..].to_vec()),
            ]
        );
        state.borrow_mut().events.clear();
        graphics.flush().unwrap();
        assert!(state.borrow().events.is_empty());
    }

    #[test]
    fn isolated_pixel_preserves_neighbors_and_failed_flush_retries() {
        let (mut driver, state) = make(Config {
            width: 8,
            height: 8,
            ..Config::default()
        });
        driver.init().unwrap();
        let mut buffer = [0; 64];
        let mut graphics = BufferedDisplay::new(&mut driver, &mut buffer).unwrap();
        graphics.clear(Rgb565::RED).unwrap();
        graphics.flush().unwrap();
        state.borrow_mut().events.clear();
        graphics
            .draw_iter([
                Pixel(Point::new(1, 1), Rgb565::BLUE),
                Pixel(Point::new(-1, 0), Rgb565::WHITE),
            ])
            .unwrap();
        state.borrow_mut().fail = Some(reg::RAMWR);
        assert_eq!(graphics.flush(), Err(Error::Spi(42)));
        state.borrow_mut().events.clear();
        graphics.flush().unwrap();
        assert_eq!(
            &state.borrow().events[..2],
            &[
                write(reg::CASET, &[0, 0, 0, 1]),
                write(reg::RASET, &[0, 0, 0, 1])
            ]
        );
        assert_eq!(
            pixel_events(&state),
            vec![(reg::RAMWR, vec![0xF8, 0, 0xF8, 0, 0xF8, 0, 0, 0x1F])]
        );
        state.borrow_mut().events.clear();
        graphics.flush().unwrap();
        assert!(state.borrow().events.is_empty());
    }

    #[test]
    fn contiguous_clipping_preserves_source_stride_and_short_iterator() {
        let (mut driver, _) = make(Config {
            width: 8,
            height: 6,
            orientation: Orientation {
                rotation: Rotation::Deg0,
                ..Orientation::default()
            },
            ..Config::default()
        });
        let mut buffer = [0; 48];
        let mut graphics = BufferedDisplay::new(&mut driver, &mut buffer).unwrap();
        assert_eq!(graphics.size(), Size::new(8, 6));
        for x in -3..10 {
            for y in -3..8 {
                graphics.clear(Rgb565::BLACK).unwrap();
                let rectangle = Rectangle::new(Point::new(x, y), Size::new(4, 4));
                graphics
                    .fill_contiguous(&rectangle, (1..=13).map(|n| Rgb565::from(RawU16::new(n))))
                    .unwrap();
                let mut expected = [0; 48];
                for index in 0..13 {
                    let px = x + index % 4;
                    let py = y + index / 4;
                    if (0..8).contains(&px) && (0..6).contains(&py) {
                        expected[(py * 8 + px) as usize] = (index + 1) as u16;
                    }
                }
                assert_eq!(
                    graphics.framebuffer(),
                    expected,
                    "source rectangle {x}, {y}"
                );
            }
        }
        // Extreme coordinates and sizes must not overflow; empty or short iterators must terminate promptly.
        graphics
            .fill_contiguous(
                &Rectangle::new(
                    Point::new(i32::MIN, i32::MIN),
                    Size::new(u32::MAX, u32::MAX),
                ),
                [Rgb565::WHITE],
            )
            .unwrap();
        graphics
            .fill_solid(
                &Rectangle::new(
                    Point::new(i32::MAX, i32::MAX),
                    Size::new(u32::MAX, u32::MAX),
                ),
                Rgb565::WHITE,
            )
            .unwrap();
    }

    #[test]
    fn solid_clipping_and_bottom_right_alignment() {
        let (mut driver, state) = make(Config {
            width: 8,
            height: 8,
            ..Config::default()
        });
        driver.init().unwrap();
        let mut buffer = [0; 64];
        let mut graphics = BufferedDisplay::new(&mut driver, &mut buffer).unwrap();
        graphics.flush().unwrap();
        state.borrow_mut().events.clear();
        graphics
            .fill_solid(
                &Rectangle::new(Point::new(7, 7), Size::new(20, 20)),
                Rgb565::GREEN,
            )
            .unwrap();
        graphics.flush().unwrap();
        assert_eq!(
            &state.borrow().events[..2],
            &[
                write(reg::CASET, &[0, 6, 0, 7]),
                write(reg::RASET, &[0, 6, 0, 7])
            ]
        );
        assert_eq!(
            pixel_events(&state),
            vec![(reg::RAMWR, vec![0, 0, 0, 0, 0, 0, 7, 0xE0])]
        );
    }
}
