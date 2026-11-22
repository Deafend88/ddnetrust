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
