//! Photos in the transcript: decoded and scaled down off the UI thread,
//! once per file, so a chat full of 12-megapixel photos holds small
//! textures. The full file stays on disk for opening.

use gpui_kit::RenderImage;
use image::codecs::gif::GifDecoder;
use image::{AnimationDecoder, DynamicImage, Frame, ImageDecoder, ImageReader};
use std::io::Cursor;
use std::path::Path;
use std::sync::Arc;

/// Longest side of a decoded thumbnail, in pixels: two device pixels per
/// point of the largest bubble.
pub const THUMB_MAX: u32 = 640;
/// Frames kept from an animated GIF.
const GIF_FRAMES: usize = 300;

/// The largest a photo is drawn inline, in points.
pub const MAX_W: f32 = 260.;
pub const MAX_H: f32 = 300.;
const MIN_SIDE: f32 = 60.;

/// The size a photo is drawn at: its own aspect ratio within the bubble
/// limits, or a 4:3 placeholder while its size is unknown.
pub fn display_size(width: Option<i64>, height: Option<i64>) -> (f32, f32) {
    let (w, h) = match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => (w as f32, h as f32),
        _ => (4., 3.),
    };
    let scale = (MAX_W / w).min(MAX_H / h);
    ((w * scale).max(MIN_SIDE), (h * scale).max(MIN_SIDE))
}

/// Decodes `path` into a texture-ready image no larger than [`THUMB_MAX`],
/// upright per its EXIF orientation. GIFs keep their animation.
pub fn thumbnail(path: &Path) -> Option<Arc<RenderImage>> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.starts_with(b"GIF8")
        && let Some(frames) = gif_frames(&bytes)
    {
        return Some(Arc::new(RenderImage::new(frames)));
    }
    let mut decoder = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?
        .into_decoder()
        .ok()?;
    let orientation = decoder.orientation().ok();
    let mut image = DynamicImage::from_decoder(decoder).ok()?;
    if let Some(orientation) = orientation {
        image.apply_orientation(orientation);
    }
    if image.width() > THUMB_MAX || image.height() > THUMB_MAX {
        image = image.thumbnail(THUMB_MAX, THUMB_MAX);
    }
    let mut rgba = image.to_rgba8();
    to_bgra(&mut rgba);
    Some(Arc::new(RenderImage::new([Frame::new(rgba)])))
}

fn gif_frames(bytes: &[u8]) -> Option<Vec<Frame>> {
    let decoder = GifDecoder::new(Cursor::new(bytes)).ok()?;
    let mut frames = Vec::new();
    for frame in decoder.into_frames().take(GIF_FRAMES) {
        let frame = frame.ok()?;
        let (left, top, delay) = (frame.left(), frame.top(), frame.delay());
        let mut buffer = frame.into_buffer();
        to_bgra(&mut buffer);
        frames.push(Frame::from_parts(buffer, left, top, delay));
    }
    (!frames.is_empty()).then_some(frames)
}

/// GPUI's textures are BGRA.
fn to_bgra(image: &mut image::RgbaImage) {
    for pixel in image.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
}

/// "1.2 MB", "340 KB".
pub fn file_size(bytes: i64) -> String {
    let b = bytes.max(0) as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} KB", b / 1e3)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_photos_in_the_bubble() {
        assert_eq!(display_size(Some(4032), Some(3024)), (260., 195.));
        assert_eq!(display_size(Some(3024), Some(4032)), (225., 300.));
        assert_eq!(display_size(None, None), (260., 195.));
        // A tall strip keeps a usable width.
        assert_eq!(display_size(Some(10), Some(1000)).0, MIN_SIDE);
    }

    #[test]
    fn scales_down_and_swaps_channels() {
        let dir = std::env::temp_dir().join(format!("bb-thumb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("big.png");
        let mut big = image::RgbaImage::new(1600, 800);
        big.pixels_mut()
            .for_each(|p| *p = image::Rgba([255, 0, 0, 255]));
        big.save(&path).unwrap();
        let thumb = thumbnail(&path).unwrap();
        let size = thumb.size(0);
        assert_eq!((size.width.0, size.height.0), (640, 320));
        // Red in RGBA is blue-first in BGRA.
        assert_eq!(&thumb.as_bytes(0).unwrap()[..4], &[0, 0, 255, 255]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn sizes() {
        assert_eq!(file_size(512), "512 bytes");
        assert_eq!(file_size(340_000), "340 KB");
        assert_eq!(file_size(1_250_000), "1.2 MB");
    }
}
