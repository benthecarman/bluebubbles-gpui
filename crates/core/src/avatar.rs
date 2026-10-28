//! Contact photos. The server sends each contact's full-size picture,
//! often several megapixels; the store keeps a small copy, so a chat list
//! of photos stays light to read and draw.

use image::codecs::jpeg::JpegEncoder;
use image::{DynamicImage, ImageDecoder, ImageReader};
use std::io::Cursor;

/// Longest side kept, in pixels: a 40-point avatar at 2× with room to
/// spare, and large enough for a notification's picture.
pub const SIZE: u32 = 192;

/// Why a photo couldn't be kept.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// A format the app has no decoder for, by its signature.
    Format(&'static str),
    Corrupt,
}

/// A small copy of `bytes`: upright, at most [`SIZE`] on its longest
/// side, as JPEG, or PNG when it has transparency.
pub fn shrink(bytes: &[u8]) -> Result<Vec<u8>, Unreadable> {
    let reader = ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| Unreadable::Corrupt)?;
    if reader.format().is_none() {
        return Err(Unreadable::Format(signature(bytes)));
    }
    let mut decoder = reader.into_decoder().map_err(|_| Unreadable::Corrupt)?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).map_err(|_| Unreadable::Corrupt)?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    if image.width() > SIZE || image.height() > SIZE {
        image = image.thumbnail(SIZE, SIZE);
    }
    let mut out = Vec::new();
    let encoded = if image.color().has_alpha() && !opaque(&image) {
        image
            .to_rgba8()
            .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
    } else {
        JpegEncoder::new_with_quality(&mut out, 88).encode_image(&image.to_rgb8())
    };
    encoded.map_err(|_| Unreadable::Corrupt)?;
    Ok(out)
}

fn opaque(image: &DynamicImage) -> bool {
    image.to_rgba8().pixels().all(|p| p.0[3] == 255)
}

/// Names the formats a Mac may hand over that the app can't decode.
fn signature(bytes: &[u8]) -> &'static str {
    match bytes.get(4..12) {
        Some(b"ftypheic" | b"ftypheix" | b"ftypmif1" | b"ftypmsf1" | b"ftyphevc") => "HEIC",
        Some(b"ftypavif") => "AVIF",
        _ if bytes.starts_with(b"%PDF") => "PDF",
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encoded(image: DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut out = Vec::new();
        image.write_to(&mut Cursor::new(&mut out), format).unwrap();
        out
    }

    #[test]
    fn shrinks_photos_to_jpeg() {
        let big = DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1200,
            900,
            image::Rgb([200, 40, 40]),
        ));
        let small = shrink(&encoded(big, image::ImageFormat::Png)).unwrap();
        assert!(
            small.starts_with(&[0xFF, 0xD8]),
            "opaque photos become JPEG"
        );
        let back = image::load_from_memory(&small).unwrap();
        assert_eq!((back.width(), back.height()), (SIZE, 144));
    }

    #[test]
    fn keeps_transparency_and_small_sizes() {
        let mut memoji = image::RgbaImage::from_pixel(100, 100, image::Rgba([0, 0, 0, 0]));
        memoji.put_pixel(50, 50, image::Rgba([255, 200, 0, 255]));
        let out = shrink(&encoded(
            DynamicImage::ImageRgba8(memoji),
            image::ImageFormat::Png,
        ))
        .unwrap();
        assert!(out.starts_with(b"\x89PNG"));
        assert_eq!(image::load_from_memory(&out).unwrap().width(), 100);
    }

    #[test]
    fn names_what_it_cannot_read() {
        let heic = b"\0\0\0\x18ftypheic\0\0\0\0mif1heic";
        assert_eq!(shrink(heic), Err(Unreadable::Format("HEIC")));
        assert_eq!(shrink(b"\xFF\xD8\xFF garbage"), Err(Unreadable::Corrupt));
    }
}
