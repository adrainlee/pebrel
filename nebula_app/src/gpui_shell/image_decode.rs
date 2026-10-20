//! Raster decode budgets shared by terminal, clipboard and document adapters.
use std::io::{self, Read};
use std::path::Path;

pub(super) const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
pub(super) const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;

pub(super) fn limit_error() -> image::ImageError {
    image::ImageError::Limits(image::error::LimitError::from_kind(
        image::error::LimitErrorKind::InsufficientMemory,
    ))
}

pub(super) fn read_file(path: &Path) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_ENCODED_BYTES as u64 {
        return Err(io::Error::other("image source exceeds its byte limit or is not a file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_ENCODED_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err(io::Error::other("image source exceeds its byte limit"));
    }
    Ok(bytes)
}

pub(super) fn rgba(
    data: &[u8],
    format: image::ImageFormat,
    encoded_limit: usize,
    decoded_limit: usize,
) -> image::ImageResult<image::RgbaImage> {
    let decoded_limit = decoded_limit.min(MAX_DECODED_BYTES);
    if data.len() > encoded_limit.min(MAX_ENCODED_BYTES) {
        return Err(limit_error());
    }
    let (width, height) =
        image::ImageReader::with_format(std::io::Cursor::new(data), format).into_dimensions()?;
    let pixels = u64::from(width) * u64::from(height);
    if pixels == 0 || pixels > decoded_limit as u64 / 4 {
        return Err(limit_error());
    }
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(data), format);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(decoded_limit as u64);
    limits.max_image_width = Some(width);
    limits.max_image_height = Some(height);
    reader.limits(limits);
    let image = reader.decode()?;
    if (image.width(), image.height()) != (width, height) {
        return Err(limit_error());
    }
    Ok(image.into_rgba8())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tiny_gif_with_large_canvas_is_rejected_before_pixel_allocation() {
        let mut gif = vec![
            71, 73, 70, 56, 57, 97, 1, 0, 1, 0, 128, 0, 0, 0, 0, 0, 255, 255, 255, 33, 249, 4, 1,
            0, 0, 0, 0, 44, 0, 0, 0, 0, 1, 0, 1, 0, 0, 2, 2, 68, 1, 0, 59,
        ];
        assert_eq!(rgba(&gif, image::ImageFormat::Gif, 64, 4).unwrap().dimensions(), (1, 1));
        for offset in [6, 8] {
            gif[offset..offset + 2].copy_from_slice(&4097u16.to_le_bytes());
        }
        assert!(matches!(
            rgba(&gif, image::ImageFormat::Gif, 64, MAX_DECODED_BYTES),
            Err(image::ImageError::Limits(_))
        ));
    }
    #[test]
    fn rejects_oversized_files_before_reading_them() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("huge.gif");
        std::fs::File::create(&path).unwrap().set_len(MAX_ENCODED_BYTES as u64 + 1).unwrap();
        assert!(read_file(&path).is_err());
        assert!(read_file(directory.path()).is_err());
    }
    #[test]
    fn animated_images_decode_only_the_first_frame() {
        let mut bytes = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut bytes);
            for color in [[255, 0, 0, 255], [0, 255, 0, 255]] {
                encoder
                    .encode_frame(image::Frame::new(image::RgbaImage::from_pixel(
                        2,
                        2,
                        image::Rgba(color),
                    )))
                    .unwrap();
            }
        }
        let image = rgba(&bytes, image::ImageFormat::Gif, MAX_ENCODED_BYTES, 16).unwrap();
        assert_eq!(image.dimensions(), (2, 2));
        assert_eq!(image.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert!(rgba(&bytes, image::ImageFormat::Gif, MAX_ENCODED_BYTES, 15).is_err());
    }
}
