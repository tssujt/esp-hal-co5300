//! Borrow an external RGB565 framebuffer and flush dirty regions on even boundaries.

use embedded_graphics::{
    pixelcolor::Rgb565,
    prelude::{DrawTarget, IntoStorage, OriginDimensions, Pixel, Size},
    primitives::Rectangle,
};

use crate::driver::{Driver, Error, Interface};

#[derive(Clone, Copy)]
struct Bounds {
    x0: u16,
    y0: u16,
    x1: u16,
    y1: u16,
}

/// Allocation-free drawing adapter that preserves all neighboring pixels.
///
/// `DrawTarget` operations only modify the borrowed framebuffer; call [`Self::flush`] explicitly.
/// Construction clears the buffer and marks the entire screen dirty, replacing existing content on the first flush.
/// A 480 by 480 framebuffer requires 460800 bytes; PSRAM is recommended.
/// The exclusive driver borrow keeps the shadow framebuffer consistent with display memory.
pub struct BufferedDisplay<'a, T: Interface> {
    display: &'a mut Driver<T>,
    buffer: &'a mut [u16],
    width: u16,
    height: u16,
    dirty: Option<Bounds>,
}

impl<'a, T: Interface> BufferedDisplay<'a, T> {
    /// Borrow a buffer exactly matching the logical canvas and clear it to black without SPI access.
    pub fn new(display: &'a mut Driver<T>, buffer: &'a mut [u16]) -> Result<Self, Error<T::Error>> {
        let (width, height) = display.dimensions();
        let expected = usize::from(width) * usize::from(height);
        if buffer.len() != expected {
            return Err(Error::InvalidDataLength {
                expected,
                actual: buffer.len(),
            });
        }
        buffer.fill(0);
        Ok(Self {
            display,
            buffer,
            width,
            height,
            dirty: Some(Bounds {
                x0: 0,
                y0: 0,
                x1: width,
                y1: height,
            }),
        })
    }

    /// Read the current framebuffer as row-major RGB565 values.
    pub fn framebuffer(&self) -> &[u16] {
        self.buffer
    }

    /// Flush the dirty region, expanding to even boundaries and filling extra pixels from the buffer.
    /// Convert native-endian u16 pixels through write_pixels using a 4 KiB stack buffer.
    /// Regions of at most 256 pixels use one RAMWR; larger regions continue with RAMWRC.
    /// No additional full-frame buffer is allocated, and row padding is never transmitted.
    /// On SPI failure, retain the entire dirty region; the next flush resets the window and starts with RAMWR.
    pub fn flush(&mut self) -> Result<(), Error<T::Error>> {
        let Some(area) = self.dirty else {
            return Ok(());
        };
        let x0 = area.x0 & !1;
        let y0 = area.y0 & !1;
        let x1 = (area.x1 + 1) & !1;
        let y1 = (area.y1 + 1) & !1;
        let stride = usize::from(self.width);
        let buffer = &*self.buffer;
        let pixels = (y0..y1).flat_map(|y| {
            let start = usize::from(y) * stride;
            buffer[start + usize::from(x0)..start + usize::from(x1)]
                .iter()
                .copied()
        });
        self.display
            .write_pixels(x0, y0, x1 - x0, y1 - y0, pixels)?;
        self.dirty = None;
        Ok(())
    }

    fn mark(&mut self, bounds: Bounds) {
        self.dirty = Some(match self.dirty {
            Some(old) => Bounds {
                x0: old.x0.min(bounds.x0),
                y0: old.y0.min(bounds.y0),
                x1: old.x1.max(bounds.x1),
                y1: old.y1.max(bounds.y1),
            },
            None => bounds,
        });
    }

    // Use i64 bounds to avoid overflow with negative coordinates, i32::MAX, or u32::MAX sizes.
    fn clip(&self, area: &Rectangle) -> Option<Bounds> {
        let x0 = i64::from(area.top_left.x).max(0);
        let y0 = i64::from(area.top_left.y).max(0);
        let x1 =
            (i64::from(area.top_left.x) + i64::from(area.size.width)).min(i64::from(self.width));
        let y1 =
            (i64::from(area.top_left.y) + i64::from(area.size.height)).min(i64::from(self.height));
        (x0 < x1 && y0 < y1).then_some(Bounds {
            x0: x0 as u16,
            y0: y0 as u16,
            x1: x1 as u16,
            y1: y1 as u16,
        })
    }
}

impl<T: Interface> OriginDimensions for BufferedDisplay<'_, T> {
    fn size(&self) -> Size {
        Size::new(u32::from(self.width), u32::from(self.height))
    }
}

impl<T: Interface> OriginDimensions for Driver<T> {
    fn size(&self) -> Size {
        let (width, height) = self.dimensions();
        Size::new(u32::from(width), u32::from(height))
    }
}

impl<T: Interface> DrawTarget for BufferedDisplay<'_, T> {
    type Color = Rgb565;
    type Error = Error<T::Error>;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels {
            if point.x < 0
                || point.y < 0
                || point.x >= i32::from(self.width)
                || point.y >= i32::from(self.height)
            {
                continue;
            }
            let (x, y) = (point.x as u16, point.y as u16);
            self.buffer[usize::from(y) * usize::from(self.width) + usize::from(x)] =
                color.into_storage();
            self.mark(Bounds {
                x0: x,
                y0: y,
                x1: x + 1,
                y1: y + 1,
            });
        }
        Ok(())
    }

    fn fill_solid(&mut self, area: &Rectangle, color: Self::Color) -> Result<(), Self::Error> {
        let Some(bounds) = self.clip(area) else {
            return Ok(());
        };
        let color = color.into_storage();
        for y in bounds.y0..bounds.y1 {
            let start = usize::from(y) * usize::from(self.width);
            self.buffer[start + usize::from(bounds.x0)..start + usize::from(bounds.x1)].fill(color);
        }
        self.mark(bounds);
        Ok(())
    }

    fn fill_contiguous<I>(&mut self, area: &Rectangle, colors: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Self::Color>,
    {
        let Some(bounds) = self.clip(area) else {
            return Ok(());
        };
        let mut colors = colors.into_iter();
        let source_width = u64::from(area.size.width);
        let skip = (i64::from(bounds.y0) - i64::from(area.top_left.y)) as u64 * source_width
            + (i64::from(bounds.x0) - i64::from(area.top_left.x)) as u64;
        if !skip_colors(&mut colors, skip) {
            return Ok(());
        }
        self.mark(bounds);
        for y in bounds.y0..bounds.y1 {
            let start = usize::from(y) * usize::from(self.width);
            for pixel in
                &mut self.buffer[start + usize::from(bounds.x0)..start + usize::from(bounds.x1)]
            {
                // embedded-graphics permits a short color iterator; leave the remaining pixels unchanged.
                let Some(color) = colors.next() else {
                    return Ok(());
                };
                *pixel = color.into_storage();
            }
            if y + 1 < bounds.y1
                && !skip_colors(&mut colors, source_width - u64::from(bounds.x1 - bounds.x0))
            {
                break;
            }
        }
        Ok(())
    }

    fn clear(&mut self, color: Self::Color) -> Result<(), Self::Error> {
        self.buffer.fill(color.into_storage());
        self.dirty = Some(Bounds {
            x0: 0,
            y0: 0,
            x1: self.width,
            y1: self.height,
        });
        Ok(())
    }
}

// Handle large clipping offsets on 32-bit targets; stop immediately when the iterator ends.
fn skip_colors<I: Iterator>(colors: &mut I, mut count: u64) -> bool {
    while count != 0 {
        let step = count.min(usize::MAX as u64) as usize;
        if colors.nth(step - 1).is_none() {
            return false;
        }
        count -= step as u64;
    }
    true
}
