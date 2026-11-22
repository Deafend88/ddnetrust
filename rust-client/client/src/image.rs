//! PNG loading. C++: `CImageLoader::LoadPng` (`src/engine/gfx/image_loader.cpp`) with
//! libpng's transformations (16 to 8 bit by dropping the low byte, palettes and gray
//! below 8 bit expanded, tRNS to alpha), and `ConvertToRgba` (image_manipulation.cpp).

use std::io::Cursor;

/// C++: `CImageInfo::EImageFormat`, same numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Rgb = 0,
    Rgba = 1,
    R = 2,
    Ra = 3,
}

impl Format {
    pub fn channels(self) -> usize {
        match self {
            Format::R => 1,
            Format::Ra => 2,
            Format::Rgb => 3,
            Format::Rgba => 4,
        }
    }
}

/// C++: `CImageInfo`.
#[derive(Clone, Debug)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub format: Format,
    pub data: Vec<u8>,
}

/// libpng's default `PNG_USER_WIDTH_MAX`/`PNG_USER_HEIGHT_MAX`.
const USER_MAX: u32 = 1_000_000;

/// C++: `CImageLoader::LoadPng` from memory. `None` where libpng fails.
pub fn load_png(bytes: &[u8]) -> Option<Image> {
    // libpng stops after the image data, so a missing IEND does not matter to C++;
    // png-rs reads the next chunk header, so give it one (unused when IEND exists)
    let mut bytes = bytes.to_vec();
    bytes.extend_from_slice(b"\0\0\0\0IEND\xae\x42\x60\x82");
    let mut options = png::DecodeOptions::default();
    options.set_ignore_adler32(false); // libpng checks the zlib checksum
    let mut decoder = png::Decoder::new_with_options(Cursor::new(bytes), options);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = reader.info().size();
    if width == 0 || height == 0 || width > USER_MAX || height > USER_MAX {
        return None;
    }
    let format = match reader.output_color_type().0 {
        png::ColorType::Grayscale => Format::R,
        png::ColorType::GrayscaleAlpha => Format::Ra,
        png::ColorType::Rgb => Format::Rgb,
        png::ColorType::Rgba => Format::Rgba,
        png::ColorType::Indexed => return None, // EXPAND removes palettes
    };
    let mut data = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut data).ok()?;
    data.truncate(info.buffer_size());
    Some(Image {
        width,
        height,
        format,
        data,
    })
}

/// C++: the file overload of `CImageLoader::LoadPng`, which also rejects gray images.
pub fn load_png_file(bytes: &[u8]) -> Option<Image> {
    load_png(bytes).filter(|i| matches!(i.format, Format::Rgb | Format::Rgba))
}

impl Image {
    /// C++: `ConvertToRgba`: gray becomes white with the gray value as alpha, gray with
    /// alpha becomes gray RGB.
    pub fn to_rgba(&self) -> Vec<u8> {
        let ch = self.format.channels();
        self.data
            .chunks_exact(ch)
            .flat_map(|p| match self.format {
                Format::Rgba => [p[0], p[1], p[2], p[3]],
                Format::Rgb => [p[0], p[1], p[2], 255],
                Format::Ra => [p[0], p[0], p[0], p[1]],
                Format::R => [255, 255, 255, p[0]],
            })
            .collect()
    }
}

/// C++: `CubicHermite` (image_manipulation.cpp), same operation order.
fn cubic_hermite(a: f32, b: f32, c: f32, d: f32, t: f32) -> f32 {
    let a2 = -a / 2.0 + (3.0 * b) / 2.0 - (3.0 * c) / 2.0 + d / 2.0;
    let b2 = a - (5.0 * b) / 2.0 + 2.0 * c - d / 2.0;
    let c2 = -a / 2.0 + c / 2.0;
    (a2 * t * t * t) + (b2 * t * t) + (c2 * t) + b
}

/// C++: `(uint8_t)f` on x86-64: through `cvttss2si`, keeping the low byte.
fn trunc_u8(f: f32) -> u8 {
    ddnet::physics::vmath::trunc_i32(f) as u8
}

/// C++: `HighestBit` (image_manipulation.cpp).
pub fn highest_bit(v: i32) -> i32 {
    if v == 0 {
        0
    } else {
        1 << (31 - v.leading_zeros())
    }
}

impl Image {
    /// C++: `ResizeImage(CImageInfo &, ...)`: bicubic, sampling `SampleBicubic`.
    pub fn resize(&mut self, new_w: u32, new_h: u32) {
        let (sw, sh, bpp) = (self.width, self.height, self.format.channels());
        let pixel = |x: i32, y: i32| {
            // C++: GetPixelClamped
            let x = x.clamp(0, sw as i32 - 1) as usize;
            let y = y.clamp(0, sh as i32 - 1) as usize;
            &self.data[x * bpp + sw as usize * bpp * y..][..bpp]
        };
        let mut out = vec![0u8; new_w as usize * new_h as usize * bpp];
        for y in 0..new_h {
            let v = y as f32 / (new_h - 1) as f32;
            for x in 0..new_w {
                let u = x as f32 / (new_w - 1) as f32;
                let fx = u * sw as f32 - 0.5;
                let (rx, frac_x) = (ddnet::physics::vmath::trunc_i32(fx), fx - fx.floor());
                let fy = v * sh as f32 - 0.5;
                let (ry, frac_y) = (ddnet::physics::vmath::trunc_i32(fy), fy - fy.floor());
                let at = |i: i32, j: i32, c: usize| {
                    pixel(rx.wrapping_add(i - 1), ry.wrapping_add(j - 1))[c] as f32
                };
                for c in 0..bpp {
                    let rows: [f32; 4] = std::array::from_fn(|j| {
                        let j = j as i32;
                        cubic_hermite(at(0, j, c), at(1, j, c), at(2, j, c), at(3, j, c), frac_x)
                    });
                    let s = cubic_hermite(rows[0], rows[1], rows[2], rows[3], frac_y);
                    // NaN stays NaN, like C++ std::clamp
                    let s = s.clamp(0.0, 255.0);
                    out[(x as usize + new_w as usize * y as usize) * bpp + c] = trunc_u8(s);
                }
            }
        }
        (self.width, self.height, self.data) = (new_w, new_h, out);
    }

    /// C++: `CGraphics_Threaded::CheckImageDivisibility` (graphics_threaded.cpp:593).
    pub fn check_divisibility(&mut self, div_x: u32, div_y: u32, allow_resize: bool) -> bool {
        let width_broken = self.width == 0 || !self.width.is_multiple_of(div_x);
        let height_broken = self.height == 0 || !self.height.is_multiple_of(div_y);
        if !(width_broken || height_broken) {
            return true;
        }
        if !(allow_resize && self.width > 0 && self.height > 0) {
            return false;
        }
        let (div_x, div_y) = (div_x as i32, div_y as i32);
        let (w, h) = if width_broken {
            let w = highest_bit(self.width as i32).max(div_x);
            (w, w / div_x * div_y)
        } else {
            let h = highest_bit(self.height as i32).max(div_y);
            (h / div_y * div_x, h)
        };
        self.resize(w as u32, h as u32);
        true
    }

    /// C++: `ConvertToGrayscale` (image_manipulation.cpp): luma in RGB, alpha kept.
    pub fn to_grayscale(&mut self) {
        if matches!(self.format, Format::R | Format::Ra) {
            return;
        }
        for p in self.data.chunks_exact_mut(self.format.channels()) {
            let luma = trunc_u8(0.2126 * p[0] as f32 + 0.7152 * p[1] as f32 + 0.0722 * p[2] as f32);
            p[..3].fill(luma);
        }
    }

    /// C++: `IsImageSubFullyTransparent` (graphics_threaded.cpp:346).
    pub fn sub_fully_transparent(&self, x: u32, y: u32, w: u32, h: u32) -> bool {
        if self.format == Format::Rgb {
            return false;
        }
        let ps = self.format.channels();
        (0..h).all(|iy| {
            (0..w).all(|ix| {
                self.data
                    [(x + ix) as usize * ps + (y + iy) as usize * ps * self.width as usize + ps - 1]
                    == 0
            })
        })
    }

    /// C++: `CImageInfo::CopyRectFrom` into a new image of the rectangle.
    pub fn sub_image(&self, x: u32, y: u32, w: u32, h: u32) -> Image {
        let ps = self.format.channels();
        let data = (y..y + h)
            .flat_map(|row| {
                let start = (row as usize * self.width as usize + x as usize) * ps;
                self.data[start..start + w as usize * ps].iter().copied()
            })
            .collect();
        Image {
            width: w,
            height: h,
            format: self.format,
            data,
        }
    }
}
