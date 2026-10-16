//! Bounded native raster previews. Original clipboard bytes are never replaced.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::Serialize;
use std::io::Cursor;
use std::path::PathBuf;

#[derive(Serialize)]
pub struct RasterPreview {
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub animation_source: bool,
    pub png_base64: String,
}

impl std::fmt::Debug for RasterPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RasterPreview")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("animation_source", &self.animation_source)
            .finish_non_exhaustive()
    }
}

pub fn raster(
    payload: &str,
    roots: &[PathBuf],
    max_edge: u32,
) -> Result<RasterPreview, &'static str> {
    if !(1..=1024).contains(&max_edge) {
        return Err("preview_dimensions");
    }
    let bytes = crate::write::image_bytes(payload, roots)?;
    let animation_source = crate::write_helpers::is_animated_clipboard_image_bytes(&bytes);
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| "image_decode")?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().map_err(|_| "image_decode")?;
    let (original_width, original_height) = (image.width(), image.height());
    crate::write::validate_dimensions(original_width, original_height)?;
    let image = if original_width > max_edge || original_height > max_edge {
        image.thumbnail(max_edge, max_edge)
    } else {
        image
    };
    let image = image::DynamicImage::ImageRgba8(image.into_rgba8());
    let (width, height) = (image.width(), image.height());
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::Png)
        .map_err(|_| "image_encode")?;
    let encoded = encoded.into_inner();
    if encoded.len() > 5 * 1024 * 1024 {
        return Err("preview_too_large");
    }
    Ok(RasterPreview {
        original_width,
        original_height,
        width,
        height,
        animation_source,
        png_base64: STANDARD.encode(encoded),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut data = Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            width,
            height,
            image::Rgba([255, 0, 0, 128]),
        ))
        .write_to(&mut data, image::ImageFormat::Png)
        .unwrap();
        data.into_inner()
    }
    fn url(bytes: &[u8]) -> String {
        format!("data:image/png;base64,{}", STANDARD.encode(bytes))
    }
    #[test]
    fn thumbnail_keeps_aspect_alpha_and_original_bytes() {
        let original = png(2000, 1000);
        let payload = url(&original);
        let preview = raster(&payload, &[], 128).unwrap();
        assert_eq!(
            (preview.original_width, preview.original_height),
            (2000, 1000)
        );
        assert_eq!((preview.width, preview.height), (128, 64));
        let decoded = image::load_from_memory(&STANDARD.decode(preview.png_base64).unwrap())
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.get_pixel(0, 0).0, [255, 0, 0, 128]);
        assert_eq!(payload, url(&original));
        assert!(!preview.animation_source);
    }
    #[test]
    fn small_previews_do_not_upscale_and_reject_bad_inputs() {
        let preview = raster(&url(&png(1, 1)), &[], 512).unwrap();
        assert_eq!((preview.width, preview.height), (1, 1));
        for edge in [0, 1025] {
            assert_eq!(
                raster(&url(&png(1, 1)), &[], edge).unwrap_err(),
                "preview_dimensions"
            );
        }
        for bad in [
            "data:image/png;base64,bad!",
            "data:image/png;base64,YmFk",
            "https://example.invalid/image.png",
            "relative.png",
        ] {
            assert!(raster(bad, &[], 512).is_err());
        }
    }
    #[test]
    fn local_paths_require_explicit_scope_and_missing_files_fail() {
        let directory = std::env::temp_dir().join(format!("TieZ-preview-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("原图.png");
        std::fs::write(&path, png(2, 3)).unwrap();
        assert_eq!(
            raster(path.to_str().unwrap(), &[], 512).unwrap_err(),
            "image_resource_unavailable"
        );
        let preview = raster(path.to_str().unwrap(), &[directory.clone()], 512).unwrap();
        assert_eq!((preview.original_width, preview.original_height), (2, 3));
        std::fs::remove_file(&path).unwrap();
        assert!(raster(path.to_str().unwrap(), &[directory.clone()], 512).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn gif_preview_is_a_poster_and_preserves_animated_source() {
        let first = image::RgbaImage::from_pixel(2, 2, image::Rgba([255, 0, 0, 255]));
        let second = image::RgbaImage::from_pixel(2, 2, image::Rgba([0, 255, 0, 255]));
        let mut gif = Vec::new();
        {
            let mut encoder = image::codecs::gif::GifEncoder::new(&mut gif);
            encoder
                .encode_frames([image::Frame::new(first), image::Frame::new(second)])
                .unwrap();
        }
        let payload = format!("data:image/gif;base64,{}", STANDARD.encode(&gif));
        let preview = raster(&payload, &[], 512).unwrap();
        assert!(preview.animation_source);
        let poster = image::load_from_memory(&STANDARD.decode(preview.png_base64).unwrap())
            .unwrap()
            .into_rgba8();
        assert_eq!(poster.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(
            STANDARD.decode(payload.split_once(',').unwrap().1).unwrap(),
            gif
        );
    }
    #[test]
    fn overlarge_source_dimensions_are_rejected_before_preview_scaling() {
        assert!(raster(&url(&png(16_385, 1)), &[], 512).is_err());
    }
}
