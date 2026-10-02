//! Bounded reads of actual images from explicitly trusted local directories.

use image::{ImageFormat, ImageReader, Limits};
use std::fs::File;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

pub(crate) struct LocalImageResource {
    pub bytes: Vec<u8>,
    pub extension: &'static str,
    pub mime: &'static str,
}

pub(crate) fn managed_image_roots(data_dir: &Path) -> [PathBuf; 2] {
    [
        data_dir.join("attachments"),
        data_dir.join("emoji_favorites"),
    ]
}

pub(crate) fn clipboard_image_roots(data_dir: &Path) -> Vec<PathBuf> {
    let mut roots = managed_image_roots(data_dir).to_vec();
    // Office and chat applications publish delayed images from temporary caches.
    roots.push(std::env::temp_dir());
    roots
}

pub(crate) fn read_local_image(
    path: &Path,
    trusted_roots: &[PathBuf],
    max_bytes: usize,
) -> Option<LocalImageResource> {
    let path = path.canonicalize().ok()?;
    if !trusted_roots.iter().any(|root| {
        root.canonicalize()
            .ok()
            .is_some_and(|root| path.starts_with(root))
    }) {
        return None;
    }

    let file = File::open(&path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() == 0 || metadata.len() > max_bytes as u64 {
        return None;
    }
    let mut bytes = Vec::new();
    file.take(max_bytes as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || bytes.len() > max_bytes {
        return None;
    }

    let mut reader = ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .ok()?;
    let (extension, mime) = match reader.format()? {
        ImageFormat::Png => ("png", "image/png"),
        ImageFormat::Jpeg => ("jpg", "image/jpeg"),
        ImageFormat::Gif => ("gif", "image/gif"),
        ImageFormat::WebP => ("webp", "image/webp"),
        ImageFormat::Bmp => ("bmp", "image/bmp"),
        _ => return None,
    };
    let mut limits = Limits::default();
    limits.max_image_width = Some(16_384);
    limits.max_image_height = Some(16_384);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    // Validate the payload without replacing the original animated bytes.
    reader.decode().ok()?;
    Some(LocalImageResource {
        bytes,
        extension,
        mime,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn fixture() -> (PathBuf, PathBuf, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("tiez-image-scope-{}", uuid::Uuid::new_v4()));
        let root = dir.join("trusted");
        fs::create_dir_all(&root).unwrap();
        let mut bytes = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(1, 1)
            .write_to(&mut bytes, ImageFormat::Png)
            .unwrap();
        (dir, root, bytes.into_inner())
    }

    #[test]
    fn accepts_mislabeled_image_and_preserves_original_bytes() {
        let (dir, root, png) = fixture();
        let path = root.join("cache.jpg");
        fs::write(&path, &png).unwrap();
        let resource = read_local_image(&path, &[root], 1024).unwrap();
        assert_eq!(resource.bytes, png);
        assert_eq!(resource.extension, "png");
        assert_eq!(resource.mime, "image/png");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_non_images_and_truncated_signatures() {
        let (dir, root, _) = fixture();
        for (name, bytes) in [
            ("private.png", b"private text".as_slice()),
            ("broken.gif", b"GIF89a".as_slice()),
        ] {
            let path = root.join(name);
            fs::write(&path, bytes).unwrap();
            assert!(read_local_image(&path, std::slice::from_ref(&root), 1024).is_none());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_parent_escape_and_sibling_prefix_even_for_valid_images() {
        let (dir, root, png) = fixture();
        fs::write(dir.join("private.png"), &png).unwrap();
        let sibling = dir.join("trusted-extra");
        fs::create_dir(&sibling).unwrap();
        fs::write(sibling.join("image.png"), &png).unwrap();
        for path in [root.join("../private.png"), sibling.join("image.png")] {
            assert!(read_local_image(&path, std::slice::from_ref(&root), 1024).is_none());
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rejects_images_larger_than_the_byte_limit() {
        let (dir, root, png) = fixture();
        let path = root.join("large.png");
        fs::write(&path, &png).unwrap();
        assert!(read_local_image(&path, &[root], png.len() - 1).is_none());
        fs::remove_dir_all(dir).unwrap();
    }
}
